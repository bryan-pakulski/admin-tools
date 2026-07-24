/// GDB process lifecycle management.
///
/// Spawns `gdb --interpreter=mi3 -q` with the appropriate arguments for the
/// chosen [`TargetMode`], and provides async helpers for line-oriented I/O over
/// the child's stdin / stdout / stderr.

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};

use crate::config::TargetMode;

// ---------------------------------------------------------------------------
// GdbProcess
// ---------------------------------------------------------------------------

pub struct GdbProcess {
    pub child: Child,
    pub stdin: ChildStdin,
    pub stdout: Lines<BufReader<ChildStdout>>,
    pub stderr: Lines<BufReader<ChildStderr>>,
}

impl GdbProcess {
    /// Spawn a new GDB subprocess configured for the given [`TargetMode`].
    ///
    /// The process is started with `--interpreter=mi3 -q` so that all output
    /// follows the MI protocol and the startup banner is suppressed.
    pub async fn spawn(gdb_path: &str, target: &TargetMode) -> Result<Self> {
        let mut cmd = Command::new(gdb_path);
        cmd.arg("--interpreter=mi3").arg("-q");

        // Allow gdb to auto-load helper scripts shipped alongside object files —
        // in particular CPython's `pythonX.Y-gdb.py`, which defines the
        // `py-bt` / `py-list` / `py-locals` commands gdbscope drives for the
        // Python view.  These `-iex` (initial eval) commands run *before* the
        // inferior / core is loaded, which is required because gdb decides
        // whether to auto-load a script at the moment the owning object file
        // (libpython) is read — for attach and core targets that happens at
        // launch, before any MI command could take effect.
        cmd.arg("-iex").arg("set auto-load safe-path /");
        cmd.arg("-iex").arg("set auto-load python-scripts on");

        match target {
            TargetMode::AttachPid(pid) => {
                cmd.arg("-p").arg(pid.to_string());
            }
            TargetMode::LaunchExec { path, .. } => {
                // Extra args are passed later via `-exec-run`.
                cmd.arg(path);
            }
            TargetMode::CoreDump {
                exec_path,
                core_path,
            } => {
                cmd.arg(exec_path).arg(core_path);
            }
            TargetMode::Remote(_) => {
                // For remote targets we launch bare GDB and send the
                // `-target-select remote` command over MI once the process is
                // ready.
            }
        }

        cmd.stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn gdb at `{gdb_path}`"))?;

        let stdin = child
            .stdin
            .take()
            .context("failed to open stdin pipe to gdb")?;

        let stdout = child
            .stdout
            .take()
            .context("failed to open stdout pipe from gdb")?;

        let stderr = child
            .stderr
            .take()
            .context("failed to open stderr pipe from gdb")?;

        let stdout_lines = BufReader::new(stdout).lines();
        let stderr_lines = BufReader::new(stderr).lines();

        Ok(Self {
            child,
            stdin,
            stdout: stdout_lines,
            stderr: stderr_lines,
        })
    }

    /// Write a raw MI command string to GDB's stdin and flush.
    ///
    /// The caller is responsible for including the trailing newline (the
    /// [`MiCommandBuilder`](super::mi_command::MiCommandBuilder) methods
    /// already append one).
    pub async fn send(&mut self, command: &str) -> Result<()> {
        self.stdin
            .write_all(command.as_bytes())
            .await
            .context("failed to write to gdb stdin")?;
        self.stdin
            .flush()
            .await
            .context("failed to flush gdb stdin")?;
        Ok(())
    }

    /// Read the next line from GDB's stdout.
    ///
    /// Returns `Ok(None)` when the stream is closed (i.e. GDB has exited).
    pub async fn next_line(&mut self) -> Result<Option<String>> {
        self.stdout
            .next_line()
            .await
            .context("error reading gdb stdout")
    }

    /// Read the next line from GDB's stderr.
    ///
    /// Returns `Ok(None)` when the stream is closed.
    pub async fn next_stderr_line(&mut self) -> Result<Option<String>> {
        self.stderr
            .next_line()
            .await
            .context("error reading gdb stderr")
    }

    /// Non-blocking check for whether the child process has exited.
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        self.child.try_wait().map_err(Into::into)
    }
}
