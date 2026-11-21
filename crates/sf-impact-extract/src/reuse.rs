//! The previous pack, as a run that reuses it reads it: its rows by file,
//! and its manifest, which says what each file was when the rows were made
//! of it and what phase one made of it that the rows do not hold.
//!
//! A run reuses a file's rows only if nothing that its rows are made of
//! has changed: the bytes of the file (its blob id), whether the file is
//! where its bundle is declared, and what every other file declares. The
//! last is the input to `Index::build`, `Catalog::build` and `Names::new`,
//! and it is compared directly, file by file, for the files that changed:
//! a file whose bytes did not change declares what it did.

use crate::blob::Format;
use crate::declarative::Relationship;
use crate::declare::{Callable, Field, TypeDecl};
use crate::metadata;
use sf_impact_common::pack::{self, Manifest, Pack, RelationshipFact};
use sf_impact_common::provenance::{self, Writer};
use sf_impact_common::rows::{Declaration, ReferencedType, Row, fold};
use sf_impact_common::scope::ScopeFile;
use std::collections::{HashMap, HashSet};
use std::ops::Range;

/// What a pack must have been written with for its rows to be reused: the
/// same tool and schema, the same scope and config, and blob ids of the
/// same length.
pub struct Identity<'a> {
    pub writer: Writer,
    pub scope: &'a provenance::Scope,
    pub config: &'a str,
    pub format: Format,
}

/// A previous pack that can be reused.
pub struct Previous {
    pack: Pack,
    manifest: Manifest,
    /// The index of each file's entry in the manifest, by path.
    entries: HashMap<String, usize>,
    /// For each entry, its rows in `pack.rows`, its types in
    /// `manifest.types` and its relationships in `manifest.relationships`.
    rows: Vec<Range<usize>>,
    types: Vec<Range<usize>>,
    relationships: Vec<Range<usize>>,
    /// The files that declared their bundle.
    anchors: HashSet<String>,
}

impl Previous {
    /// The previous pack, if it can be reused; why not otherwise. A pack
    /// that cannot be read, or that was written of anything else than
    /// `identity` says, cannot.
    pub fn of(bytes: &[u8], identity: &Identity) -> Result<Previous, String> {
        let mut pack = Pack::decode(bytes)?;
        let manifest = pack.manifest.take().ok_or("it has no manifest")?;
        let line: provenance::Refs = serde_json::from_str(&pack.provenance)
            .map_err(|e| format!("its line 1 is not read: {e}"))?;
        if line.writer != identity.writer {
            return Err(format!(
                "it was written by {} {}, schema {}",
                line.writer.tool, line.writer.version, line.writer.schema
            ));
        }
        if line.scope != *identity.scope {
            return Err("it is of another scope".into());
        }
        if line.config != identity.config {
            return Err("it is of another config".into());
        }
        let length = match identity.format {
            Format::Sha1 => 20,
            Format::Sha256 => 32,
        };
        if manifest
            .files
            .iter()
            .any(|entry| entry.blob.len() != length)
        {
            return Err("it is of a repository that names its objects otherwise".into());
        }

        let entries: HashMap<String, usize> = manifest
            .files
            .iter()
            .enumerate()
            .map(|(at, entry)| (entry.path.clone(), at))
            .collect();
        let mut rows = vec![0..0; manifest.files.len()];
        let mut at = 0;
        for run in pack
            .rows
            .chunk_by(|a, b| pack::located(a) == pack::located(b))
        {
            if let Some((_, path)) = pack::located(&run[0]) {
                rows[entries[path]] = at..at + run.len();
            }
            at += run.len();
        }
        let types = ranges(manifest.files.len(), manifest.types.iter().map(|t| t.file));
        let files = manifest.relationships.iter().map(|r| r.file);
        let relationships = ranges(manifest.files.len(), files);
        let files: Vec<ScopeFile> = manifest
            .files
            .iter()
            .map(|entry| ScopeFile {
                package: entry.package.clone(),
                path: entry.path.clone(),
            })
            .collect();
        Ok(Previous {
            anchors: metadata::bundle_declarations(&files),
            pack,
            manifest,
            entries,
            rows,
            types,
            relationships,
        })
    }

    /// The entry of a file, by exact package and path.
    pub fn entry(&self, file: &ScopeFile) -> Option<usize> {
        let at = *self.entries.get(&file.path)?;
        (self.manifest.files[at].package == file.package).then_some(at)
    }

    /// The entry of a file whose rows can be reused as they are: one whose
    /// bytes, by their blob id `id`, are those the rows were made of, and
    /// that declares its bundle now if, and only if, it did then.
    pub fn unchanged(&self, file: &ScopeFile, id: &str, is_anchor: bool) -> Option<usize> {
        let at = self.entry(file)?;
        let same = hex(&self.manifest.files[at].blob) == id
            && self.anchors.contains(&file.path) == is_anchor;
        same.then_some(at)
    }

    /// How many files the pack has an entry of.
    pub fn entries(&self) -> usize {
        self.manifest.files.len()
    }

    /// The path of an entry's file.
    pub fn path(&self, entry: usize) -> &str {
        &self.manifest.files[entry].path
    }

    pub fn rows(&self, entry: usize) -> &[Row] {
        &self.pack.rows[self.rows[entry].clone()]
    }

    /// What an entry's file declared, as its rows hold it.
    pub fn declarations(&self, entry: usize) -> impl Iterator<Item = &Declaration> {
        self.rows(entry).iter().filter_map(|row| match row {
            Row::Declaration(declaration) => Some(declaration),
            _ => None,
        })
    }

    /// An entry's types, as the manifest has them.
    pub fn types(&self, entry: usize) -> &[pack::TypeDecl] {
        &self.manifest.types[self.types[entry].clone()]
    }

    pub fn relationships(&self, entry: usize) -> &[RelationshipFact] {
        &self.manifest.relationships[self.relationships[entry].clone()]
    }
}

/// For each of `count` files, the range of a list sorted by file that is
/// that file's, given the file of each item.
fn ranges(count: usize, files: impl Iterator<Item = usize>) -> Vec<Range<usize>> {
    let mut ranges = vec![0..0; count];
    for (at, file) in files.enumerate() {
        match ranges[file].is_empty() {
            true => ranges[file] = at..at + 1,
            false => ranges[file].end = at + 1,
        }
    }
    ranges
}

/// A declaration as building the index, the catalog and the names reads
/// it: without its lines, and without what phase two adds.
fn declared(declaration: &Declaration) -> String {
    let declaration = Declaration {
        start: 0,
        end: 0,
        collision: false,
        scope: None,
        ..declaration.clone()
    };
    serde_json::to_string(&declaration).expect("a declaration has plain values")
}

/// Whether a file that changed declares what it did: the same
/// declarations, in any order, since nothing built from them depends on
/// their order; and the same types and relationships, in the same order.
pub fn declares_the_same(
    declarations: &[&Declaration],
    types: &[TypeDecl],
    relationships: &[Relationship],
    previous: Option<(&Previous, usize)>,
) -> bool {
    let mut now: Vec<String> = declarations.iter().copied().map(declared).collect();
    let Some((previous, entry)) = previous else {
        return now.is_empty() && types.is_empty() && relationships.is_empty();
    };
    let mut then: Vec<String> = previous.declarations(entry).map(declared).collect();
    now.sort_unstable();
    then.sort_unstable();
    let then_types = previous.types(entry).iter().map(from_manifest);
    let then_relationships = previous.relationships(entry).iter().map(relationship);
    now == then
        && types.iter().eq(then_types.collect::<Vec<_>>().iter())
        && relationships
            .iter()
            .eq(then_relationships.collect::<Vec<_>>().iter())
}

/// Whether a file that is gone declared nothing.
pub fn declared_nothing(previous: &Previous, entry: usize) -> bool {
    previous.declarations(entry).next().is_none()
        && previous.types(entry).is_empty()
        && previous.relationships(entry).is_empty()
}

/// A type as the manifest of the pack keeps it, as the file at `file`
/// declared it. What it leaves out is what phase one derives: a member's
/// owner is the type, and a callable's key is its name, case-folded.
pub fn to_manifest(file: usize, ty: &TypeDecl) -> pack::TypeDecl {
    let callable = |c: &Callable| pack::Callable {
        symbol: c.symbol.clone(),
        name: c.name.clone(),
        params: c.params.clone(),
        returns: c.returns.clone(),
    };
    pack::TypeDecl {
        file,
        symbol: ty.symbol.clone(),
        superclass: ty.superclass.clone(),
        interfaces: ty.interfaces.clone(),
        methods: ty.methods.iter().map(callable).collect(),
        constructors: ty.constructors.iter().map(callable).collect(),
        fields: ty
            .fields
            .iter()
            .map(|f| pack::Field {
                symbol: f.symbol.clone(),
                key: f.key.clone(),
                ty: f.ty.clone(),
                is_static: f.is_static,
                is_enum_value: f.kind == ReferencedType::EnumValue,
            })
            .collect(),
    }
}

/// The type that [`to_manifest`] keeps, as phase one declared it.
pub fn from_manifest(ty: &pack::TypeDecl) -> TypeDecl {
    let callable = |c: &pack::Callable| Callable {
        symbol: c.symbol.clone(),
        owner: ty.symbol.clone(),
        name: c.name.clone(),
        key: fold(&c.name),
        params: c.params.clone(),
        returns: c.returns.clone(),
    };
    TypeDecl {
        symbol: ty.symbol.clone(),
        superclass: ty.superclass.clone(),
        interfaces: ty.interfaces.clone(),
        methods: ty.methods.iter().map(callable).collect(),
        constructors: ty.constructors.iter().map(callable).collect(),
        fields: ty
            .fields
            .iter()
            .map(|f| Field {
                symbol: f.symbol.clone(),
                kind: match f.is_enum_value {
                    true => ReferencedType::EnumValue,
                    false => ReferencedType::Field,
                },
                owner: ty.symbol.clone(),
                key: f.key.clone(),
                ty: f.ty.clone(),
                is_static: f.is_static,
            })
            .collect(),
    }
}

pub fn to_fact(file: usize, r: &Relationship) -> RelationshipFact {
    RelationshipFact {
        file,
        field: r.field.clone(),
        to: r.to.clone(),
        child_name: r.child_name.clone(),
        controlling: r.controlling.clone(),
    }
}

pub fn relationship(fact: &RelationshipFact) -> Relationship {
    Relationship {
        field: fact.field.clone(),
        to: fact.to.clone(),
        child_name: fact.child_name.clone(),
        controlling: fact.controlling.clone(),
    }
}

/// A blob id as the manifest keeps it: the bytes that its hexadecimal
/// digits spell.
pub fn blob(id: &str) -> Vec<u8> {
    let digit = |at: usize| u8::from_str_radix(&id[at..at + 2], 16).expect("a blob id is hex");
    (0..id.len()).step_by(2).map(digit).collect()
}

fn hex(blob: &[u8]) -> String {
    blob.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{apex, declare};

    #[test]
    fn a_type_is_what_phase_one_declared_after_the_manifest_has_kept_it() {
        let source = "public class Outer extends Base implements B, A, B {\n\
                      Integer n, m; static final String S = 'x';\n\
                      public Outer() {} public Outer(Integer i) {}\n\
                      void b() {} Integer A() { return 1; } void b(String s) {}\n\
                      class Inner { List<Map<String, Decimal>> table; }\n\
                      enum State { ON, OFF }\n\
                      interface Shape { Decimal area(); }\n}\n";
        let tree = apex::parse(&mut apex::parser(), source.as_bytes());
        let declared = declare::declarations(&tree, source.as_bytes(), "pkg", "pkg/Outer.cls");
        let symbols: Vec<&str> = declared.types.iter().map(|t| t.symbol.as_str()).collect();
        assert_eq!(
            symbols,
            ["Outer", "Outer.Inner", "Outer.State", "Outer.Shape"]
        );

        for (at, ty) in declared.types.iter().enumerate() {
            let kept = to_manifest(at, ty);
            assert_eq!(from_manifest(&kept), *ty);
        }
    }

    #[test]
    fn a_blob_id_is_kept_as_the_bytes_its_digits_spell() {
        let id = "0123456789abcdef00ff0123456789abcdef00ff";

        assert_eq!(blob(id).len(), 20);
        assert_eq!(hex(&blob(id)), id);
    }

    #[test]
    fn each_file_has_the_range_of_a_list_sorted_by_file_that_is_its_own() {
        assert_eq!(
            ranges(4, [0, 0, 2, 2, 2, 3].into_iter()),
            [0..2, 0..0, 2..5, 5..6]
        );
    }
}
