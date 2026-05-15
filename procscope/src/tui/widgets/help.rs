use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

pub fn render(f: &mut Frame, area: Rect, scroll: u16) {
    // Center the help panel within the area.
    let panel = centered(area, 90, 90);
    f.render_widget(Clear, panel);

    let lines = content();
    let total = lines.len() as u16;
    let inner_h = panel.height.saturating_sub(2);
    let max_scroll = total.saturating_sub(inner_h.max(1));
    let scroll = scroll.min(max_scroll);

    let title = format!(
        " procscope — help  ({}/{})  ↑↓/jk scroll · PgUp/PgDn page · Esc/h/q close ",
        scroll + 1,
        max_scroll + 1
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .title_alignment(Alignment::Left)
        .style(Style::default().bg(Color::Black));

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));

    f.render_widget(para, panel);
}

fn centered(area: Rect, pct_w: u16, pct_h: u16) -> Rect {
    let w = area.width * pct_w / 100;
    let h = area.height * pct_h / 100;
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

fn section(label: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("─ {} ─", label),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn entry(key: &'static str, desc: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("{:<14}", key),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(desc, Style::default().fg(Color::Gray)),
    ])
}

fn body(text: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        format!("    {}", text),
        Style::default().fg(Color::Gray),
    ))
}

fn note(text: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  • {}", text),
        Style::default().fg(Color::Gray),
    ))
}

fn blank() -> Line<'static> {
    Line::from("")
}

fn content() -> Vec<Line<'static>> {
    let mut v: Vec<Line<'static>> = Vec::new();

    v.push(Line::from(Span::styled(
        "procscope — non-invasive per-thread process monitor",
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
    )));
    v.push(body(
        "Polls /proc only. No ptrace, no perf, no eBPF — the target process is not aware",
    ));
    v.push(body(
        "of you and pays zero overhead.",
    ));
    v.push(blank());

    // -- Keys --
    v.push(section("KEYBINDINGS"));
    v.push(entry("↑ ↓  or  j k", "select previous / next thread"));
    v.push(entry("Enter", "open / close the detail pane on the selected thread"));
    v.push(entry("Space", "pause / resume sampling (data is frozen, not lost)"));
    v.push(entry("+ / -", "halve / double sampling interval (range: 1ms ↔ 30s)"));
    v.push(entry("w", "cycle window forward: 15s → 30s → 1m → 2m → 5m → 10m → 30m → all"));
    v.push(entry("W (shift+w)", "cycle window backward"));
    v.push(entry("l", "toggle log scale on detail charts"));
    v.push(entry("a", "cycle list moving-average: off → 0.5s → 1s → 3s → 10s → 30s → 1m → off"));
    v.push(entry("f", "filter threads by regex on name (Esc to cancel, Enter to apply)"));
    v.push(entry("v", "alias for Enter (cycle view)"));
    v.push(entry("e", "export snapshot CSV to current directory"));
    v.push(entry("h  or  ?  or  F1", "open / close this help"));
    v.push(entry("q  or  Esc", "quit (press y / Enter to confirm)"));
    v.push(entry("Ctrl+C / Ctrl+D", "quit immediately"));
    v.push(blank());

    // -- Thread state codes --
    v.push(section("THREAD STATE CODES (st column)"));
    v.push(entry("R (green)", "Running — actively executing on a CPU"));
    v.push(entry("S (cyan)", "Sleeping — waiting on an event (recv, sleep, mutex, ...)"));
    v.push(entry("D (red)", "Disk wait — uninterruptible I/O. Often a sign of stalled I/O."));
    v.push(entry("T (magenta)", "Stopped (SIGSTOP / debugger)"));
    v.push(entry("Z (gray)", "Zombie — exited but not yet reaped"));
    v.push(entry("I (blue)", "Idle kernel thread (rare for user processes)"));
    v.push(blank());

    // -- Column abbreviations --
    v.push(section("COLUMN ABBREVIATIONS"));
    v.push(entry("tid", "thread ID (kernel PID for this LWP)"));
    v.push(entry("name", "thread name from /proc/<pid>/task/<tid>/comm"));
    v.push(entry("st", "state code (see above)"));
    v.push(entry("cpu%", "CPU% (user+sys). 100% = one full core. Averaged over the list avg"));
    v.push(body(
        "window (default 1s) so high-rate polling doesn't make the value jitter.",
    ));
    v.push(body(
        "Press 'a' to cycle the average window; the header shows '(avg Xs)'.",
    ));
    v.push(entry("sys%", "Kernel-mode CPU% (subset of cpu%). Same averaging applies."));
    v.push(entry("ctxsw v/iv", "Context switches per second (same averaging). v = voluntary"));
    v.push(body(
        "(waiting on I/O, lock, sleep). iv = involuntary (preempted by scheduler).",
    ));
    v.push(body(
        "High iv often means CPU contention.",
    ));
    v.push(entry("io%", "Fraction of recent samples observed in D-state (proxy for I/O wait)"));
    v.push(entry("wchan / syscall", "current kernel wait function and active syscall."));
    v.push(body(
        "★ marker means the thread has been flagged as frozen.",
    ));
    v.push(entry("timeline", "Inline sparkline of cpu% over the active window."));
    v.push(body(
        "× = D-state or freeze-flagged sample in that bin.",
    ));
    v.push(blank());

    // -- Freeze reasons --
    v.push(section("FREEZE FLAGS (★ marker, sorted to top of list)"));
    v.push(entry("net wait", "Thread is sleeping on a socket recv / accept / select / poll"));
    v.push(body(
        "wchan, for longer than --freeze-netwchan-ms (default 5000ms).",
    ));
    v.push(body(
        "THIS IS THE PRIMARY SIGNAL FOR send/recv FREEZES.",
    ));
    v.push(entry("D-state", "Thread stuck in uninterruptible disk wait > --freeze-d-ms (500ms)."));
    v.push(entry("no ctxsw", "CPU is being consumed but ctxsw counters not incrementing"));
    v.push(body(
        "for > --freeze-noctxsw-ms (5000ms). Often a CPU-spinning bug.",
    ));
    v.push(entry("diverged", "Peers are making progress but this thread is idle for"));
    v.push(body(
        "> --freeze-divergence-ms (3000ms). Benign sleepers are excluded.",
    ));
    v.push(blank());

    // -- Common wchan symbols --
    v.push(section("COMMON wchan / SYSCALL PAIRS"));
    v.push(entry("sk_wait_data + recvfrom", "blocking socket recv — most common net freeze"));
    v.push(entry("tcp_recvmsg + recvmsg", "blocking TCP recv with msghdr"));
    v.push(entry("inet_csk_accept + accept4", "waiting for a new TCP connection"));
    v.push(entry("wait_woken + recvfrom", "modern-kernel form of sk_wait_data"));
    v.push(entry("ep_poll + epoll_wait", "waiting on an epoll set"));
    v.push(entry("hrtimer_nanosleep", "sleep() — benign, excluded from freeze detection"));
    v.push(entry("futex_wait_queue + futex", "mutex / condvar / Tokio task wait"));
    v.push(entry("schedule_timeout", "generic scheduler timeout (any wait with a deadline)"));
    v.push(entry("pipe_read", "blocked on pipe read (stdin, IPC)"));
    v.push(blank());

    // -- Detail pane --
    v.push(section("DETAIL PANE (press Enter on a thread)"));
    v.push(body("Top:    two stacked line charts — CPU% (user|sys) and ctxsw/s (vol|invol)."));
    v.push(body("        Both charts share the X axis. X is wall-time anchored: rightmost"));
    v.push(body("        edge is 'now', leftmost is now - window."));
    v.push(body(""));
    v.push(body("Strip:  colored band, one cell per time slot. The cell is colored by the"));
    v.push(body("        worst state seen in its slot — so a single D-state sample in a bin"));
    v.push(body("        of 1000 R-state samples still shows as red."));
    v.push(body(""));
    v.push(body("Stats:  min / avg / p50 / p95 / p99 / max for every numeric field, over"));
    v.push(body("        the active window. Recomputed each frame."));
    v.push(body(""));
    v.push(body("Trans:  state / wchan / syscall transition log, newest first. The duration"));
    v.push(body("        column shows how long the thread sat in that state."));
    v.push(body(""));
    v.push(body("Drill:  syscall arguments decoded — for socket calls you see fd=N, the file"));
    v.push(body("        descriptor of the stuck socket. lsof -p <pid> | grep N maps to the"));
    v.push(body("        peer endpoint."));
    v.push(blank());

    // -- How to diagnose --
    v.push(section("HOW TO DIAGNOSE..."));
    v.push(blank());

    v.push(Line::from(Span::styled(
        "  Intermittent send/recv freeze",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    v.push(note("Run at default 100ms interval. Leave attached. Wait for the freeze."));
    v.push(note("When a thread sticks in net wait > 5s, it gets ★ and sorts to top."));
    v.push(note("Press Enter on it: the transitions table shows when the stick started,"));
    v.push(note("the drill-down shows the fd. Press e to save a snapshot CSV."));
    v.push(note("For sub-5s freezes, lower --freeze-netwchan-ms=1000 or 500."));
    v.push(note("For sub-100ms spike capture, press + repeatedly to drop to 1ms polling."));
    v.push(blank());

    v.push(Line::from(Span::styled(
        "  Thread pegged at 100% CPU",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    v.push(note("R-state thread at ~100% cpu, low ctxsw/s. The CPU chart spikes solid."));
    v.push(note("If ctxsw stops growing while CPU climbs → no ctxsw freeze (spin loop)."));
    v.push(note("Look at the transitions table — it should be EMPTY for a hot loop."));
    v.push(blank());

    v.push(Line::from(Span::styled(
        "  Storage / disk stall",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    v.push(note("D-state samples → red cells on the state strip, × in the sparkline."));
    v.push(note("io% column in the list reflects the share of D samples over the window."));
    v.push(note("Pair with iostat -x 1 in another shell to confirm device backpressure."));
    v.push(blank());

    v.push(Line::from(Span::styled(
        "  Mutex contention",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    v.push(note("futex_wait_queue wchan, futex syscall, high voluntary ctxsw/s."));
    v.push(note("Multiple threads sharing one futex_wait_queue → lock contention."));
    v.push(note("The futex's uaddr (arg0 in the drill-down) identifies the lock variable."));
    v.push(blank());

    v.push(Line::from(Span::styled(
        "  GC / pause stalls",
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )));
    v.push(note("All-threads simultaneous transition to S/futex_wait_queue then back to R."));
    v.push(note("Watch the state strip: synchronized cyan bands across many threads."));
    v.push(note("CPU dips visible on the process header during the pause."));
    v.push(blank());

    // -- Tips --
    v.push(section("TIPS"));
    v.push(note("Press + to increase polling resolution without changing the window."));
    v.push(note("Faster polling = more samples per bin = brief spikes survive aggregation."));
    v.push(note("Press w to extend window for long-term patterns (memory leaks, ramps)."));
    v.push(note("--write file.csv streams every sample to disk for offline analysis."));
    v.push(note("--export-on-exit dumps a snapshot CSV when the tool exits."));
    v.push(note("Run as same UID as target for full /proc/.../{syscall,io} access."));
    v.push(note("Run as root or with CAP_SYS_PTRACE for cross-UID and /proc/.../stack."));
    v.push(blank());

    // -- About --
    v.push(section("DATA SOURCES"));
    v.push(note("/proc/<pid>/stat              — process state, cpu ticks, num_threads"));
    v.push(note("/proc/<pid>/status            — VmRSS, ctxsw counters"));
    v.push(note("/proc/<pid>/io                — bytes read/written"));
    v.push(note("/proc/<pid>/fd                — open file descriptors (socket count)"));
    v.push(note("/proc/<pid>/task/<tid>/stat   — per-thread cpu ticks, state, last CPU"));
    v.push(note("/proc/<pid>/task/<tid>/wchan  — kernel wait function symbol"));
    v.push(note("/proc/<pid>/task/<tid>/syscall — active syscall number + 6 args"));
    v.push(note("/proc/<pid>/task/<tid>/schedstat — scheduler wait time"));
    v.push(blank());

    v.push(Line::from(Span::styled(
        "  End of help — press Esc, h, q, or ? to close",
        Style::default().fg(Color::DarkGray),
    )));

    v
}
