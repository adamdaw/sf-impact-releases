//! A metadata file as a small tree of elements that remember where in the
//! file they and their text are, so that a row can point at a line.
//!
//! A file that is not well-formed yields the tree up to the error and the
//! place of the error. What was read before it is still worth reporting.

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

pub struct Document {
    /// The root element; an element without a name if the file has none.
    pub root: Element,
    /// Byte offset of the first error, if the file is not well-formed.
    pub error: Option<usize>,
    /// Byte offset at which each line starts.
    lines: Vec<usize>,
}

#[derive(Default)]
pub struct Element {
    /// Without a namespace prefix.
    pub name: String,
    pub children: Vec<Element>,
    /// The text directly inside the element, with entities resolved.
    pub text: String,
    /// Where each run of `text` is in the file: (offset in `text`, offset in
    /// the file). An entity is one character of text and several bytes of file.
    runs: Vec<(usize, usize)>,
    /// Byte offsets of the start tag and of the end of the end tag.
    pub start: usize,
    pub end: usize,
}

impl Element {
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> {
        self.children.iter().filter(move |child| child.name == name)
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|child| child.name == name)
    }

    /// The text without the white space around it, and where it starts in
    /// the file.
    pub fn trimmed(&self) -> (&str, usize) {
        let leading = self.text.len() - self.text.trim_start().len();
        (self.text.trim(), self.offset_of(leading))
    }

    /// Where in the file the byte of `text` at `at` is.
    pub fn offset_of(&self, at: usize) -> usize {
        let run = self.runs.iter().rev().find(|(in_text, _)| *in_text <= at);
        run.map_or(self.start, |(in_text, in_file)| in_file + (at - in_text))
    }

    fn push_text(&mut self, text: &str, in_file: usize) {
        self.runs.push((self.text.len(), in_file));
        self.text.push_str(text);
    }
}

impl Document {
    /// `source` is text: see `sf_impact_common::text`.
    pub fn parse(source: &[u8]) -> Document {
        // The reader would drop a byte order mark without counting it.
        let base = match source.starts_with(UTF8_BOM) {
            true => UTF8_BOM.len(),
            false => 0,
        };
        let mut reader = Reader::from_reader(&source[base..]);
        // Open elements, outermost first. The first holds the root.
        let mut open = vec![Element::default()];
        let mut error = None;
        loop {
            let at = base + reader.buffer_position() as usize;
            let event = match reader.read_event() {
                Ok(event) => event,
                Err(_) => {
                    error = Some(base + reader.error_position() as usize);
                    break;
                }
            };
            let after = base + reader.buffer_position() as usize;
            // `open` holds the holder too: it is as long as a tag's element
            // would be deep.
            if matches!(event, Event::Start(_) | Event::Empty(_)) && open.len() > crate::XML_DEPTH {
                error = Some(at);
                break;
            }
            let innermost = open.last_mut().expect("the holder is never closed");
            match event {
                Event::Start(tag) => open.push(element(&tag, at)),
                Event::Empty(tag) => {
                    let mut empty = element(&tag, at);
                    empty.end = after;
                    innermost.children.push(empty);
                }
                Event::End(_) => {
                    // The reader has checked that the names match.
                    let mut closed = open.pop().expect("an end tag follows a start tag");
                    closed.end = after;
                    open.last_mut()
                        .expect("the holder is never closed")
                        .children
                        .push(closed);
                }
                Event::Text(text) => innermost.push_text(&text, at),
                Event::CData(data) => {
                    innermost.push_text(&data, at + "<![CDATA[".len());
                }
                Event::GeneralRef(entity) => {
                    let name = &*entity;
                    let resolved = match entity.resolve_char_ref() {
                        Ok(Some(character)) => character.to_string(),
                        _ => match name {
                            "amp" => "&".into(),
                            "lt" => "<".into(),
                            "gt" => ">".into(),
                            "apos" => "'".into(),
                            "quot" => "\"".into(),
                            // Not one of XML's own: kept as it is written.
                            other => format!("&{other};"),
                        },
                    };
                    innermost.push_text(&resolved, at);
                }
                Event::Eof => {
                    if open.len() > 1 {
                        error = Some(source.len());
                    }
                    break;
                }
                Event::Comment(_) | Event::Decl(_) | Event::PI(_) | Event::DocType(_) => {}
            }
        }

        // Whatever is still open ends where reading stopped.
        while open.len() > 1 {
            let mut unclosed = open.pop().expect("more than one is open");
            unclosed.end = source.len();
            open.last_mut()
                .expect("the holder is below it")
                .children
                .push(unclosed);
        }
        let holder = open.pop().expect("the holder is never closed");
        let root = holder.children.into_iter().next();
        if root.is_none() && error.is_none() {
            error = Some(0);
        }
        let newlines = source
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == b'\n');
        Document {
            root: root.unwrap_or_default(),
            error,
            lines: [0]
                .into_iter()
                .chain(newlines.map(|(at, _)| at + 1))
                .collect(),
        }
    }

    /// Line and column of a byte offset, both counting from 1.
    pub fn position(&self, offset: usize) -> (u32, u32) {
        let line = self.lines.partition_point(|start| *start <= offset);
        (line as u32, (offset - self.lines[line - 1]) as u32 + 1)
    }

    pub fn line(&self, offset: usize) -> u32 {
        self.position(offset).0
    }
}

fn element(tag: &BytesStart, start: usize) -> Element {
    Element {
        name: tag.local_name().as_ref().to_owned(),
        start,
        ..Element::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_are_found_by_name_without_their_namespace_prefix() {
        let document =
            Document::parse(b"<a xmlns:m=\"urn:m\"><m:b>one</m:b><c/><m:b>two</m:b></a>");

        let texts: Vec<&str> = document
            .root
            .children_named("b")
            .map(|b| b.text.as_str())
            .collect();

        assert_eq!(texts, ["one", "two"]);
        assert_eq!(document.error, None);
    }

    #[test]
    fn text_is_located_by_line_and_column_past_the_white_space_around_it() {
        let document = Document::parse(b"<a>\n  <b>\n    value </b>\n</a>");
        let b = document.root.child("b").unwrap();

        let (text, offset) = b.trimmed();

        assert_eq!(text, "value");
        assert_eq!(document.position(offset), (3, 5));
    }

    #[test]
    fn an_entity_is_one_character_of_text_and_keeps_what_follows_it_in_place() {
        let source = b"<f>A &amp; &#39;B&apos; &lt; C</f>";
        let document = Document::parse(source);
        let f = &document.root;

        assert_eq!(f.text, "A & 'B' < C");
        // `C` is the eleventh character of the text and the 30th byte of the file.
        assert_eq!(f.text.find('C'), Some(10));
        assert_eq!(f.offset_of(10), 29);
        assert_eq!(source[29], b'C');
    }

    #[test]
    fn character_data_in_a_cdata_section_is_text() {
        let source = b"<f><![CDATA[A < B]]></f>";
        let document = Document::parse(source);

        assert_eq!(document.root.text, "A < B");
        assert_eq!(source[document.root.offset_of(4)], b'B');
    }

    #[test]
    fn an_element_knows_the_lines_it_spans() {
        let document = Document::parse(b"<a>\n<b>\n<c/>\n</b>\n</a>\n");
        let b = document.root.child("b").unwrap();

        assert_eq!((document.line(b.start), document.line(b.end - 1)), (2, 4));
    }

    #[test]
    fn a_mismatched_end_tag_is_an_error_that_keeps_what_was_read_before_it() {
        let document = Document::parse(b"<a>\n<b>kept</b>\n<c>lost</d>\n</a>");

        assert_eq!(document.error.map(|at| document.line(at)), Some(3));
        assert_eq!(
            document.root.child("b").map(|b| b.text.as_str()),
            Some("kept")
        );
    }

    #[test]
    fn a_file_that_ends_inside_an_element_is_an_error() {
        let document = Document::parse(b"<a>\n<b>text");

        assert_eq!(document.error, Some(11));
        assert_eq!(document.root.name, "a");
    }

    #[test]
    fn an_end_tag_that_nothing_opened_is_an_error_not_a_crash() {
        for source in [&b"</a>"[..], b"<a/></b>", b"<a></a></a>"] {
            let document = Document::parse(source);

            assert!(document.error.is_some(), "{source:?}");
        }
    }

    /// UTF-16 without a byte order mark is not text to this tool: the file
    /// gets a `skipped_file` row, and what is parsed here is its bytes.
    #[test]
    fn text_in_an_encoding_that_is_not_recognised_is_an_error_and_yields_no_elements() {
        let source: Vec<u8> = "<a><b>x</b></a>"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();

        let document = Document::parse(&source);

        assert!(document.error.is_some());
        assert!(document.root.child("b").is_none());
    }

    #[test]
    fn a_file_without_any_element_is_an_error() {
        for source in [&b""[..], b"not xml at all", b"\0\x01\x02"] {
            let document = Document::parse(source);

            assert!(document.error.is_some(), "{source:?}");
            assert_eq!(document.root.name, "");
        }
    }

    #[test]
    fn an_element_nested_past_the_limit_is_an_error_where_it_starts() {
        let deep = |n: usize| format!("{}{}", "<a>".repeat(n), "</a>".repeat(n));

        let document = Document::parse(deep(100_000).as_bytes());

        assert_eq!(document.error, Some(3 * crate::XML_DEPTH));
        assert!(
            Document::parse(deep(crate::XML_DEPTH).as_bytes())
                .error
                .is_none()
        );
    }

    #[test]
    fn the_depth_limit_is_one_thousand_levels() {
        let deep = |n: usize| format!("{}{}", "<a>".repeat(n), "</a>".repeat(n));

        assert!(Document::parse(deep(1000).as_bytes()).error.is_none());
        assert!(Document::parse(deep(1001).as_bytes()).error.is_some());
    }

    #[test]
    fn a_byte_order_mark_is_counted_in_the_offsets_that_follow_it() {
        let source = b"\xEF\xBB\xBF<a>x</a>";
        let document = Document::parse(source);

        let (text, offset) = document.root.trimmed();

        assert_eq!(text, "x");
        assert_eq!(source[offset], b'x');
    }

    /// quick-xml 0.42 (RUSTSEC-2026-0194) checks UTF-8 everywhere, not only
    /// in text: invalid bytes in an attribute, comment, CDATA section or
    /// name are a parse error, at the line they are on.
    #[test]
    fn invalid_utf8_outside_text_is_an_error_at_its_line() {
        let sources: [&[u8]; 4] = [
            b"<a>\n<b c=\"\xFF\"/>\n</a>",
            b"<a>\n<!-- \xFF -->\n</a>",
            b"<a>\n<![CDATA[\xFF]]>\n</a>",
            b"<a>\n<b\xFF/>\n</a>",
        ];
        for source in sources {
            let document = Document::parse(source);

            assert_eq!(
                document.error.map(|at| document.line(at)),
                Some(2),
                "{source:?}"
            );
        }
    }
}
