use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::GdbSnapshot;
use super::super::ViewState;

/// Draw the local variables panel.
///
/// Shows each variable with its name, type, and value.  The currently selected
/// variable is highlighted when the panel is focused.
pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" [3] Locals ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if snap.locals.is_empty() {
        let msg = Paragraph::new("No local variables in this frame.")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, inner);
        return;
    }

    let visible_height = inner.height as usize;
    let selected = view.locals_selected.min(snap.locals.len().saturating_sub(1));

    let scroll = if selected >= visible_height {
        selected - visible_height + 1
    } else {
        0
    };

    // Determine the longest name for alignment
    let max_name = snap
        .locals
        .iter()
        .map(|v| v.name.len())
        .max()
        .unwrap_or(0)
        .min(20); // cap alignment width

    let lines: Vec<Line> = snap
        .locals
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(idx, var)| {
            let is_selected = idx == selected;

            let base_style = if is_selected && focused {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            let name_style = if is_selected && focused {
                base_style
            } else {
                Style::default().fg(Color::Green)
            };

            let type_style = if is_selected && focused {
                base_style
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let val_style = if is_selected && focused {
                base_style
            } else {
                Style::default().fg(Color::White)
            };

            Line::from(vec![
                Span::styled(
                    format!("{:<width$} ", var.name, width = max_name),
                    name_style,
                ),
                Span::styled(
                    format!("{} ", if var.type_name.is_empty() { "" } else { &var.type_name }),
                    type_style,
                ),
                Span::styled(format!("= {}", var.value), val_style),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
