pub mod input;
pub mod list;
pub mod detail;
pub mod widgets;

use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Terminal;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

use crate::config::Config;
use crate::state::SharedSnapshot;

pub use input::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Window {
    FifteenSec,
    ThirtySec,
    OneMin,
    TwoMin,
    FiveMin,
    TenMin,
    ThirtyMin,
    All,
}

impl Window {
    pub fn cycle(self) -> Self {
        match self {
            Self::FifteenSec => Self::ThirtySec,
            Self::ThirtySec => Self::OneMin,
            Self::OneMin => Self::TwoMin,
            Self::TwoMin => Self::FiveMin,
            Self::FiveMin => Self::TenMin,
            Self::TenMin => Self::ThirtyMin,
            Self::ThirtyMin => Self::All,
            Self::All => Self::FifteenSec,
        }
    }
    pub fn cycle_back(self) -> Self {
        match self {
            Self::FifteenSec => Self::All,
            Self::ThirtySec => Self::FifteenSec,
            Self::OneMin => Self::ThirtySec,
            Self::TwoMin => Self::OneMin,
            Self::FiveMin => Self::TwoMin,
            Self::TenMin => Self::FiveMin,
            Self::ThirtyMin => Self::TenMin,
            Self::All => Self::ThirtyMin,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::FifteenSec => "15s",
            Self::ThirtySec => "30s",
            Self::OneMin => "1m",
            Self::TwoMin => "2m",
            Self::FiveMin => "5m",
            Self::TenMin => "10m",
            Self::ThirtyMin => "30m",
            Self::All => "all",
        }
    }
    pub fn as_duration(self) -> Option<Duration> {
        match self {
            Self::FifteenSec => Some(Duration::from_secs(15)),
            Self::ThirtySec => Some(Duration::from_secs(30)),
            Self::OneMin => Some(Duration::from_secs(60)),
            Self::TwoMin => Some(Duration::from_secs(120)),
            Self::FiveMin => Some(Duration::from_secs(300)),
            Self::TenMin => Some(Duration::from_secs(600)),
            Self::ThirtyMin => Some(Duration::from_secs(1800)),
            Self::All => None,
        }
    }
    /// Pick the closest enum variant >= the requested duration.
    pub fn closest_ge(d: Option<Duration>) -> Self {
        let secs = match d {
            None => return Self::All,
            Some(d) => d.as_secs(),
        };
        if secs <= 15 {
            Self::FifteenSec
        } else if secs <= 30 {
            Self::ThirtySec
        } else if secs <= 60 {
            Self::OneMin
        } else if secs <= 120 {
            Self::TwoMin
        } else if secs <= 300 {
            Self::FiveMin
        } else if secs <= 600 {
            Self::TenMin
        } else if secs <= 1800 {
            Self::ThirtyMin
        } else {
            Self::All
        }
    }
}

#[derive(Debug, Clone)]
pub struct ViewState {
    pub selected: usize,
    pub detail_open: bool,
    pub window: WindowState,
    pub log_scale: bool,
    pub filter_input: Option<String>,
    pub quit_confirm: bool,
    pub status_until: Option<Instant>,
    pub help_open: bool,
    pub help_scroll: u16,
    /// Moving-average window for thread-list cell values. None = show last sample.
    pub list_avg: Option<Duration>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            selected: 0,
            detail_open: false,
            window: WindowState(Window::OneMin),
            log_scale: false,
            filter_input: None,
            quit_confirm: false,
            status_until: None,
            help_open: false,
            help_scroll: 0,
            list_avg: Some(Duration::from_secs(1)),
        }
    }
}

/// Cycle through useful list-averaging windows. None = instantaneous.
pub fn cycle_list_avg(cur: Option<Duration>) -> Option<Duration> {
    const STEPS_MS: &[u64] = &[500, 1_000, 3_000, 10_000, 30_000, 60_000];
    match cur {
        None => Some(Duration::from_millis(STEPS_MS[0])),
        Some(d) => {
            let cur_ms = d.as_millis() as u64;
            let next_idx = STEPS_MS.iter().position(|m| *m == cur_ms).map(|i| i + 1);
            match next_idx {
                Some(i) if i < STEPS_MS.len() => Some(Duration::from_millis(STEPS_MS[i])),
                _ => None, // wrap to off
            }
        }
    }
}

pub fn fmt_list_avg(w: Option<Duration>) -> String {
    match w {
        None => "off".to_string(),
        Some(d) if d < Duration::from_secs(1) => format!("{:.1}s", d.as_secs_f32()),
        Some(d) if d < Duration::from_secs(60) => format!("{}s", d.as_secs()),
        Some(d) => format!("{}m", d.as_secs() / 60),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct WindowState(pub Window);
impl Default for WindowState {
    fn default() -> Self {
        Self(Window::OneMin)
    }
}

pub struct TuiHandles {
    pub paused_tx: watch::Sender<bool>,
    pub interval_tx: watch::Sender<Duration>,
    pub status_tx: mpsc::Sender<StatusMessage>,
}

#[derive(Debug, Clone)]
pub struct StatusMessage {
    pub text: String,
    pub duration: Duration,
}

pub async fn run(
    snapshot: SharedSnapshot,
    cfg: Config,
    paused_tx: watch::Sender<bool>,
    interval_tx: watch::Sender<Duration>,
    filter_tx: watch::Sender<Option<String>>,
) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = event_loop(&mut terminal, snapshot, cfg, paused_tx, interval_tx, filter_tx).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

async fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    snapshot: SharedSnapshot,
    cfg: Config,
    paused_tx: watch::Sender<bool>,
    interval_tx: watch::Sender<Duration>,
    filter_tx: watch::Sender<Option<String>>,
) -> Result<()> {
    let mut view = ViewState::default();
    view.window.0 = Window::closest_ge(cfg.initial_window);
    view.list_avg = cfg.list_avg;
    let frame_ms = (1000 / cfg.redraw_hz.max(1)) as u64;
    let mut tick = tokio::time::interval(Duration::from_millis(frame_ms.max(10)));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut status_text: Option<String> = None;
    let mut status_until: Option<Instant> = None;

    loop {
        tick.tick().await;

        while event::poll(Duration::from_millis(0))? {
            if let Event::Key(key) = event::read()? {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Help overlay swallows most keys; arrows/jk and PageUp/PageDn scroll.
                if view.help_open {
                    match key.code {
                        KeyCode::Esc
                        | KeyCode::Char('h')
                        | KeyCode::Char('H')
                        | KeyCode::Char('q')
                        | KeyCode::Char('Q')
                        | KeyCode::Char('?')
                        | KeyCode::F(1) => {
                            view.help_open = false;
                            view.help_scroll = 0;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            view.help_scroll = view.help_scroll.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            view.help_scroll = view.help_scroll.saturating_add(1);
                        }
                        KeyCode::PageUp => {
                            view.help_scroll = view.help_scroll.saturating_sub(10);
                        }
                        KeyCode::PageDown => {
                            view.help_scroll = view.help_scroll.saturating_add(10);
                        }
                        KeyCode::Home => view.help_scroll = 0,
                        KeyCode::End => view.help_scroll = u16::MAX,
                        _ => {}
                    }
                    continue;
                }

                // Filter prompt swallows keys.
                if let Some(buf) = view.filter_input.as_mut() {
                    match key.code {
                        KeyCode::Esc => view.filter_input = None,
                        KeyCode::Enter => {
                            // Push the typed regex to the aggregator; empty clears the filter.
                            let f = if buf.trim().is_empty() {
                                None
                            } else {
                                Some(buf.clone())
                            };
                            let _ = filter_tx.send(f);
                            view.filter_input = None;
                        }
                        KeyCode::Backspace => {
                            buf.pop();
                        }
                        KeyCode::Char(c) => buf.push(c),
                        _ => {}
                    }
                    continue;
                }

                let action = input::map_with_quit_confirm(key, view.quit_confirm);
                match action {
                    Action::Quit => {
                        if view.quit_confirm {
                            return Ok(());
                        }
                        view.quit_confirm = true;
                    }
                    Action::ConfirmQuit => return Ok(()),
                    Action::CancelQuit => view.quit_confirm = false,
                    Action::SelectUp => {
                        view.selected = view.selected.saturating_sub(1);
                        view.quit_confirm = false;
                    }
                    Action::SelectDown => {
                        // Clamp against the current snapshot so the index never
                        // walks past the last row (otherwise SelectUp would need
                        // to undo every over-press).
                        let max_idx = snapshot
                            .load()
                            .threads
                            .len()
                            .saturating_sub(1);
                        view.selected = view.selected.saturating_add(1).min(max_idx);
                        view.quit_confirm = false;
                    }
                    Action::ToggleDetail => {
                        view.detail_open = !view.detail_open;
                        view.quit_confirm = false;
                    }
                    Action::TogglePause => {
                        let cur = *paused_tx.borrow();
                        let _ = paused_tx.send(!cur);
                        view.quit_confirm = false;
                    }
                    Action::SpeedUp => {
                        let cur = *interval_tx.borrow();
                        let new = scale_interval(cur, true);
                        let _ = interval_tx.send(new);
                        view.quit_confirm = false;
                    }
                    Action::SlowDown => {
                        let cur = *interval_tx.borrow();
                        let new = scale_interval(cur, false);
                        let _ = interval_tx.send(new);
                        view.quit_confirm = false;
                    }
                    Action::CycleWindow => {
                        view.window.0 = view.window.0.cycle();
                        view.quit_confirm = false;
                    }
                    Action::CycleWindowBack => {
                        view.window.0 = view.window.0.cycle_back();
                        view.quit_confirm = false;
                    }
                    Action::ToggleLogScale => {
                        view.log_scale = !view.log_scale;
                        view.quit_confirm = false;
                    }
                    Action::Export => {
                        let snap = snapshot.load_full();
                        match crate::export::write_snapshot(&snap, &PathBuf::from(".")) {
                            Ok(path) => {
                                status_text = Some(format!("wrote {}", path.display()));
                                status_until = Some(Instant::now() + Duration::from_secs(3));
                            }
                            Err(e) => {
                                status_text = Some(format!("export failed: {e}"));
                                status_until = Some(Instant::now() + Duration::from_secs(3));
                            }
                        }
                        view.quit_confirm = false;
                    }
                    Action::Filter => {
                        // Pre-fill with the active filter so it can be edited in place.
                        view.filter_input =
                            Some(snapshot.load().filter.clone().unwrap_or_default());
                        view.quit_confirm = false;
                    }
                    Action::CycleView => {
                        view.detail_open = !view.detail_open;
                        view.quit_confirm = false;
                    }
                    Action::CycleListAvg => {
                        view.list_avg = cycle_list_avg(view.list_avg);
                        view.quit_confirm = false;
                    }
                    Action::ToggleHelp => {
                        view.help_open = !view.help_open;
                        view.help_scroll = 0;
                        view.quit_confirm = false;
                    }
                    Action::None => {}
                }

                // Ctrl+C always exits.
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
                {
                    return Ok(());
                }
            }
        }

        // Clear status when expired.
        if let Some(until) = status_until {
            if Instant::now() >= until {
                status_text = None;
                status_until = None;
            }
        }

        let snap = snapshot.load_full();
        // Threads can disappear between snapshots; clamp selection here too so the
        // cursor stays on the last visible row instead of getting parked off-screen.
        let max_idx = snap.threads.len().saturating_sub(1);
        if view.selected > max_idx {
            view.selected = max_idx;
        }
        let view_ref = &view;
        let status_ref = status_text.as_deref();
        terminal.draw(|f| draw(f, &snap, view_ref, status_ref))?;

        if snap.target_gone {
            // Keep rendering the frozen snapshot but allow Ctrl+C / q to exit normally.
        }
    }
}

fn draw(
    f: &mut ratatui::Frame,
    snap: &Arc<crate::state::Snapshot>,
    view: &ViewState,
    status: Option<&str>,
) {
    let area = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(area);

    draw_header(f, chunks[0], snap, status);

    let body = chunks[1];
    if view.detail_open && !snap.threads.is_empty() {
        let split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(body);
        list::render(f, split[0], snap, view);
        detail::render(f, split[1], snap, view);
    } else {
        list::render(f, body, snap, view);
    }

    widgets::footer::render(f, chunks[2], snap, view);

    // Filter prompt overlay.
    if let Some(buf) = view.filter_input.as_ref() {
        let line = format!("filter (regex): {buf}_");
        let p = Paragraph::new(line)
            .style(Style::default().fg(Color::Black).bg(Color::Cyan))
            .block(Block::default().borders(Borders::ALL).title("filter"));
        let r = Rect {
            x: area.x + 2,
            y: area.y + area.height.saturating_sub(5),
            width: area.width.saturating_sub(4),
            height: 3,
        };
        // Reset the cells underneath so list text doesn't bleed through the panel.
        f.render_widget(Clear, r);
        f.render_widget(p, r);
    }

    // Help overlay (drawn last so it sits on top of everything).
    if view.help_open {
        widgets::help::render(f, area, view.help_scroll);
    }
}

fn draw_header(
    f: &mut ratatui::Frame,
    area: Rect,
    snap: &Arc<crate::state::Snapshot>,
    status: Option<&str>,
) {
    let p = &snap.process;
    let mut spans: Vec<Span> = Vec::new();
    spans.push(Span::styled(
        " procscope ",
        Style::default()
            .fg(Color::White)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        format!(" pid {} ", p.pid),
        Style::default().fg(Color::Black).bg(Color::Gray),
    ));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(
        format!("\"{}\"", short(&p.name, 28)),
        Style::default().fg(Color::Cyan),
    ));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("threads {}", p.num_threads),
        Style::default().fg(Color::Cyan),
    ));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("CPU {:.1}%", p.cpu_pct_host),
        Style::default().fg(Color::Yellow),
    ));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("RSS {}", widgets::stats::fmt_bytes(p.rss_bytes)),
        Style::default().fg(Color::Cyan),
    ));
    spans.push(Span::raw("  "));
    spans.push(Span::styled(
        format!("FDs {} (sock {})", p.fd_count, p.socket_count),
        Style::default().fg(Color::Cyan),
    ));
    spans.push(Span::raw("  "));
    let int_ms = snap.interval.as_millis();
    spans.push(Span::styled(
        format!("int {}ms", int_ms),
        Style::default().fg(Color::Cyan),
    ));
    if snap.paused {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            " PAUSED ",
            Style::default().fg(Color::Black).bg(Color::Yellow),
        ));
    }
    if !snap.caps.syscall_readable {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "[syscall n/a]",
            Style::default().fg(Color::DarkGray),
        ));
    }
    if !snap.caps.per_thread_io_readable {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            "[io n/a]",
            Style::default().fg(Color::DarkGray),
        ));
    }
    if snap.target_gone {
        spans.push(Span::raw("  "));
        spans.push(Span::styled(
            format!(" target {} exited ", p.pid),
            Style::default().fg(Color::White).bg(Color::Red),
        ));
    }

    let mut line2: Vec<Span> = Vec::new();
    if let Some(f) = snap.filter.as_deref() {
        line2.push(Span::styled(
            format!(" filter: {f} "),
            Style::default().fg(Color::Black).bg(Color::Cyan),
        ));
        line2.push(Span::raw(" "));
    }
    if let Some(s) = status {
        line2.push(Span::styled(
            format!(" {s} "),
            Style::default().fg(Color::Black).bg(Color::Green),
        ));
    } else {
        line2.push(Span::styled(
            format!("cmd: {}", short(&p.cmdline, 200)),
            Style::default().fg(Color::DarkGray),
        ));
    }

    let block = Block::default().borders(Borders::BOTTOM);
    let inner = block.inner(area);
    f.render_widget(block, area);
    let lines = vec![Line::from(spans), Line::from(line2)];
    f.render_widget(Paragraph::new(lines), inner);
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

fn scale_interval(cur: Duration, faster: bool) -> Duration {
    const MIN_MS: u64 = 1;
    const MAX_MS: u64 = 30_000;
    let ms = cur.as_millis() as u64;
    let new = if faster {
        (ms / 2).max(MIN_MS)
    } else {
        ms.saturating_mul(2)
    };
    Duration::from_millis(new.clamp(MIN_MS, MAX_MS))
}

pub async fn run_no_tui(snapshot: SharedSnapshot, duration: Option<Duration>) -> Result<()> {
    let start = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let snap = snapshot.load_full();
        let p = &snap.process;
        println!(
            "[{:>4.1}s] pid {} \"{}\" threads={} cpu={:.1}% rss={} fds={} sock={}",
            start.elapsed().as_secs_f32(),
            p.pid,
            p.name,
            p.num_threads,
            p.cpu_pct_host,
            widgets::stats::fmt_bytes(p.rss_bytes),
            p.fd_count,
            p.socket_count,
        );
        for t in snap.threads.iter().take(20) {
            let freeze_mark = match &t.freeze {
                Some(f) => format!(
                    " ★ {} ({}ms,{})",
                    f.wchan,
                    f.since.elapsed().as_millis(),
                    f.reason.label()
                ),
                None => String::new(),
            };
            println!(
                "  tid={:>6} {:<20} {} cpu={:>5.1}% sys={:>4.1}% ctxsw={:>5.0}/{:>5.0} wchan={:<20} sc={}{}",
                t.tid,
                short(&t.name, 20),
                t.state.label(),
                t.cpu_pct,
                t.sys_pct,
                t.ctxsw_vol_per_s,
                t.ctxsw_invol_per_s,
                short(&t.wchan, 20),
                t.syscall_name.unwrap_or("-"),
                freeze_mark,
            );
        }

        if snap.target_gone {
            println!("target exited");
            return Ok(());
        }
        if let Some(d) = duration {
            if start.elapsed() >= d {
                return Ok(());
            }
        }
    }
}
