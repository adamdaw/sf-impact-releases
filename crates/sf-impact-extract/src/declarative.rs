//! Declarative metadata in Salesforce DX source format: objects, fields,
//! validation rules, permission sets, profiles, custom metadata records,
//! labels, custom permissions and message channels. Source format keeps one
//! component in one file and names it in the file's path, so a file is recognised
//! by its name, and what it declares is known even if it does not parse.
//!
//! Flows and Lightning web components are not read here. They and every
//! other `-meta.xml` file that is not listed in `file_kind` are components
//! this module does not model: each gets an `unmodelled_file` row, so that
//! the gap is in the output.
//!
//! The same holds inside a modelled file. Every element is read for the
//! name in it, or is known to hold none, or gets an `unsupported_element`
//! row: see `parts`. And an element that should name something and does
//! not gets that row too, not a reference to nothing.

mod catalog;
mod formula;
mod parts;
mod xml;

pub use catalog::{Catalog, Found};

use parts::{Names, Part};
use sf_impact_common::rows::{
    DeclType, Declaration, Detail, RefKind, Reference, ReferencedType, Row, Tier,
    UnsupportedElement, UnsupportedReason, fold,
};
use xml::{Document, Element};

const META_SUFFIX: &str = "-meta.xml";

/// What a file in scope is, going by its name.
#[derive(Debug, PartialEq, Eq)]
pub enum FileKind {
    Modelled(Kind),
    /// A component of a type that is not modelled, with what stands between
    /// its name and `-meta.xml`.
    Unmodelled(String),
    /// Not a component of its own: Apex and its companion files, the files
    /// of a bundle, anything that is not metadata.
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Object,
    Field,
    ValidationRule,
    PermissionSet,
    Profile,
    Record,
    Labels,
    CustomPermission,
    MessageChannel,
}

pub fn file_kind(path: &str) -> FileKind {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let named = file_name
        .strip_suffix(META_SUFFIX)
        .and_then(|stem| stem.rsplit_once('.'));
    let Some((name, suffix)) = named else {
        return FileKind::Other;
    };
    // A field or a rule is named after the directory of its object, and a
    // record after its type. One that is not cannot be given a name, and
    // neither can a file whose name leaves the name, or a part of it
    // between dots, empty.
    let is_under_an_object = path.rsplit('/').nth(2).is_some();
    let is_named = name.split('.').all(|part| !part.is_empty());
    FileKind::Modelled(match suffix {
        // The companion of an Apex file holds an API version and a status.
        "cls" | "trigger" => return FileKind::Other,
        _ if !is_named => return FileKind::Unmodelled(suffix.to_string()),
        "object" => Kind::Object,
        "field" if is_under_an_object => Kind::Field,
        "validationRule" if is_under_an_object => Kind::ValidationRule,
        "permissionset" => Kind::PermissionSet,
        "profile" => Kind::Profile,
        "md" if name.contains('.') => Kind::Record,
        "labels" => Kind::Labels,
        "customPermission" => Kind::CustomPermission,
        "messageChannel" => Kind::MessageChannel,
        other => return FileKind::Unmodelled(other.to_string()),
    })
}

/// A lookup or master-detail field, as far as following a relationship
/// through it needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Relationship {
    /// The field's symbol: `Parcel__c.Shipment__c`.
    pub field: String,
    /// The object it points at.
    pub to: String,
    /// What the relationship is called from that object: `Parcels__r`.
    pub child_name: Option<String>,
    /// For a metadata relationship to a field: the field of the same type
    /// that says which object it is a field of, written with its object.
    pub controlling: Option<String>,
}

/// A metadata file after phase one.
pub struct Metadata {
    kind: Kind,
    document: Document,
    /// The component the file is, by its path: what its references are from.
    symbol: String,
    pub declarations: Vec<Declaration>,
    pub relationship: Option<Relationship>,
}

/// Phase one: what the file declares. `source` is its content as text.
pub fn read(kind: Kind, source: &[u8], package: &str, path: &str) -> Metadata {
    let document = Document::parse(source);
    let file_name = path.rsplit('/').next().unwrap_or(path);
    // `file_kind` found the name and the suffix before this is called.
    let stem = file_name.strip_suffix(META_SUFFIX).unwrap_or(file_name);
    let name = stem.rsplit_once('.').map_or(stem, |(name, _)| name);
    // `objects/<Object>/fields/<Field>.field-meta.xml`.
    let object = path.rsplit('/').nth(2).unwrap_or_default();
    let (symbol, kind_declared) = match kind {
        Kind::Object => (name.to_string(), DeclType::Object),
        Kind::Field => (format!("{object}.{name}"), DeclType::ObjectField),
        Kind::ValidationRule => (format!("{object}.{name}"), DeclType::ValidationRule),
        Kind::PermissionSet => (name.to_string(), DeclType::PermissionSet),
        Kind::Profile => (name.to_string(), DeclType::Profile),
        Kind::Record => (name.to_string(), DeclType::MetadataRecord),
        Kind::CustomPermission => (name.to_string(), DeclType::CustomPermission),
        Kind::MessageChannel => (
            format!("MessageChannel.{name}__c"),
            DeclType::MessageChannel,
        ),
        Kind::Labels => (String::new(), DeclType::Label),
    };
    let declaration = |symbol: String, start: usize, end: usize| Declaration {
        name: symbol.rsplit('.').next().unwrap_or(&symbol).to_string(),
        symbol,
        kind: kind_declared,
        package: package.to_string(),
        file: path.to_string(),
        start: document.line(start),
        end: document.line(end),
        is_test: false,
        annotations: Vec::new(),
        collision: false,
        scope: None,
    };
    let declarations = match kind {
        // One file holds every label; each is declared where it stands.
        Kind::Labels => {
            let labels = document.root.children_named("labels");
            let named =
                labels.filter_map(|label| Some((label, name_in(label.child("fullName")?)?)));
            named
                .map(|(label, (name, _))| {
                    let symbol = format!("Label.{name}");
                    declaration(symbol, label.start, label.end.saturating_sub(1))
                })
                .collect()
        }
        _ => vec![declaration(
            symbol.clone(),
            0,
            source.len().saturating_sub(1),
        )],
    };
    let named = |element: &str| document.root.child(element).and_then(name_in);
    let relationship = match (kind, named("referenceTo")) {
        (Kind::Field, Some((to, _))) => Some(Relationship {
            field: symbol.clone(),
            to: to.to_string(),
            // A custom field's relationship is `Name__r` in a query.
            child_name: named("relationshipName").map(|(called, _)| {
                match fold(name).ends_with("__c") {
                    true => format!("{called}__r"),
                    false => called.to_string(),
                }
            }),
            controlling: named("metadataRelationshipControllingField")
                .map(|(controlling, _)| controlling.to_string()),
        }),
        _ => None,
    };
    Metadata {
        kind,
        document,
        symbol,
        declarations,
        relationship,
    }
}

impl Metadata {
    /// The line of the first error, if the file is not well-formed XML.
    pub fn error_line(&self) -> Option<u32> {
        self.document.error.map(|at| self.document.line(at))
    }

    /// The first and last line of each element directly under the root:
    /// one grant of a permission set or profile, one value of a record.
    pub fn entries(&self) -> Vec<(u32, u32)> {
        let line = |offset| self.document.line(offset);
        let entries = self.document.root.children.iter();
        entries
            .map(|entry| (line(entry.start), line(entry.end.saturating_sub(1))))
            .collect()
    }
}

/// The name in an element and where it starts, or `None` if the element is
/// empty: nothing is ever called by an empty name.
fn name_in(element: &Element) -> Option<(&str, usize)> {
    Some(element.trimmed()).filter(|(name, _)| !name.is_empty())
}

/// Phase two: the `parsed` references of one metadata file, read from as
/// much of it as is well-formed, and the elements of it that were not read.
pub fn references(metadata: &Metadata, catalog: &Catalog, package: &str, path: &str) -> Vec<Row> {
    let mut out = Rows {
        metadata,
        catalog,
        package,
        path,
        rows: Vec::new(),
    };
    if metadata.kind == Kind::Record {
        out.record();
    }
    out.elements(&metadata.document.root, "");
    out.rows
}

struct Rows<'a> {
    metadata: &'a Metadata,
    catalog: &'a Catalog,
    package: &'a str,
    path: &'a str,
    rows: Vec<Row>,
}

impl Rows<'_> {
    /// Every element in `holder`, whose own path is `within`: read, passed
    /// over because it holds no name, or reported.
    fn elements(&mut self, holder: &Element, within: &str) {
        for element in &holder.children {
            let path = match within {
                "" => element.name.clone(),
                _ => format!("{within}/{}", element.name),
            };
            match parts::part(self.metadata.kind, &path) {
                Some(Part::Inert) => {}
                Some(Part::Holder(named_by)) => {
                    if named_by.is_some_and(|name| element.child(name).is_none()) {
                        self.unsupported(element, &path, UnsupportedReason::NoName);
                    }
                    self.elements(element, &path);
                }
                Some(Part::Formula) => self.formula(element),
                Some(Part::Assignment) => self.assignment(element, &path),
                Some(Part::Name(names)) => match name_in(element) {
                    Some((name, at)) => {
                        if !self.name(names, name, at) {
                            self.unsupported(element, &path, UnsupportedReason::NotInterpreted);
                        }
                    }
                    None => self.unsupported(element, &path, UnsupportedReason::NoName),
                },
                None => self.unsupported(element, &path, UnsupportedReason::NotInterpreted),
            }
        }
    }

    /// The object this file's field or validation rule belongs to.
    fn object(&self) -> &str {
        self.metadata.symbol.split('.').next().unwrap_or_default()
    }

    /// One name, found at the byte offset `at` of the file. False if the
    /// name is not one this tool can read as what the element holds: no
    /// row is made of it then.
    fn name(&mut self, names: Names, name: &str, at: usize) -> bool {
        let catalog = self.catalog;
        let (kind, found) = match names {
            Names::LookupTarget => (
                RefKind::Lookup,
                vec![catalog.named(name, ReferencedType::Object)],
            ),
            Names::Field(kind) => (kind, vec![catalog.field(name)]),
            Names::OwnField(kind) => (kind, catalog.path(self.object(), &[name])),
            Names::FilterField => match named_in_filter(catalog, self.object(), name) {
                Some(found) => (RefKind::LookupFilter, found),
                None => return false,
            },
            Names::GlobalValueSet => (
                RefKind::ValueSet,
                vec![catalog.named(name, ReferencedType::GlobalValueSet)],
            ),
            Names::Granted(ReferencedType::ObjectField) => {
                (RefKind::Grant, vec![catalog.field(name)])
            }
            Names::Granted(granted) => (RefKind::Grant, vec![catalog.named(name, granted)]),
            Names::Declared => return true,
        };
        for found in found {
            self.push(kind, found, at);
        }
        true
    }

    /// A row at the byte offset `at` of the file.
    fn push(&mut self, kind: RefKind, found: Found, at: usize) {
        self.push_from(kind, found, at, None);
    }

    fn push_from(&mut self, kind: RefKind, found: Found, at: usize, source_field: Option<&str>) {
        let (line, column) = self.metadata.document.position(at);
        self.rows.push(Row::Reference(Reference {
            tier: Tier::Parsed,
            kind,
            referenced: found.referenced,
            referenced_type: found.referenced_type,
            resolution: Some(found.resolution),
            detail: (!found.candidates.is_empty()).then_some(Detail::AmbiguousReceiver),
            candidates: found.candidates,
            referrer: Some(self.metadata.symbol.clone()),
            source_field: source_field.map(String::from),
            package: self.package.to_string(),
            file: self.path.to_string(),
            line,
            column,
        }));
    }

    /// An element that was not read, and why.
    fn unsupported(&mut self, element: &Element, path: &str, reason: UnsupportedReason) {
        let (line, column) = self.metadata.document.position(element.start);
        self.rows.push(Row::UnsupportedElement(UnsupportedElement {
            package: self.package.to_string(),
            file: self.path.to_string(),
            element: path.to_string(),
            reason,
            line,
            column,
        }));
    }

    /// The names in a formula on this file's object.
    fn formula(&mut self, formula: &Element) {
        for (at, name) in formula::names(&formula.text) {
            for found in named_in_formula(self.catalog, self.object(), name) {
                self.push(RefKind::Formula, found, formula.offset_of(at));
            }
        }
    }

    /// The type of the custom metadata record `Type.Record`: `Type__mdt`.
    fn record_type(&self) -> String {
        format!("{}__mdt", self.object())
    }

    /// What a custom metadata record refers to by being the record it is:
    /// its type, and whatever its own name names. The name is in the
    /// file's path, not in the file.
    fn record(&mut self) {
        let found = self
            .catalog
            .named(&self.record_type(), ReferencedType::Object);
        self.push(RefKind::CmdtType, found, 0);
        let record_name = self
            .metadata
            .symbol
            .split_once('.')
            .map_or("", |(_, name)| name);
        for found in self.catalog.matching(record_name) {
            self.push_from(RefKind::Cmdt, found, 0, Some("DeveloperName"));
        }
    }

    /// One `<values>` of a record: the field of its type that it sets, and
    /// what the value names. A value of a metadata relationship field
    /// names what the field's type says. Any other value is a name only if
    /// it equals one, which it can do by coincidence, so the row says which
    /// field it came from.
    fn assignment(&mut self, values: &Element, path: &str) {
        let Some((field, at)) = values.child("field").and_then(name_in) else {
            return self.unsupported(values, path, UnsupportedReason::NoName);
        };
        let assigned = format!("{}.{field}", self.record_type());
        self.push(RefKind::CmdtField, self.catalog.field(&assigned), at);
        let Some((value, at)) = values.child("value").and_then(name_in) else {
            return;
        };
        // What the field's declarations as a relationship say the value
        // names. Unless every declaration of the field says so, the value
        // may also be a plain one that happens to equal a name: a field
        // that is text in one package directory and a relationship in
        // another is read both ways, and neither reading is preferred.
        let mut found = self.related(&assigned, value);
        if found.len() < self.catalog.declarations_of(&assigned).max(1) {
            found.extend(self.catalog.matching(value));
        }
        // Declarations that agree say the same thing more than once.
        let mut distinct: Vec<Found> = Vec::new();
        for found in found {
            if !distinct.contains(&found) {
                distinct.push(found);
            }
        }
        for found in distinct {
            self.push_from(RefKind::Cmdt, found, at, Some(field));
        }
    }

    /// What `value` names as the value of the metadata relationship field
    /// `assigned`: an object, a field of the object that another value of
    /// the record names, or a record of another type. One for each
    /// declaration of the field as such a relationship; empty if it has
    /// none.
    fn related(&self, assigned: &str, value: &str) -> Vec<Found> {
        let catalog = self.catalog;
        let mut found = Vec::new();
        for relationship in catalog.lookups(assigned) {
            let to = fold(&relationship.to);
            found.push(match to.as_str() {
                "entitydefinition" => catalog.named(value, ReferencedType::Object),
                "fielddefinition" | "entityparticle" => {
                    let object = relationship.controlling.as_deref();
                    match object.and_then(|controlling| self.value_of(controlling)) {
                        Some(object) => catalog.field(&format!("{object}.{value}")),
                        // The record does not say which object.
                        None => {
                            catalog.undeclared(value.to_string(), Some(ReferencedType::ObjectField))
                        }
                    }
                }
                _ => match to.strip_suffix("__mdt") {
                    Some(_) => {
                        let record_type = &relationship.to[..relationship.to.len() - "__mdt".len()];
                        let record = format!("{record_type}.{value}");
                        catalog.named(&record, ReferencedType::MetadataRecord)
                    }
                    None => continue,
                },
            });
        }
        found
    }

    /// The value this record sets the field `controlling` to, where
    /// `controlling` is written with its type: `Rule__mdt.Object__c`.
    fn value_of(&self, controlling: &str) -> Option<&str> {
        let field = controlling.rsplit('.').next()?;
        let root = &self.metadata.document.root;
        let mut values = root.children_named("values");
        let set = values.find(|values| {
            let named = values.child("field").and_then(name_in);
            named.is_some_and(|(name, _)| name.eq_ignore_ascii_case(field))
        })?;
        set.child("value").and_then(name_in).map(|(value, _)| value)
    }
}

/// What a field named in a lookup filter is a field of: `$Source.` is the
/// object the lookup is on, any other global is as in a formula, and
/// otherwise the name starts with its object.
///
/// `None` if the name cannot be read as a field: one without its object,
/// `$Source` without a field, a global that names no field or that this
/// tool does not know. A formula reports an unknown global as written; a
/// filter value is one name and has to be a name of something.
fn named_in_filter(catalog: &Catalog, object: &str, name: &str) -> Option<Vec<Found>> {
    let segments: Vec<&str> = name.split('.').collect();
    let found = match &segments[..] {
        [source, path @ ..] if source.eq_ignore_ascii_case("$Source") && !path.is_empty() => {
            catalog.path(object, path)
        }
        [global, ..] if global.starts_with('$') => named_in_formula(catalog, object, name),
        [other, path @ ..] if !path.is_empty() => catalog.path(other, path),
        _ => return None,
    };
    let is_read = !found.is_empty() && found.iter().all(|found| found.referenced_type.is_some());
    is_read.then_some(found)
}

/// What a name in a formula on `object` refers to. A name without `$` is a
/// field path from the object. A global names something by its kind. The
/// constants of `$Api` and `$System` are the platform's. A global that is
/// not known here is reported as written.
fn named_in_formula(catalog: &Catalog, object: &str, name: &str) -> Vec<Found> {
    let segments: Vec<&str> = name.split('.').collect();
    if !name.starts_with('$') {
        return catalog.path(object, &segments);
    }
    match (fold(segments[0]).as_str(), &segments[1..]) {
        ("$api" | "$system", _) => Vec::new(),
        ("$label", [label]) => {
            vec![catalog.named(&format!("Label.{label}"), ReferencedType::Label)]
        }
        ("$permission", [permission]) => {
            vec![catalog.named(permission, ReferencedType::CustomPermission)]
        }
        ("$setup" | "$objecttype", [object]) => {
            vec![catalog.named(object, ReferencedType::Object)]
        }
        ("$setup", [setting, path @ ..]) => catalog.path(setting, path),
        ("$objecttype", [object, fields, field]) if fields.eq_ignore_ascii_case("fields") => {
            catalog.path(object, &[field])
        }
        // `$CustomMetadata.Type__mdt.Record.Field__c`.
        ("$custommetadata", [record_type, record, path @ ..]) => {
            let type_name = match fold(record_type).ends_with("__mdt") {
                true => &record_type[..record_type.len() - "__mdt".len()],
                false => record_type,
            };
            let record = format!("{type_name}.{record}");
            let mut found = vec![catalog.named(&record, ReferencedType::MetadataRecord)];
            if !path.is_empty() {
                found.extend(catalog.path(record_type, path));
            }
            found
        }
        // The running user and what belongs to them are records of objects.
        ("$user" | "$profile" | "$userrole" | "$organization" | "$recordtype", [_, ..]) => {
            catalog.path(&segments[0][1..], &segments[1..])
        }
        _ => vec![catalog.undeclared(name.to_string(), None)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_impact_common::rows::Resolution;

    #[test]
    fn a_file_is_recognised_by_what_stands_before_meta_xml() {
        let kind = |path| file_kind(path);

        assert_eq!(
            kind("app/objects/Invoice__c/Invoice__c.object-meta.xml"),
            FileKind::Modelled(Kind::Object)
        );
        assert_eq!(
            kind("app/objects/Invoice__c/fields/Total__c.field-meta.xml"),
            FileKind::Modelled(Kind::Field)
        );
        assert_eq!(
            kind("app/customMetadata/Plugin.Billing.md-meta.xml"),
            FileKind::Modelled(Kind::Record)
        );
        assert_eq!(
            kind("app/layouts/Invoice__c-Invoice Layout.layout-meta.xml"),
            FileKind::Unmodelled("layout".into())
        );
        assert_eq!(
            kind("app/lwc/list/list.js-meta.xml"),
            FileKind::Unmodelled("js".into())
        );
    }

    #[test]
    fn the_entries_of_a_file_are_the_elements_under_its_root_with_their_lines() {
        let source = "<?xml version=\"1.0\"?>\n<PermissionSet>\n    <classAccesses>\n        \
                      <apexClass>Billing</apexClass>\n    </classAccesses>\n    \
                      <label>Billing</label>\n</PermissionSet>\n";
        let path = "app/permissionsets/Billing.permissionset-meta.xml";

        let metadata = read(Kind::PermissionSet, source.as_bytes(), "app", path);

        assert_eq!(metadata.entries(), [(3, 5), (6, 6)]);
    }

    #[test]
    fn a_file_whose_path_cannot_name_its_component_is_not_modelled() {
        // No object directory above them, and no type in front of the record.
        assert_eq!(
            file_kind("Total__c.field-meta.xml"),
            FileKind::Unmodelled("field".into())
        );
        assert_eq!(
            file_kind("fields/Total__c.field-meta.xml"),
            FileKind::Unmodelled("field".into())
        );
        assert_eq!(
            file_kind("app/customMetadata/Billing.md-meta.xml"),
            FileKind::Unmodelled("md".into())
        );
    }

    #[test]
    fn apex_its_companion_files_and_files_that_are_not_metadata_are_not_components() {
        for path in [
            "app/classes/Billing.cls",
            "app/classes/Billing.cls-meta.xml",
            "app/triggers/Billing.trigger-meta.xml",
            "app/lwc/list/list.js",
            "app/README.md",
            "app/package.xml",
            "app/-meta.xml",
        ] {
            assert_eq!(file_kind(path), FileKind::Other, "{path}");
        }
    }

    fn symbols(kind: Kind, path: &str, source: &str) -> Vec<String> {
        let metadata = read(kind, source.as_bytes(), "app", path);
        metadata
            .declarations
            .into_iter()
            .map(|d| d.symbol)
            .collect()
    }

    #[test]
    fn a_component_is_named_by_its_path_whatever_the_file_says() {
        let field = symbols(
            Kind::Field,
            "app/objects/Invoice__c/fields/Total__c.field-meta.xml",
            "<CustomField><fullName>Something_Else__c</fullName></CustomField>",
        );
        let record = symbols(
            Kind::Record,
            "app/customMetadata/Plugin.Billing.md-meta.xml",
            "<CustomMetadata/>",
        );
        let unreadable = symbols(
            Kind::Object,
            "app/objects/Invoice__c/Invoice__c.object-meta.xml",
            "<<<",
        );

        assert_eq!(field, ["Invoice__c.Total__c"]);
        assert_eq!(record, ["Plugin.Billing"]);
        assert_eq!(unreadable, ["Invoice__c"]);
    }

    #[test]
    fn a_labels_file_declares_the_labels_it_could_read() {
        let path = "app/labels/CustomLabels.labels-meta.xml";
        let whole = "<CustomLabels><labels><fullName>One</fullName></labels>\
                     <labels><fullName>Two</fullName></labels></CustomLabels>";
        let cut_short = "<CustomLabels><labels><fullName>One</fullName></labels><labels><oops>";
        let empty = "<CustomLabels/>";

        assert_eq!(
            symbols(Kind::Labels, path, whole),
            ["Label.One", "Label.Two"]
        );
        assert_eq!(symbols(Kind::Labels, path, cut_short), ["Label.One"]);
        assert_eq!(symbols(Kind::Labels, path, empty), [] as [&str; 0]);
    }

    #[test]
    fn a_custom_lookup_is_called_with_an_r_suffix_from_the_object_it_points_at() {
        let lookup = |path: &str| {
            let source = "<CustomField><referenceTo>Account</referenceTo>\
                          <relationshipName>Invoices</relationshipName></CustomField>";
            let metadata = read(Kind::Field, source.as_bytes(), "app", path);
            metadata
                .relationship
                .and_then(|relationship| relationship.child_name)
        };

        assert_eq!(
            lookup("app/objects/Invoice__c/fields/Account__c.field-meta.xml").as_deref(),
            Some("Invoices__r")
        );
        assert_eq!(
            lookup("app/objects/Contact/fields/AccountId.field-meta.xml").as_deref(),
            Some("Invoices")
        );
    }

    /// The rows of a custom metadata record of type `Rule__mdt` whose
    /// `values` are these, as `kind referenced` and `?` where unresolved.
    /// `Rule__mdt.Object__c` points at `EntityDefinition`, and the fields
    /// listed in `to_fields` at a field of the object that it names.
    fn record_rows(to_fields: &[(&str, &str)], values: &[(&str, &str)]) -> Vec<String> {
        record_rows_with(to_fields, &[], values)
    }

    /// As `record_rows`, where the fields in `plain` also have a
    /// declaration that is no relationship: a text field, say. A class
    /// `Run` is declared too.
    fn record_rows_with(
        to_fields: &[(&str, &str)],
        plain: &[&str],
        values: &[(&str, &str)],
    ) -> Vec<String> {
        let relationship = |field: &str, to: &str, controlling: Option<&str>| Relationship {
            field: format!("Rule__mdt.{field}"),
            to: to.into(),
            child_name: None,
            controlling: controlling.map(String::from),
        };
        let mut relationships = vec![relationship("Object__c", "EntityDefinition", None)];
        for (field, to) in to_fields {
            relationships.push(relationship(field, to, Some("Rule__mdt.Object__c")));
        }
        let declared = |kind, symbol: String| Declaration {
            name: symbol.clone(),
            symbol,
            kind,
            package: "app".into(),
            file: String::new(),
            start: 1,
            end: 1,
            is_test: false,
            annotations: Vec::new(),
            collision: false,
            scope: None,
        };
        // Every declaration of a field is a declaration, relationship or not.
        let fields = relationships
            .iter()
            .map(|relationship| relationship.field.clone());
        let fields = fields.chain(plain.iter().map(|field| format!("Rule__mdt.{field}")));
        let mut declarations: Vec<Declaration> = fields
            .map(|field| declared(DeclType::ObjectField, field))
            .collect();
        declarations.push(declared(
            DeclType::ObjectField,
            "Invoice__c.Total__c".into(),
        ));
        declarations.push(declared(DeclType::Class, "Run".into()));
        let catalog = Catalog::build(&declarations, &relationships, &[]);
        let values: String = values
            .iter()
            .map(|(field, value)| {
                format!("<values><field>{field}</field><value>{value}</value></values>")
            })
            .collect();
        let source = format!("<CustomMetadata>{values}</CustomMetadata>");
        let path = "app/customMetadata/Rule.One.md-meta.xml";
        let metadata = read(Kind::Record, source.as_bytes(), "app", path);
        let rows = references(&metadata, &catalog, "app", path);
        let describe = |row: Row| match row {
            Row::Reference(reference) if reference.kind == RefKind::Cmdt => {
                let unresolved = reference.resolution == Some(Resolution::Unresolved);
                Some(format!(
                    "{}{}",
                    reference.referenced,
                    if unresolved { "?" } else { "" }
                ))
            }
            _ => None,
        };
        rows.into_iter().filter_map(describe).collect()
    }

    #[test]
    fn a_value_of_a_relationship_to_a_field_is_a_field_of_the_object_another_value_names() {
        let values = [("Field__c", "total__c"), ("Object__c", "Invoice__c")];

        for to in ["FieldDefinition", "EntityParticle"] {
            let rows = record_rows(&[("Field__c", to)], &values);

            assert_eq!(rows, ["Invoice__c.Total__c", "Invoice__c?"], "{to}");
        }
    }

    #[test]
    fn the_field_that_names_the_object_is_found_whatever_case_the_record_writes_it_in() {
        let values = [("Field__c", "Total__c"), ("OBJECT__C", "Invoice__c")];

        let rows = record_rows(&[("Field__c", "FieldDefinition")], &values);

        assert_eq!(rows, ["Invoice__c.Total__c", "Invoice__c?"]);
    }

    /// Regression: text in one package directory, a relationship
    /// to another type in another. Neither reading is dropped.
    #[test]
    fn a_field_declared_as_text_and_as_a_relationship_has_its_value_read_both_ways() {
        let rows = record_rows_with(
            &[("Handler__c", "Other__mdt")],
            &["Handler__c"],
            &[("Handler__c", "run")],
        );

        assert_eq!(rows, ["Other.run?", "Run"]);
    }

    #[test]
    fn a_field_declared_only_as_a_relationship_is_not_also_matched_as_a_plain_value() {
        let rows = record_rows(&[("Handler__c", "Other__mdt")], &[("Handler__c", "Run")]);

        assert_eq!(rows, ["Other.Run?"]);
    }

    #[test]
    fn a_relationship_field_declared_twice_the_same_way_gives_its_value_one_row() {
        let twice = [
            ("Field__c", "FieldDefinition"),
            ("Field__c", "FieldDefinition"),
        ];

        let rows = record_rows(
            &twice,
            &[("Field__c", "Total__c"), ("Object__c", "Invoice__c")],
        );

        assert_eq!(rows, ["Invoice__c.Total__c", "Invoice__c?"]);
    }

    #[test]
    fn a_relationship_field_declared_twice_differently_gives_its_value_a_row_each_way() {
        let differently = [
            ("Field__c", "FieldDefinition"),
            ("Field__c", "EntityDefinition"),
        ];

        let rows = record_rows(&differently, &[("Field__c", "Total__c")]);

        // A field of an object the record does not name, or an object.
        assert_eq!(rows, ["Total__c?", "Total__c?"]);
    }

    #[test]
    fn a_value_of_a_field_that_is_no_relationship_is_a_row_only_if_it_equals_a_symbol() {
        let rows = record_rows(
            &[],
            &[("Note__c", "Invoice__c.Total__c"), ("Other__c", "Total__c")],
        );

        assert_eq!(rows, ["Invoice__c.Total__c"]);
    }

    /// What a name in a formula on `Invoice__c` is, in a catalog that
    /// declares nothing: the names it would be looked up under.
    fn looked_up(name: &str) -> Vec<String> {
        let catalog = Catalog::build([], [], &[]);
        let found = named_in_formula(&catalog, "Invoice__c", name);
        assert!(
            found
                .iter()
                .all(|found| found.resolution == Resolution::Unresolved)
        );
        found.into_iter().map(|found| found.referenced).collect()
    }

    #[test]
    fn a_field_in_a_lookup_filter_is_of_the_object_it_starts_with_or_of_the_lookups_own() {
        let catalog = Catalog::build([], [], &[]);
        let named = |name: &str| -> Option<Vec<String>> {
            let found = named_in_filter(&catalog, "Invoice__c", name)?;
            Some(found.into_iter().map(|found| found.referenced).collect())
        };
        let field = |name: &str| Some(vec![name.to_string()]);

        assert_eq!(named("Account.Owner.Name"), field("Account.Owner.Name"));
        assert_eq!(named("$Source.Total__c"), field("Invoice__c.Total__c"));
        assert_eq!(named("$User.Region__c"), field("User.Region__c"));
        assert_eq!(named("$Profile.Name"), field("Profile.Name"));
    }

    #[test]
    fn a_lookup_filter_value_that_is_no_field_this_tool_can_place_is_not_read() {
        let catalog = Catalog::build([], [], &[]);

        // A field without its object, `$Source` without a field, a global
        // that is not known, and a constant of the platform's.
        for name in ["Total__c", "$Source", "$Future.Field", "$Api.Session_ID"] {
            assert!(
                named_in_filter(&catalog, "Invoice__c", name).is_none(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_global_names_a_label_a_permission_a_setting_or_a_record() {
        assert_eq!(looked_up("$Label.Greeting"), ["Label.Greeting"]);
        assert_eq!(looked_up("$Permission.Can_Approve"), ["Can_Approve"]);
        assert_eq!(looked_up("$Setup.Limits__c.Max__c"), ["Limits__c.Max__c"]);
        assert_eq!(looked_up("$Setup.Limits__c"), ["Limits__c"]);
        assert_eq!(
            looked_up("$CustomMetadata.Plugin__mdt.Billing.Handler__c"),
            ["Plugin.Billing", "Plugin__mdt.Handler__c"]
        );
        assert_eq!(
            looked_up("$CustomMetadata.Plugin__mdt.Billing"),
            ["Plugin.Billing"]
        );
    }

    #[test]
    fn a_global_for_the_running_user_or_an_object_type_names_fields_of_an_object() {
        assert_eq!(looked_up("$User.Region__c"), ["User.Region__c"]);
        assert_eq!(looked_up("$Profile.Name"), ["Profile.Name"]);
        assert_eq!(
            looked_up("$ObjectType.Account.Fields.Rating"),
            ["Account.Rating"]
        );
        assert_eq!(looked_up("$ObjectType.Account"), ["Account"]);
    }

    #[test]
    fn a_platform_constant_is_no_reference_and_an_unknown_global_is_one_as_written() {
        assert_eq!(looked_up("$Api.Session_ID"), [] as [&str; 0]);
        assert_eq!(looked_up("$System.OriginDateTime"), [] as [&str; 0]);
        assert_eq!(looked_up("$Action.Account.New"), ["$Action.Account.New"]);
        assert_eq!(looked_up("$Label"), ["$Label"]);
    }

    #[test]
    fn a_name_without_a_dollar_sign_is_a_field_path_from_the_formulas_object() {
        assert_eq!(looked_up("Total__c"), ["Invoice__c.Total__c"]);
        assert_eq!(looked_up("Owner.Name"), ["Invoice__c.Owner.Name"]);
    }
}
