use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::state::GdbSnapshot;
use super::super::ViewState;
use super::super::input::InputMode;
use super::super::layout::Panel;

fn key(label: &str) -> Span<'_> {
    Span::styled(label, Style::default().fg(Color::Cyan))
}

fn sep(label: &str) -> Span<'_> {
    Span::raw(label)
}

pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState) {
    if view.quit_confirm {
        let line = Line::from(vec![
            Span::styled(
                " Quit? ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Press "),
            Span::styled("y", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw(" to confirm, any other key to cancel"),
        ]);
        let bar = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));
        f.render_widget(bar, rect);
        return;
    }

    if view.input_mode != InputMode::Normal {
        let mode_label = match view.input_mode {
            InputMode::Command => "COMMAND",
            InputMode::Breakpoint => "BREAKPOINT",
            InputMode::BreakpointCond => "COND BREAK",
            InputMode::BreakCondEdit => "EDIT COND",
            InputMode::Watchpoint => "WATCHPOINT",
            InputMode::RegisterEdit => "EDIT REG",
            InputMode::Watch => "WATCH",
            InputMode::Memory => "MEMORY",
            InputMode::Eval => "EVAL",
            InputMode::Search => "SEARCH",
            InputMode::SearchMemory => "SEARCH MEM",
            InputMode::PatchBytes => "PATCH BYTES",
            InputMode::TypeOverlay => "TYPE OVERLAY",
            InputMode::ListFunctions => "FUNCTIONS",
            InputMode::Ptype => "PTYPE",
            InputMode::ExplorerAdd => "EXPLORER",
            InputMode::Normal => unreachable!(),
        };

        let format_hint = match view.input_mode {
            InputMode::BreakpointCond => "  loc if cond  (e.g. main.c:42 if x>0)",
            InputMode::Watchpoint => "  expr [r|w|rw]  (e.g. my_var rw)",
            InputMode::RegisterEdit => "  name value  (e.g. rax 0x42)",
            InputMode::Memory => "  addr [len] or &expr  (e.g. &buf 512)",
            InputMode::SearchMemory => "  text or \\xHH  (e.g. hello or \\x90\\x90)",
            InputMode::PatchBytes => "  addr hex_bytes  (e.g. 0x401000 90 90)",
            InputMode::TypeOverlay => "  addr type  (e.g. 0x7fff struct foo)",
            InputMode::ListFunctions => "  regex filter (or Enter for all, max 200 shown)",
            InputMode::Ptype => "  expression or type  (e.g. my_var, MyClass, ptr->member)",
            InputMode::ExplorerAdd => "  expression  (e.g. my_var, ClassName::singleton, *(Type*)0xaddr)",
            _ => "",
        };

        let line = Line::from(vec![
            Span::styled(
                format!(" {} ", mode_label),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" Enter=submit  Esc=cancel  Up/Down=history"),
            Span::styled(format_hint, Style::default().fg(Color::DarkGray)),
        ]);
        let bar = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));
        f.render_widget(bar, rect);
        return;
    }

    // Global execution hints (always shown)
    let mut spans: Vec<Span> = vec![
        key(" F5"), sep(":Run "),
        key("F6"), sep(":Trace "),
        key("F7"), sep(":Into "),
        key("F8"), sep(":Next "),
        key("F9"), sep(":Out "),
    ];

    // Panel-specific hints
    match view.focused_panel {
        Panel::Source => {
            spans.extend_from_slice(&[
                sep("| "),
                key("Enter"), sep(":SetBrk "),
                key("F10"), sep(":TogBrk "),
                key("w"), sep(":Watch "),
                key("p"), sep(":Eval "),
                key("y"), sep(":Ptype "),
                key("."), sep(":GoExec "),
            ]);
        }
        Panel::Stack => {
            spans.extend_from_slice(&[
                sep("| "),
                key("Enter"), sep(":SelectFrame "),
            ]);
        }
        Panel::Threads => {
            spans.extend_from_slice(&[
                sep("| "),
                key("Enter"), sep(":SwitchThread "),
            ]);
        }
        Panel::Breakpoints => {
            spans.extend_from_slice(&[
                sep("| "),
                key("d"), sep(":Del "),
                key("e"), sep(":Toggle "),
                key("b"), sep(":New "),
                key("B"), sep(":CondBrk "),
                key("c"), sep(":EditCond "),
                key("W"), sep(":Watchpt "),
            ]);
        }
        Panel::Locals => {
            spans.extend_from_slice(&[
                sep("| "),
                key("w"), sep(":Watch "),
                key("p"), sep(":Eval "),
                key("m"), sep(":Memory "),
                key("y"), sep(":Ptype "),
            ]);
            if view.playback_mode {
                spans.extend_from_slice(&[
                    key("H"), sep(":History "),
                ]);
            }
        }
        Panel::Watch => {
            spans.extend_from_slice(&[
                sep("| "),
                key("w"), sep(":Add "),
                key("d"), sep(":Del "),
                key("p"), sep(":Eval "),
                key("m"), sep(":Memory "),
                key("y"), sep(":Ptype "),
            ]);
        }
        Panel::Registers => {
            spans.extend_from_slice(&[
                sep("| "),
                key("E"), sep(":Edit "),
                key("6"), sep(":Toggle "),
            ]);
            if view.playback_mode {
                spans.extend_from_slice(&[
                    key("H"), sep(":History "),
                ]);
            }
        }
        Panel::Memory => {
            if view.mem_edit {
                spans.extend_from_slice(&[
                    sep("| "),
                    Span::styled(" EDIT ", Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)),
                    sep(" hex:type "),
                    key("Esc"), sep(":Stop "),
                ]);
            } else {
                spans.extend_from_slice(&[
                    sep("| "),
                    key("m"), sep(":GoAddr "),
                    key("Enter"), sep(":FollowPtr "),
                    key("S"), sep(":Search "),
                    key("v"), sep(":Select "),
                    key("t"), sep(":Cast "),
                    key("i"), sep(":Edit "),
                    key("\u{2190}\u{2191}\u{2193}\u{2192}"), sep(":Nav "),
                ]);
            }
        }
        Panel::Disasm => {
            spans.extend_from_slice(&[
                sep("| "),
                key("Enter"), sep(":Follow/Brk "),
                key("."), sep(":GoPC "),
                key("F10"), sep(":TogBrk "),
                key("x"), sep(":Xrefs "),
                key("P"), sep(":NOP "),
                key("a"), sep(":Patch "),
            ]);
        }
        Panel::Explorer => {
            spans.extend_from_slice(&[
                sep("| "),
                key("Enter"), sep(":Expand "),
                key("d"), sep(":Remove "),
                key("I"), sep(":Add "),
                key("y"), sep(":Ptype "),
                key("p"), sep(":Eval "),
            ]);
        }
        Panel::Output => {
            spans.extend_from_slice(&[
                sep("| "),
                key(":"), sep(":Cmd "),
                key(";"), sep(":Repeat "),
            ]);
        }
    }

    // Playback hints (shown when recording has entries)
    if view.rec_count > 0 {
        spans.push(sep("| "));
        if view.playback_mode {
            spans.extend_from_slice(&[
                Span::styled(
                    " PLAYBACK ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                ),
                sep(" "),
                key("["), sep(":Prev "),
                key("]"), sep(":Next "),
                key("<"), sep(":PrevBP "),
                key(">"), sep(":NextBP "),
                key("}"), sep(":Live "),
            ]);
        } else {
            spans.extend_from_slice(&[
                key("["), sep(":Rewind "),
                key("<"), sep(":PrevBP "),
                key("R"), sep(":Rec"),
                sep(if view.rec_enabled { "On " } else { "Off " }),
                key("C"), sep(":Clear "),
            ]);
        }
    }

    // Tail: help + quit + context info
    let thread_info = if let Some(tid) = snap.current_thread_id {
        format!("T{}", tid)
    } else {
        String::new()
    };

    let frame_info = if !snap.stack.is_empty() {
        let fr = &snap.stack[snap.current_frame_level as usize % snap.stack.len()];
        let func = fr.func.as_deref().unwrap_or("??");
        if let (Some(file), Some(line)) = (&fr.file, fr.line) {
            let short = file.rsplit('/').next().unwrap_or(file);
            format!("{func} {short}:{line}")
        } else {
            func.to_string()
        }
    } else {
        String::new()
    };

    // Library count indicator
    let lib_info = if !snap.mapped_libs.is_empty() {
        format!("{}libs", snap.mapped_libs.len())
    } else {
        String::new()
    };

    spans.extend_from_slice(&[
        sep("| "),
        key("I"), sep(":Explorer "),
        key("f"), sep(":Funcs "),
        key("L"), sep(":Libs "),
        key("S"), sep(":MemSearch "),
        key("?"), sep(":Help "),
        key("q"), sep(":Quit"),
        sep("  "),
        Span::styled(thread_info, Style::default().fg(Color::DarkGray)),
        sep(" "),
        Span::styled(frame_info, Style::default().fg(Color::DarkGray)),
    ]);
    if !lib_info.is_empty() {
        spans.push(sep(" "));
        spans.push(Span::styled(lib_info, Style::default().fg(Color::DarkGray)));
    }

    let line = Line::from(spans);
    let bar = Paragraph::new(line).style(
        Style::default().bg(Color::DarkGray).fg(Color::White),
    );
    f.render_widget(bar, rect);
}
