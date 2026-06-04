use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::GdbSnapshot;
use super::super::ViewState;

/// Classify an instruction mnemonic for color-coding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsnClass {
    Jump,
    Call,
    Return,
    Memory,
    Nop,
    Other,
}

/// Classify an instruction string by its mnemonic.
fn classify_insn(inst: &str) -> InsnClass {
    let mnemonic = inst.split_whitespace().next().unwrap_or("").to_lowercase();

    // Returns
    if matches!(mnemonic.as_str(), "ret" | "retn" | "retf" | "iret" | "iretd" | "iretq"
        | "sysret" | "sysexit" | "bx" /*ARM bx lr*/) {
        return InsnClass::Return;
    }

    // Calls
    if mnemonic.starts_with("call") || mnemonic == "bl" || mnemonic == "blx"
        || mnemonic == "blr" || mnemonic == "syscall" || mnemonic == "svc"
    {
        return InsnClass::Call;
    }

    // Jumps / branches (x86 + ARM)
    if mnemonic.starts_with('j')  // jmp, je, jne, jg, jl, jge, jle, ja, jb, jz, jnz, ...
        || (mnemonic.starts_with('b') && matches!(mnemonic.as_str(),
            "b" | "beq" | "bne" | "blt" | "bgt" | "ble" | "bge" | "blo" | "bhi"
            | "bhs" | "bls" | "bpl" | "bmi" | "bvs" | "bvc" | "bcs" | "bcc"
            | "bal" | "bnv" | "b.eq" | "b.ne" | "b.lt" | "b.gt" | "b.le" | "b.ge"
            | "cbz" | "cbnz" | "tbz" | "tbnz"))
        || mnemonic == "loop" || mnemonic == "loope" || mnemonic == "loopne"
    {
        return InsnClass::Jump;
    }

    // Memory operations
    if matches!(mnemonic.as_str(),
        "mov" | "movl" | "movq" | "movb" | "movw" | "movzx" | "movsx" | "movsxd"
        | "movzbl" | "movzbw" | "movswl" | "movslq" | "movabs"
        | "lea" | "leal" | "leaq"
        | "push" | "pushl" | "pushq" | "pushw" | "pusha" | "pushad" | "pushf" | "pushfd"
        | "pop" | "popl" | "popq" | "popw" | "popa" | "popad" | "popf" | "popfd"
        | "ldr" | "ldp" | "ldrb" | "ldrh" | "ldrsb" | "ldrsh" | "ldrsw"
        | "str" | "stp" | "strb" | "strh"
        | "load" | "store"
        | "cmov" | "cmove" | "cmovne" | "cmovg" | "cmovl" | "cmovge" | "cmovle"
        | "cmova" | "cmovb" | "cmovae" | "cmovbe"
        | "xchg" | "cmpxchg"
    ) {
        return InsnClass::Memory;
    }

    // NOP / padding
    if matches!(mnemonic.as_str(), "nop" | "nopl" | "nopw" | "int3" | "ud2" | "hlt") {
        return InsnClass::Nop;
    }

    InsnClass::Other
}

/// Get the color for an instruction class.
fn insn_color(class: InsnClass) -> Color {
    match class {
        InsnClass::Jump => Color::Red,
        InsnClass::Call => Color::Yellow,
        InsnClass::Return => Color::Green,
        InsnClass::Memory => Color::Cyan,
        InsnClass::Nop => Color::DarkGray,
        InsnClass::Other => Color::White,
    }
}

/// Draw the disassembly panel.
pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(" [8] Disasm ")
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if snap.disasm.is_empty() {
        let msg = Paragraph::new("No disassembly. Stop at a breakpoint to view.")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(msg, inner);
        return;
    }

    let visible_height = inner.height as usize;
    let total = snap.disasm.len();

    // Clamp cursor
    let cursor = view.disasm_cursor.min(total.saturating_sub(1));

    // Center the cursor in the visible area (like source panel)
    let scroll = if cursor >= visible_height / 2 {
        cursor - visible_height / 2
    } else {
        0
    };

    // Find the current PC address from the top stack frame
    let current_pc = snap.stack.first().map(|fr| fr.addr);

    // Collect breakpoint addresses for gutter marking
    let bp_addrs: std::collections::HashSet<u64> = snap
        .breakpoints
        .iter()
        .filter_map(|bp| bp.address)
        .collect();

    // Build a set of addresses that have xref entries for inline annotation
    let xref_addrs: std::collections::HashMap<u64, Vec<&crate::state::XrefEntry>> = {
        let mut map: std::collections::HashMap<u64, Vec<&crate::state::XrefEntry>> =
            std::collections::HashMap::new();
        for xref in &snap.xrefs {
            map.entry(xref.address).or_default().push(xref);
        }
        map
    };

    let lines: Vec<Line> = snap
        .disasm
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(idx, inst)| {
            let is_current = current_pc == Some(inst.address);
            let is_cursor = idx == cursor;
            let has_bp = bp_addrs.contains(&inst.address);

            // Gutter: breakpoint marker + PC marker
            let bp_marker = if has_bp { "\u{25cf}" } else { " " }; // filled circle
            let pc_marker = if is_current { "=>" } else { "  " };
            let gutter = format!("{}{} ", bp_marker, pc_marker);

            let bp_style = if has_bp {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let func_info = match (&inst.func_name, inst.offset) {
                (Some(name), Some(off)) => format!("<{}+{}>  ", name, off),
                (Some(name), None) => format!("<{}>  ", name),
                _ => String::new(),
            };

            let class = classify_insn(&inst.inst);
            let insn_fg = insn_color(class);

            // Build the base style for the instruction
            let insn_style = if is_current {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(insn_fg)
            };

            let addr_style = if is_current {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let func_style = if is_current {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
            };

            let mut spans = vec![
                Span::styled(gutter, bp_style),
                Span::styled(format!("{:#018x}  ", inst.address), addr_style),
                Span::styled(func_info, func_style),
                Span::styled(inst.inst.clone(), insn_style),
            ];

            // Show xref annotations inline if available
            if let Some(xrefs) = xref_addrs.get(&inst.address) {
                let xref_summary: Vec<String> = xrefs.iter().map(|x| {
                    let dir = match x.xref_type {
                        crate::state::XrefType::CallTo => "\u{2190}",   // left arrow
                        crate::state::XrefType::CallFrom => "\u{2192}", // right arrow
                        crate::state::XrefType::JumpTo => "\u{21b5}",   // corner arrow
                    };
                    let name = x.func_name.as_deref().unwrap_or("??");
                    format!("{dir}{name}")
                }).collect();
                let annotation = format!("  [{}]", xref_summary.join(", "));
                spans.push(Span::styled(
                    annotation,
                    Style::default().fg(Color::Magenta),
                ));
            }

            // Show execution flow hit counts in playback mode
            if view.playback_mode {
                if let Some(ref flow) = view.exec_flow {
                    if let Some(&count) = flow.addr_hits.get(&inst.address) {
                        let color = match count {
                            1 => Color::DarkGray,
                            2..=5 => Color::Yellow,
                            _ => Color::Red,
                        };
                        spans.push(Span::styled(
                            format!(" {count}x"),
                            Style::default().fg(color),
                        ));
                    }
                }
            }

            // Apply cursor highlight as a background on the whole line
            if is_cursor && focused {
                for span in &mut spans {
                    span.style = span.style.bg(Color::DarkGray);
                }
            }

            Line::from(spans)
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}
