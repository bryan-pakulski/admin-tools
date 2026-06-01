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
        gdbscope -r localhost:1234"
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
}
