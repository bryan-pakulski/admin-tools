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
            Panel::Source | Panel::Stack | Panel::Locals | Panel::Breakpoints | Panel::Output
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
/// | footer (1 line)                          |
/// +-----------------------------------------+
/// ```
pub fn compute(area: Rect, visible: &[Panel]) -> PanelLayout {
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

    // Vertical split: status_bar | main_area | output_area? | footer
    let mut vert_constraints = vec![Constraint::Length(1)]; // status bar

    if has_output {
        // Main gets remaining, output gets ~25% (min 6 lines)
        let total_inner = area.height.saturating_sub(2); // minus status + footer
        let output_h = (total_inner / 4).max(6).min(total_inner.saturating_sub(6));
        let main_h = total_inner.saturating_sub(output_h);
        vert_constraints.push(Constraint::Length(main_h));
        vert_constraints.push(Constraint::Length(output_h));
    } else {
        vert_constraints.push(Constraint::Fill(1));
    }
    vert_constraints.push(Constraint::Length(1)); // footer

    let vert_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vert_constraints)
        .split(area);

    let status_bar = vert_chunks[0];
    let main_area = vert_chunks[1];
    let output_area = if has_output {
        Some(vert_chunks[2])
    } else {
        None
    };
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
