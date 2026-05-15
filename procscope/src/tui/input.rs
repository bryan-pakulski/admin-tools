use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    ConfirmQuit,
    CancelQuit,
    SelectUp,
    SelectDown,
    ToggleDetail,
    TogglePause,
    SpeedUp,
    SlowDown,
    CycleWindow,
    CycleWindowBack,
    ToggleLogScale,
    Export,
    Filter,
    CycleView,
    CycleListAvg,
    ToggleHelp,
}

pub fn map(key: KeyEvent) -> Action {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => Action::Quit,
        KeyCode::Char('c') | KeyCode::Char('d') if ctrl => Action::Quit,
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => Action::ConfirmQuit,
        KeyCode::Char('n') | KeyCode::Char('N') => Action::CancelQuit,
        KeyCode::Up | KeyCode::Char('k') => Action::SelectUp,
        KeyCode::Down | KeyCode::Char('j') => Action::SelectDown,
        KeyCode::Char(' ') => Action::TogglePause,
        KeyCode::Char('+') | KeyCode::Char('=') => Action::SpeedUp,
        KeyCode::Char('-') | KeyCode::Char('_') => Action::SlowDown,
        KeyCode::Char('w') => Action::CycleWindow,
        KeyCode::Char('W') => Action::CycleWindowBack,
        KeyCode::Char('l') | KeyCode::Char('L') => Action::ToggleLogScale,
        KeyCode::Char('e') | KeyCode::Char('E') => Action::Export,
        KeyCode::Char('f') | KeyCode::Char('F') => Action::Filter,
        KeyCode::Char('v') | KeyCode::Char('V') => Action::CycleView,
        KeyCode::Char('a') | KeyCode::Char('A') => Action::CycleListAvg,
        KeyCode::Char('h') | KeyCode::Char('H') | KeyCode::Char('?') | KeyCode::F(1) => {
            Action::ToggleHelp
        }
        _ => Action::None,
    }
}

/// Smart key router. Returns ConfirmQuit only when quit_confirm is true; otherwise Enter toggles detail.
pub fn map_with_quit_confirm(key: KeyEvent, quit_confirm: bool) -> Action {
    let base = map(key);
    match (base, quit_confirm) {
        (Action::ConfirmQuit, true) => Action::ConfirmQuit,
        (Action::ConfirmQuit, false) => match key.code {
            KeyCode::Enter => Action::ToggleDetail,
            _ => Action::None,
        },
        (Action::CancelQuit, true) => Action::CancelQuit,
        (Action::CancelQuit, false) => Action::None,
        (a, _) => a,
    }
}
