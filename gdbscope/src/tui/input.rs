use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    ConfirmQuit,
    CancelQuit,

    // Execution
    RunContinue,   // F5: run if not started, continue if stopped
    StepInto,      // F7
    StepOver,      // F8
    StepOut,       // F9
    Interrupt,     // Shift+F5

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
    ToggleBreakAtLine, // F10
    PromptBreakpoint,  // b
    DeleteBreakpoint,  // d
    ToggleEnableBreak, // e

    // Inspection
    PromptWatch,  // w
    PromptMemory, // m
    PromptEval,   // p

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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Command,
    Breakpoint,
    Watch,
    Memory,
    Eval,
    Search,
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
        KeyCode::F(5) => Action::RunContinue,
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
        KeyCode::Char('d') => Action::DeleteBreakpoint,
        KeyCode::Char('e') => Action::ToggleEnableBreak,

        // ---- Inspection ----
        KeyCode::Char('w') => Action::PromptWatch,
        KeyCode::Char('m') => Action::PromptMemory,
        KeyCode::Char('p') => Action::PromptEval,

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
