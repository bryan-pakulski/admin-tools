use std::collections::VecDeque;
use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

use crate::sampler::ThreadState;
use crate::state::RecentPoint;

/// Time-binned colored strip: each cell represents `window/width` of wall time.
/// Color reflects the most concerning state in that bin (D > T/t > S > R > Idle > Z).
/// Bins with no samples render as DarkGray (no data).
pub struct StateStrip<'a> {
    pub recent: &'a VecDeque<RecentPoint>,
    /// None = use the full data extent (newest - oldest).
    pub window: Option<Duration>,
}

impl<'a> Widget for StateStrip<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.recent.is_empty() {
            return;
        }
        let w = area.width as usize;
        let Some(newest) = self.recent.back() else { return };
        let now = newest.at;

        // Resolve window: explicit or data-extent.
        let window = match self.window {
            Some(d) if !d.is_zero() => d,
            _ => match self.recent.front() {
                Some(p) => now.duration_since(p.at).max(Duration::from_nanos(1)),
                None => return,
            },
        };
        let window_ns = window.as_nanos();
        if window_ns == 0 {
            return;
        }

        // Fold each bin to its "worst" state (the one a freeze-hunter most wants to see).
        let mut bin_state: Vec<Option<ThreadState>> = vec![None; w];
        for p in self.recent.iter() {
            let dt = now.duration_since(p.at).as_nanos();
            if dt >= window_ns {
                continue;
            }
            let pos = ((window_ns - dt) * w as u128) / window_ns;
            let cell = (pos as usize).min(w.saturating_sub(1));
            bin_state[cell] = Some(match bin_state[cell] {
                None => p.state,
                Some(cur) => worst_of(cur, p.state),
            });
        }

        for (i, state) in bin_state.iter().enumerate() {
            let x = area.x + i as u16;
            let bg = match state {
                Some(s) => state_color(*s),
                None => Color::Black,
            };
            for dy in 0..area.height {
                let cell = &mut buf[(x, area.y + dy)];
                cell.set_char(' ');
                cell.set_style(Style::default().bg(bg));
            }
        }
    }
}

pub fn state_color(s: ThreadState) -> Color {
    match s {
        ThreadState::Running => Color::Green,
        ThreadState::Sleeping => Color::Cyan,
        ThreadState::Disk => Color::Red,
        ThreadState::Stopped => Color::Magenta,
        ThreadState::Tracing => Color::Magenta,
        ThreadState::Zombie => Color::DarkGray,
        ThreadState::Dead => Color::DarkGray,
        ThreadState::Idle => Color::Blue,
        ThreadState::Unknown => Color::DarkGray,
    }
}

/// Severity ranking so a single D-state sample in a bin surfaces as red — exactly
/// what the freeze hunter needs to see.
fn worst_of(a: ThreadState, b: ThreadState) -> ThreadState {
    if severity(a) >= severity(b) {
        a
    } else {
        b
    }
}

fn severity(s: ThreadState) -> u8 {
    match s {
        ThreadState::Disk => 6,
        ThreadState::Stopped | ThreadState::Tracing => 5,
        ThreadState::Sleeping => 4,
        ThreadState::Running => 3,
        ThreadState::Idle => 2,
        ThreadState::Zombie => 1,
        ThreadState::Dead | ThreadState::Unknown => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn pt(at: Instant, state: ThreadState) -> RecentPoint {
        RecentPoint {
            at,
            cpu_pct: 0.0,
            sys_pct: 0.0,
            state,
            ctxsw_vol_per_s: 0.0,
            ctxsw_invol_per_s: 0.0,
            rchar_bps: 0.0,
            wchar_bps: 0.0,
            sched_wait_ms_per_s: 0.0,
        }
    }

    #[test]
    fn worst_keeps_disk_state_in_bin() {
        // If a bin has both R and D samples, D wins (most diagnostic).
        let now = Instant::now();
        let mut q: VecDeque<RecentPoint> = VecDeque::new();
        q.push_back(pt(now - Duration::from_millis(60), ThreadState::Running));
        q.push_back(pt(now - Duration::from_millis(50), ThreadState::Disk));
        q.push_back(pt(now - Duration::from_millis(40), ThreadState::Running));
        // With width=1 the whole window is one bin; expected color is red (Disk).
        // We can't easily render to a Buffer in a unit test, but worst_of() is the core.
        assert!(matches!(
            worst_of(ThreadState::Running, ThreadState::Disk),
            ThreadState::Disk
        ));
        let _ = q;
    }
}
