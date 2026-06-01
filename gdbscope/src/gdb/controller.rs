/// High-level GDB controller -- async event loop bridging the TUI and the GDB
/// process.
///
/// The controller owns the GDB child process and the MI command builder.  It
/// reads MI output, parses it, updates shared state, and translates high-level
/// [`GdbCommand`] messages from the TUI into MI command sequences.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::io::{AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use crate::config::{Config, TargetMode};
use crate::gdb::mi_command::MiCommandBuilder;
use crate::gdb::mi_parser;
use crate::gdb::mi_types::{MiBody, MiList, MiRecord, MiValue};
use crate::gdb::process::GdbProcess;
use crate::state::*;

// ---------------------------------------------------------------------------
// GdbCommand -- messages the TUI sends to the controller
// ---------------------------------------------------------------------------

/// Commands the TUI (or any other consumer) can send to the GDB controller.
#[derive(Debug)]
pub enum GdbCommand {
    Run(Vec<String>),
    Continue,
    StepOver,
    StepInto,
    StepOut,
    Interrupt,
    SelectThread(i32),
    SelectFrame(u32),
    SetBreakpoint(String),
    DeleteBreakpoint(u32),
    ToggleBreakpoint(u32),
    RefreshRegisters,
    ReadMemory { addr: u64, count: usize },
    ReadMemoryExpr { expr: String, count: usize },
    WriteMemory { addr: u64, bytes: Vec<u8> },
    Disassemble { addr: u64, count: usize },
    EvaluateExpression(String),
    AddWatch(String),
    RemoveWatch(u32),
    RawCommand(String),
    Quit,
}

// ---------------------------------------------------------------------------
// PendingKind -- what we are waiting for from GDB
// ---------------------------------------------------------------------------

/// Tracks the kind of response we expect for a given MI token so we know how
/// to interpret the result body.
#[derive(Debug)]
enum PendingKind {
    ThreadInfo,
    StackListFrames,
    StackListLocals,
    StackSelectFrame(u32),
    ThreadSelect(i32),
    BreakInsert,
    BreakDelete(u32),
    BreakEnable(u32),
    BreakDisable(u32),
    BreakList,
    RegisterNames,
    RegisterValues,
    ReadMemory,
    ReadMemoryExpr { count: usize },
    WriteMemory { addr: u64, count: usize },
    Disassemble,
    EvalExpression { watch_id: Option<u32>, expr: String },
    FileExecSymbols,
    TargetAttach,
    TargetRemote,
    TargetCore,
    ExecRun,
    ExecContinue,
    ExecStep,
    ExecNext,
    ExecFinish,
    ExecInterrupt,
    CliCommand,
}

// ---------------------------------------------------------------------------
// WatchEntry -- internal bookkeeping for watched expressions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct WatchEntry {
    id: u32,
    expression: String,
}

// ---------------------------------------------------------------------------
// GdbController
// ---------------------------------------------------------------------------

/// The async controller that owns the GDB process and bridges it to the TUI
/// via [`SharedState`].
pub struct GdbController {
    state: SharedState,
    cmd_rx: mpsc::Receiver<GdbCommand>,
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    stderr: Lines<BufReader<ChildStderr>>,
    target_mode: TargetMode,
    commands: MiCommandBuilder,
    pending: HashMap<u64, PendingKind>,
    source_cache: HashMap<String, SourceFile>,
    watch_expressions: Vec<WatchEntry>,
    next_watch_id: u32,
    register_names_loaded: bool,
}

impl GdbController {
    // -----------------------------------------------------------------------
    // Public entry point
    // -----------------------------------------------------------------------

    /// Spawn a GDB process, create the controller, and start the event loop.
    ///
    /// Returns a sender the TUI can use to issue commands and a join handle for
    /// the background task.
    pub async fn spawn(
        config: &Config,
        state: SharedState,
    ) -> Result<(mpsc::Sender<GdbCommand>, tokio::task::JoinHandle<Result<()>>)> {
        let (cmd_tx, cmd_rx) = mpsc::channel::<GdbCommand>(64);

        let process = GdbProcess::spawn(&config.gdb_path, &config.target)
            .await
            .context("failed to spawn GDB process")?;

        // Store the target executable name in the snapshot for the status bar.
        let exec_name = match &config.target {
            TargetMode::LaunchExec { path, .. } => Some(path.clone()),
            TargetMode::CoreDump { exec_path, .. } => Some(exec_path.clone()),
            TargetMode::AttachPid(pid) => Some(format!("pid:{pid}")),
            TargetMode::Remote(addr) => Some(format!("remote:{addr}")),
        };
        {
            let mut snap = (**state.load()).clone();
            snap.target_executable = exec_name;
            state.store(Arc::new(snap));
        }

        let target = config.target.clone();
        let handle = tokio::spawn(async move {
            let mut ctrl = GdbController {
                state,
                cmd_rx,
                child: process.child,
                stdin: process.stdin,
                stdout: process.stdout,
                stderr: process.stderr,
                target_mode: target.clone(),
                commands: MiCommandBuilder::new(),
                pending: HashMap::new(),
                source_cache: HashMap::new(),
                watch_expressions: Vec::new(),
                next_watch_id: 1,
                register_names_loaded: false,
            };
            ctrl.initial_setup(&target).await?;
            ctrl.run_loop().await
        });

        Ok((cmd_tx, handle))
    }

    // -----------------------------------------------------------------------
    // Initial setup based on target mode
    // -----------------------------------------------------------------------

    async fn initial_setup(&mut self, target: &TargetMode) -> Result<()> {
        match target {
            TargetMode::AttachPid(_) => {
                // GDB attaches via the -p flag.  Once attached the inferior is
                // stopped, so we can immediately query state.
                self.send_thread_info().await?;
                self.send_stack_list_frames().await?;
                self.send_stack_list_locals().await?;
                self.send_break_list().await?;
            }
            TargetMode::LaunchExec { .. } => {
                // The executable is loaded automatically via the CLI arg.
                // The user presses Run to start execution; nothing to send here.
            }
            TargetMode::CoreDump { .. } => {
                // GDB loads the exec + core via CLI args.  The target is
                // already in a "stopped" state, so we query immediately.
                self.send_thread_info().await?;
                self.send_stack_list_frames().await?;
                self.send_stack_list_locals().await?;
                self.send_break_list().await?;
            }
            TargetMode::Remote(addr) => {
                // For remote targets we launched bare GDB and now need to
                // connect via MI.  On success the dispatch_done handler will
                // trigger the auto-refresh cascade.
                let (tok, cmd) = self.commands.target_select_remote(addr);
                self.pending.insert(tok, PendingKind::TargetRemote);
                self.send_raw(&cmd).await?;
                debug!("sent target-select remote {addr}");
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Main event loop
    // -----------------------------------------------------------------------

    async fn run_loop(&mut self) -> Result<()> {
        loop {
            tokio::select! {
                // 1. Next line from GDB stdout --------------------------------
                result = self.stdout.next_line() => {
                    match result {
                        Ok(Some(line)) => {
                            debug!("gdb stdout: {}", line);
                            match mi_parser::parse_line(&line) {
                                Ok(record) => self.handle_record(record).await,
                                Err(e) => {
                                    warn!("MI parse error: {e:#} (line: {line})");
                                    self.update_snapshot(|snap| {
                                        snap.push_output(
                                            OutputKind::Error,
                                            format!("MI parse error: {e}"),
                                        );
                                    });
                                }
                            }
                        }
                        Ok(None) => {
                            debug!("GDB stdout closed");
                            self.update_snapshot(|snap| {
                                snap.target_state = TargetState::Terminated;
                                snap.push_output(
                                    OutputKind::Info,
                                    "GDB process terminated.".into(),
                                );
                            });
                            return Ok(());
                        }
                        Err(e) => {
                            error!("error reading GDB stdout: {e:#}");
                            return Err(e.into());
                        }
                    }
                }

                // 2. Next line from GDB stderr --------------------------------
                result = self.stderr.next_line() => {
                    match result {
                        Ok(Some(line)) => {
                            debug!("gdb stderr: {}", line);
                            self.update_snapshot(|snap| {
                                snap.push_output(OutputKind::Log, line.clone());
                            });
                        }
                        Ok(None) => {
                            debug!("GDB stderr closed");
                        }
                        Err(e) => {
                            warn!("error reading GDB stderr: {e:#}");
                        }
                    }
                }

                // 3. Command from the TUI -------------------------------------
                cmd = self.cmd_rx.recv() => {
                    match cmd {
                        Some(command) => {
                            debug!("received command: {command:?}");
                            if let Err(e) = self.handle_command(command).await {
                                error!("error handling command: {e:#}");
                                self.update_snapshot(|snap| {
                                    snap.push_output(
                                        OutputKind::Error,
                                        format!("Command error: {e}"),
                                    );
                                });
                            }
                        }
                        None => {
                            debug!("command channel closed, shutting down");
                            self.shutdown().await;
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Record handling
    // -----------------------------------------------------------------------

    async fn handle_record(&mut self, record: MiRecord) {
        match record {
            MiRecord::Result { token, class, body } => {
                self.handle_result_record(token, &class, &body).await;
            }
            MiRecord::AsyncExec { class, body, .. } => {
                self.handle_async_exec(&class, &body).await;
            }
            MiRecord::AsyncNotify { class, body, .. } => {
                self.handle_async_notify(&class, &body);
            }
            MiRecord::AsyncStatus { class, .. } => {
                debug!("async status: {class}");
            }
            MiRecord::StreamConsole(text) => {
                self.update_snapshot(|snap| {
                    snap.push_output(OutputKind::Console, text);
                });
            }
            MiRecord::StreamTarget(text) => {
                self.update_snapshot(|snap| {
                    snap.push_output(OutputKind::Target, text);
                });
            }
            MiRecord::StreamLog(text) => {
                self.update_snapshot(|snap| {
                    snap.push_output(OutputKind::Log, text);
                });
            }
            MiRecord::Prompt => {
                // The prompt marker is a protocol artifact; nothing to do.
            }
        }
    }

    // -- Result records -------------------------------------------------------

    async fn handle_result_record(
        &mut self,
        token: Option<u64>,
        class: &str,
        body: &[(String, MiValue)],
    ) {
        let kind = token.and_then(|tok| self.pending.remove(&tok));

        match class {
            "done" | "connected" => {
                if let Some(kind) = kind {
                    self.dispatch_done(kind, body).await;
                }
            }
            "running" => {
                self.update_snapshot(|snap| {
                    snap.target_state = TargetState::Running;
                });
            }
            "error" => {
                let msg = MiBody::get_str(body, "msg").unwrap_or("unknown error");
                error!("GDB error: {msg}");
                self.update_snapshot(|snap| {
                    snap.push_output(OutputKind::Error, format!("Error: {msg}"));
                });

                // If a watch expression eval failed, record the error on it
                if let Some(PendingKind::EvalExpression {
                    watch_id: Some(id), ..
                }) = kind
                {
                    let err_msg = msg.to_string();
                    self.update_snapshot(|snap| {
                        if let Some(w) =
                            snap.watch_expressions.iter_mut().find(|w| w.id == id)
                        {
                            w.value = String::new();
                            w.error = Some(err_msg);
                        }
                    });
                }
            }
            "exit" => {
                self.update_snapshot(|snap| {
                    snap.target_state = TargetState::Terminated;
                    snap.push_output(OutputKind::Info, "GDB session ended.".into());
                });
            }
            other => {
                debug!("unhandled result class: {other}");
            }
        }
    }

    /// Dispatch a `^done` result to the handler matched by the pending command
    /// kind.
    async fn dispatch_done(&mut self, kind: PendingKind, body: &[(String, MiValue)]) {
        match kind {
            PendingKind::ThreadInfo => {
                self.process_thread_info(body);
            }
            PendingKind::StackListFrames => {
                self.process_stack_list_frames(body).await;
            }
            PendingKind::StackListLocals => {
                self.process_stack_list_locals(body);
            }
            PendingKind::StackSelectFrame(level) => {
                // Load source for the newly selected frame.
                self.load_source_for_frame(level).await;
                if let Err(e) = self.send_stack_list_locals().await {
                    warn!("failed to refresh locals after frame select: {e:#}");
                }
            }
            PendingKind::ThreadSelect(thread_id) => {
                self.update_snapshot(|snap| {
                    snap.current_thread_id = Some(thread_id);
                    snap.current_frame_level = 0;
                });
                if let Err(e) = self.send_stack_list_frames().await {
                    warn!("failed to refresh stack after thread select: {e:#}");
                }
                if let Err(e) = self.send_stack_list_locals().await {
                    warn!("failed to refresh locals after thread select: {e:#}");
                }
            }
            PendingKind::BreakInsert => {
                self.process_break_insert(body);
            }
            PendingKind::BreakDelete(num) => {
                self.update_snapshot(|snap| {
                    snap.breakpoints.retain(|bp| bp.number != num);
                });
            }
            PendingKind::BreakEnable(num) => {
                self.update_snapshot(|snap| {
                    if let Some(bp) =
                        snap.breakpoints.iter_mut().find(|b| b.number == num)
                    {
                        bp.enabled = true;
                    }
                });
            }
            PendingKind::BreakDisable(num) => {
                self.update_snapshot(|snap| {
                    if let Some(bp) =
                        snap.breakpoints.iter_mut().find(|b| b.number == num)
                    {
                        bp.enabled = false;
                    }
                });
            }
            PendingKind::BreakList => {
                self.process_break_list(body);
            }
            PendingKind::RegisterNames => {
                self.process_register_names(body);
            }
            PendingKind::RegisterValues => {
                self.process_register_values(body);
            }
            PendingKind::ReadMemory => {
                self.process_read_memory(body);
            }
            PendingKind::ReadMemoryExpr { count } => {
                // The result is the evaluated pointer value.
                let value = MiBody::get_str(body, "value").unwrap_or("");
                if let Some(addr) = parse_hex_u64(value) {
                    self.update_snapshot(|snap| {
                        snap.memory_address = addr;
                    });
                    if let Err(e) = self.send_read_memory(addr, count).await {
                        warn!("failed to read memory at evaluated address {addr:#x}: {e:#}");
                    }
                } else {
                    self.update_snapshot(|snap| {
                        snap.push_output(
                            OutputKind::Error,
                            format!("Could not resolve to address: {value}"),
                        );
                    });
                }
            }
            PendingKind::WriteMemory { addr, count } => {
                // Re-read the memory region after writing so the view updates.
                if let Err(e) = self.send_read_memory(addr, count).await {
                    warn!("failed to refresh memory after write: {e:#}");
                }
            }
            PendingKind::Disassemble => {
                self.process_disassemble(body);
            }
            PendingKind::EvalExpression { watch_id, expr } => {
                self.process_eval_expression(watch_id, &expr, body);
            }
            PendingKind::FileExecSymbols => {
                debug!("file-exec-and-symbols complete");
            }
            PendingKind::TargetAttach => {
                debug!("target attach complete");
                self.update_snapshot(|snap| {
                    snap.target_state = TargetState::Stopped;
                    snap.push_output(OutputKind::Info, "Attached to target.".into());
                });
                if let Err(e) = self.auto_refresh_on_stop().await {
                    warn!("auto-refresh after attach failed: {e:#}");
                }
            }
            PendingKind::TargetRemote => {
                debug!("target remote connected");
                self.update_snapshot(|snap| {
                    snap.target_state = TargetState::Stopped;
                    snap.push_output(
                        OutputKind::Info,
                        "Connected to remote target.".into(),
                    );
                });
                if let Err(e) = self.auto_refresh_on_stop().await {
                    warn!("auto-refresh after remote connect failed: {e:#}");
                }
            }
            PendingKind::TargetCore => {
                debug!("core file loaded");
                self.update_snapshot(|snap| {
                    snap.target_state = TargetState::Stopped;
                    snap.push_output(OutputKind::Info, "Core file loaded.".into());
                });
                if let Err(e) = self.auto_refresh_on_stop().await {
                    warn!("auto-refresh after core load failed: {e:#}");
                }
            }
            PendingKind::ExecRun
            | PendingKind::ExecContinue
            | PendingKind::ExecStep
            | PendingKind::ExecNext
            | PendingKind::ExecFinish => {
                debug!("exec command acknowledged");
            }
            PendingKind::ExecInterrupt => {
                debug!("exec-interrupt acknowledged");
            }
            PendingKind::CliCommand => {
                debug!("CLI command complete");
            }
        }
    }

    // -- Async exec records ---------------------------------------------------

    async fn handle_async_exec(&mut self, class: &str, body: &[(String, MiValue)]) {
        match class {
            "stopped" => {
                let reason = parse_stop_reason(body);
                debug!("target stopped: {reason:?}");

                // Handle exit immediately
                if let StopReason::ExitedNormally { code } = &reason {
                    let code = *code;
                    self.update_snapshot(|snap| {
                        snap.target_state = TargetState::Exited(code);
                        snap.stop_reason = Some(StopReason::ExitedNormally { code });
                        snap.push_output(
                            OutputKind::Info,
                            format!("Program exited with code {code}."),
                        );
                        snap.stack.clear();
                        snap.locals.clear();
                        snap.threads.clear();
                    });
                    return;
                }

                // Extract frame info if present
                let frame = MiBody::get(body, "frame").and_then(parse_frame);

                // Load source file for the stopped frame
                if let Some(ref f) = frame {
                    if let Some(ref fullname) = f.fullname {
                        self.maybe_load_source(fullname).await;
                    }
                }

                let source_line = frame.as_ref().and_then(|f| f.line);

                // Build a human-readable status message
                let status: String = match &reason {
                    StopReason::BreakpointHit { number } => {
                        format!("Breakpoint {number} hit")
                    }
                    StopReason::StepFinished => "Step finished".into(),
                    StopReason::SignalReceived { name, meaning } => {
                        format!("Signal {name}: {meaning}")
                    }
                    StopReason::FunctionFinished => "Function finished".into(),
                    StopReason::Watchpoint { number } => {
                        format!("Watchpoint {number} triggered")
                    }
                    StopReason::ExitedNormally { code } => {
                        format!("Exited with code {code}")
                    }
                    StopReason::Unknown(s) => {
                        if s.is_empty() {
                            "Stopped".into()
                        } else {
                            format!("Stopped: {s}")
                        }
                    }
                };

                self.update_snapshot(|snap| {
                    snap.target_state = TargetState::Stopped;
                    snap.stop_reason = Some(reason);
                    snap.source_line = source_line;
                    snap.status_message = Some(status);
                });

                // Auto-refresh cascade
                if let Err(e) = self.auto_refresh_on_stop().await {
                    warn!("auto-refresh on stop failed: {e:#}");
                }
            }
            "running" => {
                self.update_snapshot(|snap| {
                    snap.target_state = TargetState::Running;
                    snap.stop_reason = None;
                    snap.locals.clear();
                    snap.stack.clear();
                    snap.status_message = Some("Running...".into());
                });
            }
            other => {
                debug!("unhandled async exec class: {other}");
            }
        }
    }

    // -- Async notify records -------------------------------------------------

    fn handle_async_notify(&mut self, class: &str, body: &[(String, MiValue)]) {
        match class {
            "thread-created" => {
                let id = MiBody::get_str(body, "id")
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(-1);
                debug!("thread created: {id}");
                self.update_snapshot(|snap| {
                    snap.push_output(OutputKind::Info, format!("Thread {id} created."));
                });
            }
            "thread-exited" => {
                let id = MiBody::get_str(body, "id")
                    .and_then(|s| s.parse::<i32>().ok())
                    .unwrap_or(-1);
                debug!("thread exited: {id}");
                self.update_snapshot(|snap| {
                    snap.threads.retain(|t| t.id != id);
                    snap.push_output(OutputKind::Info, format!("Thread {id} exited."));
                });
            }
            "thread-group-started" => {
                debug!("thread group started");
            }
            "thread-group-exited" => {
                let exit_code = MiBody::get_str(body, "exit-code").unwrap_or("?");
                debug!("thread group exited with code {exit_code}");
                self.update_snapshot(|snap| {
                    snap.push_output(
                        OutputKind::Info,
                        format!("Inferior exited (code {exit_code})."),
                    );
                });
            }
            "breakpoint-modified" => {
                if let Some(bkpt_val) = MiBody::get(body, "bkpt") {
                    if let Some(bp) = parse_breakpoint(bkpt_val) {
                        self.update_snapshot(|snap| {
                            if let Some(existing) = snap
                                .breakpoints
                                .iter_mut()
                                .find(|b| b.number == bp.number)
                            {
                                *existing = bp;
                            }
                        });
                    }
                }
            }
            "library-loaded" | "library-unloaded" => {
                let lib = MiBody::get_str(body, "id")
                    .or_else(|| MiBody::get_str(body, "target-name"))
                    .unwrap_or("?");
                debug!("{class}: {lib}");
            }
            "thread-group-added" | "cmd-param-changed" | "memory-changed" => {
                debug!("notify: {class}");
            }
            other => {
                debug!("unhandled async notify: {other}");
            }
        }
    }

    // -----------------------------------------------------------------------
    // Command handling -- translate GdbCommand to MI sequences
    // -----------------------------------------------------------------------

    async fn handle_command(&mut self, cmd: GdbCommand) -> Result<()> {
        match cmd {
            GdbCommand::Run(args) => {
                let (tok, mi) = self.commands.exec_run(&args);
                self.pending.insert(tok, PendingKind::ExecRun);
                self.send_raw(&mi).await?;
            }
            GdbCommand::Continue => {
                let (tok, mi) = self.commands.exec_continue();
                self.pending.insert(tok, PendingKind::ExecContinue);
                self.send_raw(&mi).await?;
            }
            GdbCommand::StepOver => {
                let (tok, mi) = self.commands.exec_next();
                self.pending.insert(tok, PendingKind::ExecNext);
                self.send_raw(&mi).await?;
            }
            GdbCommand::StepInto => {
                let (tok, mi) = self.commands.exec_step();
                self.pending.insert(tok, PendingKind::ExecStep);
                self.send_raw(&mi).await?;
            }
            GdbCommand::StepOut => {
                let (tok, mi) = self.commands.exec_finish();
                self.pending.insert(tok, PendingKind::ExecFinish);
                self.send_raw(&mi).await?;
            }
            GdbCommand::Interrupt => {
                let (tok, mi) = self.commands.exec_interrupt();
                self.pending.insert(tok, PendingKind::ExecInterrupt);
                self.send_raw(&mi).await?;
            }
            GdbCommand::SelectThread(id) => {
                let (tok, mi) = self.commands.thread_select(id);
                self.pending.insert(tok, PendingKind::ThreadSelect(id));
                self.send_raw(&mi).await?;
            }
            GdbCommand::SelectFrame(level) => {
                let (tok, mi) = self.commands.stack_select_frame(level);
                self.pending.insert(tok, PendingKind::StackSelectFrame(level));
                self.send_raw(&mi).await?;
                self.update_snapshot(|snap| {
                    snap.current_frame_level = level;
                });
            }
            GdbCommand::SetBreakpoint(location) => {
                let (tok, mi) = self.commands.break_insert(&location);
                self.pending.insert(tok, PendingKind::BreakInsert);
                self.send_raw(&mi).await?;
            }
            GdbCommand::DeleteBreakpoint(num) => {
                let (tok, mi) = self.commands.break_delete(num);
                self.pending.insert(tok, PendingKind::BreakDelete(num));
                self.send_raw(&mi).await?;
            }
            GdbCommand::ToggleBreakpoint(num) => {
                let currently_enabled = {
                    let snap = self.state.load();
                    snap.breakpoints
                        .iter()
                        .find(|bp| bp.number == num)
                        .map(|bp| bp.enabled)
                        .unwrap_or(true)
                };
                if currently_enabled {
                    let (tok, mi) = self.commands.break_disable(num);
                    self.pending.insert(tok, PendingKind::BreakDisable(num));
                    self.send_raw(&mi).await?;
                } else {
                    let (tok, mi) = self.commands.break_enable(num);
                    self.pending.insert(tok, PendingKind::BreakEnable(num));
                    self.send_raw(&mi).await?;
                }
            }
            GdbCommand::RefreshRegisters => {
                if !self.register_names_loaded {
                    let (tok, mi) = self.commands.data_list_register_names();
                    self.pending.insert(tok, PendingKind::RegisterNames);
                    self.send_raw(&mi).await?;
                }
                let (tok, mi) = self.commands.data_list_register_values("x");
                self.pending.insert(tok, PendingKind::RegisterValues);
                self.send_raw(&mi).await?;
            }
            GdbCommand::ReadMemory { addr, count } => {
                let (tok, mi) = self.commands.data_read_memory_bytes(addr, count);
                self.pending.insert(tok, PendingKind::ReadMemory);
                self.send_raw(&mi).await?;
                self.update_snapshot(|snap| {
                    snap.memory_address = addr;
                });
            }
            GdbCommand::ReadMemoryExpr { expr, count } => {
                // Evaluate the expression (e.g. "&my_var") to get a pointer,
                // then read memory at the resulting address.
                let (tok, mi) = self.commands.data_evaluate_expression(&format!("(void*)({})", expr));
                self.pending.insert(tok, PendingKind::ReadMemoryExpr { count });
                self.send_raw(&mi).await?;
            }
            GdbCommand::WriteMemory { addr, bytes } => {
                let snap = self.state.load();
                let reload_addr = snap.memory_address;
                let reload_count = snap.memory.as_ref().map_or(256, |m| m.bytes.len());
                drop(snap);
                let (tok, mi) = self.commands.data_write_memory_bytes(addr, &bytes);
                self.pending.insert(tok, PendingKind::WriteMemory { addr: reload_addr, count: reload_count });
                self.send_raw(&mi).await?;
            }
            GdbCommand::Disassemble { addr, count } => {
                let end = addr.saturating_add(count as u64);
                let (tok, mi) = self.commands.data_disassemble_addr(addr, end);
                self.pending.insert(tok, PendingKind::Disassemble);
                self.send_raw(&mi).await?;
            }
            GdbCommand::EvaluateExpression(expr) => {
                let (tok, mi) = self.commands.data_evaluate_expression(&expr);
                self.pending.insert(
                    tok,
                    PendingKind::EvalExpression {
                        watch_id: None,
                        expr,
                    },
                );
                self.send_raw(&mi).await?;
            }
            GdbCommand::AddWatch(expr) => {
                let id = self.next_watch_id;
                self.next_watch_id += 1;
                self.watch_expressions.push(WatchEntry {
                    id,
                    expression: expr.clone(),
                });
                self.update_snapshot(|snap| {
                    snap.watch_expressions.push(WatchExpression {
                        id,
                        expression: expr.clone(),
                        value: "<evaluating>".into(),
                        type_name: String::new(),
                        error: None,
                    });
                });
                let (tok, mi) = self.commands.data_evaluate_expression(&expr);
                self.pending.insert(
                    tok,
                    PendingKind::EvalExpression {
                        watch_id: Some(id),
                        expr,
                    },
                );
                self.send_raw(&mi).await?;
            }
            GdbCommand::RemoveWatch(id) => {
                self.watch_expressions.retain(|w| w.id != id);
                self.update_snapshot(|snap| {
                    snap.watch_expressions.retain(|w| w.id != id);
                });
            }
            GdbCommand::RawCommand(raw) => {
                let (tok, mi) = self.commands.cli_command(&raw);
                self.pending.insert(tok, PendingKind::CliCommand);
                self.send_raw(&mi).await?;
            }
            GdbCommand::Quit => {
                self.shutdown().await;
                return Ok(());
            }
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Shutdown
    // -----------------------------------------------------------------------

    async fn shutdown(&mut self) {
        // For attached processes, detach first so the target continues running.
        if matches!(self.target_mode, TargetMode::AttachPid(_)) {
            let (_, mi) = self.commands.target_detach();
            let _ = self.send_raw(&mi).await;
            // Give GDB a moment to process the detach.
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }

        // Send -gdb-exit for a clean shutdown.
        let (_, mi) = self.commands.gdb_exit();
        let _ = self.send_raw(&mi).await;

        // Wait up to 2 seconds for GDB to exit gracefully.
        let exited = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            self.child.wait(),
        )
        .await;

        match exited {
            Ok(Ok(status)) => {
                debug!("GDB exited with {status}");
            }
            _ => {
                warn!("GDB did not exit gracefully, killing process");
                let _ = self.child.kill().await;
            }
        }
    }

    // -----------------------------------------------------------------------
    // Auto-refresh on stop
    // -----------------------------------------------------------------------

    /// Send all commands needed after the target stops: thread info, stack,
    /// locals, breakpoints, and (if previously requested) registers + watches.
    async fn auto_refresh_on_stop(&mut self) -> Result<()> {
        self.update_snapshot(|snap| {
            snap.source_loading = true;
        });
        self.send_thread_info().await?;
        self.send_stack_list_frames().await?;
        self.send_stack_list_locals().await?;
        self.send_break_list().await?;

        if self.register_names_loaded {
            let (tok, mi) = self.commands.data_list_register_values("x");
            self.pending.insert(tok, PendingKind::RegisterValues);
            self.send_raw(&mi).await?;
        }

        // Clone the list to avoid borrowing self while iterating.
        let entries: Vec<WatchEntry> = self.watch_expressions.clone();
        for entry in &entries {
            let (tok, mi) = self.commands.data_evaluate_expression(&entry.expression);
            self.pending.insert(
                tok,
                PendingKind::EvalExpression {
                    watch_id: Some(entry.id),
                    expr: entry.expression.clone(),
                },
            );
            self.send_raw(&mi).await?;
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Send helpers
    // -----------------------------------------------------------------------

    /// Load source file and update source_line for the frame at `level`.
    ///
    /// If the requested frame has no source (e.g. libc's `syscall`), walk up
    /// the stack to find the nearest ancestor that *does* have source — this
    /// mirrors IDE behaviour when attaching to a process stopped in a system
    /// call.
    async fn load_source_for_frame(&mut self, level: u32) {
        let snap = self.state.load();

        // Start at the requested level, then try higher-numbered frames.
        let candidate = snap.stack.iter()
            .find(|f| f.level == level && f.fullname.is_some())
            .or_else(|| {
                snap.stack.iter()
                    .filter(|f| f.level > level && f.fullname.is_some())
                    .min_by_key(|f| f.level)
            });

        if let Some(frame) = candidate {
            let fullname = frame.fullname.clone().unwrap();
            let line = frame.line;
            drop(snap);
            self.maybe_load_source(&fullname).await;
            let cached = self.source_cache.get(&fullname).cloned();
            self.update_snapshot(|s| {
                s.source = cached;
                s.source_line = line;
            });
        } else {
            drop(snap);
            self.update_snapshot(|s| {
                s.source = None;
                s.source_line = None;
            });
        }
    }

    /// Write a raw MI command string to GDB's stdin and flush.
    async fn send_raw(&mut self, command: &str) -> Result<()> {
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

    async fn send_thread_info(&mut self) -> Result<()> {
        let (tok, mi) = self.commands.thread_info();
        self.pending.insert(tok, PendingKind::ThreadInfo);
        self.send_raw(&mi).await
    }

    async fn send_stack_list_frames(&mut self) -> Result<()> {
        let (tok, mi) = self.commands.stack_list_frames();
        self.pending.insert(tok, PendingKind::StackListFrames);
        self.send_raw(&mi).await
    }

    async fn send_stack_list_locals(&mut self) -> Result<()> {
        let (tok, mi) = self.commands.stack_list_locals();
        self.pending.insert(tok, PendingKind::StackListLocals);
        self.send_raw(&mi).await
    }

    async fn send_break_list(&mut self) -> Result<()> {
        let (tok, mi) = self.commands.break_list();
        self.pending.insert(tok, PendingKind::BreakList);
        self.send_raw(&mi).await
    }

    async fn send_read_memory(&mut self, addr: u64, count: usize) -> Result<()> {
        let (tok, mi) = self.commands.data_read_memory_bytes(addr, count);
        self.pending.insert(tok, PendingKind::ReadMemory);
        self.send_raw(&mi).await
    }

    // -----------------------------------------------------------------------
    // Result-body processors
    // -----------------------------------------------------------------------

    fn process_thread_info(&mut self, body: &[(String, MiValue)]) {
        let mut threads = Vec::new();

        if let Some(threads_val) = MiBody::get(body, "threads") {
            if let Some(thread_list) = threads_val.as_list_values() {
                for tv in thread_list {
                    if let Some(t) = parse_thread(tv) {
                        threads.push(t);
                    }
                }
            }
        }

        let current_thread_id = MiBody::get_str(body, "current-thread-id")
            .and_then(|s| s.parse::<i32>().ok());

        self.update_snapshot(|snap| {
            snap.threads = threads;
            if let Some(id) = current_thread_id {
                snap.current_thread_id = Some(id);
            }
        });
    }

    async fn process_stack_list_frames(&mut self, body: &[(String, MiValue)]) {
        let mut frames = Vec::new();

        if let Some(stack_val) = MiBody::get(body, "stack") {
            match stack_val {
                MiValue::List(MiList::Results(pairs)) => {
                    for (_key, val) in pairs {
                        if let Some(f) = parse_frame(val) {
                            frames.push(f);
                        }
                    }
                }
                MiValue::List(MiList::Values(vals)) => {
                    for val in vals {
                        if let Some(f) = parse_frame(val) {
                            frames.push(f);
                        }
                    }
                }
                _ => {
                    warn!("unexpected stack format in -stack-list-frames result");
                }
            }
        }

        self.update_snapshot(|snap| {
            snap.stack = frames;
        });

        // Load source for the frame at current_frame_level (or frame #0).
        let level = self.state.load().current_frame_level;
        self.load_source_for_frame(level).await;
    }

    fn process_stack_list_locals(&mut self, body: &[(String, MiValue)]) {
        let mut locals = Vec::new();

        if let Some(locals_val) = MiBody::get(body, "locals") {
            if let Some(local_list) = locals_val.as_list_values() {
                for lv in local_list {
                    if let Some(pairs) = lv.as_tuple() {
                        let name = tuple_get_str(pairs, "name").to_string();
                        let value = tuple_get_str(pairs, "value").to_string();
                        let type_name = tuple_get_str(pairs, "type").to_string();
                        locals.push(Variable {
                            name,
                            value,
                            type_name,
                        });
                    }
                }
            }
        }

        self.update_snapshot(|snap| {
            snap.locals = locals;
        });
    }

    fn process_break_insert(&mut self, body: &[(String, MiValue)]) {
        if let Some(bkpt_val) = MiBody::get(body, "bkpt") {
            if let Some(bp) = parse_breakpoint(bkpt_val) {
                let msg = format!(
                    "Breakpoint {} at {}",
                    bp.number, bp.original_location
                );
                self.update_snapshot(|snap| {
                    snap.breakpoints.push(bp);
                    snap.push_output(OutputKind::Info, msg);
                });
            }
        }
    }

    fn process_break_list(&mut self, body: &[(String, MiValue)]) {
        let mut breakpoints = Vec::new();

        if let Some(table) = MiBody::get(body, "BreakpointTable") {
            if let Some(bkpt_body) = table.get("body") {
                match bkpt_body {
                    MiValue::List(MiList::Values(vals)) => {
                        for bv in vals {
                            if let Some(bp) = parse_breakpoint(bv) {
                                breakpoints.push(bp);
                            }
                        }
                    }
                    MiValue::List(MiList::Results(pairs)) => {
                        for (_key, val) in pairs {
                            if let Some(bp) = parse_breakpoint(val) {
                                breakpoints.push(bp);
                            }
                        }
                    }
                    MiValue::List(MiList::Empty) => {}
                    _ => {
                        warn!("unexpected BreakpointTable body format");
                    }
                }
            }
        }

        self.update_snapshot(|snap| {
            snap.breakpoints = breakpoints;
        });
    }

    fn process_register_names(&mut self, body: &[(String, MiValue)]) {
        let mut names = Vec::new();

        if let Some(reg_names) = MiBody::get(body, "register-names") {
            if let Some(name_list) = reg_names.as_list_values() {
                for nv in name_list {
                    names.push(nv.as_const().unwrap_or("").to_string());
                }
            }
        }

        self.register_names_loaded = true;
        self.update_snapshot(|snap| {
            snap.register_names = names;
        });
    }

    fn process_register_values(&mut self, body: &[(String, MiValue)]) {
        let mut registers = Vec::new();

        if let Some(reg_vals) = MiBody::get(body, "register-values") {
            if let Some(val_list) = reg_vals.as_list_values() {
                let snap = self.state.load();
                let names = &snap.register_names;

                for rv in val_list {
                    let number = rv
                        .get_str("number")
                        .and_then(|s| s.parse::<u32>().ok())
                        .unwrap_or(0);
                    let value = rv.get_str("value").unwrap_or("").to_string();
                    let name = names
                        .get(number as usize)
                        .cloned()
                        .unwrap_or_else(|| format!("reg{number}"));

                    if name.is_empty() {
                        continue;
                    }

                    registers.push(Register {
                        number,
                        name,
                        value,
                    });
                }
            }
        }

        self.update_snapshot(|snap| {
            snap.registers = registers;
        });
    }

    fn process_read_memory(&mut self, body: &[(String, MiValue)]) {
        if let Some(mem_val) = MiBody::get(body, "memory") {
            if let Some(mem_list) = mem_val.as_list_values() {
                if let Some(first) = mem_list.first() {
                    let begin = first
                        .get_str("begin")
                        .and_then(parse_hex_u64)
                        .unwrap_or(0);
                    let contents = first.get_str("contents").unwrap_or("");
                    let bytes = decode_hex_string(contents);

                    self.update_snapshot(|snap| {
                        snap.memory = Some(MemoryBlock {
                            address: begin,
                            bytes,
                        });
                    });
                }
            }
        }
    }

    fn process_disassemble(&mut self, body: &[(String, MiValue)]) {
        let mut instructions = Vec::new();

        if let Some(asm_val) = MiBody::get(body, "asm_insns") {
            if let Some(insn_list) = asm_val.as_list_values() {
                for iv in insn_list {
                    let address = iv
                        .get_str("address")
                        .and_then(parse_hex_u64)
                        .unwrap_or(0);
                    let func_name = iv.get_str("func-name").map(String::from);
                    let offset = iv
                        .get_str("offset")
                        .and_then(|s| s.parse::<u32>().ok());
                    let inst = iv.get_str("inst").unwrap_or("").to_string();

                    instructions.push(DisasmInstruction {
                        address,
                        func_name,
                        offset,
                        inst,
                    });
                }
            }
        }

        self.update_snapshot(|snap| {
            snap.disasm = instructions;
        });
    }

    fn process_eval_expression(
        &mut self,
        watch_id: Option<u32>,
        expr: &str,
        body: &[(String, MiValue)],
    ) {
        let value = MiBody::get_str(body, "value").unwrap_or("").to_string();

        match watch_id {
            Some(id) => {
                self.update_snapshot(|snap| {
                    if let Some(w) =
                        snap.watch_expressions.iter_mut().find(|w| w.id == id)
                    {
                        w.value = value;
                        w.error = None;
                    }
                });
            }
            None => {
                self.update_snapshot(|snap| {
                    snap.push_output(OutputKind::Console, format!("{expr} = {value}"));
                });
            }
        }
    }

    // -----------------------------------------------------------------------
    // Source file loading
    // -----------------------------------------------------------------------

    /// Load the source file from disk if it is not already in the cache, then
    /// update the snapshot's `source` field.
    async fn maybe_load_source(&mut self, fullname: &str) {
        if self.source_cache.contains_key(fullname) {
            let src = self.source_cache.get(fullname).cloned();
            self.update_snapshot(|snap| {
                snap.source = src;
                snap.source_loading = false;
            });
            return;
        }

        // Signal the TUI that we're loading.
        self.update_snapshot(|snap| {
            snap.source_loading = true;
        });

        let path = fullname.to_string();
        let result = tokio::task::spawn_blocking(move || {
            let contents = std::fs::read_to_string(&path)?;
            let lines: Vec<String> = contents.lines().map(String::from).collect();
            let highlighted = crate::highlight::highlight_lines(&path, &lines);
            Ok::<_, std::io::Error>((lines, highlighted))
        })
        .await;

        match result {
            Ok(Ok((lines, highlighted))) => {
                let source_file = SourceFile {
                    path: fullname.to_string(),
                    lines,
                    highlighted,
                };
                self.source_cache
                    .insert(fullname.to_string(), source_file.clone());
                self.update_snapshot(|snap| {
                    snap.source = Some(source_file);
                    snap.source_loading = false;
                });
            }
            Ok(Err(e)) => {
                debug!("failed to read source file {fullname}: {e}");
                self.update_snapshot(|snap| {
                    snap.source_loading = false;
                });
            }
            Err(e) => {
                warn!("spawn_blocking for source read panicked: {e}");
                self.update_snapshot(|snap| {
                    snap.source_loading = false;
                });
            }
        }
    }

    // -----------------------------------------------------------------------
    // Snapshot update helper
    // -----------------------------------------------------------------------

    /// Clone the current snapshot, apply the mutation, and publish the new
    /// version atomically via ArcSwap.
    fn update_snapshot(&self, f: impl FnOnce(&mut GdbSnapshot)) {
        let mut snap = (**self.state.load()).clone();
        f(&mut snap);
        self.state.store(Arc::new(snap));
    }
}

// ===========================================================================
// Free-standing parsing helpers
// ===========================================================================

/// Extract a [`Frame`] from an MI tuple value.
fn parse_frame(val: &MiValue) -> Option<Frame> {
    let level = val
        .get_str("level")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let addr = val
        .get_str("addr")
        .and_then(parse_hex_u64)
        .unwrap_or(0);
    let func = val.get_str("func").map(String::from);
    let file = val.get_str("file").map(String::from);
    let fullname = val.get_str("fullname").map(String::from);
    let line = val.get_str("line").and_then(|s| s.parse::<u32>().ok());

    let mut args = Vec::new();
    if let Some(args_val) = val.get("args") {
        if let Some(arg_list) = args_val.as_list_values() {
            for av in arg_list {
                if let Some(pairs) = av.as_tuple() {
                    let name = tuple_get_str(pairs, "name").to_string();
                    let value = tuple_get_str(pairs, "value").to_string();
                    args.push(FuncArg { name, value });
                }
            }
        }
    }

    Some(Frame {
        level,
        addr,
        func,
        file,
        fullname,
        line,
        args,
    })
}

/// Extract a [`Thread`] from an MI tuple value.
fn parse_thread(val: &MiValue) -> Option<Thread> {
    let id = val.get_str("id").and_then(|s| s.parse::<i32>().ok())?;
    let target_id = val.get_str("target-id").unwrap_or("").to_string();
    let name = val.get_str("name").map(String::from);
    let state = val.get_str("state").unwrap_or("unknown").to_string();
    let frame = val.get("frame").and_then(parse_frame);

    Some(Thread {
        id,
        target_id,
        name,
        state,
        frame,
    })
}

/// Extract a [`Breakpoint`] from an MI tuple value.
fn parse_breakpoint(val: &MiValue) -> Option<Breakpoint> {
    let number = val
        .get_str("number")
        .and_then(|s| s.parse::<u32>().ok())?;
    let enabled = val.get_str("enabled").map(|s| s == "y").unwrap_or(false);
    let bp_type = val.get_str("type").unwrap_or("breakpoint").to_string();
    let address = val.get_str("addr").and_then(parse_hex_u64);
    let func = val.get_str("func").map(String::from);
    let file = val.get_str("file").map(String::from);
    let line = val.get_str("line").and_then(|s| s.parse::<u32>().ok());
    let hit_count = val
        .get_str("times")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let original_location = val
        .get_str("original-location")
        .unwrap_or("")
        .to_string();

    Some(Breakpoint {
        number,
        enabled,
        bp_type,
        address,
        func,
        file,
        line,
        hit_count,
        original_location,
    })
}

/// Parse the stop reason from an async `*stopped` record body.
fn parse_stop_reason(body: &[(String, MiValue)]) -> StopReason {
    let reason = match MiBody::get_str(body, "reason") {
        Some(r) => r,
        None => return StopReason::Unknown(String::new()),
    };

    match reason {
        "breakpoint-hit" => {
            let number = MiBody::get_str(body, "bkptno")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            StopReason::BreakpointHit { number }
        }
        "watchpoint-trigger" => {
            let number = MiBody::get(body, "wpt")
                .and_then(|w: &MiValue| w.get_str("number"))
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            StopReason::Watchpoint { number }
        }
        "end-stepping-range" => StopReason::StepFinished,
        "function-finished" => StopReason::FunctionFinished,
        "signal-received" => {
            let name = MiBody::get_str(body, "signal-name")
                .unwrap_or("UNKNOWN")
                .to_string();
            let meaning = MiBody::get_str(body, "signal-meaning")
                .unwrap_or("")
                .to_string();
            StopReason::SignalReceived { name, meaning }
        }
        "exited-normally" => StopReason::ExitedNormally { code: 0 },
        "exited" => {
            let code = MiBody::get_str(body, "exit-code")
                .and_then(|s| {
                    if s.starts_with('0') && s.len() > 1 && !s.starts_with("0x") {
                        i32::from_str_radix(s, 8).ok()
                    } else {
                        s.parse::<i32>().ok()
                    }
                })
                .unwrap_or(0);
            StopReason::ExitedNormally { code }
        }
        other => StopReason::Unknown(other.to_string()),
    }
}

// ===========================================================================
// Small utilities
// ===========================================================================

/// Parse a hex string like `"0x7fffffffe380"` into a `u64`.
fn parse_hex_u64(s: &str) -> Option<u64> {
    let stripped = s
        .strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    u64::from_str_radix(stripped, 16).ok()
}

/// Decode a hex-encoded byte string (e.g. `"48656c6c6f"`) into raw bytes.
fn decode_hex_string(hex: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    let mut chars = hex.chars();
    while let (Some(hi), Some(lo)) = (chars.next(), chars.next()) {
        let hi = hi.to_digit(16).unwrap_or(0) as u8;
        let lo = lo.to_digit(16).unwrap_or(0) as u8;
        bytes.push((hi << 4) | lo);
    }
    bytes
}

/// Look up a string value inside a slice of key-value pairs (tuple body).
fn tuple_get_str<'a>(pairs: &'a [(String, MiValue)], key: &str) -> &'a str {
    pairs
        .iter()
        .find(|(k, _)| k == key)
        .and_then(|(_, v)| v.as_const())
        .unwrap_or("")
}
