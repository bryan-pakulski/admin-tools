/// Recursive-descent parser for GDB/MI output lines.
///
/// Each line of GDB/MI output maps to one [`MiRecord`].  The parser operates
/// on a `&mut &str` cursor that is advanced as tokens are consumed.

use anyhow::{bail, Context, Result};

use super::mi_types::{MiList, MiRecord, MiValue};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Parse a single line of GDB/MI output into an [`MiRecord`].
///
/// The line should have any trailing newline already stripped.
pub fn parse_line(line: &str) -> Result<MiRecord> {
    let trimmed = line.trim_end();

    // Prompt ------------------------------------------------------------------
    if trimmed == "(gdb)" || trimmed == "(gdb) " {
        return Ok(MiRecord::Prompt);
    }

    // Stream records ----------------------------------------------------------
    if let Some(rest) = trimmed.strip_prefix('~') {
        let s = parse_cstring_full(rest).context("console stream c-string")?;
        return Ok(MiRecord::StreamConsole(s));
    }
    if let Some(rest) = trimmed.strip_prefix('@') {
        let s = parse_cstring_full(rest).context("target stream c-string")?;
        return Ok(MiRecord::StreamTarget(s));
    }
    if let Some(rest) = trimmed.strip_prefix('&') {
        let s = parse_cstring_full(rest).context("log stream c-string")?;
        return Ok(MiRecord::StreamLog(s));
    }

    // Result / Async records --------------------------------------------------
    let mut cursor: &str = trimmed;

    // Optional leading token (decimal digits before the type prefix).
    let token = parse_token(&mut cursor);

    // The next character determines the record type.
    let kind = cursor
        .chars()
        .next()
        .context("unexpected end of MI line after token")?;

    // Advance past the prefix character.
    cursor = &cursor[1..];

    let class = parse_class(&mut cursor);

    let body = if cursor.is_empty() {
        Vec::new()
    } else {
        parse_body(&mut cursor)?
    };

    match kind {
        '^' => Ok(MiRecord::Result { token, class, body }),
        '*' => Ok(MiRecord::AsyncExec { token, class, body }),
        '+' => Ok(MiRecord::AsyncStatus { class, body }),
        '=' => Ok(MiRecord::AsyncNotify { class, body }),
        other => bail!("unknown MI record prefix '{}'", other),
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Consume leading ASCII digits from the cursor and return them as a `u64`.
fn parse_token(cursor: &mut &str) -> Option<u64> {
    let end = cursor
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(cursor.len());
    if end == 0 {
        return None;
    }
    let digits = &cursor[..end];
    *cursor = &cursor[end..];
    digits.parse::<u64>().ok()
}

/// Read the result/async class identifier (alphabetic + hyphens + digits)
/// stopping at a comma or end-of-string.
fn parse_class(cursor: &mut &str) -> String {
    let end = cursor
        .find(|c: char| c == ',')
        .unwrap_or(cursor.len());
    let class = cursor[..end].to_string();
    *cursor = &cursor[end..];
    class
}

/// Parse a comma-separated sequence of `key=value` pairs.
/// The cursor is expected to point at the first comma.
fn parse_body(cursor: &mut &str) -> Result<Vec<(String, MiValue)>> {
    let mut results = Vec::new();
    while !cursor.is_empty() {
        // Consume a leading comma.
        if cursor.starts_with(',') {
            *cursor = &cursor[1..];
        } else {
            break;
        }
        let (key, value) = parse_result_pair(cursor)?;
        results.push((key, value));
    }
    Ok(results)
}

/// Parse one `variable=value` pair.
fn parse_result_pair(cursor: &mut &str) -> Result<(String, MiValue)> {
    let key = parse_variable(cursor)?;
    if !cursor.starts_with('=') {
        bail!(
            "expected '=' after variable '{}', got {:?}",
            key,
            cursor.chars().next()
        );
    }
    *cursor = &cursor[1..]; // skip '='
    let value = parse_value(cursor)?;
    Ok((key, value))
}

/// Consume a variable name: `[a-zA-Z_][a-zA-Z0-9_-]*`.
fn parse_variable(cursor: &mut &str) -> Result<String> {
    let end = cursor
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .unwrap_or(cursor.len());
    if end == 0 {
        bail!(
            "expected variable name, got {:?}",
            cursor.chars().next()
        );
    }
    let name = cursor[..end].to_string();
    *cursor = &cursor[end..];
    Ok(name)
}

/// Parse a value: c-string `"..."`, tuple `{...}`, or list `[...]`.
fn parse_value(cursor: &mut &str) -> Result<MiValue> {
    match cursor.chars().next() {
        Some('"') => {
            let s = parse_cstring(cursor)?;
            Ok(MiValue::Const(s))
        }
        Some('{') => parse_tuple(cursor),
        Some('[') => parse_list(cursor),
        other => bail!("expected value, got {:?}", other),
    }
}

/// Parse a c-string: opening `"`, body with escapes, closing `"`.
fn parse_cstring(cursor: &mut &str) -> Result<String> {
    if !cursor.starts_with('"') {
        bail!("expected '\"' to start c-string");
    }
    *cursor = &cursor[1..]; // skip opening quote

    let mut result = String::new();
    loop {
        if cursor.is_empty() {
            bail!("unterminated c-string");
        }
        let ch = cursor.as_bytes()[0];
        match ch {
            b'"' => {
                *cursor = &cursor[1..];
                return Ok(result);
            }
            b'\\' => {
                if cursor.len() < 2 {
                    bail!("trailing backslash in c-string");
                }
                let esc = cursor.as_bytes()[1];
                match esc {
                    b'\\' => {
                        result.push('\\');
                        *cursor = &cursor[2..];
                    }
                    b'"' => {
                        result.push('"');
                        *cursor = &cursor[2..];
                    }
                    b'n' => {
                        result.push('\n');
                        *cursor = &cursor[2..];
                    }
                    b't' => {
                        result.push('\t');
                        *cursor = &cursor[2..];
                    }
                    b'r' => {
                        result.push('\r');
                        *cursor = &cursor[2..];
                    }
                    b'a' => {
                        result.push('\x07');
                        *cursor = &cursor[2..];
                    }
                    b'b' => {
                        result.push('\x08');
                        *cursor = &cursor[2..];
                    }
                    b'f' => {
                        result.push('\x0C');
                        *cursor = &cursor[2..];
                    }
                    b'0'..=b'7' => {
                        // Octal escape: 1-3 octal digits.
                        *cursor = &cursor[1..]; // skip the backslash
                        let mut val: u32 = 0;
                        let mut count = 0;
                        while count < 3 {
                            match cursor.as_bytes().first() {
                                Some(&b) if b >= b'0' && b <= b'7' => {
                                    val = val * 8 + (b - b'0') as u32;
                                    *cursor = &cursor[1..];
                                    count += 1;
                                }
                                _ => break,
                            }
                        }
                        if count == 0 {
                            bail!("invalid octal escape");
                        }
                        // Octal escapes in GDB MI are bytes, not unicode codepoints.
                        // Values > 127 are treated as raw bytes; we push the
                        // char if it is valid unicode, otherwise use the
                        // replacement character.
                        result.push(char::from_u32(val).unwrap_or('\u{FFFD}'));
                    }
                    _ => {
                        // Unknown escape -- keep the backslash and the char.
                        result.push('\\');
                        result.push(esc as char);
                        *cursor = &cursor[2..];
                    }
                }
            }
            _ => {
                // Fast-path: scan for the next special character.
                let end = cursor
                    .find(|c: char| c == '"' || c == '\\')
                    .unwrap_or(cursor.len());
                result.push_str(&cursor[..end]);
                *cursor = &cursor[end..];
            }
        }
    }
}

/// Parse a complete c-string from a standalone string (no surrounding context).
/// Used for stream records where the entire remainder is a quoted string.
fn parse_cstring_full(input: &str) -> Result<String> {
    let mut cursor = input;
    let s = parse_cstring(&mut cursor)?;
    // Allow trailing whitespace but nothing else.
    if !cursor.trim().is_empty() {
        bail!("trailing data after c-string: {:?}", cursor);
    }
    Ok(s)
}

/// Parse a tuple: `{` key=value [, key=value]* `}` or `{}`.
fn parse_tuple(cursor: &mut &str) -> Result<MiValue> {
    debug_assert!(cursor.starts_with('{'));
    *cursor = &cursor[1..]; // skip '{'

    let mut pairs = Vec::new();
    if cursor.starts_with('}') {
        *cursor = &cursor[1..];
        return Ok(MiValue::Tuple(pairs));
    }

    loop {
        let (key, value) = parse_result_pair(cursor)?;
        pairs.push((key, value));
        if cursor.starts_with(',') {
            *cursor = &cursor[1..];
        } else {
            break;
        }
    }

    if !cursor.starts_with('}') {
        bail!(
            "expected '}}' to close tuple, got {:?}",
            cursor.chars().next()
        );
    }
    *cursor = &cursor[1..];
    Ok(MiValue::Tuple(pairs))
}

/// Parse a list: `[` ... `]`.
///
/// A list may contain plain values (`[val,val,...]`) or key=value pairs
/// (`[key=val,key=val,...]`).  Empty lists (`[]`) are also valid.
fn parse_list(cursor: &mut &str) -> Result<MiValue> {
    debug_assert!(cursor.starts_with('['));
    *cursor = &cursor[1..]; // skip '['

    // Empty list.
    if cursor.starts_with(']') {
        *cursor = &cursor[1..];
        return Ok(MiValue::List(MiList::Empty));
    }

    // Peek ahead to decide if this is a value-list or a result-list.
    // If the first token looks like `identifier=`, it is a result-list.
    if is_result_list_start(cursor) {
        let mut pairs = Vec::new();
        loop {
            let (key, value) = parse_result_pair(cursor)?;
            pairs.push((key, value));
            if cursor.starts_with(',') {
                *cursor = &cursor[1..];
            } else {
                break;
            }
        }
        if !cursor.starts_with(']') {
            bail!(
                "expected ']' to close result list, got {:?}",
                cursor.chars().next()
            );
        }
        *cursor = &cursor[1..];
        Ok(MiValue::List(MiList::Results(pairs)))
    } else {
        let mut values = Vec::new();
        loop {
            let value = parse_value(cursor)?;
            values.push(value);
            if cursor.starts_with(',') {
                *cursor = &cursor[1..];
            } else {
                break;
            }
        }
        if !cursor.starts_with(']') {
            bail!(
                "expected ']' to close value list, got {:?}",
                cursor.chars().next()
            );
        }
        *cursor = &cursor[1..];
        Ok(MiValue::List(MiList::Values(values)))
    }
}

/// Peek at the cursor to decide whether we are at the start of a result-list
/// (i.e., the content begins with `variable=`).
fn is_result_list_start(cursor: &str) -> bool {
    // A variable starts with an alpha or underscore.
    let first = match cursor.chars().next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => c,
        _ => return false,
    };
    // Scan for the end of the identifier.
    let _ = first; // suppress unused warning
    let end = cursor
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
        .unwrap_or(cursor.len());
    // The character immediately after the identifier must be '='.
    cursor.as_bytes().get(end) == Some(&b'=')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gdb::mi_types::{MiBody, MiList, MiRecord, MiValue};

    // -- Helper to unwrap parse_line -----------------------------------------
    fn p(line: &str) -> MiRecord {
        parse_line(line).unwrap_or_else(|e| panic!("parse_line({:?}) failed: {}", line, e))
    }

    // -- Simple result -------------------------------------------------------

    #[test]
    fn simple_done() {
        assert_eq!(
            p("^done"),
            MiRecord::Result {
                token: None,
                class: "done".into(),
                body: vec![],
            }
        );
    }

    // -- Result with a value -------------------------------------------------

    #[test]
    fn result_with_value() {
        let rec = p("^done,value=\"42\"");
        match rec {
            MiRecord::Result { token, class, body } => {
                assert_eq!(token, None);
                assert_eq!(class, "done");
                assert_eq!(body.len(), 1);
                assert_eq!(body[0].0, "value");
                assert_eq!(body[0].1.as_const(), Some("42"));
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }

    // -- Tokenized result ----------------------------------------------------

    #[test]
    fn tokenized_result() {
        let rec = p("123^done,value=\"hello\"");
        match rec {
            MiRecord::Result { token, class, body } => {
                assert_eq!(token, Some(123));
                assert_eq!(class, "done");
                assert_eq!(MiBody::get_str(body.as_slice(), "value"), Some("hello"));
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }

    // -- Stopped async with nested tuple and empty list ----------------------

    #[test]
    fn stopped_async() {
        let line = r#"*stopped,reason="breakpoint-hit",bkptno="1",frame={addr="0x400520",func="main",args=[],file="test.c",line="10"}"#;
        let rec = p(line);
        match rec {
            MiRecord::AsyncExec { token, class, body } => {
                assert_eq!(token, None);
                assert_eq!(class, "stopped");
                assert_eq!(MiBody::get_str(body.as_slice(), "reason"), Some("breakpoint-hit"));
                assert_eq!(MiBody::get_str(body.as_slice(), "bkptno"), Some("1"));

                let frame = MiBody::get(body.as_slice(), "frame").expect("frame key");
                assert_eq!(frame.get_str("addr"), Some("0x400520"));
                assert_eq!(frame.get_str("func"), Some("main"));
                assert_eq!(frame.get_str("file"), Some("test.c"));
                assert_eq!(frame.get_str("line"), Some("10"));

                // args should be an empty list
                let args = frame.get("args").expect("args key");
                assert_eq!(args.as_list_values(), Some([].as_slice()));
            }
            other => panic!("expected AsyncExec, got {:?}", other),
        }
    }

    // -- Console stream ------------------------------------------------------

    #[test]
    fn console_stream() {
        let rec = p("~\"Hello world\\n\"");
        match rec {
            MiRecord::StreamConsole(s) => assert_eq!(s, "Hello world\n"),
            other => panic!("expected StreamConsole, got {:?}", other),
        }
    }

    // -- Log stream with escapes ---------------------------------------------

    #[test]
    fn log_stream_escapes() {
        // &"warning: \"quoted\"\n"
        // In MI output the backslashes and quotes are escaped, so the raw line
        // looks like:  &"warning: \\\"quoted\\\"\n"
        let rec = p(r#"&"warning: \\\"quoted\\\"\n""#);
        match rec {
            MiRecord::StreamLog(s) => {
                assert_eq!(s, "warning: \\\"quoted\\\"\n");
            }
            other => panic!("expected StreamLog, got {:?}", other),
        }
    }

    // -- Prompt --------------------------------------------------------------

    #[test]
    fn prompt() {
        assert_eq!(p("(gdb) "), MiRecord::Prompt);
        assert_eq!(p("(gdb)"), MiRecord::Prompt);
    }

    // -- Error result --------------------------------------------------------

    #[test]
    fn error_result() {
        let rec = p("^error,msg=\"No such file\"");
        match rec {
            MiRecord::Result { token, class, body } => {
                assert_eq!(token, None);
                assert_eq!(class, "error");
                assert_eq!(MiBody::get_str(body.as_slice(), "msg"), Some("No such file"));
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }

    // -- Nested tuples -------------------------------------------------------

    #[test]
    fn nested_tuples() {
        let line = r#"^done,bkpt={number="1",type="breakpoint",disp="keep",enabled="y",addr="0x400520",func="main",file="test.c",fullname="/home/user/test.c",line="5",times="0"}"#;
        let rec = p(line);
        match rec {
            MiRecord::Result { body, .. } => {
                let bkpt = MiBody::get(body.as_slice(), "bkpt").expect("bkpt key");
                assert_eq!(bkpt.get_str("number"), Some("1"));
                assert_eq!(bkpt.get_str("type"), Some("breakpoint"));
                assert_eq!(bkpt.get_str("addr"), Some("0x400520"));
                assert_eq!(bkpt.get_str("fullname"), Some("/home/user/test.c"));
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }

    // -- Lists of tuples (common pattern: list of frames, variables, etc.) ---

    #[test]
    fn list_of_tuples() {
        let line = r#"^done,stack=[frame={level="0",addr="0x400520",func="main"},frame={level="1",addr="0x7fff00",func="__libc_start_main"}]"#;
        let rec = p(line);
        match rec {
            MiRecord::Result { body, .. } => {
                let stack = MiBody::get(body.as_slice(), "stack").expect("stack key");
                match stack {
                    MiValue::List(MiList::Results(pairs)) => {
                        assert_eq!(pairs.len(), 2);
                        assert_eq!(pairs[0].0, "frame");
                        assert_eq!(pairs[0].1.get_str("level"), Some("0"));
                        assert_eq!(pairs[1].0, "frame");
                        assert_eq!(pairs[1].1.get_str("level"), Some("1"));
                    }
                    other => panic!("expected List(Results(...)), got {:?}", other),
                }
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }

    // -- Value-list (list of c-strings) --------------------------------------

    #[test]
    fn list_of_strings() {
        let line = r#"^done,files=["a.c","b.c","c.c"]"#;
        let rec = p(line);
        match rec {
            MiRecord::Result { body, .. } => {
                let files = MiBody::get(body.as_slice(), "files").expect("files key");
                let vals = files.as_list_values().expect("expected value list");
                assert_eq!(vals.len(), 3);
                assert_eq!(vals[0].as_const(), Some("a.c"));
                assert_eq!(vals[1].as_const(), Some("b.c"));
                assert_eq!(vals[2].as_const(), Some("c.c"));
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }

    // -- Notify async record -------------------------------------------------

    #[test]
    fn notify_async() {
        let rec = p("=thread-created,id=\"1\",group-id=\"i1\"");
        match rec {
            MiRecord::AsyncNotify { class, body } => {
                assert_eq!(class, "thread-created");
                assert_eq!(MiBody::get_str(body.as_slice(), "id"), Some("1"));
                assert_eq!(MiBody::get_str(body.as_slice(), "group-id"), Some("i1"));
            }
            other => panic!("expected AsyncNotify, got {:?}", other),
        }
    }

    // -- Status async record -------------------------------------------------

    #[test]
    fn status_async() {
        let rec = p("+download,section=\".text\",section-size=\"1024\"");
        match rec {
            MiRecord::AsyncStatus { class, body } => {
                assert_eq!(class, "download");
                assert_eq!(MiBody::get_str(body.as_slice(), "section"), Some(".text"));
            }
            other => panic!("expected AsyncStatus, got {:?}", other),
        }
    }

    // -- Target stream -------------------------------------------------------

    #[test]
    fn target_stream() {
        let rec = p("@\"target output\\n\"");
        match rec {
            MiRecord::StreamTarget(s) => assert_eq!(s, "target output\n"),
            other => panic!("expected StreamTarget, got {:?}", other),
        }
    }

    // -- Octal escape --------------------------------------------------------

    #[test]
    fn octal_escape() {
        // \101 is 'A' in octal
        let rec = p("~\"\\101\"");
        match rec {
            MiRecord::StreamConsole(s) => assert_eq!(s, "A"),
            other => panic!("expected StreamConsole, got {:?}", other),
        }
    }

    // -- MiBody trait --------------------------------------------------------

    #[test]
    fn mi_body_trait() {
        let body: Vec<(String, MiValue)> = vec![
            ("a".into(), MiValue::Const("1".into())),
            ("b".into(), MiValue::Const("2".into())),
        ];
        let slice: &[(String, MiValue)] = &body;
        assert_eq!(slice.get_str("a"), Some("1"));
        assert_eq!(slice.get_str("b"), Some("2"));
        assert_eq!(MiBody::get(slice, "c"), None);
    }

    // -- MiValue helpers -----------------------------------------------------

    #[test]
    fn mi_value_helpers() {
        let val = MiValue::Const("hello".into());
        assert_eq!(val.as_const(), Some("hello"));
        assert_eq!(val.as_tuple(), None);
        assert_eq!(val.as_list_values(), None);

        let tuple = MiValue::Tuple(vec![
            ("x".into(), MiValue::Const("1".into())),
            ("y".into(), MiValue::Const("2".into())),
        ]);
        assert!(tuple.as_tuple().is_some());
        assert_eq!(tuple.get_str("x"), Some("1"));
        assert_eq!(tuple.get_str("z"), None);
    }

    // -- Empty tuple ---------------------------------------------------------

    #[test]
    fn empty_tuple() {
        let line = "^done,value={}";
        let rec = p(line);
        match rec {
            MiRecord::Result { body, .. } => {
                let val = MiBody::get(body.as_slice(), "value").expect("value key");
                assert_eq!(val.as_tuple(), Some([].as_slice()));
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }

    // -- Multiple key-value pairs in body ------------------------------------

    #[test]
    fn multiple_body_pairs() {
        let line = r#"^done,key1="val1",key2="val2",key3="val3""#;
        let rec = p(line);
        match rec {
            MiRecord::Result { body, .. } => {
                assert_eq!(body.len(), 3);
                assert_eq!(MiBody::get_str(body.as_slice(), "key1"), Some("val1"));
                assert_eq!(MiBody::get_str(body.as_slice(), "key2"), Some("val2"));
                assert_eq!(MiBody::get_str(body.as_slice(), "key3"), Some("val3"));
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }

    // -- Running result with token -------------------------------------------

    #[test]
    fn running_with_token() {
        let rec = p("42^running");
        match rec {
            MiRecord::Result { token, class, body } => {
                assert_eq!(token, Some(42));
                assert_eq!(class, "running");
                assert!(body.is_empty());
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }

    // -- Complex nested structure (register values) --------------------------

    #[test]
    fn register_values() {
        let line = r#"^done,register-values=[{number="0",value="0x7fffffffe380"},{number="1",value="0x0"}]"#;
        let rec = p(line);
        match rec {
            MiRecord::Result { body, .. } => {
                let regs = MiBody::get(body.as_slice(), "register-values").expect("register-values key");
                let vals = regs.as_list_values().expect("expected value list");
                assert_eq!(vals.len(), 2);
                assert_eq!(vals[0].get_str("number"), Some("0"));
                assert_eq!(vals[0].get_str("value"), Some("0x7fffffffe380"));
                assert_eq!(vals[1].get_str("number"), Some("1"));
                assert_eq!(vals[1].get_str("value"), Some("0x0"));
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }

    #[test]
    fn breakpoint_table() {
        let line = r#"^done,BreakpointTable={nr_rows="2",nr_cols="6",hdr=[{width="7",alignment="-1",col_name="number",colhdr="Num"}],body=[bkpt={number="1",type="breakpoint",enabled="y",func="main",file="test.c",line="8",times="0",original-location="main"},bkpt={number="2",type="breakpoint",enabled="y",func="add",file="test.c",line="4",times="0",original-location="add"}]}"#;
        let record = parse_line(line).unwrap();
        match record {
            MiRecord::Result { body, .. } => {
                let table = MiBody::get(body.as_slice(), "BreakpointTable").unwrap();
                let bkpt_body = table.get("body").unwrap();
                match bkpt_body {
                    MiValue::List(MiList::Results(pairs)) => {
                        assert_eq!(pairs.len(), 2);
                        assert_eq!(pairs[0].0, "bkpt");
                        assert_eq!(pairs[0].1.get_str("number"), Some("1"));
                        assert_eq!(pairs[0].1.get_str("func"), Some("main"));
                        assert_eq!(pairs[1].0, "bkpt");
                        assert_eq!(pairs[1].1.get_str("number"), Some("2"));
                        assert_eq!(pairs[1].1.get_str("func"), Some("add"));
                    }
                    other => panic!("expected Results list, got {:?}", other),
                }
            }
            other => panic!("expected Result, got {:?}", other),
        }
    }
}
