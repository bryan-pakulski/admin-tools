use anyhow::{bail, Result};

use crate::cli::Args;

/// How gdbscope should connect to the target.
#[derive(Debug, Clone)]
pub enum TargetMode {
    /// Attach to an already-running process.
    AttachPid(u32),
    /// Launch an executable under GDB.
    LaunchExec { path: String, args: Vec<String> },
    /// Load a core dump produced by an executable.
    CoreDump { exec_path: String, core_path: String },
    /// Connect to a remote GDB server (e.g. `host:port`).
    Remote(String),
}

/// Fully validated runtime configuration, derived from CLI arguments.
#[derive(Debug, Clone)]
pub struct Config {
    pub target: TargetMode,
    pub gdb_path: String,
    pub redraw_hz: u32,
    pub debug: bool,
}

impl Config {
    /// Build a [`Config`] from parsed CLI [`Args`], validating that exactly one
    /// target mode has been specified.
    pub fn from_args(args: Args) -> Result<Self> {
        let mode_count = [
            args.pid.is_some(),
            args.executable.is_some(),
            args.remote.is_some(),
        ]
        .iter()
        .filter(|&&b| b)
        .count();

        if mode_count == 0 {
            bail!(
                "No target specified. Use one of:\n  \
                 -e / --exec <path>      Launch a program\n  \
                 -p / --pid <pid>        Attach to a running process\n  \
                 -r / --remote <addr>    Connect to a GDB server"
            );
        }

        if mode_count > 1 {
            bail!(
                "Conflicting targets: specify exactly one of --exec, --pid, or --remote"
            );
        }

        let target = if let Some(pid) = args.pid {
            if !args.args.is_empty() {
                bail!("Program arguments (--) are not valid when attaching by PID");
            }
            TargetMode::AttachPid(pid)
        } else if let Some(exec_path) = args.executable {
            if let Some(core_path) = args.core {
                if !args.args.is_empty() {
                    bail!("Program arguments (--) are not valid when opening a core dump");
                }
                TargetMode::CoreDump {
                    exec_path,
                    core_path,
                }
            } else {
                TargetMode::LaunchExec {
                    path: exec_path,
                    args: args.args,
                }
            }
        } else if let Some(remote) = args.remote {
            if !args.args.is_empty() {
                bail!("Program arguments (--) are not valid with --remote");
            }
            TargetMode::Remote(remote)
        } else {
            // Unreachable given the mode_count checks above, but be explicit.
            bail!("No target specified");
        };

        if args.redraw_hz == 0 {
            bail!("--redraw-hz must be at least 1");
        }

        Ok(Self {
            target,
            gdb_path: args.gdb_path,
            redraw_hz: args.redraw_hz,
            debug: args.debug,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Args;
    use clap::Parser;

    fn args_from(input: &[&str]) -> Args {
        Args::parse_from(input)
    }

    #[test]
    fn exec_mode() {
        let cfg = Config::from_args(args_from(&["g", "-e", "./prog", "--", "x"]))
            .expect("should succeed");
        match &cfg.target {
            TargetMode::LaunchExec { path, args } => {
                assert_eq!(path, "./prog");
                assert_eq!(args, &["x"]);
            }
            other => panic!("expected LaunchExec, got {other:?}"),
        }
    }

    #[test]
    fn pid_mode() {
        let cfg = Config::from_args(args_from(&["g", "-p", "99"])).expect("should succeed");
        match cfg.target {
            TargetMode::AttachPid(pid) => assert_eq!(pid, 99),
            other => panic!("expected AttachPid, got {other:?}"),
        }
    }

    #[test]
    fn core_mode() {
        let cfg = Config::from_args(args_from(&["g", "-e", "./a.out", "-c", "core.42"]))
            .expect("should succeed");
        match &cfg.target {
            TargetMode::CoreDump {
                exec_path,
                core_path,
            } => {
                assert_eq!(exec_path, "./a.out");
                assert_eq!(core_path, "core.42");
            }
            other => panic!("expected CoreDump, got {other:?}"),
        }
    }

    #[test]
    fn remote_mode() {
        let cfg =
            Config::from_args(args_from(&["g", "-r", "host:1234"])).expect("should succeed");
        match &cfg.target {
            TargetMode::Remote(addr) => assert_eq!(addr, "host:1234"),
            other => panic!("expected Remote, got {other:?}"),
        }
    }

    #[test]
    fn no_target_is_error() {
        let result = Config::from_args(args_from(&["g"]));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("No target specified"), "got: {msg}");
    }

    #[test]
    fn conflicting_targets_is_error() {
        let result = Config::from_args(args_from(&["g", "-p", "1", "-r", "host:1"]));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Conflicting"), "got: {msg}");
    }

    #[test]
    fn pid_with_trailing_args_is_error() {
        let result = Config::from_args(args_from(&["g", "-p", "1", "--", "arg"]));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not valid when attaching"), "got: {msg}");
    }

    #[test]
    fn core_with_trailing_args_is_error() {
        let result = Config::from_args(args_from(&["g", "-e", "./a", "-c", "c", "--", "x"]));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not valid when opening a core"), "got: {msg}");
    }

    #[test]
    fn remote_with_trailing_args_is_error() {
        let result = Config::from_args(args_from(&["g", "-r", "h:1", "--", "x"]));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("not valid with --remote"), "got: {msg}");
    }

    #[test]
    fn zero_redraw_hz_is_error() {
        let result = Config::from_args(args_from(&["g", "-p", "1", "--redraw-hz", "0"]));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("redraw-hz"), "got: {msg}");
    }

    #[test]
    fn custom_gdb_path() {
        let cfg = Config::from_args(args_from(&["g", "-p", "1", "--gdb-path", "/usr/local/bin/gdb"]))
            .expect("should succeed");
        assert_eq!(cfg.gdb_path, "/usr/local/bin/gdb");
    }

    #[test]
    fn debug_flag_propagated() {
        let cfg = Config::from_args(args_from(&["g", "-p", "1", "--debug"]))
            .expect("should succeed");
        assert!(cfg.debug);
    }
}
