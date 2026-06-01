use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::super::ViewState;

fn heading(text: &str) -> Line<'_> {
    Line::from(Span::styled(
        text,
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))
}

pub fn draw(f: &mut Frame, area: Rect, view: &ViewState) {
    let width = 62.min(area.width.saturating_sub(4));
    let height = 40.min(area.height.saturating_sub(4));

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;

    let rect = Rect::new(x, y, width, height);

    f.render_widget(Clear, rect);

    let block = Block::default()
        .title(" Help — press ? to close ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let help_text = vec![
        heading("Execution"),
        Line::from("  F5            Run / Continue"),
        Line::from("  Shift+F5      Interrupt (SIGINT)"),
        Line::from("  F7            Step into"),
        Line::from("  F8            Step over (next line)"),
        Line::from("  F9            Step out (finish function)"),
        Line::from(""),
        heading("Navigation (all panels)"),
        Line::from("  j / k         Move selection up / down"),
        Line::from("  g / G         Jump to top / bottom"),
        Line::from("  PgUp / PgDn   Scroll by page"),
        Line::from("  Tab           Cycle focus to next panel"),
        Line::from("  Shift+Tab     Cycle focus to previous panel"),
        Line::from("  1-9, 0        Toggle panel visibility"),
        Line::from(""),
        heading("Source panel"),
        Line::from("  j / k         Move cursor line by line"),
        Line::from("  Enter         Set breakpoint at cursor line"),
        Line::from("  F10           Toggle breakpoint at cursor"),
        Line::from("  .             Jump cursor to execution line"),
        Line::from("  /             Search in source"),
        Line::from("  n / N         Next / previous search match"),
        Line::from(""),
        heading("Stack panel"),
        Line::from("  Enter         Select frame (view source + locals)"),
        Line::from(""),
        heading("Threads panel"),
        Line::from("  Enter         Switch to selected thread"),
        Line::from(""),
        heading("Breakpoints panel"),
        Line::from("  b             Set new breakpoint (by location)"),
        Line::from("  d             Delete selected breakpoint"),
        Line::from("  e             Enable / disable selected"),
        Line::from(""),
        heading("Inspection  (auto-prefills from context)"),
        Line::from("  w             Add watch expression"),
        Line::from("  m             Examine memory at address"),
        Line::from("  p             Evaluate expression (one-shot)"),
        Line::from(""),
        Line::from(Span::styled(
            "  Prefill sources:",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "    Source — identifier on cursor line",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "    Locals — selected variable name",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(Span::styled(
            "    Watch  — selected watch expression",
            Style::default().fg(Color::DarkGray),
        )),
        Line::from(""),
        heading("Memory panel"),
        Line::from("  m             Go to address (hex)"),
        Line::from("  arrows/jk     Move cursor byte-by-byte"),
        Line::from("  v             Start / extend selection"),
        Line::from("  Esc           Clear selection / exit edit"),
        Line::from("  t             Cycle type cast for selection"),
        Line::from("                hex u8 i8 u16 u32 u64 f32 f64 utf8"),
        Line::from("  i             Enter edit mode (type hex digits)"),
        Line::from("  PgUp/PgDn     Jump by 16 rows (256 bytes)"),
        Line::from(""),
        heading("Command"),
        Line::from("  :             Enter raw GDB command"),
        Line::from("  ;             Repeat last command"),
        Line::from(""),
        heading("General"),
        Line::from("  ?  /  F1      Toggle this help"),
        Line::from("  q             Quit (with confirmation)"),
    ];

    let paragraph = Paragraph::new(help_text)
        .scroll((view.help_scroll, 0))
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, inner);
}
