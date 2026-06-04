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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlight_returns_correct_line_count() {
        let lines: Vec<String> = vec![
            "int main() {".into(),
            "    return 0;".into(),
            "}".into(),
        ];
        let result = highlight_lines("test.c", &lines);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn highlight_returns_nonempty_segments_for_c_code() {
        let lines: Vec<String> = vec!["int x = 42;".into()];
        let result = highlight_lines("test.c", &lines);
        assert_eq!(result.len(), 1);
        assert!(!result[0].is_empty(), "should produce at least one styled segment");
        // Every segment should have non-empty text (or at least the concatenation is non-empty)
        let full_text: String = result[0].iter().map(|s| s.text.as_str()).collect();
        assert!(full_text.contains("int"), "highlighted text should contain 'int'");
    }

    #[test]
    fn highlight_falls_back_to_plain_text_for_unknown_extension() {
        let lines: Vec<String> = vec!["some random text".into()];
        let result = highlight_lines("file.xyz_unknown_ext_999", &lines);
        assert_eq!(result.len(), 1);
        assert!(!result[0].is_empty());
        let full_text: String = result[0].iter().map(|s| s.text.as_str()).collect();
        assert_eq!(full_text, "some random text");
    }

    #[test]
    fn highlight_empty_input_returns_empty_output() {
        let lines: Vec<String> = vec![];
        let result = highlight_lines("test.c", &lines);
        assert!(result.is_empty());
    }
}
