//! Getting a Rust string safely into a snippet of JavaScript.
//!
//! Every string the app hands to `evaluate_javascript` goes through here rather than being pasted
//! into the script. The strings involved are ours — a stylesheet, an element id, a file name — but
//! "ours" is exactly the assumption that stops being true the day a document's own title or a
//! user's font name contains a quote. Interpolating raw text into a script is a habit worth not
//! having, and it costs twenty lines not to.

/// A JavaScript string literal, quotes included.
pub fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            // `</script` inside a literal would end an inline script element. We only ever evaluate
            // detached snippets, but the day one is written into a page this is the bug.
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            // U+2028/2029 are line terminators in older JavaScript, so a literal one splits the
            // statement in two.
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_just_quoted() {
        assert_eq!(string("hello"), "\"hello\"");
    }

    #[test]
    fn quotes_and_backslashes_cannot_end_the_literal() {
        assert_eq!(string(r#"a"b"#), r#""a\"b""#);
        assert_eq!(string(r"a\b"), r#""a\\b""#);
    }

    #[test]
    fn newlines_do_not_split_the_statement() {
        assert_eq!(string("a\nb"), r#""a\nb""#);
        assert_eq!(string("a\r\nb"), r#""a\r\nb""#);
        assert_eq!(string("a\u{2028}b"), r#""a\u2028b""#);
    }

    #[test]
    fn a_closing_script_tag_is_neutralised() {
        assert!(!string("</script>").contains('<'));
    }

    #[test]
    fn control_characters_are_escaped_rather_than_embedded() {
        assert_eq!(string("a\u{1}b"), r#""a\u0001b""#);
    }

    #[test]
    fn a_realistic_stylesheet_survives() {
        let css = "body { font-family: \"My Font\"; }\n/* note */\n";
        let literal = string(css);
        assert!(literal.starts_with('"') && literal.ends_with('"'));
        assert!(!literal[1..literal.len() - 1].contains('\n'));
    }
}
