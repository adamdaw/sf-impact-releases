//! What is declared in scope, as metadata refers to it: by API name and by
//! the kind of thing the name is. Apex that names an object, a field or a
//! label is resolved here too.
//!
//! A name declared more than once resolves to the one symbol the
//! declarations share. The only thing resolved *through* a declaration is a
//! relationship: `Carrier__r.Region__c` goes through the lookup `Carrier__c`
//! to the object it points at. Where a lookup is declared more than once and
//! the declarations point at different objects, nothing is chosen and
//! nothing is dropped: the path goes on from every one of them. What it
//! reaches from several is `ambiguous`, and an object it cannot be followed
//! from keeps the rest of the path as an unresolved reference.

use super::Relationship;
use sf_impact_common::rows::{DeclType, Declaration, RefKind, ReferencedType, Resolution, fold};
use std::collections::{BTreeMap, HashMap};

/// What a name in a metadata file turned out to be.
#[derive(Debug, PartialEq, Eq)]
pub struct Found {
    /// The declared symbol, or the name as written if it is not declared.
    pub referenced: String,
    pub referenced_type: Option<ReferencedType>,
    pub resolution: Resolution,
    /// More than one where the name is reached through declarations that
    /// differ: every symbol it may be.
    pub candidates: Vec<String>,
}

pub struct Catalog {
    /// Per case-folded symbol and kind of thing, the declared spelling.
    /// Where declarations differ in case, the spelling that sorts first.
    symbols: HashMap<String, BTreeMap<ReferencedType, String>>,
    /// Per case-folded relationship field, each of its declarations.
    lookups: HashMap<String, Vec<Relationship>>,
    /// Per case-folded field, how many files declare it. Where that is more
    /// than its declarations as a relationship, some declare it as
    /// something else.
    field_declarations: HashMap<String, usize>,
    /// Per case-folded (parent object, child relationship name), the fields
    /// that make the relationship: `Parcels__r` on `Shipment__c` is made by
    /// `Parcel__c.Shipment__c`.
    children: HashMap<(String, String), Vec<String>>,
    namespaces: Vec<String>,
}

impl Catalog {
    pub fn build<'a>(
        declarations: impl IntoIterator<Item = &'a Declaration>,
        relationships: impl IntoIterator<Item = &'a Relationship>,
        namespaces: &[String],
    ) -> Catalog {
        let mut catalog = Catalog {
            symbols: HashMap::new(),
            lookups: HashMap::new(),
            field_declarations: HashMap::new(),
            children: HashMap::new(),
            namespaces: namespaces.iter().map(|ns| fold(ns)).collect(),
        };
        for declaration in declarations {
            let Some(kind) = referenced_type(declaration.kind) else {
                continue;
            };
            if kind == ReferencedType::ObjectField {
                let declared = catalog.field_declarations.entry(fold(&declaration.symbol));
                *declared.or_default() += 1;
            }
            let spellings = catalog
                .symbols
                .entry(fold(&declaration.symbol))
                .or_default();
            let spelling = spellings
                .entry(kind)
                .or_insert_with(|| declaration.symbol.clone());
            if declaration.symbol < *spelling {
                *spelling = declaration.symbol.clone();
            }
        }
        for relationship in relationships {
            let lookups = catalog
                .lookups
                .entry(fold(&relationship.field))
                .or_default();
            lookups.push(relationship.clone());
            if let Some(name) = &relationship.child_name {
                let key = (fold(&relationship.to), fold(name));
                let fields = catalog.children.entry(key).or_default();
                fields.push(relationship.field.clone());
            }
        }
        catalog
    }

    /// The declared spelling of `name` as a thing of `kind`, if it is declared.
    fn declared(&self, name: &str, kind: ReferencedType) -> Option<&str> {
        let spellings = self.symbols.get(&fold(name))?;
        spellings.get(&kind).map(String::as_str)
    }

    /// A name that the source says is a thing of `kind`.
    pub fn named(&self, name: &str, kind: ReferencedType) -> Found {
        match self.declared(name, kind) {
            Some(symbol) => Found {
                referenced: symbol.to_string(),
                referenced_type: Some(kind),
                resolution: Resolution::Declaration,
                candidates: Vec::new(),
            },
            None => self.undeclared(name.to_string(), Some(kind)),
        }
    }

    /// A field named with its object: `Shipment__c.Weight__c`.
    pub fn field(&self, qualified: &str) -> Found {
        match qualified.split_once('.') {
            Some((object, field)) => self.path(object, &[field]).remove(0),
            None => self.undeclared(qualified.to_string(), Some(ReferencedType::ObjectField)),
        }
    }

    /// A name that nothing in scope declares: in an installed package if
    /// its namespace is configured, a gap otherwise.
    pub fn undeclared(&self, name: String, kind: Option<ReferencedType>) -> Found {
        // `ns__Name__c` is in namespace `ns` and `Name__c` in none, but a
        // class or a Flow has no suffix: `ns__Name` is in `ns`.
        let has_suffix = matches!(
            kind,
            None | Some(ReferencedType::Object | ReferencedType::ObjectField | ReferencedType::Tab)
        );
        let is_external = name.split('.').any(|segment| {
            let parts: Vec<&str> = segment.split("__").collect();
            parts.len() > 1 + usize::from(has_suffix) && self.namespaces.contains(&fold(parts[0]))
        });
        Found {
            referenced: name,
            referenced_type: kind,
            resolution: match is_external {
                true => Resolution::External,
                false => Resolution::Unresolved,
            },
            candidates: Vec::new(),
        }
    }

    /// A field path read from `object`. A segment before the last is a
    /// relationship, and the path goes on from the object it leads to: one
    /// reference for the lookup, then the next segment from there.
    ///
    /// A lookup that is declared more than once may lead to several
    /// objects. The path then goes on from each of them, and none is
    /// dropped for lack of metadata: where a segment is declared from some
    /// and not from others, the declared fields are one reference and each
    /// object it cannot be followed from keeps the rest of the path, as
    /// written, as an unresolved one.
    pub fn path(&self, object: &str, segments: &[&str]) -> Vec<Found> {
        let field = ReferencedType::ObjectField;
        let start = self.declared(object, ReferencedType::Object);
        let mut objects = vec![start.unwrap_or(object).to_string()];
        let mut found = Vec::new();
        for (at, segment) in segments.iter().enumerate() {
            // `Owner:User` is the relationship `Owner`, read as a `User`.
            let (name, read_as) = match segment.split_once(':') {
                Some((name, read_as)) => (name, Some(read_as)),
                None => (*segment, None),
            };
            let is_last = at + 1 == segments.len();
            let rest = segments[at..].join(".");
            // From each object the path has reached: the fields this
            // segment may be, the objects they lead on to, and what is
            // left of the path where it cannot be followed.
            let (mut fields, mut next, mut unfollowed) = (Vec::new(), Vec::new(), Vec::new());
            for object in &objects {
                let own = self.declared(&format!("{object}.{name}"), field);
                let hops = self.relationships(object, name);
                match own.filter(|_| is_last) {
                    Some(own) => fields.push(own),
                    // The relationship is not declared; the path may still
                    // say which object it is read as.
                    None if hops.is_empty() && read_as.is_some() && !is_last => {
                        unfollowed.push(format!("{object}.{name}"));
                    }
                    None if hops.is_empty() => unfollowed.push(format!("{object}.{rest}")),
                    None => {
                        let leads_on = hops.iter().any(|(_, objects)| !objects.is_empty());
                        // A lookup that does not say what it points at.
                        if !is_last && read_as.is_none() && !leads_on {
                            unfollowed.push(format!("{object}.{rest}"));
                        }
                        for (hop, objects) in hops {
                            fields.push(hop);
                            next.extend(objects);
                        }
                    }
                }
            }
            found.extend(one_of(fields, name));
            found.extend(
                unfollowed
                    .into_iter()
                    .map(|rest| self.undeclared(rest, Some(field))),
            );
            if let Some(read_as) = read_as {
                next = vec![read_as.to_string()];
            }
            // Sorted and each once, so that the rows do not depend on the
            // order files were read in.
            next.sort_by_key(|object| fold(object));
            next.dedup_by_key(|object| fold(object));
            objects = next;
        }
        found
    }

    /// The relationship fields that `name` names on `object`, each with the
    /// objects it leads to: a lookup of this object (`Carrier__r` is the
    /// field `Carrier__c`, `Owner` the field `OwnerId`), or a lookup of
    /// another object that points here (`Parcels__r`).
    fn relationships(&self, object: &str, name: &str) -> Vec<(&str, Vec<String>)> {
        let field = match fold(name).strip_suffix("__r") {
            Some(custom) => format!("{object}.{custom}__c"),
            None => format!("{object}.{name}Id"),
        };
        let parent = self
            .declared(&field, ReferencedType::ObjectField)
            .map(|symbol| {
                let lookups = self.lookups(symbol).iter();
                (symbol, lookups.map(|lookup| lookup.to.clone()).collect())
            });
        let children = self.children.get(&(fold(object), fold(name)));
        let children = children.into_iter().flatten().filter_map(|field| {
            let symbol = self.declared(field, ReferencedType::ObjectField)?;
            let (child, _) = symbol.split_once('.')?;
            Some((symbol, vec![child.to_string()]))
        });
        parent.into_iter().chain(children).collect()
    }

    /// The declarations of the relationship field `field`: what it points
    /// at. Empty for a field that is not a relationship.
    pub fn lookups(&self, field: &str) -> &[Relationship] {
        self.lookups.get(&fold(field)).map_or(&[], Vec::as_slice)
    }

    /// How many files declare the field `field`, as a relationship or not.
    pub fn declarations_of(&self, field: &str) -> usize {
        self.field_declarations
            .get(&fold(field))
            .copied()
            .unwrap_or(0)
    }

    /// The declared things a string value names: every kind of thing that
    /// has a symbol equal to it, ignoring case.
    pub fn matching(&self, value: &str) -> Vec<Found> {
        let kinds = self.symbols.get(&fold(value)).into_iter().flatten();
        kinds.map(|(kind, _)| self.named(value, *kind)).collect()
    }

    /// What an Apex reference that Apex declarations do not resolve refers
    /// to, if metadata declares any of it: an object, the fields of a path,
    /// or a label. `None` leaves the reference as it was.
    pub fn apex(&self, kind: RefKind, referenced: &str) -> Option<Vec<Found>> {
        let found = match kind {
            RefKind::Type
            | RefKind::Cast
            | RefKind::Construct
            | RefKind::SoqlObject
            | RefKind::DmlObject
            | RefKind::TriggerObject => vec![self.named(referenced, ReferencedType::Object)],
            RefKind::StaticAccess if self.declared(referenced, ReferencedType::Label).is_some() => {
                vec![self.named(referenced, ReferencedType::Label)]
            }
            RefKind::SoqlField
            | RefKind::DmlField
            | RefKind::SobjectField
            | RefKind::StaticAccess => {
                let (object, path) = referenced.split_once('.')?;
                self.path(object, &path.split('.').collect::<Vec<_>>())
            }
            _ => return None,
        };
        let is_declared = |found: &Found| found.resolution == Resolution::Declaration;
        found.iter().any(is_declared).then_some(found)
    }
}

/// The kind of thing a declaration is, as metadata names it. A member of
/// an Apex class is not something metadata names.
fn referenced_type(kind: DeclType) -> Option<ReferencedType> {
    Some(match kind {
        DeclType::Class | DeclType::Interface | DeclType::Enum => ReferencedType::Class,
        DeclType::Trigger => ReferencedType::Trigger,
        DeclType::Object => ReferencedType::Object,
        DeclType::ObjectField => ReferencedType::ObjectField,
        DeclType::ValidationRule => ReferencedType::ValidationRule,
        DeclType::Label => ReferencedType::Label,
        DeclType::CustomPermission => ReferencedType::CustomPermission,
        DeclType::MessageChannel => ReferencedType::MessageChannel,
        DeclType::PermissionSet => ReferencedType::PermissionSet,
        DeclType::Profile => ReferencedType::Profile,
        DeclType::MetadataRecord => ReferencedType::MetadataRecord,
        DeclType::Flow => ReferencedType::Flow,
        DeclType::Lwc => ReferencedType::Lwc,
        DeclType::Method | DeclType::Constructor | DeclType::Field | DeclType::EnumValue => {
            return None;
        }
    })
}

/// A reference to whichever of the fields `symbols` the name `name` means.
/// One is that field. Several are all candidates: this is where a path that
/// went through lookups that differ ends up.
fn one_of(mut symbols: Vec<&str>, name: &str) -> Option<Found> {
    symbols.sort_unstable();
    symbols.dedup();
    let (referenced, candidates) = match symbols[..] {
        [] => return None,
        [only] => (only, Vec::new()),
        _ => (
            name,
            symbols.iter().map(|symbol| symbol.to_string()).collect(),
        ),
    };
    Some(Found {
        referenced: referenced.to_string(),
        referenced_type: Some(ReferencedType::ObjectField),
        resolution: Resolution::Declaration,
        candidates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declaration(kind: DeclType, symbol: &str) -> Declaration {
        Declaration {
            symbol: symbol.into(),
            name: symbol.rsplit('.').next().unwrap_or(symbol).into(),
            kind,
            package: String::new(),
            file: String::new(),
            start: 1,
            end: 1,
            is_test: false,
            annotations: Vec::new(),
            collision: false,
            scope: None,
        }
    }

    fn lookup(field: &str, to: &str, child_name: Option<&str>) -> Relationship {
        Relationship {
            field: field.into(),
            to: to.into(),
            child_name: child_name.map(String::from),
            controlling: None,
        }
    }

    /// A catalog of these object fields and relationships. Objects named
    /// with a `!` are declared themselves.
    fn catalog(fields: &[&str], relationships: &[Relationship]) -> Catalog {
        let fields = fields.iter().map(|field| match field.strip_suffix('!') {
            Some(object) => declaration(DeclType::Object, object),
            None => declaration(DeclType::ObjectField, field),
        });
        let declarations: Vec<Declaration> = fields.collect();
        Catalog::build(&declarations, relationships, &["npsp".to_string()])
    }

    /// Each reference as `symbol`, `symbol?` if unresolved, `symbol^` if
    /// external, or `name[candidates]`.
    fn described(found: &[Found]) -> Vec<String> {
        let describe = |found: &Found| match (found.resolution, &found.candidates[..]) {
            (Resolution::Declaration, []) => found.referenced.clone(),
            (Resolution::Declaration, candidates) => {
                format!("{}[{}]", found.referenced, candidates.join(" "))
            }
            (Resolution::Unresolved, _) => format!("{}?", found.referenced),
            (Resolution::External, _) => format!("{}^", found.referenced),
        };
        found.iter().map(describe).collect()
    }

    #[test]
    fn a_field_resolves_whatever_its_case_to_the_spelling_it_was_declared_with() {
        let catalog = catalog(&["Invoice__c.Total__c"], &[]);

        let found = catalog.path("INVOICE__C", &["total__c"]);

        assert_eq!(described(&found), ["Invoice__c.Total__c"]);
    }

    #[test]
    fn declarations_that_differ_in_case_resolve_to_the_spelling_that_sorts_first() {
        let spellings = ["invoice__c.total__c", "Invoice__c.Total__c"];
        let reversed = [spellings[1], spellings[0]];

        for declared in [spellings, reversed] {
            let found = catalog(&declared, &[]).path("Invoice__c", &["Total__c"]);

            assert_eq!(described(&found), ["Invoice__c.Total__c"], "{declared:?}");
        }
    }

    #[test]
    fn a_field_that_is_not_declared_is_unresolved_under_the_object_as_written() {
        let catalog = catalog(&["Invoice__c!", "Invoice__c.Total__c"], &[]);

        assert_eq!(
            described(&catalog.path("invoice__c", &["Name"])),
            ["Invoice__c.Name?"]
        );
        assert_eq!(
            described(&catalog.path("account", &["Name"])),
            ["account.Name?"]
        );
    }

    #[test]
    fn a_custom_relationship_is_followed_through_its_lookup_field() {
        let catalog = catalog(
            &["Invoice__c.Account__c", "Account.Region__c"],
            &[lookup("Invoice__c.Account__c", "Account", None)],
        );

        let found = catalog.path("Invoice__c", &["Account__r", "Region__c"]);

        assert_eq!(
            described(&found),
            ["Invoice__c.Account__c", "Account.Region__c"]
        );
    }

    #[test]
    fn a_standard_relationship_is_followed_only_if_its_id_field_is_declared() {
        let declared = catalog(
            &["Contact.AccountId", "Account.Region__c"],
            &[lookup("Contact.AccountId", "Account", None)],
        );
        let undeclared = catalog(&["Account.Region__c"], &[]);
        let path = ["Account", "Region__c"];

        assert_eq!(
            described(&declared.path("Contact", &path)),
            ["Contact.AccountId", "Account.Region__c"]
        );
        assert_eq!(
            described(&undeclared.path("Contact", &path)),
            ["Contact.Account.Region__c?"]
        );
    }

    #[test]
    fn what_follows_a_relationship_that_cannot_be_followed_is_one_unresolved_name() {
        let catalog = catalog(
            &["Invoice__c.Account__c"],
            &[lookup("Invoice__c.Account__c", "Account", None)],
        );

        let found = catalog.path("Invoice__c", &["Account__r", "Owner", "Manager", "Name"]);

        assert_eq!(
            described(&found),
            ["Invoice__c.Account__c", "Account.Owner.Manager.Name?"]
        );
    }

    /// `Account__c.Name` where `Account__r.Name` is meant: the field is
    /// declared, but it is not what a path goes through.
    #[test]
    fn a_field_written_where_a_relationship_belongs_does_not_end_the_path_early() {
        let catalog = catalog(
            &["Invoice__c.Account__c", "Account.Region__c"],
            &[lookup("Invoice__c.Account__c", "Account", None)],
        );

        let found = catalog.path("Invoice__c", &["Account__c", "Region__c"]);

        assert_eq!(described(&found), ["Invoice__c.Account__c.Region__c?"]);
    }

    #[test]
    fn a_lookup_that_does_not_say_what_it_points_at_ends_the_path() {
        let catalog = catalog(&["Invoice__c.OwnerId"], &[]);

        let found = catalog.path("Invoice__c", &["Owner", "Name"]);

        assert_eq!(
            described(&found),
            ["Invoice__c.OwnerId", "Invoice__c.Owner.Name?"]
        );
    }

    #[test]
    fn a_polymorphic_relationship_goes_on_from_the_object_it_is_read_as() {
        let catalog = catalog(&["User.Region__c"], &[]);

        let found = catalog.path("Invoice__c", &["Owner:User", "Region__c"]);

        assert_eq!(described(&found), ["Invoice__c.Owner?", "User.Region__c"]);
    }

    #[test]
    fn a_declared_lookup_read_as_another_object_goes_on_from_that_object_only() {
        let catalog = catalog(
            &["Invoice__c.OwnerId", "Group.Region__c", "User.Region__c"],
            &[lookup("Invoice__c.OwnerId", "Group", None)],
        );

        let found = catalog.path("Invoice__c", &["Owner:User", "Region__c"]);

        assert_eq!(described(&found), ["Invoice__c.OwnerId", "User.Region__c"]);
    }

    #[test]
    fn a_child_relationship_is_the_lookup_that_makes_it_and_leads_to_the_child_object() {
        let catalog = catalog(
            &["Line__c.Invoice__c", "Line__c.Amount__c"],
            &[lookup("Line__c.Invoice__c", "Invoice__c", Some("Lines__r"))],
        );

        assert_eq!(
            described(&catalog.path("Invoice__c", &["Lines__r"])),
            ["Line__c.Invoice__c"]
        );
        assert_eq!(
            described(&catalog.path("Invoice__c", &["Lines__r", "Amount__c"])),
            ["Line__c.Invoice__c", "Line__c.Amount__c"]
        );
    }

    #[test]
    fn a_lookup_declared_twice_to_one_object_is_followed_as_one() {
        let catalog = catalog(
            &["Invoice__c.Payer__c", "Account.Region__c"],
            &[
                lookup("Invoice__c.Payer__c", "Account", None),
                lookup("Invoice__c.Payer__c", "account", None),
            ],
        );

        let found = catalog.path("Invoice__c", &["Payer__r", "Region__c"]);

        assert_eq!(
            described(&found),
            ["Invoice__c.Payer__c", "Account.Region__c"]
        );
    }

    /// `Payer__c` is declared twice: a lookup to `Account` and to `Contact`.
    fn two_payers() -> [Relationship; 2] {
        [
            lookup("Invoice__c.Payer__c", "Account", None),
            lookup("Invoice__c.Payer__c", "Contact", None),
        ]
    }

    #[test]
    fn a_lookup_declared_twice_to_different_objects_reaches_every_field_it_may_mean() {
        let catalog = catalog(
            &[
                "Invoice__c.Payer__c",
                "Account.Region__c",
                "Contact.Region__c",
            ],
            &two_payers(),
        );

        let found = catalog.path("Invoice__c", &["Payer__r", "Region__c"]);

        assert_eq!(
            described(&found),
            [
                "Invoice__c.Payer__c",
                "Region__c[Account.Region__c Contact.Region__c]"
            ]
        );
    }

    /// Regression. That no file declares `Account.Region__c`
    /// does not mean the path cannot end there.
    #[test]
    fn an_end_of_such_a_lookup_that_does_not_declare_the_field_stays_as_an_unresolved_row() {
        let catalog = catalog(&["Invoice__c.Payer__c", "Contact.Region__c"], &two_payers());

        let found = catalog.path("Invoice__c", &["Payer__r", "Region__c"]);

        assert_eq!(
            described(&found),
            [
                "Invoice__c.Payer__c",
                "Contact.Region__c",
                "Account.Region__c?"
            ]
        );
    }

    #[test]
    fn a_field_that_no_end_of_such_a_lookup_declares_is_unresolved_at_every_end() {
        let catalog = catalog(&["Invoice__c.Payer__c"], &two_payers());

        let found = catalog.path("Invoice__c", &["Payer__r", "Region__c"]);

        assert_eq!(
            described(&found),
            [
                "Invoice__c.Payer__c",
                "Account.Region__c?",
                "Contact.Region__c?"
            ]
        );
    }

    #[test]
    fn an_end_that_cannot_go_on_part_way_along_a_path_keeps_the_rest_of_it_as_written() {
        let mut lookups = two_payers().to_vec();
        lookups.push(lookup("Account.Rep__c", "User", None));
        let catalog = catalog(
            &["Invoice__c.Payer__c", "Account.Rep__c", "User.Region__c"],
            &lookups,
        );

        let found = catalog.path("Invoice__c", &["Payer__r", "Rep__r", "Region__c"]);

        // From `Account` the path goes on to `User`; from `Contact` it cannot.
        assert_eq!(
            described(&found),
            [
                "Invoice__c.Payer__c",
                "Account.Rep__c",
                "Contact.Rep__r.Region__c?",
                "User.Region__c"
            ]
        );
    }

    #[test]
    fn a_lookup_declared_twice_to_one_object_leaves_one_unresolved_row_not_two() {
        let catalog = catalog(
            &["Invoice__c.Payer__c"],
            &[
                lookup("Invoice__c.Payer__c", "Account", None),
                lookup("Invoice__c.Payer__c", "account", None),
            ],
        );

        let found = catalog.path("Invoice__c", &["Payer__r", "Region__c"]);

        assert_eq!(
            described(&found),
            ["Invoice__c.Payer__c", "Account.Region__c?"]
        );
    }

    #[test]
    fn apex_keeps_every_end_of_a_path_through_such_a_lookup() {
        let catalog = catalog(&["Invoice__c.Payer__c", "Contact.Region__c"], &two_payers());

        let found = catalog.apex(RefKind::SoqlField, "Invoice__c.Payer__r.Region__c");

        assert_eq!(
            found.map(|found| described(&found)),
            Some(vec![
                "Invoice__c.Payer__c".to_string(),
                "Contact.Region__c".to_string(),
                "Account.Region__c?".to_string()
            ])
        );
    }

    #[test]
    fn a_name_in_a_configured_namespace_is_external() {
        let catalog = catalog(&[], &[]);
        let resolution = |name: &str, kind| catalog.named(name, kind).resolution;

        assert_eq!(
            resolution("npsp__Gift__c", ReferencedType::Object),
            Resolution::External
        );
        assert_eq!(
            resolution("npsp__Pricing", ReferencedType::Class),
            Resolution::External
        );
        assert_eq!(
            catalog.field("Account.npsp__Level__c").resolution,
            Resolution::External
        );
        // One separator is a suffix on an object, not a namespace prefix.
        assert_eq!(
            resolution("npsp__c", ReferencedType::Object),
            Resolution::Unresolved
        );
        assert_eq!(
            resolution("Gift__c", ReferencedType::Object),
            Resolution::Unresolved
        );
    }

    #[test]
    fn a_name_is_declared_only_as_the_kind_of_thing_it_was_declared_as() {
        let declarations = [
            declaration(DeclType::Class, "Billing"),
            declaration(DeclType::PermissionSet, "Billing"),
            declaration(DeclType::Method, "Billing.run()"),
        ];
        let catalog = Catalog::build(&declarations, [], &[]);
        let resolution = |kind| catalog.named("billing", kind).resolution;

        assert_eq!(resolution(ReferencedType::Class), Resolution::Declaration);
        assert_eq!(
            resolution(ReferencedType::PermissionSet),
            Resolution::Declaration
        );
        assert_eq!(resolution(ReferencedType::Object), Resolution::Unresolved);
    }

    #[test]
    fn a_string_value_matches_every_kind_of_thing_declared_under_that_name() {
        let declarations = [
            declaration(DeclType::Interface, "Billing"),
            declaration(DeclType::PermissionSet, "Billing"),
            declaration(DeclType::Method, "Billing.run()"),
            declaration(DeclType::Flow, "Remind"),
            declaration(DeclType::Lwc, "remindPanel"),
        ];
        let catalog = Catalog::build(&declarations, [], &[]);
        let kinds = |value: &str| -> Vec<(Option<ReferencedType>, Resolution)> {
            let found = catalog.matching(value);
            found
                .iter()
                .map(|f| (f.referenced_type, f.resolution))
                .collect()
        };

        assert_eq!(
            kinds("BILLING"),
            [
                (Some(ReferencedType::Class), Resolution::Declaration),
                (Some(ReferencedType::PermissionSet), Resolution::Declaration),
            ]
        );
        assert_eq!(
            kinds("remind"),
            [(Some(ReferencedType::Flow), Resolution::Declaration)]
        );
        assert_eq!(
            kinds("RemindPanel"),
            [(Some(ReferencedType::Lwc), Resolution::Declaration)]
        );
        // A member of an Apex class is not something a record names.
        assert_eq!(kinds("Billing.run()"), []);
        assert_eq!(kinds("Nothing"), []);
    }

    #[test]
    fn apex_resolves_through_metadata_only_what_metadata_declares() {
        let declarations = [
            declaration(DeclType::Object, "Invoice__c"),
            declaration(DeclType::ObjectField, "Invoice__c.Total__c"),
            declaration(DeclType::Label, "Label.Greeting"),
        ];
        let catalog = Catalog::build(&declarations, [], &[]);
        let resolved = |kind, name| catalog.apex(kind, name).map(|found| described(&found));

        assert_eq!(
            resolved(RefKind::SoqlObject, "invoice__c"),
            Some(vec!["Invoice__c".to_string()])
        );
        assert_eq!(
            resolved(RefKind::SoqlField, "Invoice__c.total__c"),
            Some(vec!["Invoice__c.Total__c".to_string()])
        );
        assert_eq!(
            resolved(RefKind::StaticAccess, "Label.greeting"),
            Some(vec!["Label.Greeting".to_string()])
        );
        // Nothing declared: the reference is left as Apex reported it.
        assert_eq!(resolved(RefKind::SoqlObject, "Account"), None);
        assert_eq!(resolved(RefKind::SoqlField, "Invoice__c.Name"), None);
        assert_eq!(resolved(RefKind::StaticAccess, "Label.Missing"), None);
        // A call is never to an object or a field.
        assert_eq!(resolved(RefKind::Call, "Invoice__c.Total__c"), None);
    }
}
