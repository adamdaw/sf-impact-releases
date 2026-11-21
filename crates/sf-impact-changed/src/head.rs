//! What the refs say of the head: the declarations of each file and
//! their lines, which is what a hunk is mapped onto, and the rows that say
//! where the extraction of a file fell short.

use sf_impact_common::rows::{DeclType, Declaration, RefKind, Reference, Row, Tier, fold};
use sf_impact_extract::declarative::{Catalog, Relationship};
use std::collections::{HashMap, HashSet};

pub struct Head {
    /// Per file, what it declares, in the order of the file.
    declarations: HashMap<String, Vec<Declaration>>,
    /// Per directory of an LWC bundle, the declaration of the bundle.
    bundles: HashMap<String, Declaration>,
    /// Every declared name, case-folded, with the kind of thing it is.
    names: HashSet<(DeclType, String)>,
    /// The files that did not parse cleanly.
    parse_errors: HashSet<String>,
    /// Per file that is read entry by entry, the references read from it
    /// and the lines of the elements of it that were not read.
    references: HashMap<String, Vec<Reference>>,
    unsupported: HashMap<String, Vec<u32>>,
    /// The Flows with an element that was not read.
    flows_not_read: HashSet<String>,
    /// What is declared at head, for a name read from a base version.
    pub catalog: Catalog,
}

impl Head {
    /// Reads the rows of the refs, which their pack has held to the rules
    /// of refs.jsonl. `namespaces` are those of the config file: a name in
    /// one of them is external.
    pub fn read(rows: Vec<Row>, namespaces: &[String]) -> Head {
        let mut head = Head {
            declarations: HashMap::new(),
            bundles: HashMap::new(),
            names: HashSet::new(),
            parse_errors: HashSet::new(),
            references: HashMap::new(),
            unsupported: HashMap::new(),
            flows_not_read: HashSet::new(),
            catalog: Catalog::build([], [], namespaces),
        };
        // What a lookup field points at and what controls it, by the file
        // that declares the field: a field declared twice can be two
        // relationships, and neither stands for the other.
        let (mut lookups, mut controlling) = (Vec::new(), HashMap::new());
        for row in rows {
            match row {
                Row::Declaration(declaration) => {
                    head.names
                        .insert(name(declaration.kind, &declaration.symbol));
                    if let Some((bundle, _)) = sf_impact_extract::bundle(&declaration.file)
                        && declaration.kind == DeclType::Lwc
                    {
                        head.bundles.insert(bundle.to_string(), declaration.clone());
                    }
                    let file = head.declarations.entry(declaration.file.clone());
                    file.or_default().push(declaration);
                }
                Row::ParseError(error) => {
                    head.parse_errors.insert(error.file);
                }
                Row::Reference(reference) if reference.tier == Tier::Parsed => {
                    let referrer = reference.referrer.clone().unwrap_or_default();
                    match reference.kind {
                        RefKind::Lookup => {
                            lookups.push((reference.file, referrer, reference.referenced));
                        }
                        RefKind::ControllingField => {
                            controlling.insert(reference.file, reference.referenced);
                        }
                        _ if sf_impact_extract::has_entries(&reference.file) => {
                            let file = head.references.entry(reference.file.clone());
                            file.or_default().push(reference);
                        }
                        _ => {}
                    }
                }
                Row::UnsupportedElement(element)
                    if sf_impact_extract::has_entries(&element.file) =>
                {
                    let file = head.unsupported.entry(element.file);
                    file.or_default().push(element.line);
                }
                Row::UnsupportedElement(element)
                    if sf_impact_extract::flow_name(&element.file).is_some() =>
                {
                    head.flows_not_read.insert(element.file);
                }
                _ => {}
            }
        }
        // The catalog follows a custom metadata relationship through the
        // field's declaration. The rows of the field say what that was.
        let relationships: Vec<Relationship> = lookups
            .into_iter()
            .map(|(file, field, to)| Relationship {
                controlling: controlling.get(&file).cloned(),
                field,
                to,
                child_name: None,
            })
            .collect();
        let declarations = head.declarations.values().flatten();
        head.catalog = Catalog::build(declarations, &relationships, namespaces);
        head
    }

    pub fn declarations(&self, file: &str) -> &[Declaration] {
        self.declarations.get(file).map_or(&[], Vec::as_slice)
    }

    /// The declaration of the LWC bundle in the directory `bundle`.
    pub fn bundle(&self, bundle: &str) -> Option<&Declaration> {
        self.bundles.get(bundle)
    }

    /// Whether anything at head has this name and is this kind of thing.
    pub fn declares(&self, kind: DeclType, symbol: &str) -> bool {
        self.names.contains(&name(kind, symbol))
    }

    pub fn has_parse_error(&self, file: &str) -> bool {
        self.parse_errors.contains(file)
    }

    /// Whether the file is a Flow with an element that was not read.
    pub fn is_flow_not_read(&self, file: &str) -> bool {
        self.flows_not_read.contains(file)
    }

    /// The references read from a file that is read entry by entry.
    pub fn references(&self, file: &str) -> &[Reference] {
        self.references.get(file).map_or(&[], Vec::as_slice)
    }

    /// The lines of the elements of such a file that were not read.
    pub fn unsupported(&self, file: &str) -> &[u32] {
        self.unsupported.get(file).map_or(&[], Vec::as_slice)
    }
}

/// What something is called, and among what: classes, interfaces and enums
/// share their names.
fn name(kind: DeclType, symbol: &str) -> (DeclType, String) {
    let kind = match kind {
        DeclType::Interface | DeclType::Enum => DeclType::Class,
        kind => kind,
    };
    (kind, fold(symbol))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_impact_common::rows::{ReferencedType, Resolution};

    const FIELD: &str = r#"{"row":"declaration","symbol":"Rule__mdt.Target__c","name":"Target__c","type":"object_field","package":"app","file":"app/objects/Rule__mdt/fields/Target__c.field-meta.xml","start":1,"end":5,"is_test":false,"annotations":[],"collision":false,"scope":"packageDirectories"}"#;
    const LOOKUP: &str = r#"{"row":"reference","tier":"parsed","kind":"lookup","referenced":"EntityDefinition","referenced_type":"object","resolution":"unresolved","referrer":"Rule__mdt.Target__c","package":"app","file":"app/objects/Rule__mdt/fields/Target__c.field-meta.xml","line":4,"column":18}"#;
    const CLASS: &str = r#"{"row":"declaration","symbol":"Invoice","name":"Invoice","type":"class","package":"app","file":"app/classes/Invoice.cls","start":1,"end":9,"is_test":false,"annotations":[],"collision":false,"scope":"packageDirectories"}"#;
    const TEXTUAL: &str = r#"{"row":"reference","tier":"textual","kind":"text","referenced":"Invoice","referrer":null,"package":"app","file":"app/classes/Invoice.cls","line":1,"column":14}"#;

    fn head(lines: &[&str]) -> Head {
        let rows = lines.iter().map(|line| serde_json::from_str(line).unwrap());
        Head::read(rows.collect(), &[])
    }

    #[test]
    fn the_declarations_of_a_file_are_those_of_its_rows() {
        let head = head(&[CLASS, TEXTUAL, FIELD]);

        let symbols: Vec<_> = head
            .declarations("app/classes/Invoice.cls")
            .iter()
            .collect();

        assert_eq!(symbols.len(), 1);
        assert_eq!((symbols[0].start, symbols[0].end), (1, 9));
        assert_eq!(head.declarations("app/classes/Other.cls"), []);
    }

    #[test]
    fn an_interface_has_the_name_a_class_would_have() {
        let head = head(&[CLASS]);

        assert!(head.declares(DeclType::Class, "Invoice"));
        assert!(head.declares(DeclType::Interface, "INVOICE"));
        assert!(!head.declares(DeclType::Flow, "Invoice"));
    }

    /// A value of a relationship to an object names an object whether or
    /// not anything declares it; a plain value names only what is declared.
    #[test]
    fn the_catalog_knows_a_relationship_field_from_the_rows_of_the_field() {
        let record = sf_impact_common::scope::ScopeFile {
            path: "app/customMetadata/Rule.One.md-meta.xml".into(),
            package: "app".into(),
        };
        let content = b"<CustomMetadata><values><field>Target__c</field>\
                        <value>Account</value></values></CustomMetadata>";
        let record_read = sf_impact_extract::read_entries(&record, content.to_vec()).unwrap();
        let named = |head: &Head| -> Vec<(RefKind, String, Option<ReferencedType>)> {
            let rows = sf_impact_extract::declarative::references(
                &record_read,
                &head.catalog,
                &record.package,
                &record.path,
            );
            let references = rows.into_iter().filter_map(|row| match row {
                Row::Reference(reference) if reference.kind == RefKind::Cmdt => Some(reference),
                _ => None,
            });
            references
                .map(|r| {
                    assert_eq!(r.resolution, Some(Resolution::Unresolved));
                    (r.kind, r.referenced, r.referenced_type)
                })
                .collect()
        };

        let with_lookup = named(&head(&[FIELD, LOOKUP]));
        let without = named(&head(&[FIELD]));

        assert_eq!(
            with_lookup,
            [(
                RefKind::Cmdt,
                "Account".into(),
                Some(ReferencedType::Object)
            )]
        );
        assert_eq!(without, []);
    }
}
