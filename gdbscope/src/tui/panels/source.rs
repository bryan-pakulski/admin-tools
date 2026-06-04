use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::state::{GdbSnapshot, StyledSegment};
use super::super::ViewState;

const SPINNER: &[&str] = &["\u{28cb}", "\u{2819}", "\u{2839}", "\u{2838}", "\u{283c}", "\u{2834}", "\u{2826}", "\u{2827}", "\u{2807}", "\u{280f}"];

pub fn draw(f: &mut Frame, rect: Rect, snap: &GdbSnapshot, view: &ViewState, focused: bool) {
    let title = if snap.source_loading {
        let phase = SPINNER[(view.tick_count as usize / 3) % SPINNER.len()];
        format!(" [1] Source {phase} loading... ")
    } else {
        match &snap.source {
            Some(src) => {
                let name = src.path.rsplit('/').next().unwrap_or(&src.path);
                format!(" [1] Source [{}] ", name)
            }
            None => " [1] Source ".to_string(),
        }
    };

    let border_style = if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(rect);
    f.render_widget(block, rect);

    if snap.source_loading {
        let phase = SPINNER[(view.tick_count as usize / 3) % SPINNER.len()];
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  {phase} Loading source file..."),
                Style::default().fg(Color::Yellow),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Resolving debug symbols and reading file from disk.",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let msg = Paragraph::new(lines);
        f.render_widget(msg, inner);
        return;
    }

    let source = match &snap.source {
        Some(src) => src,
        None => {
            let mut lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    "  No source code available for the current frame.",
                    Style::default().fg(Color::DarkGray),
                )),
                Line::from(""),
            ];

            // Detect runtime and show language-specific hints
            let is_python = snap.stack.iter().any(|f| {
                f.func.as_deref().map_or(false, |n| n.contains("PyEval") || n.contains("_Py"))
            });
            let is_ruby = snap.stack.iter().any(|f| {
                f.func.as_deref().map_or(false, |n| n.contains("rb_") || n.contains("ruby"))
            });
            let is_java = snap.stack.iter().any(|f| {
                f.func.as_deref().map_or(false, |n| n.contains("JVM") || n.contains("JavaThread"))
            });
            let is_node = snap.stack.iter().any(|f| {
                f.func.as_deref().map_or(false, |n| n.contains("v8::") || n.contains("node::"))
            });

            if is_python {
                lines.extend_from_slice(&[
                    Line::from(Span::styled(
                        "  Python runtime detected. Try these GDB commands (:):",
                        Style::default().fg(Color::Yellow),
                    )),
                    Line::from(Span::styled("    py-bt          Python backtrace", Style::default().fg(Color::Cyan))),
                    Line::from(Span::styled("    py-list        Python source at current frame", Style::default().fg(Color::Cyan))),
                    Line::from(Span::styled("    py-locals      Python local variables", Style::default().fg(Color::Cyan))),
                    Line::from(Span::styled("    py-print EXPR  Evaluate Python expression", Style::default().fg(Color::Cyan))),
                    Line::from(""),
                    Line::from(Span::styled(
                        "  Press : to enter a command, or navigate stack with [2].",
                        Style::default().fg(Color::DarkGray),
                    )),
                ]);
            } else if is_ruby {
                lines.extend_from_slice(&[
                    Line::from(Span::styled(
                        "  Ruby runtime detected. Try: rb_backtrace, rb_ps",
                        Style::default().fg(Color::Yellow),
                    )),
                ]);
            } else if is_java {
                lines.extend_from_slice(&[
                    Line::from(Span::styled(
                        "  JVM detected. Try: info threads, thread apply all bt",
                        Style::default().fg(Color::Yellow),
                    )),
                ]);
            } else if is_node {
                lines.extend_from_slice(&[
                    Line::from(Span::styled(
                        "  Node.js/V8 detected. Try: v8 bt, v8 source",
                        Style::default().fg(Color::Yellow),
                    )),
                ]);
            } else if snap.target_state == crate::state::TargetState::Stopped && !snap.stack.is_empty() {
                lines.extend_from_slice(&[
                    Line::from(Span::styled(
                        "  No debug symbols. Use these panels instead:",
                        Style::default().fg(Color::Yellow),
                    )),
                    Line::from(Span::styled("    [8] Disasm     Disassembly view (primary for RE)", Style::default().fg(Color::Cyan))),
                    Line::from(Span::styled("    [6] Registers  CPU register values", Style::default().fg(Color::Cyan))),
                    Line::from(Span::styled("    [7] Memory     Hex memory browser", Style::default().fg(Color::Cyan))),
                    Line::from(""),
                    Line::from(Span::styled("    x  Xrefs    f  Functions    s  Symbols", Style::default().fg(Color::Cyan))),
                    Line::from(Span::styled("    P  NOP      a  Patch        T  Type cast", Style::default().fg(Color::Cyan))),
                    Line::from(""),
                    Line::from(Span::styled("    b  *0xADDR  Set breakpoint at address", Style::default().fg(Color::Cyan))),
                ]);
            } else {
                lines.push(Line::from(Span::styled(
                    "  Press F5 to run, or b to set a breakpoint.",
                    Style::default().fg(Color::DarkGray),
                )));
            }

            let msg = Paragraph::new(lines);
            f.render_widget(msg, inner);
            return;
        }
    };

    if source.lines.is_empty() {
        return;
    }

    let visible_height = inner.height as usize;
    if visible_height == 0 {
        return;
    }

    let cursor_line = view.source_cursor;
    let exec_line = snap.source_line.unwrap_or(0) as usize;

    let scroll = if cursor_line > 0 {
        let target = cursor_line.saturating_sub(1);
        if target >= visible_height / 2 {
            target - visible_height / 2
        } else {
            0
        }
    } else {
        0
    };

    let bp_lines: std::collections::HashSet<u32> = snap
        .breakpoints
        .iter()
        .filter(|bp| {
            bp.file.as_ref().map_or(false, |bf| {
                snap.source.as_ref().map_or(false, |s| s.path.ends_with(bf.as_str()))
            })
        })
        .filter_map(|bp| bp.line)
        .collect();

    let line_num_width = format!("{}", source.lines.len()).len();
    let has_highlight = !source.highlighted.is_empty();

    let lines: Vec<Line> = source
        .lines
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_height)
        .map(|(idx, text)| {
            let line_num = idx + 1;
            let is_exec = exec_line > 0 && line_num == exec_line;
            let is_cursor = focused && line_num == cursor_line;
            let is_bp = bp_lines.contains(&(line_num as u32));
            let is_bp_enabled = snap.breakpoints.iter().any(|bp| {
                bp.line == Some(line_num as u32)
                    && bp.enabled
                    && bp
                        .file
                        .as_ref()
                        .map_or(false, |bf| source.path.ends_with(bf.as_str()))
            });

            let marker = if is_exec && is_bp {
                Span::styled("*>", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
            } else if is_exec {
                Span::styled("=>", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
            } else if is_bp && is_bp_enabled {
                Span::styled("* ", Style::default().fg(Color::Red))
            } else if is_bp {
                Span::styled("o ", Style::default().fg(Color::DarkGray))
            } else {
                Span::raw("  ")
            };

            let num_str = format!("{:>width$} ", line_num, width = line_num_width);
            let num_style = if is_exec {
                Style::default().fg(Color::Yellow)
            } else if is_cursor {
                Style::default().fg(Color::Cyan)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            // Build the source text spans
            let text_spans = if has_highlight {
                build_highlighted_spans(
                    source.highlighted.get(idx),
                    text,
                    is_exec,
                    is_cursor,
                    &view.search_query,
                )
            } else {
                build_plain_spans(text, is_exec, is_cursor, &view.search_query)
            };

            let mut spans = vec![marker, Span::styled(num_str, num_style)];
            spans.extend(text_spans);

            // Show execution flow hit counts in playback mode
            if view.playback_mode {
                if let Some(ref flow) = view.exec_flow {
                    if let Some(ref src) = snap.source {
                        if let Some(file_hits) = flow.line_hits.get(&src.path) {
                            if let Some(&count) = file_hits.get(&(line_num as u32)) {
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
                }
            }

            Line::from(spans)
        })
        .collect();

    let paragraph = Paragraph::new(lines);
    f.render_widget(paragraph, inner);
}

fn build_highlighted_spans<'a>(
    segments: Option<&Vec<StyledSegment>>,
    fallback: &str,
    is_exec: bool,
    is_cursor: bool,
    search_query: &Option<String>,
) -> Vec<Span<'a>> {
    let segments = match segments {
        Some(s) if !s.is_empty() => s,
        _ => return build_plain_spans(fallback, is_exec, is_cursor, search_query),
    };

    // Determine background and modifier overlays for special lines
    let (bg, mods) = line_overlay(is_exec, is_cursor);

    segments
        .iter()
        .map(|seg| {
            let fg = Color::Rgb(seg.fg.r, seg.fg.g, seg.fg.b);

            let mut style = if is_exec || is_cursor {
                // On highlighted lines, keep syntax fg but apply bg overlay
                let mut s = Style::default().fg(fg);
                if let Some(b) = bg {
                    s = s.bg(b);
                }
                s.add_modifier(mods)
            } else if matches_search(search_query, &seg.text) {
                Style::default().fg(Color::Black).bg(Color::Yellow)
            } else {
                Style::default().fg(fg)
            };

            // Dim syntax colors slightly on non-focused cursor lines
            if is_cursor && !is_exec {
                style = style.add_modifier(Modifier::UNDERLINED);
            }

            Span::styled(seg.text.clone(), style)
        })
        .collect()
}

fn build_plain_spans<'a>(
    text: &str,
    is_exec: bool,
    is_cursor: bool,
    search_query: &Option<String>,
) -> Vec<Span<'a>> {
    let style = if is_cursor && is_exec {
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
    } else if is_exec {
        Style::default()
            .fg(Color::White)
            .bg(Color::DarkGray)
            .add_modifier(Modifier::BOLD)
    } else if is_cursor {
        Style::default()
            .fg(Color::White)
            .add_modifier(Modifier::UNDERLINED)
    } else if matches_search(search_query, text) {
        Style::default().fg(Color::Black).bg(Color::Yellow)
    } else {
        Style::default().fg(Color::White)
    };
    vec![Span::styled(text.to_string(), style)]
}

fn line_overlay(is_exec: bool, is_cursor: bool) -> (Option<Color>, Modifier) {
    if is_exec && is_cursor {
        (Some(Color::DarkGray), Modifier::BOLD)
    } else if is_exec {
        (Some(Color::DarkGray), Modifier::BOLD)
    } else if is_cursor {
        (None, Modifier::empty())
    } else {
        (None, Modifier::empty())
    }
}

fn matches_search(query: &Option<String>, text: &str) -> bool {
    match query {
        Some(q) if !q.is_empty() => text.contains(q.as_str()),
        _ => false,
    }
}
