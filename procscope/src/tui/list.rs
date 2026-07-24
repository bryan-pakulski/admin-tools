use std::sync::Arc;

use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Cell, Row, Table, TableState};
use ratatui::Frame;

use crate::state::{Snapshot, ThreadView};
use crate::tui::widgets::sparkline;
use crate::tui::widgets::state_strip::state_color;
use crate::tui::widgets::stats::{fmt_pct, fmt_rate};
use crate::tui::ViewState;

pub fn render(f: &mut Frame, area: Rect, snap: &Arc<Snapshot>, view: &ViewState) {
    let header_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Gray)
        .add_modifier(Modifier::BOLD);
    let avg_tag = match view.list_avg {
        Some(d) if !d.is_zero() => format!("(avg {})", crate::tui::fmt_list_avg(view.list_avg)),
        _ => String::new(),
    };
    let header = Row::new(vec![
        Cell::from(" tid"),
        Cell::from("name"),
        Cell::from("st"),
        Cell::from(format!("cpu% {avg_tag}")),
        Cell::from("sys%"),
        Cell::from("ctxsw v/iv"),
        Cell::from("io%"),
        Cell::from("wchan / syscall"),
        Cell::from("timeline"),
    ])
    .style(header_style);

    let rows: Vec<Row> = snap
        .threads
        .iter()
        .map(|t| {
            let st_color = state_color(t.state);
            let st_span = Span::styled(
                format!(" {} ", t.state.label()),
                Style::default().fg(Color::Black).bg(st_color),
            );

            let wchan_cell = if let Some(f) = &t.freeze {
                let stuck = f.since.elapsed().as_millis() as u64;
                Cell::from(Span::styled(
                    format!(
                        "★ {} ({})",
                        short(&t.wchan, 18),
                        crate::tui::widgets::stats::fmt_duration_ms(stuck)
                    ),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ))
            } else if t.wchan.is_empty() {
                let sc = t.syscall_name.unwrap_or("running");
                Cell::from(Span::styled(sc.to_string(), Style::default().fg(Color::Green)))
            } else {
                let sc = t.syscall_name.unwrap_or("");
                let label = if sc.is_empty() {
                    t.wchan.clone()
                } else {
                    format!("{} / {}", short(&t.wchan, 18), sc)
                };
                Cell::from(Span::styled(label, Style::default().fg(Color::Cyan)))
            };

            // Smooth the displayed numbers with a moving average over view.list_avg.
            // Falls back to the published instantaneous values when avg is None.
            let avg = averaged(t, view.list_avg);

            let cpu_color = match avg.cpu_pct {
                v if v >= 80.0 => Color::Red,
                v if v >= 50.0 => Color::Yellow,
                v if v > 0.0 => Color::Cyan,
                _ => Color::DarkGray,
            };

            Row::new(vec![
                Cell::from(format!(" {:>7}", t.tid)),
                Cell::from(short(&t.name, 20)),
                Cell::from(st_span),
                Cell::from(Span::styled(
                    fmt_pct(avg.cpu_pct),
                    Style::default().fg(cpu_color),
                )),
                Cell::from(Span::styled(
                    fmt_pct(avg.sys_pct),
                    Style::default().fg(Color::Cyan),
                )),
                Cell::from(Span::styled(
                    format!("{}/{}", fmt_rate(avg.ctxsw_v), fmt_rate(avg.ctxsw_iv)),
                    Style::default().fg(Color::Cyan),
                )),
                Cell::from(Span::styled(
                    format!("{:>4.0}%", t.iowait_pct),
                    if t.iowait_pct > 50.0 {
                        Style::default().fg(Color::Red)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                )),
                wchan_cell,
                Cell::from(sparkline::inline(
                    t,
                    area.width.saturating_sub(80) as usize,
                    view.window.0.as_duration(),
                )),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(20),
            Constraint::Length(4),
            Constraint::Length(6),
            Constraint::Length(6),
            Constraint::Length(13),
            Constraint::Length(5),
            Constraint::Length(40),
            Constraint::Min(8),
        ],
    )
    .header(header)
    .row_highlight_style(
        Style::default()
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol("» ")
    .block(Block::default().borders(Borders::NONE));

    let mut state = TableState::default();
    let sel = view.selected.min(snap.threads.len().saturating_sub(1));
    state.select(if snap.threads.is_empty() { None } else { Some(sel) });
    f.render_stateful_widget(table, area, &mut state);
}

struct AvgValues {
    cpu_pct: f32,
    sys_pct: f32,
    ctxsw_v: f32,
    ctxsw_iv: f32,
}

/// Mean of the last `window` of samples in `t.recent`. With no window, returns
/// the published instantaneous values. With an empty window slice, also falls
/// back to the instantaneous values so the cell never reads as zero spuriously.
fn averaged(t: &ThreadView, window: Option<std::time::Duration>) -> AvgValues {
    let Some(w) = window else {
        return AvgValues {
            cpu_pct: t.cpu_pct,
            sys_pct: t.sys_pct,
            ctxsw_v: t.ctxsw_vol_per_s,
            ctxsw_iv: t.ctxsw_invol_per_s,
        };
    };
    let Some(newest) = t.recent.back() else {
        return AvgValues {
            cpu_pct: t.cpu_pct,
            sys_pct: t.sys_pct,
            ctxsw_v: t.ctxsw_vol_per_s,
            ctxsw_iv: t.ctxsw_invol_per_s,
        };
    };
    let cutoff = newest.at.checked_sub(w);

    let mut n = 0u32;
    let mut cpu = 0.0f32;
    let mut sys = 0.0f32;
    let mut cv = 0.0f32;
    let mut civ = 0.0f32;
    for p in t.recent.iter().rev() {
        if let Some(c) = cutoff {
            if p.at < c {
                break;
            }
        }
        cpu += p.cpu_pct;
        sys += p.sys_pct;
        cv += p.ctxsw_vol_per_s;
        civ += p.ctxsw_invol_per_s;
        n += 1;
    }
    if n == 0 {
        return AvgValues {
            cpu_pct: t.cpu_pct,
            sys_pct: t.sys_pct,
            ctxsw_v: t.ctxsw_vol_per_s,
            ctxsw_iv: t.ctxsw_invol_per_s,
        };
    }
    let nf = n as f32;
    AvgValues {
        cpu_pct: cpu / nf,
        sys_pct: sys / nf,
        ctxsw_v: cv / nf,
        ctxsw_iv: civ / nf,
    }
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
