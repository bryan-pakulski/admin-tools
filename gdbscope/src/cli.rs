use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "gdbscope",
    about = "Developer-friendly GDB TUI wrapper",
    version,
    after_help = "EXAMPLES:\n  \
        gdbscope -e ./my_program -- arg1 arg2\n  \
        gdbscope -p 12345\n  \
        gdbscope -e ./my_program -c core.1234\n  \
        gdbscope -e ./my_program -c core.1234 -s /path/to/src\n  \
        gdbscope -r localhost:1234\n\n  \
        Off-site core dump analysis (different OS/glibc):\n  \
        gdbscope-collect -e /opt/app/server -c /var/cores/core.123\n  \
        scp coredump-server-*.tar.gz devbox:~/\n  \
        tar xf coredump-server-*.tar.gz\n  \
        gdbscope -e sysroot/opt/app/server -c sysroot/core.123 --sysroot sysroot"
)]
pub struct Args {
    /// Attach to a running process by PID
    #[arg(short = 'p', long)]
    pub pid: Option<u32>,

    /// Launch a program
    #[arg(short = 'e', long = "exec")]
    pub executable: Option<String>,

    /// Program arguments (when using --exec)
    #[arg(last = true)]
    pub args: Vec<String>,

    /// Open a core dump (requires --exec)
    #[arg(short = 'c', long, requires = "executable")]
    pub core: Option<String>,

    /// Connect to remote GDB server (host:port)
    #[arg(short = 'r', long = "remote")]
    pub remote: Option<String>,

    /// Path to GDB binary
    #[arg(long, default_value = "gdb")]
    pub gdb_path: String,

    /// TUI redraw rate in Hz
    #[arg(long, default_value_t = 30)]
    pub redraw_hz: u32,

    /// Enable debug tracing to stderr (disables TUI)
    #[arg(long)]
    pub debug: bool,

    /// Maximum number of recorded debug states to keep (0 = disable recording)
    #[arg(long, default_value_t = 1000)]
    pub record_max: usize,

    /// Maximum age of recorded states in seconds (0 = unlimited)
    #[arg(long, default_value_t = 300)]
    pub record_secs: u64,

    /// Max steps to capture when trace-continuing (F6) to a breakpoint
    #[arg(long, default_value_t = 500)]
    pub trace_depth: usize,

    /// Add directories to GDB's source file search path (repeatable)
    #[arg(short = 's', long = "source-dir")]
    pub source_dirs: Vec<String>,

    /// Set GDB sysroot for resolving shared libraries from a different system
    /// (e.g. analysing a CentOS 7 core dump on an OL9 dev machine)
    #[arg(long)]
    pub sysroot: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_exec_with_args() {
        let args = Args::parse_from(["gdbscope", "-e", "./my_prog", "--", "a", "b"]);
        assert_eq!(args.executable.as_deref(), Some("./my_prog"));
        assert_eq!(args.args, vec!["a", "b"]);
        assert!(args.pid.is_none());
    }

    #[test]
    fn parse_attach_pid() {
        let args = Args::parse_from(["gdbscope", "-p", "42"]);
        assert_eq!(args.pid, Some(42));
        assert!(args.executable.is_none());
    }

    #[test]
    fn parse_remote() {
        let args = Args::parse_from(["gdbscope", "-r", "localhost:3333"]);
        assert_eq!(args.remote.as_deref(), Some("localhost:3333"));
    }

    #[test]
    fn parse_core_dump() {
        let args = Args::parse_from(["gdbscope", "-e", "./prog", "-c", "core.123"]);
        assert_eq!(args.executable.as_deref(), Some("./prog"));
        assert_eq!(args.core.as_deref(), Some("core.123"));
    }

    #[test]
    fn default_gdb_path() {
        let args = Args::parse_from(["gdbscope", "-p", "1"]);
        assert_eq!(args.gdb_path, "gdb");
    }

    #[test]
    fn default_redraw_hz() {
        let args = Args::parse_from(["gdbscope", "-p", "1"]);
        assert_eq!(args.redraw_hz, 30);
    }

    #[test]
    fn debug_flag() {
        let args = Args::parse_from(["gdbscope", "-p", "1", "--debug"]);
        assert!(args.debug);
    }

    #[test]
    fn parse_source_dirs() {
        let args = Args::parse_from([
            "gdbscope", "-e", "./prog",
            "--source-dir", "/path/to/src",
            "--source-dir", "/another/path",
        ]);
        assert_eq!(args.source_dirs, vec!["/path/to/src", "/another/path"]);
    }

    #[test]
    fn parse_source_dirs_short() {
        let args = Args::parse_from(["gdbscope", "-e", "./prog", "-s", "/path/to/src"]);
        assert_eq!(args.source_dirs, vec!["/path/to/src"]);
    }

    #[test]
    fn source_dirs_empty_by_default() {
        let args = Args::parse_from(["gdbscope", "-p", "1"]);
        assert!(args.source_dirs.is_empty());
    }

    #[test]
    fn parse_sysroot() {
        let args = Args::parse_from(["gdbscope", "-e", "./prog", "-c", "core.1", "--sysroot", "/tmp/sysroot"]);
        assert_eq!(args.sysroot.as_deref(), Some("/tmp/sysroot"));
    }

    #[test]
    fn sysroot_none_by_default() {
        let args = Args::parse_from(["gdbscope", "-p", "1"]);
        assert!(args.sysroot.is_none());
    }
}
