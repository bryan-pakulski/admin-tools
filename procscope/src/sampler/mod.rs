pub mod cpu_total;
pub mod proc_reader;
pub mod syscall_table;
pub mod thread_reader;

use std::time::{Duration, Instant, SystemTime};

use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

pub use cpu_total::CpuTotal;
pub use proc_reader::ProcessSample;
pub use thread_reader::{SchedStat, SyscallInfo, ThreadIo, ThreadSample, ThreadState};

#[derive(Debug)]
pub struct SampleBatch {
    pub at: Instant,
    pub wall: SystemTime,
    pub process: Option<ProcessSample>,
    pub cpu_total: Option<CpuTotal>,
    pub threads: Vec<ThreadSample>,
    /// Set true when the target process has exited; this is the final batch.
    pub target_gone: bool,
}

impl SampleBatch {
    pub fn target_gone() -> Self {
        Self {
            at: Instant::now(),
            wall: SystemTime::now(),
            process: None,
            cpu_total: None,
            threads: Vec::new(),
            target_gone: true,
        }
    }
}

pub struct SamplerHandles {
    pub samples_rx: mpsc::Receiver<SampleBatch>,
    pub paused_tx: watch::Sender<bool>,
    pub interval_tx: watch::Sender<Duration>,
}

pub fn spawn(pid: i32, initial_interval: Duration) -> SamplerHandles {
    let (samples_tx, samples_rx) = mpsc::channel(64);
    let (paused_tx, paused_rx) = watch::channel(false);
    let (interval_tx, interval_rx) = watch::channel(initial_interval);

    tokio::spawn(run(pid, samples_tx, paused_rx, interval_rx));

    SamplerHandles {
        samples_rx,
        paused_tx,
        interval_tx,
    }
}

async fn run(
    pid: i32,
    tx: mpsc::Sender<SampleBatch>,
    paused_rx: watch::Receiver<bool>,
    interval_rx: watch::Receiver<Duration>,
) {
    let mut last_interval = *interval_rx.borrow();
    let mut tick = tokio::time::interval(last_interval);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let original_starttime = match proc_reader::read_starttime(pid) {
        Ok(s) => Some(s),
        Err(_) => None,
    };

    loop {
        tick.tick().await;

        let cur = *interval_rx.borrow();
        if cur != last_interval {
            tick = tokio::time::interval(cur);
            tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
            last_interval = cur;
        }
        if *paused_rx.borrow() {
            continue;
        }

        // Detect PID reuse cheaply by re-reading starttime; if it diverges, treat as gone.
        if let Some(orig) = original_starttime {
            match proc_reader::read_starttime(pid) {
                Ok(s) if s != orig => {
                    let _ = tx.send(SampleBatch::target_gone()).await;
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let _ = tx.send(SampleBatch::target_gone()).await;
                    return;
                }
                _ => {}
            }
        }

        let process = match proc_reader::read_process(pid) {
            Ok(p) => p,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let _ = tx.send(SampleBatch::target_gone()).await;
                return;
            }
            Err(_) => continue,
        };

        let cpu_total = cpu_total::read().ok();

        let tids = thread_reader::list_tids(pid).unwrap_or_default();
        let mut threads = Vec::with_capacity(tids.len());
        for tid in tids {
            if let Ok(s) = thread_reader::read_thread(pid, tid) {
                threads.push(s);
            }
        }

        let batch = SampleBatch {
            at: Instant::now(),
            wall: SystemTime::now(),
            process: Some(process),
            cpu_total,
            threads,
            target_gone: false,
        };

        if tx.send(batch).await.is_err() {
            return;
        }
    }
}
