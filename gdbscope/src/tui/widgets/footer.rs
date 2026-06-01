use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::GdbSnapshot;
use super::super::ViewState;
use super::super::input::InputMode;
use super::super::layout::Panel;

fn key(label: &str) -> Span<'_> {
    Span::styled(label, Style::default().fg(Color::Cyan))
}

fn sep(label: &str) -> Span<'_> {
    Span::raw(label)
}

pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState) {
    if view.quit_confirm {
        let line = Line::from(vec![
            Span::styled(
                " Quit? ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Press "),
            Span::styled("y", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw(" to confirm, any other key to cancel"),
        ]);
        let bar = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));
        f.render_widget(bar, rect);
        return;
    }

    if view.input_mode != InputMode::Normal {
        let mode_label = match view.input_mode {
            InputMode::Command => "COMMAND",
            InputMode::Breakpoint => "BREAKPOINT",
            InputMode::Watch => "WATCH",
            InputMode::Memory => "MEMORY",
            InputMode::Eval => "EVAL",
            InputMode::Search => "SEARCH",
            InputMode::Normal => unreachable!(),
        };

        let line = Line::from(vec![
            Span::styled(
                format!(" {} ", mode_label),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Enter=submit  Esc=cancel  Up/Down=history"),
        ]);
        let bar = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));
        f.render_widget(bar, rect);
        return;
    }

    // Global execution hints (always shown)
    let mut spans: Vec<Span> = vec![
        key(" F5"), sep(":Run "),
        key("F7"), sep(":Into "),
        key("F8"), sep(":Next "),
        key("F9"), sep(":Out "),
    ];

    // Panel-specific hints
    match view.focused_panel {
        Panel::Source => {
            spans.extend_from_slice(&[
                sep("| "),
                key("Enter"), sep(":SetBrk "),
                key("F10"), sep(":TogBrk "),
                key("w"), sep(":Watch "),
                key("p"), sep(":Eval "),
                key("."), sep(":GoExec "),
            ]);
        }
        Panel::Stack => {
            spans.extend_from_slice(&[
                sep("| "),
                key("Enter"), sep(":SelectFrame "),
            ]);
        }
        Panel::Threads => {
            spans.extend_from_slice(&[
                sep("| "),
                key("Enter"), sep(":SwitchThread "),
            ]);
        }
        Panel::Breakpoints => {
            spans.extend_from_slice(&[
                sep("| "),
                key("d"), sep(":Del "),
                key("e"), sep(":Toggle "),
                key("b"), sep(":New "),
            ]);
        }
        Panel::Locals => {
            spans.extend_from_slice(&[
                sep("| "),
                key("w"), sep(":Watch "),
                key("p"), sep(":Eval "),
                key("m"), sep(":Memory "),
            ]);
        }
        Panel::Watch => {
            spans.extend_from_slice(&[
                sep("| "),
                key("w"), sep(":Add "),
                key("d"), sep(":Del "),
                key("p"), sep(":Eval "),
                key("m"), sep(":Memory "),
            ]);
        }
        Panel::Registers => {
            spans.extend_from_slice(&[
                sep("| "),
                key("6"), sep(":Toggle "),
            ]);
        }
        Panel::Memory => {
            if view.mem_edit {
                spans.extend_from_slice(&[
                    sep("| "),
                    Span::styled(" EDIT ", Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)),
                    sep(" hex:type "),
                    key("Esc"), sep(":Stop "),
                ]);
            } else {
                spans.extend_from_slice(&[
                    sep("| "),
                    key("m"), sep(":GoAddr "),
                    key("v"), sep(":Select "),
                    key("t"), sep(":Cast "),
                    key("i"), sep(":Edit "),
                    key("\u{2190}\u{2191}\u{2193}\u{2192}"), sep(":Nav "),
                ]);
            }
        }
        Panel::Disasm => {
            spans.extend_from_slice(&[
                sep("| "),
                key("8"), sep(":Toggle "),
            ]);
        }
        Panel::Output => {
            spans.extend_from_slice(&[
                sep("| "),
                key(":"), sep(":Cmd "),
                key(";"), sep(":Repeat "),
            ]);
        }
    }

    // Tail: help + quit + context info
    let thread_info = if let Some(tid) = snap.current_thread_id {
        format!("T{}", tid)
    } else {
        String::new()
    };

    let frame_info = if !snap.stack.is_empty() {
        let fr = &snap.stack[snap.current_frame_level as usize % snap.stack.len()];
        let func = fr.func.as_deref().unwrap_or("??");
        if let (Some(file), Some(line)) = (&fr.file, fr.line) {
            let short = file.rsplit('/').next().unwrap_or(file);
            format!("{func} {short}:{line}")
        } else {
            func.to_string()
        }
    } else {
        String::new()
    };

    spans.extend_from_slice(&[
        sep("| "),
        key("?"), sep(":Help "),
        key("q"), sep(":Quit"),
        sep("  "),
        Span::styled(thread_info, Style::default().fg(Color::DarkGray)),
        sep(" "),
        Span::styled(frame_info, Style::default().fg(Color::DarkGray)),
    ]);

    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(
        Style::default().bg(Color::DarkGray).fg(Color::White),
    );
    f.render_widget(bar, rect);
}
