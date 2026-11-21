//! Object and field names in embedded SOQL and SOSL. The Apex grammar parses
//! a `[SELECT ..]` in place, so these come from syntax. A query assembled in
//! a string is not seen here; the textual tier is what covers it.
//!
//! A field is reported as written, prefixed with the object it is selected
//! from: `Account.Owner.Name`. A child relationship subquery is a path too:
//! `Account.Contacts` and, for its fields, `Account.Contacts.Email`.

use super::{Value, Walker};
use crate::apex::{kids, text};
use crate::index::Ty;
use sf_impact_common::rows::RefKind;
use tree_sitter::Node;

impl Walker<'_> {
    pub(super) fn query(&mut self, node: Node) {
        for body in kids(node) {
            match body.kind() {
                "soql_query_body" => self.select(body, None),
                _ => self.search(body),
            }
        }
    }

    /// A SOQL query returns a list of the object it selects from.
    pub(super) fn query_value(&self, node: Node) -> Value {
        let body = kids(node).into_iter().next();
        let object = body
            .filter(|body| body.kind() == "soql_query_body")
            .and_then(from)
            .map(|(object, _)| text(object, self.source));
        match object {
            Some(object) => Value::Instance(vec![Ty::platform(&format!("List<{object}>"))]),
            None => Value::Unknown,
        }
    }

    /// One `SELECT`. `parent` is the object of the enclosing query when this
    /// is a child relationship subquery in its field list.
    fn select(&mut self, body: Node, parent: Option<&str>) {
        let Some((object, alias)) = from(body) else {
            return self.fields(body, "", None);
        };
        let name = text(object, self.source).to_string();
        let alias = alias.map(|alias| text(alias, self.source).to_string());
        let object = match parent {
            Some(parent) => {
                let relationship = format!("{parent}.{name}");
                self.emit_sobject(RefKind::SoqlField, object, relationship.clone(), None);
                relationship
            }
            None => {
                self.emit_sobject(RefKind::SoqlObject, object, name.clone(), None);
                name
            }
        };
        self.fields(body, &object, alias.as_deref());
    }

    /// SOSL: each `RETURNING Object(fields)` is an object and its fields.
    fn search(&mut self, body: Node) {
        for clause in kids(body) {
            if clause.kind() != "returning_clause" {
                self.fields(clause, "", None);
                continue;
            }
            for returned in kids(clause) {
                let Some(object) = kids(returned).into_iter().next() else {
                    continue;
                };
                let name = text(object, self.source).to_string();
                self.emit_sobject(RefKind::SoqlObject, object, name.clone(), None);
                self.fields(returned, &name, None);
            }
        }
    }

    /// The field names anywhere under `node`, as fields of `object`. Bound
    /// Apex expressions (`:variable`) go back to the Apex walk.
    fn fields(&mut self, node: Node, object: &str, alias: Option<&str>) {
        for child in kids(node) {
            match child.kind() {
                "from_clause" => {}
                "field_identifier" if !object.is_empty() => {
                    let path = text(child, self.source);
                    let path = strip_qualifier(&path, alias, object);
                    self.emit_sobject(RefKind::SoqlField, child, format!("{object}.{path}"), None);
                }
                "subquery" => {
                    // In the field list a subquery follows a child relationship
                    // of this object; in a condition it is a query of its own.
                    let is_relationship =
                        matches!(node.kind(), "select_clause" | "selected_fields");
                    for body in kids(child) {
                        self.select(body, is_relationship.then_some(object));
                    }
                }
                "type_of_clause" if !object.is_empty() => self.type_of(child, object),
                "bound_apex_expression" => self.children(child),
                _ => self.fields(child, object, alias),
            }
        }
    }

    /// `TYPEOF What WHEN Account THEN Phone ELSE Name END`.
    fn type_of(&mut self, clause: Node, object: &str) {
        let mut polymorphic = String::new();
        for part in kids(clause) {
            match part.kind() {
                "identifier" | "dotted_identifier" => {
                    polymorphic = format!("{object}.{}", text(part, self.source));
                    self.emit_sobject(RefKind::SoqlField, part, polymorphic.clone(), None);
                }
                "when_expression" => {
                    let mut when = String::new();
                    for part in kids(part) {
                        if part.kind() == "identifier" {
                            when = text(part, self.source).to_string();
                            self.emit_sobject(RefKind::SoqlObject, part, when.clone(), None);
                        } else {
                            self.field_list(part, &when);
                        }
                    }
                }
                _ => {
                    for list in kids(part) {
                        self.field_list(list, &polymorphic);
                    }
                }
            }
        }
    }

    fn field_list(&mut self, list: Node, object: &str) {
        for field in kids(list) {
            let name = format!("{object}.{}", text(field, self.source));
            self.emit_sobject(RefKind::SoqlField, field, name, None);
        }
    }
}

/// The object a query selects from, and its alias if it has one.
fn from(body: Node) -> Option<(Node, Option<Node>)> {
    let from = body.child_by_field_name("from_clause")?;
    let first = kids(from).into_iter().next()?;
    if first.kind() != "storage_alias" {
        return Some((first, None));
    }
    let parts = kids(first);
    Some((*parts.first()?, parts.get(1).copied()))
}

/// `a.Name` and `Account.Name` in `FROM Account a` are both `Name`.
fn strip_qualifier<'a>(path: &'a str, alias: Option<&str>, object: &str) -> &'a str {
    match path.split_once('.') {
        Some((first, rest))
            if alias.is_some_and(|alias| alias.eq_ignore_ascii_case(first))
                || object.eq_ignore_ascii_case(first) =>
        {
            rest
        }
        _ => path,
    }
}
