use std::collections::VecDeque;

use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::symbols;
use ratatui::text::Span;
use ratatui::widgets::{Axis, Block, Borders, Chart, Dataset, GraphType};
use ratatui::Frame;

use crate::state::RecentPoint;

pub type Extractor = fn(&RecentPoint) -> f64;

/// Render a Chart with one or two datasets (line graph).
///
/// The X-axis is anchored to wall time: rightmost = the newest sample,
/// leftmost = `now - window`. If `window` is None the axis covers the data extent.
/// Y labels are right-padded to a fixed width so two stacked charts share the
/// SAME plot-area column geometry — that's what makes the X axes line up.
///
/// `show_x_labels` lets the caller hide x-axis labels on the upper of a stacked
/// pair so they only appear once on the bottom chart.
pub fn render_chart(
    f: &mut Frame,
    area: Rect,
    title: &str,
    recent: &VecDeque<RecentPoint>,
    window: Option<std::time::Duration>,
    series: &[(&str, Color, Extractor)],
    y_label: &str,
    show_x_labels: bool,
) {
    if recent.is_empty() {
        let block = Block::default().borders(Borders::ALL).title(title);
        f.render_widget(block, area);
        return;
    }

    let last_at = recent.back().unwrap().at;

    let datas: Vec<Vec<(f64, f64)>> = series
        .iter()
        .map(|(_, _, extract)| {
            recent
                .iter()
                .map(|p| {
                    let secs_before = -(last_at.duration_since(p.at).as_secs_f64());
                    (secs_before, extract(p))
                })
                .collect()
        })
        .collect();

    let mut y_max = 1.0f64;
    for d in &datas {
        for (_, y) in d {
            if *y > y_max {
                y_max = *y;
            }
        }
    }
    y_max = (y_max * 1.15).max(1.0);

    // X-axis bounds: window-anchored when given, otherwise data extent.
    let x_min = match window {
        Some(w) => -w.as_secs_f64(),
        None => datas[0].first().map(|(x, _)| *x).unwrap_or(-1.0),
    };
    let x_max = 0.0;

    let datasets: Vec<Dataset<'_>> = series
        .iter()
        .zip(datas.iter())
        .map(|((label, color, _), data)| {
            Dataset::default()
                .name(*label)
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(*color))
                .data(data)
        })
        .collect();

    // Pad Y labels to a fixed width so the plot area starts at the same column
    // regardless of magnitude.
    const Y_LABEL_W: usize = 6;
    let y0 = format!("{:>w$}", format_y(0.0), w = Y_LABEL_W);
    let yh = format!("{:>w$}", format_y(y_max / 2.0), w = Y_LABEL_W);
    let ym = format!("{:>w$}", format_y(y_max), w = Y_LABEL_W);

    let x_labels: Vec<Span> = if show_x_labels {
        vec![
            Span::raw(format_time_label(x_min)),
            Span::raw(format_time_label(x_min * 0.75)),
            Span::raw(format_time_label(x_min * 0.50)),
            Span::raw(format_time_label(x_min * 0.25)),
            Span::raw("now"),
        ]
    } else {
        vec![]
    };

    let chart = Chart::new(datasets)
        .block(Block::default().borders(Borders::ALL).title(title))
        .x_axis(
            Axis::default()
                .style(Style::default().fg(Color::DarkGray))
                .labels(x_labels)
                .bounds([x_min, x_max]),
        )
        .y_axis(
            Axis::default()
                .style(Style::default().fg(Color::DarkGray))
                .title(Span::styled(y_label, Style::default().fg(Color::DarkGray)))
                .labels(vec![Span::raw(y0), Span::raw(yh), Span::raw(ym)])
                .bounds([0.0, y_max]),
        );

    f.render_widget(chart, area);
}

fn format_y(v: f64) -> String {
    if v >= 1000.0 {
        format!("{:.0}", v)
    } else if v >= 10.0 {
        format!("{:.0}", v)
    } else if v >= 1.0 {
        format!("{:.1}", v)
    } else {
        format!("{:.2}", v)
    }
}

fn format_time_label(secs_before: f64) -> String {
    let s = secs_before.abs();
    if s >= 3600.0 {
        let h = s / 3600.0;
        if (h - h.round()).abs() < 0.05 {
            format!("-{:.0}h", h)
        } else {
            format!("-{:.1}h", h)
        }
    } else if s >= 60.0 {
        let m = s / 60.0;
        if (m - m.round()).abs() < 0.05 {
            format!("-{:.0}m", m)
        } else {
            format!("-{:.1}m", m)
        }
    } else if s >= 1.0 {
        format!("-{:.0}s", s)
    } else if s > 0.0 {
        format!("-{:.0}ms", s * 1000.0)
    } else {
        "0".to_string()
    }
}
