use std::collections::VecDeque;
use std::time::{Instant, SystemTime};

use crate::state::{DisasmInstruction, Frame, GdbSnapshot, MemoryBlock, Register, StopReason, Variable};

/// A captured snapshot of the debug state at a stop event.
#[derive(Debug, Clone)]
pub struct RecordedState {
    pub seq: u64,
    pub timestamp: Instant,
    pub wall_time: SystemTime,
    pub stop_reason: Option<StopReason>,
    pub is_anchor: bool, // true for breakpoint hits — trace navigation stops here
    pub thread_id: Option<i32>,
    pub stack: Vec<Frame>,
    pub frame_level: u32,
    pub locals: Vec<Variable>,
    pub source_path: Option<String>,
    pub source_line: Option<u32>,
    pub registers: Vec<Register>,
    pub disasm: Vec<DisasmInstruction>,
    pub watch_values: Vec<(String, String)>,
    pub memory: Option<MemoryBlock>,
}

/// What changed between two consecutive recorded states.
#[derive(Debug, Clone)]
pub struct StateDiff {
    pub locals_changed: Vec<VarChange>,
    pub locals_added: Vec<String>,
    pub locals_removed: Vec<String>,
    pub registers_changed: Vec<RegChange>,
    pub watches_changed: Vec<WatchChange>,
    pub source_changed: bool,
    pub thread_changed: bool,
    pub memory_changed: Vec<MemoryChange>,
}

#[derive(Debug, Clone)]
pub struct VarChange {
    pub name: String,
    pub old_value: String,
    pub new_value: String,
}

#[derive(Debug, Clone)]
pub struct RegChange {
    pub name: String,
    pub old_value: String,
    pub new_value: String,
}

#[derive(Debug, Clone)]
pub struct WatchChange {
    pub expression: String,
    pub old_value: String,
    pub new_value: String,
}

#[derive(Debug, Clone)]
pub struct MemoryChange {
    pub offset: usize,
    pub old_byte: u8,
    pub new_byte: u8,
}

/// The recording buffer -- a ring buffer of states and diffs.
#[derive(Debug)]
pub struct Recording {
    pub states: VecDeque<RecordedState>,
    /// `diffs[i]` is the diff from `states[i-1]` to `states[i]`; `diffs[0]` is
    /// `None` because the first state has no predecessor.
    pub diffs: VecDeque<Option<StateDiff>>,
    pub max_entries: usize,
    pub max_age_secs: Option<u64>,
    pub enabled: bool,
    next_seq: u64,
}

impl Recording {
    pub fn new(max_entries: usize, max_age_secs: Option<u64>) -> Self {
        Self {
            states: VecDeque::with_capacity(max_entries.min(4096)),
            diffs: VecDeque::with_capacity(max_entries.min(4096)),
            max_entries,
            max_age_secs,
            enabled: true,
            next_seq: 0,
        }
    }

    /// Capture the current GDB snapshot as a new recorded state.
    /// Computes the diff from the previous state automatically.
    /// If `anchor` is true, this state is marked as a breakpoint anchor for
    /// trace navigation.
    pub fn capture(&mut self, snap: &GdbSnapshot) {
        self.capture_with_anchor(snap, false);
    }

    pub fn capture_anchor(&mut self, snap: &GdbSnapshot) {
        self.capture_with_anchor(snap, true);
    }

    fn capture_with_anchor(&mut self, snap: &GdbSnapshot, anchor: bool) {
        if !self.enabled {
            return;
        }

        let is_anchor = anchor || matches!(
            snap.stop_reason,
            Some(StopReason::BreakpointHit { .. }) | Some(StopReason::Watchpoint { .. })
        );

        let state = RecordedState {
            seq: self.next_seq,
            timestamp: Instant::now(),
            wall_time: SystemTime::now(),
            stop_reason: snap.stop_reason.clone(),
            is_anchor,
            thread_id: snap.current_thread_id,
            stack: snap.stack.clone(),
            frame_level: snap.current_frame_level,
            locals: snap.locals.clone(),
            source_path: snap.source.as_ref().map(|s| s.path.clone()).or_else(|| {
                // Fall back to fullname from the current stack frame when
                // no source file is loaded (e.g. during tracing)
                snap.stack.iter()
                    .find(|f| f.level == snap.current_frame_level && f.fullname.is_some())
                    .or_else(|| snap.stack.iter().find(|f| f.fullname.is_some()))
                    .and_then(|f| f.fullname.clone())
            }),
            source_line: snap.source_line.or_else(|| {
                snap.stack.iter()
                    .find(|f| f.level == snap.current_frame_level)
                    .or_else(|| snap.stack.first())
                    .and_then(|f| f.line)
            }),
            registers: snap.registers.clone(),
            disasm: snap.disasm.clone(),
            watch_values: snap
                .watch_expressions
                .iter()
                .map(|w| (w.expression.clone(), w.value.clone()))
                .collect(),
            memory: snap.memory.clone(),
        };
        self.next_seq += 1;

        // Compute diff from previous state
        let diff = self.states.back().map(|prev| compute_diff(prev, &state));

        self.states.push_back(state);
        self.diffs.push_back(diff);

        // Enforce max entries
        while self.states.len() > self.max_entries {
            self.states.pop_front();
            self.diffs.pop_front();
        }

        // Enforce max age
        if let Some(max_secs) = self.max_age_secs {
            let cutoff = Instant::now() - std::time::Duration::from_secs(max_secs);
            while self
                .states
                .front()
                .map_or(false, |s| s.timestamp < cutoff)
            {
                self.states.pop_front();
                self.diffs.pop_front();
            }
        }
    }

    pub fn len(&self) -> usize {
        self.states.len()
    }

    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&RecordedState> {
        self.states.get(index)
    }

    pub fn get_diff(&self, index: usize) -> Option<&StateDiff> {
        self.diffs.get(index).and_then(|d| d.as_ref())
    }

    pub fn clear(&mut self) {
        self.states.clear();
        self.diffs.clear();
    }

    /// Find the previous anchor (breakpoint) index before `from`.
    pub fn prev_anchor(&self, from: usize) -> Option<usize> {
        (0..from).rev().find(|&i| self.states.get(i).map_or(false, |s| s.is_anchor))
    }

    /// Find the next anchor (breakpoint) index after `from`.
    pub fn next_anchor(&self, from: usize) -> Option<usize> {
        ((from + 1)..self.states.len()).find(|&i| self.states.get(i).map_or(false, |s| s.is_anchor))
    }

    /// Count the number of anchor states.
    pub fn anchor_count(&self) -> usize {
        self.states.iter().filter(|s| s.is_anchor).count()
    }
}

/// Compute the diff between two consecutive states.
fn compute_diff(prev: &RecordedState, curr: &RecordedState) -> StateDiff {
    // --- Locals ---
    let mut locals_changed = Vec::new();
    let mut locals_added = Vec::new();
    let mut locals_removed = Vec::new();

    for cv in &curr.locals {
        match prev.locals.iter().find(|pv| pv.name == cv.name) {
            Some(pv) if pv.value != cv.value => {
                locals_changed.push(VarChange {
                    name: cv.name.clone(),
                    old_value: pv.value.clone(),
                    new_value: cv.value.clone(),
                });
            }
            None => {
                locals_added.push(cv.name.clone());
            }
            _ => {}
        }
    }
    for pv in &prev.locals {
        if !curr.locals.iter().any(|cv| cv.name == pv.name) {
            locals_removed.push(pv.name.clone());
        }
    }

    // --- Registers ---
    let mut registers_changed = Vec::new();
    for cr in &curr.registers {
        if let Some(pr) = prev.registers.iter().find(|r| r.name == cr.name) {
            if pr.value != cr.value {
                registers_changed.push(RegChange {
                    name: cr.name.clone(),
                    old_value: pr.value.clone(),
                    new_value: cr.value.clone(),
                });
            }
        }
    }

    // --- Watches ---
    let mut watches_changed = Vec::new();
    for (expr, val) in &curr.watch_values {
        if let Some((_, old_val)) = prev.watch_values.iter().find(|(e, _)| e == expr) {
            if old_val != val {
                watches_changed.push(WatchChange {
                    expression: expr.clone(),
                    old_value: old_val.clone(),
                    new_value: val.clone(),
                });
            }
        }
    }

    // --- Source ---
    let source_changed =
        prev.source_path != curr.source_path || prev.source_line != curr.source_line;

    // --- Thread ---
    let thread_changed = prev.thread_id != curr.thread_id;

    // --- Memory ---
    let mut memory_changed = Vec::new();
    if let (Some(pm), Some(cm)) = (&prev.memory, &curr.memory) {
        if pm.address == cm.address {
            let len = pm.bytes.len().min(cm.bytes.len());
            for i in 0..len {
                if pm.bytes[i] != cm.bytes[i] {
                    memory_changed.push(MemoryChange {
                        offset: i,
                        old_byte: pm.bytes[i],
                        new_byte: cm.bytes[i],
                    });
                }
            }
        }
    }

    StateDiff {
        locals_changed,
        locals_added,
        locals_removed,
        registers_changed,
        watches_changed,
        source_changed,
        thread_changed,
        memory_changed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{
        GdbSnapshot, MemoryBlock, Register, Variable, WatchExpression,
    };

    /// Helper: build a minimal snapshot with given locals and registers.
    fn make_snapshot(
        locals: Vec<(&str, &str)>,
        registers: Vec<(&str, &str)>,
        watches: Vec<(&str, &str)>,
        source_path: Option<&str>,
        source_line: Option<u32>,
        thread_id: Option<i32>,
        memory: Option<MemoryBlock>,
    ) -> GdbSnapshot {
        let mut snap = GdbSnapshot::empty();
        snap.locals = locals
            .into_iter()
            .map(|(n, v)| Variable {
                name: n.to_string(),
                value: v.to_string(),
                type_name: "int".to_string(),
            })
            .collect();
        snap.registers = registers
            .into_iter()
            .map(|(n, v)| Register {
                number: 0,
                name: n.to_string(),
                value: v.to_string(),
            })
            .collect();
        snap.watch_expressions = watches
            .into_iter()
            .enumerate()
            .map(|(i, (e, v))| WatchExpression {
                id: i as u32,
                expression: e.to_string(),
                value: v.to_string(),
                type_name: "int".to_string(),
                error: None,
            })
            .collect();
        if let Some(path) = source_path {
            snap.source = Some(crate::state::SourceFile {
                path: path.to_string(),
                lines: Vec::new(),
                highlighted: Vec::new(),
            });
        }
        snap.source_line = source_line;
        snap.current_thread_id = thread_id;
        snap.memory = memory;
        snap
    }

    #[test]
    fn compute_diff_detects_local_changes() {
        let snap1 = make_snapshot(
            vec![("x", "1"), ("y", "hello")],
            vec![],
            vec![],
            None,
            None,
            None,
            None,
        );
        let snap2 = make_snapshot(
            vec![("x", "2"), ("y", "hello")],
            vec![],
            vec![],
            None,
            None,
            None,
            None,
        );

        let mut rec = Recording::new(100, None);
        rec.capture(&snap1);
        rec.capture(&snap2);

        let diff = rec.get_diff(1).expect("should have diff for index 1");
        assert_eq!(diff.locals_changed.len(), 1);
        assert_eq!(diff.locals_changed[0].name, "x");
        assert_eq!(diff.locals_changed[0].old_value, "1");
        assert_eq!(diff.locals_changed[0].new_value, "2");
        assert!(diff.locals_added.is_empty());
        assert!(diff.locals_removed.is_empty());
    }

    #[test]
    fn compute_diff_detects_added_and_removed_locals() {
        let snap1 = make_snapshot(
            vec![("x", "1"), ("y", "2")],
            vec![],
            vec![],
            None,
            None,
            None,
            None,
        );
        // 'y' removed, 'z' added
        let snap2 = make_snapshot(
            vec![("x", "1"), ("z", "3")],
            vec![],
            vec![],
            None,
            None,
            None,
            None,
        );

        let mut rec = Recording::new(100, None);
        rec.capture(&snap1);
        rec.capture(&snap2);

        let diff = rec.get_diff(1).unwrap();
        assert!(diff.locals_changed.is_empty());
        assert_eq!(diff.locals_added, vec!["z"]);
        assert_eq!(diff.locals_removed, vec!["y"]);
    }

    #[test]
    fn compute_diff_detects_register_changes() {
        let snap1 = make_snapshot(
            vec![],
            vec![("rax", "0x0"), ("rbx", "0xff")],
            vec![],
            None,
            None,
            None,
            None,
        );
        let snap2 = make_snapshot(
            vec![],
            vec![("rax", "0x42"), ("rbx", "0xff")],
            vec![],
            None,
            None,
            None,
            None,
        );

        let mut rec = Recording::new(100, None);
        rec.capture(&snap1);
        rec.capture(&snap2);

        let diff = rec.get_diff(1).unwrap();
        assert_eq!(diff.registers_changed.len(), 1);
        assert_eq!(diff.registers_changed[0].name, "rax");
        assert_eq!(diff.registers_changed[0].old_value, "0x0");
        assert_eq!(diff.registers_changed[0].new_value, "0x42");
    }

    #[test]
    fn compute_diff_detects_source_and_thread_changes() {
        let snap1 = make_snapshot(
            vec![],
            vec![],
            vec![],
            Some("main.c"),
            Some(10),
            Some(1),
            None,
        );
        let snap2 = make_snapshot(
            vec![],
            vec![],
            vec![],
            Some("main.c"),
            Some(15),
            Some(2),
            None,
        );

        let mut rec = Recording::new(100, None);
        rec.capture(&snap1);
        rec.capture(&snap2);

        let diff = rec.get_diff(1).unwrap();
        assert!(diff.source_changed);
        assert!(diff.thread_changed);
    }

    #[test]
    fn compute_diff_detects_watch_changes() {
        let snap1 = make_snapshot(
            vec![],
            vec![],
            vec![("*ptr", "42"), ("arr[0]", "10")],
            None,
            None,
            None,
            None,
        );
        let snap2 = make_snapshot(
            vec![],
            vec![],
            vec![("*ptr", "99"), ("arr[0]", "10")],
            None,
            None,
            None,
            None,
        );

        let mut rec = Recording::new(100, None);
        rec.capture(&snap1);
        rec.capture(&snap2);

        let diff = rec.get_diff(1).unwrap();
        assert_eq!(diff.watches_changed.len(), 1);
        assert_eq!(diff.watches_changed[0].expression, "*ptr");
        assert_eq!(diff.watches_changed[0].old_value, "42");
        assert_eq!(diff.watches_changed[0].new_value, "99");
    }

    #[test]
    fn compute_diff_detects_memory_changes() {
        let mem1 = MemoryBlock {
            address: 0x1000,
            bytes: vec![0x00, 0x11, 0x22, 0x33],
        };
        let mem2 = MemoryBlock {
            address: 0x1000,
            bytes: vec![0x00, 0xFF, 0x22, 0xAA],
        };

        let snap1 = make_snapshot(vec![], vec![], vec![], None, None, None, Some(mem1));
        let snap2 = make_snapshot(vec![], vec![], vec![], None, None, None, Some(mem2));

        let mut rec = Recording::new(100, None);
        rec.capture(&snap1);
        rec.capture(&snap2);

        let diff = rec.get_diff(1).unwrap();
        assert_eq!(diff.memory_changed.len(), 2);
        assert_eq!(diff.memory_changed[0].offset, 1);
        assert_eq!(diff.memory_changed[0].old_byte, 0x11);
        assert_eq!(diff.memory_changed[0].new_byte, 0xFF);
        assert_eq!(diff.memory_changed[1].offset, 3);
        assert_eq!(diff.memory_changed[1].old_byte, 0x33);
        assert_eq!(diff.memory_changed[1].new_byte, 0xAA);
    }

    #[test]
    fn recording_enforces_max_entries() {
        let mut rec = Recording::new(3, None);
        for i in 0..5 {
            let mut snap = GdbSnapshot::empty();
            snap.current_frame_level = i;
            rec.capture(&snap);
        }

        assert_eq!(rec.len(), 3);
        // The oldest two should have been evicted; remaining seqs are 2, 3, 4
        assert_eq!(rec.get(0).unwrap().seq, 2);
        assert_eq!(rec.get(1).unwrap().seq, 3);
        assert_eq!(rec.get(2).unwrap().seq, 4);
    }

    #[test]
    fn recording_disabled_does_not_capture() {
        let mut rec = Recording::new(100, None);
        rec.enabled = false;
        rec.capture(&GdbSnapshot::empty());
        assert!(rec.is_empty());
    }

    #[test]
    fn recording_clear() {
        let mut rec = Recording::new(100, None);
        rec.capture(&GdbSnapshot::empty());
        rec.capture(&GdbSnapshot::empty());
        assert_eq!(rec.len(), 2);
        rec.clear();
        assert!(rec.is_empty());
    }

    #[test]
    fn first_state_has_no_diff() {
        let mut rec = Recording::new(100, None);
        rec.capture(&GdbSnapshot::empty());
        assert!(rec.get_diff(0).is_none());
    }

    #[test]
    fn prev_anchor_and_next_anchor_navigation() {
        let mut rec = Recording::new(100, None);

        // Capture 5 states: indices 0,1,2,3,4
        // Make indices 1 and 3 anchors (breakpoint hits)
        let plain_snap = GdbSnapshot::empty();
        let mut bp_snap = GdbSnapshot::empty();
        bp_snap.stop_reason = Some(StopReason::BreakpointHit { number: 1 });

        rec.capture(&plain_snap);   // 0: not anchor
        rec.capture(&bp_snap);      // 1: anchor (breakpoint)
        rec.capture(&plain_snap);   // 2: not anchor
        rec.capture(&bp_snap);      // 3: anchor (breakpoint)
        rec.capture(&plain_snap);   // 4: not anchor

        assert_eq!(rec.len(), 5);

        // prev_anchor from index 4 should find 3
        assert_eq!(rec.prev_anchor(4), Some(3));
        // prev_anchor from index 3 should find 1
        assert_eq!(rec.prev_anchor(3), Some(1));
        // prev_anchor from index 1 should find nothing
        assert_eq!(rec.prev_anchor(1), None);
        // prev_anchor from index 0 should find nothing
        assert_eq!(rec.prev_anchor(0), None);

        // next_anchor from index 0 should find 1
        assert_eq!(rec.next_anchor(0), Some(1));
        // next_anchor from index 1 should find 3
        assert_eq!(rec.next_anchor(1), Some(3));
        // next_anchor from index 3 should find nothing
        assert_eq!(rec.next_anchor(3), None);
        // next_anchor from index 4 should find nothing
        assert_eq!(rec.next_anchor(4), None);
    }

    #[test]
    fn anchor_count_matches() {
        let mut rec = Recording::new(100, None);
        let plain_snap = GdbSnapshot::empty();
        let mut bp_snap = GdbSnapshot::empty();
        bp_snap.stop_reason = Some(StopReason::BreakpointHit { number: 1 });

        rec.capture(&plain_snap); // not anchor
        rec.capture(&bp_snap);    // anchor
        rec.capture(&plain_snap); // not anchor
        rec.capture(&bp_snap);    // anchor
        rec.capture(&bp_snap);    // anchor

        assert_eq!(rec.anchor_count(), 3);
    }

    #[test]
    fn capture_anchor_sets_is_anchor_true() {
        let mut rec = Recording::new(100, None);
        let snap = GdbSnapshot::empty();

        // capture_anchor forces is_anchor = true even without a breakpoint stop reason
        rec.capture_anchor(&snap);
        assert!(rec.get(0).unwrap().is_anchor);

        // regular capture without breakpoint stop reason should not be an anchor
        rec.capture(&snap);
        assert!(!rec.get(1).unwrap().is_anchor);
    }

    #[test]
    fn watchpoint_stop_reason_is_anchor() {
        let mut rec = Recording::new(100, None);
        let mut snap = GdbSnapshot::empty();
        snap.stop_reason = Some(StopReason::Watchpoint { number: 2 });
        rec.capture(&snap);
        assert!(rec.get(0).unwrap().is_anchor);
    }

    #[test]
    fn step_finished_is_not_anchor() {
        let mut rec = Recording::new(100, None);
        let mut snap = GdbSnapshot::empty();
        snap.stop_reason = Some(StopReason::StepFinished);
        rec.capture(&snap);
        assert!(!rec.get(0).unwrap().is_anchor);
    }

    #[test]
    fn no_diff_when_nothing_changed() {
        let snap = make_snapshot(
            vec![("x", "1")],
            vec![("rax", "0x0")],
            vec![],
            Some("main.c"),
            Some(10),
            Some(1),
            None,
        );

        let mut rec = Recording::new(100, None);
        rec.capture(&snap);
        rec.capture(&snap);

        let diff = rec.get_diff(1).unwrap();
        assert!(diff.locals_changed.is_empty());
        assert!(diff.locals_added.is_empty());
        assert!(diff.locals_removed.is_empty());
        assert!(diff.registers_changed.is_empty());
        assert!(diff.watches_changed.is_empty());
        assert!(!diff.source_changed);
        assert!(!diff.thread_changed);
        assert!(diff.memory_changed.is_empty());
    }
}
