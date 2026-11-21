//! Phase two: the `parsed` references of one Apex file, resolved against the
//! index of everything declared in scope.
//!
//! The walk keeps just enough of Apex's static semantics to say what a name
//! refers to: the enclosing types, the variables in scope and the declared
//! type of an expression where that can be read off the source. Where it
//! cannot, the row says so (`detail`) instead of guessing.
//!
//! A reference is a row unless it is proven to be the platform's alone: a
//! platform name (see `Index::foreign`), or a member of a value whose static
//! type is proven to be a platform type. That proof is narrow. A platform
//! class without type arguments returns platform types from its methods,
//! because Apex has no generic methods. A collection returns its elements
//! from the methods modelled here. Anything else (a platform type with type
//! arguments, a collection method that is not modelled, a call that left no
//! row) proves nothing, and what follows it is reported as of unknown type.

mod query;

use crate::apex::{
    by_field, callable_symbol, column, field_text, is_type, kids, line, param_types, text,
    type_text,
};
use crate::declarative::{Catalog, Found};
use crate::declare::{Callable, Field};
use crate::index::{Binding, Index, Outside, Shape, Ty, element, generic, head, outer};
use crate::platform::is_universal_method;
use sf_impact_common::rows::{Detail, RefKind, Reference, ReferencedType, Resolution, Tier, fold};
use std::collections::{BTreeMap, BTreeSet};
use tree_sitter::{Node, Tree};

pub fn references(
    tree: &Tree,
    source: &[u8],
    index: &Index,
    catalog: &Catalog,
    package: &str,
    file: &str,
) -> Vec<Reference> {
    let mut walker = Walker {
        source,
        index,
        catalog,
        package,
        file,
        out: Vec::new(),
        types: Vec::new(),
        referrer: None,
        variables: Vec::new(),
        switches: Vec::new(),
    };
    walker.visit(tree.root_node(), None);
    walker.out
}

struct Walker<'a> {
    source: &'a [u8],
    index: &'a Index,
    /// What metadata declares: objects, their fields and labels.
    catalog: &'a Catalog,
    package: &'a str,
    file: &'a str,
    out: Vec<Reference>,
    /// The enclosing types, innermost last.
    types: Vec<Enclosing>,
    /// Symbol of the innermost enclosing declaration.
    referrer: Option<String>,
    /// Variables in scope as (folded name, declared type), innermost last.
    variables: Vec<(String, Ty)>,
    /// What each enclosing `switch` is on.
    switches: Vec<Switched>,
}

struct Enclosing {
    symbol: String,
    /// The superclass this declaration names, as written.
    superclass: Option<String>,
}

/// The type of the value a `switch` is on, which is what its labels name
/// values of.
enum Switched {
    /// An enum declared in scope, by index key; several where which one is
    /// not decided.
    Enum(Vec<String>),
    /// A platform type: its values are the platform's.
    Platform,
    /// A type that is not declared in scope, as written.
    Foreign(String),
    Unknown,
}

/// What an expression denotes, as far as the source says.
enum Value {
    /// A value of one of these declared types. Nearly always exactly one.
    /// Several where the source does not decide: a name declared more than
    /// once whose declarations differ, or a call that could not be bound.
    Instance(Vec<Ty>),
    /// A type declared in scope, by index key: the `Foo` of `Foo.bar()`.
    /// Several where the name may mean more than one type.
    Declared(Vec<String>),
    /// A dotted name that is neither a variable nor a declared type: a
    /// platform class, a namespace, an SObject type.
    Named(String),
    /// A value proven to be of some platform type. Its members are the
    /// platform's.
    Platform,
    Unknown,
}

/// What a bare name refers to where it is written.
enum Name<'a> {
    Variable(Ty),
    /// A field of an enclosing type: every one the name may be.
    Fields(Vec<&'a Field>),
    /// Declared types, by index key.
    Types(Vec<String>),
    Nothing,
}

/// What a reference site refers to.
struct Target {
    referenced: String,
    /// What kind of thing the declaration, or every candidate, is. `None`
    /// where it is not declared, or the candidates are of different kinds.
    referenced_type: Option<ReferencedType>,
    resolution: Resolution,
    detail: Option<Detail>,
    candidates: Vec<String>,
    /// Set when the site turned out to be another kind of reference than
    /// its syntax suggests: `SObjectType.Account.getDescribe()` is a
    /// reference to a type, not a call into the repository.
    kind: Option<RefKind>,
}

const CLASS: Option<ReferencedType> = Some(ReferencedType::Class);
const METHOD: Option<ReferencedType> = Some(ReferencedType::Method);
const CONSTRUCTOR: Option<ReferencedType> = Some(ReferencedType::Constructor);

impl Target {
    fn new(referenced: impl Into<String>, resolution: Resolution) -> Target {
        Target {
            referenced: referenced.into(),
            referenced_type: None,
            resolution,
            detail: None,
            candidates: Vec::new(),
            kind: None,
        }
    }

    fn declared(symbol: &str, kind: Option<ReferencedType>) -> Target {
        let mut target = Target::new(symbol, Resolution::Declaration);
        target.referenced_type = kind;
        target
    }

    fn because(mut self, detail: Detail) -> Target {
        self.detail = Some(detail);
        self
    }

    /// One of several declarations, which the source does not decide between.
    fn among(
        referenced: impl Into<String>,
        kind: Option<ReferencedType>,
        detail: Detail,
        candidates: Vec<String>,
    ) -> Target {
        let mut target = Target::declared(&referenced.into(), kind).because(detail);
        target.candidates = candidates;
        target
    }

    /// A reference to `name` through something whose type is not known.
    fn of_unknown_type(name: &str) -> Target {
        Target::new(name, Resolution::Unresolved).because(Detail::UnknownType)
    }

    /// A reference to whichever of `symbols` the name `name` means here.
    /// One declaration, or several spellings of one name, is that
    /// declaration. Different declarations are all candidates: this is
    /// where a name reached through declarations that differ ends up.
    fn one_of<'s>(
        symbols: impl IntoIterator<Item = (&'s str, Option<ReferencedType>)>,
        name: &str,
    ) -> Option<Target> {
        // Per case-folded identity, the spelling that sorts first, so that
        // it does not depend on the order files were read in.
        let mut spellings: BTreeMap<String, &str> = BTreeMap::new();
        let mut kinds = BTreeSet::new();
        for (symbol, kind) in symbols {
            let spelling = spellings.entry(fold(symbol)).or_insert(symbol);
            *spelling = (*spelling).min(symbol);
            kinds.insert(kind);
        }
        let kind = match kinds.len() {
            1 => kinds.pop_first().flatten(),
            _ => None,
        };
        let mut distinct: Vec<String> = spellings.values().map(|s| s.to_string()).collect();
        distinct.sort();
        match distinct.len() {
            0 => None,
            1 => Some(Target::declared(&distinct[0], kind)),
            _ => Some(Target::among(
                name,
                kind,
                Detail::AmbiguousReceiver,
                distinct,
            )),
        }
    }
}

impl<'a> Walker<'a> {
    /// `parent` is the node's parent where the caller has it, which spares
    /// a lookup: `Node::parent` walks down from the root to find it.
    fn visit(&mut self, node: Node, parent: Option<Node>) {
        debug_assert!(
            parent.is_none_or(|parent| Some(parent.id()) == node.parent().map(|p| p.id())),
            "{} is given a parent that is not its own",
            node.kind()
        );
        match node.kind() {
            "class_declaration" | "interface_declaration" | "enum_declaration" => {
                self.type_declaration(node)
            }
            "trigger_declaration" => self.trigger(node),
            "method_declaration" | "constructor_declaration" if !self.types.is_empty() => {
                self.callable(node)
            }
            "field_declaration" | "constant_declaration" if !self.types.is_empty() => {
                self.field(node)
            }
            "block" | "for_statement" | "catch_clause" | "switch_rule" => self.scoped(node),
            "local_variable_declaration" => self.local(node),
            "formal_parameter" | "when_sobject_type" => {
                self.children(node);
                let parts = kids(node);
                let ty = parts.iter().find(|part| is_type(part.kind()));
                let name = parts.iter().find(|part| part.kind() == "identifier");
                if let (Some(ty), Some(name)) = (ty, name) {
                    self.declare(*name, *ty);
                }
            }
            "enhanced_for_statement" => self.for_each(node),
            "switch_expression" => self.switch(node),
            "switch_label" => self.switch_label(node),
            "method_invocation" => self.invocation(node),
            "explicit_constructor_invocation" => {
                let target = self.explicit_construction(node);
                let at = by_field(node, "constructor");
                self.emit(RefKind::Construct, at, target);
                self.children(node);
            }
            "object_creation_expression" => self.creation(node),
            "cast_expression" => {
                if let Some(ty) = by_field(node, "type") {
                    self.type_reference(ty, RefKind::Cast);
                }
                if let Some(value) = by_field(node, "value") {
                    self.visit(value, Some(node));
                }
            }
            "field_access" => self.field_access(node, parent),
            "identifier" => self.identifier(node, parent),
            "dml_expression" => self.dml(node),
            "query_expression" => self.query(node),
            // Annotation arguments are literals; type parameters declare names.
            "modifiers" | "annotation" | "type_parameters" => {}
            kind if is_type(kind) => self.type_reference(node, RefKind::Type),
            _ => self.children(node),
        }
    }

    fn children(&mut self, node: Node) {
        for child in kids(node) {
            self.visit(child, Some(node));
        }
    }

    /// Visits a node whose variables go out of scope at its end.
    fn scoped(&mut self, node: Node) {
        let outer_scope = self.variables.len();
        self.children(node);
        self.variables.truncate(outer_scope);
    }

    // Declarations: these set the referrer and the scope for what they contain.

    fn type_declaration(&mut self, node: Node) {
        let Some(name) = field_text(node, "name", self.source) else {
            return self.children(node);
        };
        let symbol = match self.types.last() {
            Some(outer) => format!("{}.{name}", outer.symbol),
            None => name.to_string(),
        };
        let superclass = kids(node)
            .into_iter()
            .find(|child| child.kind() == "superclass")
            .and_then(|superclass| kids(superclass).into_iter().next())
            .map(|ty| type_text(ty, self.source));
        let outer_referrer = self.referrer.replace(symbol.clone());
        self.types.push(Enclosing { symbol, superclass });
        for child in kids(node) {
            let kind = match child.kind() {
                "superclass" | "extends_interfaces" => RefKind::Extends,
                "interfaces" => RefKind::Implements,
                _ => {
                    self.visit(child, Some(node));
                    continue;
                }
            };
            // The types are direct children, or sit inside one `type_list`.
            for ty in kids(child) {
                if ty.kind() == "type_list" {
                    for ty in kids(ty) {
                        self.type_reference(ty, kind);
                    }
                } else {
                    self.type_reference(ty, kind);
                }
            }
        }
        self.types.pop();
        self.referrer = outer_referrer;
    }

    fn trigger(&mut self, node: Node) {
        let Some(name) = field_text(node, "name", self.source) else {
            return self.children(node);
        };
        let outer_referrer = self.referrer.replace(name.to_string());
        if let Some(object) = by_field(node, "object") {
            let name = text(object, self.source).to_string();
            self.emit_sobject(RefKind::TriggerObject, object, name, None);
        }
        if let Some(body) = by_field(node, "body") {
            self.visit(body, Some(node));
        }
        self.referrer = outer_referrer;
    }

    fn callable(&mut self, node: Node) {
        let Some(name) = field_text(node, "name", self.source) else {
            return self.children(node);
        };
        let symbol = callable_symbol(self.context(), &name, &param_types(node, self.source));
        let outer_referrer = self.referrer.replace(symbol);
        self.scoped(node);
        self.referrer = outer_referrer;
    }

    fn field(&mut self, node: Node) {
        let owner = self.context().to_string();
        let symbol = |declarator: Node| {
            field_text(declarator, "name", self.source).map(|name| format!("{owner}.{name}"))
        };
        let ty = by_field(node, "type").map(|ty| Ty::new(type_text(ty, self.source), &owner));
        let outer_referrer = self.referrer.clone();
        // The declared type and any accessors belong to the first field.
        if let Some(first) = by_field(node, "declarator").and_then(symbol) {
            self.referrer = Some(first);
        }
        for child in kids(node) {
            match child.kind() {
                "variable_declarator" => {
                    if let Some(symbol) = symbol(child) {
                        self.referrer = Some(symbol);
                    }
                    self.children(child);
                }
                "accessor_list" => {
                    for accessor in kids(child) {
                        let outer_scope = self.variables.len();
                        // A setter's implicit parameter.
                        if let Some(ty) = &ty {
                            self.variables.push(("value".to_string(), ty.clone()));
                        }
                        self.children(accessor);
                        self.variables.truncate(outer_scope);
                    }
                }
                _ => self.visit(child, Some(node)),
            }
        }
        self.referrer = outer_referrer;
    }

    fn local(&mut self, node: Node) {
        let ty = by_field(node, "type");
        for child in kids(node) {
            if child.kind() != "variable_declarator" {
                self.visit(child, Some(node));
                continue;
            }
            // The initialiser cannot see the variable it initialises.
            self.children(child);
            if let (Some(name), Some(ty)) = (by_field(child, "name"), ty) {
                self.declare(name, ty);
            }
        }
    }

    fn for_each(&mut self, node: Node) {
        let outer_scope = self.variables.len();
        let field = |name| by_field(node, name);
        if let Some(ty) = field("type") {
            self.visit(ty, Some(node));
        }
        if let Some(value) = field("value") {
            self.visit(value, Some(node));
        }
        if let (Some(name), Some(ty)) = (field("name"), field("type")) {
            self.declare(name, ty);
        }
        if let Some(body) = field("body") {
            self.visit(body, Some(node));
        }
        self.variables.truncate(outer_scope);
    }

    fn declare(&mut self, name: Node, ty: Node) {
        let ty = Ty::new(type_text(ty, self.source), self.context());
        self.variables.push((fold(&text(name, self.source)), ty));
    }

    /// `switch on x`: remembers what `x` is, for the labels to refer to.
    fn switch(&mut self, node: Node) {
        let condition = by_field(node, "condition");
        let on = match condition.map(|condition| self.value(condition)) {
            Some(Value::Instance(types)) => {
                let enums: Vec<String> = types
                    .iter()
                    .flat_map(|ty| self.index.keys_of(ty))
                    .map(String::from)
                    .collect();
                match &types[..] {
                    _ if !enums.is_empty() => Switched::Enum(enums),
                    [only] if self.is_platform_type(only) => Switched::Platform,
                    [only] => Switched::Foreign(head(&only.text).to_string()),
                    _ => Switched::Unknown,
                }
            }
            Some(Value::Platform) => Switched::Platform,
            // `Trigger.operationType`: a member of a platform class.
            Some(Value::Named(name)) => match self.foreign_member(name, false).1 {
                Value::Platform => Switched::Platform,
                _ => Switched::Unknown,
            },
            _ => Switched::Unknown,
        };
        self.switches.push(on);
        self.children(node);
        self.switches.pop();
    }

    /// `when UP, DOWN`: a bare name in a label is a value of the enum being
    /// switched on, written without its type.
    fn switch_label(&mut self, node: Node) {
        for label in kids(node) {
            let (Some(on), "identifier") = (self.switches.last(), label.kind()) else {
                self.visit(label, Some(node));
                continue;
            };
            let name = text(label, self.source);
            let target = match on {
                Switched::Enum(enums) => {
                    let values = enums.iter().flat_map(|key| self.index.fields(key, &name));
                    Target::one_of(
                        values.map(|value| (value.symbol.as_str(), Some(value.kind))),
                        &name,
                    )
                    .or_else(|| self.missing_member(&strs(enums), &name))
                }
                Switched::Platform => None,
                Switched::Foreign(of_enum) => {
                    self.foreign_member(format!("{of_enum}.{name}"), false).0
                }
                Switched::Unknown => Some(Target::of_unknown_type(&name)),
            };
            self.emit(RefKind::StaticAccess, Some(label), target);
        }
    }

    // Reference sites.

    /// A type written in the source. `kind` applies to the type itself; its
    /// type arguments are plain `type` references.
    fn type_reference(&mut self, node: Node, kind: RefKind) {
        match node.kind() {
            "type_identifier" | "scoped_type_identifier" => {
                let target = self.type_target(&type_text(node, self.source));
                self.emit(kind, Some(node), target);
            }
            "generic_type" => {
                for (position, part) in kids(node).into_iter().enumerate() {
                    if position == 0 {
                        self.type_reference(part, kind);
                    } else {
                        self.children(part);
                    }
                }
            }
            // An array's element, or a type behind an annotation.
            _ => {
                for part in kids(node) {
                    if is_type(part.kind()) {
                        self.type_reference(part, kind);
                    }
                }
            }
        }
    }

    fn creation(&mut self, node: Node) {
        let Some(ty) = by_field(node, "type") else {
            return self.children(node);
        };
        // `new Foo(..)`, or the `Foo` of `new Foo<Bar>(..)`.
        let parts = kids(ty);
        let (created, type_arguments) = match ty.kind() {
            "generic_type" if !parts.is_empty() => (parts[0], &parts[1..]),
            _ => (ty, &parts[..0]),
        };
        let name = type_text(created, self.source);
        let args = self.arguments(node);
        let target = self.construction(&name, &args);
        self.emit(RefKind::Construct, Some(created), target);
        for arguments in type_arguments {
            self.children(*arguments);
        }

        // Only a type that is not an Apex class takes `Field = value`
        // arguments: an SObject, whose fields they name.
        let is_declared = !self.index.resolve_types(&name, self.context()).is_empty();
        let record = (!is_declared).then(|| self.index.foreign(&name, false));
        for child in kids(node) {
            match (&record, child.kind()) {
                _ if child.id() == ty.id() => {}
                (Some(record), "argument_list") => {
                    for argument in kids(child) {
                        let field = by_field(argument, "left").filter(|left| {
                            argument.kind() == "assignment_expression"
                                && left.kind() == "identifier"
                        });
                        let Some(field) = field else {
                            self.visit(argument, Some(child));
                            continue;
                        };
                        if record.outside != Outside::Platform {
                            let name = format!("{}.{}", record.name, text(field, self.source));
                            self.emit_sobject(RefKind::SobjectField, field, name, None);
                        }
                        if let Some(value) = by_field(argument, "right") {
                            self.visit(value, Some(argument));
                        }
                    }
                }
                _ => self.visit(child, Some(node)),
            }
        }
    }

    fn invocation(&mut self, node: Node) {
        let (target, _) = self.call(node);
        // The platform's own DML entry points are calls into the platform,
        // and name the object they write.
        let upsert_key = match target {
            None => self.database_dml(node),
            Some(_) => None,
        };
        self.emit(RefKind::Call, by_field(node, "name"), target);
        for child in kids(node) {
            if child.kind() != "argument_list" {
                self.visit(child, Some(node));
                continue;
            }
            for argument in kids(child) {
                if Some(argument.id()) != upsert_key {
                    self.visit(argument, Some(child));
                }
            }
        }
    }

    /// `Type.member`. Access through an instance is not a reference kind.
    fn field_access(&mut self, node: Node, parent: Option<Node>) {
        let (Some(object), Some(field)) = (by_field(node, "object"), by_field(node, "field"))
        else {
            return self.children(node);
        };
        let name = text(field, self.source);
        // `X.class` is a property of the type named `X`, never of a
        // variable or field that shares its name.
        if name.eq_ignore_ascii_case("class") {
            let target = self.type_target(&type_text(object, self.source));
            return self.emit(RefKind::Type, Some(object), target);
        }
        match self.value(object) {
            Value::Declared(keys) => {
                // `Outer.Inner` names a type; what is done with it is the reference.
                if self.inner_types(&keys, &name).is_empty() {
                    let fields = keys.iter().flat_map(|key| self.index.fields(key, &name));
                    let target = Target::one_of(
                        fields.map(|field| (field.symbol.as_str(), Some(field.kind))),
                        &name,
                    )
                    .or_else(|| self.missing_member(&strs(&keys), &name));
                    self.emit(RefKind::StaticAccess, Some(field), target);
                }
            }
            // The whole dotted name is one reference, reported at its end.
            Value::Named(qualifier) if !is_qualifier(node, parent) => {
                let (target, _) = self.foreign_member(format!("{qualifier}.{name}"), false);
                self.emit(RefKind::StaticAccess, Some(field), target);
            }
            _ => {}
        }
        self.visit(object, Some(node));
    }

    /// A bare name in an expression. Only a static field is a reference; a
    /// variable or an instance field of the same name comes first, as in Apex.
    fn identifier(&mut self, node: Node, parent: Option<Node>) {
        if is_declared_name(node, parent) {
            return;
        }
        let name = text(node, self.source);
        let target = match self.lookup(&name) {
            Name::Fields(fields) => {
                let statics = fields.iter().filter(|field| field.is_static);
                Target::one_of(
                    statics.map(|field| (field.symbol.as_str(), Some(field.kind))),
                    &name,
                )
            }
            // Nothing in scope has this name. If a supertype outside the
            // repository could declare it, it is that type's; otherwise it
            // is reported as missing rather than passed over.
            Name::Nothing
                if !is_qualifier(node, parent)
                    && self.index.foreign(&name, false).outside != Outside::Platform =>
            {
                let (context, outer) = self.scopes();
                self.missing_member(&[&context, &outer], &name)
            }
            _ => None,
        };
        self.emit(RefKind::StaticAccess, Some(node), target);
    }

    fn dml(&mut self, node: Node) {
        if let Some(target) = by_field(node, "target") {
            let object = self.dml_operand(target);
            if let Some(key) = by_field(node, "upsert_key") {
                let field = type_text(key, self.source);
                let field = match (object, field.contains('.')) {
                    (Some(object), false) => format!("{object}.{field}"),
                    _ => self.index.foreign(&field, false).name,
                };
                self.emit_sobject(RefKind::DmlField, key, field, None);
            }
        }
        for operand in ["target", "merge_with"] {
            if let Some(operand) = by_field(node, operand) {
                self.visit(operand, Some(node));
            }
        }
    }

    /// `Database.insert(records)` and its siblings. Returns the argument
    /// that was reported as an upsert key, which is then not a reference
    /// of its own.
    fn database_dml(&mut self, node: Node) -> Option<usize> {
        let receiver = fold(&type_text(by_field(node, "object")?, self.source));
        let method = fold(&field_text(node, "name", self.source)?);
        let writes = [
            "insert",
            "update",
            "upsert",
            "delete",
            "undelete",
            "merge",
            "insertimmediate",
            "updateimmediate",
            "deleteimmediate",
            "insertasync",
            "updateasync",
            "deleteasync",
        ];
        if !matches!(receiver.as_str(), "database" | "system.database")
            || !writes.contains(&method.as_str())
        {
            return None;
        }
        let arguments = kids(by_field(node, "arguments")?);
        self.dml_operand(*arguments.first()?);
        // `Database.upsert(records, Lead.External__c)`: a field token.
        let key = *arguments.get(1).filter(|_| method == "upsert")?;
        let Value::Named(field) = self.value(key) else {
            return None;
        };
        let field = self.index.foreign(&field, false).name;
        self.emit_sobject(RefKind::DmlField, key, field, None);
        Some(key.id())
    }

    /// Reports the object a DML operand is a record of: the declared type
    /// of what is written, or of its elements. Returns its name if known.
    fn dml_operand(&mut self, operand: Node) -> Option<String> {
        let object = match self.value(operand) {
            Value::Instance(types) => match &types[..] {
                [only] => {
                    let record = element(only).unwrap_or(only.clone());
                    Some(self.index.foreign(&record.text, false))
                }
                _ => None,
            },
            _ => None,
        }
        // `SObject`, an `Id` or a `Map` say nothing about which object.
        .filter(|record| record.outside != Outside::Platform && !record.name.contains('<'))
        .map(|record| record.name);
        match &object {
            Some(object) => self.emit_sobject(RefKind::DmlObject, operand, object.clone(), None),
            None => {
                let unknown = Some(Detail::UnknownType);
                self.emit_sobject(RefKind::DmlObject, operand, "SObject".to_string(), unknown);
            }
        }
        object
    }

    // What a reference site refers to.

    /// The type that `name`, written here, refers to.
    fn type_target(&self, name: &str) -> Option<Target> {
        let keys = self.index.resolve_types(name, self.context());
        if keys.is_empty() {
            let mut target = self.foreign_member(name.to_string(), false).0?;
            // The site decides the kind: a cast stays a cast.
            target.kind = None;
            return Some(target);
        }
        Target::one_of(keys.iter().map(|key| (self.index.symbol(key), CLASS)), name)
    }

    /// A dotted name that is not declared in scope: `external`,
    /// `unresolved`, or no row when all of it is the platform's. Also says
    /// what a value of it is: see [`Index::foreign`].
    fn foreign_member(&self, name: String, is_call: bool) -> (Option<Target>, Value) {
        let foreign = self.index.foreign(&name, is_call);
        let resolution = match foreign.outside {
            Outside::Platform => return (None, Value::Platform),
            Outside::External => Resolution::External,
            Outside::Unknown | Outside::Nowhere => Resolution::Unresolved,
        };
        let mut target = Target::new(foreign.name, resolution);
        let value = match foreign.shape {
            // The name goes on into something that is not understood.
            Shape::AsWritten => Value::Unknown,
            // An object token or a label, and then the platform's members.
            Shape::Object => {
                target.kind = Some(RefKind::Type);
                Value::Platform
            }
            Shape::Member => {
                target.kind = Some(RefKind::StaticAccess);
                Value::Platform
            }
        };
        (Some(target), value)
    }

    /// A member that the types in `keys` and their ancestors do not
    /// declare, named after the first of them. Where one of them has a
    /// supertype outside the repository, the member is inherited from
    /// there; otherwise it should exist and does not.
    fn missing_member(&self, keys: &[&str], name: &str) -> Option<Target> {
        let owner = keys.first().map_or("", |key| self.index.symbol(key));
        let referenced = match owner {
            "" => name.to_string(),
            owner => format!("{owner}.{name}"),
        };
        // A declared type may share its name with a platform one: a class
        // `Database` beside `Database.SaveResult`. Apex falls back to the
        // platform type for members the declared one lacks; so does this.
        let simple_name = owner.rsplit('.').next().unwrap_or(owner);
        if self.index.foreign(simple_name, false).outside == Outside::Platform {
            return None;
        }
        let outside = keys.iter().map(|key| self.index.outside(key)).max();
        match outside.unwrap_or(Outside::Nowhere) {
            Outside::Platform => None,
            Outside::External => Some(Target::new(referenced, Resolution::External)),
            Outside::Unknown | Outside::Nowhere => {
                Some(Target::new(referenced, Resolution::Unresolved).because(Detail::NoMember))
            }
        }
    }

    /// What a call refers to, and what it returns.
    fn call(&self, node: Node) -> (Option<Target>, Value) {
        let Some(name) = field_text(node, "name", self.source) else {
            return (None, Value::Unknown);
        };
        let args = self.arguments(node);
        match by_field(node, "object") {
            Some(object) => self.call_on(self.value(object), &name, &args),
            // Unqualified: a method of this type or its ancestors, else of
            // the outer type. The nearer scope that has the name wins,
            // unless which scope has it depends on which declaration of a
            // divergent type is meant.
            None => {
                let (context, outer) = self.scopes();
                let mut overloads = self.index.methods(&context, &name);
                if overloads.is_empty() || self.index.is_tainted(&context) {
                    overloads.extend(self.index.methods(&outer, &name));
                }
                self.bound(&overloads, &[&context, &outer], &name, &args)
            }
        }
    }

    fn call_on(&self, receiver: Value, name: &str, args: &[Option<Ty>]) -> (Option<Target>, Value) {
        match receiver {
            Value::Declared(keys) => {
                let outcomes = keys.iter().map(|key| self.member_call(key, name, args));
                self.either(outcomes.collect(), name)
            }
            Value::Instance(types) => {
                let outcomes = types
                    .iter()
                    .flat_map(|ty| self.instance_calls(ty, name, args));
                self.either(outcomes.collect(), name)
            }
            Value::Named(qualifier) => self.foreign_member(format!("{qualifier}.{name}"), true),
            Value::Platform => (None, Value::Platform),
            // The receiver could be anything, so the call could be to
            // anything of that name: say so.
            Value::Unknown => (Some(Target::of_unknown_type(name)), Value::Unknown),
        }
    }

    /// A call on a value of type `ty`: one outcome for each declared type
    /// that `ty` may mean, or one for a type that is not declared here.
    fn instance_calls(
        &self,
        ty: &Ty,
        name: &str,
        args: &[Option<Ty>],
    ) -> Vec<(Option<Target>, Value)> {
        let keys = self.index.keys_of(ty);
        if !keys.is_empty() {
            let outcomes = keys.iter().map(|key| self.member_call(key, name, args));
            return outcomes.collect();
        }
        if let Some(returned) = collection_member(ty, name) {
            return vec![(None, returned)];
        }
        if ty.context.is_none() {
            return vec![(None, Value::Platform)];
        }
        // A record of an object that metadata declares: its methods are
        // the platform's, and what they return is a record or the platform's.
        if self.index.is_object(head(&ty.text)) {
            return vec![(None, Value::Platform)];
        }
        // Not declared here: an installed package's type, an SObject, a
        // class that is missing, or a platform class.
        let (target, returned) = self.foreign_member(format!("{}.{name}", head(&ty.text)), true);
        let returned = match returned {
            // A platform type with type arguments may return one of them.
            Value::Platform if generic(&ty.text).is_some() => Value::Unknown,
            returned => returned,
        };
        vec![(target, returned)]
    }

    fn member_call(&self, key: &str, name: &str, args: &[Option<Ty>]) -> (Option<Target>, Value) {
        self.bound(&self.index.methods(key, name), &[key], name, args)
    }

    /// A call to `name` given the methods of that name that are in reach of
    /// the types in `keys`, where they were looked for.
    fn bound(
        &self,
        overloads: &[&Callable],
        keys: &[&str],
        name: &str,
        args: &[Option<Ty>],
    ) -> (Option<Target>, Value) {
        let is_tainted = keys.iter().any(|key| self.index.is_tainted(key));
        match self.index.bind(overloads, args, is_tainted) {
            Binding::Bound(declarations) => {
                let symbols = declarations.iter().map(|d| (d.symbol.as_str(), METHOD));
                let target = Target::one_of(symbols, name);
                (target, self.returned(&declarations))
            }
            // The methods come from declarations that differ: which of them
            // exist together is not known, so they are all candidates.
            Binding::Ambiguous(overloads) if is_tainted => {
                let target =
                    Target::among(name, METHOD, Detail::AmbiguousReceiver, symbols(&overloads));
                (Some(target), self.returned(&overloads))
            }
            Binding::Ambiguous(overloads) => {
                // Named after the type that declares them all, or after
                // the type they were called on when several do.
                let first = overloads[0];
                let one_owner = overloads.iter().all(|o| o.owner == first.owner);
                let owner = match (one_owner, keys.first()) {
                    (false, Some(key)) => self.index.symbol(key),
                    _ => &first.owner,
                };
                let group = format!("{owner}.{}", first.name);
                let target = Target::among(
                    group,
                    METHOD,
                    Detail::AmbiguousOverload,
                    symbols(&overloads),
                );
                (Some(target), self.returned(&overloads))
            }
            Binding::None if is_universal_method(&fold(name)) => (None, Value::Platform),
            Binding::None => match self.missing_member(keys, name) {
                Some(target) => (Some(target), Value::Unknown),
                None => (None, Value::Platform),
            },
        }
    }

    /// What one of these methods returns. They are the declarations a call
    /// may be to; where their return types differ, the value is one of them.
    fn returned(&self, callables: &[&Callable]) -> Value {
        let types: Option<Vec<Ty>> = callables
            .iter()
            .map(|callable| Some(Ty::new(callable.returns.as_ref()?, &callable.owner)))
            .collect();
        types.map_or(Value::Unknown, |types| self.one_of(types))
    }

    /// The outcome of a reference that is one of several, each with its
    /// own target and value: the receiver was one of several types. The
    /// values are combined whatever the targets were: an alternative that
    /// left no row (a collection's own method, say) still returns something.
    fn either(
        &self,
        mut outcomes: Vec<(Option<Target>, Value)>,
        name: &str,
    ) -> (Option<Target>, Value) {
        match outcomes.len() {
            // No alternative at all is no proof of anything.
            0 => return (Some(Target::of_unknown_type(name)), Value::Unknown),
            1 => return outcomes.remove(0),
            _ => {}
        }
        let (targets, values): (Vec<_>, Vec<_>) = outcomes.into_iter().unzip();
        let targets: Vec<Target> = targets.into_iter().flatten().collect();
        let is_understood = targets
            .iter()
            .all(|t| t.resolution == Resolution::Declaration);
        let target = match (targets.is_empty(), is_understood) {
            // Every alternative is the platform's.
            (true, _) => None,
            (false, true) => {
                let candidates = targets.iter().flat_map(|target| {
                    let symbols = match target.candidates.len() {
                        0 => std::slice::from_ref(&target.referenced),
                        _ => &target.candidates[..],
                    };
                    symbols
                        .iter()
                        .map(|symbol| (symbol.as_str(), target.referenced_type))
                });
                Target::one_of(candidates, name)
            }
            // One alternative leads somewhere that is not understood.
            (false, false) => Some(Target::of_unknown_type(name)),
        };
        (target, self.any_of(values))
    }

    /// A value that is one of `values`. Alternatives that are all typed give
    /// a value of one of those types, and alternatives that are all the
    /// platform's give the platform's. A mix of the two is neither: a value
    /// that may be a `String` or a `Foo` is not a `Foo`, and passing it to a
    /// method must not choose the `Foo` overload. It is unknown.
    fn any_of(&self, values: Vec<Value>) -> Value {
        let mut types = Vec::new();
        let mut may_be_platform = false;
        for value in values {
            match value {
                Value::Instance(alternatives) => types.extend(alternatives),
                Value::Platform => may_be_platform = true,
                _ => return Value::Unknown,
            }
        }
        match (types.is_empty(), may_be_platform) {
            (true, _) => Value::Platform,
            (false, false) => self.one_of(types),
            (false, true) => Value::Unknown,
        }
    }

    /// `new name(args)`: a declared constructor, or the type itself when it
    /// declares none that fits (the implicit or an inherited constructor).
    fn construction(&self, name: &str, args: &[Option<Ty>]) -> Option<Target> {
        let keys = self.index.resolve_types(name, self.context());
        if keys.is_empty() {
            return self.type_target(name);
        }
        let outcomes = keys.iter().map(|key| {
            let symbol = self.index.symbol(key);
            let is_tainted = self.index.is_tainted(key);
            let target = match self
                .index
                .bind(&self.index.constructors(key), args, is_tainted)
            {
                Binding::Bound(declarations) => {
                    let symbols = declarations
                        .iter()
                        .map(|d| (d.symbol.as_str(), CONSTRUCTOR));
                    Target::one_of(symbols, name)
                }
                Binding::Ambiguous(overloads) => {
                    let detail = match is_tainted {
                        true => Detail::AmbiguousReceiver,
                        false => Detail::AmbiguousOverload,
                    };
                    Some(Target::among(
                        symbol,
                        CONSTRUCTOR,
                        detail,
                        symbols(&overloads),
                    ))
                }
                Binding::None => Some(Target::declared(symbol, CLASS)),
            };
            (target, Value::Unknown)
        });
        self.either(outcomes.collect(), name).0
    }

    /// `this(..)` or `super(..)` at the head of a constructor.
    fn explicit_construction(&self, node: Node) -> Option<Target> {
        let args = self.arguments(node);
        let this = self.types.last()?;
        match by_field(node, "constructor")?.kind() {
            "this" => self.construction(&this.symbol, &args),
            _ => self.construction(head(this.superclass.as_ref()?), &args),
        }
    }

    /// The types of the arguments of a call, for choosing an overload. An
    /// argument counts as typed only when its type is settled.
    fn arguments(&self, node: Node) -> Vec<Option<Ty>> {
        let arguments = by_field(node, "arguments");
        arguments
            .map(kids)
            .unwrap_or_default()
            .into_iter()
            .map(|argument| match self.value(argument) {
                Value::Instance(types) => match &types[..] {
                    [only] if self.index.keys_of(only).len() <= 1 => Some(only.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    // The declared type of an expression.

    fn value(&self, node: Node) -> Value {
        let field = |name| by_field(node, name);
        let of = |node: Option<Node>| node.map_or(Value::Unknown, |node| self.value(node));
        let platform = |name| Value::Instance(vec![Ty::platform(name)]);
        let typed = |ty: Ty| Value::Instance(vec![ty]);
        match node.kind() {
            "identifier" => match self.lookup(&text(node, self.source)) {
                Name::Variable(ty) => typed(ty),
                Name::Fields(fields) => self.one_of(fields.into_iter().map(field_type).collect()),
                Name::Types(keys) => Value::Declared(keys),
                Name::Nothing => Value::Named(text(node, self.source).to_string()),
            },
            "this" => match self.types.last() {
                Some(this) => typed(Ty::new(&this.symbol, &this.symbol)),
                None => Value::Unknown,
            },
            "super" => match self.types.last() {
                Some(Enclosing {
                    symbol,
                    superclass: Some(superclass),
                }) => typed(Ty::new(superclass, symbol)),
                _ => Value::Unknown,
            },
            "field_access" => {
                let Some(name) = field_text(node, "field", self.source) else {
                    return Value::Unknown;
                };
                if name.eq_ignore_ascii_case("class") {
                    return platform("Type");
                }
                match of(field("object")) {
                    Value::Declared(keys) => match self.inner_types(&keys, &name) {
                        inner if !inner.is_empty() => Value::Declared(inner),
                        _ => self.member_value(&strs(&keys), &name),
                    },
                    Value::Named(qualifier) => Value::Named(format!("{qualifier}.{name}")),
                    Value::Instance(types) => {
                        self.any_of(types.iter().map(|ty| self.field_of(ty, &name)).collect())
                    }
                    Value::Platform => Value::Platform,
                    Value::Unknown => Value::Unknown,
                }
            }
            "method_invocation" => self.call(node).1,
            "object_creation_expression" | "map_creation_expression" | "cast_expression" => {
                match field("type") {
                    Some(ty) => typed(Ty::new(type_text(ty, self.source), self.context())),
                    None => Value::Unknown,
                }
            }
            // `new Foo[3]` and `new Foo[]{..}` are lists of `Foo`;
            // `new List<Foo>{..}` has no dimensions and is what it says.
            "array_creation_expression" => match field("type") {
                Some(ty) => {
                    let dimensions = match field("dimensions") {
                        Some(_) => "[]",
                        None => "",
                    };
                    let ty = format!("{}{dimensions}", type_text(ty, self.source));
                    typed(Ty::new(ty, self.context()))
                }
                None => Value::Unknown,
            },
            "parenthesized_expression" => of(kids(node).first().copied()),
            "ternary_expression" => self.common(field("consequence"), field("alternative")),
            "array_access" => match of(field("array")) {
                Value::Instance(types) => {
                    let elements: Option<Vec<Ty>> = types.iter().map(element).collect();
                    elements.map_or(Value::Unknown, |elements| self.one_of(elements))
                }
                Value::Named(name) => self.foreign_member(name, false).1,
                Value::Platform => Value::Platform,
                _ => Value::Unknown,
            },
            "string_literal" | "multi_line_string_literal" => platform("String"),
            "int" if text(node, self.source).ends_with(['l', 'L']) => platform("Long"),
            "int" => platform("Integer"),
            "decimal_floating_point_literal" => platform("Decimal"),
            "boolean" | "instanceof_expression" => platform("Boolean"),
            "class_literal" => platform("Type"),
            "binary_expression" => {
                let is_string = |value: &Value| {
                    matches!(value, Value::Instance(types)
                        if matches!(&types[..], [only] if only.text.eq_ignore_ascii_case("String")))
                };
                match field("operator").map(|operator| operator.kind()) {
                    Some("==" | "!=" | "===" | "!==" | "<" | "<=" | ">" | ">=" | "<>") => {
                        platform("Boolean")
                    }
                    Some("&&" | "||") => platform("Boolean"),
                    Some("??") => self.common(field("left"), field("right")),
                    operator => match (of(field("left")), of(field("right"))) {
                        (left, right)
                            if operator == Some("+") && (is_string(&left) || is_string(&right)) =>
                        {
                            platform("String")
                        }
                        (Value::Instance(left), Value::Instance(right))
                            if left.len() == 1 && left == right =>
                        {
                            Value::Instance(left)
                        }
                        // Arithmetic on mixed types converts: to some
                        // primitive, but which is not worked out here.
                        _ => Value::Platform,
                    },
                }
            }
            "unary_expression" => match field("operator").map(|operator| operator.kind()) {
                Some("!") => platform("Boolean"),
                _ => of(field("operand")),
            },
            "assignment_expression" => of(field("left")),
            "update_expression" => of(field("operand")),
            "query_expression" => self.query_value(node),
            _ => Value::Unknown,
        }
    }

    /// The type both operands of `a ? b : c` or `b ?? c` have. Where they
    /// differ, their common type is not worked out and neither is claimed.
    fn common(&self, a: Option<Node>, b: Option<Node>) -> Value {
        let is_null = |node: Option<Node>| node.is_some_and(|node| node.kind() == "null_literal");
        let of = |node: Option<Node>| node.map_or(Value::Unknown, |node| self.value(node));
        if is_null(a) || is_null(b) {
            return of(if is_null(a) { b } else { a });
        }
        let is_platform = |value: &Value| match value {
            Value::Instance(types) => types.iter().all(|ty| self.is_platform_type(ty)),
            Value::Platform => true,
            _ => false,
        };
        match (of(a), of(b)) {
            (Value::Instance(a), Value::Instance(b))
                if a.len() == 1 && b.len() == 1 && self.index.is_same_type(&a[0], &b[0]) =>
            {
                Value::Instance(a)
            }
            (a, b) if is_platform(&a) && is_platform(&b) => Value::Platform,
            _ => Value::Unknown,
        }
    }

    /// What a bare name means here: a variable, else a field of this type
    /// or its ancestors, else of the outer type, else a type. Where this
    /// type is tainted, whether it has the field depends on which
    /// declaration is meant, so the outer type's fields count as well.
    fn lookup(&self, name: &str) -> Name<'a> {
        let folded = fold(name);
        if let Some((_, ty)) = self.variables.iter().rev().find(|(n, _)| *n == folded) {
            return Name::Variable(ty.clone());
        }
        let index = self.index;
        let (context, outer) = self.scopes();
        let mut fields = index.fields(&context, name);
        if fields.is_empty() || index.is_tainted(&context) {
            fields.extend(index.fields(&outer, name));
        }
        if !fields.is_empty() {
            return Name::Fields(fields);
        }
        let types = index.resolve_types(name, self.context());
        match types.is_empty() {
            true => Name::Nothing,
            false => Name::Types(types.into_iter().map(String::from).collect()),
        }
    }

    /// The value of the field `name` of one of the declared types `keys`.
    fn member_value(&self, keys: &[&str], name: &str) -> Value {
        let fields = keys.iter().flat_map(|key| self.index.fields(key, name));
        self.one_of(fields.map(field_type).collect())
    }

    /// The value of the field `name` of a value of type `ty`.
    fn field_of(&self, ty: &Ty, name: &str) -> Value {
        let keys = self.index.keys_of(ty);
        match keys.is_empty() {
            false => self.member_value(&keys, name),
            true if self.is_platform_type(ty) => Value::Platform,
            true => Value::Unknown,
        }
    }

    /// A value of one of `types`, which may all be the same type.
    fn one_of(&self, types: Vec<Ty>) -> Value {
        let mut distinct: Vec<Ty> = Vec::new();
        for ty in types {
            if !distinct
                .iter()
                .any(|seen| self.index.is_same_type(seen, &ty))
            {
                distinct.push(ty);
            }
        }
        match distinct.is_empty() {
            true => Value::Unknown,
            false => Value::Instance(distinct),
        }
    }

    /// Whether a type is proven to be a platform class whose members are
    /// platform-typed: not declared in scope, without type arguments, and
    /// either a literal's type or a name on the platform list.
    fn is_platform_type(&self, ty: &Ty) -> bool {
        self.index.keys_of(ty).is_empty()
            && generic(&ty.text).is_none()
            && (ty.context.is_none()
                || self.index.foreign(&ty.text, false).outside == Outside::Platform)
    }

    /// The keys of the types nested in one of `keys` that are called `name`.
    fn inner_types(&self, keys: &[String], name: &str) -> Vec<String> {
        let mut inner: Vec<String> = keys
            .iter()
            .flat_map(|key| self.index.resolve_types(&format!("{key}.{name}"), ""))
            .map(String::from)
            .collect();
        inner.sort();
        inner.dedup();
        inner
    }

    /// The enclosing type, or `""` in a trigger.
    fn context(&self) -> &str {
        self.types.last().map_or("", |this| &this.symbol)
    }

    /// The index keys of the enclosing type and of the type it is nested
    /// in: where an unqualified name is looked for. Either may be `""`.
    fn scopes(&self) -> (String, String) {
        let outer = outer(self.context()).map(fold).unwrap_or_default();
        (fold(self.context()), outer)
    }

    // Rows.

    fn emit(&mut self, kind: RefKind, at: Option<Node>, target: Option<Target>) {
        let (Some(at), Some(target)) = (at, target) else {
            return;
        };
        let kind = target.kind.unwrap_or(kind);
        // A name that Apex does not declare may be one that metadata does:
        // an object, a label, or a field path, which is a row for each
        // field along it.
        let is_open = target.resolution == Resolution::Unresolved && target.detail.is_none();
        let declared = match is_open {
            true => self.catalog.apex(kind, &target.referenced),
            false => None,
        };
        let targets = match declared {
            Some(found) => found.into_iter().map(Target::from).collect(),
            None => vec![target],
        };
        for target in targets {
            let referenced_type = match target.resolution {
                Resolution::Declaration => target.referenced_type,
                _ => target.referenced_type.or(undeclared(kind)),
            };
            self.out.push(Reference {
                tier: Tier::Parsed,
                kind,
                referenced: target.referenced,
                referenced_type,
                resolution: Some(target.resolution),
                detail: target.detail,
                candidates: target.candidates,
                referrer: self.referrer.clone(),
                source_field: None,
                package: self.package.to_string(),
                file: self.file.to_string(),
                line: line(at),
                column: column(at),
            });
        }
    }

    /// An SObject or one of its fields. Apex does not declare them, so they
    /// are `unresolved` here unless in a namespace; `emit` then resolves
    /// what metadata declares.
    fn emit_sobject(&mut self, kind: RefKind, at: Node, name: String, detail: Option<Detail>) {
        let resolution = match self.index.foreign(&name, false).outside {
            Outside::External => Resolution::External,
            _ => Resolution::Unresolved,
        };
        let mut target = Target::new(name, resolution);
        target.detail = detail;
        self.emit(kind, Some(at), Some(target));
    }
}

/// What metadata declares for a name that Apex left unresolved.
impl From<Found> for Target {
    fn from(found: Found) -> Target {
        let mut target = Target::new(found.referenced, found.resolution);
        target.referenced_type = found.referenced_type;
        if !found.candidates.is_empty() {
            target.detail = Some(Detail::AmbiguousReceiver);
            target.candidates = found.candidates;
        }
        target
    }
}

/// What kind of thing a name that nothing in scope declares is, as the
/// syntax says. A type name is in Apex's one type namespace: `class`. An
/// object that metadata declares would have resolved, so the name is not
/// one; it may still be the platform's SObject. `None` where the syntax
/// leaves the kind open: a static access may be to a field, an enum value,
/// a label or an object.
fn undeclared(kind: RefKind) -> Option<ReferencedType> {
    Some(match kind {
        RefKind::Call => ReferencedType::Method,
        RefKind::Construct
        | RefKind::Type
        | RefKind::Cast
        | RefKind::Extends
        | RefKind::Implements => ReferencedType::Class,
        RefKind::SoqlObject | RefKind::DmlObject | RefKind::TriggerObject => ReferencedType::Object,
        RefKind::SoqlField | RefKind::DmlField | RefKind::SobjectField => {
            ReferencedType::ObjectField
        }
        _ => return None,
    })
}

/// What a method of a collection returns: an element, another collection
/// of them, something that is the platform's, or, for a method that is not
/// modelled here, something unknown. `None` if `ty` is not a collection.
fn collection_member(ty: &Ty, method: &str) -> Option<Value> {
    let (collection, arguments) = generic(&ty.text)?;
    if !matches!(
        collection.as_str(),
        "list" | "set" | "map" | "iterable" | "iterator"
    ) {
        return None;
    }
    let of = |text: String| Value::Instance(vec![ty.with_text(&text)]);
    Some(
        match (collection.as_str(), &arguments[..], fold(method).as_str()) {
            ("list", [element], "get" | "remove") => of(element.to_string()),
            ("list" | "set" | "iterable", [element], "iterator") => {
                of(format!("Iterator<{element}>"))
            }
            ("iterator", [element], "next") => of(element.to_string()),
            ("map", [_, value], "get" | "remove" | "put") => of(value.to_string()),
            ("map", [_, value], "values") => of(format!("List<{value}>")),
            ("map", [key, _], "keyset") => of(format!("Set<{key}>")),
            ("list" | "set" | "map", _, "clone" | "deepclone") => Value::Instance(vec![ty.clone()]),
            // Known to return a primitive, a type token or nothing.
            (
                _,
                _,
                "size" | "isempty" | "contains" | "containskey" | "containsall" | "indexof" | "add"
                | "addall" | "clear" | "equals" | "hashcode" | "set" | "sort" | "tostring"
                | "hasnext" | "remove" | "removeall" | "retainall" | "putall" | "getsobjecttype",
            ) => Value::Platform,
            // Any other method: what it returns is not modelled, so not known.
            _ => Value::Unknown,
        },
    )
}

fn field_type(field: &Field) -> Ty {
    Ty::new(&field.ty, &field.owner)
}

/// The symbols of `callables`, sorted, each once.
fn symbols(callables: &[&Callable]) -> Vec<String> {
    let mut symbols: Vec<String> = callables.iter().map(|c| c.symbol.clone()).collect();
    symbols.sort();
    symbols.dedup();
    symbols
}

fn strs(keys: &[String]) -> Vec<&str> {
    keys.iter().map(String::as_str).collect()
}

/// Whether `node` is the `a.b` of a longer `a.b.c` or of `a.b.c()`.
fn is_qualifier<'t>(node: Node<'t>, parent: Option<Node<'t>>) -> bool {
    parent_of(node, parent).is_some_and(|parent| {
        matches!(parent.kind(), "field_access" | "method_invocation")
            && by_field(parent, "object").is_some_and(|object| object.id() == node.id())
    })
}

/// Whether an identifier is the name of what its parent declares or calls,
/// rather than an expression.
fn is_declared_name<'t>(node: Node<'t>, parent: Option<Node<'t>>) -> bool {
    parent_of(node, parent).is_some_and(|parent| {
        let is_name = by_field(parent, "name").is_some_and(|name| name.id() == node.id());
        is_name
            || matches!(
                parent.kind(),
                "when_sobject_type" | "enum_constant" | "break_statement" | "continue_statement"
            )
    })
}

/// The parent of `node`: `known` where the caller has it, else looked up.
fn parent_of<'t>(node: Node<'t>, known: Option<Node<'t>>) -> Option<Node<'t>> {
    debug_assert!(known.is_none_or(|known| Some(known.id()) == node.parent().map(|p| p.id())));
    known.or_else(|| node.parent())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{apex, declare};
    use sf_impact_common::rows::{DeclType, Declaration};

    /// The parsed references of the last source, resolved against all of
    /// them, as "kind referenced" with the resolution when it is not a
    /// declaration, the detail when there is one, and any candidates.
    fn references_of(sources: &[&str]) -> Vec<String> {
        references_with(sources, &[])
    }

    /// As `references_of`, in a repository whose metadata also declares
    /// these objects and object fields.
    fn references_with(sources: &[&str], metadata: &[(DeclType, &str)]) -> Vec<String> {
        resolved(sources, metadata)
            .into_iter()
            .map(|r| {
                let kind = serde_json::to_string(&r.kind).unwrap();
                let mut detail = r.detail.map(|d| format!(" ({d:?})")).unwrap_or_default();
                if !r.candidates.is_empty() {
                    detail += &format!(" {{{}}}", r.candidates.join(" | "));
                }
                let resolution = match r.resolution {
                    Some(Resolution::Declaration) => "",
                    Some(Resolution::External) => " [external]",
                    _ => " [unresolved]",
                };
                format!(
                    "{} {}{resolution}{detail}",
                    kind.trim_matches('"'),
                    r.referenced
                )
            })
            .collect()
    }

    /// The parsed references of the last source as "kind referenced
    /// referenced_type", or "-" where it has none.
    fn typed_of(sources: &[&str]) -> Vec<String> {
        resolved(sources, &[])
            .into_iter()
            .map(|r| {
                format!(
                    "{} {} {}",
                    serde_json::to_string(&r.kind).unwrap().trim_matches('"'),
                    r.referenced,
                    r.referenced_type.map_or("-".to_string(), |t| {
                        serde_json::to_string(&t)
                            .unwrap()
                            .trim_matches('"')
                            .to_string()
                    })
                )
            })
            .collect()
    }

    fn resolved(sources: &[&str], metadata: &[(DeclType, &str)]) -> Vec<Reference> {
        let trees: Vec<_> = sources
            .iter()
            .map(|source| apex::parse(&mut apex::parser(), source.as_bytes()))
            .collect();
        let declared: Vec<_> = trees
            .iter()
            .zip(sources)
            .map(|(tree, source)| declare::declarations(tree, source.as_bytes(), "pkg", "f.cls"))
            .collect();
        let rows: Vec<_> = declared.iter().flat_map(|d| d.rows.clone()).collect();
        let types = declared.into_iter().flat_map(|d| d.types);
        let metadata = metadata.iter().map(|(kind, symbol)| Declaration {
            symbol: symbol.to_string(),
            name: symbol.to_string(),
            kind: *kind,
            package: "pkg".into(),
            file: "f-meta.xml".into(),
            start: 1,
            end: 1,
            is_test: false,
            annotations: Vec::new(),
            collision: false,
            scope: None,
        });
        let rows: Vec<Declaration> = rows.into_iter().chain(metadata).collect();
        let index = Index::build(types, &rows, &["npsp".to_string(), "ns".to_string()]);

        let (tree, source) = (trees.last().unwrap(), sources.last().unwrap());
        let catalog = Catalog::build(&rows, [], &[]);
        references(tree, source.as_bytes(), &index, &catalog, "pkg", "f.cls")
    }

    #[test]
    fn parameter_respelling_preserves_declarations_and_references() {
        for parameter in ["System.List<Id>", "Id[]"] {
            let source = format!(
                "public class Account {{ public Account({parameter} ids) {{}} \
                 public void process({parameter} ids) {{ process(ids); new Account(ids); }} }}"
            );
            let references = resolved(&[&source], &[]);
            let calls: Vec<_> = references
                .iter()
                .filter(|r| matches!(r.kind, RefKind::Call | RefKind::Construct))
                .map(|r| (r.referrer.as_deref(), r.referenced.as_str()))
                .collect();
            assert_eq!(
                calls,
                [
                    (
                        Some("Account.process(list<id>)"),
                        "Account.process(list<id>)"
                    ),
                    (
                        Some("Account.process(list<id>)"),
                        "Account.Account(list<id>)"
                    ),
                ]
            );
        }
    }

    /// `X.class` is a property of the type named `X`: a field or variable
    /// of the same name does not make it a read of that field.
    #[test]
    fn a_class_literal_names_the_type_even_where_a_field_shares_its_name() {
        let shadow = "public class Shadow { public class Name {} public static String Name; \
            static void literalOnly() { System.Type token = Name.class; \
            System.Type qualified = Shadow.Name.class; } }";

        assert_eq!(
            typed_of(&[shadow]),
            ["type Shadow.Name class", "type Shadow.Name class"]
        );
    }

    const FOO: &str =
        "public class Foo { public void go() {} public static Foo make() { return null; } }";

    #[test]
    fn a_call_on_a_local_variable_binds_through_its_declared_type() {
        let found = references_of(&[FOO, "class P { void m() { Foo f = Foo.make(); f.go(); } }"]);

        // In the order the walk meets them, not yet sorted by position.
        assert_eq!(found, ["type Foo", "call Foo.make()", "call Foo.go()"]);
    }

    #[test]
    fn a_chained_call_binds_through_the_declared_return_type() {
        let found = references_of(&[FOO, "class P { void m() { Foo.make().go(); } }"]);

        assert_eq!(found, ["call Foo.go()", "call Foo.make()"]);
    }

    #[test]
    fn a_call_through_a_field_of_this_binds_through_the_field_type() {
        let found = references_of(&[
            FOO,
            "class P { Foo helper; void m() { this.helper.go(); } }",
        ]);

        assert_eq!(found, ["type Foo", "call Foo.go()"]);
    }

    #[test]
    fn variables_go_out_of_scope_with_their_block() {
        let source = "class P { void m() { { Foo f; } f.go(); } }";

        let found = references_of(&[FOO, source]);

        // The second `f` is not the variable; it is an unknown name.
        assert_eq!(found, ["type Foo", "call f.go [unresolved]"]);
    }

    #[test]
    fn loop_and_switch_variables_are_typed() {
        let source = "class P { void m(List<Foo> all, Object o) {
            for (Foo each : all) { each.go(); }
            switch on o { when Foo matched { matched.go(); } }
        } }";

        let found = references_of(&[FOO, source]);

        assert_eq!(
            found,
            [
                "type Foo",
                "type Foo",
                "call Foo.go()",
                "type Foo",
                "call Foo.go()"
            ]
        );
    }

    #[test]
    fn a_catch_variable_is_typed() {
        let oops = "public class Oops extends Exception { public Foo culprit() { return null; } }";
        let source = "class P { void m() {
            try { } catch (Oops caught) { caught.culprit().go(); }
        } }";

        let found = references_of(&[FOO, oops, source]);

        assert_eq!(found, ["type Oops", "call Foo.go()", "call Oops.culprit()"]);
    }

    #[test]
    fn a_setter_sees_its_implicit_value_parameter() {
        let found = references_of(&[FOO, "class P { Foo held { set { value.go(); } } }"]);

        assert_eq!(found, ["type Foo", "call Foo.go()"]);
    }

    #[test]
    fn an_inner_class_is_found_by_its_bare_name_and_sees_the_outer_statics() {
        let source = "class Outer {
            static void help() {}
            class Inner { void go() { help(); } }
            void m(Inner i) { i.go(); }
        }";

        let found = references_of(&[source]);

        assert_eq!(
            found,
            [
                "call Outer.help()",
                "type Outer.Inner",
                "call Outer.Inner.go()"
            ]
        );
    }

    #[test]
    fn a_missing_member_of_a_declared_type_is_unresolved_and_says_why() {
        let found = references_of(&[FOO, "class P { void m() { Foo.vanish(); Foo.GONE.x(); } }"]);

        assert_eq!(
            found,
            [
                "call Foo.vanish [unresolved] (NoMember)",
                // What `Foo.GONE` would be is not known, so neither is `x`.
                "call x [unresolved] (UnknownType)",
                "static_access Foo.GONE [unresolved] (NoMember)"
            ]
        );
    }

    #[test]
    fn a_member_inherited_from_a_platform_supertype_is_not_reported() {
        let source = "class P { void m(Oops e) { e.getMessage(); e.toString(); } }";

        let found = references_of(&["class Oops extends Exception {}", source]);

        assert_eq!(found, ["type Oops"]);
    }

    #[test]
    fn a_declared_class_that_shadows_a_platform_name_does_not_capture_platform_members() {
        // NebulaLogger declares empty `Database` and `Type` classes to test just this.
        let shadows = ["public class Database {}", "public class Type {}"];
        let source = "class P { void m() {
            Object t = Database.SaveResult.class;
            Database.insert(null);
            String name = Foo.class.getName();
        } }";

        let found = references_of(&[shadows[0], shadows[1], FOO, source]);

        // `Database.insert` is still the platform's DML, of a `null` here.
        assert_eq!(
            found,
            ["dml_object SObject [unresolved] (UnknownType)", "type Foo"]
        );
    }

    #[test]
    fn names_that_are_neither_declared_nor_the_platform_are_reported_once_per_chain() {
        let source = "class P { void m() {
            Object a = Account.Name;
            Object b = Gift__c.SObjectType.getDescribe();
            Object c = npsp.Util.VERSION;
            System.debug(Schema.SObjectType.Account.fields.Name);
        } }";

        let found = references_of(&[source]);

        assert_eq!(
            found,
            [
                "static_access Account.Name [unresolved]",
                "call Gift__c.SObjectType.getDescribe [unresolved]",
                "static_access npsp.Util.VERSION [external]",
                // The describe form names the same field as `Account.Name`.
                "static_access Account.Name [unresolved]",
            ]
        );
    }

    #[test]
    fn sosl_names_each_returned_object_and_its_fields() {
        let source = "class P { void m(String term) {
            Object found = [FIND :term IN ALL FIELDS RETURNING Account(Name), Contact];
        } }";

        let found = references_of(&[source]);

        assert_eq!(
            found,
            [
                "soql_object Account [unresolved]",
                "soql_field Account.Name [unresolved]",
                "soql_object Contact [unresolved]"
            ]
        );
    }

    #[test]
    fn typeof_names_the_polymorphic_field_and_each_branch() {
        let source = "class P { void m() {
            Object found = [SELECT TYPEOF What WHEN Account THEN Phone ELSE Name END FROM Event];
        } }";

        let found = references_of(&[source]);

        assert_eq!(
            found,
            [
                "soql_object Event [unresolved]",
                "soql_field Event.What [unresolved]",
                "soql_object Account [unresolved]",
                "soql_field Account.Phone [unresolved]",
                "soql_field Event.What.Name [unresolved]",
            ]
        );
    }

    #[test]
    fn a_dml_operand_of_unknown_type_is_reported_as_some_sobject() {
        let found = references_of(&["class P { void m(List<SObject> rows) { insert rows; } }"]);

        assert_eq!(found, ["dml_object SObject [unresolved] (UnknownType)"]);
    }
    // Regression cases for parsed references.

    #[test]
    fn an_array_creation_is_a_list_and_binds_the_list_overload() {
        let probe = "public class Probe {
            public void pick(Item v) {}
            public void pick(List<Item> v) {}
            public void test(Item[] items) {
                pick(new Item[]{});
                pick(new Item[]{ new Item() });
                pick(new Item[3]);
                pick(items[0]);
            }
        }";

        let found = references_of(&["public class Item {}", probe]);

        let calls: Vec<_> = found.iter().filter(|r| r.starts_with("call")).collect();
        assert_eq!(
            calls,
            [
                "call Probe.pick(list<item>)",
                "call Probe.pick(list<item>)",
                "call Probe.pick(list<item>)",
                "call Probe.pick(item)"
            ]
        );
    }

    const TWINS_A: &str = "public class Twins {
        public static Alpha make() { return null; }
        public static Alpha child;
        public Twins(String s) {}
    }";
    const TWINS_B: &str = "public class Twins {
        public static Beta make() { return null; }
        public static Beta child;
        public Twins(String s) {}
    }";
    const ALPHA: &str = "public class Alpha { public void go() {} }";
    const BETA: &str = "public class Beta { public void go() {} }";

    #[test]
    fn colliding_declarations_that_disagree_keep_every_alternative_downstream() {
        let probe = "public class Probe {
            public void test() {
                Twins.make().go();
                Twins.child.go();
            }
        }";
        let uncertain = "call go (AmbiguousReceiver) {Alpha.go() | Beta.go()}";

        // Whichever declaration is read first, neither is chosen.
        for twins in [[TWINS_A, TWINS_B], [TWINS_B, TWINS_A]] {
            let found = references_of(&[twins[0], twins[1], ALPHA, BETA, probe]);

            assert_eq!(
                found,
                [
                    uncertain,
                    "call Twins.make()",
                    uncertain,
                    "static_access Twins.child"
                ]
            );
        }
    }

    #[test]
    fn a_constructor_declared_twice_is_one_candidate_not_an_ambiguity() {
        let probe = "public class Probe { public void test() { new Twins('x'); } }";

        let found = references_of(&[TWINS_A, TWINS_B, ALPHA, BETA, probe]);

        assert_eq!(found, ["construct Twins.Twins(string)"]);
    }

    #[test]
    fn a_method_declared_twice_with_one_return_type_types_the_call() {
        let twins = "public class Twins { public static Alpha make() { return null; } }";
        let probe = "public class Probe { public void test() { Twins.make().go(); } }";

        let found = references_of(&[twins, twins, ALPHA, probe]);

        assert_eq!(found, ["call Alpha.go()", "call Twins.make()"]);
    }

    #[test]
    fn same_signature_methods_on_unrelated_types_do_not_hide_each_other() {
        // `Twins` extends `Alpha` in one declaration and `Beta` in the other.
        let twins = [
            "public class Twins extends Alpha {}",
            "public class Twins extends Beta {}",
        ];
        let probe = "public class Probe { public void test(Twins t) { t.go(); } }";
        // One declaration, with a superclass and an interface that both
        // declare `go()`: neither is an ancestor of the other.
        let mixed = "public class Mixed extends Alpha implements Going {}";
        let going = "public interface Going { void go(); }";
        let caller = "public class Probe { public void test(Mixed m) { m.go(); } }";

        let through_twins = with_twins(twins, &[ALPHA, BETA], probe);
        let through_mixed = references_of(&[mixed, going, ALPHA, caller]);

        assert_eq!(
            through_twins,
            [
                "type Twins",
                "call go (AmbiguousReceiver) {Alpha.go() | Beta.go()}"
            ]
        );
        assert_eq!(
            through_mixed,
            [
                "type Mixed",
                "call Mixed.go (AmbiguousOverload) {Alpha.go() | Going.go()}"
            ]
        );
    }

    #[test]
    fn objects_and_fields_under_schema_are_references_not_platform_names() {
        let probe = "public class Probe {
            public static void schemaRefs() {
                Schema.MyObject__c obj;
                Object token = Schema.SObjectType.MyObject__c;
                Object field = Schema.SObjectType.MyObject__c.fields.CustomField__c;
                Object nebula = Schema.LogEntry__c.StackTrace__c;
                Object described = SObjectType.MyObject__c.fields.Other__c.getDescribe();
                Object global = Schema.getGlobalDescribe();
                Schema.DescribeFieldResult result;
            }
        }";

        let found = references_of(&[probe]);

        assert_eq!(
            found,
            [
                "type MyObject__c [unresolved]",
                "type MyObject__c [unresolved]",
                "static_access MyObject__c.CustomField__c [unresolved]",
                "static_access LogEntry__c.StackTrace__c [unresolved]",
                "static_access MyObject__c.Other__c [unresolved]",
            ]
        );
    }

    #[test]
    fn a_field_that_metadata_declares_resolves_and_one_it_does_not_stays_unresolved() {
        let probe = "public class Probe {
            public static void tokens() {
                Object declared = Schema.Invoice__c.Total__c;
                Object undeclared = Schema.Invoice__c.Missing__c;
            }
        }";

        let total = (DeclType::ObjectField, "Invoice__c.Total__c");

        let found = references_with(&[probe], &[total]);

        assert_eq!(
            found,
            [
                "static_access Invoice__c.Total__c",
                "static_access Invoice__c.Missing__c [unresolved]",
            ]
        );
    }

    /// Custom metadata types and custom settings are read through static
    /// methods of the object, and an object's token is a member of it.
    #[test]
    fn a_member_of_an_object_that_metadata_declares_is_a_reference_to_the_object() {
        let probe = "public class Probe {
            public static void members() {
                Object token = Settings__c.SObjectType;
                Object defaults = Settings__c.getOrgDefaults();
                Object all = Plugin__mdt.getAll().values().deepClone();
                Object label = Settings__c.Limit__c.getDescribe().getLabel();
            }
        }";
        let declared = [
            (DeclType::Object, "Settings__c"),
            (DeclType::Object, "Plugin__mdt"),
            (DeclType::ObjectField, "Settings__c.Limit__c"),
        ];

        let found = references_with(&[probe], &declared);

        // What follows the object or the field is the platform's.
        assert_eq!(
            found,
            [
                "type Settings__c",
                "type Settings__c",
                "type Plugin__mdt",
                "static_access Settings__c.Limit__c",
            ]
        );
    }

    #[test]
    fn a_method_called_on_a_record_of_a_declared_object_is_the_platforms() {
        let probe = "public class Probe {
            public static void members(Settings__c settings) {
                Object value = settings.get('Limit__c');
                Object copy = settings.clone().getSObjectType();
            }
        }";

        let found = references_with(&[probe], &[(DeclType::Object, "Settings__c")]);

        // The parameter's type, and nothing for the calls.
        assert_eq!(found, ["type Settings__c"]);
    }

    /// Without the object in metadata nothing says `Settings__c` is one:
    /// a class that is missing from the repository looks the same.
    #[test]
    fn a_member_of_a_name_that_nothing_declares_is_reported_as_written() {
        let probe = "public class Probe {
            public static void members(Settings__c settings) {
                Object defaults = Settings__c.getOrgDefaults();
                Object value = settings.get('Limit__c');
            }
        }";

        let found = references_of(&[probe]);

        assert_eq!(
            found,
            [
                "type Settings__c [unresolved]",
                "call Settings__c.getOrgDefaults [unresolved]",
                "call Settings__c.get [unresolved]",
            ]
        );
    }

    /// An Apex class hides an object of the same name, as it does in an
    /// org: a member the class lacks is not the object's field.
    #[test]
    fn a_member_missing_from_a_class_is_not_looked_up_as_a_field_of_an_object() {
        let account = "public class Account {}";
        let probe = "public class Probe {
            public static void tokens() {
                Object field = Account.Region__c;
            }
        }";

        let region = (DeclType::ObjectField, "Account.Region__c");

        let found = references_with(&[account, probe], &[region]);

        assert_eq!(
            found,
            ["static_access Account.Region__c [unresolved] (NoMember)"]
        );
    }

    #[test]
    fn labels_and_flow_interviews_are_references_not_platform_names() {
        let probe = "public class Probe {
            public void test(Map<String, Object> inputs) {
                String a = Label.Greeting;
                String b = System.Label.Farewell;
                Object c = new Flow.Interview.Welcome_Flow(inputs);
            }
        }";

        let found = references_of(&[probe]);

        assert_eq!(
            found,
            [
                "static_access Label.Greeting [unresolved]",
                "static_access Label.Farewell [unresolved]",
                "construct Flow.Interview.Welcome_Flow [unresolved]",
            ]
        );
    }

    #[test]
    fn a_call_on_a_receiver_whose_type_is_not_declared_is_still_a_row() {
        let probe = "public class Probe {
            public void test(ns.Remote remote, Missing other, Object value) {
                remote.run();
                other.run();
                ((Missing)value).go();
            }
        }";

        let found = references_of(&[probe]);

        let calls: Vec<_> = found.iter().filter(|r| r.starts_with("call")).collect();
        assert_eq!(
            calls,
            [
                "call ns.Remote.run [external]",
                "call Missing.run [unresolved]",
                "call Missing.go [unresolved]"
            ]
        );
    }

    #[test]
    fn a_call_on_an_element_of_a_collection_binds_through_the_element_type() {
        let probe = "public class Probe {
            public void test(Map<String, ns.Remote> remotes, List<Foo> foos, Map<Id, List<Foo>> byId) {
                remotes.get('x').run();
                foos.get(0).go();
                byId.values()[0].get(0).go();
                for (Foo each : byId.get(null)) { each.go(); }
                foos.size().format();
            }
        }";

        let found = references_of(&[FOO, probe]);

        let calls: Vec<_> = found.iter().filter(|r| r.starts_with("call")).collect();
        assert_eq!(
            calls,
            [
                "call ns.Remote.run [external]",
                "call Foo.go()",
                "call Foo.go()",
                "call Foo.go()"
            ]
        );
    }

    #[test]
    fn a_call_on_a_receiver_of_unknown_type_is_a_row_whatever_its_name() {
        // `lost` and `x` are declared nowhere: the row does not depend on that.
        let probe = "public class Probe {
            public void test(Missing other) {
                other.make().lost();
                other.thing.x();
            }
        }";

        let found = references_of(&[probe]);

        assert_eq!(
            found,
            [
                "type Missing [unresolved]",
                "call lost [unresolved] (UnknownType)",
                "call Missing.make [unresolved]",
                "call x [unresolved] (UnknownType)",
            ]
        );
    }

    #[test]
    fn what_a_platform_method_returns_is_the_platforms() {
        // A platform class without type arguments returns platform types:
        // Apex has no generic methods. `Type.newInstance()` returns `Object`,
        // and nothing in the repository can be called on that without a cast.
        // This holds only for such classes; see the test that follows the
        // collections for what does not stay the platform's.
        let probe = "public class Probe {
            public void test(String s, Object o) {
                String.valueOf(o).trim().toLowerCase();
                System.Type.forName(s).newInstance().toString();
                s.split(',')[0].trim();
            }
        }";

        assert_eq!(references_of(&[FOO, probe]), Vec::<String>::new());
    }

    #[test]
    fn sobject_initialiser_fields_are_references() {
        let probe = "public class Probe {
            public void test(String name) {
                insert new Contact(LastName = 'x', AccountId = lookup(name));
                Account a = new Account(Name = name);
            }
            Id lookup(String name) { return null; }
        }";

        let found = references_of(&[probe]);

        assert_eq!(
            found,
            [
                "dml_object Contact [unresolved]",
                "construct Contact [unresolved]",
                "sobject_field Contact.LastName [unresolved]",
                "sobject_field Contact.AccountId [unresolved]",
                "call Probe.lookup(string)",
                "type Account [unresolved]",
                "construct Account [unresolved]",
                "sobject_field Account.Name [unresolved]",
            ]
        );
    }

    #[test]
    fn dml_through_the_database_class_names_its_object() {
        let probe = "public class Probe {
            public void test(List<Lead> leads, Id anId) {
                Database.insert(new Account(Name = 'x'));
                System.Database.update(leads, false);
                Database.upsert(leads, Lead.External__c);
                Database.delete(anId);
                Database.setSavepoint();
            }
        }";

        let found = references_of(&[probe]);

        let dml: Vec<_> = found.iter().filter(|r| r.starts_with("dml")).collect();
        assert_eq!(
            dml,
            [
                "dml_object Account [unresolved]",
                "dml_object Lead [unresolved]",
                "dml_object Lead [unresolved]",
                "dml_field Lead.External__c [unresolved]",
                "dml_object SObject [unresolved] (UnknownType)",
            ]
        );
    }

    #[test]
    fn an_unqualified_static_field_is_a_reference_however_short_its_name() {
        let probe = "public class Probe {
            private static Integer rate = 5;
            private Integer count;
            public enum Mode { UP, DOWN }
            public static Integer get() { return rate; }
            public Integer total() { rate += 1; return count * rate; }
            public class Inner { Integer twice() { return rate * 2; } }
        }";

        let found = references_of(&[probe]);

        assert_eq!(
            found,
            [
                "static_access Probe.rate",
                "static_access Probe.rate",
                "static_access Probe.rate",
                "static_access Probe.rate",
            ]
        );
    }

    #[test]
    fn the_body_of_a_trigger_is_walked_for_references() {
        let trigger = "trigger OnAccount on Account (before insert) { Foo.make().go(); }";

        assert_eq!(
            references_of(&[FOO, trigger]),
            [
                "trigger_object Account [unresolved]",
                "call Foo.go()",
                "call Foo.make()"
            ]
        );
    }

    #[test]
    fn a_local_variable_or_parameter_shadows_a_static_field() {
        let probe = "public class Probe {
            private static Integer rate = 5;
            public static Integer local() { Integer rate = 1; return rate; }
            public static Integer param(Integer rate) { return rate; }
            public static Integer loop(List<Integer> all) {
                for (Integer rate : all) { return rate; }
                return 0;
            }
        }";

        assert_eq!(references_of(&[probe]), Vec::<String>::new());
    }

    #[test]
    fn enum_values_in_switch_labels_are_references() {
        let probe = "public class Probe {
            public enum Mode { UP, DOWN, IDLE }
            public void test(Mode mode) {
                switch on mode { when UP, DOWN { return; } when else { return; } }
            }
        }";

        let found = references_of(&[probe]);

        assert_eq!(
            found,
            [
                "type Probe.Mode",
                "static_access Probe.Mode.UP",
                "static_access Probe.Mode.DOWN"
            ]
        );
    }

    #[test]
    fn labels_of_a_switch_on_something_not_declared_here_say_what_they_can() {
        // Found in apex-recipes: a switch on the platform's `TriggerOperation`.
        let probe = "public class Probe {
            public void test(TriggerOperation op, ns.Mode theirs, Missing lost, boolean flag) {
                switch on op { when BEFORE_INSERT { return; } }
                switch on Trigger.operationType { when AFTER_INSERT { return; } }
                switch on theirs { when FAST { return; } }
                switch on lost { when GONE { return; } }
                switch on lost.kind() { when ODD { return; } }
                if (flag) { return; }
            }
        }";

        let found = references_of(&[probe]);

        assert_eq!(
            found,
            [
                "type ns.Mode [external]",
                "type Missing [unresolved]",
                "static_access ns.Mode.FAST [external]",
                "static_access Missing.GONE [unresolved]",
                "call Missing.kind [unresolved]",
                "static_access ODD [unresolved] (UnknownType)",
            ]
        );
    }

    #[test]
    fn a_name_that_resolves_to_nothing_is_reported_not_skipped() {
        let probe = "public class Probe extends ns.Base {
            public Object test() { return inherited; }
        }";
        let closed = "public class Probe { public Object test() { return nowhere; } }";

        assert_eq!(
            references_of(&[probe]),
            [
                "extends ns.Base [external]",
                "static_access Probe.inherited [external]"
            ]
        );
        assert_eq!(
            references_of(&[closed]),
            ["static_access Probe.nowhere [unresolved] (NoMember)"]
        );
    }

    #[test]
    fn a_conditional_whose_branches_differ_does_not_choose_an_overload() {
        let sources = [
            "public virtual class Parent {}",
            "public class Child extends Parent {}",
            "public class Probe {
                public void pick(Parent value) {}
                public void pick(Child value) {}
                public void test(Boolean condition, Child child) {
                    pick(condition ? new Child() : new Parent());
                    pick(condition ? child : null);
                    pick(child ?? new Parent());
                }
            }",
        ];

        let found = references_of(&sources);

        let calls: Vec<_> = found.iter().filter(|r| r.starts_with("call")).collect();
        let unsure = "call Probe.pick (AmbiguousOverload) {Probe.pick(child) | Probe.pick(parent)}";
        assert_eq!(calls, [unsure, "call Probe.pick(child)", unsure]);
    }
    // Member names are short so that the textual tier
    // cannot stand in for a parsed reference that went missing.

    /// Runs `probe` against both orders of the two declarations of `Twins`.
    fn with_twins(twins: [&str; 2], others: &[&str], probe: &str) -> Vec<String> {
        let found: Vec<Vec<String>> = [[twins[0], twins[1]], [twins[1], twins[0]]]
            .iter()
            .map(|twins| {
                let mut sources = twins.to_vec();
                sources.extend(others);
                sources.push(probe);
                references_of(&sources)
            })
            .collect();
        assert_eq!(found[0], found[1], "the order of declarations decided");
        found[0].clone()
    }

    const TWINS_PROBE: &str = "public class Probe {
        public void test(Twins t) {
            t.make().get(0).go();
        }
    }";

    #[test]
    fn collection_returns_that_differ_between_declarations_both_reach_the_next_call() {
        let twins = [
            "public class Twins {public List<Alpha> make() {return null;}}",
            "public class Twins {public List<Beta> make() {return null;}}",
        ];

        let found = with_twins(twins, &[ALPHA, BETA], TWINS_PROBE);

        assert_eq!(
            found,
            [
                "type Twins",
                "call go (AmbiguousReceiver) {Alpha.go() | Beta.go()}",
                "call Twins.make()",
            ]
        );
    }

    #[test]
    fn collections_spelled_with_their_namespace_return_their_elements() {
        let probe = "public class Probe {
            public void test(List<Foo> plain, System.List<Foo> qualified, System.Map<String,Foo> mapped,
                    System.Iterable<Foo> iterable, system.set<Foo> lower, System.List<System.List<Foo>> nested) {
                plain.get(0).go();
                qualified.get(0).go();
                mapped.get('x').go();
                iterable.iterator().next().go();
                lower.iterator().next().go();
                nested[0].get(0).go();
                for (Foo each : mapped.values()) { each.go(); }
            }
        }";

        let found = references_of(&[FOO, probe]);

        let calls: Vec<_> = found.iter().filter(|r| r.starts_with("call")).collect();
        assert_eq!(calls, ["call Foo.go()"; 7]);
    }

    #[test]
    fn a_namespace_in_front_of_a_parameter_type_does_not_change_the_overload() {
        let probe = "public class Probe {
            void pick(System.List<Foo> many) {}
            void pick(System.String name) {}
            void test(List<Foo> foos, String s) { pick(foos); pick(s); pick(new Foo[]{}); }
        }";

        let found = references_of(&[FOO, probe]);

        let calls: Vec<_> = found.iter().filter(|r| r.starts_with("call")).collect();
        assert_eq!(
            calls,
            [
                "call Probe.pick(list<foo>)",
                "call Probe.pick(string)",
                "call Probe.pick(list<foo>)"
            ]
        );
    }

    #[test]
    fn an_override_in_one_declaration_does_not_hide_what_the_other_inherits() {
        let twins = [
            "public class Twins extends Alpha { public override void go() {} }",
            "public class Twins extends Beta {}",
        ];
        let alpha = "public virtual class Alpha { public static Integer rate; public virtual void go() {} }";
        let beta =
            "public virtual class Beta { public static Integer rate; public virtual void go() {} }";
        let probe = "public class Probe {
            public void test(Twins t) {
                t.go();
                Integer x = Twins.rate;
            }
        }";

        let found = with_twins(twins, &[alpha, beta], probe);

        assert_eq!(
            found,
            [
                "type Twins",
                "call go (AmbiguousReceiver) {Alpha.go() | Beta.go() | Twins.go()}",
                "static_access rate (AmbiguousReceiver) {Alpha.rate | Beta.rate}",
            ]
        );
    }

    #[test]
    fn a_bare_static_or_enum_value_reached_through_differing_declarations_names_them_all() {
        let twins = [
            "public virtual class Twins extends Alpha {}",
            "public virtual class Twins extends Beta {}",
        ];
        let alpha = "public virtual class Alpha { public static Integer rate; public enum Mode { UP } public static Mode mode; }";
        let beta = "public virtual class Beta { public static Integer rate; public enum Mode { UP } public static Mode mode; }";
        let probe = "public class Probe extends Twins {
            public Integer test() {
                switch on mode { when UP { return 1; } }
                return rate;
            }
        }";

        let found = with_twins(twins, &[alpha, beta], probe);

        assert_eq!(
            found,
            [
                "extends Twins",
                "static_access mode (AmbiguousReceiver) {Alpha.mode | Beta.mode}",
                "static_access UP (AmbiguousReceiver) {Alpha.Mode.UP | Beta.Mode.UP}",
                "static_access rate (AmbiguousReceiver) {Alpha.rate | Beta.rate}",
            ]
        );
    }

    #[test]
    fn the_same_generic_text_in_two_contexts_is_two_types() {
        let twins = [
            "public class Twins extends Alpha {}",
            "public class Twins extends Beta {}",
        ];
        let alpha = "public virtual class Alpha {
            public class Inner { public void go() {} }
            public List<Inner> make() {return null;}
        }";
        let beta = "public virtual class Beta {
            public class Inner { public void go() {} }
            public List<Inner> make() {return null;}
        }";

        let found = with_twins(twins, &[alpha, beta], TWINS_PROBE);

        assert_eq!(
            found,
            [
                "type Twins",
                "call go (AmbiguousReceiver) {Alpha.Inner.go() | Beta.Inner.go()}",
                "call make (AmbiguousReceiver) {Alpha.make() | Beta.make()}",
            ]
        );
    }

    #[test]
    fn a_nested_type_inherited_differently_by_two_declarations_is_both() {
        let twins = [
            "public class Twins extends Alpha { public Inner make() {return null;} }",
            "public class Twins extends Beta { public Inner make() {return null;} }",
        ];
        let alpha = "public virtual class Alpha { public class Inner { public void go() {} } }";
        let beta = "public virtual class Beta { public class Inner { public void go() {} } }";
        let probe = "public class Probe {
            public void test(Twins t) {
                t.make().go();
            }
        }";
        let inside = "public class Probe extends Twins {
            public void test(Inner i) {
                i.go();
                new Inner();
            }
        }";

        let found = with_twins(twins, &[alpha, beta], probe);
        let named_inside = with_twins(twins, &[alpha, beta], inside);

        let go = "call go (AmbiguousReceiver) {Alpha.Inner.go() | Beta.Inner.go()}";
        assert_eq!(found, ["type Twins", go, "call Twins.make()"]);
        // The name itself is one of two types wherever it is written.
        let inner = "Inner (AmbiguousReceiver) {Alpha.Inner | Beta.Inner}";
        assert_eq!(
            named_inside,
            [
                "extends Twins".to_string(),
                format!("type {inner}"),
                go.to_string(),
                format!("construct {inner}"),
            ]
        );
    }

    #[test]
    fn declarations_that_are_the_same_in_substance_resolve_as_one() {
        // Byte-identical copies in two packages, as public corpora have them.
        let twins = "public class Twins extends Alpha {
            public static Integer rate;
            public Alpha make() { return null; }
        }";
        let probe = "public class Probe {
            public void test(Twins t) { t.make().go(); t.go(); Integer x = Twins.rate; }
        }";

        let found = references_of(&[twins, twins, ALPHA, probe]);

        assert_eq!(
            found,
            [
                "type Twins",
                "call Alpha.go()",
                "call Twins.make()",
                "call Alpha.go()",
                "static_access Twins.rate",
            ]
        );
    }

    #[test]
    fn only_a_proven_platform_type_stays_the_platforms_along_a_chain() {
        let probe = "public class Probe {
            public void test(Comparator<Foo> order, Database.Batchable<Foo> job, List<Foo> foos, Foo a) {
                order.compare(a, a).format();
                job.start(null).iterator().next().go();
                foos.mystery().go();
                foos.size().format();
            }
        }";

        let found = references_of(&[FOO, probe]);

        // A platform type with type arguments, or a collection method that
        // is not modelled, may return anything: what follows is reported.
        let calls: Vec<_> = found.iter().filter(|r| r.starts_with("call")).collect();
        assert_eq!(
            calls,
            [
                "call format [unresolved] (UnknownType)",
                "call go [unresolved] (UnknownType)",
                "call next [unresolved] (UnknownType)",
                "call iterator [unresolved] (UnknownType)",
                "call go [unresolved] (UnknownType)",
            ]
        );
    }
    // Regression case.

    #[test]
    fn a_result_that_may_be_the_platforms_or_the_repositorys_settles_no_overload() {
        let probe = "public class Probe {
            public String make(String x) { return x; }
            public Wrap make(Integer x) { return new Wrap(); }
            public void pick(String x) {}
            public void pick(Foo x) {}
            public void test(Account a) {
                pick(make(a.Name).trim());
                pick(make('x'.trim()).trim());
                pick(make('x'));
                pick(make(1).trim());
            }
        }";
        let wrap = "public class Wrap { public Foo trim() { return new Foo(); } }";

        let found = references_of(&["public class Foo {}", wrap, probe]);

        let calls: Vec<_> = found.iter().filter(|r| r.starts_with("call")).collect();
        // `make(..)` may return a String or a Wrap, so `.trim()` may return
        // a String or a Foo: neither `pick` can be chosen.
        let pick = "call Probe.pick (AmbiguousOverload) {Probe.pick(foo) | Probe.pick(string)}";
        let make = "call Probe.make (AmbiguousOverload) {Probe.make(integer) | Probe.make(string)}";
        assert_eq!(
            calls,
            [
                pick,
                "call Wrap.trim()",
                make,
                pick,
                "call Wrap.trim()",
                make,
                // The controls: where the types are settled, so is the overload.
                "call Probe.pick(string)",
                "call Probe.make(string)",
                "call Probe.pick(foo)",
                "call Wrap.trim()",
                "call Probe.make(integer)",
            ]
        );
    }

    /// Every identifier of `source`, in order, as "text" with " declared"
    /// when `is_declared_name` holds and " qualifier" when `is_qualifier`.
    /// Each is asked both with its parent given and with it looked up.
    fn identifier_roles(source: &str) -> Vec<String> {
        fn walk(node: Node, parent: Option<Node>, source: &[u8], out: &mut Vec<String>) {
            if node.kind() == "identifier" {
                let declared = is_declared_name(node, parent);
                let qualifier = is_qualifier(node, parent);
                assert_eq!(declared, is_declared_name(node, None));
                assert_eq!(qualifier, is_qualifier(node, None));
                let mut role = text(node, source).to_string();
                if declared {
                    role += " declared";
                }
                if qualifier {
                    role += " qualifier";
                }
                out.push(role);
            }
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                walk(child, Some(node), source, out);
            }
        }
        let tree = apex::parse(&mut apex::parser(), source.as_bytes());
        let mut out = Vec::new();
        walk(tree.root_node(), None, source.as_bytes(), &mut out);
        out
    }

    #[test]
    fn declared_names_and_qualifiers_are_told_apart_from_expressions() {
        let source = "class Outer {
            class Inner { Integer count; }
            enum Color { RED, GREEN }
            static Integer total = 0;
            Integer size { get; set; }
            Outer(Integer seed) { this.size = seed; }
            void run(Account acct) {
                Integer local = Outer.total;
                System.debug(acct.Name);
                a.b.c.d();
                x.y.z = Outer.Inner.count;
                for (Integer k : items) { if (k > 0) { break; } continue; }
                switch on acct { when Account a2 { } when else { } }
                List<Account> rows = [SELECT Id FROM Account WHERE Name = :local];
            }
        }";
        assert_eq!(
            identifier_roles(source),
            [
                "Outer declared",
                "Inner declared",
                "count declared",
                "Color declared",
                "RED declared",
                "GREEN declared",
                "total declared",
                "size declared",
                "Outer declared",
                "seed declared",
                "size",
                "seed",
                "run declared",
                "acct declared",
                "local declared",
                // `Outer.total`: the type qualifies, the member does not.
                "Outer qualifier",
                "total",
                "System qualifier",
                // The method a call names is its declared name.
                "debug declared",
                "acct qualifier",
                "Name",
                "a qualifier",
                "b",
                "c",
                "d declared",
                "x qualifier",
                "y",
                "z",
                // A nested type through its outer type.
                "Outer qualifier",
                "Inner",
                "count",
                "k declared",
                "items",
                "k",
                "acct",
                "a2 declared",
                "rows declared",
                "Id",
                "Account",
                "Name",
                "local",
            ]
        );
    }
}
