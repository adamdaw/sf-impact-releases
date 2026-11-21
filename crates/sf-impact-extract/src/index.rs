//! Every Apex type declared in scope, keyed by case-folded name, with what is
//! needed to resolve a reference: members, supertypes and overload binding.
//!
//! The same name can be declared more than once (in two package directories,
//! say). Such declarations share one entry. If they are the same in what
//! matters here (supertypes, signatures, member types) they resolve as one.
//! If they differ the entry is *divergent*, and so is every type that
//! inherits from it. Nothing is then chosen or hidden: a lookup that passes
//! through a divergent entry returns every member reachable from every
//! declaration, without override hiding, and a type name looked up from
//! inside one may mean several types. An org cannot hold two classes of one
//! name, so this trades precision for never preferring the declaration
//! that happened to be read first.

use crate::declare::{Callable, Field, TypeDecl};
use crate::platform::{is_platform, is_schema_class};
use sf_impact_common::rows::{DeclType, Declaration, fold};
pub use sf_impact_common::types::generic;
use sf_impact_common::types::{canonical, is_closed, normal};
use std::collections::{HashMap, HashSet};

/// A type as written, and the type in whose body it was written: `Inner`
/// means `Outer.Inner` inside `Outer` and something else outside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ty {
    pub text: String,
    /// `None` for a type that is the platform's whatever the repository
    /// declares: the `String` of a string literal is never a class `String`.
    pub context: Option<String>,
}

impl Ty {
    pub fn new(text: impl Into<String>, context: impl Into<String>) -> Ty {
        Ty {
            text: text.into(),
            context: Some(context.into()),
        }
    }

    pub fn platform(text: &str) -> Ty {
        Ty {
            text: text.into(),
            context: None,
        }
    }

    /// The same context, different text: the element type of a list, say.
    pub fn with_text(&self, text: &str) -> Ty {
        Ty {
            text: text.into(),
            context: self.context.clone(),
        }
    }
}

/// Where a name that is not declared in scope must live. Ordered so that the
/// greater value is the one to report when a type has several supertypes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Outside {
    /// Nothing points outside the repository.
    Nowhere,
    /// The Apex platform.
    Platform,
    /// A configured installed-package namespace.
    External,
    /// Not known: an SObject, a dependency without a namespace, or a mistake.
    Unknown,
}

/// A dotted name that is not declared in scope, reduced to what it names.
#[derive(Debug, PartialEq, Eq)]
pub struct Foreign {
    pub outside: Outside,
    /// The name to report, without any platform qualifier in front of it:
    /// `Schema.Invoice__c.Total__c` is `Invoice__c.Total__c`.
    pub name: String,
    pub shape: Shape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// The name as written is the reference, last segment included.
    AsWritten,
    /// `name` is an SObject type; what followed it was the platform's.
    Object,
    /// `name` is a member of something the repository owns (an object's
    /// field, a custom label); what followed it was the platform's.
    Member,
}

pub enum Binding<'a> {
    /// One overload. It has several declarations when its type's name is
    /// declared more than once; they are alternatives, not overloads.
    Bound(Vec<&'a Callable>),
    /// Several overloads fit and the argument types do not decide.
    Ambiguous(Vec<&'a Callable>),
    /// No overload takes this many arguments.
    None,
}

struct Entry {
    /// Every declaration of this name; more than one is a collision.
    declarations: Vec<TypeDecl>,
    /// The declarations differ in supertypes, signatures or member types.
    is_divergent: bool,
    /// This type or one of its ancestors is divergent.
    is_tainted: bool,
    /// The name as declared. Where declarations differ in case, the one that
    /// sorts first, so that it does not depend on the order they were read.
    symbol: String,
    /// Keys of this type and every supertype declared in scope, nearest first.
    ancestors: Vec<String>,
    /// The furthest outside the repository that any ancestor's supertype is.
    outside: Outside,
}

pub struct Index {
    // Keys come from untrusted input; seed the hasher against collision floods.
    types: HashMap<String, Entry, foldhash::fast::RandomState>,
    collisions: HashSet<(DeclType, String)>,
    /// The objects that metadata in scope shows to exist, case-folded: one
    /// with an object file, or with a field or a validation rule of its own.
    objects: HashSet<String>,
    symbols: HashMap<(DeclType, String), String>,
    namespaces: Vec<String>,
}

impl Index {
    pub fn build<'a>(
        types: impl IntoIterator<Item = TypeDecl>,
        declarations: impl IntoIterator<Item = &'a Declaration>,
        namespaces: &[String],
    ) -> Index {
        let mut index = Index {
            types: HashMap::default(),
            collisions: HashSet::new(),
            objects: HashSet::new(),
            symbols: HashMap::new(),
            namespaces: namespaces.iter().map(|ns| fold(ns)).collect(),
        };

        for decl in types {
            let entry = index
                .types
                .entry(fold(&decl.symbol))
                .or_insert_with(|| Entry {
                    declarations: Vec::new(),
                    is_divergent: false,
                    is_tainted: false,
                    symbol: decl.symbol.clone(),
                    ancestors: Vec::new(),
                    outside: Outside::Nowhere,
                });
            if decl.symbol < entry.symbol {
                entry.symbol = decl.symbol.clone();
            }
            entry.declarations.push(decl);
        }

        let mut seen = HashSet::new();
        for declaration in declarations {
            let key = collision_key(declaration);
            index
                .symbols
                .entry(key.clone())
                .and_modify(|symbol| {
                    if declaration.symbol < *symbol {
                        *symbol = declaration.symbol.clone();
                    }
                })
                .or_insert_with(|| declaration.symbol.clone());
            if !seen.insert(key.clone()) {
                index.collisions.insert(key);
            }
            let object = match declaration.kind {
                DeclType::Object => Some(declaration.symbol.as_str()),
                DeclType::ObjectField | DeclType::ValidationRule => {
                    declaration.symbol.split('.').next()
                }
                _ => None,
            };
            index.objects.extend(object.map(fold));
        }

        for entry in index.types.values_mut() {
            let declarations = &entry.declarations;
            entry.is_divergent = declarations
                .windows(2)
                .any(|pair| shape(&pair[0]) != shape(&pair[1]));
        }
        index.link_supertypes();
        index
    }

    /// Fills in `ancestors` and `outside` once every type is known. The
    /// supertypes of every declaration of a name count: which of them a
    /// given instance has is not knowable here.
    fn link_supertypes(&mut self) {
        // Direct supertypes first. A supertype name is looked up lexically
        // only (own inner types, the outer type's, then top level): looking
        // through inherited scopes here would need the answer being computed.
        let mut direct: HashMap<&str, (Vec<&str>, Outside)> = HashMap::new();
        for (key, entry) in &self.types {
            let (mut inside, mut outside) = (Vec::new(), Outside::Nowhere);
            for decl in &entry.declarations {
                for supertype in decl.superclass.iter().chain(&decl.interfaces) {
                    let name = head(supertype);
                    let scopes = [Some(decl.symbol.as_str()), outer(&decl.symbol), None];
                    let found = scopes.iter().find_map(|scope| {
                        let candidate = match scope {
                            Some(scope) => fold(&format!("{scope}.{name}")),
                            None => fold(name),
                        };
                        self.types
                            .get_key_value(&candidate)
                            .map(|(k, _)| k.as_str())
                    });
                    match found {
                        Some(found) => inside.push(found),
                        None => outside = outside.max(self.foreign(name, false).outside),
                    }
                }
            }
            // Sorted, so that ancestor order does not depend on file order.
            inside.sort_unstable();
            inside.dedup();
            direct.insert(key, (inside, outside));
        }

        let mut linked = Vec::new();
        for key in self.types.keys() {
            let (mut ancestors, mut outside) = (vec![key.as_str()], Outside::Nowhere);
            let mut next = 0;
            while let Some(current) = ancestors.get(next).copied() {
                let (supertypes, current_outside) = &direct[current];
                outside = outside.max(*current_outside);
                for supertype in supertypes {
                    if !ancestors.contains(supertype) {
                        ancestors.push(supertype);
                    }
                }
                next += 1;
            }
            let ancestors: Vec<String> = ancestors.into_iter().map(String::from).collect();
            linked.push((key.clone(), ancestors, outside));
        }
        for (key, ancestors, outside) in linked {
            let is_tainted = ancestors.iter().any(|key| self.types[key].is_divergent);
            let entry = self.types.get_mut(&key).expect("key came from the map");
            entry.ancestors = ancestors;
            entry.outside = outside;
            entry.is_tainted = is_tainted;
        }
    }

    /// The keys of the declared types that `name` may mean when written
    /// inside `context`: an inner type of the context, of its supertypes, of
    /// its outer type, or a top-level type; the nearest of those, as in
    /// Apex. Where the context is tainted, which is nearest depends on which
    /// declaration is meant, so every one of them is returned. `name`
    /// carries no type arguments. Sorted; empty if nothing is declared.
    pub fn resolve_types(&self, name: &str, context: &str) -> Vec<&str> {
        let name = fold(name);
        let context = fold(context);
        let outer = outer(&context).map(String::from).unwrap_or_default();
        let mut declared = self
            .ancestors(&context)
            .into_iter()
            .chain(self.ancestors(&outer))
            .map(|scope| format!("{scope}.{name}"))
            .chain([name.clone()])
            .filter_map(|candidate| self.types.get_key_value(&candidate))
            .map(|(key, _)| key.as_str());
        if !self.is_tainted(&context) && !self.is_tainted(&outer) {
            return declared.next().into_iter().collect();
        }
        let mut found: Vec<&str> = declared.collect();
        found.sort_unstable();
        found.dedup();
        found
    }

    /// Whether lookups through `key` pass through a divergent entry: `key`
    /// or one of its ancestors is declared more than once, differently.
    pub fn is_tainted(&self, key: &str) -> bool {
        self.types.get(key).is_some_and(|entry| entry.is_tainted)
    }

    /// `key` and the keys of its supertypes declared in scope, nearest first.
    /// Empty if `key` is not a declared type.
    pub fn ancestors(&self, key: &str) -> Vec<&str> {
        match self.types.get(key) {
            Some(entry) => entry.ancestors.iter().map(String::as_str).collect(),
            None => Vec::new(),
        }
    }

    /// The declared types a value of type `ty` may be an instance of:
    /// usually one, none for a type that is not declared in scope, and
    /// several only where its name is looked up through a divergent entry.
    pub fn keys_of(&self, ty: &Ty) -> Vec<&str> {
        match &ty.context {
            Some(context) if generic(&ty.text).is_none() => self.resolve_types(&ty.text, context),
            _ => Vec::new(),
        }
    }

    /// The declared spelling of a type, given its key.
    pub fn symbol(&self, key: &str) -> &str {
        self.types.get(key).map_or("", |entry| &entry.symbol)
    }

    /// See [`Outside`]: where members that `key` does not declare may come from.
    pub fn outside(&self, key: &str) -> Outside {
        self.types
            .get(key)
            .map_or(Outside::Nowhere, |entry| entry.outside)
    }

    /// Methods called `name` on `key` and its ancestors. A method hides one
    /// with the same signature in an ancestor of its type, and only there:
    /// same-signature methods of two unrelated supertypes are both returned.
    /// Through a divergent entry nothing hides anything.
    pub fn methods(&self, key: &str, name: &str) -> Vec<&Callable> {
        let name = fold(name);
        self.visible(
            key,
            |decl| decl.methods.iter().filter(|m| m.key == name).collect(),
            |nearer, further| signature(nearer) == signature(further),
        )
    }

    /// Fields called `name` on `key` and its ancestors, nearest first, with
    /// the same hiding as methods.
    pub fn fields(&self, key: &str, name: &str) -> Vec<&Field> {
        let name = fold(name);
        self.visible(
            key,
            |decl| decl.fields.iter().filter(|f| f.key == name).collect(),
            |_, _| true,
        )
    }

    /// The members that `pick` selects from `key` and its ancestors, less
    /// those hidden by a member of a type that inherits from theirs. Where
    /// `key` is tainted, which type inherits from which depends on which
    /// declaration is meant, so nothing is hidden.
    fn visible<'a, T>(
        &'a self,
        key: &str,
        pick: impl Fn(&'a TypeDecl) -> Vec<&'a T>,
        hides: impl Fn(&T, &T) -> bool,
    ) -> Vec<&'a T> {
        let hiding = !self.is_tainted(key);
        let mut found: Vec<(&str, &T)> = Vec::new();
        for owner in self.ancestors(key) {
            for member in self.declarations(owner).flat_map(&pick) {
                let is_hidden = hiding
                    && found.iter().any(|(nearer, other)| {
                        *nearer != owner
                            && self.ancestors(nearer).contains(&owner)
                            && hides(other, member)
                    });
                if !is_hidden {
                    found.push((owner, member));
                }
            }
        }
        found.into_iter().map(|(_, member)| member).collect()
    }

    /// The constructors of every declaration of `key`.
    pub fn constructors(&self, key: &str) -> Vec<&Callable> {
        self.declarations(key)
            .flat_map(|decl| &decl.constructors)
            .collect()
    }

    /// The declarations of `key` to resolve against: all of them where they
    /// differ, one where they are the same in substance.
    fn declarations(&self, key: &str) -> impl Iterator<Item = &TypeDecl> {
        self.types.get(key).into_iter().flat_map(|entry| {
            let count = match entry.is_divergent {
                true => entry.declarations.len(),
                false => 1,
            };
            entry.declarations.iter().take(count)
        })
    }

    /// What a dotted name that is not declared in scope refers to. A name is
    /// the platform's only when all of it is: a platform class or namespace
    /// in front does not settle it, because `Schema.Invoice__c`, the describe
    /// form `SObjectType.Invoice__c.fields.Total__c`, `Label.Greeting` and
    /// `Flow.Interview.Welcome` all name things a repository owns.
    ///
    /// `ends_in_method` says the last segment is a method being called.
    pub fn foreign(&self, name: &str, ends_in_method: bool) -> Foreign {
        let mut segments: Vec<&str> = name.split('.').collect();
        // Segments that could be an object, a field or a label: not the method.
        let named = |segments: &[&str]| segments.len() - usize::from(ends_in_method);
        let is = |segment: &str, word: &str| segment.eq_ignore_ascii_case(word);

        // `Schema.` and `System.` qualify; they are not part of the name.
        let mut is_schema_qualified = false;
        if named(&segments) >= 2 {
            let (first, second) = (segments[0], fold(segments[1]));
            let is_system = is(first, "System")
                && matches!(second.as_str(), "label" | "schema" | "sobjecttype");
            if is_system {
                segments.remove(0);
            }
        }
        if named(&segments) >= 2 && is(segments[0], "Schema") {
            let second = fold(segments[1]);
            if second == "sobjecttype" || !is_schema_class(&second) {
                segments.remove(0);
                is_schema_qualified = true;
            }
        }

        let record = |object: &str, shape, name: String| Foreign {
            outside: match self.classify(object) {
                Outside::External => Outside::External,
                _ => Outside::Unknown,
            },
            name,
            shape,
        };
        // The describe form: `SObjectType.Object[.fields.Field]`.
        if named(&segments) >= 2 && is(segments[0], "SObjectType") {
            let object = segments[1];
            let member = (named(&segments) >= 4).then(|| (fold(segments[2]), segments[3]));
            return match member {
                Some((group, field)) if group == "fields" => {
                    record(object, Shape::Member, format!("{object}.{field}"))
                }
                Some((group, set)) if group == "fieldsets" => {
                    record(object, Shape::Member, format!("{object}.fieldSets.{set}"))
                }
                _ => record(object, Shape::Object, object.to_string()),
            };
        }
        // What `Schema.` qualifies is an object, and so is a name that
        // metadata declares as one: `Object` is its type,
        // `Object.SObjectType` its token, `Object.Field` the token of one
        // of its fields, and `Object.getAll()` a method of the platform's.
        // Not a name that is also a platform class (`Site`, `Network`):
        // without the qualifier Apex means the class.
        let is_declared_object = self.is_object(segments[0]) && !is_platform(&fold(segments[0]));
        let is_object = is_schema_qualified || is_declared_object;
        if is_object && !is(segments[0], "SObjectType") {
            let object = segments[0];
            return match segments.get(1).filter(|_| named(&segments) >= 2) {
                Some(field) if !is(field, "SObjectType") => {
                    record(object, Shape::Member, format!("{object}.{field}"))
                }
                _ => record(object, Shape::Object, object.to_string()),
            };
        }
        // A custom label is a string; what follows its name is the platform's.
        if named(&segments) >= 2 && is(segments[0], "Label") {
            return Foreign {
                outside: Outside::Unknown,
                name: segments[..2].join("."),
                shape: Shape::Member,
            };
        }
        let is_flow =
            named(&segments) >= 3 && is(segments[0], "Flow") && is(segments[1], "Interview");
        Foreign {
            outside: match is_flow {
                true => Outside::Unknown,
                false => self.classify(segments[0]),
            },
            name: segments.join("."),
            shape: Shape::AsWritten,
        }
    }

    /// Where a single name that is not declared in scope lives: `ns` or
    /// `ns__Object__c` in a configured namespace, a platform name, or neither.
    fn classify(&self, name: &str) -> Outside {
        let name = fold(name);
        // `ns__Name__c` is in namespace `ns`; `Name__c` is in none.
        let prefix = match name.split("__").collect::<Vec<_>>()[..] {
            [prefix, _, _, ..] => prefix.to_string(),
            _ => name.clone(),
        };
        if self.namespaces.contains(&name) || self.namespaces.contains(&prefix) {
            Outside::External
        } else if is_platform(&name) {
            Outside::Platform
        } else {
            Outside::Unknown
        }
    }

    /// Whether metadata in scope shows `name` to be an object.
    pub fn is_object(&self, name: &str) -> bool {
        self.objects.contains(&fold(name))
    }

    pub fn declared(&self, kind: DeclType, name: &str) -> Option<&str> {
        self.symbols.get(&(kind, fold(name))).map(String::as_str)
    }

    pub fn collides(&self, declaration: &Declaration) -> bool {
        self.collisions.contains(&collision_key(declaration))
    }

    /// Chooses among overloads by argument count, then by argument types.
    /// `None` is an argument whose type is not known; it fits any parameter.
    /// An overload is only ruled out when a known type certainly does not fit.
    ///
    /// `is_tainted` says the overloads come from declarations that differ.
    /// Which of them exist together is then not known, so an exact match is
    /// not preferred over the others that fit: it may not be there.
    pub fn bind<'a>(
        &self,
        overloads: &[&'a Callable],
        args: &[Option<Ty>],
        is_tainted: bool,
    ) -> Binding<'a> {
        let by_count: Vec<&Callable> = overloads
            .iter()
            .copied()
            .filter(|overload| overload.params.len() == args.len())
            .collect();
        if by_count.is_empty() {
            return Binding::None;
        }
        if is_one_overload(&by_count) {
            return Binding::Bound(by_count);
        }

        let params = |overload: &'a Callable| {
            overload
                .params
                .iter()
                .map(|param| Ty::new(param, &overload.owner))
                .zip(args)
        };
        // Apex prefers the overload whose parameter types match exactly.
        let exact: Vec<&Callable> = by_count
            .iter()
            .copied()
            .filter(|overload| {
                params(overload).all(|(param, arg)| {
                    arg.as_ref()
                        .is_some_and(|arg| self.is_same_type(arg, &param))
                })
            })
            .collect();
        if !is_tainted && !exact.is_empty() && is_one_overload(&exact) {
            return Binding::Bound(exact);
        }

        let fitting: Vec<&Callable> = by_count
            .iter()
            .copied()
            .filter(|overload| {
                params(overload)
                    .all(|(param, arg)| arg.as_ref().is_none_or(|arg| self.accepts(&param, arg)))
            })
            .collect();
        if fitting.is_empty() {
            Binding::Ambiguous(by_count)
        } else if is_one_overload(&fitting) {
            Binding::Bound(fitting)
        } else {
            Binding::Ambiguous(fitting)
        }
    }

    /// Whether two types, each written in its own context, are certainly
    /// the same type. The same text in two contexts is not: `List<Inner>`
    /// is a list of whatever `Inner` means where it was written.
    pub fn is_same_type(&self, a: &Ty, b: &Ty) -> bool {
        if a == b {
            return true;
        }
        match (generic(&a.text), generic(&b.text)) {
            (Some((a_name, a_arguments)), Some((b_name, b_arguments))) => {
                a_name == b_name
                    && a_arguments.len() == b_arguments.len()
                    && a_arguments
                        .iter()
                        .zip(&b_arguments)
                        .all(|(a_argument, b_argument)| {
                            self.is_same_type(&a.with_text(a_argument), &b.with_text(b_argument))
                        })
            }
            (None, None) => match (&self.keys_of(a)[..], &self.keys_of(b)[..]) {
                ([], []) => normal(&a.text) == normal(&b.text),
                ([a], [b]) => a == b,
                _ => false,
            },
            _ => false,
        }
    }

    /// Whether a parameter of type `param` could take an argument of type
    /// `arg`. False only when that is certain; types that are not understood
    /// (the platform's classes, SObjects) are given the benefit of the doubt.
    fn accepts(&self, param: &Ty, arg: &Ty) -> bool {
        let (param_head, arg_head) = (head_name(&param.text), head_name(&arg.text));
        if param_head == "object" {
            return true;
        }
        let (param_keys, arg_keys) = (self.keys_of(param), self.keys_of(arg));
        // A name that may mean several types rules nothing out.
        if param_keys.len() > 1 || arg_keys.len() > 1 {
            return true;
        }
        // A record is not a primitive, a collection or an Apex class.
        if param_head == "sobject" {
            return arg_keys.is_empty() && !is_closed(&arg_head);
        }
        match (param_keys.first(), arg_keys.first()) {
            (Some(param), Some(arg)) => self.ancestors(arg).contains(param),
            (Some(_), None) => !is_closed(&arg_head),
            (None, Some(_)) => !is_closed(&param_head),
            (None, None) => match (is_closed(&param_head), is_closed(&arg_head)) {
                // Two collections of the same kind fit if their elements do.
                (true, true) if param_head == arg_head => match (element(param), element(arg)) {
                    (Some(param), Some(arg)) => self.accepts(&param, &arg),
                    _ => true,
                },
                (true, true) => widens(&arg_head, &param_head),
                // Nothing but a primitive or a collection converts to one:
                // an `Account` is neither a `String` nor a `List<SObject>`.
                (true, false) => false,
                (false, _) => true,
            },
        }
    }
}

/// What two declarations of one name must share to resolve as one: their
/// supertypes, the signatures and return types of their methods and
/// constructors, and the types of their fields.
fn shape(decl: &TypeDecl) -> impl PartialEq + use<> {
    fn sorted<T: Ord>(mut items: Vec<T>) -> Vec<T> {
        items.sort();
        items
    }
    let text = |ty: &Option<String>| ty.as_deref().map(normal);
    let callables = |callables: &[Callable]| {
        let described = callables
            .iter()
            .map(|c| (c.key.clone(), signature(c), text(&c.returns)));
        sorted(described.collect())
    };
    let fields = decl
        .fields
        .iter()
        .map(|field| (field.key.clone(), normal(&field.ty), field.is_static));
    (
        text(&decl.superclass),
        sorted(decl.interfaces.iter().map(|ty| normal(ty)).collect()),
        callables(&decl.methods),
        callables(&decl.constructors),
        sorted(fields.collect()),
    )
}

/// Whether these are all the same overload: one signature on one type. More
/// than one of them means the type's name is declared more than once.
fn is_one_overload(callables: &[&Callable]) -> bool {
    let identity = |callable: &Callable| (fold(&callable.owner), signature(callable));
    callables
        .windows(2)
        .all(|pair| identity(pair[0]) == identity(pair[1]))
}

/// What distinguishes one overload of a name from another.
fn signature(callable: &Callable) -> Vec<String> {
    callable.params.iter().map(|param| normal(param)).collect()
}

/// The element type of a list, set, iterable, iterator or array type.
pub fn element(ty: &Ty) -> Option<Ty> {
    match generic(&ty.text)? {
        (collection, arguments) if arguments.len() == 1 => {
            let is_sequence = matches!(
                collection.as_str(),
                "list" | "set" | "iterable" | "iterator"
            );
            is_sequence.then(|| ty.with_text(arguments[0]))
        }
        _ => None,
    }
}

/// A type without its type arguments, as written. An array is a list.
pub fn head(text: &str) -> &str {
    if text.ends_with("[]") {
        "List"
    } else {
        text.split('<').next().unwrap_or(text)
    }
}

/// The canonical name of a type without its type arguments: see [`canonical`].
fn head_name(text: &str) -> String {
    canonical(head(text))
}

/// The type a nested type is declared in.
pub fn outer(symbol: &str) -> Option<&str> {
    symbol.rsplit_once('.').map(|(outer, _)| outer)
}

/// Classes, interfaces and enums share one namespace, so a class and an
/// interface of the same name collide.
fn collision_key(declaration: &Declaration) -> (DeclType, String) {
    let kind = match declaration.kind {
        DeclType::Interface | DeclType::Enum => DeclType::Class,
        other => other,
    };
    (kind, fold(&declaration.symbol))
}

/// Implicit conversions Apex applies to arguments.
fn widens(from: &str, to: &str) -> bool {
    matches!(
        (from, to),
        ("integer", "long" | "double" | "decimal")
            | ("long", "double" | "decimal")
            | ("double", "decimal")
            | ("id", "string")
            | ("string", "id")
            | ("date", "datetime")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn method(owner: &str, name: &str, params: &[&str]) -> Callable {
        let params: Vec<String> = params.iter().map(|p| p.to_string()).collect();
        Callable {
            symbol: crate::apex::callable_symbol(owner, name, &params),
            owner: owner.into(),
            name: name.into(),
            key: fold(name),
            params,
            returns: None,
        }
    }

    fn class(symbol: &str, superclass: Option<&str>, methods: Vec<Callable>) -> TypeDecl {
        TypeDecl {
            symbol: symbol.into(),
            superclass: superclass.map(String::from),
            interfaces: Vec::new(),
            methods,
            constructors: Vec::new(),
            fields: Vec::new(),
        }
    }

    fn index(types: Vec<TypeDecl>) -> Index {
        Index::build(types, [], &["npsp".to_string()])
    }

    /// What `Index::bind` decided, with symbols standing in for the
    /// declarations: whether it chose, could not choose, or found nothing
    /// is part of the answer.
    #[derive(Debug, PartialEq)]
    enum Outcome {
        Bound(Vec<String>),
        Ambiguous(Vec<String>),
        None,
    }

    fn bound(symbol: &str) -> Outcome {
        Outcome::Bound(vec![symbol.to_string()])
    }

    fn ambiguous(symbols: &[&str]) -> Outcome {
        Outcome::Ambiguous(symbols.iter().map(|symbol| symbol.to_string()).collect())
    }

    fn bind(index: &Index, overloads: &[Callable], args: &[Option<&str>]) -> Outcome {
        let overloads: Vec<&Callable> = overloads.iter().collect();
        let args: Vec<Option<Ty>> = args.iter().map(|a| a.map(|a| Ty::new(a, ""))).collect();
        let symbols =
            |callables: Vec<&Callable>| callables.iter().map(|c| c.symbol.clone()).collect();
        match index.bind(&overloads, &args, false) {
            Binding::Bound(declarations) => Outcome::Bound(symbols(declarations)),
            Binding::Ambiguous(overloads) => Outcome::Ambiguous(symbols(overloads)),
            Binding::None => Outcome::None,
        }
    }

    #[test]
    fn a_single_overload_with_the_right_count_binds_whatever_the_argument_types() {
        let overloads = [method("A", "m", &["String"]), method("A", "m", &[])];

        let outcome = bind(&index(vec![]), &overloads, &[None]);

        assert_eq!(outcome, bound("A.m(string)"));
    }

    #[test]
    fn known_argument_types_choose_between_overloads_of_equal_count() {
        let overloads = [
            method("A", "m", &["String"]),
            method("A", "m", &["Integer"]),
        ];

        let outcome = bind(&index(vec![]), &overloads, &[Some("Integer")]);

        assert_eq!(outcome, bound("A.m(integer)"));
    }

    #[test]
    fn an_exact_match_beats_a_widening_conversion() {
        let overloads = [
            method("A", "m", &["Decimal"]),
            method("A", "m", &["Integer"]),
        ];

        let outcome = bind(&index(vec![]), &overloads, &[Some("Integer")]);

        assert_eq!(outcome, bound("A.m(integer)"));
    }

    #[test]
    fn an_unknown_argument_type_leaves_every_fitting_overload_as_a_candidate() {
        let overloads = [
            method("A", "m", &["String"]),
            method("A", "m", &["Integer"]),
        ];

        let outcome = bind(&index(vec![]), &overloads, &[None]);

        assert_eq!(outcome, ambiguous(&["A.m(string)", "A.m(integer)"]));
    }

    #[test]
    fn platform_classes_that_are_not_understood_rule_each_other_out_never() {
        let overloads = [
            method("A", "m", &["Exception"]),
            method("A", "m", &["Database.Error"]),
        ];

        let outcome = bind(&index(vec![]), &overloads, &[Some("DmlException")]);

        assert_eq!(
            outcome,
            ambiguous(&["A.m(exception)", "A.m(database.error)"])
        );
    }

    #[test]
    fn a_record_is_told_apart_from_a_list_of_records_and_from_a_string() {
        let overloads = [
            method("A", "m", &["SObject"]),
            method("A", "m", &["List<SObject>"]),
            method("A", "m", &["String"]),
        ];
        let index = index(vec![]);

        assert_eq!(
            bind(&index, &overloads, &[Some("Account")]),
            bound("A.m(sobject)")
        );
        assert_eq!(
            bind(&index, &overloads, &[Some("List<Account>")]),
            bound("A.m(list<sobject>)")
        );
        assert_eq!(
            bind(&index, &overloads, &[Some("Id")]),
            bound("A.m(string)")
        );
    }

    #[test]
    fn lists_are_told_apart_by_their_elements_where_those_are_understood() {
        let overloads = [
            method("A", "m", &["List<String>"]),
            method("A", "m", &["List<Database.SaveResult>"]),
            method("A", "m", &["List<Database.UpsertResult>"]),
        ];
        let index = index(vec![]);

        // Whether one platform class extends another is not known here.
        assert_eq!(
            bind(&index, &overloads, &[Some("List<Database.DeleteResult>")]),
            ambiguous(&[
                "A.m(list<database.saveresult>)",
                "A.m(list<database.upsertresult>)"
            ])
        );
        assert_eq!(
            bind(&index, &overloads, &[Some("String[]")]),
            bound("A.m(list<string>)")
        );
    }

    #[test]
    fn a_subclass_argument_fits_a_superclass_parameter_but_not_an_unrelated_one() {
        let index = index(vec![
            class("Base", None, vec![]),
            class("Derived", Some("Base"), vec![]),
            class("Other", None, vec![]),
        ]);
        let overloads = [method("A", "m", &["Base"]), method("A", "m", &["Other"])];

        let outcome = bind(&index, &overloads, &[Some("Derived")]);

        assert_eq!(outcome, bound("A.m(base)"));
    }

    #[test]
    fn a_literal_is_a_platform_type_even_if_a_class_shares_its_name() {
        let index = index(vec![class("String", None, vec![])]);
        let overloads = [
            method("A", "m", &["String"]),
            method("A", "m", &["Integer"]),
        ];

        let literal = [Some(Ty::platform("Integer"))];
        let Binding::Bound(chosen) =
            index.bind(&overloads.iter().collect::<Vec<_>>(), &literal, false)
        else {
            panic!("an Integer literal should choose m(integer)");
        };

        assert_eq!(chosen[0].symbol, "A.m(integer)");
        assert!(index.keys_of(&Ty::platform("String")).is_empty());
        assert_eq!(index.keys_of(&Ty::new("String", "A")), ["string"]);
    }

    #[test]
    fn no_overload_with_the_right_count_binds_nothing() {
        let overloads = [method("A", "m", &["String"])];

        assert_eq!(
            bind(&index(vec![]), &overloads, &[None, None]),
            Outcome::None
        );
    }

    #[test]
    fn an_override_hides_the_inherited_method_and_keeps_other_overloads() {
        let index = index(vec![
            class(
                "Base",
                None,
                vec![method("Base", "m", &["String"]), method("Base", "m", &[])],
            ),
            class(
                "Derived",
                Some("Base"),
                vec![method("Derived", "M", &["string"])],
            ),
        ]);

        let found: Vec<&str> = index
            .methods("derived", "m")
            .iter()
            .map(|m| m.symbol.as_str())
            .collect();

        assert_eq!(found, ["Derived.M(string)", "Base.m()"]);
    }

    #[test]
    fn an_inner_type_name_resolves_inside_its_outer_type_and_its_subclasses() {
        let index = index(vec![
            class("Outer", None, vec![]),
            class("Outer.Inner", None, vec![]),
            class("Sub", Some("Outer"), vec![]),
        ]);

        assert_eq!(index.resolve_types("INNER", "Outer"), ["outer.inner"]);
        assert_eq!(index.resolve_types("Inner", "Sub"), ["outer.inner"]);
        assert!(index.resolve_types("Inner", "Elsewhere").is_empty());
    }

    #[test]
    fn a_supertype_cycle_does_not_hang() {
        let index = index(vec![
            class("A", Some("B"), vec![]),
            class("B", Some("A"), vec![]),
        ]);

        assert_eq!(index.ancestors("a"), ["a", "b"]);
    }

    #[test]
    fn a_type_reports_the_furthest_outside_supertype_of_its_ancestors() {
        let index = index(vec![
            class("Mine", Some("Exception"), vec![]),
            class("Theirs", Some("npsp.Base"), vec![]),
            class("Lost", Some("Missing"), vec![]),
            class("Child", Some("Lost"), vec![]),
            class("Plain", None, vec![]),
        ]);

        assert_eq!(index.outside("mine"), Outside::Platform);
        assert_eq!(index.outside("theirs"), Outside::External);
        assert_eq!(index.outside("child"), Outside::Unknown);
        assert_eq!(index.outside("plain"), Outside::Nowhere);
    }

    fn named(index: &Index, name: &str) -> (Outside, String, Shape) {
        let foreign = index.foreign(name, false);
        (foreign.outside, foreign.name, foreign.shape)
    }

    #[test]
    fn a_name_in_a_configured_namespace_is_external() {
        let index = index(vec![]);

        assert_eq!(named(&index, "npsp.Thing.VERSION").0, Outside::External);
        assert_eq!(named(&index, "npsp__Gift__c").0, Outside::External);
        assert_eq!(named(&index, "Gift__c").0, Outside::Unknown);
        // One separator is a suffix, not a namespace prefix.
        assert_eq!(named(&index, "npsp__c").0, Outside::Unknown);
    }

    #[test]
    fn a_name_is_the_platforms_only_when_all_of_it_is() {
        let index = index(vec![]);
        let platform = |name| index.foreign(name, true).outside == Outside::Platform;

        assert!(platform("System.debug"));
        assert!(platform("Schema.getGlobalDescribe"));
        assert!(platform("Schema.DescribeFieldResult.getLabel"));
        assert_eq!(named(&index, "Schema.SObjectType").0, Outside::Platform);
        assert_eq!(named(&index, "Schema.SObjectField").0, Outside::Platform);
    }

    #[test]
    fn schema_in_front_of_an_object_is_a_qualifier_not_the_name() {
        let index = index(vec![]);
        let unknown = |name: &str, shape| (Outside::Unknown, name.to_string(), shape);

        assert_eq!(
            named(&index, "Schema.Invoice__c"),
            unknown("Invoice__c", Shape::Object)
        );
        assert_eq!(
            named(&index, "Schema.Invoice__c.Total__c"),
            unknown("Invoice__c.Total__c", Shape::Member)
        );
        assert_eq!(
            named(&index, "System.Schema.Account.SObjectType"),
            unknown("Account", Shape::Object)
        );
        // What comes after the field token is the platform's.
        let described = index.foreign("Schema.Invoice__c.Total__c.getDescribe", true);
        assert_eq!(
            (described.name.as_str(), described.shape),
            ("Invoice__c.Total__c", Shape::Member)
        );
        // Without the qualifier, `Invoice__c` is not known to be an object.
        assert_eq!(
            named(&index, "Invoice__c.Total__c"),
            unknown("Invoice__c.Total__c", Shape::AsWritten)
        );
        assert_eq!(named(&index, "Schema.npsp__Gift__c").0, Outside::External);
    }

    #[test]
    fn a_name_that_metadata_declares_as_an_object_needs_no_qualifier_to_be_one() {
        let declared = |kind, symbol: &str| Declaration {
            symbol: symbol.into(),
            name: symbol.into(),
            kind,
            package: String::new(),
            file: String::new(),
            start: 1,
            end: 1,
            is_test: false,
            annotations: Vec::new(),
            collision: false,
            scope: None,
        };
        let declarations = [
            declared(DeclType::Object, "Settings__c"),
            // Only a field of `Account` and a rule of `Contact` are in the
            // repository.
            declared(DeclType::ObjectField, "Account.Region__c"),
            declared(DeclType::ValidationRule, "Contact.Needs_Email"),
        ];
        let index = Index::build([], &declarations, &[]);
        let unknown = |name: &str, shape| (Outside::Unknown, name.to_string(), shape);

        assert_eq!(
            named(&index, "settings__c.SObjectType"),
            unknown("settings__c", Shape::Object)
        );
        assert_eq!(
            named(&index, "Settings__c.Limit__c"),
            unknown("Settings__c.Limit__c", Shape::Member)
        );
        let called = index.foreign("Account.getSObjectType", true);
        assert_eq!(
            (called.name.as_str(), called.shape),
            ("Account", Shape::Object)
        );
        assert_eq!(
            named(&index, "Contact.SObjectType"),
            unknown("Contact", Shape::Object)
        );
        // Nothing says `Invoice__c` is an object.
        assert_eq!(
            named(&index, "Invoice__c.SObjectType"),
            unknown("Invoice__c.SObjectType", Shape::AsWritten)
        );
    }

    /// `Network` is a standard object and a platform class. Unqualified,
    /// Apex means the class, whatever the repository adds to the object.
    #[test]
    fn a_platform_class_that_shares_its_name_with_an_object_stays_the_platforms() {
        let field = Declaration {
            symbol: "Network.Region__c".into(),
            name: "Region__c".into(),
            kind: DeclType::ObjectField,
            package: String::new(),
            file: String::new(),
            start: 1,
            end: 1,
            is_test: false,
            annotations: Vec::new(),
            collision: false,
            scope: None,
        };
        let index = Index::build([], &[field], &[]);

        assert_eq!(
            index.foreign("Network.getNetworkId", true).outside,
            Outside::Platform
        );
        assert_eq!(
            named(&index, "Schema.Network.Region__c"),
            (
                Outside::Unknown,
                "Network.Region__c".to_string(),
                Shape::Member
            )
        );
    }

    #[test]
    fn the_describe_form_names_an_object_or_one_of_its_fields() {
        let index = index(vec![]);
        let unknown = |name: &str, shape| (Outside::Unknown, name.to_string(), shape);

        assert_eq!(
            named(&index, "Schema.SObjectType.Invoice__c"),
            unknown("Invoice__c", Shape::Object)
        );
        assert_eq!(
            named(&index, "SObjectType.Invoice__c.fields.Total__c"),
            unknown("Invoice__c.Total__c", Shape::Member)
        );
        assert_eq!(
            named(&index, "SObjectType.Invoice__c.fieldSets.Summary"),
            unknown("Invoice__c.fieldSets.Summary", Shape::Member)
        );
        // `fields.getMap()` is a method of the describe result, not a field.
        let called = index.foreign("SObjectType.Invoice__c.fields.getMap", true);
        assert_eq!(
            (called.name.as_str(), called.shape),
            ("Invoice__c", Shape::Object)
        );
    }

    #[test]
    fn labels_and_flow_interviews_belong_to_the_repository() {
        let index = index(vec![]);
        let unknown = |name: &str, shape| (Outside::Unknown, name.to_string(), shape);

        assert_eq!(
            named(&index, "System.Label.Greeting"),
            unknown("Label.Greeting", Shape::Member)
        );
        assert_eq!(
            named(&index, "Flow.Interview.Welcome"),
            unknown("Flow.Interview.Welcome", Shape::AsWritten)
        );
        assert_eq!(named(&index, "Flow.Interview").0, Outside::Platform);
    }

    #[test]
    fn every_declaration_of_a_colliding_name_keeps_its_members() {
        let returning = |ty: &str| Callable {
            returns: Some(ty.into()),
            ..method("Twins", "make", &[])
        };
        let index = index(vec![
            class("Twins", None, vec![returning("Alpha")]),
            class("Twins", None, vec![returning("Beta")]),
        ]);

        let returns: Vec<_> = index
            .methods("twins", "make")
            .iter()
            .map(|m| m.returns.clone().unwrap())
            .collect();

        assert_eq!(returns, ["Alpha", "Beta"]);
        let Binding::Bound(alternatives) = index.bind(&index.methods("twins", "make"), &[], true)
        else {
            panic!("two declarations of one signature are one overload");
        };
        assert_eq!(alternatives.len(), 2);
    }

    #[test]
    fn a_method_hides_only_those_of_its_own_ancestors() {
        let go = |owner| vec![method(owner, "go", &[])];
        let mut mixed = class("Mixed", Some("Alpha"), vec![]);
        mixed.interfaces = vec!["Beta".into()];
        let index = index(vec![
            mixed,
            class("Alpha", Some("Root"), go("Alpha")),
            class("Beta", None, go("Beta")),
            class("Root", None, go("Root")),
        ]);

        let found: Vec<&str> = index
            .methods("mixed", "go")
            .iter()
            .map(|m| m.symbol.as_str())
            .collect();

        // `Alpha.go()` overrides `Root.go()`; it does not override `Beta.go()`.
        assert_eq!(found, ["Alpha.go()", "Beta.go()"]);
    }

    #[test]
    fn nothing_is_hidden_through_declarations_that_differ() {
        let go = |owner| vec![method(owner, "go", &[])];
        let index = index(vec![
            class("Twins", Some("Alpha"), go("Twins")),
            class("Twins", Some("Beta"), vec![]),
            class("Alpha", None, go("Alpha")),
            class("Beta", None, go("Beta")),
            class("Sub", Some("Twins"), go("Sub")),
        ]);

        let found = |key| -> Vec<&str> {
            let methods = index.methods(key, "go");
            methods.iter().map(|m| m.symbol.as_str()).collect()
        };

        // One `Twins` overrides `Alpha.go()`; the other inherits `Beta.go()`.
        assert_eq!(found("twins"), ["Twins.go()", "Alpha.go()", "Beta.go()"]);
        // A subtype of a divergent type is in the same position.
        assert_eq!(
            found("sub"),
            ["Sub.go()", "Twins.go()", "Alpha.go()", "Beta.go()"]
        );
        assert!(index.is_tainted("sub"));
        assert!(!index.is_tainted("alpha"));
    }

    #[test]
    fn declarations_that_are_the_same_in_substance_are_not_divergent() {
        let twins = || {
            class(
                "Twins",
                Some("alpha"),
                vec![method("Twins", "go", &["System.List<Id>"])],
            )
        };
        let respelled = class(
            "Twins",
            Some("Alpha"),
            vec![method("Twins", "GO", &["Id[]"])],
        );
        let index = index(vec![twins(), respelled, class("Alpha", None, vec![])]);

        assert!(!index.is_tainted("twins"));
        assert_eq!(index.methods("twins", "go").len(), 1);
    }

    #[test]
    fn a_name_looked_up_through_differing_declarations_may_mean_several_types() {
        let index = index(vec![
            class("Twins", Some("Alpha"), vec![]),
            class("Twins", Some("Beta"), vec![]),
            class("Alpha", None, vec![]),
            class("Alpha.Inner", None, vec![]),
            class("Beta", None, vec![]),
            class("Beta.Inner", None, vec![]),
        ]);

        assert_eq!(
            index.resolve_types("Inner", "Twins"),
            ["alpha.inner", "beta.inner"]
        );
        // Outside the divergent type the usual single answer holds.
        assert_eq!(index.resolve_types("Inner", "Alpha"), ["alpha.inner"]);
        // The same text in the two contexts is not the same type.
        let list_in = |context| Ty::new("List<Inner>", context);
        assert!(!index.is_same_type(&list_in("Alpha"), &list_in("Beta")));
        assert!(index.is_same_type(&list_in("Alpha"), &Ty::new("Alpha.Inner[]", "Beta")));
    }

    #[test]
    fn a_collection_or_primitive_spelled_with_its_namespace_is_the_same_type() {
        assert_eq!(normal("System.List<System.String>"), "list<string>");
        assert_eq!(normal("system.map<Id,SYSTEM.Set<Foo>>"), "map<id,set<foo>>");
        assert_eq!(
            generic("System.Iterable<Foo>"),
            Some(("iterable".to_string(), vec!["Foo"]))
        );
        // Only where `System.` qualifies a primitive or a collection.
        assert_eq!(normal("System.Type"), "system.type");
        let element_of = |text| element(&Ty::platform(text)).map(|ty| ty.text);
        assert_eq!(element_of("System.Set<Foo>").as_deref(), Some("Foo"));
    }

    #[test]
    fn arrays_and_lists_have_elements_and_maps_do_not() {
        let element_of = |text| element(&Ty::platform(text)).map(|ty| ty.text);

        assert_eq!(element_of("Account[]").as_deref(), Some("Account"));
        assert_eq!(
            element_of("List<Map<Id,Foo>>").as_deref(),
            Some("Map<Id,Foo>")
        );
        assert_eq!(element_of("Map<Id,Foo>"), None);
        assert_eq!(
            generic("Map<Id,List<Map<String,Foo>>>"),
            Some(("map".to_string(), vec!["Id", "List<Map<String,Foo>>"]))
        );
    }
}
