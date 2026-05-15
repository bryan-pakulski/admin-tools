use std::collections::HashMap;
use std::time::Instant;

use crate::config::FreezeThresholds;
use crate::sampler::{ThreadSample, ThreadState};

/// Wait channels that indicate "blocking on socket / poll / select" — when a thread
/// sits in one of these continuously for `net_wchan` it is almost certainly the
/// recv/send freeze the user is hunting.
const NET_WCHANS: &[&str] = &[
    "sk_wait_data",
    "tcp_recvmsg",
    "tcp_recvmsg_locked",
    "inet_csk_accept",
    "do_select",
    "ep_poll",
    "poll_schedule_timeout",
];

/// Syscalls whose presence — combined with a generic "wait" wchan — is a strong
/// signal of a socket recv/send wait. Modern kernels often surface a generic
/// wait_woken / schedule_timeout symbol rather than the more specific sk_wait_data,
/// so we cross-reference the syscall.
const NET_SYSCALLS: &[&str] = &[
    "recvfrom",
    "recvmsg",
    "sendto",
    "sendmsg",
    "accept",
    "accept4",
    "connect",
    "poll",
    "ppoll",
    "select",
    "pselect6",
    "epoll_wait",
    "epoll_pwait",
    "epoll_pwait2",
];

/// Generic wait wchans that *might* indicate a socket wait when paired with a
/// network syscall. Treated as NetWchan only when the syscall matches.
const GENERIC_WAIT_WCHANS: &[&str] = &[
    "wait_woken",
    "schedule_timeout",
];

/// Wchans signalling deliberate idle work — exclude from CpuDivergence so we don't
/// spam every healthy sleeping thread in a real process.
const BENIGN_IDLE_WCHANS: &[&str] = &[
    "hrtimer_nanosleep",
    "do_nanosleep",
    "futex_wait_queue",
    "futex_wait",
    "pipe_read",
    "pipe_wait",
    "do_sigtimedwait",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreezeReason {
    DState,
    NetWchan,
    NoCtxSwitch,
    CpuDivergence,
}

impl FreezeReason {
    pub fn label(self) -> &'static str {
        match self {
            Self::DState => "D-state",
            Self::NetWchan => "net wait",
            Self::NoCtxSwitch => "no ctxsw",
            Self::CpuDivergence => "diverged",
        }
    }
}

#[derive(Debug, Clone)]
pub struct FreezeFlag {
    pub since: Instant,
    pub wchan: String,
    pub syscall_name: Option<&'static str>,
    pub reason: FreezeReason,
}

#[derive(Debug)]
enum TidState {
    Healthy,
    Suspect {
        since: Instant,
        wchan: String,
        syscall: Option<&'static str>,
        reason: FreezeReason,
    },
    Flagged {
        since: Instant,
        wchan: String,
        syscall: Option<&'static str>,
        reason: FreezeReason,
    },
}

#[derive(Debug)]
struct PrevCounters {
    vol_ctxsw: u64,
    invol_ctxsw: u64,
}

#[derive(Debug)]
struct PerTid {
    state: TidState,
    prev: Option<PrevCounters>,
    starttime_ticks: u64,
}

#[derive(Debug)]
pub struct FreezeDetector {
    thresholds: FreezeThresholds,
    tids: HashMap<i32, PerTid>,
}

impl FreezeDetector {
    pub fn new(thresholds: FreezeThresholds) -> Self {
        Self {
            thresholds,
            tids: HashMap::new(),
        }
    }

    pub fn observe(
        &mut self,
        sample: &ThreadSample,
        now: Instant,
        peers_have_progress: bool,
        delta_cpu_ticks: u64,
    ) -> Option<FreezeFlag> {
        let thresholds = self.thresholds;
        // Handle TID reuse by checking starttime_ticks.
        let entry = self
            .tids
            .entry(sample.tid)
            .or_insert_with(|| PerTid {
                state: TidState::Healthy,
                prev: None,
                starttime_ticks: sample.starttime_ticks,
            });

        if entry.starttime_ticks != sample.starttime_ticks {
            *entry = PerTid {
                state: TidState::Healthy,
                prev: None,
                starttime_ticks: sample.starttime_ticks,
            };
        }

        let prev = entry.prev.replace(PrevCounters {
            vol_ctxsw: sample.vol_ctxsw,
            invol_ctxsw: sample.invol_ctxsw,
        });

        let reason = classify(sample, prev.as_ref(), peers_have_progress, delta_cpu_ticks);

        match (&entry.state, reason) {
            (_, None) => {
                entry.state = TidState::Healthy;
                None
            }
            (TidState::Healthy, Some(r)) => {
                entry.state = TidState::Suspect {
                    since: now,
                    wchan: sample.wchan.clone(),
                    syscall: sample.syscall.map(|s| {
                        crate::sampler::syscall_table::name(s.nr)
                    }),
                    reason: r,
                };
                None
            }
            (TidState::Suspect { since, wchan, syscall, reason: prev_r }, Some(r)) => {
                let since = *since;
                let wchan = wchan.clone();
                let syscall = *syscall;
                let threshold = match r {
                    FreezeReason::DState => thresholds.d_state,
                    FreezeReason::NetWchan => thresholds.net_wchan,
                    FreezeReason::NoCtxSwitch => thresholds.no_ctxsw,
                    FreezeReason::CpuDivergence => thresholds.cpu_divergence,
                };
                if r == *prev_r && now.duration_since(since) >= threshold {
                    let flag = FreezeFlag {
                        since,
                        wchan: wchan.clone(),
                        syscall_name: syscall,
                        reason: r,
                    };
                    entry.state = TidState::Flagged {
                        since,
                        wchan,
                        syscall,
                        reason: r,
                    };
                    Some(flag)
                } else if r != *prev_r {
                    // Reason changed — restart suspicion clock.
                    entry.state = TidState::Suspect {
                        since: now,
                        wchan: sample.wchan.clone(),
                        syscall: sample.syscall.map(|s| {
                            crate::sampler::syscall_table::name(s.nr)
                        }),
                        reason: r,
                    };
                    None
                } else {
                    None
                }
            }
            (TidState::Flagged { since, wchan, syscall, reason: prev_r }, Some(r)) => {
                // Persist while still in a concerning state. If the wchan changed mid-flag
                // (different wait channel), unflag and start a fresh suspicion.
                if r == *prev_r && wchan == &sample.wchan {
                    Some(FreezeFlag {
                        since: *since,
                        wchan: wchan.clone(),
                        syscall_name: *syscall,
                        reason: *prev_r,
                    })
                } else {
                    entry.state = TidState::Suspect {
                        since: now,
                        wchan: sample.wchan.clone(),
                        syscall: sample.syscall.map(|s| {
                            crate::sampler::syscall_table::name(s.nr)
                        }),
                        reason: r,
                    };
                    None
                }
            }
        }
    }

    pub fn forget(&mut self, tid: i32) {
        self.tids.remove(&tid);
    }
}

fn classify(
    sample: &ThreadSample,
    prev: Option<&PrevCounters>,
    peers_have_progress: bool,
    delta_cpu_ticks: u64,
) -> Option<FreezeReason> {
    if sample.state == ThreadState::Disk {
        return Some(FreezeReason::DState);
    }

    let syscall_name = sample
        .syscall
        .map(|s| crate::sampler::syscall_table::name(s.nr))
        .unwrap_or("");

    // Explicit socket-wait wchan.
    let explicit_net = NET_WCHANS
        .iter()
        .any(|w| sample.wchan.eq_ignore_ascii_case(w));

    // Generic wait wchan + network syscall → also count as net wait.
    let generic_with_net_syscall = GENERIC_WAIT_WCHANS
        .iter()
        .any(|w| sample.wchan.eq_ignore_ascii_case(w))
        && NET_SYSCALLS.iter().any(|s| *s == syscall_name);

    if sample.state == ThreadState::Sleeping && (explicit_net || generic_with_net_syscall) {
        return Some(FreezeReason::NetWchan);
    }

    if let Some(p) = prev {
        let same_ctxsw = sample.vol_ctxsw == p.vol_ctxsw && sample.invol_ctxsw == p.invol_ctxsw;
        if same_ctxsw && delta_cpu_ticks > 0 {
            return Some(FreezeReason::NoCtxSwitch);
        }
    }

    let is_benign = BENIGN_IDLE_WCHANS
        .iter()
        .any(|w| sample.wchan.eq_ignore_ascii_case(w));

    if delta_cpu_ticks == 0
        && peers_have_progress
        && !is_benign
        && matches!(
            sample.state,
            ThreadState::Sleeping | ThreadState::Disk | ThreadState::Idle
        )
    {
        return Some(FreezeReason::CpuDivergence);
    }
    None
}
