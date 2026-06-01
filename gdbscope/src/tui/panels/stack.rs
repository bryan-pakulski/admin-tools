use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::GdbSnapshot;
use super::super::ViewState;

/// Draw the call stack panel.
///
/// Shows each frame with its level, function name, file/line location, and
/// highlights the currently selected frame.
pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" [2] Stack ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if snap.stack.is_empty() {
        let msg = Paragraph::new("No stack frames")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, inner);
        return;
    }

    let visible_height = inner.height as usize;
    let selected = view.stack_selected.min(snap.stack.len().saturating_sub(1));

    // Scroll so that the selected item is visible
    let scroll = if selected >= visible_height {
        selected - visible_height + 1
    } else {
        0
    };

    let lines: Vec<Line> = snap
        .stack
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(idx, frame)| {
            let is_selected = idx == selected;
            let is_current = frame.level == snap.current_frame_level;

            let marker = if is_current { ">" } else { " " };

            let func = frame
                .func
                .as_deref()
                .unwrap_or("??");

            let location = match (&frame.file, frame.line) {
                (Some(file), Some(line)) => {
                    let short = file.rsplit('/').next().unwrap_or(file);
                    format!(" at {}:{}", short, line)
                }
                (Some(file), None) => {
                    let short = file.rsplit('/').next().unwrap_or(file);
                    format!(" at {}", short)
                }
                _ => format!(" @ {:#x}", frame.addr),
            };

            let style = if is_selected && focused {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            Line::from(vec![
                Span::styled(
                    format!("{} #{} ", marker, frame.level),
                    style,
                ),
                Span::styled(func.to_string(), style.add_modifier(Modifier::BOLD)),
                Span::styled(location, style),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
