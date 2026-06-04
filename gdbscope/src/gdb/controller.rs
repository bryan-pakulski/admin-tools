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
use crate::tui::SharedRecording;
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
    TraceContinue,
    TraceContinueFull,
    StepOver,
    StepInto,
    StepOut,
    Interrupt,
    SelectThread(i32),
    SelectFrame(u32),
    SetBreakpoint(String),
    SetBreakpointCond { location: String, condition: String },
    BreakCondition { number: u32, condition: String },
    SetWatchpoint { expr: String, kind: crate::gdb::mi_command::WatchKind },
    DeleteBreakpoint(u32),
    ToggleBreakpoint(u32),
    SetRegister { name: String, value: String },
    RefreshRegisters,
    ReadMemory { addr: u64, count: usize },
    ReadMemoryExpr { expr: String, count: usize },
    WriteMemory { addr: u64, bytes: Vec<u8> },
    Disassemble { addr: u64, count: usize },
    EvaluateExpression(String),
    AddWatch(String),
    RemoveWatch(u32),
    SearchMemoryString { start: u64, length: u64, pattern: String },
    SearchMemoryBytes { start: u64, length: u64, bytes: Vec<u8> },
    PatchBytes { addr: u64, bytes: Vec<u8> },
    /// Analyze cross-references at a given address using the current disassembly.
    AnalyzeXrefs { addr: u64 },
    /// Cast memory at an address as a typed value (e.g. "struct sockaddr_in").
    TypeOverlay { addr: u64, type_expr: String },
    /// List known functions (optional regex filter).
    ListFunctions(Option<String>),
    /// Resolve an address to the nearest symbol.
    ResolveSymbol(u64),
    RefreshLibraries,
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
    StackInfoFrame,
    StackListLocalsSimple,
    ThreadSelect(i32),
    BreakInsert,
    BreakDelete(u32),
    BreakEnable(u32),
    BreakDisable(u32),
    BreakCondition(u32),
    BreakWatch,
    BreakList,
    RegisterNames,
    RegisterValues,
    SetRegister,
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
    SearchMemory,
    PatchBytes { addr: u64, byte_count: usize },
    TypeOverlay { addr: u64, type_expr: String },
    ListFunctions,
    ResolveSymbol,
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
    recording: SharedRecording,
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
    tracing: bool,
    trace_steps_remaining: usize,
    trace_max_steps: usize,
    trace_refresh_pending: usize, // count of outstanding refresh queries during trace
    trace_is_bp: bool,            // whether the current trace stop was a breakpoint
    exec_args: Vec<String>,
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
        recording: SharedRecording,
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
        let trace_depth = config.trace_depth;
        let exec_args = match &config.target {
            TargetMode::LaunchExec { args, .. } => args.clone(),
            _ => Vec::new(),
        };
        let handle = tokio::spawn(async move {
            let mut ctrl = GdbController {
                state,
                recording,
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
                tracing: false,
                trace_steps_remaining: 0,
                trace_max_steps: trace_depth,
                trace_refresh_pending: 0,
                trace_is_bp: false,
                exec_args,
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
                self.update_snapshot(|snap| { snap.source_loading = true; });
                self.send_thread_info().await?;
                self.send_stack_list_frames().await?;
                self.send_stack_list_locals().await?;
                self.send_break_list().await?;
                if !self.register_names_loaded {
                    let (tok, mi) = self.commands.data_list_register_names();
                    self.pending.insert(tok, PendingKind::RegisterNames);
                    self.send_raw(&mi).await?;
                }
                let (tok, mi) = self.commands.data_list_register_values("x");
                self.pending.insert(tok, PendingKind::RegisterValues);
                self.send_raw(&mi).await?;
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
        let is_trace_refresh = self.tracing && matches!(
            kind,
            PendingKind::StackListFrames
            | PendingKind::StackListLocals
            | PendingKind::StackListLocalsSimple
            | PendingKind::StackInfoFrame
            | PendingKind::RegisterNames
            | PendingKind::RegisterValues
            | PendingKind::Disassemble
        );

        match kind {
            PendingKind::ThreadInfo => {
                self.process_thread_info(body);
            }
            PendingKind::StackListFrames => {
                self.process_stack_list_frames(body).await;
            }
            PendingKind::StackListLocals | PendingKind::StackListLocalsSimple => {
                self.process_stack_list_locals(body);
            }
            PendingKind::StackInfoFrame => {
                // Lightweight frame response: update just frame #0 in the stack
                if let Some(frame_val) = MiBody::get(body, "frame") {
                    if let Some(f) = parse_frame(frame_val) {
                        self.update_snapshot(|snap| {
                            if snap.stack.is_empty() {
                                snap.stack.push(f.clone());
                            } else {
                                snap.stack[0] = f.clone();
                            }
                            snap.source_line = f.line;
                            snap.has_debug_info = f.fullname.is_some();
                        });
                    }
                }
            }
            PendingKind::StackSelectFrame(level) => {
                // Load source for the newly selected frame.
                self.load_source_for_frame(level).await;
                // Load disassembly around the selected frame's address.
                self.load_disasm_for_frame(level).await;
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
            PendingKind::BreakCondition(num) => {
                // Condition set successfully — refresh breakpoints to pick
                // up the new condition text.
                self.update_snapshot(|snap| {
                    snap.push_output(
                        OutputKind::Info,
                        format!("Condition updated on breakpoint {num}"),
                    );
                });
                if let Err(e) = self.send_break_list().await {
                    warn!("failed to refresh breakpoints after condition: {e:#}");
                }
            }
            PendingKind::BreakWatch => {
                // Watchpoint created — refresh the full breakpoint list so
                // it shows up in the panel.
                if let Err(e) = self.send_break_list().await {
                    warn!("failed to refresh breakpoints after watchpoint: {e:#}");
                }
            }
            PendingKind::BreakList => {
                self.process_break_list(body);
            }
            PendingKind::SetRegister => {
                // Register set — refresh register values to reflect the
                // change in the panel.
                let (tok, mi) = self.commands.data_list_register_values("x");
                self.pending.insert(tok, PendingKind::RegisterValues);
                if let Err(e) = self.send_raw(&mi).await {
                    warn!("failed to refresh registers after set: {e:#}");
                }
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
            PendingKind::SearchMemory => {
                debug!("memory search complete (results in console output)");
            }
            PendingKind::TypeOverlay { addr, type_expr } => {
                // Parse the GDB result value to extract struct fields
                let value = MiBody::get_str(body, "value").unwrap_or("").to_string();
                let fields = parse_struct_fields(&value);
                let overlay = TypeOverlay {
                    type_name: type_expr.clone(),
                    address: addr,
                    total_size: 0, // not available without ptype parsing
                    fields,
                };
                let field_count = overlay.fields.len();
                self.update_snapshot(|snap| {
                    if field_count == 0 {
                        snap.push_output(
                            OutputKind::Console,
                            format!("*({type_expr}*)0x{addr:x} = {value}"),
                        );
                    } else {
                        snap.push_output(
                            OutputKind::Info,
                            format!("Type overlay: ({type_expr}*)0x{addr:x} — {field_count} fields"),
                        );
                        for f in &overlay.fields {
                            snap.push_output(
                                OutputKind::Console,
                                format!("  .{} = {}", f.name, f.value),
                            );
                        }
                    }
                    snap.type_overlay = Some(overlay);
                });
            }
            PendingKind::ListFunctions => {
                self.process_function_list(body);
            }
            PendingKind::ResolveSymbol => {
                debug!("info symbol complete (results in console output)");
            }
            PendingKind::PatchBytes { addr, byte_count } => {
                debug!("patch bytes complete at {addr:#x}");
                // Re-read the disassembly around the patched address so the
                // panel reflects the new instructions.
                let start = addr.saturating_sub(32);
                let end = addr.saturating_add(96);
                let (tok, mi) = self.commands.data_disassemble_addr(start, end);
                self.pending.insert(tok, PendingKind::Disassemble);
                if let Err(e) = self.send_raw(&mi).await {
                    warn!("failed to refresh disasm after patch: {e:#}");
                }
                // Also re-read memory if the memory panel is viewing this area.
                let snap = self.state.load();
                let mem_start = snap.memory_address;
                let mem_len = snap.memory.as_ref().map_or(0, |m| m.bytes.len());
                drop(snap);
                if mem_len > 0
                    && addr >= mem_start
                    && addr < mem_start + mem_len as u64
                {
                    if let Err(e) = self.send_read_memory(mem_start, mem_len).await {
                        warn!("failed to refresh memory after patch: {e:#}");
                    }
                }
                self.update_snapshot(|snap| {
                    snap.push_output(
                        OutputKind::Info,
                        format!("Patched {} byte(s) at {addr:#x}", byte_count),
                    );
                });
            }
        }

        // If we're tracing and this was a trace-refresh response, check if
        // all refresh queries are done so we can capture + step.
        if is_trace_refresh && self.trace_refresh_pending > 0 {
            self.trace_refresh_done().await;
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

                let is_bp = matches!(reason, StopReason::BreakpointHit { .. } | StopReason::Watchpoint { .. });

                self.update_snapshot(|snap| {
                    snap.target_state = TargetState::Stopped;
                    snap.stop_reason = Some(reason);
                    snap.source_line = source_line;
                    snap.status_message = Some(status);
                });

                if self.tracing {
                    self.trace_is_bp = is_bp;
                    if !is_bp && self.trace_steps_remaining > 0 {
                        self.trace_steps_remaining -= 1;
                    }
                    let steps_done = self.trace_max_steps - self.trace_steps_remaining;
                    self.update_snapshot(|snap| {
                        snap.status_message = Some(format!(
                            "Tracing... step {steps_done}/{}",
                            self.trace_max_steps
                        ));
                    });

                    // Send optimized queries (stack-info-frame + simple locals)
                    // then capture + step when all responses arrive.
                    self.trace_refresh_pending = 0;
                    if let Err(e) = self.send_trace_refresh().await {
                        warn!("trace refresh failed: {e:#}");
                        self.tracing = false;
                    }
                } else {
                    // Normal (non-trace) stop: full refresh + capture
                    if let Err(e) = self.auto_refresh_on_stop().await {
                        warn!("auto-refresh on stop failed: {e:#}");
                    }
                    self.capture_recording_with_anchor(is_bp);
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
            "breakpoint-modified" | "breakpoint-created" => {
                if let Some(bkpt_val) = MiBody::get(body, "bkpt") {
                    if let Some(bp) = parse_breakpoint(bkpt_val) {
                        self.update_snapshot(|snap| {
                            if let Some(existing) = snap
                                .breakpoints
                                .iter_mut()
                                .find(|b| b.number == bp.number)
                            {
                                *existing = bp;
                            } else {
                                snap.breakpoints.push(bp);
                            }
                        });
                    }
                }
            }
            "breakpoint-deleted" => {
                if let Some(id_str) = MiBody::get_str(body, "id") {
                    if let Ok(id) = id_str.parse::<u32>() {
                        self.update_snapshot(|snap| {
                            snap.breakpoints.retain(|b| b.number != id);
                        });
                    }
                }
            }
            "library-loaded" => {
                let name = MiBody::get_str(body, "id").unwrap_or("?").to_string();
                let target_name = MiBody::get_str(body, "target-name").unwrap_or("?").to_string();
                let syms = MiBody::get_str(body, "symbols-loaded")
                    .map(|s| s == "1")
                    .unwrap_or(false);
                let ranges = MiBody::get(body, "ranges");
                let base_addr = ranges.and_then(|r| {
                    if let Some(vals) = r.as_list_values() {
                        vals.first()
                            .and_then(|v| v.get_str("from"))
                            .and_then(parse_hex_u64)
                    } else {
                        None
                    }
                });
                debug!("library-loaded: {name}");
                self.update_snapshot(|snap| {
                    snap.mapped_libs.push(MappedLibrary {
                        name,
                        target_name,
                        base_addr,
                        symbols_loaded: syms,
                    });
                });
            }
            "library-unloaded" => {
                let name = MiBody::get_str(body, "id").unwrap_or("").to_string();
                debug!("library-unloaded: {name}");
                self.update_snapshot(|snap| {
                    snap.mapped_libs.retain(|l| l.name != name);
                });
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
                let run_args = if args.is_empty() { &self.exec_args } else { &args };
                let (tok, mi) = self.commands.exec_run(run_args);
                self.pending.insert(tok, PendingKind::ExecRun);
                self.send_raw(&mi).await?;
            }
            GdbCommand::Continue => {
                self.tracing = false;
                let (tok, mi) = self.commands.exec_continue();
                self.pending.insert(tok, PendingKind::ExecContinue);
                self.send_raw(&mi).await?;
            }
            GdbCommand::TraceContinue | GdbCommand::TraceContinueFull => {
                self.tracing = true;
                self.trace_steps_remaining = self.trace_max_steps;
                self.update_snapshot(|snap| {
                    snap.status_message = Some(format!(
                        "Tracing... step 0/{}",
                        self.trace_max_steps
                    ));
                });
                let (tok, mi) = self.commands.exec_next();
                self.pending.insert(tok, PendingKind::ExecNext);
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
                self.tracing = false;
                // Send MI interrupt command
                let (tok, mi) = self.commands.exec_interrupt();
                self.pending.insert(tok, PendingKind::ExecInterrupt);
                let _ = self.send_raw(&mi).await;
                // Also send SIGINT to the GDB process as a fallback — GDB
                // forwards it to the inferior.
                if let Some(pid) = self.child.id() {
                    unsafe { libc::kill(pid as i32, libc::SIGINT); }
                }
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
            GdbCommand::SetBreakpointCond { location, condition } => {
                let (tok, mi) = self.commands.break_insert_cond(&location, &condition);
                self.pending.insert(tok, PendingKind::BreakInsert);
                self.send_raw(&mi).await?;
            }
            GdbCommand::BreakCondition { number, condition } => {
                let (tok, mi) = self.commands.break_condition(number, &condition);
                self.pending.insert(tok, PendingKind::BreakCondition(number));
                self.send_raw(&mi).await?;
            }
            GdbCommand::SetWatchpoint { expr, kind } => {
                let (tok, mi) = self.commands.break_watch(&expr, kind);
                self.pending.insert(tok, PendingKind::BreakWatch);
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
            GdbCommand::SetRegister { name, value } => {
                let (tok, mi) = self.commands.set_register(&name, &value);
                self.pending.insert(tok, PendingKind::SetRegister);
                self.send_raw(&mi).await?;
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
            GdbCommand::SearchMemoryString { start, length, pattern } => {
                let (tok, mi) = self.commands.find_string(start, length, &pattern);
                self.pending.insert(tok, PendingKind::SearchMemory);
                self.send_raw(&mi).await?;
                self.update_snapshot(|snap| {
                    snap.push_output(
                        OutputKind::Info,
                        format!("Searching for \"{}\" from {start:#x} ({length} bytes)...", pattern),
                    );
                });
            }
            GdbCommand::SearchMemoryBytes { start, length, bytes } => {
                let (tok, mi) = self.commands.find_bytes(start, length, &bytes);
                self.pending.insert(tok, PendingKind::SearchMemory);
                self.send_raw(&mi).await?;
                let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                self.update_snapshot(|snap| {
                    snap.push_output(
                        OutputKind::Info,
                        format!("Searching for bytes [{hex}] from {start:#x} ({length} bytes)..."),
                    );
                });
            }
            GdbCommand::PatchBytes { addr, bytes } => {
                let count = bytes.len();
                let (tok, mi) = self.commands.data_write_memory_bytes(addr, &bytes);
                self.pending.insert(tok, PendingKind::PatchBytes { addr, byte_count: count });
                self.send_raw(&mi).await?;
            }
            GdbCommand::AnalyzeXrefs { addr } => {
                // Perform xref analysis locally using the current disassembly snapshot.
                let xrefs = self.analyze_xrefs_local(addr);
                let target_name = {
                    let snap = self.state.load();
                    snap.disasm.iter()
                        .find(|i| i.address == addr)
                        .and_then(|i| i.func_name.clone())
                        .unwrap_or_else(|| format!("0x{addr:x}"))
                };
                let count = xrefs.len();
                self.update_snapshot(|snap| {
                    // Build summary for the output panel
                    if xrefs.is_empty() {
                        snap.push_output(
                            OutputKind::Info,
                            format!("No xrefs found for {target_name} (0x{addr:x}) in current disassembly window"),
                        );
                    } else {
                        snap.push_output(
                            OutputKind::Info,
                            format!("Xrefs for {target_name} (0x{addr:x}): {count} found"),
                        );
                        for xref in &xrefs {
                            let direction = match xref.xref_type {
                                XrefType::CallTo => "Called by",
                                XrefType::CallFrom => "Calls",
                                XrefType::JumpTo => "Jump from",
                            };
                            let name = xref.func_name.as_deref().unwrap_or("??");
                            snap.push_output(
                                OutputKind::Console,
                                format!("  {direction}: 0x{:x} <{name}>  {}", xref.address, xref.context),
                            );
                        }
                    }
                    snap.xrefs = xrefs;
                });
            }
            GdbCommand::TypeOverlay { addr, type_expr } => {
                // Send the expression evaluation to GDB: *(type*)addr
                let (tok, mi) = self.commands.print_typed(&type_expr, addr);
                self.pending.insert(tok, PendingKind::TypeOverlay { addr, type_expr });
                self.send_raw(&mi).await?;
            }
            GdbCommand::ListFunctions(pattern) => {
                let (tok, mi) = self.commands.symbol_info_functions(pattern.as_deref());
                self.pending.insert(tok, PendingKind::ListFunctions);
                self.send_raw(&mi).await?;
                let msg = match &pattern {
                    Some(p) => format!("Searching functions matching '{p}'..."),
                    None => "Listing all functions...".into(),
                };
                self.update_snapshot(|snap| {
                    snap.push_output(OutputKind::Info, msg);
                });
            }
            GdbCommand::ResolveSymbol(addr) => {
                let (tok, mi) = self.commands.info_symbol(addr);
                self.pending.insert(tok, PendingKind::ResolveSymbol);
                self.send_raw(&mi).await?;
            }
            GdbCommand::RefreshLibraries => {
                let (tok, mi) = self.commands.cli_command("info sharedlibrary");
                self.pending.insert(tok, PendingKind::CliCommand);
                self.send_raw(&mi).await?;
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
    /// Send refresh queries during tracing. Unlike auto_refresh_on_stop,
    /// this tracks how many responses are outstanding so we know when to
    /// capture state and issue the next step.
    async fn send_trace_refresh(&mut self) -> Result<()> {
        // Optimized trace queries: use stack-info-frame (current frame only,
        // not full backtrace) and simple-values for locals (skip complex
        // type evaluation). Skip registers — they're captured on final stop.
        // This gives ~120 steps/sec vs ~25 with full queries.

        let (tok, mi) = self.commands.stack_info_frame();
        self.pending.insert(tok, PendingKind::StackInfoFrame);
        self.send_raw(&mi).await?;
        self.trace_refresh_pending += 1;

        let (tok, mi) = self.commands.stack_list_locals_simple();
        self.pending.insert(tok, PendingKind::StackListLocalsSimple);
        self.send_raw(&mi).await?;
        self.trace_refresh_pending += 1;

        Ok(())
    }

    /// Called when a trace-mode refresh response arrives. When all pending
    /// refreshes are done, capture the full state and issue the next step
    /// (or stop tracing if we've hit a breakpoint or exhausted steps).
    async fn trace_refresh_done(&mut self) {
        if self.trace_refresh_pending == 0 {
            return;
        }
        self.trace_refresh_pending -= 1;
        if self.trace_refresh_pending > 0 {
            return; // still waiting for more responses
        }

        // All refreshes complete — capture full state
        self.capture_recording_with_anchor(self.trace_is_bp);

        // Decide whether to continue tracing or stop
        if self.trace_is_bp || self.trace_steps_remaining == 0 {
            let steps_done = self.trace_max_steps - self.trace_steps_remaining;
            self.tracing = false;
            if self.trace_is_bp {
                self.update_snapshot(|snap| {
                    snap.status_message = Some(format!(
                        "Breakpoint hit after {steps_done} traced steps"
                    ));
                });
            } else {
                self.update_snapshot(|snap| {
                    snap.status_message = Some(format!(
                        "Trace complete ({steps_done} steps captured)"
                    ));
                });
            }
            // Full refresh now that tracing is done — load source + disasm
            // for the final stopped position so the TUI shows current state.
            let level = self.state.load().current_frame_level;
            self.load_source_for_frame(level).await;
            self.load_disasm_for_frame(level).await;
            return;
        }

        // Issue next step
        let (tok, mi) = self.commands.exec_next();
        self.pending.insert(tok, PendingKind::ExecNext);
        if let Err(e) = self.send_raw(&mi).await {
            warn!("trace auto-step failed: {e:#}");
            self.tracing = false;
        }
    }

    async fn auto_refresh_on_stop(&mut self) -> Result<()> {
        self.update_snapshot(|snap| {
            snap.source_loading = true;
        });
        self.send_thread_info().await?;
        self.send_stack_list_frames().await?;
        self.send_stack_list_locals().await?;
        self.send_break_list().await?;

        // Note: disassembly is loaded in process_stack_list_frames() after
        // the stack response arrives (so we have the actual PC address).

        // Always load register names (if not yet loaded) then values
        if !self.register_names_loaded {
            let (tok, mi) = self.commands.data_list_register_names();
            self.pending.insert(tok, PendingKind::RegisterNames);
            self.send_raw(&mi).await?;
        }
        let (tok, mi) = self.commands.data_list_register_values("x");
        self.pending.insert(tok, PendingKind::RegisterValues);
        self.send_raw(&mi).await?;

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
                s.source_loading = false;
            });
        }
    }

    /// Load disassembly around the address of the frame at `level`.
    async fn load_disasm_for_frame(&mut self, level: u32) {
        let snap = self.state.load();
        let pc = snap.stack.iter()
            .find(|f| f.level == level)
            .or_else(|| snap.stack.first())
            .map(|f| f.addr)
            .unwrap_or(0);
        drop(snap);
        if pc != 0 {
            let start = pc.saturating_sub(64);
            let end = pc.saturating_add(128);
            let (tok, mi) = self.commands.data_disassemble_addr(start, end);
            self.pending.insert(tok, PendingKind::Disassemble);
            if let Err(e) = self.send_raw(&mi).await {
                warn!("failed to load disassembly for frame {level}: {e:#}");
            }
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

        let has_debug = frames.first().map_or(false, |f| f.fullname.is_some());
        self.update_snapshot(|snap| {
            snap.has_debug_info = has_debug;
            snap.stack = frames;
        });

        if self.tracing {
            // During tracing, skip source file I/O and disasm to keep steps
            // fast.  source_line is set from the *stopped frame info; source
            // files are loaded on demand during playback.
            self.update_snapshot(|snap| {
                snap.source_loading = false;
            });
        } else {
            // Normal stop: load source file and disassembly.
            let level = self.state.load().current_frame_level;
            self.load_source_for_frame(level).await;
            self.load_disasm_for_frame(level).await;
        }
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

    fn process_function_list(&mut self, body: &[(String, MiValue)]) {
        // Parse -symbol-info-functions response:
        // ^done,symbols={debug=[{filename="...",fullname="...",symbols=[{line="N",name="func",...}]}]}
        let mut entries: Vec<String> = Vec::new();

        if let Some(symbols) = MiBody::get(body, "symbols") {
            // Process debug symbols
            for section in &["debug", "nondebug"] {
                if let Some(file_list) = symbols.get(section) {
                    if let Some(files) = file_list.as_list_values() {
                        for file_val in files {
                            let filename = file_val.get_str("filename").unwrap_or("??");
                            let short = filename.rsplit('/').next().unwrap_or(filename);
                            if let Some(sym_list) = file_val.get("symbols") {
                                if let Some(syms) = sym_list.as_list_values() {
                                    for sym in syms {
                                        let name = sym.get_str("name").unwrap_or("??");
                                        let desc = sym.get_str("description").unwrap_or("");
                                        let line = sym.get_str("line").unwrap_or("");
                                        if !line.is_empty() {
                                            entries.push(format!("  {name:<40} {short}:{line}"));
                                        } else if !desc.is_empty() {
                                            entries.push(format!("  {name:<40} {desc}"));
                                        } else {
                                            entries.push(format!("  {name:<40} {short}"));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let count = entries.len();
        // Cap output to prevent flooding
        let truncated = entries.len() > 200;
        entries.truncate(200);

        self.update_snapshot(|snap| {
            snap.push_output(OutputKind::Info, format!("--- Functions ({count} found) ---"));
            for entry in entries {
                snap.push_output(OutputKind::Console, entry);
            }
            if truncated {
                snap.push_output(
                    OutputKind::Info,
                    format!("... truncated at 200 of {count}. Use f with a filter pattern."),
                );
            }
        });
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

    // -----------------------------------------------------------------------
    // Cross-reference analysis (local, no GDB round-trip)
    // -----------------------------------------------------------------------

    /// Analyze cross-references for a given address using the current
    /// disassembly snapshot.  This is purely local — no MI commands are sent.
    fn analyze_xrefs_local(&self, target_addr: u64) -> Vec<XrefEntry> {
        let snap = self.state.load();
        let mut xrefs = Vec::new();

        // Find the function name at the target address (if any)
        let target_func = snap.disasm.iter()
            .find(|i| i.address == target_addr)
            .and_then(|i| i.func_name.clone());

        for inst in &snap.disasm {
            // Check if this instruction calls/jumps TO our target
            if let Some(call_target) = parse_call_target(&inst.inst) {
                if call_target == target_addr {
                    xrefs.push(XrefEntry {
                        address: inst.address,
                        func_name: inst.func_name.clone(),
                        xref_type: XrefType::CallTo,
                        context: inst.inst.clone(),
                    });
                }
            }

            // Check if the target address itself makes calls/jumps (outgoing)
            if inst.address == target_addr {
                if let Some(call_target) = parse_call_target(&inst.inst) {
                    // Find the name of whatever it calls
                    let callee_name = snap.disasm.iter()
                        .find(|i| i.address == call_target)
                        .and_then(|i| i.func_name.clone());
                    let xref_type = if inst.inst.trim().starts_with("call")
                        || inst.inst.trim().starts_with("bl")
                    {
                        XrefType::CallFrom
                    } else {
                        XrefType::JumpTo
                    };
                    xrefs.push(XrefEntry {
                        address: call_target,
                        func_name: callee_name,
                        xref_type,
                        context: inst.inst.clone(),
                    });
                }
            }
        }

        // Also scan for any instructions in the same function that call/jump
        // to the target, if the target is a function entry point (offset == 0
        // or Some(0)).
        if let Some(ref func_name) = target_func {
            for inst in &snap.disasm {
                if inst.func_name.as_ref() != Some(func_name) {
                    // Only look at instructions in OTHER functions
                    if let Some(call_target) = parse_call_target(&inst.inst) {
                        // Does this call target land within the target function?
                        // We already captured exact address matches above, but
                        // check for the function name in angle brackets as well.
                        if call_target != target_addr {
                            // Check if the instruction text references the function
                            if inst.inst.contains(&format!("<{func_name}>"))
                                || inst.inst.contains(&format!("<{func_name}+"))
                                || inst.inst.contains(&format!("<{func_name}@"))
                            {
                                // Avoid duplicates
                                let already = xrefs.iter().any(|x| x.address == inst.address);
                                if !already {
                                    xrefs.push(XrefEntry {
                                        address: inst.address,
                                        func_name: inst.func_name.clone(),
                                        xref_type: XrefType::CallTo,
                                        context: inst.inst.clone(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        xrefs
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

    /// Capture the current snapshot into the recording buffer.
    fn capture_recording_with_anchor(&self, is_anchor: bool) {
        if let Ok(mut rec) = self.recording.lock() {
            let snap = self.state.load();
            if is_anchor {
                rec.capture_anchor(&snap);
            } else {
                rec.capture(&snap);
            }
            let count = rec.len();
            drop(rec);
            self.update_snapshot(|s| {
                s.recording_count = count;
            });
        }
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
    let condition = val.get_str("cond").map(String::from);

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
        condition,
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

// ===========================================================================
// Analysis helpers — xref parsing, type overlay field parsing
// ===========================================================================

/// Parse a call/jmp target address from an instruction string.
///
/// Handles formats like:
/// - `call   0x401000 <printf@plt>`
/// - `jmp    0x401020`
/// - `je     0x401030 <main+0x10>`
/// - `callq  *0x401000(%rip)`
fn parse_call_target(inst: &str) -> Option<u64> {
    let trimmed = inst.trim();
    let lower = trimmed.to_lowercase();

    // Must start with a call/jump mnemonic
    let is_branch = lower.starts_with("call")
        || lower.starts_with("jmp")
        || lower.starts_with("je")
        || lower.starts_with("jne")
        || lower.starts_with("jg")
        || lower.starts_with("jl")
        || lower.starts_with("jge")
        || lower.starts_with("jle")
        || lower.starts_with("ja")
        || lower.starts_with("jb")
        || lower.starts_with("jae")
        || lower.starts_with("jbe")
        || lower.starts_with("jz")
        || lower.starts_with("jnz")
        || lower.starts_with("js")
        || lower.starts_with("jns")
        || lower.starts_with("jo")
        || lower.starts_with("jno")
        || lower.starts_with("jp")
        || lower.starts_with("jnp")
        || lower.starts_with("bl")
        || lower.starts_with("b.");

    if !is_branch {
        return None;
    }

    // Find a hex address in the operand portion (after the mnemonic)
    for word in trimmed.split_whitespace().skip(1) {
        let clean = word
            .trim_start_matches("0x")
            .trim_start_matches("0X")
            .trim_start_matches('*');
        // Stop at non-hex characters (e.g. angle bracket from "<func>")
        let hex_part: String = clean.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        if !hex_part.is_empty() {
            if let Ok(addr) = u64::from_str_radix(&hex_part, 16) {
                return Some(addr);
            }
        }
    }

    None
}

/// Parse GDB's struct output format `{field1 = val1, field2 = val2, ...}`
/// into individual field entries.
fn parse_struct_fields(value: &str) -> Vec<TypeOverlayField> {
    let trimmed = value.trim();
    // If it doesn't look like a struct, return empty
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return Vec::new();
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let mut fields = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    let mut in_string = false;
    let mut escape_next = false;

    for ch in inner.chars() {
        if escape_next {
            current.push(ch);
            escape_next = false;
            continue;
        }
        if ch == '\\' && in_string {
            current.push(ch);
            escape_next = true;
            continue;
        }
        if ch == '"' {
            in_string = !in_string;
            current.push(ch);
            continue;
        }
        if in_string {
            current.push(ch);
            continue;
        }
        match ch {
            '{' => {
                depth += 1;
                current.push(ch);
            }
            '}' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                if let Some(field) = parse_single_field(&current) {
                    fields.push(field);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        if let Some(field) = parse_single_field(&current) {
            fields.push(field);
        }
    }
    fields
}

/// Parse a single `name = value` pair from GDB struct output.
fn parse_single_field(s: &str) -> Option<TypeOverlayField> {
    let parts: Vec<&str> = s.splitn(2, '=').collect();
    if parts.len() == 2 {
        let name = parts[0].trim().to_string();
        if name.is_empty() {
            return None;
        }
        Some(TypeOverlayField {
            name,
            type_name: String::new(),
            offset: 0,
            size: 0,
            value: parts[1].trim().to_string(),
        })
    } else {
        None
    }
}

// ===========================================================================
// Tests for analysis helpers
// ===========================================================================

#[cfg(test)]
mod analysis_tests {
    use super::*;

    #[test]
    fn parse_call_target_basic() {
        assert_eq!(parse_call_target("call   0x401000 <printf@plt>"), Some(0x401000));
        assert_eq!(parse_call_target("callq  0x401234"), Some(0x401234));
        assert_eq!(parse_call_target("jmp    0x401020"), Some(0x401020));
        assert_eq!(parse_call_target("je     0x401030 <main+0x10>"), Some(0x401030));
    }

    #[test]
    fn parse_call_target_non_branch() {
        assert_eq!(parse_call_target("mov    $0x401000,%rax"), None);
        assert_eq!(parse_call_target("push   %rbp"), None);
        assert_eq!(parse_call_target("nop"), None);
    }

    #[test]
    fn parse_struct_fields_basic() {
        let fields = parse_struct_fields("{x = 42, name = \"hello\", ptr = 0x7fff5000}");
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "x");
        assert_eq!(fields[0].value, "42");
        assert_eq!(fields[1].name, "name");
        assert_eq!(fields[1].value, "\"hello\"");
        assert_eq!(fields[2].name, "ptr");
        assert_eq!(fields[2].value, "0x7fff5000");
    }

    #[test]
    fn parse_struct_fields_nested() {
        let fields = parse_struct_fields("{a = 1, inner = {x = 2, y = 3}, b = 4}");
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].name, "a");
        assert_eq!(fields[0].value, "1");
        assert_eq!(fields[1].name, "inner");
        assert_eq!(fields[1].value, "{x = 2, y = 3}");
        assert_eq!(fields[2].name, "b");
        assert_eq!(fields[2].value, "4");
    }

    #[test]
    fn parse_struct_fields_non_struct() {
        assert!(parse_struct_fields("42").is_empty());
        assert!(parse_struct_fields("0x7fff5000").is_empty());
    }

    #[test]
    fn parse_struct_fields_empty() {
        assert!(parse_struct_fields("{}").is_empty());
    }
}
