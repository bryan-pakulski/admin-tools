use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::super::{InspectorState, ViewState};

pub fn draw(f: &mut Frame, area: Rect, view: &ViewState) {
    let inspector = match view.inspector {
        Some(ref i) => i,
        None => return,
    };

    let lines = build_lines(inspector);
    let content_height = lines.len() as u16 + 2; // +2 for borders

    let content_width = lines.iter()
        .map(|l| l.spans.iter().map(|s| s.content.len()).sum::<usize>())
        .max()
        .unwrap_or(30) as u16 + 4; // padding

    let width = content_width.max(40).min(area.width.saturating_sub(4));
    let height = content_height.min(area.height.saturating_sub(2));

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect::new(x, y, width, height);

    f.render_widget(Clear, rect);

    let title = format!(" Inspect: {} ", truncate(&inspector.expr, 30));
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, rect);
}

fn build_lines(inspector: &InspectorState) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    if !inspector.type_name.is_empty() {
        lines.push(detail_line("Type", &inspector.type_name));
    }
    if !inspector.value.is_empty() {
        lines.push(detail_line("Value", &inspector.value));
    }
    if inspector.type_name.is_empty() && inspector.value.is_empty() {
        lines.push(Line::from(Span::styled(
            "  (no type/value info available)".to_string(),
            Style::default().fg(Color::DarkGray),
        )));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Actions".to_string(),
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    )));

    for (i, action) in inspector.actions.iter().enumerate() {
        let is_selected = i == inspector.selected;
        let style = if is_selected {
            Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        let key_style = if is_selected {
            style
        } else {
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)
        };

        let desc_style = if is_selected {
            style
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let prefix = if is_selected { "> " } else { "  " };
        let entry = format!("[{}] {:<14}", action.key, action.label);
        let desc = action.desc.to_string();

        lines.push(Line::from(vec![
            Span::styled(prefix.to_string(), style),
            Span::styled(entry, key_style),
            Span::styled(desc, desc_style),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "  Press key or Esc to close".to_string(),
        Style::default().fg(Color::DarkGray),
    )));

    lines
}

fn detail_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("  {label:<8}"),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(
            value.to_string(),
            Style::default().fg(Color::White),
        ),
    ])
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}
