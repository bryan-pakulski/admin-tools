use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use tokio::sync::mpsc;
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;

use crate::capabilities::Capabilities;
use crate::config::FreezeThresholds;
use crate::export::CsvLog;
use crate::freeze::{FreezeDetector, FreezeFlag};
use crate::sampler::{CpuTotal, ProcessSample, SampleBatch, ThreadSample, ThreadState};
use crate::state::{
    new_shared, ProcessPoint, ProcessView, RecentPoint, SharedSnapshot, Snapshot, ThreadView,
    Transition, RECENT_CAP, TRANSITION_CAP,
};

#[derive(Debug)]
struct ThreadHistory {
    prev: Option<ThreadSample>,
    prev_at: Option<Instant>,
    recent: VecDeque<RecentPoint>,
    transitions: VecDeque<Transition>,
    last_state: Option<ThreadState>,
    last_wchan: String,
    last_syscall: Option<&'static str>,
    starttime_ticks: u64,
    freeze: Option<FreezeFlag>,
    sched_wait_ns_per_s: f64,
}

impl ThreadHistory {
    fn new(starttime_ticks: u64) -> Self {
        Self {
            prev: None,
            prev_at: None,
            recent: VecDeque::with_capacity(RECENT_CAP),
            transitions: VecDeque::with_capacity(TRANSITION_CAP),
            last_state: None,
            last_wchan: String::new(),
            last_syscall: None,
            starttime_ticks,
            freeze: None,
            sched_wait_ns_per_s: 0.0,
        }
    }
}

#[derive(Debug)]
struct ProcessHistory {
    prev: Option<ProcessSample>,
    prev_at: Option<Instant>,
    prev_cpu_total: Option<CpuTotal>,
    recent: VecDeque<ProcessPoint>,
}

pub struct AggregatorHandles {
    pub snapshot: SharedSnapshot,
}

pub struct AggregatorConfig {
    pub pid: i32,
    pub interval: Duration,
    pub caps: Capabilities,
    pub thresholds: FreezeThresholds,
    pub filter: Option<String>,
    pub recent_cap: usize,
    pub max_history: Duration,
}

pub fn spawn(
    cfg: AggregatorConfig,
    samples_rx: mpsc::Receiver<SampleBatch>,
    paused_rx: watch::Receiver<bool>,
    interval_rx: watch::Receiver<Duration>,
    csv_log: Option<CsvLog>,
) -> AggregatorHandles {
    let snapshot = new_shared(cfg.pid, cfg.interval, cfg.caps);
    let snap_clone = snapshot.clone();
    tokio::spawn(run(
        snap_clone,
        cfg.thresholds,
        cfg.filter,
        cfg.recent_cap,
        cfg.max_history,
        samples_rx,
        paused_rx,
        interval_rx,
        csv_log,
    ));
    AggregatorHandles { snapshot }
}

async fn run(
    snapshot: SharedSnapshot,
    thresholds: FreezeThresholds,
    filter: Option<String>,
    recent_cap: usize,
    max_history: Duration,
    mut samples_rx: mpsc::Receiver<SampleBatch>,
    paused_rx: watch::Receiver<bool>,
    interval_rx: watch::Receiver<Duration>,
    mut csv_log: Option<CsvLog>,
) {
    let mut histories: HashMap<i32, ThreadHistory> = HashMap::new();
    let mut process_history = ProcessHistory {
        prev: None,
        prev_at: None,
        prev_cpu_total: None,
        recent: VecDeque::with_capacity(recent_cap.min(RECENT_CAP)),
    };
    let mut detector = FreezeDetector::new(thresholds);

    let clk_tck = clk_tck();
    let ncores = num_cores();

    let filter_re = filter
        .as_deref()
        .and_then(|s| regex::Regex::new(s).ok())
        .map(Arc::new);

    let mut publish_tick = tokio::time::interval(Duration::from_millis(50));
    publish_tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut have_data = false;
    let mut target_gone = false;

    loop {
        tokio::select! {
            biased;
            maybe_batch = samples_rx.recv() => {
                match maybe_batch {
                    None => return,
                    Some(batch) => {
                        if batch.target_gone {
                            target_gone = true;
                            publish_now(
                                &snapshot,
                                &histories,
                                &process_history,
                                &paused_rx,
                                &interval_rx,
                                filter.as_deref(),
                                &filter_re,
                                target_gone,
                            );
                            continue;
                        }
                        ingest(
                            &mut histories,
                            &mut process_history,
                            &mut detector,
                            &mut csv_log,
                            batch,
                            clk_tck,
                            ncores,
                            recent_cap,
                            max_history,
                        );
                        have_data = true;
                    }
                }
            }
            _ = publish_tick.tick() => {
                if have_data {
                    publish_now(
                        &snapshot,
                        &histories,
                        &process_history,
                        &paused_rx,
                        &interval_rx,
                        filter.as_deref(),
                        &filter_re,
                        target_gone,
                    );
                }
            }
        }
    }
}

fn ingest(
    histories: &mut HashMap<i32, ThreadHistory>,
    process_history: &mut ProcessHistory,
    detector: &mut FreezeDetector,
    csv_log: &mut Option<CsvLog>,
    batch: SampleBatch,
    clk_tck: f64,
    ncores: f64,
    recent_cap: usize,
    max_history: Duration,
) {
    let Some(proc_sample) = batch.process.clone() else {
        return;
    };

    let now = batch.at;
    let wall = batch.wall;

    // First pass: do any peers show CPU progress this tick?
    let mut peers_have_progress = false;
    for t in &batch.threads {
        if let Some(h) = histories.get(&t.tid) {
            if h.starttime_ticks == t.starttime_ticks {
                if let Some(prev) = &h.prev {
                    let du = t.utime.saturating_sub(prev.utime);
                    let ds = t.stime.saturating_sub(prev.stime);
                    if du + ds > 0 {
                        peers_have_progress = true;
                        break;
                    }
                }
            }
        }
    }

    let mut seen = HashSet::with_capacity(batch.threads.len());

    for t in batch.threads {
        seen.insert(t.tid);

        let history = histories
            .entry(t.tid)
            .or_insert_with(|| ThreadHistory::new(t.starttime_ticks));
        if history.starttime_ticks != t.starttime_ticks {
            *history = ThreadHistory::new(t.starttime_ticks);
        }

        let prev = history.prev.clone();
        let wall_s = match history.prev_at {
            Some(p) => now.duration_since(p).as_secs_f64().max(1e-3),
            None => 0.0,
        };

        let (cpu_pct, sys_pct, ctxsw_v, ctxsw_iv, rchar_bps, wchar_bps, sched_wait_ns_per_s) =
            if let (Some(p), true) = (&prev, wall_s > 0.0) {
                let du = t.utime.saturating_sub(p.utime) as f64 / clk_tck;
                let ds = t.stime.saturating_sub(p.stime) as f64 / clk_tck;
                let user_pct = (100.0 * du / wall_s) as f32;
                let sys_pct = (100.0 * ds / wall_s) as f32;
                let cpu_pct = (user_pct + sys_pct).max(0.0);
                let v = (t.vol_ctxsw.saturating_sub(p.vol_ctxsw) as f64 / wall_s) as f32;
                let iv = (t.invol_ctxsw.saturating_sub(p.invol_ctxsw) as f64 / wall_s) as f32;
                let (rbps, wbps) = match (t.io.as_ref(), p.io.as_ref()) {
                    (Some(cur), Some(prev_io)) => (
                        t.io.as_ref().map(|io| io.rchar.saturating_sub(prev_io.rchar)).unwrap_or(0)
                            as f64
                            / wall_s,
                        cur.wchar.saturating_sub(prev_io.wchar) as f64 / wall_s,
                    ),
                    _ => (0.0, 0.0),
                };
                let wait_dn =
                    t.schedstat.wait_ns.saturating_sub(p.schedstat.wait_ns) as f64 / wall_s;
                (cpu_pct, sys_pct, v, iv, rbps, wbps, wait_dn)
            } else {
                (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)
            };

        let delta_cpu_ticks = match &prev {
            Some(p) => t.utime.saturating_sub(p.utime) + t.stime.saturating_sub(p.stime),
            None => 0,
        };
        let flag = detector.observe(&t, now, peers_have_progress, delta_cpu_ticks);
        history.freeze = flag.clone();
        history.sched_wait_ns_per_s = sched_wait_ns_per_s;

        let syscall_name = t.syscall.map(|s| crate::sampler::syscall_table::name(s.nr));

        // Transition log when state/wchan/syscall changes.
        let changed = history.last_state.map(|s| s != t.state).unwrap_or(true)
            || history.last_wchan != t.wchan
            || history.last_syscall != syscall_name;
        if changed {
            if history.transitions.len() == TRANSITION_CAP {
                history.transitions.pop_front();
            }
            history.transitions.push_back(Transition {
                at: now,
                wall_us: wall
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_micros() as i128)
                    .unwrap_or(0),
                state: t.state,
                wchan: t.wchan.clone(),
                syscall_name,
            });
            history.last_state = Some(t.state);
            history.last_wchan = t.wchan.clone();
            history.last_syscall = syscall_name;
        }

        // Drop samples by either count cap OR wall-time age. Wall-time bound keeps
        // the buffer covering a fixed window regardless of polling rate.
        while let Some(front) = history.recent.front() {
            let too_old = now.duration_since(front.at) > max_history;
            let too_many = history.recent.len() >= recent_cap;
            if too_old || too_many {
                history.recent.pop_front();
            } else {
                break;
            }
        }
        history.recent.push_back(RecentPoint {
            at: now,
            cpu_pct,
            sys_pct,
            state: t.state,
            ctxsw_vol_per_s: ctxsw_v,
            ctxsw_invol_per_s: ctxsw_iv,
            rchar_bps,
            wchar_bps,
            sched_wait_ms_per_s: (sched_wait_ns_per_s / 1e6) as f32,
        });

        if let Some(log) = csv_log.as_mut() {
            let _ = log.write_thread_row(
                wall,
                proc_sample.pid,
                t.tid,
                &t.name,
                t.state,
                cpu_pct,
                sys_pct,
                ctxsw_v,
                ctxsw_iv,
                &t.wchan,
                syscall_name,
                t.syscall.map(|s| s.args).unwrap_or([0; 6]),
                flag.as_ref(),
                rchar_bps,
                wchar_bps,
            );
        }

        history.prev = Some(t);
        history.prev_at = Some(now);
    }

    // Drop history + detector entries for TIDs that disappeared.
    let dead: Vec<i32> = histories
        .keys()
        .copied()
        .filter(|tid| !seen.contains(tid))
        .collect();
    for tid in dead {
        histories.remove(&tid);
        detector.forget(tid);
    }

    // Process row delta.
    let wall_s = match process_history.prev_at {
        Some(p) => now.duration_since(p).as_secs_f64().max(1e-3),
        None => 0.0,
    };
    let cpu_pct_host = if let (Some(prev), true) = (process_history.prev.as_ref(), wall_s > 0.0) {
        let du = proc_sample.utime.saturating_sub(prev.utime) as f64 / clk_tck;
        let ds = proc_sample.stime.saturating_sub(prev.stime) as f64 / clk_tck;
        let core_pct = 100.0 * (du + ds) / wall_s;
        (core_pct / ncores) as f32
    } else {
        0.0
    };
    while let Some(front) = process_history.recent.front() {
        let too_old = now.duration_since(front.at) > max_history;
        let too_many = process_history.recent.len() >= recent_cap;
        if too_old || too_many {
            process_history.recent.pop_front();
        } else {
            break;
        }
    }
    process_history.recent.push_back(ProcessPoint {
        at: now,
        cpu_pct_host,
        rss_bytes: proc_sample.vm_rss_kb * 1024,
        fd_count: proc_sample.fd_count,
    });

    process_history.prev = Some(proc_sample);
    process_history.prev_at = Some(now);
    process_history.prev_cpu_total = batch.cpu_total;
}

fn publish_now(
    snapshot: &SharedSnapshot,
    histories: &HashMap<i32, ThreadHistory>,
    process_history: &ProcessHistory,
    paused_rx: &watch::Receiver<bool>,
    interval_rx: &watch::Receiver<Duration>,
    filter: Option<&str>,
    filter_re: &Option<Arc<regex::Regex>>,
    target_gone: bool,
) {
    let process = match process_history.prev.as_ref() {
        Some(p) => {
            let name = if !p.cmdline.is_empty() {
                p.cmdline
                    .split_whitespace()
                    .next()
                    .map(|s| s.rsplit('/').next().unwrap_or(s).to_string())
                    .unwrap_or_else(|| p.comm.clone())
            } else {
                p.comm.clone()
            };
            ProcessView {
                pid: p.pid,
                name,
                cmdline: p.cmdline.clone(),
                num_threads: p.num_threads,
                cpu_pct_host: process_history
                    .recent
                    .back()
                    .map(|r| r.cpu_pct_host)
                    .unwrap_or(0.0),
                rss_bytes: p.vm_rss_kb * 1024,
                vm_size_bytes: p.vm_size_kb * 1024,
                fd_count: p.fd_count,
                socket_count: p.socket_count,
                recent: process_history.recent.clone(),
            }
        }
        None => snapshot.load().process.clone(),
    };

    let mut threads: Vec<ThreadView> = histories
        .iter()
        .filter_map(|(_tid, h)| {
            let cur = h.prev.as_ref()?;
            if let Some(re) = filter_re {
                if !re.is_match(&cur.name) {
                    return None;
                }
            }
            let last = h.recent.back();
            let cpu_pct = last.map(|r| r.cpu_pct).unwrap_or(0.0);
            let sys_pct = last.map(|r| r.sys_pct).unwrap_or(0.0);
            let ctxsw_v = last.map(|r| r.ctxsw_vol_per_s).unwrap_or(0.0);
            let ctxsw_iv = last.map(|r| r.ctxsw_invol_per_s).unwrap_or(0.0);
            let (mean_cpu, cpu_p50, cpu_p95, cpu_p99, cpu_max) = compute_percentiles(&h.recent);
            let iowait_pct = compute_iowait_pct(&h.recent);
            let syscall_name = cur.syscall.map(|s| crate::sampler::syscall_table::name(s.nr));

            Some(ThreadView {
                tid: cur.tid,
                name: cur.name.clone(),
                state: cur.state,
                processor: cur.processor,
                cpu_pct,
                sys_pct,
                mean_cpu_pct: mean_cpu,
                ctxsw_vol_per_s: ctxsw_v,
                ctxsw_invol_per_s: ctxsw_iv,
                iowait_pct,
                wchan: cur.wchan.clone(),
                syscall_name,
                syscall_args: cur.syscall.map(|s| s.args).unwrap_or([0; 6]),
                freeze: h.freeze.clone(),
                recent: h.recent.clone(),
                transitions: h.transitions.clone(),
                cpu_p50,
                cpu_p95,
                cpu_p99,
                cpu_max,
                sched_wait_ns_per_s: h.sched_wait_ns_per_s,
                starttime_ticks: cur.starttime_ticks,
            })
        })
        .collect();

    threads.sort_by(|a, b| {
        let af = a.freeze.is_some();
        let bf = b.freeze.is_some();
        match (af, bf) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            (true, true) => {
                let asince = a.freeze.as_ref().map(|f| f.since).unwrap();
                let bsince = b.freeze.as_ref().map(|f| f.since).unwrap();
                asince.cmp(&bsince)
            }
            (false, false) => b
                .mean_cpu_pct
                .partial_cmp(&a.mean_cpu_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.tid.cmp(&b.tid)),
        }
    });

    let cur = snapshot.load_full();
    let (status, status_until) = match cur.status_until {
        Some(until) if Instant::now() < until => (cur.status.clone(), Some(until)),
        _ => (None, None),
    };

    let new_snap = Snapshot {
        process,
        threads,
        paused: *paused_rx.borrow(),
        interval: *interval_rx.borrow(),
        filter: filter.map(|s| s.to_string()),
        status,
        status_until,
        caps: cur.caps,
        target_gone,
    };
    snapshot.store(Arc::new(new_snap));
}

fn compute_percentiles(recent: &VecDeque<RecentPoint>) -> (f32, f32, f32, f32, f32) {
    if recent.is_empty() {
        return (0.0, 0.0, 0.0, 0.0, 0.0);
    }
    let mut values: Vec<f32> = recent.iter().map(|p| p.cpu_pct).collect();
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pick = |q: f32| -> f32 {
        let idx = ((values.len() - 1) as f32 * q).round() as usize;
        values[idx.min(values.len() - 1)]
    };
    (
        mean,
        pick(0.50),
        pick(0.95),
        pick(0.99),
        *values.last().unwrap(),
    )
}

fn compute_iowait_pct(recent: &VecDeque<RecentPoint>) -> f32 {
    if recent.is_empty() {
        return 0.0;
    }
    let d = recent
        .iter()
        .filter(|r| matches!(r.state, ThreadState::Disk))
        .count();
    100.0 * d as f32 / recent.len() as f32
}

fn clk_tck() -> f64 {
    let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if v <= 0 {
        100.0
    } else {
        v as f64
    }
}

fn num_cores() -> f64 {
    let v = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    if v <= 0 {
        1.0
    } else {
        v as f64
    }
}
