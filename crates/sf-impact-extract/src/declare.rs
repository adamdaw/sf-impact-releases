//! Phase one: the declarations of one Apex file. These become `declaration`
//! rows and, across all files, the index that references are resolved against.

use crate::apex::{callable_symbol, field_text, kids, line, param_types, type_text};
use sf_impact_common::rows::{DeclType, Declaration, ReferencedType, fold};
use tree_sitter::{Node, Tree};

/// What one file declares. `rows` are in source order.
#[derive(Default)]
pub struct FileDecls {
    pub rows: Vec<Declaration>,
    /// For each of `rows`: whether nothing else is on its first and last
    /// lines, so that a line it starts or ends on is a line of it alone.
    pub alone: Vec<bool>,
    pub types: Vec<TypeDecl>,
}

/// A class, interface or enum, with the members needed to resolve references.
/// Type names are kept as written; they are resolved in the context of `symbol`.
#[derive(Debug, PartialEq)]
pub struct TypeDecl {
    pub symbol: String,
    pub superclass: Option<String>,
    pub interfaces: Vec<String>,
    pub methods: Vec<Callable>,
    pub constructors: Vec<Callable>,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Callable {
    pub symbol: String,
    /// The declaring type.
    pub owner: String,
    pub name: String,
    /// The name, case-folded: what calls are matched on.
    pub key: String,
    pub params: Vec<String>,
    /// `None` for `void` and for constructors.
    pub returns: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub symbol: String,
    /// `Field`, or `EnumValue` for a value of an enum.
    pub kind: ReferencedType,
    pub owner: String,
    /// The name, case-folded.
    pub key: String,
    pub ty: String,
    /// Reachable through the type's name, and unqualified inside the type.
    pub is_static: bool,
}

pub fn declarations(tree: &Tree, source: &[u8], package: &str, file: &str) -> FileDecls {
    let mut collector = Collector {
        source,
        package,
        file,
        out: FileDecls::default(),
    };
    collector.container(tree.root_node(), None);
    collector.out
}

struct Collector<'a> {
    source: &'a [u8],
    package: &'a str,
    file: &'a str,
    out: FileDecls,
}

impl Collector<'_> {
    /// Declarations directly inside `node`. `owner` indexes `out.types`.
    /// Error nodes are looked through, so a file that fails to parse still
    /// yields the declarations the grammar recovered.
    fn container(&mut self, node: Node, owner: Option<usize>) {
        for child in kids(node) {
            match child.kind() {
                "class_declaration" => self.type_decl(child, DeclType::Class, owner),
                "interface_declaration" => self.type_decl(child, DeclType::Interface, owner),
                "enum_declaration" => self.type_decl(child, DeclType::Enum, owner),
                "trigger_declaration" => {
                    if let Some(name) = field_text(child, "name", self.source) {
                        self.row(child, name.to_string(), &name, DeclType::Trigger);
                    }
                }
                "ERROR" | "class_body" | "interface_body" | "enum_body" => {
                    self.container(child, owner)
                }
                _ => {
                    if let Some(owner) = owner {
                        self.member(child, owner);
                    }
                }
            }
        }
    }

    fn type_decl(&mut self, node: Node, kind: DeclType, outer: Option<usize>) {
        let Some(name) = field_text(node, "name", self.source) else {
            return;
        };
        let symbol = match outer {
            Some(outer) => format!("{}.{name}", self.out.types[outer].symbol),
            None => name.to_string(),
        };
        self.row(node, symbol.clone(), &name, kind);

        let mut decl = TypeDecl {
            symbol,
            superclass: None,
            interfaces: Vec::new(),
            methods: Vec::new(),
            constructors: Vec::new(),
            fields: Vec::new(),
        };
        for child in kids(node) {
            match child.kind() {
                "superclass" => {
                    decl.superclass = kids(child).first().map(|ty| type_text(*ty, self.source));
                }
                // `implements` on a class or enum, `extends` on an interface.
                "interfaces" | "extends_interfaces" => {
                    for list in kids(child) {
                        for ty in kids(list) {
                            decl.interfaces.push(type_text(ty, self.source));
                        }
                    }
                }
                _ => {}
            }
        }
        self.out.types.push(decl);
        let this = self.out.types.len() - 1;
        self.container(node, Some(this));
    }

    fn member(&mut self, node: Node, owner: usize) {
        let owner_symbol = self.out.types[owner].symbol.clone();
        match node.kind() {
            "method_declaration" | "constructor_declaration" => {
                let Some(name) = field_text(node, "name", self.source) else {
                    return;
                };
                let params = param_types(node, self.source);
                let callable = Callable {
                    symbol: callable_symbol(&owner_symbol, &name, &params),
                    owner: owner_symbol,
                    name: name.to_string(),
                    key: fold(&name),
                    params,
                    returns: node
                        .child_by_field_name("type")
                        .filter(|ty| ty.kind() != "void_type")
                        .map(|ty| type_text(ty, self.source)),
                };
                if node.kind() == "method_declaration" {
                    self.row(node, callable.symbol.clone(), &name, DeclType::Method);
                    self.out.types[owner].methods.push(callable);
                } else {
                    self.row(node, callable.symbol.clone(), &name, DeclType::Constructor);
                    self.out.types[owner].constructors.push(callable);
                }
            }
            "field_declaration" | "constant_declaration" => {
                let Some(ty) = node.child_by_field_name("type") else {
                    return;
                };
                let ty = type_text(ty, self.source);
                // Interface constants are implicitly static.
                let is_static =
                    node.kind() == "constant_declaration" || has_modifier(node, "static");
                let mut cursor = node.walk();
                for declarator in node.children_by_field_name("declarator", &mut cursor) {
                    let Some(name) = field_text(declarator, "name", self.source) else {
                        continue;
                    };
                    let symbol = format!("{owner_symbol}.{name}");
                    self.row(node, symbol.clone(), &name, DeclType::Field);
                    self.out.types[owner].fields.push(Field {
                        symbol,
                        kind: ReferencedType::Field,
                        owner: owner_symbol.clone(),
                        key: fold(&name),
                        ty: ty.clone(),
                        is_static,
                    });
                }
            }
            "enum_constant" => {
                let Some(name) = field_text(node, "name", self.source) else {
                    return;
                };
                let symbol = format!("{owner_symbol}.{name}");
                self.row(node, symbol.clone(), &name, DeclType::EnumValue);
                // An enum value is a static field whose type is its enum.
                self.out.types[owner].fields.push(Field {
                    symbol,
                    kind: ReferencedType::EnumValue,
                    owner: owner_symbol.clone(),
                    key: fold(&name),
                    ty: owner_symbol,
                    is_static: true,
                });
            }
            _ => {}
        }
    }

    fn row(&mut self, node: Node, symbol: String, name: &str, kind: DeclType) {
        let annotations = annotations(node, self.source);
        self.out.alone.push(stands_alone(node, self.source));
        self.out.rows.push(Declaration {
            symbol,
            name: name.to_string(),
            kind,
            package: self.package.to_string(),
            file: self.file.to_string(),
            start: line(node),
            end: node.end_position().row as u32 + 1,
            is_test: annotations.iter().any(|a| a.eq_ignore_ascii_case("IsTest"))
                || has_modifier(node, "testMethod"),
            annotations,
            collision: false,
            scope: None,
        });
    }
}

/// Whether nothing but white space is before a declaration on its first
/// line and after it on its last. Anything else there, the rest of a class
/// header, the end of an initializer, another member, even a comment, is
/// something a change to that line could be a change to.
fn stands_alone(declaration: Node, source: &[u8]) -> bool {
    let before = source[..declaration.start_byte()].rsplit(|byte| *byte == b'\n');
    let after = source[declaration.end_byte()..].split(|byte| *byte == b'\n');
    let is_blank = |text: Option<&[u8]>| text.unwrap_or_default().trim_ascii().is_empty();
    is_blank(before.take(1).next()) && is_blank(after.take(1).next())
}

/// Annotation names as written, in source order: `["AuraEnabled"]`.
fn annotations(declaration: Node, source: &[u8]) -> Vec<String> {
    modifiers(declaration)
        .into_iter()
        .filter(|modifier| modifier.kind() == "annotation")
        .filter_map(|annotation| field_text(annotation, "name", source))
        .map(|name| name.to_string())
        .collect()
}

fn has_modifier(declaration: Node, modifier: &str) -> bool {
    modifiers(declaration)
        .into_iter()
        .flat_map(kids)
        .any(|found| found.kind() == modifier)
}

fn modifiers(declaration: Node) -> Vec<Node> {
    kids(declaration)
        .into_iter()
        .filter(|child| child.kind() == "modifiers")
        .flat_map(kids)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apex;

    fn declared(source: &str) -> Vec<(String, DeclType)> {
        let tree = apex::parse(&mut apex::parser(), source.as_bytes());
        declarations(&tree, source.as_bytes(), "pkg", "A.cls")
            .rows
            .into_iter()
            .map(|row| (row.symbol, row.kind))
            .collect()
    }

    #[test]
    fn overloads_are_distinct_symbols() {
        let rows = declared("class A { void m(String s) {} void m(Integer i, List<Id> ids) {} }");

        assert_eq!(
            rows,
            [
                ("A".into(), DeclType::Class),
                ("A.m(string)".into(), DeclType::Method),
                ("A.m(integer,list<id>)".into(), DeclType::Method),
            ]
        );
    }

    #[test]
    fn members_of_inner_types_are_qualified_by_the_outer_type() {
        let rows = declared("class A { class B { B(Id x) {} } enum E { ONE } Integer n, m; }");

        assert_eq!(
            rows,
            [
                ("A".into(), DeclType::Class),
                ("A.B".into(), DeclType::Class),
                ("A.B.B(id)".into(), DeclType::Constructor),
                ("A.E".into(), DeclType::Enum),
                ("A.E.ONE".into(), DeclType::EnumValue),
                ("A.n".into(), DeclType::Field),
                ("A.m".into(), DeclType::Field),
            ]
        );
    }

    #[test]
    fn test_classes_and_methods_are_marked_whichever_way_they_are_written() {
        let source = "@isTest class T { @IsTest static void a() {} static testMethod void b() {} void c() {} }";
        let tree = apex::parse(&mut apex::parser(), source.as_bytes());

        let is_test: Vec<bool> = declarations(&tree, source.as_bytes(), "pkg", "T.cls")
            .rows
            .iter()
            .map(|row| row.is_test)
            .collect();

        assert_eq!(is_test, [true, true, true, false]);
    }

    #[test]
    fn declarations_survive_a_syntax_error_elsewhere_in_the_file() {
        let rows = declared("class A { void ok() {} void broken( { } }");

        assert!(rows.contains(&("A.ok()".into(), DeclType::Method)));
    }
}
