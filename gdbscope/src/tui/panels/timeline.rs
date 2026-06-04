use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::super::ViewState;

/// Draw the recording timeline bar.
///
/// This is a compact 4-line panel that shows:
///   - Line 1: border + title with recording state / playback position
///   - Line 2: horizontal timeline of recorded states as colored marks
///   - Line 3: diff summary for the selected playback state
///   - Line 4: bottom border
///
/// When not in playback mode, shows a "LIVE" indicator and the timeline
/// scrolls to the right edge.  When in playback mode, the selected state
/// is highlighted and the diff details are shown.
pub fn draw(f: &mut Frame, rect: Rect, view: &ViewState) {
    let rec_label = if !view.rec_enabled {
        "PAUSED"
    } else if view.playback_mode {
        "PLAYBACK"
    } else {
        "REC"
    };

    let position_label = if view.playback_mode {
        format!("{}/{}", view.playback_index + 1, view.rec_count)
    } else {
        "LIVE".to_string()
    };

    let title = format!(
        " Timeline [{rec_label}] {count} states | {pos} ",
        count = view.rec_count,
        pos = position_label,
    );

    let border_color = if view.playback_mode {
        Color::Magenta
    } else {
        Color::DarkGray
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Build two content lines: timeline bar and diff summary
    let mut lines: Vec<Line> = Vec::new();

    // --- Line 1: Timeline bar ---
    lines.push(build_timeline_bar(view, inner.width as usize));

    // --- Line 2: Diff summary (only meaningful in playback mode) ---
    if inner.height >= 2 {
        lines.push(build_diff_line(view, inner.width as usize));
    }

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

/// Build the horizontal timeline bar showing recorded states as colored marks.
///
/// Each state is rendered as a single character, colored by stop reason:
///   - Yellow: step events
///   - Red: breakpoint hits
///   - Blue: signals
///   - Cyan: watchpoints
///   - White: other stops
///
/// The current playback position (or live position) is shown with brackets.
fn build_timeline_bar(view: &ViewState, width: usize) -> Line<'static> {
    if view.rec_count == 0 {
        return Line::from(Span::styled(
            "  (no recorded states)",
            Style::default().fg(Color::DarkGray),
        ));
    }

    let entries = &view.rec_entries;
    let total = entries.len();

    // Determine the visible window of entries
    // Reserve 2 chars for the position indicator at the end
    let bar_width = width.saturating_sub(2);
    if bar_width == 0 {
        return Line::from("");
    }

    // Determine which index to center on
    let center_idx = if view.playback_mode {
        view.playback_index
    } else {
        total.saturating_sub(1)
    };

    // Calculate visible range
    let half = bar_width / 2;
    let start = if center_idx > half {
        center_idx - half
    } else {
        0
    };
    let end = (start + bar_width).min(total);
    // Re-adjust start if we have room at the end
    let start = if end < bar_width {
        0
    } else {
        end.saturating_sub(bar_width)
    };

    let mut spans: Vec<Span<'static>> = Vec::new();

    // Leading ellipsis if scrolled
    if start > 0 {
        spans.push(Span::styled(
            "<",
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        spans.push(Span::raw(" "));
    }

    for i in start..end {
        let is_playback_pos = view.playback_mode && i == view.playback_index;
        let is_live_pos = !view.playback_mode && i == total.saturating_sub(1);

        let entry = &entries[i];
        let dot_color = stop_label_color(&entry.stop_label);

        let ch = if is_playback_pos || is_live_pos {
            "\u{25c6}" // filled diamond
        } else if entry.is_anchor {
            "\u{25cf}" // filled circle — anchor (breakpoint)
        } else {
            "\u{00b7}" // middle dot — regular step
        };

        let style = if is_playback_pos {
            Style::default()
                .fg(Color::Black)
                .bg(dot_color)
                .add_modifier(Modifier::BOLD)
        } else if is_live_pos {
            Style::default()
                .fg(dot_color)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(dot_color)
        };

        spans.push(Span::styled(ch.to_string(), style));
    }

    // Trailing ellipsis if more entries exist beyond
    if end < total {
        spans.push(Span::styled(
            ">",
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Pad remaining width
    let used: usize = spans.iter().map(|s| s.content.len()).sum();
    if used < width {
        spans.push(Span::raw(" ".repeat(width - used)));
    }

    Line::from(spans)
}

/// Build the diff summary line for the current playback position.
fn build_diff_line(view: &ViewState, width: usize) -> Line<'static> {
    if !view.playback_mode {
        // In live mode, show the source location of the most recent entry
        let loc_text = if let Some(entry) = view.rec_entries.last() {
            let loc = entry.source_loc.as_deref().unwrap_or("??");
            format!("  {} | {}", entry.stop_label, loc)
        } else {
            String::new()
        };
        return Line::from(Span::styled(
            truncate_string(&loc_text, width),
            Style::default().fg(Color::DarkGray),
        ));
    }

    // Playback mode: show diff details
    let mut spans: Vec<Span<'static>> = Vec::new();

    if let Some(ref diff) = view.rec_diff {
        // Stop label
        spans.push(Span::styled(
            format!(" {} ", diff.stop_label),
            Style::default()
                .fg(Color::Black)
                .bg(stop_label_color(&diff.stop_label))
                .add_modifier(Modifier::BOLD),
        ));

        // Source location change
        if diff.source_from.is_some() || diff.source_to.is_some() {
            let from = diff.source_from.as_deref().unwrap_or("??");
            let to = diff.source_to.as_deref().unwrap_or("??");
            spans.push(Span::styled(
                format!(" {}\u{2192}{}", from, to),
                Style::default().fg(Color::White),
            ));
        }

        // Changed variables (compact)
        let sep = Span::styled(" ", Style::default());
        if !diff.vars_changed.is_empty() {
            spans.push(sep.clone());
            spans.push(Span::styled(
                "\u{0394}",  // delta symbol
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            ));
            for (name, old, new) in &diff.vars_changed {
                spans.push(Span::styled(
                    format!(" {}:{}\u{2192}{}", name, old, new),
                    Style::default().fg(Color::Yellow),
                ));
            }
        }

        // Added variables
        if !diff.vars_added.is_empty() {
            spans.push(sep.clone());
            spans.push(Span::styled(
                format!("+{}", diff.vars_added.join(",")),
                Style::default().fg(Color::Green),
            ));
        }

        // Removed variables
        if !diff.vars_removed.is_empty() {
            spans.push(sep.clone());
            spans.push(Span::styled(
                format!("-{}", diff.vars_removed.join(",")),
                Style::default().fg(Color::Red),
            ));
        }

        // Changed watches (compact)
        if !diff.watches_changed.is_empty() {
            spans.push(sep.clone());
            spans.push(Span::styled(
                "\u{0394}w",
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ));
            for (expr, old, new) in &diff.watches_changed {
                spans.push(Span::styled(
                    format!(" {}:{}\u{2192}{}", expr, old, new),
                    Style::default().fg(Color::Cyan),
                ));
            }
        }

        // Register and memory change counts
        let mut counts: Vec<String> = Vec::new();
        if diff.regs_changed > 0 {
            counts.push(format!("regs:{}", diff.regs_changed));
        }
        if diff.mem_changed > 0 {
            counts.push(format!("mem:{}", diff.mem_changed));
        }
        if diff.thread_changed {
            counts.push("thread".to_string());
        }
        if !counts.is_empty() {
            spans.push(Span::styled(
                format!(" | {}", counts.join(" ")),
                Style::default().fg(Color::DarkGray),
            ));
        }
    } else {
        // No diff at this position (e.g., first recorded state)
        let loc = view.rec_playback_source_loc.as_deref().unwrap_or("");
        spans.push(Span::styled(
            format!("  (first recorded state) {}", loc),
            Style::default().fg(Color::DarkGray),
        ));
    }

    Line::from(spans)
}

/// Map a stop label string to a display color.
fn stop_label_color(label: &str) -> Color {
    if label.starts_with("step") {
        Color::Yellow
    } else if label.starts_with("bp#") {
        Color::Red
    } else if label.starts_with("sig:") {
        Color::Blue
    } else if label.starts_with("wp#") {
        Color::Cyan
    } else if label.starts_with("return") {
        Color::Green
    } else if label.starts_with("exit") {
        Color::Magenta
    } else {
        Color::White
    }
}

/// Truncate a string to fit within a given width, adding "..." if needed.
fn truncate_string(s: &str, max_width: usize) -> String {
    if s.len() <= max_width {
        s.to_string()
    } else if max_width > 3 {
        format!("{}...", &s[..max_width - 3])
    } else {
        s[..max_width].to_string()
    }
}
