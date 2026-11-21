//! Text from a repository as standard error shows it. A path or a string
//! of a pack can hold a line feed, which would start a line of its own (a
//! CI runner takes a line that starts `::` as a command), or a terminal
//! escape. Either is written as an escape instead: `\n`, `\u{1b}`.

/// `text` with every control character, and every character that turns
/// or ends a line of text, escaped: for one value in a message.
pub fn shown(text: &str) -> String {
    escaped(text, |_| true)
}

/// A whole message, as [`shown`] but keeping its tabs. A message is one
/// line: a line feed in it, from a value or from anywhere, is escaped, so
/// that no value can start a line of its own. What goes on after a message
/// (the usage) is printed apart from it.
pub fn said(message: &str) -> String {
    escaped(message, |c| c != '\t')
}

fn escaped(text: &str, escapes: impl Fn(char) -> bool) -> String {
    if !text.chars().any(|c| troubles(c) && escapes(c)) {
        return text.to_string();
    }
    let mut shown = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            _ if !(troubles(c) && escapes(c)) => shown.push(c),
            '\n' => shown.push_str("\\n"),
            '\r' => shown.push_str("\\r"),
            '\t' => shown.push_str("\\t"),
            _ => shown.push_str(&format!("\\u{{{:x}}}", c as u32)),
        }
    }
    shown
}

/// A control character (C0, DEL, C1), or one that ends a line or turns
/// the direction of the text.
fn troubles(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_with_nothing_to_escape_is_as_it_was() {
        let plain = "app/classes/Ünïcode Service.cls: `x` \"quoted\" \\ back";

        assert_eq!(shown(plain), plain);
        assert_eq!(said(plain), plain);
        assert_eq!(said("a tab\tis kept"), "a tab\tis kept");
    }

    #[test]
    fn a_value_cannot_start_a_line_or_write_a_terminal_escape() {
        let path =
            "app/A\n::error title=x::y\r\n::stop-commands::t\u{1b}]8;;u\u{7}\u{9b}\u{7f}.cls";

        let shown = shown(path);

        assert_eq!(
            shown,
            "app/A\\n::error title=x::y\\r\\n::stop-commands::t\\u{1b}]8;;u\\u{7}\\u{9b}\\u{7f}.cls"
        );
        assert!(!shown.chars().any(char::is_control));
    }

    #[test]
    fn a_message_keeps_its_lines_and_loses_its_escapes() {
        let message = "first\n::error::x\u{1b}[2K\u{1b}[1Asecond\u{202e}\u{2028}";

        assert_eq!(
            said(message),
            "first\\n::error::x\\u{1b}[2K\\u{1b}[1Asecond\\u{202e}\\u{2028}"
        );
        assert_eq!(
            shown("a\u{2029}b\u{2066}c\u{061c}"),
            "a\\u{2029}b\\u{2066}c\\u{61c}"
        );
    }
}
