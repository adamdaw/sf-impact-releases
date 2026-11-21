//! Flow and bundle references before resolution. XML and JS scanners report
//! diagnostics independently of the textual tier, including on partial input.
use crate::declarative::{Catalog, Found};
use crate::index::{Index, Outside};
use sf_impact_common::rows::{
    DeclType, Declaration, Detail, RefKind, Reference, ReferencedType, Resolution, Tier,
    UnsupportedReason, fold,
};
use sf_impact_common::scope::ScopeFile;

pub mod flow;
pub mod js;
pub mod lwc;
pub mod xml;

#[derive(Clone, Copy, Debug)]
// Metadata targets never bind to a same-named Apex declaration.
pub enum Target {
    Class,
    Action,
    Method,
    Flow,
    Lwc,
    Object,
    Field,
    Label,
    Permission,
    Channel,
    Exposure,
    MetadataRecord,
}

impl Target {
    /// What kind of thing the name is. `None` for a standard action and a
    /// page an LWC is exposed on, which this tool does not declare.
    fn referenced_type(self) -> Option<ReferencedType> {
        Some(match self {
            Target::Class => ReferencedType::Class,
            Target::Method => ReferencedType::Method,
            Target::Flow => ReferencedType::Flow,
            Target::Lwc => ReferencedType::Lwc,
            Target::Object => ReferencedType::Object,
            Target::Field => ReferencedType::ObjectField,
            Target::Label => ReferencedType::Label,
            Target::Permission => ReferencedType::CustomPermission,
            Target::Channel => ReferencedType::MessageChannel,
            Target::MetadataRecord => ReferencedType::MetadataRecord,
            Target::Action | Target::Exposure => return None,
        })
    }
}

pub struct Named {
    pub name: String,
    pub kind: RefKind,
    pub target: Target,
    pub offset: usize,
    pub unknown: bool,
}

#[derive(Default)]
pub struct Parsed {
    pub references: Vec<Named>,
    /// Offsets at which a scanner could not read the file: syntax errors
    /// and what could not be recovered. They become one `parse_error` row.
    pub errors: Vec<usize>,
    /// Elements that are well-formed but not read, each with its path below
    /// the root and its offset. Each becomes an `unsupported_element` row.
    pub unsupported: Vec<(String, usize, UnsupportedReason)>,
}

impl Parsed {
    pub fn add(&mut self, name: &str, kind: RefKind, target: Target, offset: usize) {
        if !name.is_empty() {
            self.references.push(Named {
                name: name.into(),
                kind,
                target,
                offset,
                unknown: false,
            });
        }
    }
}

pub fn flow_name(path: &str) -> Option<&str> {
    path.rsplit('/').next()?.strip_suffix(".flow-meta.xml")
}

/// The path identifies a bundle even when its main JS or metadata file is absent.
pub fn bundle(path: &str) -> Option<(&str, &str)> {
    let mut offset = 0;
    let mut parts = path.split('/');
    while let Some(part) = parts.next() {
        offset += part.len() + 1;
        if part == "lwc" {
            let name = parts.next()?;
            if name.is_empty() || parts.next().is_none() {
                return None;
            }
            return Some((&path[..offset + name.len()], name));
        }
    }
    None
}

/// Prefer the main JS, then template, then metadata; otherwise first path.
/// Exactly one file holds the declaration, all bundle files are referrers.
pub fn bundle_declarations(files: &[ScopeFile]) -> std::collections::HashSet<String> {
    let mut chosen = std::collections::BTreeMap::new();
    for file in files {
        if let Some((path, name)) = bundle(&file.path) {
            let rank = if file.path == format!("{path}/{name}.js") {
                0
            } else if file.path == format!("{path}/{name}.html") {
                1
            } else if file.path == format!("{path}/{name}.js-meta.xml") {
                2
            } else {
                3
            };
            let candidate = (rank, &file.path);
            chosen
                .entry(path)
                .and_modify(|old| {
                    if candidate < *old {
                        *old = candidate;
                    }
                })
                .or_insert(candidate);
        }
    }
    chosen.into_values().map(|(_, path)| path.clone()).collect()
}

pub fn declaration(file: &ScopeFile, source: &[u8], bundle_anchor: bool) -> Option<Declaration> {
    let (name, kind) = if let Some(name) = flow_name(&file.path) {
        (name, DeclType::Flow)
    } else if bundle_anchor {
        (bundle(&file.path)?.1, DeclType::Lwc)
    } else {
        return None;
    };
    Some(Declaration {
        symbol: name.into(),
        name: name.into(),
        kind,
        package: file.package.clone(),
        file: file.path.clone(),
        start: 1,
        end: (source.split(|b| *b == b'\n').count() - usize::from(source.ends_with(b"\n"))).max(1)
            as u32,
        is_test: false,
        annotations: Vec::new(),
        collision: false,
        scope: None,
    })
}

pub fn parse(file: &ScopeFile, source: &[u8]) -> Parsed {
    if flow_name(&file.path).is_none()
        && !file.path.ends_with(".js")
        && !file.path.ends_with(".ts")
        && !file.path.ends_with(".css")
        && !file.path.ends_with(".html")
        && !file.path.ends_with(".js-meta.xml")
    {
        return Parsed::default();
    }
    let text = match std::str::from_utf8(source) {
        Ok(text) => text,
        Err(e) => {
            return Parsed {
                errors: vec![e.valid_up_to()],
                ..Parsed::default()
            };
        }
    };
    if flow_name(&file.path).is_some() {
        flow::parse(text)
    } else if file.path.ends_with(".js") || file.path.ends_with(".ts") {
        lwc::javascript(text)
    } else if file.path.ends_with(".css") {
        lwc::stylesheet(text)
    } else if file.path.ends_with(".html") {
        lwc::template(text)
    } else if file.path.ends_with(".js-meta.xml") {
        lwc::meta(text)
    } else {
        Parsed::default()
    }
}

pub fn position(source: &[u8], offset: usize) -> (u32, u32) {
    let prefix = &source[..offset.min(source.len())];
    let line = prefix.iter().filter(|b| **b == b'\n').count() as u32 + 1;
    let column = prefix
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(prefix.len(), |at| prefix.len() - at - 1) as u32
        + 1;
    (line, column)
}

/// Translate the scanner's names to catalog identities. Resolution and
/// relationship traversal belong to the catalog shared with Apex and formulas.
fn declared(named: &Named, catalog: &Catalog) -> Option<Vec<Found>> {
    let found = match named.target {
        Target::Object => vec![catalog.named(&named.name, ReferencedType::Object)],
        Target::Field => {
            let (object, path) = named.name.split_once('.')?;
            catalog.path(object, &path.split('.').collect::<Vec<_>>())
        }
        Target::Label => vec![catalog.named(&named.name, ReferencedType::Label)],
        Target::Permission => vec![
            catalog.named(
                named
                    .name
                    .strip_prefix("CustomPermission.")
                    .unwrap_or(&named.name),
                ReferencedType::CustomPermission,
            ),
        ],
        Target::MetadataRecord => {
            let (object, record) = named.name.split_once('.')?;
            let type_name = if fold(object).ends_with("__mdt") {
                &object[..object.len() - "__mdt".len()]
            } else {
                object
            };
            vec![catalog.named(
                &format!("{type_name}.{record}"),
                ReferencedType::MetadataRecord,
            )]
        }
        Target::Channel => vec![catalog.named(&named.name, ReferencedType::MessageChannel)],
        _ => return None,
    };
    // As in Apex, an entirely undeclared name retains its spelling and
    // namespace handling. A partially declared path keeps every catalog result,
    // including the unresolved ends of a divergent lookup.
    found
        .iter()
        .any(|f| f.resolution == Resolution::Declaration)
        .then_some(found)
}

pub fn resolve(
    parsed: &Parsed,
    source: &[u8],
    file: &ScopeFile,
    index: &Index,
    catalog: &Catalog,
) -> Vec<Reference> {
    let owner = flow_name(&file.path).or_else(|| bundle(&file.path).map(|(_, name)| name));
    parsed
        .references
        .iter()
        .flat_map(|named| {
            let (line, column) = position(source, named.offset);
            let mut row = Reference {
                tier: Tier::Parsed,
                kind: named.kind,
                referenced: named.name.clone(),
                referenced_type: named.target.referenced_type(),
                resolution: Some(Resolution::Unresolved),
                detail: None,
                candidates: Vec::new(),
                referrer: owner.map(String::from),
                source_field: None,
                package: file.package.clone(),
                file: file.path.clone(),
                line,
                column,
            };
            if named.unknown {
                row.detail = Some(Detail::UnknownType);
                return vec![row];
            }
            if let Some(found) = declared(named, catalog) {
                return found
                    .into_iter()
                    .map(|found| Reference {
                        referenced: found.referenced,
                        referenced_type: found.referenced_type,
                        resolution: Some(found.resolution),
                        detail: (!found.candidates.is_empty()).then_some(Detail::AmbiguousReceiver),
                        candidates: found.candidates,
                        ..row.clone()
                    })
                    .collect();
            }
            let kind = match named.target {
                Target::Class => Some(DeclType::Class),
                Target::Flow => Some(DeclType::Flow),
                Target::Lwc => Some(DeclType::Lwc),
                _ => None,
            };
            if let Some(symbol) = kind.and_then(|kind| index.declared(kind, &named.name)) {
                row.referenced = symbol.into();
                row.resolution = Some(Resolution::Declaration);
            } else if matches!(named.target, Target::Method)
                && let Some((class, method)) = named.name.rsplit_once('.')
            {
                let keys = index.resolve_types(class, "");
                let mut symbols: Vec<_> = keys
                    .iter()
                    .flat_map(|key| index.methods(key, method))
                    .map(|m| m.symbol.clone())
                    .collect();
                symbols.sort();
                // Signature comparison is case-insensitive just like the Apex index.
                let mut seen = std::collections::HashSet::new();
                symbols.retain(|symbol| seen.insert(fold(symbol)));
                if !symbols.is_empty() {
                    row.resolution = Some(Resolution::Declaration);
                    if symbols.len() == 1 {
                        row.referenced = symbols.remove(0);
                    } else {
                        row.detail = Some(if keys.iter().any(|k| index.is_tainted(k)) {
                            Detail::AmbiguousReceiver
                        } else {
                            Detail::AmbiguousOverload
                        });
                        row.candidates = symbols;
                    }
                } else if !keys.is_empty() {
                    row.detail = Some(Detail::NoMember);
                }
            }
            // Metadata names are never omitted as Apex platform types. Strip only
            // our category prefix for namespace detection, not the actual API name.
            let external_name = match named.target {
                Target::Label => named.name.strip_prefix("Label.").unwrap_or(&named.name),
                Target::Permission => named
                    .name
                    .strip_prefix("CustomPermission.")
                    .unwrap_or(&named.name),
                Target::Channel => named
                    .name
                    .strip_prefix("MessageChannel.")
                    .unwrap_or(&named.name),
                _ => &named.name,
            };
            let namespace = if matches!(
                named.target,
                Target::Permission | Target::Class | Target::Flow | Target::Action | Target::Lwc
            ) {
                external_name
                    .split_once("__")
                    .or_else(|| external_name.split_once(':'))
                    .map(|(ns, _)| ns)
                    .unwrap_or(external_name)
            } else {
                external_name
            };
            if row.resolution != Some(Resolution::Declaration)
                && index.foreign(namespace, false).outside == Outside::External
            {
                row.resolution = Some(Resolution::External);
            }
            vec![row]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binary_bundle_assets_are_not_misreported_as_parse_failures() {
        let file = ScopeFile {
            package: "pkg".into(),
            path: "pkg/lwc/example/icon.png".into(),
        };
        assert!(parse(&file, b"\xff\0").errors.is_empty());
    }
}
