pub mod input;
pub mod layout;
pub mod panels;
pub mod widgets;

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::gdb::controller::GdbCommand;
use crate::recording::Recording;
use crate::state::{GdbSnapshot, SharedState, StopReason, TargetState};

use input::{Action, InputMode};
use layout::Panel;

/// Shared handle to the recording buffer, used by the TUI to read playback
/// state and by the controller to write new captures.
pub type SharedRecording = Arc<std::sync::Mutex<Recording>>;

// ---------------------------------------------------------------------------
// Memory type interpretation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemCast {
    Hex,
    U8,
    I8,
    U16LE,
    U32LE,
    U64LE,
    I16LE,
    I32LE,
    I64LE,
    F32LE,
    F64LE,
    Utf8,
}

impl MemCast {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hex => "hex",
            Self::U8 => "u8",
            Self::I8 => "i8",
            Self::U16LE => "u16",
            Self::U32LE => "u32",
            Self::U64LE => "u64",
            Self::I16LE => "i16",
            Self::I32LE => "i32",
            Self::I64LE => "i64",
            Self::F32LE => "f32",
            Self::F64LE => "f64",
            Self::Utf8 => "utf8",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::Hex => Self::U8,
            Self::U8 => Self::I8,
            Self::I8 => Self::U16LE,
            Self::U16LE => Self::U32LE,
            Self::U32LE => Self::U64LE,
            Self::U64LE => Self::I16LE,
            Self::I16LE => Self::I32LE,
            Self::I32LE => Self::I64LE,
            Self::I64LE => Self::F32LE,
            Self::F32LE => Self::F64LE,
            Self::F64LE => Self::Utf8,
            Self::Utf8 => Self::Hex,
        }
    }
}

// ---------------------------------------------------------------------------
// Recording timeline summary types (populated from Recording each frame)
// ---------------------------------------------------------------------------

/// Lightweight entry for rendering the timeline bar.
#[derive(Debug, Clone)]
pub struct RecTimelineEntry {
    pub seq: u64,
    pub stop_label: String,    // "step", "bp#1", "signal SIGSEGV", etc.
    pub source_loc: Option<String>, // "main.c:42"
    pub is_anchor: bool,       // breakpoint/watchpoint hit
}

/// Diff summary at the current playback position.
#[derive(Debug, Clone, Default)]
pub struct RecDiffSummary {
    pub vars_changed: Vec<(String, String, String)>,  // (name, old, new)
    pub vars_added: Vec<String>,
    pub vars_removed: Vec<String>,
    pub regs_changed: usize,
    pub mem_changed: usize,
    pub watches_changed: Vec<(String, String, String)>, // (expr, old, new)
    pub source_from: Option<String>,  // "file.c:10"
    pub source_to: Option<String>,    // "file.c:15"
    pub thread_changed: bool,
    pub stop_label: String,
}

// ---------------------------------------------------------------------------
// Execution flow data (computed from recording for playback display)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct ExecFlowData {
    /// source_path -> line_number -> hit count
    pub line_hits: HashMap<String, HashMap<u32, u32>>,
    /// disasm address -> hit count
    pub addr_hits: HashMap<u64, u32>,
    /// Total number of recorded states analyzed
    pub total_steps: usize,
}

// ---------------------------------------------------------------------------
// View state
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ViewState {
    pub panels_visible: HashSet<Panel>,
    pub focused_panel: Panel,

    // Per-panel scroll / selection
    pub source_scroll: usize,
    pub source_cursor: usize, // 1-based line number the user has navigated to
    pub source_follow_exec: bool, // auto-jump cursor to execution line on stop
    pub stack_selected: usize,
    pub threads_selected: usize,
    pub locals_selected: usize,
    pub breakpoints_selected: usize,
    pub registers_scroll: usize,
    pub memory_scroll: usize,
    pub mem_cursor: usize,       // byte offset within the loaded MemoryBlock
    pub mem_sel_start: Option<usize>, // start of selection range (byte offset)
    pub mem_sel_end: Option<usize>,   // end of selection range (exclusive)
    pub mem_edit: bool,          // true = typing hex digits to overwrite
    pub mem_edit_nibble: Option<u8>,  // first nibble of a two-nibble hex edit
    pub mem_cast: MemCast,       // current type interpretation for selection
    pub disasm_scroll: usize,
    pub disasm_cursor: usize,    // index into snap.disasm for cursor navigation
    pub watch_selected: usize,
    pub output_scroll: usize,
    pub output_follow: bool,

    // Input
    pub input_mode: InputMode,
    pub input_buffer: String,
    pub input_cursor: usize,

    // Command history
    pub command_history: Vec<String>,
    pub history_index: usize,
    pub last_command: Option<String>,

    // Auto-layout for no-symbols / RE mode
    pub layout_auto_switched: bool,

    // Overlays
    pub help_open: bool,
    pub help_scroll: u16,
    pub quit_confirm: bool,

    // Source search
    pub search_query: Option<String>,
    pub search_matches: Vec<usize>,
    pub search_current: usize,

    // Animation frame counter (increments each tick)
    pub tick_count: u64,

    // Timeline / playback
    pub playback_mode: bool,       // true = viewing a past recorded state
    pub playback_index: usize,     // index into recording.states
    pub timeline_scroll: usize,    // horizontal scroll for the timeline bar
    pub rec_count: usize,          // total recorded states
    pub rec_enabled: bool,         // recording on/off
    pub rec_entries: Vec<RecTimelineEntry>,   // lightweight timeline for rendering
    pub rec_diff: Option<RecDiffSummary>,     // diff at current playback position
    pub rec_playback_source_loc: Option<String>, // source location at playback index
    pub rec_playback_snap: Option<GdbSnapshot>,  // reconstructed snapshot for playback panels
    pub playback_source_cache: std::collections::HashMap<String, crate::state::SourceFile>,

    // Execution flow analysis
    pub exec_flow: Option<ExecFlowData>,
    pub exec_flow_computed_at: usize,
}

impl Default for ViewState {
    fn default() -> Self {
        let mut visible = HashSet::new();
        for &panel in Panel::all() {
            if panel.default_visible() {
                visible.insert(panel);
            }
        }
        Self {
            panels_visible: visible,
            focused_panel: Panel::Source,

            source_scroll: 0,
            source_cursor: 1,
            source_follow_exec: true,
            stack_selected: 0,
            threads_selected: 0,
            locals_selected: 0,
            breakpoints_selected: 0,
            registers_scroll: 0,
            memory_scroll: 0,
            mem_cursor: 0,
            mem_sel_start: None,
            mem_sel_end: None,
            mem_edit: false,
            mem_edit_nibble: None,
            mem_cast: MemCast::Hex,
            disasm_scroll: 0,
            disasm_cursor: 0,
            watch_selected: 0,
            output_scroll: 0,
            output_follow: true,

            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            input_cursor: 0,

            command_history: Vec::new(),
            history_index: 0,
            last_command: None,

            layout_auto_switched: false,

            help_open: false,
            help_scroll: 0,
            quit_confirm: false,

            search_query: None,
            search_matches: Vec::new(),
            search_current: 0,

            tick_count: 0,

            playback_mode: false,
            playback_index: 0,
            timeline_scroll: 0,
            rec_count: 0,
            rec_enabled: true,
            rec_entries: Vec::new(),
            rec_diff: None,
            rec_playback_source_loc: None,
            rec_playback_snap: None,
            playback_source_cache: std::collections::HashMap::new(),

            exec_flow: None,
            exec_flow_computed_at: 0,
        }
    }
}

impl ViewState {
    /// Return the visible panels in their canonical ordering.
    pub fn visible_panels_ordered(&self) -> Vec<Panel> {
        Panel::all()
            .iter()
            .copied()
            .filter(|p| self.panels_visible.contains(p))
            .collect()
    }

    /// Cycle the focused panel forward through the visible panels.
    fn cycle_focus_forward(&mut self) {
        let ordered = self.visible_panels_ordered();
        if ordered.is_empty() {
            return;
        }
        let pos = ordered
            .iter()
            .position(|p| *p == self.focused_panel)
            .unwrap_or(0);
        let next = (pos + 1) % ordered.len();
        self.focused_panel = ordered[next];
    }

    /// Cycle the focused panel backward through the visible panels.
    fn cycle_focus_back(&mut self) {
        let ordered = self.visible_panels_ordered();
        if ordered.is_empty() {
            return;
        }
        let pos = ordered
            .iter()
            .position(|p| *p == self.focused_panel)
            .unwrap_or(0);
        let prev = if pos == 0 { ordered.len() - 1 } else { pos - 1 };
        self.focused_panel = ordered[prev];
    }

    /// Start input mode: set mode, clear buffer, position cursor at 0.
    fn start_input(&mut self, mode: InputMode) {
        self.input_mode = mode;
        self.input_buffer.clear();
        self.input_cursor = 0;
        self.history_index = self.command_history.len();
    }

    /// Start input mode with a prefilled value.
    fn start_input_with(&mut self, mode: InputMode, prefill: String) {
        self.input_mode = mode;
        self.input_cursor = prefill.len();
        self.input_buffer = prefill;
        self.history_index = self.command_history.len();
    }

    /// Cancel input mode and return to normal.
    fn cancel_input(&mut self) {
        self.input_mode = InputMode::Normal;
        self.input_buffer.clear();
        self.input_cursor = 0;
    }

    /// Insert a character at the current cursor position.
    fn input_insert(&mut self, c: char) {
        self.input_buffer.insert(self.input_cursor, c);
        self.input_cursor += c.len_utf8();
    }

    /// Delete the character before the cursor (backspace).
    fn input_backspace(&mut self) {
        if self.input_cursor > 0 {
            // Find the previous char boundary
            let prev = self.input_buffer[..self.input_cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.input_buffer.remove(prev);
            self.input_cursor = prev;
        }
    }

    /// Delete the character at the cursor (delete key).
    fn input_delete(&mut self) {
        if self.input_cursor < self.input_buffer.len() {
            self.input_buffer.remove(self.input_cursor);
        }
    }

    /// Move cursor left by one character.
    fn input_left(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor = self.input_buffer[..self.input_cursor]
                .char_indices()
                .last()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    /// Move cursor right by one character.
    fn input_right(&mut self) {
        if self.input_cursor < self.input_buffer.len() {
            self.input_cursor += self.input_buffer[self.input_cursor..]
                .chars()
                .next()
                .map(|c| c.len_utf8())
                .unwrap_or(0);
        }
    }

    /// Navigate to a previous command in history.
    fn history_up(&mut self) {
        if !self.command_history.is_empty() && self.history_index > 0 {
            self.history_index -= 1;
            self.input_buffer = self.command_history[self.history_index].clone();
            self.input_cursor = self.input_buffer.len();
        }
    }

    /// Navigate to a more recent command in history.
    fn history_down(&mut self) {
        if self.history_index < self.command_history.len() {
            self.history_index += 1;
            if self.history_index < self.command_history.len() {
                self.input_buffer = self.command_history[self.history_index].clone();
            } else {
                self.input_buffer.clear();
            }
            self.input_cursor = self.input_buffer.len();
        }
    }

    /// Return the count of items in the currently focused panel (for up/down
    /// selection clamping).
    fn focused_item_count(&self, snap: &GdbSnapshot) -> usize {
        match self.focused_panel {
            Panel::Stack => snap.stack.len(),
            Panel::Locals => snap.locals.len(),
            Panel::Threads => snap.threads.len(),
            Panel::Breakpoints => snap.breakpoints.len(),
            Panel::Watch => snap.watch_expressions.len(),
            Panel::Source => snap.source.as_ref().map_or(0, |s| s.lines.len()),
            Panel::Registers => snap.registers.len(),
            Panel::Output => snap.output.len(),
            Panel::Memory => {
                snap.memory
                    .as_ref()
                    .map_or(0, |m| (m.bytes.len() + 15) / 16)
            }
            Panel::Disasm => snap.disasm.len(),
        }
    }

    /// Get a mutable reference to the scroll/selection value for the focused
    /// panel.
    fn focused_selection_mut(&mut self) -> &mut usize {
        match self.focused_panel {
            Panel::Stack => &mut self.stack_selected,
            Panel::Locals => &mut self.locals_selected,
            Panel::Threads => &mut self.threads_selected,
            Panel::Breakpoints => &mut self.breakpoints_selected,
            Panel::Watch => &mut self.watch_selected,
            Panel::Source => &mut self.source_cursor,
            Panel::Registers => &mut self.registers_scroll,
            Panel::Output => &mut self.output_scroll,
            Panel::Memory => &mut self.memory_scroll,
            Panel::Disasm => &mut self.disasm_cursor,
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the TUI event loop, taking ownership of the terminal.
///
/// Returns when the user quits or an unrecoverable error occurs.
pub async fn run(
    state: SharedState,
    cmd_tx: mpsc::Sender<GdbCommand>,
    redraw_hz: u32,
    recording: SharedRecording,
) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = event_loop(&mut terminal, state, cmd_tx, redraw_hz, recording).await;

    // Cleanup -- always restore the terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

// ---------------------------------------------------------------------------
// Event loop
// ---------------------------------------------------------------------------

async fn event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: SharedState,
    cmd_tx: mpsc::Sender<GdbCommand>,
    redraw_hz: u32,
    recording: SharedRecording,
) -> Result<()> {
    let mut view = ViewState::default();
    let tick_rate = Duration::from_millis(1000 / redraw_hz as u64);
    let mut tick = tokio::time::interval(tick_rate);
    let mut prev_source_line: Option<u32> = None;

    loop {
        tick.tick().await;

        // Drain all pending crossterm events
        while event::poll(Duration::ZERO)? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Memory edit mode intercepts hex digit keys
                if view.input_mode == InputMode::Normal
                    && view.focused_panel == Panel::Memory
                    && view.mem_edit
                {
                    use crossterm::event::KeyCode;
                    if let KeyCode::Char(c) = key.code {
                        if c.is_ascii_hexdigit() {
                            if dispatch_mem_hex(&mut view, &state, &cmd_tx, c).await {
                                continue;
                            }
                        }
                    }
                    if key.code == KeyCode::Esc {
                        view.mem_edit = false;
                        view.mem_edit_nibble = None;
                        continue;
                    }
                }

                // Help overlay intercepts all keys when open
                if view.help_open {
                    use crossterm::event::KeyCode;
                    match key.code {
                        KeyCode::Char('j') | KeyCode::Down => {
                            view.help_scroll = view.help_scroll.saturating_add(1);
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            view.help_scroll = view.help_scroll.saturating_sub(1);
                        }
                        KeyCode::PageDown => {
                            view.help_scroll = view.help_scroll.saturating_add(20);
                        }
                        KeyCode::PageUp => {
                            view.help_scroll = view.help_scroll.saturating_sub(20);
                        }
                        KeyCode::Char('g') | KeyCode::Home => {
                            view.help_scroll = 0;
                        }
                        KeyCode::Char('G') | KeyCode::End => {
                            view.help_scroll = 200; // past the end, clamps in render
                        }
                        KeyCode::Char('q') | KeyCode::Char('?') | KeyCode::Esc | KeyCode::F(1) => {
                            view.help_open = false;
                        }
                        _ => {}
                    }
                    continue;
                }

                let action = match view.input_mode {
                    InputMode::Normal => input::map_normal(key, view.quit_confirm),
                    _ => input::map_input(key),
                };

                if dispatch_action(action, &mut view, &state, &cmd_tx, &recording).await {
                    return Ok(());
                }
            }
        }

        let live_snap = state.load();

        // Sync recording timeline data into ViewState for rendering
        sync_recording_view(&mut view, &recording);

        // Decide which snapshot to render: playback or live
        let render_snap = if view.playback_mode {
            if let Some(ref mut pb) = view.rec_playback_snap {
                // Try to fill in the source file for the playback state
                if pb.source.is_none() {
                    // Find the source path: try current frame level, then walk
                    // up the stack to find ANY frame with source.
                    let pb_path = pb.stack.iter()
                        .find(|f| f.level == pb.current_frame_level && f.fullname.is_some())
                        .or_else(|| pb.stack.iter().find(|f| f.fullname.is_some()))
                        .and_then(|f| f.fullname.clone());
                    if let (Some(ref live_src), Some(ref wanted)) = (&live_snap.source, &pb_path) {
                        if live_src.path == *wanted {
                            pb.source = Some(live_src.clone());
                        }
                    }
                    // Otherwise, check playback cache or load from disk
                    if pb.source.is_none() {
                        if let Some(ref path) = pb_path {
                            if let Some(cached) = view.playback_source_cache.get(path) {
                                pb.source = Some(cached.clone());
                            } else if let Ok(contents) = std::fs::read_to_string(path) {
                                let lines: Vec<String> = contents.lines().map(String::from).collect();
                                let highlighted = crate::highlight::highlight_lines(path, &lines);
                                let src = crate::state::SourceFile {
                                    path: path.clone(),
                                    lines,
                                    highlighted,
                                };
                                view.playback_source_cache.insert(path.clone(), src.clone());
                                pb.source = Some(src);
                            }
                        }
                    }
                }
                pb.breakpoints = live_snap.breakpoints.clone();
                pb.output = live_snap.output.clone();
                pb.target_executable = live_snap.target_executable.clone();
                pb.recording_count = live_snap.recording_count;

                // Update source cursor: use recorded source_line, or fall back
                // to the line from the frame that has source.
                let playback_line = pb.source_line.or_else(|| {
                    pb.stack.iter()
                        .find(|f| f.level == pb.current_frame_level && f.line.is_some())
                        .or_else(|| pb.stack.iter().find(|f| f.line.is_some()))
                        .and_then(|f| f.line)
                });
                if let Some(line) = playback_line {
                    pb.source_line = Some(line);
                    view.source_cursor = line as usize;
                }
            }
            view.rec_playback_snap.as_ref().unwrap_or(&live_snap)
        } else {
            // Auto-sync cursor to execution line when GDB stops at a new location
            if view.source_follow_exec {
                if let Some(line) = live_snap.source_line {
                    if prev_source_line != Some(line) {
                        view.source_cursor = line as usize;
                    }
                }
            }
            prev_source_line = live_snap.source_line;
            &live_snap
        };

        // Auto-switch to RE layout when no debug symbols detected
        if !view.layout_auto_switched {
            let no_debug = !render_snap.has_debug_info
                && render_snap.source.is_none()
                && render_snap.target_state == TargetState::Stopped
                && !render_snap.stack.is_empty();
            if no_debug {
                view.layout_auto_switched = true;
                // Switch to RE layout: Disasm + Registers + Memory + Stack + Breakpoints + Output
                view.panels_visible.clear();
                view.panels_visible.insert(Panel::Disasm);
                view.panels_visible.insert(Panel::Registers);
                view.panels_visible.insert(Panel::Memory);
                view.panels_visible.insert(Panel::Stack);
                view.panels_visible.insert(Panel::Breakpoints);
                view.panels_visible.insert(Panel::Output);
                view.focused_panel = Panel::Disasm;
            }
        }

        view.tick_count = view.tick_count.wrapping_add(1);
        terminal.draw(|f| draw(f, render_snap, &view))?;
    }
}

/// Read the Recording once per frame and populate ViewState fields for the
/// timeline panel to render from.  The lock is held only for the duration of
/// this function (microseconds).
fn sync_recording_view(view: &mut ViewState, recording: &SharedRecording) {
    // Acquire the lock BRIEFLY — just read the count and any new entries.
    // Avoid holding the lock while doing expensive work, since the controller
    // also needs it to capture states during tracing.
    let (rec_len, rec_enabled, new_entries) = {
        let rec = match recording.lock() {
            Ok(r) => r,
            Err(poisoned) => poisoned.into_inner(),
        };

        let len = rec.len();
        let enabled = rec.enabled;

        // Incrementally append new entries (don't rebuild the whole list)
        let mut new = Vec::new();
        if len > view.rec_entries.len() {
            for i in view.rec_entries.len()..len {
                if let Some(state) = rec.get(i) {
                    let stop_label = stop_reason_label(&state.stop_reason);
                    let source_loc = match (&state.source_path, state.source_line) {
                        (Some(path), Some(line)) => {
                            let short = path.rsplit('/').next().unwrap_or(path);
                            Some(format!("{}:{}", short, line))
                        }
                        _ => None,
                    };
                    new.push(RecTimelineEntry {
                        seq: state.seq,
                        stop_label,
                        source_loc,
                        is_anchor: state.is_anchor,
                    });
                }
            }
        } else if len < view.rec_entries.len() {
            // Recording was cleared or entries expired
            new.clear();
        }

        (len, enabled, new)
    };
    // Lock is released here.

    view.rec_count = rec_len;
    view.rec_enabled = rec_enabled;

    if rec_len < view.rec_entries.len() {
        // Recording shrunk (clear + rebuild would need the lock again;
        // just truncate for now — full rebuild on next frame if needed)
        view.rec_entries.clear();
        view.exec_flow = None;
        view.exec_flow_computed_at = 0;
    }
    view.rec_entries.extend(new_entries);

    // Build execution flow data — but only when NOT actively tracing,
    // to avoid expensive full scans that block the controller.
    let is_tracing = {
        let snap_state = view.rec_count > view.exec_flow_computed_at;
        snap_state && rec_enabled
    };
    if rec_len > 0 && rec_len != view.exec_flow_computed_at && !is_tracing {
        // Full scan — only runs when tracing has stopped and count is stable
        let rec = match recording.lock() {
            Ok(r) => r,
            Err(_) => return, // skip if we can't get the lock
        };

        let mut line_hits: std::collections::HashMap<String, std::collections::HashMap<u32, u32>> = std::collections::HashMap::new();
        let mut addr_hits: std::collections::HashMap<u64, u32> = std::collections::HashMap::new();

        for i in 0..rec.len() {
            if let Some(state) = rec.get(i) {
                if let (Some(path), Some(line)) = (&state.source_path, state.source_line) {
                    *line_hits
                        .entry(path.clone())
                        .or_default()
                        .entry(line)
                        .or_default() += 1;
                }
                if let Some(frame) = state.stack.first() {
                    *addr_hits.entry(frame.addr).or_default() += 1;
                }
            }
        }

        drop(rec);

        view.exec_flow = Some(ExecFlowData {
            line_hits,
            addr_hits,
            total_steps: rec_len,
        });
        view.exec_flow_computed_at = rec_len;
    } else if rec_len == 0 {
        view.exec_flow = None;
        view.exec_flow_computed_at = 0;
    }

    // Playback state (only needed when in playback mode, brief lock)
    if view.playback_mode {
        if let Ok(rec) = recording.lock() {
            if !rec.is_empty() {
                view.playback_index = view.playback_index.min(rec.len().saturating_sub(1));
            }

            if let Some(diff) = rec.get_diff(view.playback_index) {
                let state = rec.get(view.playback_index);
                let prev_state = if view.playback_index > 0 {
                    rec.get(view.playback_index - 1)
                } else {
                    None
                };

                let source_from = prev_state.and_then(|s| {
                    match (&s.source_path, s.source_line) {
                        (Some(p), Some(l)) => {
                            let short = p.rsplit('/').next().unwrap_or(p);
                            Some(format!("{}:{}", short, l))
                        }
                        _ => None,
                    }
                });
                let source_to = state.and_then(|s| {
                    match (&s.source_path, s.source_line) {
                        (Some(p), Some(l)) => {
                            let short = p.rsplit('/').next().unwrap_or(p);
                            Some(format!("{}:{}", short, l))
                        }
                        _ => None,
                    }
                });

                view.rec_diff = Some(RecDiffSummary {
                    vars_changed: diff
                        .locals_changed
                        .iter()
                        .map(|c| (c.name.clone(), c.old_value.clone(), c.new_value.clone()))
                        .collect(),
                    vars_added: diff.locals_added.clone(),
                    vars_removed: diff.locals_removed.clone(),
                    regs_changed: diff.registers_changed.len(),
                    mem_changed: diff.memory_changed.len(),
                    watches_changed: diff
                        .watches_changed
                        .iter()
                        .map(|c| {
                            (
                                c.expression.clone(),
                                c.old_value.clone(),
                                c.new_value.clone(),
                            )
                        })
                        .collect(),
                    source_from,
                    source_to,
                    thread_changed: diff.thread_changed,
                    stop_label: state
                        .map(|s| stop_reason_label(&s.stop_reason))
                        .unwrap_or_default(),
                });
            } else {
                let state = rec.get(view.playback_index);
                view.rec_diff = Some(RecDiffSummary {
                    stop_label: state
                        .map(|s| stop_reason_label(&s.stop_reason))
                        .unwrap_or_default(),
                    ..Default::default()
                });
            }

            view.rec_playback_source_loc = rec.get(view.playback_index).and_then(|s| {
                match (&s.source_path, s.source_line) {
                    (Some(p), Some(l)) => {
                        let short = p.rsplit('/').next().unwrap_or(p);
                        Some(format!("{}:{}", short, l))
                    }
                    _ => None,
                }
            });
            if let Some(rs) = rec.get(view.playback_index) {
                view.rec_playback_snap = Some(build_playback_snapshot(rs));
            }
        }
    } else {
        view.rec_diff = None;
        view.rec_playback_source_loc = None;
        view.rec_playback_snap = None;
    }
}

/// Reconstruct a GdbSnapshot from a RecordedState for playback rendering.
fn build_playback_snapshot(rs: &crate::recording::RecordedState) -> GdbSnapshot {
    use crate::state::*;
    GdbSnapshot {
        target_state: TargetState::Stopped,
        stop_reason: rs.stop_reason.clone(),
        threads: Vec::new(),
        current_thread_id: rs.thread_id,
        stack: rs.stack.clone(),
        current_frame_level: rs.frame_level,
        locals: rs.locals.clone(),
        breakpoints: Vec::new(),
        registers: rs.registers.clone(),
        register_names: Vec::new(),
        memory: rs.memory.clone(),
        memory_address: rs.memory.as_ref().map_or(0, |m| m.address),
        disasm: rs.disasm.clone(),
        xrefs: Vec::new(),
        type_overlay: None,
        watch_expressions: rs
            .watch_values
            .iter()
            .map(|(expr, val)| WatchExpression {
                id: 0,
                expression: expr.clone(),
                value: val.clone(),
                type_name: String::new(),
                error: None,
            })
            .collect(),
        mapped_libs: Vec::new(),
        source: None, // filled in below from live snapshot
        source_line: rs.source_line,
        source_loading: false,
        output: Vec::new(),
        status_message: None,
        target_executable: None,
        recording_count: 0,
        has_debug_info: false,
    }
}

/// Convert a StopReason to a short label for the timeline.
fn stop_reason_label(reason: &Option<StopReason>) -> String {
    match reason {
        Some(StopReason::StepFinished) => "step".to_string(),
        Some(StopReason::BreakpointHit { number }) => format!("bp#{}", number),
        Some(StopReason::Watchpoint { number }) => format!("wp#{}", number),
        Some(StopReason::SignalReceived { name, .. }) => format!("sig:{}", name),
        Some(StopReason::FunctionFinished) => "return".to_string(),
        Some(StopReason::ExitedNormally { code }) => format!("exit({})", code),
        Some(StopReason::Unknown(s)) => {
            if s.len() > 12 {
                format!("{}...", &s[..12])
            } else {
                s.clone()
            }
        }
        None => "stop".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Action dispatcher
// ---------------------------------------------------------------------------

/// Dispatch a single action. Returns `true` if the application should exit.
async fn dispatch_action(
    action: Action,
    view: &mut ViewState,
    state: &SharedState,
    cmd_tx: &mpsc::Sender<GdbCommand>,
    recording: &SharedRecording,
) -> bool {
    match action {
        Action::None => {}

        // ---- Quit ----
        Action::Quit => {
            if view.help_open {
                view.help_open = false;
            } else if view.focused_panel == Panel::Memory {
                if view.mem_edit {
                    view.mem_edit = false;
                    view.mem_edit_nibble = None;
                } else if view.mem_sel_start.is_some() {
                    view.mem_sel_start = None;
                    view.mem_sel_end = None;
                } else {
                    // Leave memory panel — return focus to source
                    view.focused_panel = Panel::Source;
                }
            } else {
                view.quit_confirm = true;
            }
        }
        Action::ConfirmQuit => {
            let _ = cmd_tx.send(GdbCommand::Quit).await;
            return true;
        }
        Action::CancelQuit => {
            view.quit_confirm = false;
        }

        // ---- Execution ----
        Action::RunContinue => {
            let snap = state.load();
            let cmd = match snap.target_state {
                TargetState::NotStarted => GdbCommand::Run(vec![]),
                TargetState::Stopped => GdbCommand::Continue,
                _ => return false,
            };
            view.source_follow_exec = true;
            view.playback_mode = false;
            let _ = cmd_tx.send(cmd).await;
        }
        Action::TraceContinue => {
            let snap = state.load();
            if matches!(snap.target_state, TargetState::Stopped) {
                view.source_follow_exec = true;
                view.playback_mode = false;
                let _ = cmd_tx.send(GdbCommand::TraceContinue).await;
            }
        }
        Action::StepInto => {
            let snap = state.load();
            if matches!(snap.target_state, TargetState::Stopped) {
                view.source_follow_exec = true;
                let _ = cmd_tx.send(GdbCommand::StepInto).await;
            }
        }
        Action::StepOver => {
            let snap = state.load();
            if matches!(snap.target_state, TargetState::Stopped) {
                view.source_follow_exec = true;
                let _ = cmd_tx.send(GdbCommand::StepOver).await;
            }
        }
        Action::StepOut => {
            let snap = state.load();
            if matches!(snap.target_state, TargetState::Stopped) {
                view.source_follow_exec = true;
                let _ = cmd_tx.send(GdbCommand::StepOut).await;
            }
        }
        Action::Interrupt => {
            let _ = cmd_tx.send(GdbCommand::Interrupt).await;
        }

        // ---- Navigation ----
        Action::SelectUp => {
            if view.focused_panel == Panel::Source {
                view.source_cursor = view.source_cursor.saturating_sub(1).max(1);
                view.source_follow_exec = false;
            } else if view.focused_panel == Panel::Memory {
                let snap = state.load();
                let mem_len = snap.memory.as_ref().map_or(0, |m| m.bytes.len());
                if mem_len > 0 {
                    view.mem_cursor = view.mem_cursor.saturating_sub(16);
                    if view.mem_sel_start.is_some() {
                        view.mem_sel_end = Some(view.mem_cursor);
                    }
                }
            } else {
                let snap = state.load();
                let count = view.focused_item_count(&snap);
                if count > 0 {
                    let sel = view.focused_selection_mut();
                    *sel = sel.saturating_sub(1);
                }
                if view.focused_panel == Panel::Output {
                    view.output_follow = false;
                }
            }
        }
        Action::SelectDown => {
            if view.focused_panel == Panel::Source {
                let snap = state.load();
                let line_count = snap.source.as_ref().map_or(0, |s| s.lines.len());
                if line_count > 0 {
                    view.source_cursor = (view.source_cursor + 1).min(line_count);
                }
                view.source_follow_exec = false;
            } else if view.focused_panel == Panel::Memory {
                let snap = state.load();
                let mem_len = snap.memory.as_ref().map_or(0, |m| m.bytes.len());
                if mem_len > 0 {
                    view.mem_cursor = (view.mem_cursor + 16).min(mem_len.saturating_sub(1));
                    if view.mem_sel_start.is_some() {
                        view.mem_sel_end = Some(view.mem_cursor);
                    }
                }
            } else {
                let snap = state.load();
                let count = view.focused_item_count(&snap);
                if count > 0 {
                    let sel = view.focused_selection_mut();
                    *sel = (*sel + 1).min(count.saturating_sub(1));
                }
                if view.focused_panel == Panel::Output {
                    let total = snap.output.len();
                    if view.output_scroll >= total.saturating_sub(1) {
                        view.output_follow = true;
                    }
                }
            }
        }
        Action::GoTop => {
            if view.focused_panel == Panel::Source {
                view.source_cursor = 1;
                view.source_follow_exec = false;
            } else {
                let sel = view.focused_selection_mut();
                *sel = 0;
                if view.focused_panel == Panel::Output {
                    view.output_follow = false;
                }
            }
        }
        Action::GoBottom => {
            if view.focused_panel == Panel::Source {
                let snap = state.load();
                let line_count = snap.source.as_ref().map_or(0, |s| s.lines.len());
                if line_count > 0 {
                    view.source_cursor = line_count;
                }
                view.source_follow_exec = false;
            } else {
                let snap = state.load();
                let count = view.focused_item_count(&snap);
                if count > 0 {
                    let sel = view.focused_selection_mut();
                    *sel = count.saturating_sub(1);
                }
                if view.focused_panel == Panel::Output {
                    view.output_follow = true;
                }
            }
        }
        Action::Enter => {
            let snap = state.load();
            match view.focused_panel {
                Panel::Source => {
                    if let Some(ref src) = snap.source {
                        if view.source_cursor >= 1 && view.source_cursor <= src.lines.len() {
                            let location = format!("{}:{}", src.path, view.source_cursor);
                            let _ = cmd_tx
                                .send(GdbCommand::SetBreakpoint(location))
                                .await;
                        }
                    }
                }
                Panel::Stack => {
                    if let Some(frame) = snap.stack.get(view.stack_selected) {
                        let _ = cmd_tx.send(GdbCommand::SelectFrame(frame.level)).await;
                        view.source_follow_exec = true;
                    }
                }
                Panel::Threads => {
                    if let Some(thread) = snap.threads.get(view.threads_selected) {
                        let _ = cmd_tx.send(GdbCommand::SelectThread(thread.id)).await;
                        view.source_follow_exec = true;
                    }
                }
                Panel::Disasm => {
                    // Set breakpoint at cursor address
                    if let Some(inst) = snap.disasm.get(view.disasm_cursor) {
                        let location = format!("*0x{:x}", inst.address);
                        let _ = cmd_tx
                            .send(GdbCommand::SetBreakpoint(location))
                            .await;
                    }
                }
                Panel::Memory => {
                    // Follow pointer at cursor position
                    if let Some(ref mem) = snap.memory {
                        let cursor = view.mem_cursor;
                        if cursor + 8 <= mem.bytes.len() {
                            let addr = u64::from_le_bytes(
                                mem.bytes[cursor..cursor + 8].try_into().unwrap()
                            );
                            if addr != 0 {
                                let _ = cmd_tx
                                    .send(GdbCommand::ReadMemory { addr, count: 256 })
                                    .await;
                                view.mem_cursor = 0;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Action::CycleFocusForward => {
            view.cycle_focus_forward();
        }
        Action::CycleFocusBack => {
            view.cycle_focus_back();
        }

        // ---- Panels ----
        Action::TogglePanel(idx) => {
            if let Some(&panel) = Panel::all().get(idx) {
                if view.panels_visible.contains(&panel) {
                    view.panels_visible.remove(&panel);
                    // If we just hid the focused panel, move focus
                    if view.focused_panel == panel {
                        let ordered = view.visible_panels_ordered();
                        if let Some(&first) = ordered.first() {
                            view.focused_panel = first;
                        }
                    }
                } else {
                    view.panels_visible.insert(panel);
                }
            }
        }

        // ---- Breakpoints ----
        Action::ToggleBreakAtLine => {
            let snap = state.load();
            if view.focused_panel == Panel::Disasm {
                // Toggle breakpoint at disasm cursor address
                if let Some(inst) = snap.disasm.get(view.disasm_cursor) {
                    let addr = inst.address;
                    let existing = snap.breakpoints.iter().find(|bp| {
                        bp.address == Some(addr)
                    });
                    if let Some(bp) = existing {
                        let _ = cmd_tx
                            .send(GdbCommand::DeleteBreakpoint(bp.number))
                            .await;
                    } else {
                        let location = format!("*0x{:x}", addr);
                        let _ = cmd_tx
                            .send(GdbCommand::SetBreakpoint(location))
                            .await;
                    }
                }
            } else if let Some(ref src) = snap.source {
                let line = view.source_cursor;
                if line >= 1 && line <= src.lines.len() {
                    let existing = snap.breakpoints.iter().find(|bp| {
                        bp.line == Some(line as u32)
                            && bp
                                .file
                                .as_ref()
                                .map_or(false, |f| src.path.ends_with(f.as_str()))
                    });
                    if let Some(bp) = existing {
                        let _ = cmd_tx
                            .send(GdbCommand::DeleteBreakpoint(bp.number))
                            .await;
                    } else {
                        let location = format!("{}:{}", src.path, line);
                        let _ = cmd_tx
                            .send(GdbCommand::SetBreakpoint(location))
                            .await;
                    }
                }
            }
        }
        Action::PromptBreakpoint => {
            view.start_input(InputMode::Breakpoint);
        }
        Action::PromptBreakpointCond => {
            view.start_input(InputMode::BreakpointCond);
        }
        Action::PromptBreakCondEdit => {
            let snap = state.load();
            if let Some(bp) = snap.breakpoints.get(view.breakpoints_selected) {
                let prefill = bp.condition.clone().unwrap_or_default();
                // Store the BP number in the input buffer as "number condition"
                // We parse it back out on submit.
                let tagged = format!("{} {}", bp.number, prefill);
                view.start_input_with(InputMode::BreakCondEdit, tagged);
            }
        }
        Action::PromptWatchpoint => {
            view.start_input(InputMode::Watchpoint);
        }
        Action::PromptRegisterEdit => {
            let snap = state.load();
            if view.focused_panel == Panel::Registers {
                if let Some(reg) = snap.registers.get(view.registers_scroll) {
                    let prefill = format!("{} {}", reg.name, reg.value);
                    view.start_input_with(InputMode::RegisterEdit, prefill);
                }
            }
        }
        Action::DeleteBreakpoint => {
            let snap = state.load();
            if view.focused_panel == Panel::Watch {
                if let Some(w) = snap.watch_expressions.get(view.watch_selected) {
                    let _ = cmd_tx.send(GdbCommand::RemoveWatch(w.id)).await;
                }
            } else if let Some(bp) = snap.breakpoints.get(view.breakpoints_selected) {
                let _ = cmd_tx.send(GdbCommand::DeleteBreakpoint(bp.number)).await;
            }
        }
        Action::ToggleEnableBreak => {
            let snap = state.load();
            if let Some(bp) = snap.breakpoints.get(view.breakpoints_selected) {
                let _ = cmd_tx.send(GdbCommand::ToggleBreakpoint(bp.number)).await;
            }
        }

        // ---- Inspection prompts ----
        Action::PromptWatch => {
            let snap = state.load();
            match get_prefill(view, &snap, InputMode::Watch) {
                Some(prefill) => view.start_input_with(InputMode::Watch, prefill),
                None => view.start_input(InputMode::Watch),
            }
        }
        Action::PromptMemory => {
            let snap = state.load();
            match get_prefill(view, &snap, InputMode::Memory) {
                Some(prefill) => view.start_input_with(InputMode::Memory, prefill),
                None => view.start_input(InputMode::Memory),
            }
        }
        Action::PromptEval => {
            let snap = state.load();
            match get_prefill(view, &snap, InputMode::Eval) {
                Some(prefill) => view.start_input_with(InputMode::Eval, prefill),
                None => view.start_input(InputMode::Eval),
            }
        }

        // ---- Command ----
        Action::PromptCommand => {
            view.start_input(InputMode::Command);
        }
        Action::RepeatLastCommand => {
            if let Some(ref cmd) = view.last_command.clone() {
                let _ = cmd_tx.send(GdbCommand::RawCommand(cmd.clone())).await;
            }
        }

        // ---- Help ----
        Action::ToggleHelp => {
            view.help_open = !view.help_open;
            view.help_scroll = 0;
        }

        // ---- Input mode ----
        Action::InputSubmit => {
            let buf = view.input_buffer.clone();
            let mode = view.input_mode;
            view.cancel_input();

            if buf.is_empty() {
                return false;
            }

            // Save to history
            view.command_history.push(buf.clone());
            view.last_command = Some(buf.clone());

            match mode {
                InputMode::Command => {
                    let _ = cmd_tx.send(GdbCommand::RawCommand(buf)).await;
                }
                InputMode::Breakpoint => {
                    let _ = cmd_tx.send(GdbCommand::SetBreakpoint(buf)).await;
                }
                InputMode::Watch => {
                    let _ = cmd_tx.send(GdbCommand::AddWatch(buf)).await;
                }
                InputMode::Memory => {
                    let parts: Vec<&str> = buf.split_whitespace().collect();
                    if let Some(expr) = parts.first() {
                        let count = parts
                            .get(1)
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(256);

                        // Try parsing as a raw hex address first
                        let stripped = expr
                            .trim_start_matches("0x")
                            .trim_start_matches("0X");
                        if !expr.starts_with('&')
                            && u64::from_str_radix(stripped, 16).is_ok()
                        {
                            let addr = u64::from_str_radix(stripped, 16).unwrap();
                            let _ = cmd_tx
                                .send(GdbCommand::ReadMemory { addr, count })
                                .await;
                        } else {
                            // Expression (e.g. "&my_var", "buf", "ptr->data")
                            // — ask GDB to evaluate it to a pointer
                            let _ = cmd_tx
                                .send(GdbCommand::ReadMemoryExpr {
                                    expr: expr.to_string(),
                                    count,
                                })
                                .await;
                        }
                    }
                }
                InputMode::Eval => {
                    let _ = cmd_tx.send(GdbCommand::EvaluateExpression(buf)).await;
                }
                InputMode::BreakpointCond => {
                    // Parse: "location if condition" or "location"
                    if let Some(idx) = buf.find(" if ") {
                        let location = buf[..idx].trim().to_string();
                        let condition = buf[idx + 4..].trim().to_string();
                        if !location.is_empty() && !condition.is_empty() {
                            let _ = cmd_tx
                                .send(GdbCommand::SetBreakpointCond { location, condition })
                                .await;
                        } else if !location.is_empty() {
                            let _ = cmd_tx
                                .send(GdbCommand::SetBreakpoint(location))
                                .await;
                        }
                    } else {
                        // No condition — set as normal breakpoint
                        let _ = cmd_tx.send(GdbCommand::SetBreakpoint(buf)).await;
                    }
                }
                InputMode::BreakCondEdit => {
                    // Buffer is "number condition_text"
                    // The first token is the breakpoint number; the rest is the condition.
                    let parts: Vec<&str> = buf.splitn(2, ' ').collect();
                    if let Some(num_str) = parts.first() {
                        if let Ok(number) = num_str.parse::<u32>() {
                            let condition = parts
                                .get(1)
                                .map(|s| s.trim().to_string())
                                .unwrap_or_default();
                            if !condition.is_empty() {
                                let _ = cmd_tx
                                    .send(GdbCommand::BreakCondition { number, condition })
                                    .await;
                            }
                        }
                    }
                }
                InputMode::Watchpoint => {
                    // Parse: "expr [r|w|rw|a]"
                    // Default is write. Trailing modifier selects kind.
                    use crate::gdb::mi_command::WatchKind;
                    let trimmed = buf.trim();
                    let (expr, kind) = if let Some(stripped) = trimmed.strip_suffix(" rw") {
                        (stripped.to_string(), WatchKind::Access)
                    } else if let Some(stripped) = trimmed.strip_suffix(" a") {
                        (stripped.to_string(), WatchKind::Access)
                    } else if let Some(stripped) = trimmed.strip_suffix(" r") {
                        (stripped.to_string(), WatchKind::Read)
                    } else if let Some(stripped) = trimmed.strip_suffix(" w") {
                        (stripped.to_string(), WatchKind::Write)
                    } else {
                        (trimmed.to_string(), WatchKind::Write)
                    };
                    if !expr.is_empty() {
                        let _ = cmd_tx
                            .send(GdbCommand::SetWatchpoint { expr, kind })
                            .await;
                    }
                }
                InputMode::RegisterEdit => {
                    // Buffer is "name value"
                    let parts: Vec<&str> = buf.splitn(2, ' ').collect();
                    if parts.len() == 2 {
                        let name = parts[0].trim().to_string();
                        let value = parts[1].trim().to_string();
                        if !name.is_empty() && !value.is_empty() {
                            let _ = cmd_tx
                                .send(GdbCommand::SetRegister { name, value })
                                .await;
                        }
                    }
                }
                InputMode::Search => {
                    // Perform source search
                    let snap = state.load();
                    view.search_query = Some(buf.clone());
                    view.search_matches.clear();
                    view.search_current = 0;
                    if let Some(ref src) = snap.source {
                        for (i, line) in src.lines.iter().enumerate() {
                            if line.contains(&buf) {
                                view.search_matches.push(i + 1); // 1-based
                            }
                        }
                    }
                    if let Some(&first) = view.search_matches.first() {
                        view.source_cursor = first;
                        view.source_follow_exec = false;
                    }
                }
                InputMode::SearchMemory => {
                    // Format: "pattern [start_addr [length]]"
                    // If pattern starts with \x, treat as hex bytes.
                    // Otherwise, treat as string search.
                    let snap = state.load();
                    let mem_start = snap.memory_address;
                    let mem_len = snap.memory.as_ref().map_or(0, |m| m.bytes.len());
                    drop(snap);

                    // Use a generous default search range if memory is loaded,
                    // otherwise search from 0 with a large range.
                    let (default_start, default_len) = if mem_len > 0 {
                        (mem_start, 0x100000u64) // 1 MiB from current memory address
                    } else {
                        (0u64, 0x100000u64)
                    };

                    let parts: Vec<&str> = buf.splitn(3, ' ').collect();
                    let pattern = parts.first().map(|s| *s).unwrap_or("");
                    let start = parts
                        .get(1)
                        .and_then(|s| {
                            let stripped = s.trim_start_matches("0x").trim_start_matches("0X");
                            u64::from_str_radix(stripped, 16).ok()
                        })
                        .unwrap_or(default_start);
                    let length = parts
                        .get(2)
                        .and_then(|s| {
                            let stripped = s.trim_start_matches("0x").trim_start_matches("0X");
                            u64::from_str_radix(stripped, 16)
                                .ok()
                                .or_else(|| s.parse::<u64>().ok())
                        })
                        .unwrap_or(default_len);

                    if pattern.starts_with("\\x") || pattern.starts_with("0x") {
                        // Parse as hex bytes: \xNN\xNN... or 0xNN0xNN...
                        let hex_str = pattern
                            .replace("\\x", "")
                            .replace("0x", "")
                            .replace(' ', "");
                        let mut bytes = Vec::new();
                        let mut chars = hex_str.chars();
                        let mut valid = true;
                        while let (Some(hi), Some(lo)) = (chars.next(), chars.next()) {
                            if let (Some(h), Some(l)) = (hi.to_digit(16), lo.to_digit(16)) {
                                bytes.push(((h << 4) | l) as u8);
                            } else {
                                valid = false;
                                break;
                            }
                        }
                        if valid && !bytes.is_empty() {
                            let _ = cmd_tx
                                .send(GdbCommand::SearchMemoryBytes { start, length, bytes })
                                .await;
                        }
                    } else if !pattern.is_empty() {
                        let _ = cmd_tx
                            .send(GdbCommand::SearchMemoryString {
                                start,
                                length,
                                pattern: pattern.to_string(),
                            })
                            .await;
                    }
                }
                InputMode::PatchBytes => {
                    // Format: "address hex_bytes..."
                    // e.g. "0x401000 90 90 90" or "0x401000 eb fe"
                    let parts: Vec<&str> = buf.splitn(2, ' ').collect();
                    if let Some(addr_str) = parts.first() {
                        let stripped = addr_str
                            .trim_start_matches("0x")
                            .trim_start_matches("0X");
                        if let Ok(addr) = u64::from_str_radix(stripped, 16) {
                            if let Some(hex_part) = parts.get(1) {
                                let hex_clean = hex_part.replace(' ', "");
                                let mut bytes = Vec::new();
                                let mut chars = hex_clean.chars();
                                let mut valid = true;
                                while let (Some(hi), Some(lo)) = (chars.next(), chars.next()) {
                                    if let (Some(h), Some(l)) =
                                        (hi.to_digit(16), lo.to_digit(16))
                                    {
                                        bytes.push(((h << 4) | l) as u8);
                                    } else {
                                        valid = false;
                                        break;
                                    }
                                }
                                if valid && !bytes.is_empty() {
                                    let _ = cmd_tx
                                        .send(GdbCommand::PatchBytes { addr, bytes })
                                        .await;
                                }
                            }
                        }
                    }
                }
                InputMode::TypeOverlay => {
                    // Format: "0xADDR type_expression" e.g. "0x7fff5000 struct sockaddr_in"
                    let parts: Vec<&str> = buf.splitn(2, ' ').collect();
                    if let Some(addr_str) = parts.first() {
                        let stripped = addr_str
                            .trim_start_matches("0x")
                            .trim_start_matches("0X");
                        if let Ok(addr) = u64::from_str_radix(stripped, 16) {
                            if let Some(type_expr) = parts.get(1) {
                                let type_expr = type_expr.trim().to_string();
                                if !type_expr.is_empty() {
                                    let _ = cmd_tx
                                        .send(GdbCommand::TypeOverlay { addr, type_expr })
                                        .await;
                                }
                            }
                        }
                    }
                }
                InputMode::ListFunctions => {
                    let pattern = if buf.is_empty() { None } else { Some(buf) };
                    let _ = cmd_tx.send(GdbCommand::ListFunctions(pattern)).await;
                }
                InputMode::Normal => {} // unreachable
            }
        }
        Action::InputCancel => {
            view.cancel_input();
        }
        Action::InputBackspace => {
            view.input_backspace();
        }
        Action::InputDelete => {
            view.input_delete();
        }
        Action::InputLeft => {
            view.input_left();
        }
        Action::InputRight => {
            view.input_right();
        }
        Action::InputHome => {
            view.input_cursor = 0;
        }
        Action::InputEnd => {
            view.input_cursor = view.input_buffer.len();
        }
        Action::InputChar(c) => {
            view.input_insert(c);
        }
        Action::HistoryUp => {
            view.history_up();
        }
        Action::HistoryDown => {
            view.history_down();
        }

        // ---- Source ----
        Action::JumpToExecLine => {
            let snap = state.load();
            if let Some(line) = snap.source_line {
                view.source_cursor = line as usize;
                view.source_follow_exec = true;
            }
        }
        Action::PromptSearch => {
            view.start_input(InputMode::Search);
        }
        Action::SearchNext => {
            if !view.search_matches.is_empty() {
                view.search_current = (view.search_current + 1) % view.search_matches.len();
                view.source_cursor = view.search_matches[view.search_current];
                view.source_follow_exec = false;
            }
        }
        Action::SearchPrev => {
            if !view.search_matches.is_empty() {
                view.search_current = if view.search_current == 0 {
                    view.search_matches.len() - 1
                } else {
                    view.search_current - 1
                };
                view.source_cursor = view.search_matches[view.search_current];
                view.source_follow_exec = false;
            }
        }

        // ---- Scroll ----
        Action::PageUp => {
            if view.focused_panel == Panel::Source {
                view.source_cursor = view.source_cursor.saturating_sub(20).max(1);
                view.source_follow_exec = false;
            } else if view.focused_panel == Panel::Memory {
                view.mem_cursor = view.mem_cursor.saturating_sub(256);
                if view.mem_sel_start.is_some() {
                    view.mem_sel_end = Some(view.mem_cursor);
                }
            } else {
                let sel = view.focused_selection_mut();
                *sel = sel.saturating_sub(20);
                if view.focused_panel == Panel::Output {
                    view.output_follow = false;
                }
            }
        }
        Action::PageDown => {
            if view.focused_panel == Panel::Source {
                let snap = state.load();
                let line_count = snap.source.as_ref().map_or(0, |s| s.lines.len());
                if line_count > 0 {
                    view.source_cursor = (view.source_cursor + 20).min(line_count);
                }
                view.source_follow_exec = false;
            } else if view.focused_panel == Panel::Memory {
                let snap = state.load();
                let mem_len = snap.memory.as_ref().map_or(0, |m| m.bytes.len());
                if mem_len > 0 {
                    view.mem_cursor = (view.mem_cursor + 256).min(mem_len.saturating_sub(1));
                    if view.mem_sel_start.is_some() {
                        view.mem_sel_end = Some(view.mem_cursor);
                    }
                }
            } else {
                let snap = state.load();
                let count = view.focused_item_count(&snap);
                if count > 0 {
                    let sel = view.focused_selection_mut();
                    *sel = (*sel + 20).min(count.saturating_sub(1));
                }
                if view.focused_panel == Panel::Output {
                    let total = snap.output.len();
                    if view.output_scroll >= total.saturating_sub(20) {
                        view.output_follow = true;
                    }
                }
            }
        }

        // ---- Memory panel actions ----
        Action::MemCursorLeft | Action::MemNavBack => {
            if view.focused_panel == Panel::Memory {
                view.mem_cursor = view.mem_cursor.saturating_sub(1);
                if view.mem_sel_start.is_some() {
                    view.mem_sel_end = Some(view.mem_cursor);
                }
            }
        }
        Action::MemCursorRight | Action::MemNavForward => {
            if view.focused_panel == Panel::Memory {
                let snap = state.load();
                let mem_len = snap.memory.as_ref().map_or(0, |m| m.bytes.len());
                if mem_len > 0 {
                    view.mem_cursor = (view.mem_cursor + 1).min(mem_len.saturating_sub(1));
                    if view.mem_sel_start.is_some() {
                        view.mem_sel_end = Some(view.mem_cursor);
                    }
                }
            }
        }
        Action::MemStartSelect => {
            if view.focused_panel == Panel::Memory {
                if view.mem_sel_start.is_some() {
                    // Extend selection to cursor
                    view.mem_sel_end = Some(view.mem_cursor);
                } else {
                    view.mem_sel_start = Some(view.mem_cursor);
                    view.mem_sel_end = Some(view.mem_cursor);
                }
            }
        }
        Action::MemClearSelect => {
            if view.focused_panel == Panel::Memory {
                view.mem_sel_start = None;
                view.mem_sel_end = None;
                view.mem_edit = false;
                view.mem_edit_nibble = None;
            }
        }
        Action::MemCycleCast => {
            if view.focused_panel == Panel::Memory {
                view.mem_cast = view.mem_cast.cycle();
            }
        }
        Action::MemToggleEdit => {
            if view.focused_panel == Panel::Memory {
                view.mem_edit = !view.mem_edit;
                view.mem_edit_nibble = None;
            }
        }

        // ---- Timeline / playback ----
        Action::PlaybackPrev => {
            if view.rec_count > 0 {
                if view.playback_mode {
                    if view.playback_index > 0 {
                        view.playback_index -= 1;
                    }
                } else {
                    // Enter playback mode at the last recorded state
                    view.playback_mode = true;
                    view.playback_index = view.rec_count.saturating_sub(1);
                    // Then step back one if possible
                    if view.playback_index > 0 {
                        view.playback_index -= 1;
                    }
                }
            }
        }
        Action::PlaybackNext => {
            if view.playback_mode {
                if view.playback_index + 1 < view.rec_count {
                    view.playback_index += 1;
                } else {
                    // Past the end — return to live mode
                    view.playback_mode = false;
                    view.source_follow_exec = true;
                }
            }
        }
        Action::PlaybackFirst => {
            if view.rec_count > 0 {
                view.playback_mode = true;
                view.playback_index = 0;
            }
        }
        Action::PlaybackLast => {
            if view.playback_mode {
                // Return to live mode
                view.playback_mode = false;
                view.source_follow_exec = true;
            }
        }
        Action::ToggleRecording => {
            if let Ok(mut rec) = recording.lock() {
                rec.enabled = !rec.enabled;
            }
        }
        Action::ClearRecording => {
            view.playback_mode = false;
            view.rec_playback_snap = None;
            view.rec_entries.clear();
            view.rec_diff = None;
            view.rec_count = 0;
            view.exec_flow = None;
            view.exec_flow_computed_at = 0;
            view.source_follow_exec = true;
            if let Ok(mut rec) = recording.lock() {
                rec.clear();
            }
        }
        Action::PlaybackPrevAnchor => {
            if let Ok(rec) = recording.lock() {
                let from = if view.playback_mode {
                    view.playback_index
                } else {
                    rec.len().saturating_sub(1)
                };
                if let Some(idx) = rec.prev_anchor(from) {
                    view.playback_mode = true;
                    view.playback_index = idx;
                }
            }
        }
        Action::PlaybackNextAnchor => {
            if let Ok(rec) = recording.lock() {
                let from = if view.playback_mode {
                    view.playback_index
                } else {
                    0
                };
                if let Some(idx) = rec.next_anchor(from) {
                    if idx >= rec.len().saturating_sub(1) {
                        view.playback_mode = false;
                    } else {
                        view.playback_mode = true;
                        view.playback_index = idx;
                    }
                } else {
                    view.playback_mode = false;
                }
            }
        }

        // ---- Playback analysis ----
        Action::ShowValueHistory => {
            if view.playback_mode {
                if let Ok(rec) = recording.lock() {
                    let snap = state.load();
                    let history = match view.focused_panel {
                        Panel::Locals => {
                            let render = view.rec_playback_snap.as_ref().unwrap_or(&snap);
                            let var_name = render
                                .locals
                                .get(view.locals_selected)
                                .map(|v| v.name.clone());
                            var_name.map(|name| build_var_history(&rec, &name))
                        }
                        Panel::Registers => {
                            let render = view.rec_playback_snap.as_ref().unwrap_or(&snap);
                            let reg_name = render
                                .registers
                                .get(view.registers_scroll)
                                .map(|r| r.name.clone());
                            reg_name.map(|name| build_reg_history(&rec, &name))
                        }
                        _ => None,
                    };
                    drop(snap);

                    if let Some((entries, var_name, is_register)) = history {
                        let total = rec.len();
                        drop(rec);
                        // Format and push to output
                        let mut s = (**state.load()).clone();
                        let changes = entries.len();
                        s.push_output(
                            crate::state::OutputKind::Info,
                            format!(
                                "--- History of '{}' ({} changes across {} states) ---",
                                var_name, changes, total
                            ),
                        );
                        for (seq, label, loc, value) in &entries {
                            let loc_str = loc.as_deref().unwrap_or("??");
                            if is_register {
                                s.push_output(
                                    crate::state::OutputKind::Console,
                                    format!(
                                        "  #{:<4} {:<6} {:<16} {} = {}",
                                        seq, label, loc_str, var_name, value
                                    ),
                                );
                            } else {
                                s.push_output(
                                    crate::state::OutputKind::Console,
                                    format!(
                                        "  #{:<4} {:<6} {:<16} {} = {}",
                                        seq, label, loc_str, var_name, value
                                    ),
                                );
                            }
                        }
                        // Build trend line for numeric values
                        let trend = build_value_trend(&entries);
                        if !trend.is_empty() {
                            s.push_output(
                                crate::state::OutputKind::Info,
                                format!("  Trend: {}", trend),
                            );
                        }
                        state.store(Arc::new(s));
                    }
                }
            }
        }

        // ---- Libraries ----
        Action::ShowLibraries => {
            let _ = cmd_tx.send(GdbCommand::RefreshLibraries).await;
            // Also show a summary of notification-tracked libraries
            let snap = state.load();
            let count = snap.mapped_libs.len();
            if count > 0 {
                let summary = format!(
                    "{count} loaded libraries (L for details in output panel)"
                );
                drop(snap);
                let mut s = (**state.load()).clone();
                s.push_output(crate::state::OutputKind::Info, summary);
                state.store(Arc::new(s));
            }
        }

        // ---- Memory search ----
        Action::PromptSearchMemory => {
            view.start_input(InputMode::SearchMemory);
        }

        // ---- Disasm patching ----
        Action::PatchNop => {
            if view.focused_panel == Panel::Disasm {
                let snap = state.load();
                let cursor = view.disasm_cursor;
                if let Some(inst) = snap.disasm.get(cursor) {
                    let addr = inst.address;
                    // Compute instruction length from address gap to next instruction
                    let inst_len = if let Some(next) = snap.disasm.get(cursor + 1) {
                        (next.address - addr) as usize
                    } else {
                        // Last instruction — assume a conservative default
                        // (1 byte for x86 single-byte instructions, but typically
                        // instructions are at least 1 byte).
                        1
                    };
                    if inst_len > 0 && inst_len <= 15 {
                        let nop_bytes = vec![0x90u8; inst_len];
                        let _ = cmd_tx
                            .send(GdbCommand::PatchBytes {
                                addr,
                                bytes: nop_bytes,
                            })
                            .await;
                    }
                }
            }
        }
        Action::PromptPatchBytes => {
            if view.focused_panel == Panel::Disasm {
                let snap = state.load();
                if let Some(inst) = snap.disasm.get(view.disasm_cursor) {
                    let prefill = format!("{:#x} ", inst.address);
                    view.start_input_with(InputMode::PatchBytes, prefill);
                } else {
                    view.start_input(InputMode::PatchBytes);
                }
            } else {
                view.start_input(InputMode::PatchBytes);
            }
        }

        // ---- Analysis ----
        Action::AnalyzeXrefs => {
            if view.focused_panel == Panel::Disasm {
                let snap = state.load();
                if let Some(inst) = snap.disasm.get(view.disasm_cursor) {
                    let addr = inst.address;
                    let _ = cmd_tx
                        .send(GdbCommand::AnalyzeXrefs { addr })
                        .await;
                }
            }
        }
        Action::PromptTypeOverlay => {
            let snap = state.load();
            // Try to prefill the address from the current context
            let prefill = if view.focused_panel == Panel::Disasm {
                snap.disasm.get(view.disasm_cursor)
                    .map(|inst| format!("0x{:x} ", inst.address))
            } else if view.focused_panel == Panel::Memory {
                snap.memory.as_ref()
                    .map(|m| format!("0x{:x} ", m.address + view.mem_cursor as u64))
            } else {
                None
            };
            match prefill {
                Some(p) => view.start_input_with(InputMode::TypeOverlay, p),
                None => view.start_input(InputMode::TypeOverlay),
            }
        }
        Action::PromptListFunctions => {
            view.start_input(InputMode::ListFunctions);
        }
        Action::ResolveSymbol => {
            if view.focused_panel == Panel::Disasm {
                let snap = state.load();
                if let Some(inst) = snap.disasm.get(view.disasm_cursor) {
                    let addr = inst.address;
                    let _ = cmd_tx
                        .send(GdbCommand::ResolveSymbol(addr))
                        .await;
                }
            }
        }
    }

    false
}

/// Handle a hex digit keypress in memory edit mode.
/// Collects two nibbles then sends a WriteMemory command.
/// Returns true if the key was consumed.
async fn dispatch_mem_hex(
    view: &mut ViewState,
    state: &SharedState,
    cmd_tx: &mpsc::Sender<GdbCommand>,
    c: char,
) -> bool {
    let nibble = match c.to_ascii_lowercase() {
        '0'..='9' => c as u8 - b'0',
        'a'..='f' => c as u8 - b'a' + 10,
        _ => return false,
    };

    match view.mem_edit_nibble.take() {
        None => {
            // First nibble — store and wait for second
            view.mem_edit_nibble = Some(nibble);
            true
        }
        Some(high) => {
            // Second nibble — compute byte and write
            let byte = (high << 4) | nibble;
            let snap = state.load();
            if let Some(ref mem) = snap.memory {
                let addr = mem.address + view.mem_cursor as u64;
                let _ = cmd_tx
                    .send(GdbCommand::WriteMemory {
                        addr,
                        bytes: vec![byte],
                    })
                    .await;
                // Advance cursor
                let mem_len = mem.bytes.len();
                drop(snap);
                if view.mem_cursor + 1 < mem_len {
                    view.mem_cursor += 1;
                }
            }
            true
        }
    }
}

// ---------------------------------------------------------------------------
// Playback analysis helpers
// ---------------------------------------------------------------------------

/// Build the value history of a local variable across all recorded states.
/// Returns (entries, variable_name, is_register).
/// Each entry is (seq, stop_label, source_loc, value).
fn build_var_history(
    rec: &Recording,
    name: &str,
) -> (Vec<(u64, String, Option<String>, String)>, String, bool) {
    let mut history = Vec::new();
    let mut last_value = String::new();
    for i in 0..rec.len() {
        if let Some(state) = rec.get(i) {
            if let Some(var) = state.locals.iter().find(|v| v.name == name) {
                if var.value != last_value {
                    let loc = match (&state.source_path, state.source_line) {
                        (Some(p), Some(l)) => {
                            let short = p.rsplit('/').next().unwrap_or(p);
                            Some(format!("{short}:{l}"))
                        }
                        _ => None,
                    };
                    let label = stop_reason_label(&state.stop_reason);
                    history.push((state.seq, label, loc, var.value.clone()));
                    last_value = var.value.clone();
                }
            }
        }
    }
    (history, name.to_string(), false)
}

/// Build the value history of a register across all recorded states.
/// Returns (entries, register_name, is_register).
fn build_reg_history(
    rec: &Recording,
    name: &str,
) -> (Vec<(u64, String, Option<String>, String)>, String, bool) {
    let mut history = Vec::new();
    let mut last_value = String::new();
    for i in 0..rec.len() {
        if let Some(state) = rec.get(i) {
            if let Some(reg) = state.registers.iter().find(|r| r.name == name) {
                if reg.value != last_value {
                    let loc = if let Some(frame) = state.stack.first() {
                        Some(format!("{:#x}", frame.addr))
                    } else {
                        None
                    };
                    let label = stop_reason_label(&state.stop_reason);
                    history.push((state.seq, label, loc, reg.value.clone()));
                    last_value = reg.value.clone();
                }
            }
        }
    }
    (history, name.to_string(), true)
}

/// Build a one-line trend summary from value history entries.
/// Attempts to parse values as integers and shows a compact representation.
fn build_value_trend(entries: &[(u64, String, Option<String>, String)]) -> String {
    if entries.is_empty() {
        return String::new();
    }

    let parsed: Vec<Option<i128>> = entries
        .iter()
        .map(|(_, _, _, val)| parse_int_value(val))
        .collect();

    // If all values are parseable as integers, show trend
    if parsed.iter().all(|v| v.is_some()) {
        let nums: Vec<i128> = parsed.into_iter().flatten().collect();
        if nums.len() <= 8 {
            let parts: Vec<String> = nums.iter().map(|n| format!("{n}")).collect();
            let trend_desc = classify_trend(&nums);
            if trend_desc.is_empty() {
                parts.join(" -> ")
            } else {
                format!("{}  ({})", parts.join(" -> "), trend_desc)
            }
        } else {
            // Show first 3, ..., last 3
            let first: Vec<String> = nums[..3].iter().map(|n| format!("{n}")).collect();
            let last: Vec<String> = nums[nums.len() - 3..].iter().map(|n| format!("{n}")).collect();
            let trend_desc = classify_trend(&nums);
            if trend_desc.is_empty() {
                format!("{} -> ... -> {}", first.join(" -> "), last.join(" -> "))
            } else {
                format!(
                    "{} -> ... -> {}  ({})",
                    first.join(" -> "),
                    last.join(" -> "),
                    trend_desc
                )
            }
        }
    } else {
        // Non-numeric values, just show count
        String::new()
    }
}

/// Try to parse a value string as an integer (handles hex, decimal, negative).
fn parse_int_value(val: &str) -> Option<i128> {
    let trimmed = val.trim();
    if let Some(hex) = trimmed.strip_prefix("0x").or_else(|| trimmed.strip_prefix("0X")) {
        i128::from_str_radix(hex, 16).ok()
    } else {
        trimmed.parse::<i128>().ok()
    }
}

/// Classify a numeric trend (monotonic increase, decrease, constant, etc.)
fn classify_trend(nums: &[i128]) -> &'static str {
    if nums.len() <= 1 {
        return "";
    }
    let all_inc = nums.windows(2).all(|w| w[1] >= w[0]);
    let all_dec = nums.windows(2).all(|w| w[1] <= w[0]);
    let all_same = nums.windows(2).all(|w| w[1] == w[0]);

    if all_same {
        "constant"
    } else if all_inc {
        "monotonic increase"
    } else if all_dec {
        "monotonic decrease"
    } else {
        ""
    }
}

// ---------------------------------------------------------------------------
// Smart prefill helpers
// ---------------------------------------------------------------------------

/// Extract context-appropriate prefill text for watch/eval/memory prompts
/// based on the currently focused panel and selection.
fn get_prefill(view: &ViewState, snap: &GdbSnapshot, mode: InputMode) -> Option<String> {
    match view.focused_panel {
        Panel::Locals => {
            let var = snap.locals.get(view.locals_selected)?;
            match mode {
                InputMode::Memory => {
                    if var.value.starts_with("0x") {
                        Some(var.value.clone())
                    } else {
                        Some(format!("&{}", var.name))
                    }
                }
                _ => Some(var.name.clone()),
            }
        }
        Panel::Watch => {
            let w = snap.watch_expressions.get(view.watch_selected)?;
            match mode {
                InputMode::Memory => {
                    if w.value.starts_with("0x") {
                        Some(w.value.clone())
                    } else {
                        Some(format!("&{}", w.expression))
                    }
                }
                _ => Some(w.expression.clone()),
            }
        }
        Panel::Source => {
            let src = snap.source.as_ref()?;
            let line_text = src.lines.get(view.source_cursor.checked_sub(1)?)?;
            let ident = extract_identifier(line_text)?;
            match mode {
                InputMode::Memory => Some(format!("&{ident}")),
                _ => Some(ident),
            }
        }
        _ => None,
    }
}

/// Extract the longest identifier-like token from a source line.
/// Prefers the left-hand side of assignments, otherwise the first
/// substantive identifier (skipping keywords).
fn extract_identifier(line: &str) -> Option<String> {
    let trimmed = line.trim();

    // If there's an assignment, take the LHS target.
    if let Some(lhs) = trimmed.split('=').next() {
        let lhs = lhs.trim();
        // Walk backwards from the end of LHS to find the last identifier
        // (handles `int x`, `auto& foo`, `let mut bar`, etc.)
        if let Some(id) = last_ident(lhs) {
            if !is_keyword(&id) {
                return Some(id);
            }
        }
    }

    // Fallback: first non-keyword identifier on the line.
    ident_tokens(trimmed)
        .into_iter()
        .find(|t| !is_keyword(t))
}

fn last_ident(s: &str) -> Option<String> {
    let mut end = s.len();
    while end > 0 && !s.as_bytes()[end - 1].is_ascii_alphanumeric() && s.as_bytes()[end - 1] != b'_' {
        end -= 1;
    }
    if end == 0 {
        return None;
    }
    let mut start = end;
    while start > 0 && (s.as_bytes()[start - 1].is_ascii_alphanumeric() || s.as_bytes()[start - 1] == b'_') {
        start -= 1;
    }
    if start == end {
        return None;
    }
    let token = &s[start..end];
    if token.chars().next()?.is_ascii_digit() {
        return None;
    }
    Some(token.to_string())
}

fn ident_tokens(s: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            tokens.push(s[start..i].to_string());
        } else {
            i += 1;
        }
    }
    tokens
}

fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        // C/C++
        "if" | "else" | "for" | "while" | "do" | "switch" | "case" | "break"
        | "continue" | "return" | "goto" | "struct" | "union" | "enum"
        | "typedef" | "extern" | "static" | "const" | "volatile" | "inline"
        | "void" | "int" | "char" | "short" | "long" | "float" | "double"
        | "signed" | "unsigned" | "sizeof" | "auto" | "register"
        | "class" | "public" | "private" | "protected" | "virtual"
        | "namespace" | "using" | "template" | "typename" | "new" | "delete"
        | "try" | "catch" | "throw" | "nullptr" | "true" | "false" | "bool"
        // Rust
        | "fn" | "let" | "mut" | "pub" | "mod" | "use" | "crate" | "self"
        | "super" | "impl" | "trait" | "where" | "type" | "as" | "in"
        | "ref" | "match" | "loop" | "move" | "unsafe" | "async" | "await"
        | "dyn" | "Self"
        // Python
        | "def" | "import" | "from" | "with" | "yield" | "lambda"
        | "pass" | "raise" | "except" | "finally" | "global"
        | "nonlocal" | "assert" | "del" | "elif" | "is" | "not" | "or" | "and"
        // Go
        | "func" | "var" | "package" | "range" | "defer" | "chan" | "go"
        | "select" | "interface" | "map"
    )
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn draw(f: &mut ratatui::Frame, snap: &GdbSnapshot, view: &ViewState) {
    let visible = view.visible_panels_ordered();
    let show_timeline = view.rec_count > 0 || view.playback_mode;
    let panel_layout = layout::compute_with_timeline(f.area(), &visible, show_timeline);

    // Status bar
    widgets::status_bar::draw(f, panel_layout.status_bar, snap);

    // Left panels
    for &(panel, rect) in &panel_layout.left_panels {
        draw_panel(f, panel, rect, snap, view);
    }

    // Right panels
    for &(panel, rect) in &panel_layout.right_panels {
        draw_panel(f, panel, rect, snap, view);
    }

    // Output area
    if let Some(rect) = panel_layout.output_area {
        panels::output::draw(f, rect, snap, view);
    }

    // Timeline bar
    if let Some(rect) = panel_layout.timeline_area {
        panels::timeline::draw(f, rect, view);
    }

    // Footer
    widgets::footer::draw(f, panel_layout.footer, snap, view);

    // Input prompt overlay (if in input mode)
    if view.input_mode != InputMode::Normal {
        widgets::prompt::draw(f, f.area(), view);
    }

    // Help overlay
    if view.help_open {
        widgets::help::draw(f, f.area(), view);
    }
}

fn draw_panel(
    f: &mut ratatui::Frame,
    panel: Panel,
    rect: Rect,
    snap: &GdbSnapshot,
    view: &ViewState,
) {
    let focused = panel == view.focused_panel;
    match panel {
        Panel::Source => panels::source::draw(f, rect, snap, view, focused),
        Panel::Stack => panels::stack::draw(f, rect, snap, view, focused),
        Panel::Locals => panels::locals::draw(f, rect, snap, view, focused),
        Panel::Threads => panels::threads::draw(f, rect, snap, view, focused),
        Panel::Breakpoints => panels::breakpoints::draw(f, rect, snap, view, focused),
        Panel::Registers => panels::registers::draw(f, rect, snap, view, focused),
        Panel::Memory => panels::memory::draw(f, rect, snap, view, focused),
        Panel::Disasm => panels::disasm::draw(f, rect, snap, view, focused),
        Panel::Watch => panels::watch::draw(f, rect, snap, view, focused),
        Panel::Output => panels::output::draw(f, rect, snap, view),
    }
}
