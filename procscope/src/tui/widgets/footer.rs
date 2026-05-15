use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use std::sync::Arc;

use crate::state::Snapshot;
use crate::tui::ViewState;

pub fn render(f: &mut Frame, area: Rect, snap: &Arc<Snapshot>, view: &ViewState) {
    let key = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let label = Style::default().fg(Color::Gray);
    let warn = Style::default()
        .fg(Color::Black)
        .bg(Color::Yellow)
        .add_modifier(Modifier::BOLD);

    if view.quit_confirm {
        let line = Line::from(vec![
            Span::styled(" quit? ", warn),
            Span::raw(" press "),
            Span::styled("y", key),
            Span::raw("/"),
            Span::styled("Enter", key),
            Span::raw(" to confirm, "),
            Span::styled("n", key),
            Span::raw("/"),
            Span::styled("Esc", key),
            Span::raw(" to cancel"),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    let pause_label = if snap.paused { "resume" } else { "pause" };
    let win = view.window.0.label();
    let scale = if view.log_scale { "log" } else { "lin" };
    let avg = crate::tui::fmt_list_avg(view.list_avg);

    let line = Line::from(vec![
        Span::styled("↑↓", key),
        Span::styled(" select  ", label),
        Span::styled("Enter", key),
        Span::styled(" detail  ", label),
        Span::styled("Space", key),
        Span::styled(format!(" {pause_label}  "), label),
        Span::styled("+/-", key),
        Span::styled(" freq  ", label),
        Span::styled("w", key),
        Span::styled(format!(" win:{win}  "), label),
        Span::styled("a", key),
        Span::styled(format!(" avg:{avg}  "), label),
        Span::styled("l", key),
        Span::styled(format!(" scale:{scale}  "), label),
        Span::styled("f", key),
        Span::styled(" filter  ", label),
        Span::styled("e", key),
        Span::styled(" csv  ", label),
        Span::styled("h", key),
        Span::styled(" help  ", label),
        Span::styled("q", key),
        Span::styled(" quit", label),
    ]);
    f.render_widget(Paragraph::new(line), area);
}
