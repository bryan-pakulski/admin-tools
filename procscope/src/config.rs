use std::time::Duration;

use anyhow::{anyhow, Context, Result};

use crate::cli::Args;

#[derive(Debug, Clone)]
pub struct Config {
    pub pid: i32,
    pub interval: Duration,
    pub filter: Option<String>,
    pub write_csv: Option<String>,
    pub export_on_exit: bool,
    pub duration: Option<Duration>,
    pub redraw_hz: u32,
    pub no_tui: bool,
    pub debug: bool,
    pub freeze: FreezeThresholds,
    /// Initial visible capture window. `None` = show entire buffer.
    pub initial_window: Option<Duration>,
    /// Hard cap on samples per thread (memory bound).
    pub recent_cap: usize,
    /// Drop samples older than this from the buffer (wall-time bound).
    /// Together with recent_cap this makes the buffer cover a fixed time range
    /// regardless of polling rate.
    pub max_history: Duration,
    /// Initial moving-average window for list values. None = show last sample.
    pub list_avg: Option<Duration>,
}

#[derive(Debug, Clone, Copy)]
pub struct FreezeThresholds {
    pub d_state: Duration,
    pub net_wchan: Duration,
    pub no_ctxsw: Duration,
    pub cpu_divergence: Duration,
}

impl Default for FreezeThresholds {
    fn default() -> Self {
        Self {
            d_state: Duration::from_millis(500),
            net_wchan: Duration::from_millis(5000),
            no_ctxsw: Duration::from_millis(5000),
            cpu_divergence: Duration::from_millis(3000),
        }
    }
}

impl Config {
    pub fn from_args(args: Args) -> Result<Self> {
        let pid = match (args.pid, args.name.as_ref()) {
            (Some(p), _) => p,
            (None, Some(n)) => find_pid_by_comm(n)
                .with_context(|| format!("no process with comm matching {:?}", n))?,
            (None, None) => {
                return Err(anyhow!("must supply --pid <PID> or --name <SUBSTR>"));
            }
        };

        // Sanity check: PID must exist.
        if !std::path::Path::new(&format!("/proc/{}/stat", pid)).exists() {
            return Err(anyhow!("/proc/{}/stat not found — process not running?", pid));
        }

        let interval = Duration::from_millis(args.interval_ms.max(1));

        let initial_window = if args.window_secs == 0 {
            None
        } else {
            Some(Duration::from_secs(args.window_secs))
        };

        // Buffer enough history to fully populate the LARGEST window the user might
        // pick (30 minutes by default) at the current interval. We bound by BOTH
        // wall-time (max_history) and sample count (recent_cap) so runtime polling-
        // rate changes don't make the visible window shrink.
        let history_secs = args
            .history_secs
            .unwrap_or_else(|| (args.window_secs.max(15)).saturating_mul(2).max(1800));
        let interval_ms = interval.as_millis().max(1) as u64;
        let raw = (history_secs.saturating_mul(1000) / interval_ms) as usize;
        let recent_cap = raw.clamp(256, args.history_samples_max);
        let max_history = Duration::from_secs(history_secs);

        Ok(Self {
            pid,
            interval,
            filter: args.filter,
            write_csv: args.write,
            export_on_exit: args.export_on_exit,
            duration: args.duration_secs.map(Duration::from_secs),
            redraw_hz: args.redraw_hz.max(1),
            no_tui: args.no_tui || args.debug,
            debug: args.debug,
            freeze: FreezeThresholds {
                d_state: Duration::from_millis(args.freeze_d_ms),
                net_wchan: Duration::from_millis(args.freeze_netwchan_ms),
                no_ctxsw: Duration::from_millis(args.freeze_noctxsw_ms),
                cpu_divergence: Duration::from_millis(args.freeze_divergence_ms),
            },
            initial_window,
            recent_cap,
            max_history,
            list_avg: if args.list_avg_secs == 0 {
                None
            } else {
                Some(Duration::from_secs(args.list_avg_secs))
            },
        })
    }
}

fn find_pid_by_comm(substr: &str) -> Result<i32> {
    let dir = std::fs::read_dir("/proc").context("read /proc")?;
    for entry in dir.flatten() {
        let name = entry.file_name();
        let s = name.to_string_lossy();
        let Ok(pid) = s.parse::<i32>() else { continue };
        let comm_path = format!("/proc/{}/comm", pid);
        let Ok(comm) = std::fs::read_to_string(&comm_path) else { continue };
        if comm.trim().contains(substr) {
            return Ok(pid);
        }
    }
    Err(anyhow!("no /proc/<pid>/comm contained {:?}", substr))
}
