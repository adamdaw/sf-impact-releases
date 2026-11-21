//! A small flat XML tree. Offsets refer to the original UTF-8 source, including
//! entity and CDATA spelling. No DTDs or external entities are expanded.
use quick_xml::{Reader, events::Event};

#[derive(Default)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub text: String,
    pub offsets: Vec<usize>,
    pub attributes: Vec<(String, String, usize)>,
    pub start: usize,
}
impl Node {
    pub fn value(&self) -> &str {
        self.text.trim()
    }
    pub fn offset(&self) -> usize {
        self.at(self.text.len() - self.text.trim_start().len())
    }
    pub fn at(&self, index: usize) -> usize {
        self.offsets.get(index).copied().unwrap_or(self.start)
    }
    fn append(&mut self, text: &str, offset: usize, literal: bool) {
        self.text.push_str(text);
        self.offsets
            .extend((0..text.len()).map(|i| offset + if literal { i } else { 0 }));
    }
}
#[derive(Default)]
pub struct Document {
    pub nodes: Vec<Node>,
    pub errors: Vec<usize>,
}
impl Document {
    pub fn child(&self, node: usize, name: &str) -> Option<&Node> {
        self.nodes[node]
            .children
            .iter()
            .map(|i| &self.nodes[*i])
            .find(|n| n.name == name)
    }
    pub fn value(&self, node: usize, name: &str) -> Option<&str> {
        self.child(node, name)
            .map(Node::value)
            .filter(|s| !s.is_empty())
    }
}
fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or_default().into()
}

pub fn parse(source: &str, root: &str) -> Document {
    let mut doc = Document::default();
    let mut reader = Reader::from_str(source);
    reader.config_mut().check_comments = true;
    let mut stack: Vec<usize> = Vec::new();
    let mut roots = 0;
    loop {
        let offset = reader.buffer_position() as usize;
        let event = reader.read_event();
        match event {
            Ok(Event::Start(_)) | Ok(Event::Empty(_)) if stack.len() >= crate::XML_DEPTH => {
                doc.errors.push(offset);
                break;
            }
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = local(e.name().as_ref());
                if stack.is_empty() {
                    roots += 1;
                    if roots != 1 || name != root {
                        doc.errors.push(offset);
                    }
                }
                let mut node = Node {
                    name,
                    parent: stack.last().copied(),
                    start: offset,
                    ..Node::default()
                };
                for attr in e.attributes() {
                    match attr {
                        Ok(attr) => match quick_xml::escape::unescape(&attr.value) {
                            Ok(value) => {
                                // Reader::from_str borrows attribute bytes from this input,
                                // so pointer subtraction gives their original source offset.
                                let at = attr.value.as_ptr() as usize - source.as_ptr() as usize;
                                node.attributes.push((
                                    local(attr.key.as_ref()),
                                    value.into_owned(),
                                    at,
                                ));
                            }
                            Err(_) => doc.errors.push(offset),
                        },
                        Err(_) => doc.errors.push(offset),
                    }
                }
                let id = doc.nodes.len();
                if let Some(parent) = node.parent {
                    doc.nodes[parent].children.push(id);
                }
                doc.nodes.push(node);
                if matches!(event, Ok(Event::Start(_))) {
                    stack.push(id);
                }
            }
            Ok(Event::End(_)) => {
                stack.pop();
            }
            Ok(Event::Text(e)) => {
                let text = &*e;
                {
                    if let Some(id) = stack.last() {
                        doc.nodes[*id].append(text, offset, true);
                    } else if !text.trim().is_empty() {
                        doc.errors.push(offset);
                    }
                }
            }
            Ok(Event::CData(e)) => {
                let text = &*e;
                {
                    if let Some(id) = stack.last() {
                        doc.nodes[*id].append(text, offset + 9, true);
                    } else {
                        doc.errors.push(offset);
                    }
                }
            }
            Ok(Event::GeneralRef(e)) => {
                let decoded = quick_xml::escape::unescape(&format!("&{};", &*e))
                    .ok()
                    .map(|s| s.into_owned());
                match (stack.last(), decoded) {
                    (Some(id), Some(text)) => doc.nodes[*id].append(&text, offset, false),
                    _ => doc.errors.push(offset),
                }
            }
            Ok(Event::DocType(_)) => doc.errors.push(offset),
            Ok(Event::Eof) => {
                if roots == 0 || !stack.is_empty() {
                    doc.errors.push(offset);
                }
                break;
            }
            Err(_) => {
                doc.errors.push(reader.error_position() as usize);
                break;
            }
            _ => {}
        }
    }
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fmt::Write,
        time::{Duration, Instant},
    };

    #[test]
    fn an_element_nested_past_the_limit_is_an_error_where_it_starts() {
        let deep = |n: usize| format!("<Flow>{}{}</Flow>", "<a>".repeat(n), "</a>".repeat(n));

        let document = parse(&deep(100_000), "Flow");

        // `<Flow>` is the first level.
        assert_eq!(document.errors, [6 + 3 * (crate::XML_DEPTH - 1)]);
        assert_eq!(document.nodes.len(), crate::XML_DEPTH);
        assert!(parse(&deep(crate::XML_DEPTH - 1), "Flow").errors.is_empty());
    }

    #[test]
    fn the_depth_limit_is_one_thousand_levels() {
        // `<Flow>` is level 1, so 999 `<a>` make 1,000 levels.
        let deep = |n: usize| format!("<Flow>{}{}</Flow>", "<a>".repeat(n), "</a>".repeat(n));

        assert!(parse(&deep(999), "Flow").errors.is_empty());
        assert!(!parse(&deep(1000), "Flow").errors.is_empty());
    }

    #[test]
    fn many_distinct_attributes_are_checked_in_bounded_time() {
        // RUSTSEC-2026-0194: one tag with many distinct attribute names.
        let mut source = String::from("<root");
        for i in 0..200_000 {
            write!(source, " a{i}=\"v\"").unwrap();
        }
        source.push_str("/>");
        let start = Instant::now();
        let doc = parse(&source, "root");
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(doc.errors.is_empty());
        assert_eq!(doc.nodes[0].attributes.len(), 200_000);
    }
}
