use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::super::ViewState;
use super::super::input::InputMode;

/// Draw the input prompt overlay at the bottom of the screen.
///
/// Shows a labelled input field with the current buffer contents and a cursor
/// indicator.
pub fn draw(f: &mut Frame, area: Rect, view: &ViewState) {
    let label = match view.input_mode {
        InputMode::Command => ":",
        InputMode::Breakpoint => "break ",
        InputMode::Watch => "watch ",
        InputMode::Memory => "memory ",
        InputMode::Eval => "eval ",
        InputMode::Search => "/",
        InputMode::Normal => return,
    };

    // Place the prompt one line above the footer
    let prompt_y = area.y + area.height.saturating_sub(2);
    let rect = Rect::new(area.x, prompt_y, area.width, 1);

    f.render_widget(Clear, rect);

    // Build the displayed text with cursor
    let before_cursor = &view.input_buffer[..view.input_cursor];
    let cursor_char = view
        .input_buffer
        .get(view.input_cursor..view.input_cursor + 1)
        .unwrap_or(" ");
    let after_cursor = if view.input_cursor < view.input_buffer.len() {
        &view.input_buffer[view.input_cursor + 1..]
    } else {
        ""
    };

    let line = Line::from(vec![
        Span::styled(
            label,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(before_cursor, Style::default().fg(Color::White)),
        Span::styled(
            cursor_char,
            Style::default()
                .fg(Color::Black)
                .bg(Color::White),
        ),
        Span::styled(after_cursor, Style::default().fg(Color::White)),
    ]);

    let paragraph = Paragraph::new(line).style(Style::default().bg(Color::DarkGray));
    f.render_widget(paragraph, rect);
}
