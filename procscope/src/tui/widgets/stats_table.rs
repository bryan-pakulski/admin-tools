use std::collections::VecDeque;
use std::time::Duration;

use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::state::RecentPoint;
use crate::tui::widgets::stats::{fmt_bps, fmt_pct, fmt_rate};

#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub p50: f64,
    pub p95: f64,
    pub p99: f64,
    pub n: usize,
}

impl Stats {
    pub fn from_iter(values: impl IntoIterator<Item = f64>) -> Self {
        let mut v: Vec<f64> = values.into_iter().collect();
        if v.is_empty() {
            return Self::default();
        }
        let n = v.len();
        let sum: f64 = v.iter().sum();
        let mean = sum / n as f64;
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let pick = |q: f64| -> f64 {
            let idx = ((n - 1) as f64 * q).round() as usize;
            v[idx.min(n - 1)]
        };
        Self {
            min: v[0],
            max: *v.last().unwrap(),
            mean,
            p50: pick(0.50),
            p95: pick(0.95),
            p99: pick(0.99),
            n,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Fmt {
    Pct,
    Rate,
    Bps,
    Ms,
}

impl Fmt {
    fn apply(self, v: f64) -> String {
        match self {
            Fmt::Pct => fmt_pct(v as f32),
            Fmt::Rate => fmt_rate(v as f32),
            Fmt::Bps => fmt_bps(v),
            Fmt::Ms => {
                if v < 1.0 {
                    format!("{:.2}ms", v)
                } else if v < 100.0 {
                    format!("{:.1}ms", v)
                } else {
                    format!("{:.0}ms", v)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_from_simple_iter() {
        let s = Stats::from_iter([1.0, 2.0, 3.0, 4.0, 5.0]);
        assert_eq!(s.n, 5);
        assert_eq!(s.min, 1.0);
        assert_eq!(s.max, 5.0);
        assert_eq!(s.mean, 3.0);
        assert_eq!(s.p50, 3.0);
    }

    #[test]
    fn stats_empty_safe() {
        let s = Stats::from_iter(std::iter::empty::<f64>());
        assert_eq!(s.n, 0);
        assert_eq!(s.min, 0.0);
        assert_eq!(s.max, 0.0);
    }

    #[test]
    fn stats_p95_p99() {
        let s = Stats::from_iter((1..=100).map(|i| i as f64));
        assert!((s.p95 - 95.0).abs() <= 1.0);
        assert!((s.p99 - 99.0).abs() <= 1.0);
    }
}

type Extractor = fn(&RecentPoint) -> f64;

/// Render a table with one column per field (cpu%, sys%, ctxsw v/iv, rchar, wchar, sched_wait)
/// and one row per stat (min/avg/p50/p95/p99/max).
pub fn render(
    f: &mut Frame,
    area: Rect,
    recent: &VecDeque<RecentPoint>,
    window: Option<Duration>,
) {
    let cutoff = match (window, recent.back()) {
        (Some(w), Some(last)) => last.at.checked_sub(w),
        _ => None,
    };

    // Per-field stats over the windowed slice.
    fn compute(
        recent: &VecDeque<RecentPoint>,
        cutoff: Option<std::time::Instant>,
        extract: Extractor,
    ) -> Stats {
        Stats::from_iter(
            recent
                .iter()
                .filter(|p| cutoff.map_or(true, |c| p.at >= c))
                .map(extract),
        )
    }

    let fields: &[(&str, Fmt, Extractor)] = &[
        ("cpu%", Fmt::Pct, |p| p.cpu_pct as f64),
        ("sys%", Fmt::Pct, |p| p.sys_pct as f64),
        ("ctxsw v/s", Fmt::Rate, |p| p.ctxsw_vol_per_s as f64),
        ("ctxsw iv/s", Fmt::Rate, |p| p.ctxsw_invol_per_s as f64),
        ("rchar B/s", Fmt::Bps, |p| p.rchar_bps),
        ("wchar B/s", Fmt::Bps, |p| p.wchar_bps),
        ("sched wait", Fmt::Ms, |p| p.sched_wait_ms_per_s as f64),
    ];

    let stats: Vec<Stats> = fields
        .iter()
        .map(|(_, _, ex)| compute(recent, cutoff, *ex))
        .collect();

    let header_style = Style::default()
        .fg(Color::Black)
        .bg(Color::Gray)
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(Color::DarkGray);

    let header_cells: Vec<Cell> = std::iter::once(Cell::from(" stat "))
        .chain(fields.iter().map(|(name, _, _)| Cell::from(*name)))
        .collect();
    let header = Row::new(header_cells).style(header_style);

    let mk_row = |label: &str, color: Color, pick: fn(&Stats) -> f64| -> Row<'static> {
        let mut cells: Vec<Cell> = Vec::with_capacity(1 + fields.len());
        cells.push(Cell::from(Span::styled(
            format!(" {}", label),
            label_style,
        )));
        for (s, (_, fmt, _)) in stats.iter().zip(fields.iter()) {
            cells.push(Cell::from(Span::styled(
                fmt.apply(pick(s)),
                Style::default().fg(color),
            )));
        }
        Row::new(cells)
    };

    let rows = vec![
        mk_row("min", Color::Green, |s| s.min),
        mk_row("avg", Color::Cyan, |s| s.mean),
        mk_row("p50", Color::Cyan, |s| s.p50),
        mk_row("p95", Color::Yellow, |s| s.p95),
        mk_row("p99", Color::Red, |s| s.p99),
        mk_row("max", Color::Red, |s| s.max),
    ];

    let widths: Vec<Constraint> = std::iter::once(Constraint::Length(6))
        .chain(fields.iter().map(|_| Constraint::Length(11)))
        .collect();

    let title = match stats.first().map(|s| s.n).unwrap_or(0) {
        0 => " stats (no samples) ".to_string(),
        n => format!(" stats  n={}  ", n),
    };
    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(title));

    f.render_widget(table, area);
}
