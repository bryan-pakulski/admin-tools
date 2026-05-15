use std::collections::VecDeque;
use std::time::Instant;

use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::state::Transition;
use crate::tui::widgets::state_strip::state_color;
use crate::tui::widgets::stats::fmt_duration_ms;

/// Render the transition log as a table with columns:
///   wall-time  state  syscall  wchan  time-in-state
///
/// `time-in-state` for transition `i` is `transitions[i+1].at - transitions[i].at`
/// (or `now - transitions[last].at` for the most recent).
pub fn render(f: &mut Frame, area: Rect, transitions: &VecDeque<Transition>, now: Instant) {
    let header_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Gray)
        .add_modifier(Modifier::BOLD);
    let header = Row::new(vec![
        Cell::from(" wall time"),
        Cell::from("st"),
        Cell::from("syscall"),
        Cell::from("wchan"),
        Cell::from("duration"),
    ])
    .style(header_style);

    // Materialise transitions in newest-first order and compute "duration in this state".
    let mut entries: Vec<(&Transition, std::time::Duration)> = Vec::with_capacity(transitions.len());
    let trs: Vec<&Transition> = transitions.iter().collect();
    for (i, t) in trs.iter().enumerate() {
        let next_at = if i + 1 < trs.len() {
            trs[i + 1].at
        } else {
            now
        };
        let dur = next_at.saturating_duration_since(t.at);
        entries.push((*t, dur));
    }
    entries.reverse(); // newest first

    let rows: Vec<Row> = entries
        .into_iter()
        .map(|(tr, dur)| {
            let wall_secs = (tr.wall_us / 1_000_000) as i64;
            let frac_ms = (((tr.wall_us % 1_000_000) / 1_000) as i64).abs();
            // HH:MM:SS.mmm
            let time = format_wall(wall_secs, frac_ms as u32);

            let st_color = state_color(tr.state);
            let state_cell = Cell::from(Span::styled(
                format!(" {} ", tr.state.label()),
                Style::default().fg(Color::Black).bg(st_color),
            ));
            let syscall_cell = Cell::from(Span::styled(
                tr.syscall_name.unwrap_or("—").to_string(),
                Style::default().fg(Color::Yellow),
            ));
            let wchan_cell = if tr.wchan.is_empty() {
                Cell::from(Span::styled(
                    "—".to_string(),
                    Style::default().fg(Color::DarkGray),
                ))
            } else {
                Cell::from(Span::styled(
                    tr.wchan.clone(),
                    Style::default().fg(Color::Cyan),
                ))
            };
            let dur_ms = dur.as_millis() as u64;
            let dur_color = match dur_ms {
                v if v >= 5000 => Color::Red,
                v if v >= 1000 => Color::Yellow,
                _ => Color::Cyan,
            };
            let dur_cell = Cell::from(Span::styled(
                fmt_duration_ms(dur_ms),
                Style::default()
                    .fg(dur_color)
                    .add_modifier(Modifier::BOLD),
            ));

            Row::new(vec![
                Cell::from(Span::styled(time, Style::default().fg(Color::DarkGray))),
                state_cell,
                syscall_cell,
                wchan_cell,
                dur_cell,
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(12),
        Constraint::Length(3),
        Constraint::Length(14),
        Constraint::Length(24),
        Constraint::Length(10),
    ];

    let title = if transitions.is_empty() {
        " transitions (none yet) ".to_string()
    } else {
        format!(" transitions ({}, newest first) ", transitions.len())
    };
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(table, area);
}

fn format_wall(unix_secs: i64, frac_ms: u32) -> String {
    // Compute HH:MM:SS from unix_secs (UTC).
    let secs_in_day = unix_secs.rem_euclid(86_400);
    let h = secs_in_day / 3600;
    let m = (secs_in_day % 3600) / 60;
    let s = secs_in_day % 60;
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, frac_ms)
}
