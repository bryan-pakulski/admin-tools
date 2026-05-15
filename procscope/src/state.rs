use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;

use crate::capabilities::Capabilities;
use crate::freeze::FreezeFlag;
use crate::sampler::ThreadState;

pub const RECENT_CAP: usize = 1024;
pub const TRANSITION_CAP: usize = 32;

#[derive(Debug, Clone, Copy)]
pub struct RecentPoint {
    pub at: Instant,
    pub cpu_pct: f32,
    pub sys_pct: f32,
    pub state: ThreadState,
    pub ctxsw_vol_per_s: f32,
    pub ctxsw_invol_per_s: f32,
    pub rchar_bps: f64,
    pub wchar_bps: f64,
    pub sched_wait_ms_per_s: f32,
}

#[derive(Debug, Clone)]
pub struct Transition {
    pub at: Instant,
    pub wall_us: i128,
    pub state: ThreadState,
    pub wchan: String,
    pub syscall_name: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub struct ThreadView {
    pub tid: i32,
    pub name: String,
    pub state: ThreadState,
    pub processor: i32,
    pub cpu_pct: f32,
    pub sys_pct: f32,
    pub mean_cpu_pct: f32,
    pub ctxsw_vol_per_s: f32,
    pub ctxsw_invol_per_s: f32,
    pub iowait_pct: f32,
    pub wchan: String,
    pub syscall_name: Option<&'static str>,
    pub syscall_args: [u64; 6],
    pub freeze: Option<FreezeFlag>,
    pub recent: VecDeque<RecentPoint>,
    pub transitions: VecDeque<Transition>,
    pub cpu_p50: f32,
    pub cpu_p95: f32,
    pub cpu_p99: f32,
    pub cpu_max: f32,
    pub sched_wait_ns_per_s: f64,
    pub starttime_ticks: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct ProcessPoint {
    pub at: Instant,
    pub cpu_pct_host: f32,
    pub rss_bytes: u64,
    pub fd_count: u32,
}

#[derive(Debug, Clone)]
pub struct ProcessView {
    pub pid: i32,
    pub name: String,
    pub cmdline: String,
    pub num_threads: u32,
    pub cpu_pct_host: f32,
    pub rss_bytes: u64,
    pub vm_size_bytes: u64,
    pub fd_count: u32,
    pub socket_count: u32,
    pub recent: VecDeque<ProcessPoint>,
}

impl ProcessView {
    pub fn empty(pid: i32) -> Self {
        Self {
            pid,
            name: String::new(),
            cmdline: String::new(),
            num_threads: 0,
            cpu_pct_host: 0.0,
            rss_bytes: 0,
            vm_size_bytes: 0,
            fd_count: 0,
            socket_count: 0,
            recent: VecDeque::with_capacity(RECENT_CAP),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    pub process: ProcessView,
    pub threads: Vec<ThreadView>,
    pub paused: bool,
    pub interval: Duration,
    pub filter: Option<String>,
    pub status: Option<String>,
    pub status_until: Option<Instant>,
    pub caps: Capabilities,
    pub target_gone: bool,
}

impl Snapshot {
    pub fn empty(pid: i32, interval: Duration, caps: Capabilities) -> Self {
        Self {
            process: ProcessView::empty(pid),
            threads: Vec::new(),
            paused: false,
            interval,
            filter: None,
            status: None,
            status_until: None,
            caps,
            target_gone: false,
        }
    }
}

pub type SharedSnapshot = Arc<ArcSwap<Snapshot>>;

pub fn new_shared(pid: i32, interval: Duration, caps: Capabilities) -> SharedSnapshot {
    Arc::new(ArcSwap::from_pointee(Snapshot::empty(pid, interval, caps)))
}
