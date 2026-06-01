use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::GdbSnapshot;
use super::super::ViewState;

/// Draw the threads panel.
pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" [4] Threads ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if snap.threads.is_empty() {
        let msg = Paragraph::new("No threads")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, inner);
        return;
    }

    let visible_height = inner.height as usize;
    let selected = view.threads_selected.min(snap.threads.len().saturating_sub(1));

    let scroll = if selected >= visible_height {
        selected - visible_height + 1
    } else {
        0
    };

    let lines: Vec<Line> = snap
        .threads
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(idx, thread)| {
            let is_selected = idx == selected;
            let is_current = snap.current_thread_id == Some(thread.id);

            let marker = if is_current { ">" } else { " " };

            let name = thread
                .name
                .as_deref()
                .unwrap_or(&thread.target_id);

            let func = thread
                .frame
                .as_ref()
                .and_then(|f| f.func.as_deref())
                .unwrap_or("");

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

            Line::from(Span::styled(
                format!("{} #{} {} [{}] {}", marker, thread.id, name, thread.state, func),
                style,
            ))
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
