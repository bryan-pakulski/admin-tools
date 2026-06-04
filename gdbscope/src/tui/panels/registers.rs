use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::GdbSnapshot;
use super::super::ViewState;

/// Draw the registers panel.
pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" [6] Registers ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if snap.registers.is_empty() {
        let msg = Paragraph::new("No register data. Toggle panel [6] while stopped.")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, inner);
        return;
    }

    let visible_height = inner.height as usize;
    let scroll = view.registers_scroll.min(snap.registers.len().saturating_sub(visible_height));

    let max_name = snap
        .registers
        .iter()
        .map(|r| r.name.len())
        .max()
        .unwrap_or(0)
        .min(12);

    let selected = view.registers_scroll;

    let lines: Vec<Line> = snap
        .registers
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(idx, reg)| {
            let is_selected = idx == selected && focused;
            if is_selected {
                let style = Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD);
                Line::from(Span::styled(
                    format!("{:<width$} {}", reg.name, reg.value, width = max_name),
                    style,
                ))
            } else {
                Line::from(vec![
                    Span::styled(
                        format!("{:<width$} ", reg.name, width = max_name),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::styled(
                        reg.value.clone(),
                        Style::default().fg(Color::White),
                    ),
                ])
            }
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
