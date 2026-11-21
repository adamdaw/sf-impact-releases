//! The textual tier: every whole-word, case-insensitive occurrence of a
//! declared name in a file, comments and string literals included. This is
//! the grep a careful reviewer would run by hand, done once for every name.
//! A file that cannot be searched is reported, not passed over.

use sf_impact_common::config::Config;
use sf_impact_common::rows::fold;
use std::collections::HashSet;

/// The names to look for, case-folded.
// Keys come from untrusted input; seed the hasher against collision floods.
pub struct Names(HashSet<String, foldhash::fast::RandomState>);

impl Names {
    /// Declared names shorter than the configured minimum are left out
    /// unless the allow-list names them: short names match too much.
    pub fn new<'a>(declared: impl IntoIterator<Item = &'a str>, config: &Config) -> Names {
        let allowed: HashSet<String> = config.textual_allow_list.iter().map(|n| fold(n)).collect();
        let names = declared
            .into_iter()
            .map(fold)
            .filter(|name| name.len() >= config.textual_min_length || allowed.contains(name))
            .collect();
        Names(names)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Match {
    /// The word as written in the file.
    pub word: String,
    pub line: u32,
    /// One-based byte offset within the line.
    pub column: u32,
}

/// Decode searchable text using the common rule. On refusal, preserve the
/// original bytes so the parsed tier can recover declarations while the
/// textual tier reports a skipped file.
pub(crate) fn searchable(bytes: Vec<u8>) -> Result<Vec<u8>, Vec<u8>> {
    sf_impact_common::text(&bytes)
        .map(String::into_bytes)
        .ok_or(bytes)
}

/// Occurrences of the names in `bytes`, which are text: see [`sf_impact_common::text`].
/// A word is a run of ASCII letters, digits and underscores, which is what
/// an Apex identifier is made of.
pub fn matches(bytes: &[u8], names: &Names) -> Vec<Match> {
    let is_word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let mut found = Vec::new();
    let mut folded = String::new();
    let (mut line, mut line_start, mut at) = (1, 0, 0);
    while at < bytes.len() {
        if !is_word(bytes[at]) {
            if bytes[at] == b'\n' {
                line += 1;
                line_start = at + 1;
            }
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() && is_word(bytes[at]) {
            at += 1;
        }
        let word = &bytes[start..at];
        folded.clear();
        folded.extend(word.iter().map(|byte| byte.to_ascii_lowercase() as char));
        if names.0.contains(&folded) {
            found.push(Match {
                // A word is ASCII by construction.
                word: String::from_utf8_lossy(word).into_owned(),
                line,
                column: (start - line_start) as u32 + 1,
            });
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(declared: &[&str]) -> Names {
        Names::new(declared.iter().copied(), &Config::default())
    }

    fn words(text: &str, names: &Names) -> Vec<String> {
        let found = matches(text.as_bytes(), names);
        found.into_iter().map(|m| m.word).collect()
    }

    #[test]
    fn a_name_matches_whatever_its_case_and_keeps_the_case_it_was_written_in() {
        let found = words("new MATHUTIL(); mathUtil.go()", &names(&["MathUtil"]));

        assert_eq!(found, ["MATHUTIL", "mathUtil"]);
    }

    #[test]
    fn match_order_follows_text_not_declaration_order() {
        let text = "Circle Square Circle";
        let forward = names(&["Circle", "Square"]);
        let reverse = names(&["Square", "Circle"]);

        assert_eq!(
            matches(text.as_bytes(), &forward),
            matches(text.as_bytes(), &reverse)
        );
        assert_eq!(words(text, &forward), ["Circle", "Square", "Circle"]);
    }

    #[test]
    fn only_whole_words_match() {
        let found = words(
            "MathUtilTest MyMathUtil Math_Util 'MathUtil'",
            &names(&["MathUtil"]),
        );

        assert_eq!(found, ["MathUtil"]);
    }

    #[test]
    fn names_shorter_than_the_minimum_are_skipped_unless_allowed() {
        let config = Config {
            textual_allow_list: vec!["money".into()],
            ..Config::default()
        };
        let names = Names::new(["Money", "Shape", "Circle"], &config);

        let found = words("Money Shape Circle", &names);

        assert_eq!(found, ["Money", "Circle"]);
    }

    #[test]
    fn a_match_is_located_by_line_and_byte_column() {
        let found = matches(b"first\n  x = Circle.area();\n", &names(&["Circle"]));

        assert_eq!(
            found,
            [Match {
                word: "Circle".into(),
                line: 2,
                column: 7
            }]
        );
    }

    fn utf16(text: &str, bom: [u8; 2], to_bytes: fn(u16) -> [u8; 2]) -> Vec<u8> {
        let units = text.encode_utf16().flat_map(to_bytes);
        bom.into_iter().chain(units).collect()
    }

    /// Regression: a permission set saved as UTF-16.
    #[test]
    fn utf16_text_with_a_byte_order_mark_is_searched() {
        let xml =
            "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\n<apexClass>ServiceTarget</apexClass>";
        let names = names(&["ServiceTarget"]);

        for bytes in [
            utf16(xml, [0xFF, 0xFE], u16::to_le_bytes),
            utf16(xml, [0xFE, 0xFF], u16::to_be_bytes),
        ] {
            let Ok(text) = searchable(bytes) else {
                panic!("UTF-16 with a byte order mark is text");
            };

            let found = matches(&text, &names);

            assert_eq!(found.len(), 1);
            assert_eq!(
                (found[0].word.as_str(), found[0].line),
                ("ServiceTarget", 2)
            );
        }
    }

    #[test]
    fn a_file_that_cannot_be_searched_is_told_apart_from_one_with_no_matches() {
        let text = b"no names here".to_vec();
        let image = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR Circle".to_vec();
        let unmarked: Vec<u8> = "Circle".encode_utf16().flat_map(u16::to_le_bytes).collect();
        let utf32: Vec<u8> = [0xFEFF, 0x43]
            .into_iter()
            .flat_map(u32::to_le_bytes)
            .collect();

        assert_eq!(searchable(text.clone()), Ok(text));
        assert_eq!(searchable(image.clone()), Err(image));
        assert_eq!(searchable(unmarked.clone()), Err(unmarked));
        assert_eq!(searchable(utf32.clone()), Err(utf32));
    }

    #[test]
    fn non_ascii_text_around_a_name_does_not_hide_it() {
        let found = words(
            "// r\u{e9}sum\u{e9} Circle\u{2014}Circle",
            &names(&["Circle"]),
        );

        assert_eq!(found, ["Circle", "Circle"]);
    }
}
