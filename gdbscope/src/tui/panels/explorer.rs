use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::{ExplorerNode, GdbSnapshot};
use super::super::ViewState;

pub fn visible_nodes(nodes: &[ExplorerNode]) -> Vec<usize> {
    let mut result = Vec::new();
    let mut skip_below: Option<u16> = None;
    for (i, node) in nodes.iter().enumerate() {
        if let Some(max_depth) = skip_below {
            if node.depth > max_depth {
                continue;
            }
            skip_below = None;
        }
        result.push(i);
        if node.has_children && !node.expanded {
            skip_below = Some(node.depth);
        }
    }
    result
}

pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" [I] Explorer ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if snap.explorer_nodes.is_empty() {
        let msg = Paragraph::new("Press I to add an expression to explore.")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, inner);
        return;
    }

    let vis = visible_nodes(&snap.explorer_nodes);
    let visible_height = inner.height as usize;
    let selected = view.explorer_selected;

    let sel_vis_idx = vis.iter().position(|&i| i == selected).unwrap_or(0);

    let scroll = if sel_vis_idx >= visible_height {
        sel_vis_idx - visible_height + 1
    } else {
        0
    };

    let inner_width = inner.width as usize;

    let lines: Vec<Line> = vis
        .iter()
        .skip(scroll)
        .take(visible_height)
        .map(|&idx| {
            let node = &snap.explorer_nodes[idx];
            let is_selected = idx == selected;

            let indent = "  ".repeat(node.depth as usize);
            let arrow = if node.has_children {
                if node.expanded { "\u{25be} " } else { "\u{25b8} " }
            } else {
                "  "
            };

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
            } else if node.changed {
                Style::default()
                    .fg(Color::Red)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };

            let prefix = format!("{indent}{arrow}{}", node.display_name);
            let type_str = if node.type_name.is_empty() {
                String::new()
            } else {
                format!("  {}", node.type_name)
            };
            let val_str = if node.value.is_empty() {
                String::new()
            } else {
                let max_val = inner_width
                    .saturating_sub(prefix.len())
                    .saturating_sub(type_str.len())
                    .saturating_sub(5);
                let v = &node.value;
                if v.len() > max_val && max_val > 3 {
                    format!("  = {}..", &v[..max_val - 2])
                } else {
                    format!("  = {v}")
                }
            };

            Line::from(vec![
                Span::styled(prefix, name_style),
                Span::styled(type_str, type_style),
                Span::styled(val_str, val_style),
            ])
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
