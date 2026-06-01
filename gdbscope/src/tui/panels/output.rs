use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::{GdbSnapshot, OutputKind};
use super::super::ViewState;

/// Draw the GDB output / console panel.
///
/// Shows console output, target output, log messages, and errors with
/// colour-coded prefixes.  When `output_follow` is true the view auto-scrolls
/// to the bottom.
pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState) {
    let follow_indicator = if view.output_follow { " [follow]" } else { "" };
    let title = format!(" [0] Output{follow_indicator} ");

    let border_style = if view.focused_panel == crate::tui::layout::Panel::Output {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if snap.output.is_empty() {
        return;
    }

    let visible_height = inner.height as usize;
    let total = snap.output.len();

    let scroll = if view.output_follow {
        // Auto-scroll to bottom
        total.saturating_sub(visible_height)
    } else {
        view.output_scroll.min(total.saturating_sub(visible_height))
    };

    let lines: Vec<Line> = snap
        .output
        .iter()
        .skip(scroll)
        .take(visible_height)
        .map(|line| {
            let (prefix, prefix_color) = match line.kind {
                OutputKind::Console => ("gdb> ", Color::Blue),
                OutputKind::Target => ("out> ", Color::Green),
                OutputKind::Log => ("log> ", Color::DarkGray),
                OutputKind::Error => ("err> ", Color::Red),
                OutputKind::Info => ("inf> ", Color::Yellow),
            };

            let text_style = match line.kind {
                OutputKind::Error => Style::default().fg(Color::Red),
                OutputKind::Info => Style::default().fg(Color::Yellow),
                OutputKind::Log => Style::default().fg(Color::DarkGray),
                _ => Style::default().fg(Color::White),
            };

            Line::from(vec![
                Span::styled(prefix, Style::default().fg(prefix_color)),
                Span::styled(line.text.clone(), text_style),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
