use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::GdbSnapshot;
use super::super::ViewState;

pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let title = if snap.breakpoints.is_empty() {
        " [5] Breakpoints ".to_string()
    } else {
        format!(" [5] Breakpoints ({}) ", snap.breakpoints.len())
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if snap.breakpoints.is_empty() {
        let msg = Paragraph::new("No breakpoints set. Press b or F10 to add one.")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, inner);
        return;
    }

    let visible_height = inner.height as usize;
    let selected = view.breakpoints_selected.min(snap.breakpoints.len().saturating_sub(1));

    let scroll = if selected >= visible_height {
        selected - visible_height + 1
    } else {
        0
    };

    let lines: Vec<Line> = snap
        .breakpoints
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(idx, bp)| {
            let is_selected = idx == selected;

            let enabled_icon = if bp.enabled { "\u{25cf}" } else { "\u{25cb}" };
            let enabled_color = if bp.enabled { Color::Green } else { Color::Red };

            let location = match (&bp.file, bp.line) {
                (Some(file), Some(line)) => {
                    let short = file.rsplit('/').next().unwrap_or(file);
                    format!("{short}:{line}")
                }
                _ => bp.original_location.clone(),
            };

            let func_info = bp.func.as_deref().unwrap_or("");

            let hits = if bp.hit_count > 0 {
                format!(" ({}x)", bp.hit_count)
            } else {
                String::new()
            };

            // Show condition if present
            let cond_text = bp.condition.as_ref().map(|c| format!(" if {c}")).unwrap_or_default();

            // Show breakpoint type tag for watchpoints
            let type_tag = if bp.bp_type.contains("watchpoint") {
                if bp.bp_type.contains("acc") || bp.bp_type.contains("access") {
                    " [rw]"
                } else if bp.bp_type.contains("read") {
                    " [rd]"
                } else {
                    " [wp]"
                }
            } else {
                ""
            };

            if is_selected && focused {
                let style = Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD);
                Line::from(Span::styled(
                    format!("{enabled_icon} #{:<3} {location:<30} {func_info}{type_tag}{cond_text}{hits}",
                        bp.number),
                    style,
                ))
            } else {
                let mut spans = vec![
                    Span::styled(
                        format!("{enabled_icon} "),
                        Style::default().fg(enabled_color),
                    ),
                    Span::styled(
                        format!("#{:<3} ", bp.number),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(
                        location,
                        Style::default().fg(Color::White),
                    ),
                ];
                if !func_info.is_empty() {
                    spans.push(Span::styled(
                        format!(" {func_info}"),
                        Style::default().fg(Color::Yellow),
                    ));
                }
                if !type_tag.is_empty() {
                    spans.push(Span::styled(
                        type_tag.to_string(),
                        Style::default().fg(Color::Magenta),
                    ));
                }
                if !cond_text.is_empty() {
                    spans.push(Span::styled(
                        cond_text,
                        Style::default().fg(Color::Cyan),
                    ));
                }
                if !hits.is_empty() {
                    spans.push(Span::styled(
                        hits,
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                Line::from(spans)
            }
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
