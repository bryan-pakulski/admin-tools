use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::GdbSnapshot;
use super::super::{MemCast, ViewState};

const BYTES_PER_ROW: usize = 16;

pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState, focused: bool) {
    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let edit_tag = if view.mem_edit && focused { " EDIT" } else { "" };
    let cast_tag = if view.mem_sel_start.is_some() {
        format!(" [{}]", view.mem_cast.label())
    } else {
        String::new()
    };
    let title = format!(" [7] Memory @ {:#x}{edit_tag}{cast_tag} ", snap.memory_address);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    let mem = match &snap.memory {
        Some(m) => m,
        None => {
            let msg = Paragraph::new("No memory loaded. Press m to inspect an address.")
                .style(Style::default().fg(Color::DarkGray));
            f.render_widget(msg, inner);
            return;
        }
    };

    if mem.bytes.is_empty() {
        return;
    }

    // Split: hex dump on top, interpretation bar at the bottom (2 lines)
    let has_selection = view.mem_sel_start.is_some();
    let interp_height = if has_selection { 3u16 } else { 0 };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(interp_height),
        ])
        .split(inner);

    let hex_area = chunks[0];
    let interp_area = if has_selection { Some(chunks[1]) } else { None };

    let visible_height = hex_area.height as usize;
    let total_rows = (mem.bytes.len() + BYTES_PER_ROW - 1) / BYTES_PER_ROW;

    // Scroll to keep cursor visible
    let cursor_row = view.mem_cursor / BYTES_PER_ROW;
    let scroll = if cursor_row >= view.memory_scroll + visible_height {
        cursor_row - visible_height + 1
    } else if cursor_row < view.memory_scroll {
        cursor_row
    } else {
        view.memory_scroll
    };

    let sel_start = view.mem_sel_start.unwrap_or(usize::MAX);
    let sel_end = view.mem_sel_end.unwrap_or(0);
    let sel_lo = sel_start.min(sel_end);
    let sel_hi = sel_start.max(sel_end);

    let lines: Vec<Line> = (0..total_rows)
        .skip(scroll)
        .take(visible_height)
        .map(|row| {
            let offset = row * BYTES_PER_ROW;
            let addr = mem.address + offset as u64;
            let chunk = &mem.bytes[offset..mem.bytes.len().min(offset + BYTES_PER_ROW)];

            let mut spans: Vec<Span> = Vec::with_capacity(BYTES_PER_ROW + 4);

            // Address column
            spans.push(Span::styled(
                format!("{:012x}  ", addr),
                Style::default().fg(Color::Yellow),
            ));

            // Hex bytes
            for (i, &b) in chunk.iter().enumerate() {
                let byte_idx = offset + i;
                let is_cursor = focused && byte_idx == view.mem_cursor;
                let is_selected = has_selection && byte_idx >= sel_lo && byte_idx <= sel_hi;

                if i > 0 && i % 8 == 0 {
                    spans.push(Span::raw(" "));
                }

                let style = if is_cursor && view.mem_edit {
                    Style::default().fg(Color::Black).bg(Color::Red).add_modifier(Modifier::BOLD)
                } else if is_cursor {
                    Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
                } else if is_selected {
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else {
                    byte_color(b)
                };

                spans.push(Span::styled(format!("{:02x} ", b), style));
            }

            // Pad short last row
            let pad = BYTES_PER_ROW - chunk.len();
            if pad > 0 {
                let extra_spaces = if chunk.len() <= 8 && BYTES_PER_ROW > 8 { 1 } else { 0 };
                spans.push(Span::raw(" ".repeat(pad * 3 + extra_spaces)));
            }

            spans.push(Span::raw(" "));

            // ASCII column
            for (i, &b) in chunk.iter().enumerate() {
                let byte_idx = offset + i;
                let is_cursor = focused && byte_idx == view.mem_cursor;
                let is_selected = has_selection && byte_idx >= sel_lo && byte_idx <= sel_hi;
                let ch = if b.is_ascii_graphic() || b == b' ' { b as char } else { '.' };

                let style = if is_cursor {
                    Style::default().fg(Color::Black).bg(Color::Cyan)
                } else if is_selected {
                    Style::default().fg(Color::Black).bg(Color::Yellow)
                } else if b.is_ascii_graphic() {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                spans.push(Span::styled(ch.to_string(), style));
            }

            Line::from(spans)
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, hex_area);

    // Draw type interpretation bar if there's a selection
    if let Some(area) = interp_area {
        draw_interpretation(f, area, mem, view, sel_lo, sel_hi);
    }
}

fn draw_interpretation(
    f: &mut Frame,
    area: Rect,
    mem: &crate::state::MemoryBlock,
    view: &ViewState,
    sel_lo: usize,
    sel_hi: usize,
) {
    let bytes = &mem.bytes[sel_lo..=(sel_hi.min(mem.bytes.len() - 1))];
    let addr = mem.address + sel_lo as u64;

    let interp = interpret_bytes(bytes, view.mem_cast);
    let size_info = format!("{} bytes @ {:#x}", bytes.len(), addr);

    let line1 = Line::from(vec![
        Span::styled(
            format!(" {} ", view.mem_cast.label()),
            Style::default().fg(Color::Black).bg(Color::Magenta).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(interp, Style::default().fg(Color::White)),
        Span::raw("  "),
        Span::styled(size_info, Style::default().fg(Color::DarkGray)),
    ]);

    let all_casts = cast_summary(bytes);
    let line2 = Line::from(vec![
        Span::styled(
            format!(" {all_casts}"),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let p = Paragraph::new(vec![line1, line2]);
    f.render_widget(p, inner);
}

fn interpret_bytes(bytes: &[u8], cast: MemCast) -> String {
    match cast {
        MemCast::Hex => {
            bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
        }
        MemCast::U8 => {
            bytes.iter().map(|b| format!("{b}")).collect::<Vec<_>>().join(", ")
        }
        MemCast::I8 => {
            bytes.iter().map(|b| format!("{}", *b as i8)).collect::<Vec<_>>().join(", ")
        }
        MemCast::U16LE => read_chunks(bytes, 2, |b| format!("{}", u16::from_le_bytes([b[0], b[1]]))),
        MemCast::U32LE => read_chunks(bytes, 4, |b| format!("{}", u32::from_le_bytes(b[..4].try_into().unwrap()))),
        MemCast::U64LE => read_chunks(bytes, 8, |b| format!("{}", u64::from_le_bytes(b[..8].try_into().unwrap()))),
        MemCast::I16LE => read_chunks(bytes, 2, |b| format!("{}", i16::from_le_bytes([b[0], b[1]]))),
        MemCast::I32LE => read_chunks(bytes, 4, |b| format!("{}", i32::from_le_bytes(b[..4].try_into().unwrap()))),
        MemCast::I64LE => read_chunks(bytes, 8, |b| format!("{}", i64::from_le_bytes(b[..8].try_into().unwrap()))),
        MemCast::F32LE => read_chunks(bytes, 4, |b| format!("{:.6}", f32::from_le_bytes(b[..4].try_into().unwrap()))),
        MemCast::F64LE => read_chunks(bytes, 8, |b| format!("{:.6}", f64::from_le_bytes(b[..8].try_into().unwrap()))),
        MemCast::Utf8 => {
            match std::str::from_utf8(bytes) {
                Ok(s) => format!("{:?}", s),
                Err(_) => {
                    let lossy = String::from_utf8_lossy(bytes);
                    format!("{:?} (lossy)", lossy)
                }
            }
        }
    }
}

fn read_chunks(bytes: &[u8], size: usize, convert: fn(&[u8]) -> String) -> String {
    if bytes.len() < size {
        return format!("(need {} bytes, have {})", size, bytes.len());
    }
    let mut values = Vec::new();
    let mut i = 0;
    while i + size <= bytes.len() {
        values.push(convert(&bytes[i..i + size]));
        i += size;
    }
    values.join(", ")
}

fn cast_summary(bytes: &[u8]) -> String {
    let mut parts = Vec::new();
    if !bytes.is_empty() {
        parts.push(format!("u8:{}", bytes[0]));
    }
    if bytes.len() >= 2 {
        let v = u16::from_le_bytes([bytes[0], bytes[1]]);
        parts.push(format!("u16:{v}"));
    }
    if bytes.len() >= 4 {
        let v = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        parts.push(format!("u32:{v}"));
        let fv = f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        parts.push(format!("f32:{fv:.4}"));
    }
    if bytes.len() >= 8 {
        let v = u64::from_le_bytes(bytes[0..8].try_into().unwrap());
        parts.push(format!("u64:{v}"));
        parts.push(format!("ptr:{v:#x}"));
    }
    parts.join("  ")
}

fn byte_color(b: u8) -> Style {
    if b == 0 {
        Style::default().fg(Color::DarkGray)
    } else if b.is_ascii_graphic() || b == b' ' {
        Style::default().fg(Color::White)
    } else if b == 0xff {
        Style::default().fg(Color::Red)
    } else {
        Style::default().fg(Color::Cyan)
    }
}
