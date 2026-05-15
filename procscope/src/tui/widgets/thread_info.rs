use std::time::SystemTime;

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::sampler::ThreadState;
use crate::state::ThreadView;
use crate::tui::widgets::stats::fmt_duration_ms;

/// Decode the syscall + its args into a readable string, given the syscall name.
/// We focus on socket-related syscalls — these are the user's freeze-hunting targets.
pub fn decode_syscall(name: &str, args: [u64; 6]) -> String {
    match name {
        // recvfrom / recvmsg / sendto / sendmsg: (fd, buf, len/msg, flags, src_addr, addrlen)
        "recvfrom" | "sendto" => format!(
            "{name}(fd={}, buf=0x{:x}, len={}, flags=0x{:x}, addr=0x{:x})",
            args[0] as i32, args[1], args[2], args[3], args[4]
        ),
        "recvmsg" | "sendmsg" => format!(
            "{name}(fd={}, msg=0x{:x}, flags=0x{:x})",
            args[0] as i32, args[1], args[2]
        ),
        // read / write / pread / pwrite: (fd, buf, count, [offset])
        "read" | "write" => format!(
            "{name}(fd={}, buf=0x{:x}, count={})",
            args[0] as i32, args[1], args[2]
        ),
        "pread64" | "pwrite64" => format!(
            "{name}(fd={}, buf=0x{:x}, count={}, offset={})",
            args[0] as i32, args[1], args[2], args[3]
        ),
        // accept / accept4: (fd, addr, addrlen [, flags])
        "accept" => format!(
            "accept(fd={}, addr=0x{:x}, addrlen=0x{:x})",
            args[0] as i32, args[1], args[2]
        ),
        "accept4" => format!(
            "accept4(fd={}, addr=0x{:x}, addrlen=0x{:x}, flags=0x{:x})",
            args[0] as i32, args[1], args[2], args[3]
        ),
        "connect" => format!(
            "connect(fd={}, addr=0x{:x}, addrlen={})",
            args[0] as i32, args[1], args[2]
        ),
        // poll / ppoll: (fds, nfds, timeout_ms_or_timespec)
        "poll" => format!(
            "poll(fds=0x{:x}, nfds={}, timeout_ms={})",
            args[0], args[1], args[2] as i32
        ),
        "ppoll" => format!(
            "ppoll(fds=0x{:x}, nfds={}, timespec=0x{:x}, sigmask=0x{:x})",
            args[0], args[1], args[2], args[3]
        ),
        // epoll: (epfd, events, maxevents, timeout)
        "epoll_wait" | "epoll_pwait" | "epoll_pwait2" => format!(
            "{name}(epfd={}, events=0x{:x}, maxevents={}, timeout_ms={})",
            args[0] as i32, args[1], args[2], args[3] as i32
        ),
        // futex: (uaddr, op, val, timespec, ...)
        "futex" => format!(
            "futex(uaddr=0x{:x}, op={}, val={}, timeout=0x{:x})",
            args[0], args[1], args[2], args[3]
        ),
        // nanosleep / clock_nanosleep: (clock_id?, flags?, req, rem)
        "nanosleep" => format!("nanosleep(req=0x{:x}, rem=0x{:x})", args[0], args[1]),
        "clock_nanosleep" => format!(
            "clock_nanosleep(clk={}, flags={}, req=0x{:x}, rem=0x{:x})",
            args[0], args[1], args[2], args[3]
        ),
        // close: (fd)
        "close" => format!("close(fd={})", args[0] as i32),
        // generic fallback
        _ => format!(
            "{name}(0x{:x}, 0x{:x}, 0x{:x}, 0x{:x}, 0x{:x}, 0x{:x})",
            args[0], args[1], args[2], args[3], args[4], args[5]
        ),
    }
}

/// Render a 3-line info panel: identity / syscall / freeze-status.
pub fn render(f: &mut Frame, area: Rect, t: &ThreadView, now_wall: SystemTime, clk_tck: u64) {
    let block = Block::default().borders(Borders::ALL).title(" drill-down ");
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Compute thread uptime from starttime_ticks (ticks since boot).
    let uptime_str = uptime_string(t.starttime_ticks, clk_tck);

    // Line 1: identity.
    let id_line = Line::from(vec![
        Span::styled("tid ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            t.tid.to_string(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("name ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            t.name.clone(),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("cpu# ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            t.processor.to_string(),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("  "),
        Span::styled("state ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            format!(" {} ", t.state.label()),
            Style::default()
                .fg(Color::Black)
                .bg(crate::tui::widgets::state_strip::state_color(t.state)),
        ),
        Span::raw("  "),
        Span::styled("up ", Style::default().fg(Color::DarkGray)),
        Span::styled(uptime_str, Style::default().fg(Color::Cyan)),
        Span::raw("  "),
        Span::styled("wchan ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            if t.wchan.is_empty() {
                "—".to_string()
            } else {
                t.wchan.clone()
            },
            Style::default().fg(Color::Cyan),
        ),
    ]);

    // Line 2: syscall args (or "running" if no syscall in-flight).
    let syscall_line = match t.syscall_name {
        Some(name) => Line::from(vec![
            Span::styled("syscall ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                decode_syscall(name, t.syscall_args),
                Style::default().fg(Color::Yellow),
            ),
        ]),
        None => Line::from(vec![
            Span::styled("syscall ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                match t.state {
                    ThreadState::Running => "running (no syscall in-flight)",
                    _ => "— (not currently in a syscall)",
                },
                Style::default().fg(Color::Green),
            ),
        ]),
    };

    // Line 3: freeze status or healthy.
    let status_line = match &t.freeze {
        Some(flag) => Line::from(vec![
            Span::styled(
                " ★ FROZEN ",
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  reason "),
            Span::styled(
                flag.reason.label(),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  stuck "),
            Span::styled(
                fmt_duration_ms(flag.since.elapsed().as_millis() as u64),
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
            Span::raw("  in "),
            Span::styled(flag.wchan.clone(), Style::default().fg(Color::Yellow)),
        ]),
        None => Line::from(vec![
            Span::styled("status ", Style::default().fg(Color::DarkGray)),
            Span::styled("healthy", Style::default().fg(Color::Green)),
        ]),
    };

    let _ = now_wall; // reserved for future use (e.g. drift from now to last transition)
    let lines = vec![id_line, syscall_line, status_line];
    f.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recvfrom_args_decoded() {
        let s = decode_syscall("recvfrom", [14, 0x7fffaa00, 64, 0, 0x7fffbb00, 16]);
        assert!(s.starts_with("recvfrom(fd=14"), "{s}");
        assert!(s.contains("len=64"), "{s}");
    }

    #[test]
    fn epoll_args_decoded() {
        let s = decode_syscall("epoll_wait", [5, 0x7fffaa00, 32, u64::MAX - 1, 0, 0]);
        assert!(s.contains("epfd=5"), "{s}");
        assert!(s.contains("maxevents=32"), "{s}");
    }

    #[test]
    fn unknown_syscall_falls_back_to_hex() {
        let s = decode_syscall("syscall_999", [1, 2, 3, 4, 5, 6]);
        assert!(s.contains("0x1"), "{s}");
        assert!(s.contains("0x6"), "{s}");
    }
}

fn uptime_string(starttime_ticks: u64, clk_tck: u64) -> String {
    // Boot time in seconds since epoch from /proc/uptime — first field is uptime, but we
    // need wall_now - (boot_time + starttime_ticks/clk_tck) = uptime_now - starttime_ticks/clk_tck.
    let proc_uptime_secs: f64 = std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| {
            s.split_whitespace()
                .next()
                .and_then(|f| f.parse::<f64>().ok())
        })
        .unwrap_or(0.0);
    let thread_secs = starttime_ticks as f64 / clk_tck.max(1) as f64;
    let elapsed = (proc_uptime_secs - thread_secs).max(0.0);
    if elapsed >= 3600.0 {
        format!("{}h{:02}m", (elapsed as u64) / 3600, ((elapsed as u64) / 60) % 60)
    } else if elapsed >= 60.0 {
        format!("{}m{:02}s", (elapsed as u64) / 60, (elapsed as u64) % 60)
    } else {
        format!("{:.1}s", elapsed)
    }
}
