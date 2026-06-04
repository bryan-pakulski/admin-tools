use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::super::ViewState;

fn h(text: &str) -> Line<'_> {
    Line::from(Span::styled(
        text,
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    ))
}

fn d(text: &str) -> Line<'_> {
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
        .title(" Help — ? close, j/k scroll ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let t = vec![
        h("Execution"),
        Line::from("  F5             Run / Continue"),
        Line::from("  F6             Trace to breakpoint (full state each step)"),
        Line::from("  F7             Step into"),
        Line::from("  F8             Step over (auto instruction-step if no source)"),
        Line::from("  F9             Step out"),
        Line::from("  Shift+F5       Interrupt"),
        Line::from("  Ctrl+X         Interrupt (works on all terminals)"),
        Line::from(""),
        h("Navigation"),
        Line::from("  j/k  Up/Down   Move selection"),
        Line::from("  g / G          Top / bottom"),
        Line::from("  PgUp/PgDn      Page scroll"),
        Line::from("  Enter          Activate (panel-specific)"),
        Line::from("  Tab/Shift+Tab  Cycle panel focus"),
        Line::from("  1-9, 0         Toggle panel visibility"),
        Line::from("  Esc            Exit mode / clear / leave panel"),
        Line::from("  Mouse click    Focus + select. Scroll wheel scrolls"),
        Line::from(""),
        h("[1] Source"),
        Line::from("  Enter          Set breakpoint at cursor"),
        Line::from("  F10            Toggle breakpoint"),
        Line::from("  .              Jump to execution line"),
        Line::from("  /  n/N         Search, next/prev match"),
        Line::from("  w  p           Watch / eval identifier (prefilled)"),
        Line::from("  x              Call graph from stack trace"),
        Line::from(""),
        h("[2] Stack"),
        Line::from("  Enter          Select frame (updates source+locals+disasm)"),
        Line::from(""),
        h("[3] Locals"),
        Line::from("  w  p  m        Watch / eval / memory (prefilled)"),
        d("  Changed values glow red bold"),
        Line::from(""),
        h("[4] Threads"),
        Line::from("  Enter          Switch thread"),
        Line::from(""),
        h("[5] Breakpoints"),
        Line::from("  b              Breakpoint (func, file:line, *0xaddr)"),
        Line::from("  B              Conditional (location if condition)"),
        Line::from("  c              Edit condition on selected"),
        Line::from("  W              Hardware watchpoint (expr [r|w|rw])"),
        Line::from("  d  e           Delete / enable-disable"),
        Line::from(""),
        h("[6] Registers"),
        Line::from("  E              Edit register (name value)"),
        d("  Auto-loaded every stop. Changed values red bold"),
        Line::from(""),
        h("[7] Memory"),
        Line::from("  m              Go to address (hex or &expr)"),
        Line::from("  Enter          Follow pointer at cursor"),
        Line::from("  v              Start/extend selection"),
        Line::from("  t              Cycle type cast (hex u8..u64 f32 f64 utf8)"),
        Line::from("  i              Hex edit mode"),
        Line::from("  S              Search (string or \\xHH hex)"),
        Line::from("  Esc            Clear / exit / leave"),
        Line::from(""),
        h("[8] Disassembly"),
        Line::from("  Enter          Follow call/jump target (else set breakpoint)"),
        Line::from("  .              Jump cursor to PC"),
        Line::from("  F10            Toggle breakpoint at address"),
        Line::from("  x              Cross-references"),
        Line::from("  s              Resolve symbol"),
        Line::from("  P              NOP instruction"),
        Line::from("  a              Patch bytes"),
        d("  Function boundaries + call targets shown inline"),
        d("  Colors: red=jump yellow=call green=ret cyan=mem gray=nop"),
        Line::from(""),
        h("[9] Watch"),
        Line::from("  w  d  p  m     Add / remove / eval / memory"),
        Line::from(""),
        h("[0] Output"),
        Line::from("  :              Raw GDB command"),
        Line::from("  ;              Repeat last command"),
        Line::from(""),
        h("Inspection (auto-prefills from context)"),
        Line::from("  w              Watch (Source/Locals/Watch prefill)"),
        Line::from("  m              Memory (prefills &var or pointer)"),
        Line::from("  p              Eval (prefills from context)"),
        Line::from("  T              Type overlay (0xADDR struct name)"),
        Line::from(""),
        h("Analysis"),
        Line::from("  x              Xrefs (Disasm) / call graph (Source/Stack)"),
        Line::from("  f              List functions (regex filter)"),
        Line::from("  s              Resolve symbol at address"),
        Line::from("  S              Search memory"),
        Line::from("  L              Loaded libraries"),
        Line::from(""),
        h("Tracing + Playback"),
        Line::from("  F6             Trace: frame + locals + registers + disasm each step"),
        Line::from("  [ / ]          Step backward / forward in history"),
        Line::from("  < / >          Prev / next breakpoint anchor"),
        Line::from("  { / }          First / return to live"),
        Line::from("  R              Toggle recording"),
        Line::from("  C              Clear recording"),
        Line::from("  H              Value history (Locals/Registers)"),
        d("  Hit counts during playback: gray=1x yellow=2-5x red=6+"),
        Line::from(""),
        h("Patching"),
        Line::from("  P              NOP instruction at cursor"),
        Line::from("  a              Write bytes (0xADDR hex_bytes)"),
        Line::from(""),
        h("RE Mode (no symbols)"),
        d("  Auto-switches layout: Disasm+Registers+Memory+Stack+Breakpoints"),
        d("  Python/Ruby/Java/Node detected -> runtime command hints"),
        d("  All analysis keys work: x T f s S L P a"),
        Line::from(""),
        h("General"),
        Line::from("  ? / F1         This help (j/k scroll)"),
        Line::from("  q              Quit (y confirms)"),
    ];

    let paragraph = Paragraph::new(t)
        .scroll((view.help_scroll, 0))
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, inner);
}
