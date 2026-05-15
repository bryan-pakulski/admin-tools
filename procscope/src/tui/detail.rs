use std::sync::Arc;
use std::time::{Instant, SystemTime};

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::{RecentPoint, Snapshot};
use crate::tui::widgets::state_strip::StateStrip;
use crate::tui::widgets::{stats_table, thread_info, timeline, transitions_table};
use crate::tui::ViewState;

pub fn render(f: &mut Frame, area: Rect, snap: &Arc<Snapshot>, view: &ViewState) {
    let Some(thread) = snap
        .threads
        .get(view.selected.min(snap.threads.len().saturating_sub(1)))
    else {
        let p = Paragraph::new("no thread selected")
            .block(Block::default().borders(Borders::ALL).title(" detail "));
        f.render_widget(p, area);
        return;
    };

    let window_cutoff = view.window.0.as_duration();
    let cutoff_at = match (window_cutoff, thread.recent.back()) {
        (Some(w), Some(last)) => last.at.checked_sub(w),
        _ => None,
    };

    // Filter recent points to the window. Charts/strip/sparkline still receive
    // the window so axes stay anchored to wall time even if the filtered slice
    // is shorter than the window (e.g. while the buffer is still warming up).
    let recent_owned: std::collections::VecDeque<RecentPoint> = match cutoff_at {
        Some(start) => thread
            .recent
            .iter()
            .filter(|p| p.at >= start)
            .copied()
            .collect(),
        None => thread.recent.clone(),
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(12), // stacked charts (CPU + ctxsw, 6 rows each)
            Constraint::Length(2),  // state strip
            Constraint::Length(9),  // stats table (header + 6 rows + borders)
            Constraint::Min(6),     // transitions table (flexes)
            Constraint::Length(5),  // drill-down info block
        ])
        .split(area);

    render_charts(f, chunks[0], thread, &recent_owned, window_cutoff);
    render_state_strip(f, chunks[1], &recent_owned, window_cutoff);
    stats_table::render(f, chunks[2], &recent_owned, window_cutoff);
    transitions_table::render(f, chunks[3], &thread.transitions, Instant::now());

    // CLK_TCK for uptime calc.
    let clk_tck = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as u64;
    thread_info::render(f, chunks[4], thread, SystemTime::now(), clk_tck);
}

fn render_charts(
    f: &mut Frame,
    area: Rect,
    thread: &crate::state::ThreadView,
    recent: &std::collections::VecDeque<RecentPoint>,
    window: Option<std::time::Duration>,
) {
    // Stacked vertically so both charts share the SAME column geometry and the
    // X-axis of the bottom chart aligns 1:1 with the top chart.
    let charts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    fn user_only(p: &RecentPoint) -> f64 {
        (p.cpu_pct - p.sys_pct).max(0.0) as f64
    }
    fn sys_only(p: &RecentPoint) -> f64 {
        p.sys_pct as f64
    }
    fn vol_ctxsw(p: &RecentPoint) -> f64 {
        p.ctxsw_vol_per_s as f64
    }
    fn invol_ctxsw(p: &RecentPoint) -> f64 {
        p.ctxsw_invol_per_s as f64
    }

    // Top: CPU. Hide X-axis labels so they don't clutter; the bottom chart shows them.
    timeline::render_chart(
        f,
        charts[0],
        &format!("[{}] CPU% user|sys", short(&thread.name, 14)),
        recent,
        window,
        &[
            ("user", Color::Cyan, user_only as timeline::Extractor),
            ("sys", Color::Red, sys_only as timeline::Extractor),
        ],
        "%",
        false, // hide x labels
    );

    // Bottom: context switches. Show X-axis labels (shared across both charts).
    timeline::render_chart(
        f,
        charts[1],
        "ctxsw/s vol|invol",
        recent,
        window,
        &[
            ("vol", Color::Green, vol_ctxsw as timeline::Extractor),
            ("invol", Color::Magenta, invol_ctxsw as timeline::Extractor),
        ],
        "/s",
        true, // show x labels (on the bottom chart only)
    );
}

fn render_state_strip(
    f: &mut Frame,
    area: Rect,
    recent: &std::collections::VecDeque<RecentPoint>,
    window: Option<std::time::Duration>,
) {
    let strip_block = Block::default().borders(Borders::TOP).title(Line::from(vec![
        Span::raw("state strip — "),
        Span::styled("R", Style::default().fg(Color::Green)),
        Span::raw(" run "),
        Span::styled("S", Style::default().fg(Color::Cyan)),
        Span::raw(" sleep "),
        Span::styled("D", Style::default().fg(Color::Red)),
        Span::raw(" disk "),
        Span::styled("T", Style::default().fg(Color::Magenta)),
        Span::raw(" stopped "),
        Span::styled("Z", Style::default().fg(Color::DarkGray)),
        Span::raw(" zombie"),
    ]));
    let inner = strip_block.inner(area);
    f.render_widget(strip_block, area);
    f.render_widget(StateStrip { recent, window }, inner);
}

fn short(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
