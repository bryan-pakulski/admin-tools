use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::{GdbSnapshot, TargetState};

/// Draw the status bar across the top of the screen.
///
/// Shows the target executable name, the current target state, and any status
/// message from the last GDB response.
pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot) {
    let exe = snap
        .target_executable
        .as_deref()
        .unwrap_or("<no target>");

    let (state_text, state_color) = match &snap.target_state {
        TargetState::NotStarted => ("NOT STARTED", Color::DarkGray),
        TargetState::Running => ("RUNNING", Color::Green),
        TargetState::Stopped => ("STOPPED", Color::Yellow),
        TargetState::Exited(code) => {
            // We cannot format the code into the static string, so we handle
            // it specially below.
            if *code == 0 {
                ("EXITED(0)", Color::Blue)
            } else {
                ("EXITED", Color::Red)
            }
        }
        TargetState::Terminated => ("TERMINATED", Color::Red),
    };

    let stop_info = match &snap.stop_reason {
        Some(reason) => {
            let s = match reason {
                crate::state::StopReason::BreakpointHit { number } => {
                    format!(" | breakpoint #{}", number)
                }
                crate::state::StopReason::Watchpoint { number } => {
                    format!(" | watchpoint #{}", number)
                }
                crate::state::StopReason::StepFinished => " | step".to_string(),
                crate::state::StopReason::SignalReceived { name, meaning } => {
                    format!(" | signal {} ({})", name, meaning)
                }
                crate::state::StopReason::FunctionFinished => " | function returned".to_string(),
                crate::state::StopReason::ExitedNormally { code } => {
                    format!(" | exited({})", code)
                }
                crate::state::StopReason::Unknown(s) => format!(" | {}", s),
            };
            s
        }
        None => String::new(),
    };

    // Thread + frame context
    let context_info = if matches!(snap.target_state, TargetState::Stopped) {
        let tid = snap.current_thread_id
            .map(|id| format!(" | Thread {id}"))
            .unwrap_or_default();
        let frame_str = snap.stack.iter()
            .find(|fr| fr.level == snap.current_frame_level)
            .or_else(|| snap.stack.first())
            .map(|fr| {
                let func = fr.func.as_deref().unwrap_or("??");
                if let (Some(file), Some(line)) = (&fr.file, fr.line) {
                    let short = file.rsplit('/').next().unwrap_or(file);
                    format!(" | #{} {} at {short}:{line}", fr.level, func)
                } else {
                    format!(" | #{} {func}", fr.level)
                }
            })
            .unwrap_or_default();
        format!("{tid}{frame_str}")
    } else {
        String::new()
    };

    let status_msg = snap
        .status_message
        .as_deref()
        .map(|s| format!("  {}", s))
        .unwrap_or_default();

    let line = Line::from(vec![
        Span::styled(
            " gdbscope ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(exe, Style::default().fg(Color::White)),
        Span::raw(" | "),
        Span::styled(
            state_text,
            Style::default()
                .fg(state_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(stop_info, Style::default().fg(Color::White)),
        Span::styled(context_info, Style::default().fg(Color::White)),
        Span::styled(status_msg, Style::default().fg(Color::DarkGray)),
    ]);

    let bar = Paragraph::new(line).style(
        Style::default()
            .bg(Color::DarkGray)
            .fg(Color::White),
    );

    f.render_widget(bar, rect);
}
