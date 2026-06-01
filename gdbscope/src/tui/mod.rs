pub mod input;
pub mod layout;
pub mod panels;
pub mod widgets;

use std::collections::HashSet;
use std::io;
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
use crate::state::{GdbSnapshot, SharedState, TargetState};

use input::{Action, InputMode};
use layout::Panel;

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
            watch_selected: 0,
            output_scroll: 0,
            output_follow: true,

            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            input_cursor: 0,

            command_history: Vec::new(),
            history_index: 0,
            last_command: None,

            help_open: false,
            help_scroll: 0,
            quit_confirm: false,

            search_query: None,
            search_matches: Vec::new(),
            search_current: 0,

            tick_count: 0,
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
            Panel::Disasm => &mut self.disasm_scroll,
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
) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = event_loop(&mut terminal, state, cmd_tx, redraw_hz).await;

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

                let action = match view.input_mode {
                    InputMode::Normal => input::map_normal(key, view.quit_confirm),
                    _ => input::map_input(key),
                };

                if dispatch_action(action, &mut view, &state, &cmd_tx).await {
                    return Ok(());
                }
            }
        }

        let snap = state.load();

        // Auto-sync cursor to execution line when GDB stops at a new location
        if view.source_follow_exec {
            if let Some(line) = snap.source_line {
                if prev_source_line != Some(line) {
                    view.source_cursor = line as usize;
                }
            }
        }
        prev_source_line = snap.source_line;

        view.tick_count = view.tick_count.wrapping_add(1);
        terminal.draw(|f| draw(f, &snap, &view))?;
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
            let _ = cmd_tx.send(cmd).await;
        }
        Action::StepInto => {
            view.source_follow_exec = true;
            let _ = cmd_tx.send(GdbCommand::StepInto).await;
        }
        Action::StepOver => {
            view.source_follow_exec = true;
            let _ = cmd_tx.send(GdbCommand::StepOver).await;
        }
        Action::StepOut => {
            view.source_follow_exec = true;
            let _ = cmd_tx.send(GdbCommand::StepOut).await;
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
            if let Some(ref src) = snap.source {
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
    let panel_layout = layout::compute(f.area(), &visible);

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
