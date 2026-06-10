use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    ConfirmQuit,
    CancelQuit,

    // Execution
    RunContinue,      // F5: run if not started, continue if stopped
    TraceContinue,        // F6: trace with locals (optimized, ~120 steps/sec)
    StepInto,         // F7
    StepOver,         // F8
    StepOut,          // F9
    Interrupt,        // Shift+F5

    // Navigation
    SelectUp,
    SelectDown,
    GoTop,
    GoBottom,
    Enter,            // activate selection
    CycleFocusForward, // Tab
    CycleFocusBack,   // Shift+Tab

    // Panels
    TogglePanel(usize), // 0-9

    // Breakpoints
    ToggleBreakAtLine,    // F10
    PromptBreakpoint,     // b
    PromptBreakpointCond, // B — breakpoint with condition
    PromptBreakCondEdit,  // c — edit condition on selected breakpoint
    PromptWatchpoint,     // W — hardware watchpoint
    DeleteBreakpoint,     // d
    ToggleEnableBreak,    // e

    // Register editing
    PromptRegisterEdit,   // E — edit selected register

    // Inspection
    PromptWatch,  // w
    PromptMemory, // m
    PromptEval,   // p
    PromptPtype,  // y

    // Command
    PromptCommand,    // :
    RepeatLastCommand, // ;

    // Help
    ToggleHelp, // ? or F1 or h

    // Input mode actions
    InputSubmit,    // Enter in input mode
    InputCancel,    // Esc in input mode
    InputBackspace,
    InputDelete,
    InputLeft,
    InputRight,
    InputHome,
    InputEnd,
    InputChar(char),

    // History in input mode
    HistoryUp,
    HistoryDown,

    // Source panel
    JumpToExecLine, // . (dot) — jump cursor back to current execution line
    PromptSearch, // /
    SearchNext,   // n
    SearchPrev,   // N (Shift+n)

    // Memory panel
    MemCursorLeft,
    MemCursorRight,
    MemStartSelect,   // v — start/extend selection
    MemClearSelect,    // Esc clears
    MemCycleCast,      // t — cycle type interpretation
    MemToggleEdit,     // i — enter edit mode
    MemNavForward,     // l or right in non-edit
    MemNavBack,        // h or left in non-edit

    // Scroll for output panel
    PageUp,
    PageDown,

    // Timeline / playback
    PlaybackPrev,      // [ — go to previous recorded state
    PlaybackNext,      // ] — go to next recorded state
    PlaybackFirst,     // { — go to first recorded state
    PlaybackLast,      // } — go to last state / return to live
    PlaybackPrevAnchor, // < — jump to previous breakpoint anchor
    PlaybackNextAnchor, // > — jump to next breakpoint anchor
    ToggleRecording,   // R — enable/disable recording
    ClearRecording,    // C — clear all recorded states

    // Libraries / sections
    ShowLibraries,     // L — show mapped libraries in output

    // Memory search
    PromptSearchMemory, // S — search memory for string/hex pattern

    // Disasm patching
    PatchNop,          // P — NOP out instruction at disasm cursor
    PromptPatchBytes,  // a — write raw bytes at disasm cursor address

    // Analysis
    AnalyzeXrefs,      // x — analyze cross-references at disasm cursor
    PromptTypeOverlay, // T — type overlay: cast memory as a typed struct
    PromptListFunctions, // f — list functions (with optional filter)
    ResolveSymbol,     // s — resolve address at disasm cursor to symbol

    // Explorer
    ToggleExplorer,       // I — toggle explorer panel / add from context
    PromptExplorerAdd,    // (internal) open prompt to add expression

    // Playback analysis
    ShowValueHistory,  // H — show value history for selected variable/register
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Command,
    Breakpoint,
    BreakpointCond,
    BreakCondEdit,
    Watchpoint,
    RegisterEdit,
    Watch,
    Memory,
    Eval,
    Search,
    SearchMemory,
    PatchBytes,
    ListFunctions,
    TypeOverlay,
    Ptype,
    ExplorerAdd,
}

/// Map a key event to an action when in normal (non-input) mode.
///
/// When `quit_confirm` is true the user has already pressed `q` once and we are
/// waiting for `y`/`n` confirmation.
pub fn map_normal(key: KeyEvent, quit_confirm: bool) -> Action {
    if quit_confirm {
        return match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Action::ConfirmQuit,
            _ => Action::CancelQuit,
        };
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        // ---- Quit ----
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Char('c') if ctrl => Action::Quit,

        // ---- Execution ----
        KeyCode::F(5) if shift => Action::Interrupt,
        KeyCode::F(15) | KeyCode::F(17) => Action::Interrupt, // Shift+F5 on many terminals
        KeyCode::Char('x') if ctrl => Action::Interrupt,       // Ctrl+X as reliable alternative
        KeyCode::F(5) => Action::RunContinue,
        KeyCode::F(6) => Action::TraceContinue,
        KeyCode::F(7) => Action::StepInto,
        KeyCode::F(8) => Action::StepOver,
        KeyCode::F(9) => Action::StepOut,

        // ---- Navigation ----
        KeyCode::Up | KeyCode::Char('k') => Action::SelectUp,
        KeyCode::Down | KeyCode::Char('j') => Action::SelectDown,
        KeyCode::Home | KeyCode::Char('g') => Action::GoTop,
        KeyCode::End | KeyCode::Char('G') => Action::GoBottom,
        KeyCode::Enter => Action::Enter,
        KeyCode::Tab if shift => Action::CycleFocusBack,
        KeyCode::BackTab => Action::CycleFocusBack,
        KeyCode::Tab => Action::CycleFocusForward,

        // ---- Panel toggles (number keys) ----
        KeyCode::Char('1') => Action::TogglePanel(0),
        KeyCode::Char('2') => Action::TogglePanel(1),
        KeyCode::Char('3') => Action::TogglePanel(2),
        KeyCode::Char('4') => Action::TogglePanel(3),
        KeyCode::Char('5') => Action::TogglePanel(4),
        KeyCode::Char('6') => Action::TogglePanel(5),
        KeyCode::Char('7') => Action::TogglePanel(6),
        KeyCode::Char('8') => Action::TogglePanel(7),
        KeyCode::Char('9') => Action::TogglePanel(8),
        KeyCode::Char('0') => Action::TogglePanel(9),

        // ---- Breakpoints ----
        KeyCode::F(10) => Action::ToggleBreakAtLine,
        KeyCode::Char('b') => Action::PromptBreakpoint,
        KeyCode::Char('B') => Action::PromptBreakpointCond,
        KeyCode::Char('c') => Action::PromptBreakCondEdit,
        KeyCode::Char('W') => Action::PromptWatchpoint,
        KeyCode::Char('d') => Action::DeleteBreakpoint,
        KeyCode::Char('e') => Action::ToggleEnableBreak,
        KeyCode::Char('E') => Action::PromptRegisterEdit,

        // ---- Inspection ----
        KeyCode::Char('w') => Action::PromptWatch,
        KeyCode::Char('m') => Action::PromptMemory,
        KeyCode::Char('p') => Action::PromptEval,
        KeyCode::Char('y') => Action::PromptPtype,

        // ---- Command ----
        KeyCode::Char(':') => Action::PromptCommand,
        KeyCode::Char(';') => Action::RepeatLastCommand,

        // ---- Help ----
        KeyCode::Char('?') => Action::ToggleHelp,
        KeyCode::Char('h') => Action::ToggleHelp,
        KeyCode::F(1) => Action::ToggleHelp,

        // ---- Source ----
        KeyCode::Char('.') => Action::JumpToExecLine,
        KeyCode::Char('/') => Action::PromptSearch,
        KeyCode::Char('n') => Action::SearchNext,
        KeyCode::Char('N') => Action::SearchPrev,

        // ---- Memory panel ----
        KeyCode::Char('v') => Action::MemStartSelect,
        KeyCode::Char('t') => Action::MemCycleCast,
        KeyCode::Char('i') => Action::MemToggleEdit,
        KeyCode::Left => Action::MemCursorLeft,
        KeyCode::Right => Action::MemCursorRight,

        // ---- Libraries / search / patch ----
        KeyCode::Char('L') => Action::ShowLibraries,
        KeyCode::Char('S') => Action::PromptSearchMemory,
        KeyCode::Char('P') => Action::PatchNop,
        KeyCode::Char('a') => Action::PromptPatchBytes,

        // ---- Analysis ----
        KeyCode::Char('x') => Action::AnalyzeXrefs,
        KeyCode::Char('T') => Action::PromptTypeOverlay,
        KeyCode::Char('f') => Action::PromptListFunctions,
        KeyCode::Char('s') => Action::ResolveSymbol,

        // ---- Explorer ----
        KeyCode::Char('I') => Action::ToggleExplorer,

        // ---- Playback analysis ----
        KeyCode::Char('H') => Action::ShowValueHistory,

        // ---- Timeline / playback ----
        KeyCode::Char('[') => Action::PlaybackPrev,
        KeyCode::Char(']') => Action::PlaybackNext,
        KeyCode::Char('{') => Action::PlaybackFirst,
        KeyCode::Char('}') => Action::PlaybackLast,
        KeyCode::Char('<') => Action::PlaybackPrevAnchor,
        KeyCode::Char('>') => Action::PlaybackNextAnchor,
        KeyCode::Char('R') => Action::ToggleRecording,
        KeyCode::Char('C') => Action::ClearRecording,

        // ---- Scroll ----
        KeyCode::PageUp => Action::PageUp,
        KeyCode::PageDown => Action::PageDown,

        _ => Action::None,
    }
}

/// Map a key event to an action when in an input mode (command prompt,
/// breakpoint prompt, etc.).
pub fn map_input(key: KeyEvent) -> Action {
    match key.code {
        KeyCode::Enter => Action::InputSubmit,
        KeyCode::Esc => Action::InputCancel,
        KeyCode::Backspace => Action::InputBackspace,
        KeyCode::Delete => Action::InputDelete,
        KeyCode::Left => Action::InputLeft,
        KeyCode::Right => Action::InputRight,
        KeyCode::Home => Action::InputHome,
        KeyCode::End => Action::InputEnd,
        KeyCode::Up => Action::HistoryUp,
        KeyCode::Down => Action::HistoryDown,
        KeyCode::Char(c) => Action::InputChar(c),
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    fn key_ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    // --- map_normal tests ---

    #[test]
    fn normal_f5_maps_to_run_continue() {
        assert_eq!(map_normal(key(KeyCode::F(5)), false), Action::RunContinue);
    }

    #[test]
    fn normal_f7_maps_to_step_into() {
        assert_eq!(map_normal(key(KeyCode::F(7)), false), Action::StepInto);
    }

    #[test]
    fn normal_q_maps_to_quit() {
        assert_eq!(map_normal(key(KeyCode::Char('q')), false), Action::Quit);
    }

    #[test]
    fn normal_ctrl_c_maps_to_quit() {
        assert_eq!(map_normal(key_ctrl(KeyCode::Char('c')), false), Action::Quit);
    }

    #[test]
    fn normal_number_keys_map_to_toggle_panel() {
        assert_eq!(map_normal(key(KeyCode::Char('1')), false), Action::TogglePanel(0));
        assert_eq!(map_normal(key(KeyCode::Char('2')), false), Action::TogglePanel(1));
        assert_eq!(map_normal(key(KeyCode::Char('3')), false), Action::TogglePanel(2));
        assert_eq!(map_normal(key(KeyCode::Char('4')), false), Action::TogglePanel(3));
        assert_eq!(map_normal(key(KeyCode::Char('5')), false), Action::TogglePanel(4));
        assert_eq!(map_normal(key(KeyCode::Char('6')), false), Action::TogglePanel(5));
        assert_eq!(map_normal(key(KeyCode::Char('7')), false), Action::TogglePanel(6));
        assert_eq!(map_normal(key(KeyCode::Char('8')), false), Action::TogglePanel(7));
        assert_eq!(map_normal(key(KeyCode::Char('9')), false), Action::TogglePanel(8));
        assert_eq!(map_normal(key(KeyCode::Char('0')), false), Action::TogglePanel(9));
    }

    #[test]
    fn normal_quit_confirm_y_maps_to_confirm_quit() {
        assert_eq!(map_normal(key(KeyCode::Char('y')), true), Action::ConfirmQuit);
        assert_eq!(map_normal(key(KeyCode::Char('Y')), true), Action::ConfirmQuit);
    }

    #[test]
    fn normal_quit_confirm_other_maps_to_cancel_quit() {
        assert_eq!(map_normal(key(KeyCode::Char('n')), true), Action::CancelQuit);
        assert_eq!(map_normal(key(KeyCode::Char('x')), true), Action::CancelQuit);
        assert_eq!(map_normal(key(KeyCode::Esc), true), Action::CancelQuit);
    }

    #[test]
    fn normal_bracket_keys_map_to_playback() {
        assert_eq!(map_normal(key(KeyCode::Char('[')), false), Action::PlaybackPrev);
        assert_eq!(map_normal(key(KeyCode::Char(']')), false), Action::PlaybackNext);
    }

    #[test]
    fn normal_brace_keys_map_to_playback_first_last() {
        assert_eq!(map_normal(key(KeyCode::Char('{')), false), Action::PlaybackFirst);
        assert_eq!(map_normal(key(KeyCode::Char('}')), false), Action::PlaybackLast);
    }

    #[test]
    fn normal_shift_f5_maps_to_interrupt() {
        assert_eq!(map_normal(key_shift(KeyCode::F(5)), false), Action::Interrupt);
    }

    #[test]
    fn normal_f6_maps_to_trace_continue() {
        assert_eq!(map_normal(key(KeyCode::F(6)), false), Action::TraceContinue);
    }

    #[test]
    fn normal_f8_maps_to_step_over() {
        assert_eq!(map_normal(key(KeyCode::F(8)), false), Action::StepOver);
    }

    #[test]
    fn normal_f9_maps_to_step_out() {
        assert_eq!(map_normal(key(KeyCode::F(9)), false), Action::StepOut);
    }

    #[test]
    fn normal_tab_maps_to_cycle_focus() {
        assert_eq!(map_normal(key(KeyCode::Tab), false), Action::CycleFocusForward);
    }

    #[test]
    fn normal_unknown_key_maps_to_none() {
        assert_eq!(map_normal(key(KeyCode::F(20)), false), Action::None);
    }

    // --- map_input tests ---

    #[test]
    fn input_enter_maps_to_submit() {
        assert_eq!(map_input(key(KeyCode::Enter)), Action::InputSubmit);
    }

    #[test]
    fn input_esc_maps_to_cancel() {
        assert_eq!(map_input(key(KeyCode::Esc)), Action::InputCancel);
    }

    #[test]
    fn input_char_maps_to_input_char() {
        assert_eq!(map_input(key(KeyCode::Char('a'))), Action::InputChar('a'));
        assert_eq!(map_input(key(KeyCode::Char('Z'))), Action::InputChar('Z'));
        assert_eq!(map_input(key(KeyCode::Char('5'))), Action::InputChar('5'));
    }

    #[test]
    fn input_backspace_maps_to_input_backspace() {
        assert_eq!(map_input(key(KeyCode::Backspace)), Action::InputBackspace);
    }

    #[test]
    fn input_arrow_keys_map_correctly() {
        assert_eq!(map_input(key(KeyCode::Left)), Action::InputLeft);
        assert_eq!(map_input(key(KeyCode::Right)), Action::InputRight);
        assert_eq!(map_input(key(KeyCode::Up)), Action::HistoryUp);
        assert_eq!(map_input(key(KeyCode::Down)), Action::HistoryDown);
    }

    #[test]
    fn input_home_end_map_correctly() {
        assert_eq!(map_input(key(KeyCode::Home)), Action::InputHome);
        assert_eq!(map_input(key(KeyCode::End)), Action::InputEnd);
    }

    #[test]
    fn input_unknown_key_maps_to_none() {
        assert_eq!(map_input(key(KeyCode::F(12))), Action::None);
    }
}
