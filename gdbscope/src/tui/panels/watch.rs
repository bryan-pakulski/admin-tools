use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::GdbSnapshot;
use super::super::ViewState;

/// Draw the watch expressions panel.
pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" [9] Watch ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if snap.watch_expressions.is_empty() {
        let msg = Paragraph::new("No watch expressions (press 'w' to add)")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, inner);
        return;
    }

    let visible_height = inner.height as usize;
    let selected = view.watch_selected.min(snap.watch_expressions.len().saturating_sub(1));

    let scroll = if selected >= visible_height {
        selected - visible_height + 1
    } else {
        0
    };

    let lines: Vec<Line> = snap
        .watch_expressions
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(idx, expr)| {
            let is_selected = idx == selected;

            let style = if is_selected && focused {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            if let Some(ref err) = expr.error {
                Line::from(vec![
                    Span::styled(
                        format!("{} ", expr.expression),
                        if is_selected && focused {
                            style
                        } else {
                            Style::default().fg(Color::White)
                        },
                    ),
                    Span::styled(
                        format!("= <{}>", err),
                        if is_selected && focused {
                            style
                        } else {
                            Style::default().fg(Color::Red)
                        },
                    ),
                ])
            } else {
                Line::from(vec![
                    Span::styled(
                        format!("{} ", expr.expression),
                        if is_selected && focused {
                            style
                        } else {
                            Style::default().fg(Color::Green)
                        },
                    ),
                    Span::styled(
                        format!("= {}", expr.value),
                        if is_selected && focused {
                            style
                        } else {
                            Style::default().fg(Color::White)
                        },
                    ),
                ])
            }
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
