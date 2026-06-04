use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::GdbSnapshot;
use super::super::ViewState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InsnClass {
    Jump,
    Call,
    Return,
    Memory,
    Nop,
    Other,
}

fn classify_insn(inst: &str) -> InsnClass {
    let mnemonic = inst.split_whitespace().next().unwrap_or("").to_lowercase();

    if matches!(mnemonic.as_str(), "ret" | "retn" | "retf" | "iret" | "iretd" | "iretq"
        | "sysret" | "sysexit" | "bx") {
        return InsnClass::Return;
    }
    if mnemonic.starts_with("call") || mnemonic == "bl" || mnemonic == "blx"
        || mnemonic == "blr" || mnemonic == "syscall" || mnemonic == "svc"
    {
        return InsnClass::Call;
    }
    if mnemonic.starts_with('j')
        || (mnemonic.starts_with('b') && matches!(mnemonic.as_str(),
            "b" | "beq" | "bne" | "blt" | "bgt" | "ble" | "bge" | "blo" | "bhi"
            | "bhs" | "bls" | "bpl" | "bmi" | "bvs" | "bvc" | "bcs" | "bcc"
            | "bal" | "bnv" | "b.eq" | "b.ne" | "b.lt" | "b.gt" | "b.le" | "b.ge"
            | "cbz" | "cbnz" | "tbz" | "tbnz"))
        || mnemonic == "loop" || mnemonic == "loope" || mnemonic == "loopne"
    {
        return InsnClass::Jump;
    }
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
    if matches!(mnemonic.as_str(), "nop" | "nopl" | "nopw" | "int3" | "ud2" | "hlt" | "endbr64" | "endbr32") {
        return InsnClass::Nop;
    }
    InsnClass::Other
}

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

/// Extract a call/jump target address from an instruction string.
pub fn parse_call_target(inst: &str) -> Option<u64> {
    let class = classify_insn(inst);
    if !matches!(class, InsnClass::Call | InsnClass::Jump) {
        return None;
    }
    for word in inst.split_whitespace().skip(1) {
        let clean = word.trim_start_matches("0x").trim_start_matches('*');
        let hex_part = clean.split(|c: char| !c.is_ascii_hexdigit()).next().unwrap_or(clean);
        if hex_part.len() >= 4 {
            if let Ok(addr) = u64::from_str_radix(hex_part, 16) {
                return Some(addr);
            }
        }
    }
    None
}

pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    // Show current function in title if available
    let func_title = snap.disasm.get(view.disasm_cursor)
        .and_then(|inst| inst.func_name.as_deref())
        .map(|name| format!(" [8] Disasm <{name}> "))
        .unwrap_or_else(|| " [8] Disasm ".to_string());

    let block = Block::default()
        .title(func_title)
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
    let cursor = view.disasm_cursor.min(total.saturating_sub(1));

    let scroll = if cursor >= visible_height / 2 {
        cursor - visible_height / 2
    } else {
        0
    };

    let current_pc = snap.stack.first().map(|fr| fr.addr);

    let bp_addrs: std::collections::HashSet<u64> = snap
        .breakpoints
        .iter()
        .filter_map(|bp| bp.address)
        .collect();

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
            let class = classify_insn(&inst.inst);

            // Function boundary: show a separator when function changes
            let prev_func = if idx > 0 {
                snap.disasm.get(idx - 1).and_then(|i| i.func_name.as_deref())
            } else {
                None
            };
            let cur_func = inst.func_name.as_deref();
            let is_func_entry = idx == scroll // first visible line
                || (cur_func.is_some() && cur_func != prev_func);

            // Gutter
            let bp_marker = if has_bp { "\u{25cf}" } else { " " };
            let pc_marker = if is_current { "=>" } else { "  " };
            let gutter = format!("{bp_marker}{pc_marker} ");

            let bp_style = if has_bp {
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
            } else if is_current {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            // Function label for entry points
            let func_label = if is_func_entry {
                if let (Some(name), Some(0)) | (Some(name), None) = (cur_func, inst.offset) {
                    format!("<{name}>: ")
                } else if let Some(name) = cur_func {
                    if inst.offset == Some(0) {
                        format!("<{name}>: ")
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            };

            let insn_style = if is_current {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(insn_color(class))
            };

            let addr_style = if is_current {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            let mut spans = vec![
                Span::styled(gutter, bp_style),
                Span::styled(format!("{:012x} ", inst.address), addr_style),
            ];

            if !func_label.is_empty() {
                spans.push(Span::styled(
                    func_label,
                    Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD),
                ));
            }

            spans.push(Span::styled(inst.inst.clone(), insn_style));

            // For call instructions, show the target function name
            if class == InsnClass::Call {
                if let Some(target_addr) = parse_call_target(&inst.inst) {
                    let target_name = snap.disasm.iter()
                        .find(|i| i.address == target_addr)
                        .and_then(|i| i.func_name.as_deref());
                    if let Some(name) = target_name {
                        spans.push(Span::styled(
                            format!("  ; -> {name}"),
                            Style::default().fg(Color::Yellow),
                        ));
                    }
                }
            }

            // Jump target hints for conditional jumps
            if class == InsnClass::Jump {
                if let Some(target_addr) = parse_call_target(&inst.inst) {
                    let direction = if target_addr < inst.address { "\u{2191}" } else { "\u{2193}" };
                    let offset = if target_addr >= inst.address {
                        format!("+{:#x}", target_addr - inst.address)
                    } else {
                        format!("-{:#x}", inst.address - target_addr)
                    };
                    spans.push(Span::styled(
                        format!("  ; {direction}{offset}"),
                        Style::default().fg(Color::Red),
                    ));
                }
            }

            // Xref annotations
            if let Some(xrefs) = xref_addrs.get(&inst.address) {
                let xref_summary: Vec<String> = xrefs.iter().map(|x| {
                    let dir = match x.xref_type {
                        crate::state::XrefType::CallTo => "\u{2190}",
                        crate::state::XrefType::CallFrom => "\u{2192}",
                        crate::state::XrefType::JumpTo => "\u{21b5}",
                    };
                    let name = x.func_name.as_deref().unwrap_or("??");
                    format!("{dir}{name}")
                }).collect();
                spans.push(Span::styled(
                    format!("  [{}]", xref_summary.join(", ")),
                    Style::default().fg(Color::Magenta),
                ));
            }

            // Playback hit counts
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

            // Cursor highlight
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
