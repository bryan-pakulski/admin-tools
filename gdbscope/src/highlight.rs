use syntect::highlighting::{ThemeSet, Style as SynStyle};
use syntect::parsing::SyntaxSet;
use syntect::easy::HighlightLines;

use crate::state::{StyledSegment, SyntaxColor};

pub fn highlight_lines(path: &str, lines: &[String]) -> Vec<Vec<StyledSegment>> {
    let ss = SyntaxSet::load_defaults_newlines();
    let ts = ThemeSet::load_defaults();

    let theme = &ts.themes["base16-ocean.dark"];

    let syntax = ss
        .find_syntax_for_file(path)
        .ok()
        .flatten()
        .unwrap_or_else(|| ss.find_syntax_plain_text());

    let mut h = HighlightLines::new(syntax, theme);

    lines
        .iter()
        .map(|line| {
            let line_nl = format!("{line}\n");
            match h.highlight_line(&line_nl, &ss) {
                Ok(ranges) => ranges
                    .into_iter()
                    .map(|(style, text)| to_segment(style, text))
                    .collect(),
                Err(_) => vec![StyledSegment {
                    text: line.clone(),
                    fg: SyntaxColor { r: 192, g: 192, b: 192 },
                }],
            }
        })
        .collect()
}

fn to_segment(style: SynStyle, text: &str) -> StyledSegment {
    let text = text.trim_end_matches('\n').to_string();
    let c = style.foreground;
    StyledSegment {
        text,
        fg: SyntaxColor { r: c.r, g: c.g, b: c.b },
    }
}
