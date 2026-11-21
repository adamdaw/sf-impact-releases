//! Small helpers over the tree-sitter-sfapex syntax tree.

use sf_impact_common::types::normal;
use std::borrow::Cow;
use std::sync::LazyLock;
use tree_sitter::{Language, Node, Parser, Tree};

pub fn parser() -> Parser {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_sfapex::apex::LANGUAGE.into())
        .expect("the bundled Apex grammar matches the tree-sitter runtime");
    parser
}

pub fn parse(parser: &mut Parser, source: &[u8]) -> Tree {
    parser
        .parse(source, None)
        .expect("parsing without a timeout or cancellation always returns a tree")
}

/// The grammar's fields that are read by name here and in `refs`.
const FIELDS: [&str; 22] = [
    "alternative",
    "arguments",
    "array",
    "body",
    "condition",
    "consequence",
    "constructor",
    "declarator",
    "dimensions",
    "field",
    "left",
    "merge_with",
    "name",
    "object",
    "operand",
    "operator",
    "parameters",
    "right",
    "target",
    "type",
    "upsert_key",
    "value",
];

/// The ids of `FIELDS`, in order, found once: `Node::child_by_field_name`
/// finds a field's id by comparing its name with the grammar's on every call.
static FIELD_IDS: LazyLock<[u16; FIELDS.len()]> = LazyLock::new(|| {
    let language = Language::from(tree_sitter_sfapex::apex::LANGUAGE);
    FIELDS.map(|name| {
        language
            .field_id_for_name(name)
            .unwrap_or_else(|| panic!("the Apex grammar has a field named `{name}`"))
            .get()
    })
});

/// The child of `node` in the field named `field`, as
/// `Node::child_by_field_name` finds it.
pub fn by_field<'t>(node: Node<'t>, field: &str) -> Option<Node<'t>> {
    match FIELDS.iter().position(|known| *known == field) {
        Some(index) => node.child_by_field_id(FIELD_IDS[index]),
        None => node.child_by_field_name(field),
    }
}

pub fn text<'a>(node: Node, source: &'a [u8]) -> Cow<'a, str> {
    String::from_utf8_lossy(&source[node.byte_range()])
}

/// The named children of a node, without comments.
pub fn kids(node: Node) -> Vec<Node> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor)
        .filter(|child| !matches!(child.kind(), "line_comment" | "block_comment"))
        .collect()
}

pub fn line(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

/// One-based byte offset within the line.
pub fn column(node: Node) -> u32 {
    node.start_position().column as u32 + 1
}

pub fn is_type(kind: &str) -> bool {
    matches!(
        kind,
        "type_identifier"
            | "scoped_type_identifier"
            | "generic_type"
            | "array_type"
            | "annotated_type"
            | "java_type"
            // `boolean` written in lower case has a node kind of its own.
            | "boolean_type"
    )
}

/// A type as written, without whitespace: `Map<String,List<Outer.Inner>>`.
pub fn type_text(node: Node, source: &[u8]) -> String {
    text(node, source)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect()
}

/// The line of the first error or missing node, if the tree has an error.
pub fn errors(tree: &Tree) -> Option<u32> {
    let root = tree.root_node();
    error_line(root.has_error(), first_error(tree), line(root))
}

/// A tree that says it has an error is never clean: if no error or missing
/// node was found, the error is reported at the root's first line.
fn error_line(has_error: bool, first: Option<u32>, root: u32) -> Option<u32> {
    has_error.then(|| first.unwrap_or(root))
}

/// The line of the first error or missing node that the cursor reaches.
fn first_error(tree: &Tree) -> Option<u32> {
    let mut cursor = tree.walk();
    loop {
        let node = cursor.node();
        if node.is_error() || node.is_missing() {
            return Some(line(node));
        }
        // Only subtrees that contain an error need visiting.
        if node.has_error() && cursor.goto_first_child() {
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return None;
            }
        }
    }
}

/// The line of the first node, in source order, that is more than `limit`
/// levels below the root, if there is one. The tree is not recursed into:
/// how deep it is is what is in question.
pub fn too_deep(tree: &Tree, limit: usize) -> Option<u32> {
    let mut cursor = tree.walk();
    let mut depth = 0;
    loop {
        // A subtree of n nodes is at most n - 1 levels below its root, and
        // one that cannot reach past the limit is not looked into.
        let below = cursor.node().descendant_count() - 1;
        if depth + below > limit && cursor.goto_first_child() {
            depth += 1;
            if depth > limit {
                return Some(line(cursor.node()));
            }
            continue;
        }
        while !cursor.goto_next_sibling() {
            if !cursor.goto_parent() {
                return None;
            }
            depth -= 1;
        }
    }
}

/// `Owner.name(P1,P2)`: overloads are distinct declarations, so identity
/// includes canonical parameter types; owner and member spelling is preserved.
pub fn callable_symbol(owner: &str, name: &str, params: &[String]) -> String {
    format!(
        "{owner}.{name}({})",
        params
            .iter()
            .map(|p| normal(p))
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// The parameter types of a method or constructor declaration.
pub fn param_types(declaration: Node, source: &[u8]) -> Vec<String> {
    let Some(parameters) = by_field(declaration, "parameters") else {
        return Vec::new();
    };
    kids(parameters)
        .into_iter()
        .filter_map(|parameter| by_field(parameter, "type"))
        .map(|ty| type_text(ty, source))
        .collect()
}

pub fn field_text<'a>(node: Node, field: &str, source: &'a [u8]) -> Option<Cow<'a, str>> {
    by_field(node, field).map(|child| text(child, source))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `too_deep` without skipping any subtree.
    fn first_past(node: Node, depth: usize, limit: usize) -> Option<u32> {
        if depth > limit {
            return Some(line(node));
        }
        let mut cursor = node.walk();
        let children: Vec<Node> = node.children(&mut cursor).collect();
        children
            .into_iter()
            .find_map(|child| first_past(child, depth + 1, limit))
    }

    #[test]
    fn the_first_node_past_the_limit_is_the_one_a_full_walk_finds_first() {
        let sources = [
            "public class A {\n  Integer s = ((1));\n}\n",
            "public class A {\n  void m() {\n    if (x) { y(((2))); }\n  }\n  Integer t = (((((3)))));\n}\n",
            "public class A {\n  class B {\n    String s = 'a' + 'b' + 'c' + 'd';\n  }\n}\n",
            "public class A {\n  Integer s = ((((\n",
        ];
        for source in sources {
            let tree = parse(&mut parser(), source.as_bytes());
            let deepest = (0..).find(|limit| first_past(tree.root_node(), 0, *limit).is_none());
            for limit in 0..=deepest.unwrap() {
                assert_eq!(
                    too_deep(&tree, limit),
                    first_past(tree.root_node(), 0, limit),
                    "{source} at {limit}"
                );
            }
        }
    }

    #[test]
    fn equivalent_parameter_spellings_have_one_symbol() {
        let qualified = callable_symbol("Account", "process", &["System.List<Id>".into()]);
        let array = callable_symbol("Account", "process", &["Id[]".into()]);
        assert_eq!(qualified, array);
        assert_eq!(qualified, "Account.process(list<id>)");
        assert_eq!(
            callable_symbol("Account", "Process", &["Id[]".into()]),
            "Account.Process(list<id>)"
        );
    }

    #[test]
    fn a_field_is_the_same_child_by_id_as_by_name() {
        let source = b"class A extends B {
            Integer n { get; set; }
            A(Integer seed) { this(seed, 1); }
            void m(Account[] rows) {
                Integer x = rows.size() > 0 ? -rows[0].n : (Integer) n;
                for (Account a : rows) { x += a.n; }
                if (x > 1) { upsert rows Account.Fields.Id; } else { merge rows[0] rows[1]; }
                Object o = new List<Integer>[2];
            }
        }";
        let tree = parse(&mut parser(), source);
        let mut nodes = vec![tree.root_node()];
        let mut found = [0; FIELDS.len()];
        while let Some(node) = nodes.pop() {
            for (index, field) in FIELDS.iter().chain(&["no_such_field"]).enumerate() {
                let by_name = node.child_by_field_name(field);
                assert_eq!(by_field(node, field), by_name, "{field} of {}", node.kind());
                if let (Some(found), Some(_)) = (found.get_mut(index), by_name) {
                    *found += 1;
                }
            }
            let mut cursor = node.walk();
            nodes.extend(node.children(&mut cursor));
        }
        // The source gives every field a child somewhere, so each id is
        // compared on a match and not only on an absence.
        for (field, found) in FIELDS.iter().zip(found) {
            assert!(found > 0, "no `{field}` in the source");
        }
    }

    #[test]
    fn a_tree_that_reports_an_error_always_has_an_error_line() {
        assert_eq!(error_line(false, None, 1), None);
        assert_eq!(error_line(true, Some(6), 1), Some(6));
        // No error or missing node was found, yet the tree says it has an
        // error: it is reported at the root, never as a clean file.
        assert_eq!(error_line(true, None, 3), Some(3));
    }

    #[test]
    fn a_real_tree_gives_the_line_of_its_first_error() {
        let mut parser = parser();
        let clean = parse(&mut parser, b"class A {}\n");
        assert_eq!(errors(&clean), None);
        let broken = parse(&mut parser, b"class A {\n  void m() {\n    x = ;\n  }\n}\n");
        assert_eq!(errors(&broken), Some(3));
    }
}
