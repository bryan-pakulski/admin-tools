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

fn hint(text: &str) -> Line<'_> {
    Line::from(Span::styled(text, Style::default().fg(Color::DarkGray)))
}

pub fn draw(f: &mut Frame, area: Rect, view: &ViewState) {
    let width = 66.min(area.width.saturating_sub(2));
    let height = area.height.saturating_sub(2);

    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;

    let rect = Rect::new(x, y, width, height);

    f.render_widget(Clear, rect);

    let block = Block::default()
        .title(" Help — ? to close, j/k to scroll ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let help_text = vec![
        // ---- EXECUTION ----
        heading("Execution"),
        Line::from("  F5             Run (if not started) / Continue (if stopped)"),
        Line::from("  F6             Trace — step line-by-line to next breakpoint"),
        Line::from("  F7             Step into function call"),
        Line::from("  F8             Step over (next line, skip function bodies)"),
        Line::from("  F9             Step out (finish current function)"),
        Line::from("  Shift+F5       Interrupt (pause a running program)"),
        Line::from("  Ctrl+X         Interrupt (alternative, works on all terminals)"),
        Line::from(""),

        // ---- GLOBAL NAVIGATION ----
        heading("Navigation"),
        Line::from("  j / k          Move selection up / down in focused panel"),
        Line::from("  g / G          Jump to top / bottom of list"),
        Line::from("  PgUp / PgDn    Scroll by page"),
        Line::from("  Enter          Activate selection (panel-specific action)"),
        Line::from("  Tab            Cycle focus to next panel"),
        Line::from("  Shift+Tab      Cycle focus to previous panel"),
        Line::from("  1-9, 0         Toggle panel visibility (number in title)"),
        Line::from("  Esc            Exit current mode / clear / unfocus"),
        Line::from(""),

        // ---- SOURCE [1] ----
        heading("[1] Source"),
        Line::from("  j / k          Move cursor line by line"),
        Line::from("  Enter          Set breakpoint at cursor line"),
        Line::from("  F10            Toggle breakpoint at cursor (set or delete)"),
        Line::from("  .              Jump cursor back to execution line"),
        Line::from("  /              Search in source text"),
        Line::from("  n / N          Jump to next / previous search match"),
        Line::from("  w              Watch identifier on cursor line (prefilled)"),
        Line::from("  p              Evaluate identifier on cursor line (prefilled)"),
        Line::from(""),

        // ---- STACK [2] ----
        heading("[2] Stack"),
        Line::from("  Enter          Select frame — source, locals, registers update"),
        Line::from(""),

        // ---- LOCALS [3] ----
        heading("[3] Locals"),
        Line::from("  w              Watch selected variable (prefilled)"),
        Line::from("  p              Evaluate selected variable (prefilled)"),
        Line::from("  m              View variable in memory browser (prefilled)"),
        Line::from(""),

        // ---- THREADS [4] ----
        heading("[4] Threads"),
        Line::from("  Enter          Switch to selected thread"),
        Line::from(""),

        // ---- BREAKPOINTS [5] ----
        heading("[5] Breakpoints"),
        Line::from("  b              Set breakpoint (function, file:line, or *addr)"),
        Line::from("  B              Conditional breakpoint (location if condition)"),
        Line::from("  c              Edit condition on selected breakpoint"),
        Line::from("  W              Set hardware watchpoint (expr [r|w|rw])"),
        Line::from("  d              Delete selected breakpoint / watchpoint"),
        Line::from("  e              Enable / disable selected breakpoint"),
        hint("  Condition syntax:  main.c:42 if x > 100"),
        hint("  Watchpoint syntax: my_var  |  my_var r  |  my_var rw"),
        hint("  Address breakpoint: *0x401000"),
        Line::from(""),

        // ---- REGISTERS [6] ----
        heading("[6] Registers"),
        Line::from("  E              Edit selected register value"),
        hint("  Format: name value   (e.g., rax 0x42)"),
        hint("  Registers auto-load on every stop"),
        Line::from(""),

        // ---- MEMORY [7] ----
        heading("[7] Memory"),
        Line::from("  m              Go to address (hex or expression like &my_var)"),
        Line::from("  Arrows / j/k   Move cursor byte-by-byte / row-by-row"),
        Line::from("  PgUp / PgDn    Jump by 256 bytes"),
        Line::from("  Enter          Follow pointer at cursor (read 8 bytes as addr)"),
        Line::from("  v              Start / extend byte selection"),
        Line::from("  t              Cycle type cast for selection"),
        hint("                 hex u8 i8 u16 u32 u64 i16 i32 i64 f32 f64 utf8"),
        Line::from("  i              Enter hex edit mode (type hex digits to write)"),
        Line::from("  S              Search memory for string or hex bytes"),
        Line::from("  Esc            Clear selection / exit edit / leave panel"),
        hint("  Search syntax: hello world  |  \\x90\\x90\\x90  |  0x41 0x42"),
        Line::from(""),

        // ---- DISASM [8] ----
        heading("[8] Disassembly"),
        Line::from("  j / k          Move cursor through instructions"),
        Line::from("  Enter          Set breakpoint at cursor address"),
        Line::from("  F10            Toggle breakpoint at cursor address"),
        Line::from("  x              Analyze cross-references (who calls / what calls)"),
        Line::from("  s              Resolve symbol at cursor address"),
        Line::from("  P              NOP out instruction at cursor (x86: 0x90 fill)"),
        Line::from("  a              Patch raw bytes at cursor address"),
        hint("  Xrefs shown inline as magenta annotations"),
        hint("  Colors: red=jump yellow=call green=ret cyan=mem gray=nop"),
        Line::from(""),

        // ---- WATCH [9] ----
        heading("[9] Watch Expressions"),
        Line::from("  w              Add watch expression"),
        Line::from("  d              Remove selected watch"),
        Line::from("  p              Evaluate selected expression"),
        Line::from("  m              View in memory browser"),
        hint("  Expressions re-evaluated automatically on each stop"),
        Line::from(""),

        // ---- OUTPUT [0] ----
        heading("[0] Output"),
        Line::from("  :              Enter raw GDB command"),
        Line::from("  ;              Repeat last raw command"),
        hint("  Shows GDB console output, errors, and info messages"),
        Line::from(""),

        // ---- INSPECTION ----
        heading("Smart Inspection (auto-prefills from context)"),
        Line::from("  w              Add watch — prefilled from Source/Locals/Watch"),
        Line::from("  m              Memory — prefilled with &variable or pointer value"),
        Line::from("  p              Evaluate — prefilled from focused panel context"),
        Line::from("  T              Type overlay — cast memory as C struct/type"),
        hint("  Type overlay:  0xADDR struct name  |  0xADDR int[10]"),
        Line::from(""),

        // ---- ANALYSIS ----
        heading("Analysis"),
        Line::from("  x              Cross-references at disasm cursor"),
        Line::from("  T              Type overlay on memory (struct field view)"),
        Line::from("  f              List all known functions"),
        Line::from("  s              Resolve symbol at address"),
        Line::from("  S              Search memory for string / hex bytes"),
        Line::from("  L              Show loaded shared libraries"),
        Line::from(""),

        // ---- PATCHING ----
        heading("Patching (Disasm panel)"),
        Line::from("  P              NOP instruction at cursor"),
        Line::from("  a              Write raw bytes at address"),
        hint("  Patch format:  0xADDR hex_bytes"),
        hint("  Examples:  0x401000 eb fe   (infinite loop)"),
        hint("             0x401000 90 90 90  (3x NOP)"),
        Line::from(""),

        // ---- TRACE / RECORDING ----
        heading("Execution Tracing"),
        Line::from("  F6             Trace to breakpoint with full state capture"),
        hint("  Each step records: frame, locals, registers, disassembly"),
        hint("  Full stack backtrace captured on final stop"),
        hint("  Auto instruction-steps when no source line info"),
        hint("  Cancel with F5 or Ctrl+X"),
        Line::from(""),
        heading("Timeline / Playback"),
        Line::from("  [  /  ]        Step backward / forward in recorded history"),
        Line::from("  <  /  >        Jump to previous / next breakpoint anchor"),
        Line::from("  {  /  }        Jump to first recorded state / return to live"),
        Line::from("  R              Toggle recording on / off"),
        Line::from("  C              Clear all recorded states"),
        hint("  Timeline: \u{00b7} = step  \u{25cf} = breakpoint anchor"),
        hint("  Colors: Yellow=step  Red=breakpoint  Blue=signal"),
        hint("  Diff line shows variable changes between states"),
        Line::from(""),

        // ---- PLAYBACK ANALYSIS ----
        heading("Playback Analysis"),
        Line::from("  H              Show value history for selected variable/register"),
        hint("  Works in Locals or Registers panel during playback mode"),
        hint("  Scans full recording trace for value changes"),
        hint("  Results displayed in Output panel with trend analysis"),
        hint("  Source/Disasm panels show line/address hit counts in playback"),
        hint("  Colors: gray=1x  yellow=2-5x  red=6+ (hot loop)"),
        Line::from(""),

        // ---- MOUSE ----
        heading("Mouse Support"),
        Line::from("  Left click     Focus panel and select item under cursor"),
        Line::from("  Scroll up/dn   Scroll focused panel by 3 lines"),
        hint("  Click Source line to move cursor, Stack frame to select, etc."),
        Line::from(""),

        // ---- CHANGE HIGHLIGHTING ----
        heading("Change Highlighting"),
        hint("  Variables and registers that changed on the last stop"),
        hint("  are shown in red bold until the next stop event."),
        hint("  Applies to Locals [3] and Registers [6] panels."),
        Line::from(""),

        // ---- RE MODE ----
        heading("Reverse Engineering Mode"),
        hint("  Auto-activates when no debug symbols detected"),
        hint("  Layout switches to: Disasm + Registers + Memory + Stack"),
        hint("  All analysis keys (x, T, f, s, S, L, P, a) work globally"),
        Line::from(""),

        // ---- GENERAL ----
        heading("General"),
        Line::from("  ?  /  F1       Toggle this help (scroll with j/k)"),
        Line::from("  q              Quit (press y to confirm)"),
    ];

    let paragraph = Paragraph::new(help_text)
        .scroll((view.help_scroll, 0))
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, inner);
}
