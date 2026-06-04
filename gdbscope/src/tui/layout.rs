use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Panel {
    Source,
    Stack,
    Locals,
    Threads,
    Breakpoints,
    Registers,
    Memory,
    Disasm,
    Watch,
    Output,
}

impl Panel {
    /// Human-readable label for the panel title bar.
    pub fn label(self) -> &'static str {
        match self {
            Panel::Source => "Source",
            Panel::Stack => "Stack",
            Panel::Locals => "Locals",
            Panel::Threads => "Threads",
            Panel::Breakpoints => "Breakpoints",
            Panel::Registers => "Registers",
            Panel::Memory => "Memory",
            Panel::Disasm => "Disasm",
            Panel::Watch => "Watch",
            Panel::Output => "Output",
        }
    }

    /// Keyboard hint shown in the panel toggle UI (1-9, 0).
    pub fn key_hint(self) -> &'static str {
        match self {
            Panel::Source => "1",
            Panel::Stack => "2",
            Panel::Locals => "3",
            Panel::Threads => "4",
            Panel::Breakpoints => "5",
            Panel::Registers => "6",
            Panel::Memory => "7",
            Panel::Disasm => "8",
            Panel::Watch => "9",
            Panel::Output => "0",
        }
    }

    /// Whether this panel is visible by default on startup.
    pub fn default_visible(self) -> bool {
        matches!(
            self,
            Panel::Source | Panel::Stack | Panel::Locals | Panel::Breakpoints
        )
    }

    /// All panel variants in canonical order.
    pub fn all() -> &'static [Panel] {
        &[
            Panel::Source,
            Panel::Stack,
            Panel::Locals,
            Panel::Threads,
            Panel::Breakpoints,
            Panel::Registers,
            Panel::Memory,
            Panel::Disasm,
            Panel::Watch,
            Panel::Output,
        ]
    }

    /// Returns the index of this panel in the `all()` ordering (used for
    /// TogglePanel mapping).
    pub fn index(self) -> usize {
        Panel::all()
            .iter()
            .position(|&p| p == self)
            .expect("panel not in all()")
    }

    /// Returns true if this panel belongs on the left side of the layout
    /// (source code / disassembly).
    fn is_left(self) -> bool {
        matches!(self, Panel::Source | Panel::Disasm)
    }
}

pub struct PanelLayout {
    pub status_bar: Rect,
    pub main_area: Rect,
    pub output_area: Option<Rect>,
    pub timeline_area: Option<Rect>,
    pub footer: Rect,
    pub left_panels: Vec<(Panel, Rect)>,
    pub right_panels: Vec<(Panel, Rect)>,
}

/// Compute the panel layout for the given terminal area.
///
/// The layout is structured as follows:
///
/// ```text
/// +-----------------------------------------+
/// | status bar (1 line)                      |
/// +-----------------------------------------+
/// | left panels     |  right panels          |
/// | (Source, Disasm) |  (Stack, Locals, ...)  |
/// |                  |                        |
/// +-----------------------------------------+
/// | output panel (if visible, ~25% or 6 min) |
/// +-----------------------------------------+
/// | timeline bar (if recording, 4 lines)     |
/// +-----------------------------------------+
/// | footer (1 line)                          |
/// +-----------------------------------------+
/// ```
pub fn compute(area: Rect, visible: &[Panel]) -> PanelLayout {
    compute_with_timeline(area, visible, false)
}

/// Like [`compute`] but optionally reserves space for the timeline bar.
pub fn compute_with_timeline(area: Rect, visible: &[Panel], show_timeline: bool) -> PanelLayout {
    // Separate panels into categories
    let left: Vec<Panel> = visible
        .iter()
        .copied()
        .filter(|p| p.is_left())
        .collect();
    let right: Vec<Panel> = visible
        .iter()
        .copied()
        .filter(|p| !p.is_left() && *p != Panel::Output)
        .collect();
    let has_output = visible.contains(&Panel::Output);

    // Timeline height: 4 lines for border + timeline bar + diff line + border
    let timeline_h: u16 = if show_timeline { 4 } else { 0 };

    // Fixed rows: status(1) + footer(1) + timeline
    let fixed_rows = 2 + timeline_h;

    // Vertical split: status_bar | main_area | output_area? | timeline? | footer
    let mut vert_constraints = vec![Constraint::Length(1)]; // status bar

    if has_output {
        // Main gets remaining, output gets ~25% (min 6 lines)
        let total_inner = area.height.saturating_sub(fixed_rows);
        let output_h = (total_inner / 4).max(6).min(total_inner.saturating_sub(6));
        let main_h = total_inner.saturating_sub(output_h);
        vert_constraints.push(Constraint::Length(main_h));
        vert_constraints.push(Constraint::Length(output_h));
    } else {
        vert_constraints.push(Constraint::Fill(1));
    }
    if show_timeline {
        vert_constraints.push(Constraint::Length(timeline_h));
    }
    vert_constraints.push(Constraint::Length(1)); // footer

    let vert_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vert_constraints)
        .split(area);

    let status_bar = vert_chunks[0];
    let main_area = vert_chunks[1];

    // The chunk indices shift depending on which optional sections are present
    let mut next_idx = 2;
    let output_area = if has_output {
        let r = Some(vert_chunks[next_idx]);
        next_idx += 1;
        r
    } else {
        None
    };
    let timeline_area = if show_timeline {
        let r = Some(vert_chunks[next_idx]);
        next_idx += 1;
        r
    } else {
        None
    };
    let _ = next_idx; // suppress unused warning
    let footer = vert_chunks[vert_chunks.len() - 1];

    // Horizontal split of main_area into left and right
    let mut left_panels_out = Vec::new();
    let mut right_panels_out = Vec::new();

    if !left.is_empty() && !right.is_empty() {
        // Split main area: left ~60%, right ~40%
        let horiz_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(main_area);

        split_panels_vertically(&left, horiz_chunks[0], &mut left_panels_out);
        split_panels_vertically(&right, horiz_chunks[1], &mut right_panels_out);
    } else if !left.is_empty() {
        // Only left panels, they get full width
        split_panels_vertically(&left, main_area, &mut left_panels_out);
    } else if !right.is_empty() {
        // Only right panels, they get full width
        split_panels_vertically(&right, main_area, &mut right_panels_out);
    }

    PanelLayout {
        status_bar,
        main_area,
        output_area,
        timeline_area,
        footer,
        left_panels: left_panels_out,
        right_panels: right_panels_out,
    }
}

/// Split a set of panels vertically within the given area, distributing space
/// equally among them.
fn split_panels_vertically(
    panels: &[Panel],
    area: Rect,
    out: &mut Vec<(Panel, Rect)>,
) {
    if panels.is_empty() {
        return;
    }
    let constraints: Vec<Constraint> = panels
        .iter()
        .map(|_| Constraint::Fill(1))
        .collect();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    for (panel, &rect) in panels.iter().zip(chunks.iter()) {
        out.push((*panel, rect));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn test_area() -> Rect {
        Rect::new(0, 0, 120, 40)
    }

    #[test]
    fn panel_all_returns_10_panels() {
        assert_eq!(Panel::all().len(), 10);
    }

    #[test]
    fn panel_default_visible_correct() {
        assert!(Panel::Source.default_visible());
        assert!(Panel::Stack.default_visible());
        assert!(Panel::Locals.default_visible());
        assert!(Panel::Breakpoints.default_visible());
        assert!(!Panel::Output.default_visible());

        assert!(!Panel::Threads.default_visible());
        assert!(!Panel::Registers.default_visible());
        assert!(!Panel::Memory.default_visible());
        assert!(!Panel::Disasm.default_visible());
        assert!(!Panel::Watch.default_visible());
    }

    #[test]
    fn panel_index_consistent_with_all() {
        for (i, panel) in Panel::all().iter().enumerate() {
            assert_eq!(panel.index(), i, "panel {:?} index mismatch", panel);
        }
    }

    #[test]
    fn compute_with_default_panels_produces_nonzero_rects() {
        let visible: Vec<Panel> = Panel::all()
            .iter()
            .copied()
            .filter(|p| p.default_visible())
            .collect();
        let layout = compute(test_area(), &visible);

        assert!(layout.status_bar.width > 0);
        assert!(layout.status_bar.height > 0);
        assert!(layout.footer.width > 0);
        assert!(layout.footer.height > 0);
        assert!(layout.main_area.width > 0);
        assert!(layout.main_area.height > 0);

        // Output is NOT default visible (toggled with 0)
        assert!(layout.output_area.is_none());

        // Left panels should include Source
        assert!(!layout.left_panels.is_empty());
        for (panel, rect) in &layout.left_panels {
            assert!(rect.width > 0, "left panel {:?} has zero width", panel);
            assert!(rect.height > 0, "left panel {:?} has zero height", panel);
        }

        // Right panels should include Stack, Locals, Breakpoints
        assert!(!layout.right_panels.is_empty());
        for (panel, rect) in &layout.right_panels {
            assert!(rect.width > 0, "right panel {:?} has zero width", panel);
            assert!(rect.height > 0, "right panel {:?} has zero height", panel);
        }
    }

    #[test]
    fn compute_with_timeline_allocates_timeline_area() {
        let visible: Vec<Panel> = Panel::all()
            .iter()
            .copied()
            .filter(|p| p.default_visible())
            .collect();

        let layout_no_tl = compute_with_timeline(test_area(), &visible, false);
        assert!(layout_no_tl.timeline_area.is_none());

        let layout_tl = compute_with_timeline(test_area(), &visible, true);
        assert!(layout_tl.timeline_area.is_some());
        let tl = layout_tl.timeline_area.unwrap();
        assert!(tl.width > 0);
        assert_eq!(tl.height, 4); // timeline is 4 lines
    }

    #[test]
    fn horizontal_split_separates_left_and_right() {
        let visible = vec![Panel::Source, Panel::Stack, Panel::Locals];
        let layout = compute(test_area(), &visible);

        assert!(!layout.left_panels.is_empty());
        assert!(!layout.right_panels.is_empty());

        let left_x = layout.left_panels[0].1.x;
        let right_x = layout.right_panels[0].1.x;
        assert!(
            right_x > left_x,
            "right panels (x={}) should have greater x than left panels (x={})",
            right_x,
            left_x
        );
    }

    #[test]
    fn only_right_panels_get_full_width() {
        let visible = vec![Panel::Stack, Panel::Locals];
        let layout = compute(test_area(), &visible);

        assert!(layout.left_panels.is_empty());
        assert!(!layout.right_panels.is_empty());
        // Right panels should use the full main_area width
        for (_panel, rect) in &layout.right_panels {
            assert_eq!(rect.width, layout.main_area.width);
        }
    }

    #[test]
    fn no_output_panel_means_no_output_area() {
        let visible = vec![Panel::Source, Panel::Stack];
        let layout = compute(test_area(), &visible);
        assert!(layout.output_area.is_none());
    }

    #[test]
    fn panel_labels_are_nonempty() {
        for panel in Panel::all() {
            assert!(!panel.label().is_empty());
        }
    }

    #[test]
    fn panel_key_hints_are_nonempty() {
        for panel in Panel::all() {
            assert!(!panel.key_hint().is_empty());
        }
    }
}
