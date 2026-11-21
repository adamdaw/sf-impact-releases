//! Lines of text: what a hunk is, and the comparison of two versions of a
//! file line by line. `sf-impact-delta` compares the two versions of a file that
//! changed in place; `sf-impact-changed` compares the two files that a symbol
//! moved between. Both mean the same thing by a hunk, so both compare here.

use imara_diff::{Algorithm, Diff, InternedInput};
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// A run of lines that differ: `@@ -base_start,base_lines +head_start,head_lines @@`.
/// Where a side has no lines, its start is the line the other side's lines
/// come after.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hunk {
    pub base_start: u32,
    pub base_lines: u32,
    pub head_start: u32,
    pub head_lines: u32,
}

/// The hunks between two versions of a text, without context lines.
///
/// A line is everything up to and including its `\n`, and two lines are the
/// same only if they are the same bytes. So nothing is ignored: a line that
/// ends in CRLF is not the line that ends in LF, and a last line without a
/// line end is not that line with one.
///
/// The comparison is Myers' with the indent heuristic, which is what git
/// does by default: where a run of added or removed lines could sit in
/// more than one place, it sits where the indentation says a block starts.
pub fn hunks(base: &[u8], head: &[u8]) -> Vec<Hunk> {
    let input = InternedInput::new(base, head);
    let mut diff = Diff::compute(Algorithm::Myers, &input);
    diff.postprocess_lines(&input);
    // Lines count from 1, and a side without lines starts at the line before.
    let side = |lines: Range<u32>| match lines.end - lines.start {
        0 => (lines.start, 0),
        count => (lines.start + 1, count),
    };
    diff.hunks()
        .map(|hunk| {
            let (base_start, base_lines) = side(hunk.before);
            let (head_start, head_lines) = side(hunk.after);
            Hunk {
                base_start,
                base_lines,
                head_start,
                head_lines,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(base_start: u32, base_lines: u32, head_start: u32, head_lines: u32) -> Hunk {
        Hunk {
            base_start,
            base_lines,
            head_start,
            head_lines,
        }
    }

    fn compare(base: &str, head: &str) -> Vec<Hunk> {
        hunks(base.as_bytes(), head.as_bytes())
    }

    #[test]
    fn a_changed_line_is_a_hunk_of_one_line_on_each_side() {
        assert_eq!(compare("a\nb\nc\n", "a\nB\nc\n"), [hunk(2, 1, 2, 1)]);
    }

    #[test]
    fn a_side_without_lines_starts_at_the_line_the_others_come_after() {
        assert_eq!(compare("a\nc\n", "a\nb\nc\n"), [hunk(1, 0, 2, 1)]);
        assert_eq!(compare("a\nb\nc\n", "a\nc\n"), [hunk(2, 1, 1, 0)]);
        // At the top of the file that is line 0.
        assert_eq!(compare("b\n", "a\nb\n"), [hunk(0, 0, 1, 1)]);
        assert_eq!(compare("a\nb\n", "b\n"), [hunk(1, 1, 0, 0)]);
    }

    #[test]
    fn changes_apart_are_hunks_apart_and_the_same_text_has_none() {
        let hunks = compare("a\nb\nc\nd\ne\n", "A\nb\nc\nd\nE\nf\n");

        assert_eq!(hunks, [hunk(1, 1, 1, 1), hunk(5, 1, 5, 2)]);
        assert_eq!(compare("a\nb\n", "a\nb\n"), []);
        assert_eq!(compare("", ""), []);
    }

    #[test]
    fn a_line_end_is_part_of_its_line() {
        // CRLF to LF: every line is another line.
        assert_eq!(compare("a\r\nb\r\n", "a\nb\n"), [hunk(1, 2, 1, 2)]);
        // A last line that gains its line end.
        assert_eq!(compare("a\nb", "a\nb\n"), [hunk(2, 1, 2, 1)]);
        // A carriage return alone ends no line.
        assert_eq!(compare("a\rb\n", "a\rB\n"), [hunk(1, 1, 1, 1)]);
    }

    /// The method could as well be read as added from the `}` of the one
    /// before it to its own last line but one. Read so, it would change
    /// the line the method before it ends on.
    #[test]
    fn an_added_block_is_the_block_not_the_end_of_one_and_the_start_of_the_next() {
        let method = |name: &str| format!("    void {name}() {{\n        go();\n    }}\n");
        let class = |methods: &[&str]| {
            let methods: Vec<String> = methods.iter().map(|name| method(name)).collect();
            format!("class A {{\n{}}}\n", methods.join("\n"))
        };

        let hunks = compare(
            &class(&["first", "third"]),
            &class(&["first", "second", "third"]),
        );

        // `second` whole, then the blank line after it: as git has it.
        assert_eq!(hunks, [hunk(5, 0, 6, 4)]);
    }

    #[test]
    fn bytes_that_are_not_text_in_any_encoding_are_compared_as_they_are() {
        assert_eq!(
            hunks(b"\xFF\xFE\n\x80\n", b"\xFF\xFE\n\x81\n"),
            [hunk(2, 1, 2, 1)]
        );
    }
}
