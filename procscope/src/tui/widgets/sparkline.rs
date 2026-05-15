use std::collections::VecDeque;
use std::time::Duration;

use crate::sampler::ThreadState;
use crate::state::{RecentPoint, ThreadView};

const BLOCKS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Time-binned inline sparkline. Each cell represents `window/width` of wall time.
/// We take the **max** CPU% in the bin so brief spikes don't get averaged away,
/// and render `×` when any sample in the bin is in D-state or freeze-flagged.
/// At higher polling rates more samples land in each bin → higher fidelity, but
/// the time range covered stays exactly `window`.
pub fn inline(view: &ThreadView, width: usize, window: Option<Duration>) -> String {
    let bins = bin_recent(&view.recent, width, window);
    if bins.is_empty() {
        return String::new();
    }
    let max_cpu = bins.iter().map(|b| b.max_cpu).fold(0.0f32, f32::max).max(1.0);
    let frozen = view.freeze.is_some();

    let mut out = String::with_capacity(width);
    for b in bins {
        if b.has_disk || (frozen && b.max_cpu == 0.0 && b.has_data) {
            out.push('×');
        } else if !b.has_data {
            out.push(' ');
        } else if b.max_cpu == 0.0 {
            out.push('▁');
        } else {
            let h = (b.max_cpu / max_cpu).clamp(0.0, 1.0);
            let idx = ((h * (BLOCKS.len() - 1) as f32).round() as usize).min(BLOCKS.len() - 1);
            out.push(BLOCKS[idx]);
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
struct Bin {
    max_cpu: f32,
    has_disk: bool,
    has_data: bool,
}

/// Bin `recent` into `width` cells covering wall time `[newest - window, newest]`.
/// Empty bins are surfaced as `has_data == false` so the caller can render them as gaps.
fn bin_recent(
    recent: &VecDeque<RecentPoint>,
    width: usize,
    window: Option<Duration>,
) -> Vec<Bin> {
    let mut out = vec![
        Bin {
            max_cpu: 0.0,
            has_disk: false,
            has_data: false,
        };
        width
    ];
    if width == 0 || recent.is_empty() {
        return Vec::new();
    }
    let newest = recent.back().unwrap().at;
    let window = match window {
        Some(d) if !d.is_zero() => d,
        _ => match recent.front() {
            Some(p) => newest.duration_since(p.at).max(Duration::from_nanos(1)),
            None => return Vec::new(),
        },
    };
    let window_ns = window.as_nanos();
    if window_ns == 0 {
        return Vec::new();
    }
    for p in recent.iter() {
        let dt = newest.duration_since(p.at).as_nanos();
        if dt >= window_ns {
            continue;
        }
        let pos = ((window_ns - dt) * width as u128) / window_ns;
        let cell = (pos as usize).min(width - 1);
        let slot = &mut out[cell];
        slot.has_data = true;
        if p.cpu_pct > slot.max_cpu {
            slot.max_cpu = p.cpu_pct;
        }
        if matches!(p.state, ThreadState::Disk) {
            slot.has_disk = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn pt(at: Instant, cpu: f32, state: ThreadState) -> RecentPoint {
        RecentPoint {
            at,
            cpu_pct: cpu,
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
    fn one_sample_per_bin_at_low_rate() {
        // 11 samples evenly spaced from now-1000ms to now, asking for 10 bins
        // covering a 1s window. Expectation: every bin populated.
        let now = Instant::now();
        let mut q: VecDeque<RecentPoint> = VecDeque::new();
        for i in 0..=10 {
            let at = now - Duration::from_millis(1000 - i * 100);
            q.push_back(pt(at, (i * 10) as f32, ThreadState::Running));
        }
        let bins = bin_recent(&q, 10, Some(Duration::from_secs(1)));
        assert_eq!(bins.len(), 10);
        let populated = bins.iter().filter(|b| b.has_data).count();
        assert!(
            populated >= 9,
            "expected ~all bins populated, got {populated}: {:?}",
            bins
        );
    }

    #[test]
    fn higher_polling_rate_keeps_same_window_coverage() {
        // 100 samples over 1s mapped into 10 bins — each bin gets 10 samples,
        // but the window covered is STILL 1s.
        let now = Instant::now();
        let mut q: VecDeque<RecentPoint> = VecDeque::new();
        for i in 0..100 {
            let at = now - Duration::from_millis(990 - i as u64 * 10);
            q.push_back(pt(at, (i * 1) as f32, ThreadState::Running));
        }
        let bins = bin_recent(&q, 10, Some(Duration::from_secs(1)));
        assert_eq!(bins.len(), 10);
        assert!(bins.iter().all(|b| b.has_data));
        // The max per bin should reflect the highest sample within that 100ms slice.
        // Last bin (newest) covers samples ~990ms-old up to ~now → values ~90..=99.
        assert!(
            bins.last().unwrap().max_cpu >= 90.0,
            "last bin max should be ~99, got {}",
            bins.last().unwrap().max_cpu
        );
    }

    #[test]
    fn brief_disk_state_surfaces_through_bin() {
        // 10 samples in a 1s window, only one is in D-state. The corresponding bin
        // must report has_disk=true.
        let now = Instant::now();
        let mut q: VecDeque<RecentPoint> = VecDeque::new();
        for i in 0..10 {
            let st = if i == 5 {
                ThreadState::Disk
            } else {
                ThreadState::Running
            };
            let at = now - Duration::from_millis(900 - i as u64 * 100);
            q.push_back(pt(at, 50.0, st));
        }
        let bins = bin_recent(&q, 10, Some(Duration::from_secs(1)));
        assert!(
            bins.iter().any(|b| b.has_disk),
            "at least one bin must surface the D-state sample"
        );
    }

    #[test]
    fn samples_outside_window_are_dropped() {
        // 10 samples spanning 10s but window is 1s — only the last second's worth
        // should land in bins.
        let now = Instant::now();
        let mut q: VecDeque<RecentPoint> = VecDeque::new();
        for i in 0..10 {
            let at = now - Duration::from_secs(9 - i as u64);
            q.push_back(pt(at, (i * 10) as f32, ThreadState::Running));
        }
        let bins = bin_recent(&q, 10, Some(Duration::from_secs(1)));
        let populated = bins.iter().filter(|b| b.has_data).count();
        assert!(
            populated <= 2,
            "expected at most 2 bins populated, got {populated}: {:?}",
            bins
        );
    }
}
