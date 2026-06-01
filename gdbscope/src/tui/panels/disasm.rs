use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::GdbSnapshot;
use super::super::ViewState;

/// Draw the disassembly panel.
pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" [8] Disasm ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if snap.disasm.is_empty() {
        let msg = Paragraph::new("No disassembly")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, inner);
        return;
    }

    let visible_height = inner.height as usize;
    let scroll = view.disasm_scroll.min(snap.disasm.len().saturating_sub(visible_height));

    // Find the current PC address from the top stack frame
    let current_pc = snap.stack.first().map(|f| f.addr);

    let lines: Vec<Line> = snap
        .disasm
        .iter()
        .skip(scroll)
        .take(visible_height)
        .map(|inst| {
            let is_current = current_pc == Some(inst.address);

            let marker = if is_current { "=> " } else { "   " };

            let func_info = match (&inst.func_name, inst.offset) {
                (Some(name), Some(off)) => format!("<{}+{}>  ", name, off),
                (Some(name), None) => format!("<{}>  ", name),
                _ => String::new(),
            };

            let style = if is_current {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            Line::from(vec![
                Span::styled(marker, style),
                Span::styled(
                    format!("{:#018x}  ", inst.address),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(func_info, Style::default().fg(Color::Cyan)),
                Span::styled(inst.inst.clone(), style),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
