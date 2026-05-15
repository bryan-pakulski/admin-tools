use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "procscope",
    about = "Non-invasive per-thread Linux process monitor (TUI timeline)",
    version
)]
pub struct Args {
    /// PID of the process to monitor (mutually exclusive with --name).
    #[arg(short = 'p', long)]
    pub pid: Option<i32>,

    /// Pick the first PID whose `/proc/<pid>/comm` contains this substring.
    #[arg(short = 'n', long, conflicts_with = "pid")]
    pub name: Option<String>,

    /// Sampling interval in milliseconds. Adjustable at runtime with +/-.
    /// Floor is 1ms — be aware that sub-CLK_TCK intervals (~10ms on most kernels) make
    /// per-sample CPU% noisy, but state/wchan/syscall observations stay meaningful.
    #[arg(long, default_value_t = 100)]
    pub interval_ms: u64,

    /// Initial visible capture window, in seconds. 0 = "all". Adjustable at runtime with `w`.
    #[arg(long, default_value_t = 60)]
    pub window_secs: u64,

    /// How many seconds of history to keep in the sample buffer. Caps memory.
    /// Default = max(window_secs * 2, 1800) — enough to cover the largest
    /// adjustable window (30m). At very low intervals the actual cap is also
    /// bounded by --history-samples-max to prevent OOM.
    #[arg(long)]
    pub history_secs: Option<u64>,

    /// Hard upper bound on sample buffer length per thread (prevents OOM at sub-ms intervals).
    #[arg(long, default_value_t = 200_000)]
    pub history_samples_max: usize,

    /// Moving-average window (seconds) for values shown in the thread list.
    /// 0 = show instantaneous last-sample values. Adjustable at runtime with `a`.
    /// Smoothing prevents jitter at high polling rates without losing fidelity in
    /// the detail charts (which keep raw samples).
    #[arg(long, default_value_t = 1)]
    pub list_avg_secs: u64,

    /// Restrict displayed threads to those whose name matches this regex.
    #[arg(short = 'f', long)]
    pub filter: Option<String>,

    /// Stream every sample to a CSV at this path.
    #[arg(short = 'w', long)]
    pub write: Option<String>,

    /// Write a snapshot CSV on exit.
    #[arg(long)]
    pub export_on_exit: bool,

    /// Auto-exit after N seconds (useful with --no-tui).
    #[arg(long)]
    pub duration_secs: Option<u64>,

    /// TUI redraw rate in Hz.
    #[arg(long, default_value_t = 20)]
    pub redraw_hz: u32,

    /// Disable the TUI and dump samples to stdout.
    #[arg(long)]
    pub no_tui: bool,

    /// Verbose tracing to stderr (also disables TUI).
    #[arg(long)]
    pub debug: bool,

    /// D-state freeze threshold in milliseconds.
    #[arg(long, default_value_t = 500)]
    pub freeze_d_ms: u64,

    /// Net-wchan freeze threshold in milliseconds (sk_wait_data, tcp_recvmsg, ...).
    #[arg(long, default_value_t = 5000)]
    pub freeze_netwchan_ms: u64,

    /// "No context switch" freeze threshold in milliseconds.
    #[arg(long, default_value_t = 5000)]
    pub freeze_noctxsw_ms: u64,

    /// CPU divergence (peers active, this thread idle) threshold in milliseconds.
    #[arg(long, default_value_t = 3000)]
    pub freeze_divergence_ms: u64,
}
