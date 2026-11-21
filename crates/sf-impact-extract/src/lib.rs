//! The library behind `sf-impact-extract`: reads the tree a Salesforce DX project
//! keeps on disk into a refs pack (`docs/pack.md`), whose rows are those of
//! `refs.jsonl`, one declaration or reference each. The rows are defined by
//! `schema/refs.schema.json`.

mod apex;
mod blob;
pub mod declarative;
mod declare;
mod index;
mod metadata;
mod platform;
mod refs;
mod reuse;
mod textual;
mod tree;

use declarative::{Catalog, FileKind, Kind, Metadata, Relationship};
use declare::TypeDecl;
pub use metadata::{bundle, flow_name};
use rayon::prelude::*;
use reuse::Previous;
use sf_impact_common::config::{CONFIG_FILE, Config};
use sf_impact_common::diagnostic::{said, shown};
use sf_impact_common::files;
use sf_impact_common::pack::{self, FileEntry, FileRows, Manifest, Pack, RelationshipFact};
use sf_impact_common::provenance::{self, Writer};
use sf_impact_common::rows::{
    Declaration, ParseError, RefKind, Reference, Row, ScopeSource, SkipReason, SkippedFile, Tier,
    UnmodelledFile, UnsupportedElement,
};
use sf_impact_common::scope::{PROJECT_FILE, Scope, ScopeFile};
use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::{fs, io};
use tree_sitter::{Parser, Tree};

/// The walk recurses once per level of syntactic nesting, and a long chain
/// of string concatenations nests once per operand.
const WORKER_STACK_BYTES: usize = 64 * 1024 * 1024;

/// How many levels an Apex file's syntax tree may nest, well within what
/// `WORKER_STACK_BYTES` holds. A file nested deeper is not walked at all.
const APEX_DEPTH: usize = 4_000;

/// How deeply the elements of a metadata file may nest. An element nested
/// deeper is a syntax error where it starts, and the file is read up to it.
const XML_DEPTH: usize = 1_000;

/// A modelled file after phase one: kept in memory so that phase two
/// resolves references without reading or parsing the file again.
struct Parsed {
    /// The content as text, or as it is on disk if it is not text.
    source: Result<Vec<u8>, Vec<u8>>,
    syntax: Syntax,
    /// For each declaration of an Apex file, whether it has its first and
    /// last lines to itself. Empty for any other file.
    alone: Vec<bool>,
}

enum Syntax {
    Apex(Tree, Vec<Declaration>),
    /// An object, a field, a permission set and the like.
    Metadata(Box<Metadata>),
    /// A Flow, or a file of a Lightning web component bundle.
    FlowOrBundle(metadata::Parsed, Vec<Declaration>),
    /// An Apex file whose syntax nests more than `APEX_DEPTH` levels, and
    /// the line of the first node past that. It declares and references
    /// nothing; its text is still searched.
    TooDeep(u32),
}

impl Parsed {
    fn declarations(&self) -> &[Declaration] {
        match &self.syntax {
            Syntax::Apex(_, declarations) | Syntax::FlowOrBundle(_, declarations) => declarations,
            Syntax::Metadata(metadata) => &metadata.declarations,
            Syntax::TooDeep(_) => &[],
        }
    }

    /// The line of the first error, if the file could not be parsed
    /// cleanly. What was recovered is still read.
    fn parse_error(&self) -> Option<u32> {
        match &self.syntax {
            Syntax::Apex(tree, _) => apex::errors(tree),
            Syntax::TooDeep(line) => Some(*line),
            // XML stops at its first error. What came before it is read.
            Syntax::Metadata(metadata) => metadata.error_line(),
            // A scanner that could not read part of the file says where.
            Syntax::FlowOrBundle(found, _) => {
                let bytes = self.source.as_ref().unwrap_or_else(|bytes| bytes);
                let first = found.errors.iter().min()?;
                Some(metadata::position(bytes, *first).0)
            }
        }
    }
}

/// Which reader a file is for, going by its path.
#[derive(Clone, Copy)]
enum Model {
    Apex,
    FlowOrBundle,
    Metadata(Kind),
}

/// `None` for a file that phase one does not read: it declares nothing.
fn model(path: &str) -> Option<Model> {
    if is_apex(path) {
        return Some(Model::Apex);
    }
    if metadata::flow_name(path).is_some() || metadata::bundle(path).is_some() {
        return Some(Model::FlowOrBundle);
    }
    match declarative::file_kind(path) {
        FileKind::Modelled(kind) => Some(Model::Metadata(kind)),
        FileKind::Unmodelled(_) | FileKind::Other => None,
    }
}

/// What stands between the name and `-meta.xml` in the name of a metadata
/// file of a type that is not modelled: the file has an `unmodelled_file`
/// row and nothing else is read from it.
pub fn unmodelled(path: &str) -> Option<String> {
    match (model(path), declarative::file_kind(path)) {
        (None, FileKind::Unmodelled(suffix)) => Some(suffix),
        _ => None,
    }
}

/// Whether a file lists what it names entry by entry: a permission set, a
/// profile or a custom metadata record.
pub fn has_entries(path: &str) -> bool {
    entries_kind(path).is_some()
}

fn entries_kind(path: &str) -> Option<Kind> {
    match model(path)? {
        Model::Metadata(kind @ (Kind::PermissionSet | Kind::Profile | Kind::Record)) => Some(kind),
        _ => None,
    }
}

/// One version of a file that [`has_entries`], read from its content alone.
/// `None` for any other file.
pub fn read_entries(file: &ScopeFile, bytes: Vec<u8>) -> Option<Metadata> {
    let kind = entries_kind(&file.path)?;
    let source = textual::searchable(bytes);
    let bytes = source.as_ref().unwrap_or_else(|bytes| bytes);
    Some(declarative::read(kind, bytes, &file.package, &file.path))
}

/// Phase one for one file: its content parsed, and what it declares.
/// `source` is the content as text, or as it is if it is not text. A bundle
/// is declared in one of its files, and `is_anchor` says that this is it.
fn parse(
    model: Model,
    parser: &mut Parser,
    file: &ScopeFile,
    source: Result<Vec<u8>, Vec<u8>>,
    is_anchor: bool,
) -> (Parsed, Vec<TypeDecl>) {
    let bytes = source.as_ref().unwrap_or_else(|bytes| bytes);
    let mut alone = Vec::new();
    let (syntax, types) = match model {
        Model::Apex => {
            let tree = apex::parse(parser, bytes);
            match apex::too_deep(&tree, APEX_DEPTH) {
                Some(line) => (Syntax::TooDeep(line), Vec::new()),
                None => {
                    let found = declare::declarations(&tree, bytes, &file.package, &file.path);
                    alone = found.alone;
                    (Syntax::Apex(tree, found.rows), found.types)
                }
            }
        }
        Model::FlowOrBundle => {
            let declarations = metadata::declaration(file, bytes, is_anchor)
                .into_iter()
                .collect();
            let syntax = Syntax::FlowOrBundle(metadata::parse(file, bytes), declarations);
            (syntax, Vec::new())
        }
        Model::Metadata(kind) => {
            let metadata = declarative::read(kind, bytes, &file.package, &file.path);
            (Syntax::Metadata(Box::new(metadata)), Vec::new())
        }
    };
    let parsed = Parsed {
        source,
        syntax,
        alone,
    };
    (parsed, types)
}

/// Whether phase one reads a file, going by its path: whether it can
/// declare anything.
pub fn reads(path: &str) -> bool {
    model(path).is_some()
}

/// What one version of one file declares.
pub struct OneFile {
    /// In source order. `collision` is not known without the tree, and is
    /// false.
    pub declarations: Vec<Declaration>,
    /// For each declaration: whether nothing else is on its first and last
    /// lines, so that a changed line it starts or ends on can only be a
    /// change to it. Known of Apex alone; false of everything else.
    pub alone: Vec<bool>,
    /// The file could not be parsed cleanly: `declarations` are what was
    /// recovered and may be incomplete.
    pub parse_error: bool,
}

/// Phase one for a file whose text is all there is: one version of it
/// out of version control, without the tree around it. `None` for a file
/// that phase one does not read. A file of a bundle declares nothing here:
/// a bundle is declared in one file that is chosen among all of its files.
pub fn read_one(file: &ScopeFile, text: &str) -> Option<OneFile> {
    let source = Ok(text.as_bytes().to_vec());
    let (parsed, _) = parse(model(&file.path)?, &mut apex::parser(), file, source, false);
    let declarations = parsed.declarations().to_vec();
    let alone = (0..declarations.len()).map(|at| parsed.alone.get(at) == Some(&true));
    Some(OneFile {
        parse_error: parsed.parse_error().is_some(),
        alone: alone.collect(),
        declarations,
    })
}

/// The lines of `refs.jsonl` for the project at `root`, in output order:
/// what the file is of, scope errors, then the rows of each file in path
/// order. They are the projection of the pack that [`extract_kept`] gives.
/// `trust_repo` says to ask git which commit the tree is though another
/// user owns the repository.
pub fn extract(root: &Path, jobs: usize, trust_repo: bool) -> Result<Vec<String>, String> {
    let (pack, _kept) = extract_kept(root, jobs, trust_repo)?;
    Ok(Pack::decode(&pack)?.to_jsonl())
}

/// What [`extract_kept`] built to resolve references, handed back instead
/// of freed: the index of types, the catalog and the names searched for.
pub struct Kept {
    _index: index::Index,
    _catalog: Catalog,
    _names: textual::Names,
}

/// The refs pack of the project at `root`, handing back what was built to
/// make it rather than freeing it, so that a process about to exit need not.
pub fn extract_kept(root: &Path, jobs: usize, trust_repo: bool) -> Result<(Vec<u8>, Kept), String> {
    extract_reusing(root, jobs, trust_repo, None)
}

/// [`extract_kept`], reusing the rows of `previous`, the pack of an
/// earlier run, for every file whose rows would be the same. The pack is
/// the one that a run without `previous` writes, byte for byte. A previous
/// pack that cannot be reused, whatever is wrong with it, is not an error:
/// why is said on standard error, and the run is a run without it.
pub fn extract_reusing(
    root: &Path,
    jobs: usize,
    trust_repo: bool,
    previous: Option<&[u8]>,
) -> Result<(Vec<u8>, Kept), String> {
    extract_reading(root, jobs, trust_repo, previous, &|path| fs::read(path))
}

/// How the content of a file is had: by `fs::read`, unless a test has to
/// say what is read, and see how often.
type ReadFile<'a> = &'a (dyn Fn(&Path) -> io::Result<Vec<u8>> + Sync);

/// [`extract_reusing`], reading every file with `read_file`.
fn extract_reading(
    root: &Path,
    jobs: usize,
    trust_repo: bool,
    previous: Option<&[u8]>,
    read_file: ReadFile,
) -> Result<(Vec<u8>, Kept), String> {
    // What is checked out, asked of git before a file is read and never
    // after: what is read is held to it by the bytes that were read.
    let checkout = tree::Checkout::of(root, trust_repo);
    let mut disk = Disk {
        root,
        format: checkout.as_ref().ok().map(|checkout| checkout.format),
        read_file,
        read_first: BTreeMap::new(),
    };
    let said_of = |file: &str, e: io::Error| format!("{}: {e}", root.join(file).display());
    // Each only if it is a regular file of its own, not a link out of the
    // root or to a device, and of a size that a config file is.
    let root_file = |file: &str| {
        let regular = files::regular(&root.join(file), files::MAX_CONFIG_BYTES);
        regular.and_then(|_| disk.bytes(file))
    };
    let project = root_file(PROJECT_FILE);
    let project = project.map_err(|e| said_of(PROJECT_FILE, e))?;
    let scope = Scope::of(root, &project)?;
    let config_file = match root_file(CONFIG_FILE) {
        Ok(bytes) => Some(bytes),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(said_of(CONFIG_FILE, e)),
    };
    let config = Config::of(root, config_file.as_deref())?;
    // A package directory can be the root, and these two are then files
    // in scope. They are not read again there: what is searched and what
    // is held to the commit is what the scope and the config were read of.
    disk.read_first.insert(PROJECT_FILE, project.clone());
    if let Some(config) = &config_file {
        disk.read_first.insert(CONFIG_FILE, config.clone());
    }
    let disk = disk;
    let files = scope
        .files(root)
        .map_err(|e| format!("{}: {e}", root.display()))?;
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(jobs)
        .stack_size(WORKER_STACK_BYTES)
        .build()
        .map_err(|e| e.to_string())?;
    // What was read, for whatever reads the file to hold it to.
    let declared = provenance::Scope::declared(&project).map_err(|e| {
        format!(
            "{}: {}",
            root.join(PROJECT_FILE).display(),
            shown(&e.to_string())
        )
    })?;
    let writer = Writer::of(&provenance::REFS, env!("CARGO_PKG_VERSION"));
    let config_hash = provenance::config_hash(&config);

    let anchors = metadata::bundle_declarations(&files);
    let run = Run {
        disk: &disk,
        files: &files,
        scope: &scope,
        config: &config,
        anchors: &anchors,
    };
    let previous = previous.map(|bytes| match disk.format {
        Some(format) => {
            let identity = reuse::Identity {
                writer: writer.clone(),
                scope: &declared,
                config: &config_hash,
                format,
            };
            reuse::Previous::of(bytes, &identity)
        }
        None => Err("no commit could be read, so files have no blob ids".into()),
    });
    let made = match &previous {
        Some(Ok(previous)) => Some(pool.install(|| run.again(previous))?),
        Some(Err(why)) => {
            eprintln!(
                "sf-impact-extract: note: the previous pack was not reused: {}",
                said(why)
            );
            None
        }
        None => None,
    };
    let Made {
        per_file,
        types,
        relationships,
        kept,
    } = match made {
        Some(made) => made,
        None => pool.install(|| run.cold())?,
    };
    drop(previous);

    let mut scope_errors: Vec<(String, _)> = scope
        .errors
        .into_iter()
        .map(|error| (to_line(&Row::ScopeError(error.clone())), error))
        .collect();
    scope_errors.sort_by(|a, b| a.0.cmp(&b.0));
    for (error, _) in &scope_errors {
        eprintln!(
            "sf-impact-extract: warning: a declared directory was not read: {}",
            said(error)
        );
    }

    // What was read, for whatever reads the file to hold it to: the
    // commit, if the bytes that the rows are made of are the commit's.
    // Every file in scope is read in a run, so where files have ids, every
    // one has an entry in the manifest.
    let (per_file, ids): (Vec<Option<FileRows>>, Vec<Id>) = per_file.into_iter().unzip();
    let (tree, at, note, manifest) = match checkout {
        Ok(checkout) => {
            let id = |bytes: &[u8]| checkout.format.blob(bytes);
            let mut read = BTreeMap::new();
            record(&mut read, PROJECT_FILE, id(&project))?;
            if let Some(config) = &config_file {
                record(&mut read, CONFIG_FILE, id(config))?;
            }
            let mut entries = Vec::with_capacity(files.len());
            for (file, id) in files.iter().zip(ids) {
                if let Some(id) = id {
                    entries.push(FileEntry {
                        package: file.package.clone(),
                        path: file.path.clone(),
                        blob: reuse::blob(&id),
                    });
                    record(&mut read, &file.path, id)?;
                }
            }
            let at = checkout.root.clone();
            let (tree, note) = checkout.read_as(&declared, &read);
            let manifest = Manifest {
                files: entries,
                types,
                relationships,
            };
            (tree, Some(at), note, Some(manifest))
        }
        Err(why) => (
            provenance::Tree::Unknown,
            None,
            Some(tree::unknown(&why)),
            None,
        ),
    };
    if let Some(note) = note {
        eprintln!("sf-impact-extract: note: {}", said(&note));
    }
    let provenance = provenance::line(provenance::Refs {
        writer,
        tree,
        root: at,
        scope: declared,
        config: config_hash,
    });
    let mut pack = pack::Writer::new(scope_errors.iter().map(|(_, error)| error));
    for rows in per_file.into_iter().flatten() {
        pack.push(rows)?;
    }
    Ok((pack.finish(&provenance, manifest.as_ref())?, kept))
}

/// What a run reads and resolves against, apart from the files' content.
struct Run<'a> {
    disk: &'a Disk<'a>,
    files: &'a [ScopeFile],
    scope: &'a Scope,
    config: &'a Config,
    /// The files that a bundle is declared in.
    anchors: &'a HashSet<String>,
}

/// One file in scope after phase one.
enum Seen {
    /// Read and parsed in this run, if phase one reads it: what it declares,
    /// and the id of its bytes if they were read here. A file that phase
    /// one does not read is read in phase two.
    Changed {
        parsed: Option<Parsed>,
        types: Vec<TypeDecl>,
        id: Id,
    },
    /// The same as when the previous pack was written: its entry there,
    /// and the id of its bytes, which are the bytes its rows were made of.
    Unchanged { entry: usize, id: String },
    /// The same as when the previous pack was written, in a run in which
    /// what another file declares has changed: what it declares is its
    /// entry's, and its rows are made again in phase two, of `bytes` if
    /// phase one reads it, since the pack keeps no syntax tree.
    Declared {
        entry: usize,
        id: String,
        bytes: Option<Vec<u8>>,
    },
}

/// What the files in scope make: each one's rows, encoded, and the id of
/// its bytes; what the manifest keeps of what they declare, by file; and
/// what resolved them.
struct Made {
    per_file: Vec<(Option<FileRows>, Id)>,
    types: Vec<pack::TypeDecl>,
    relationships: Vec<RelationshipFact>,
    kept: Kept,
}

impl Run<'_> {
    /// Every file read and resolved.
    fn cold(&self) -> Result<Made, String> {
        // Phase one: parse every Apex and metadata file, Flows and the
        // files of LWC bundles included, and collect what it declares.
        let seen = self
            .files
            .par_iter()
            .with_max_len(1)
            .map_init(apex::parser, |parser, file| {
                let Some(model) = model(&file.path) else {
                    let types = Vec::new();
                    return Ok(Seen::Changed {
                        parsed: None,
                        types,
                        id: None,
                    });
                };
                let is_anchor = self.anchors.contains(&file.path);
                let (id, source) = self.disk.read(file)?;
                let (parsed, types) = parse(model, parser, file, source, is_anchor);
                Ok(Seen::Changed {
                    parsed: Some(parsed),
                    types,
                    id,
                })
            })
            .collect::<Result<_, String>>()?;
        self.make(seen, None)
    }

    /// Every file read, to know which have changed since `previous` was
    /// written, and only those parsed. If what every file declares is what
    /// it declared then, the rows of every other file are the previous
    /// pack's. If not, what the files that did not change declare is still
    /// the previous pack's, and phase two makes the rows of every file.
    fn again(&self, previous: &Previous) -> Result<Made, String> {
        let (seen, kept): (Vec<Seen>, Vec<Option<Vec<u8>>>) = self
            .files
            .par_iter()
            .with_max_len(1)
            .map_init(apex::parser, |parser, file| {
                let is_anchor = self.anchors.contains(&file.path);
                let (id, bytes) = self.disk.read_bytes(file)?;
                let id = id.expect("a previous pack is read only where files have ids");
                let model = model(&file.path);
                if let Some(entry) = previous.unchanged(file, &id, is_anchor) {
                    // Kept in case phase two has to parse it after all.
                    let bytes = model.is_some().then_some(bytes);
                    return Ok((Seen::Unchanged { entry, id }, bytes));
                }
                // Read again in phase two, as a run without a previous
                // pack reads it: its id is then that of what it is read as.
                let Some(model) = model else {
                    let types = Vec::new();
                    let seen = Seen::Changed {
                        parsed: None,
                        types,
                        id: None,
                    };
                    return Ok((seen, None));
                };
                let source = textual::searchable(bytes);
                let (parsed, types) = parse(model, parser, file, source, is_anchor);
                let seen = Seen::Changed {
                    parsed: Some(parsed),
                    types,
                    id: Some(id),
                };
                Ok((seen, None))
            })
            .collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .unzip();

        // Whether every file that changed, is new or is gone declares what
        // it did. Every other file does.
        let mut gone = vec![true; previous.entries()];
        let mut changed = 0;
        let mut why = None;
        for (file, seen) in self.files.iter().zip(&seen) {
            let entry = previous.entry(file);
            if let Some(entry) = entry {
                gone[entry] = false;
            }
            let Seen::Changed { parsed, types, .. } = seen else {
                continue;
            };
            changed += 1;
            if why.is_some() {
                continue;
            }
            let declarations: Vec<&Declaration> =
                parsed.iter().flat_map(Parsed::declarations).collect();
            let relationships: Vec<Relationship> =
                parsed.iter().filter_map(relationship).cloned().collect();
            let then = entry.map(|entry| (previous, entry));
            if !reuse::declares_the_same(&declarations, types, &relationships, then) {
                why = Some(format!(
                    "{} declares other things than it did",
                    shown(&file.path)
                ));
            }
        }
        let mut gone = (0..gone.len()).filter(|entry| gone[*entry]);
        if why.is_none()
            && let Some(entry) = gone.find(|entry| !reuse::declared_nothing(previous, *entry))
        {
            why = Some(format!(
                "{}, which declared things, is gone",
                shown(previous.path(entry))
            ));
        }
        let files = self.files.len();
        let seen = match why {
            None => {
                eprintln!(
                    "sf-impact-extract: note: the previous pack was reused: {changed} of \
                     {files} files in scope changed"
                );
                drop(kept);
                seen
            }
            Some(why) => {
                eprintln!(
                    "sf-impact-extract: note: the previous pack was reused in part: {}, so \
                     the rows of every file are made again; {changed} of {files} files in scope \
                     changed, and what the others declare is the previous pack's",
                    said(&why)
                );
                let declared = |(seen, bytes)| match seen {
                    Seen::Unchanged { entry, id } => Seen::Declared { entry, id, bytes },
                    seen => seen,
                };
                seen.into_iter().zip(kept).map(declared).collect()
            }
        };
        self.make(seen, Some(previous))
    }

    /// What every file declares, made into what resolves references, and
    /// then the rows of every file: resolved now if it changed, as the
    /// previous pack has them if it did not.
    fn make(&self, mut seen: Vec<Seen>, previous: Option<&Previous>) -> Result<Made, String> {
        let previous_of = |entry: &usize| {
            let previous = previous.expect("only a previous pack has unchanged files");
            (previous, *entry)
        };
        // What the manifest keeps, before the index takes the types.
        let (mut types, mut relationships) = (Vec::new(), Vec::new());
        for (at, seen) in seen.iter().enumerate() {
            match seen {
                Seen::Changed {
                    parsed,
                    types: declared,
                    ..
                } => {
                    types.extend(declared.iter().map(|ty| reuse::to_manifest(at, ty)));
                    let found = parsed.as_ref().and_then(relationship);
                    relationships.extend(found.map(|r| reuse::to_fact(at, r)));
                }
                Seen::Unchanged { entry, .. } | Seen::Declared { entry, .. } => {
                    let (previous, entry) = previous_of(entry);
                    let file = |ty: &pack::TypeDecl| pack::TypeDecl {
                        file: at,
                        ..ty.clone()
                    };
                    types.extend(previous.types(entry).iter().map(file));
                    let file = |r: &RelationshipFact| RelationshipFact {
                        file: at,
                        ..r.clone()
                    };
                    relationships.extend(previous.relationships(entry).iter().map(file));
                }
            }
        }

        let mut index_types = Vec::new();
        for seen in &mut seen {
            match seen {
                Seen::Changed { types, .. } => index_types.append(types),
                Seen::Unchanged { entry, .. } | Seen::Declared { entry, .. } => {
                    let (previous, entry) = previous_of(entry);
                    let kept = previous.types(entry).iter().map(reuse::from_manifest);
                    index_types.extend(kept);
                }
            }
        }
        let (index, catalog, names) = {
            // Neither the index, the catalog nor the names depend on the
            // order of the declarations: the rows of a file that did not
            // change have them in position order, phase one in source order.
            let mut declarations: Vec<&Declaration> = Vec::new();
            for seen in &seen {
                match seen {
                    Seen::Changed { parsed, .. } => {
                        declarations.extend(parsed.iter().flat_map(Parsed::declarations));
                    }
                    Seen::Unchanged { entry, .. } | Seen::Declared { entry, .. } => {
                        let (previous, entry) = previous_of(entry);
                        declarations.extend(previous.declarations(entry));
                    }
                }
            }
            let namespaces = &self.config.namespaces;
            let index = index::Index::build(index_types, declarations.iter().copied(), namespaces);
            let found = relationships
                .iter()
                .map(reuse::relationship)
                .collect::<Vec<_>>();
            let catalog = Catalog::build(declarations.iter().copied(), &found, namespaces);
            let names = declarations.iter().map(|d| d.name.as_str());
            let names = textual::Names::new(names, self.config);
            (index, catalog, names)
        };

        // Phase two: every file in scope, now that every name is known.
        // A file is read once: here, if it was not read to be parsed.
        let rows_of = |file: &ScopeFile, parsed: Option<Parsed>| {
            let source = self.scope.sources[&file.package];
            let (rows, read_here) =
                file_rows(self.disk, file, parsed, &index, &catalog, &names, source)?;
            // Encoded here, in parallel: the rows of every file are not
            // all held at once, only what the pack makes of them.
            let rows = (!rows.is_empty())
                .then(|| FileRows::of(&rows))
                .transpose()?;
            Ok::<_, String>((rows, read_here))
        };
        let per_file = self
            .files
            .par_iter()
            .zip(seen)
            .with_max_len(1)
            .map_init(apex::parser, |parser, (file, seen)| match seen {
                Seen::Changed { parsed, id, .. } => {
                    let (rows, read_here) = rows_of(file, parsed)?;
                    Ok((rows, id.or(read_here)))
                }
                Seen::Declared { id, bytes, .. } => {
                    // Parsed now, as phase one would have parsed it, for
                    // its syntax: what it declares is known already.
                    let parsed = bytes.map(|bytes| {
                        let model = model(&file.path).expect("only what phase one reads is kept");
                        let is_anchor = self.anchors.contains(&file.path);
                        let source = textual::searchable(bytes);
                        parse(model, parser, file, source, is_anchor).0
                    });
                    let (rows, read_here) = rows_of(file, parsed)?;
                    Ok((rows, read_here.or(Some(id))))
                }
                Seen::Unchanged { entry, id } => {
                    let rows = previous_of(&entry).0.rows(entry);
                    let rows = (!rows.is_empty()).then(|| FileRows::of(rows));
                    Ok((rows.transpose()?, Some(id)))
                }
            })
            .collect::<Result<_, String>>()?;
        let kept = Kept {
            _index: index,
            _catalog: catalog,
            _names: names,
        };
        Ok(Made {
            per_file,
            types,
            relationships,
            kept,
        })
    }
}

/// The relationship that a metadata file declares, if it declares one.
fn relationship(parsed: &Parsed) -> Option<&Relationship> {
    match &parsed.syntax {
        Syntax::Metadata(metadata) => metadata.relationship.as_ref(),
        Syntax::Apex(..) | Syntax::FlowOrBundle(..) | Syntax::TooDeep(_) => None,
    }
}

/// Notes the id of the bytes that were read of a file. A file has one:
/// an id never takes the place of an earlier one, and a file that was read
/// as other bytes a second time is an error, since neither id would stand
/// for all that the rows are made of.
fn record(read: &mut BTreeMap<String, String>, path: &str, id: String) -> Result<(), String> {
    match read.get(path) {
        Some(earlier) if *earlier != id => Err(format!(
            "{} was read more than once and was not the same bytes each time: the rows \
             cannot be held to any one content of it",
            shown(path)
        )),
        Some(_) => Ok(()),
        None => {
            read.insert(path.to_string(), id);
            Ok(())
        }
    }
}

/// The id of the bytes that were read of a file, where a commit is known
/// to hold them to.
type Id = Option<String>;

/// The content of a file: as text if it is text, as it is otherwise.
type Source = Result<Vec<u8>, Vec<u8>>;

/// The project on disk, and how the repository it is in names its
/// objects, if it is in one that could be read.
struct Disk<'a> {
    root: &'a Path,
    format: Option<blob::Format>,
    read_file: ReadFile<'a>,
    /// The bytes of the project file and of the config, by path from the
    /// root, as they were read before any other file: neither is read a
    /// second time.
    read_first: BTreeMap<&'static str, Vec<u8>>,
}

impl Disk<'_> {
    /// The bytes of the file at a path from the root.
    fn bytes(&self, path: &str) -> io::Result<Vec<u8>> {
        (self.read_file)(&self.root.join(path))
    }

    /// The content of a file, and the id of the bytes that were read of
    /// it, which are the bytes that the rows of the file are made of. Each
    /// file is read once in a run: one that was read first is not read here.
    fn read(&self, file: &ScopeFile) -> Result<(Id, Source), String> {
        let (id, bytes) = self.read_bytes(file)?;
        Ok((id, textual::searchable(bytes)))
    }

    /// [`Disk::read`], with the bytes as they are.
    fn read_bytes(&self, file: &ScopeFile) -> Result<(Id, Vec<u8>), String> {
        let bytes = match self.read_first.get(file.path.as_str()) {
            Some(read) => read.clone(),
            None => {
                let bytes = self.bytes(&file.path);
                bytes.map_err(|e| format!("{}: {e}", shown(&file.path)))?
            }
        };
        let id = self.format.map(|format| format.blob(&bytes));
        Ok((id, bytes))
    }
}

/// The rows of one file, sorted: its parse error, its skipped-file row and
/// its unmodelled-file row if any, its declarations, its references by
/// position, then the elements of it that were not read. With them, the id
/// of the bytes of the file, if it is read here: a file that was not parsed.
fn file_rows(
    disk: &Disk,
    file: &ScopeFile,
    parsed: Option<Parsed>,
    index: &index::Index,
    catalog: &Catalog,
    names: &textual::Names,
    scope: ScopeSource,
) -> Result<(Vec<Row>, Id), String> {
    let mut rows = Vec::new();
    let mut read_here = None;
    if let Some(line) = parsed.as_ref().and_then(Parsed::parse_error) {
        rows.push(Row::ParseError(ParseError {
            package: file.package.clone(),
            file: file.path.clone(),
            line,
        }));
    }
    let (source, declarations) = match parsed {
        Some(Parsed {
            source,
            syntax: Syntax::Apex(tree, declarations),
            ..
        }) => {
            // A tree with errors is still walked: what the grammar recovered
            // is worth more than nothing, and the parse error row says so.
            let bytes = source.as_ref().unwrap_or_else(|bytes| bytes);
            let references =
                refs::references(&tree, bytes, index, catalog, &file.package, &file.path);
            rows.extend(references.into_iter().map(Row::Reference));
            (source, declarations)
        }
        Some(Parsed {
            source,
            syntax: Syntax::TooDeep(_),
            ..
        }) => (source, Vec::new()),
        Some(Parsed {
            source,
            syntax: Syntax::Metadata(metadata),
            ..
        }) => {
            rows.extend(declarative::references(
                &metadata,
                catalog,
                &file.package,
                &file.path,
            ));
            (source, metadata.declarations)
        }
        Some(Parsed {
            source,
            syntax: Syntax::FlowOrBundle(found, declarations),
            ..
        }) => {
            // What a scanner could read is resolved, errors or not.
            let bytes = source.as_ref().unwrap_or_else(|bytes| bytes);
            // A structure the Flow reader does not know is not a syntax
            // error: it is an element that was not read.
            for (element, offset, reason) in &found.unsupported {
                let (line, column) = metadata::position(bytes, *offset);
                rows.push(Row::UnsupportedElement(UnsupportedElement {
                    package: file.package.clone(),
                    file: file.path.clone(),
                    element: element.clone(),
                    reason: *reason,
                    line,
                    column,
                }));
            }
            let references = metadata::resolve(&found, bytes, file, index, catalog);
            rows.extend(references.into_iter().map(Row::Reference));
            (source, declarations)
        }
        None => {
            if let Some(suffix) = unmodelled(&file.path) {
                rows.push(Row::UnmodelledFile(UnmodelledFile {
                    package: file.package.clone(),
                    file: file.path.clone(),
                    suffix,
                }));
            }
            let (id, source) = disk.read(file)?;
            read_here = id;
            (source, Vec::new())
        }
    };

    // What is not text was not searched, and the output has to say so.
    let source = source.unwrap_or_else(|_| {
        rows.push(Row::SkippedFile(SkippedFile {
            package: file.package.clone(),
            file: file.path.clone(),
            reason: SkipReason::Binary,
        }));
        Vec::new()
    });
    let matches = textual::matches(&source, names);
    let line_enclosing = (declarations.len() > 8 && !matches.is_empty()).then(|| {
        let line_count = source.iter().filter(|&&byte| byte == b'\n').count() + 1;
        enclosing_table(&declarations, line_count)
    });
    for found in matches {
        let declaration = match &line_enclosing {
            Some(table) => found
                .line
                .checked_sub(1)
                .and_then(|line| table.get(line as usize))
                .and_then(|&entry| entry)
                .map(|index| &declarations[index]),
            None => enclosing(&declarations, found.line),
        };
        rows.push(Row::Reference(Reference {
            tier: Tier::Textual,
            kind: RefKind::Text,
            referenced: found.word,
            referenced_type: None,
            resolution: None,
            detail: None,
            candidates: Vec::new(),
            referrer: declaration
                .map(|d| d.symbol.clone())
                .or_else(|| metadata::bundle(&file.path).map(|(_, name)| name.to_string())),
            source_field: None,
            package: file.package.clone(),
            file: file.path.clone(),
            line: found.line,
            column: found.column,
        }));
    }
    rows.extend(declarations.into_iter().map(|mut declaration| {
        declaration.collision = index.collides(&declaration);
        declaration.scope = Some(scope);
        Row::Declaration(declaration)
    }));

    // The serialised row breaks a tie of position only, so only a tie
    // serialises its rows.
    rows.sort_by(|a, b| {
        let by_text = || to_line(a).cmp(&to_line(b));
        position(a).cmp(&position(b)).then_with(by_text)
    });
    Ok((rows, read_here))
}

/// The innermost declaration whose lines include `line`.
fn enclosing(declarations: &[Declaration], line: u32) -> Option<&Declaration> {
    declarations
        .iter()
        .filter(|d| d.start <= line && line <= d.end)
        .min_by_key(|d| d.end - d.start)
}

/// One entry per source line, with the same first-wins ties as `enclosing`.
/// Declaration endpoints never determine the allocation size.
fn enclosing_table(declarations: &[Declaration], line_count: usize) -> Vec<Option<usize>> {
    let mut table = vec![None; line_count];
    for (index, declaration) in declarations.iter().enumerate() {
        if declaration.start > declaration.end {
            continue;
        }
        let span = declaration.end - declaration.start;
        let start = declaration.start.saturating_sub(1) as usize;
        let end = (declaration.end as usize).min(line_count);
        for entry in table.iter_mut().take(end).skip(start) {
            if entry
                .is_none_or(|best: usize| span < declarations[best].end - declarations[best].start)
            {
                *entry = Some(index);
            }
        }
    }
    table
}

/// Sort key within a file: row kind, then line and column. The serialised
/// row breaks ties, which makes the order total.
fn position(row: &Row) -> (u8, u32, u32) {
    match row {
        Row::ScopeError(_) => (0, 0, 0),
        Row::ParseError(error) => (1, error.line, 0),
        Row::SkippedFile(_) => (2, 0, 0),
        Row::UnmodelledFile(_) => (3, 0, 0),
        Row::Declaration(declaration) => (4, declaration.start, 0),
        Row::Reference(reference) => (5, reference.line, reference.column),
        Row::UnsupportedElement(element) => (6, element.line, element.column),
    }
}

fn to_line(row: &Row) -> String {
    serde_json::to_string(row).expect("a row has only string keys and plain values")
}

pub fn is_apex(path: &str) -> bool {
    let extension = path.rsplit('.').next().unwrap_or_default();
    extension.eq_ignore_ascii_case("cls") || extension.eq_ignore_ascii_case("trigger")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Mutex;

    /// A project whose one package directory is its root, so that its
    /// project file and its config are files in scope: the one commit of a
    /// repository in a temporary directory, removed on drop.
    struct AtRoot(PathBuf);

    const AT_ROOT: &str = "{\"packageDirectories\": [{\"path\": \".\"}]}\n";

    impl AtRoot {
        fn committed(name: &str) -> AtRoot {
            let name = format!("sf-impact-extract-at-root-{}-{name}", std::process::id());
            let project = AtRoot(std::env::temp_dir().join(name));
            let _ = fs::remove_dir_all(&project.0);
            fs::create_dir_all(&project.0).unwrap();
            for (file, content) in [
                (PROJECT_FILE, AT_ROOT),
                (CONFIG_FILE, "{}\n"),
                ("Service.cls", "public class Service {\n}\n"),
            ] {
                fs::write(project.0.join(file), content).unwrap();
            }
            for args in [
                &["init", "--quiet", "--initial-branch=main"][..],
                &["add", "--all"],
                &["commit", "--quiet", "--message", "base"],
            ] {
                let mut git = Command::new("git");
                git.current_dir(&project.0)
                    .args(args)
                    .env("GIT_CONFIG_GLOBAL", "/dev/null")
                    .env("GIT_CONFIG_NOSYSTEM", "1");
                for who in ["AUTHOR", "COMMITTER"] {
                    git.env(format!("GIT_{who}_NAME"), "A Test")
                        .env(format!("GIT_{who}_EMAIL"), "test@example.com");
                }
                let output = git.output().expect("git runs");
                assert!(output.status.success(), "git {args:?}: {output:?}");
            }
            project
        }
    }

    impl Drop for AtRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_project_file_or_a_config_that_is_in_scope_is_read_once_and_held_to_the_commit_by_that() {
        // What each file is when it is first read, which the commit does
        // not have, and how the output shows that it is made of that: a
        // row for the directory that only the stale project file declares,
        // and the hash of the stale config. Another process restores the
        // file after: any later read gives what the commit has.
        type Shows = fn(&[String]) -> bool;
        let stale: [(&str, &str, Shows); 2] = [
            (
                PROJECT_FILE,
                "{\"packageDirectories\": [{\"path\": \".\"}, {\"path\": \"../stale\"}]}\n",
                |lines| {
                    lines
                        .iter()
                        .any(|line| line.contains(r#""path":"../stale""#))
                },
            ),
            (
                CONFIG_FILE,
                "{\"namespaces\": [\"stale_namespace\"]}\n",
                |lines| !lines[0].contains(r#""config":"4e7844462778448c""#),
            ),
        ];

        for (file, first_read, is_of_first_read) in stale {
            let project = AtRoot::committed(file);
            let reads = Mutex::new(BTreeMap::new());
            let read_file = |path: &Path| {
                let name = path.strip_prefix(&project.0).unwrap();
                let name = name.to_str().unwrap().to_string();
                let mut reads = reads.lock().unwrap();
                let earlier = reads.entry(name.clone()).or_insert(0);
                *earlier += 1;
                match name == file && *earlier == 1 {
                    true => Ok(first_read.as_bytes().to_vec()),
                    false => fs::read(path),
                }
            };

            let (pack, _) = extract_reading(&project.0, 1, false, None, &read_file).unwrap();
            let lines = Pack::decode(&pack).unwrap().to_jsonl();

            // The output is of what was first read, so the tree is not the
            // commit, whatever the file is by the time it is in scope.
            let first: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
            assert!(is_of_first_read(&lines), "{file}: {first}");
            assert_eq!(first["tree"]["state"], "dirty", "{file}: {first}");
            // And nothing is read a second time to be held to it.
            let reads = reads.into_inner().unwrap();
            let once = [CONFIG_FILE, PROJECT_FILE, "Service.cls"].map(|name| (name.to_string(), 1));
            assert_eq!(reads, BTreeMap::from(once), "{file}");
        }
    }

    #[test]
    fn a_file_is_not_noted_with_another_id_than_it_was_first_read_with() {
        let mut read = BTreeMap::new();

        assert_eq!(record(&mut read, "a/A.cls", "1".into()), Ok(()));
        assert_eq!(record(&mut read, "a/B.cls", "2".into()), Ok(()));
        // The same bytes again say nothing new.
        assert_eq!(record(&mut read, "a/A.cls", "1".into()), Ok(()));
        // Other bytes are not taken for the first: neither id stands for
        // what the rows are made of.
        let refused = record(&mut read, "a/A.cls", "3".into()).unwrap_err();
        assert!(refused.contains("a/A.cls"), "{refused}");
        assert_eq!(read["a/A.cls"], "1");
    }

    #[test]
    fn a_reference_belongs_to_the_innermost_declaration_around_its_line() {
        let declaration = |symbol: &str, start, end| Declaration {
            symbol: symbol.into(),
            name: symbol.into(),
            kind: sf_impact_common::rows::DeclType::Class,
            package: String::new(),
            file: String::new(),
            start,
            end,
            is_test: false,
            annotations: Vec::new(),
            collision: false,
            scope: None,
        };
        let declarations = [declaration("A", 1, 20), declaration("A.m()", 5, 9)];

        let symbol = |line| enclosing(&declarations, line).map(|d| d.symbol.as_str());

        assert_eq!(symbol(7), Some("A.m()"));
        assert_eq!(symbol(12), Some("A"));
        assert_eq!(symbol(21), None);
    }

    fn enclosing_declarations(ranges: &[(u32, u32)]) -> Vec<Declaration> {
        ranges
            .iter()
            .enumerate()
            .map(|(i, &(start, end))| Declaration {
                symbol: i.to_string(),
                name: i.to_string(),
                kind: sf_impact_common::rows::DeclType::Class,
                package: String::new(),
                file: String::new(),
                start,
                end,
                is_test: false,
                annotations: Vec::new(),
                collision: false,
                scope: None,
            })
            .collect()
    }

    #[test]
    fn file_rows_matches_linear_enclosing_at_eight_and_nine_declarations() {
        let file = file("pkg/classes/Outer.cls");
        let disk = Disk {
            root: Path::new("."),
            format: None,
            read_file: &|_| panic!("parsed source must not be read again"),
            read_first: BTreeMap::new(),
        };
        let config = Config::default();
        let names = textual::Names::new(["Needle"], &config);
        let no_names = textual::Names::new([], &config);
        for count in [8, 9] {
            let mut source = String::from("class Outer {\n class Inner {\n");
            for i in 0..count - 2 {
                source.push_str(&format!("  void m{i}() {{ /* Needle */ }}\n"));
            }
            source.push_str(" } // Needle\n} // Needle");
            let parse_source = || {
                parse(
                    Model::Apex,
                    &mut apex::parser(),
                    &file,
                    Ok(source.as_bytes().to_vec()),
                    false,
                )
            };
            let (parsed, types) = parse_source();
            assert!(parsed.parse_error().is_none());
            let declarations = parsed.declarations().to_vec();
            assert_eq!(declarations.len(), count);
            let index = index::Index::build(types, &declarations, &[]);
            let catalog = Catalog::build(&declarations, [], &[]);
            // Preserve all parsed rows and declarations, then add textual rows
            // using the original linear scan as the independent equivalent.
            let (base, _) = file_rows(
                &disk,
                &file,
                Some(parse_source().0),
                &index,
                &catalog,
                &no_names,
                ScopeSource::PackageDirectories,
            )
            .unwrap();
            let mut expected: Vec<Row> = base;
            let matches = textual::matches(source.as_bytes(), &names);
            assert_eq!(
                matches.last().unwrap().line as usize,
                source.lines().count()
            );
            for found in matches {
                expected.push(Row::Reference(Reference {
                    tier: Tier::Textual,
                    kind: RefKind::Text,
                    referenced: found.word,
                    referenced_type: None,
                    resolution: None,
                    detail: None,
                    candidates: Vec::new(),
                    referrer: enclosing(&declarations, found.line).map(|d| d.symbol.clone()),
                    source_field: None,
                    package: file.package.clone(),
                    file: file.path.clone(),
                    line: found.line,
                    column: found.column,
                }));
            }
            let mut expected: Vec<_> = expected
                .iter()
                .map(|row| (position(row), to_line(row)))
                .collect();
            expected.sort();
            let expected: Vec<_> = expected.into_iter().map(|(_, row)| row).collect();
            let (actual, _) = file_rows(
                &disk,
                &file,
                Some(parsed),
                &index,
                &catalog,
                &names,
                ScopeSource::PackageDirectories,
            )
            .unwrap();
            let actual: Vec<_> = actual.iter().map(to_line).collect();
            assert_eq!(actual, expected, "{count} declarations");
        }
    }

    #[test]
    fn enclosing_table_keeps_nested_boundaries_and_first_wins_ties() {
        let declarations = enclosing_declarations(&[(1, 20), (5, 9), (5, 9), (7, 11), (8, 8)]);
        let table = enclosing_table(&declarations, 21);
        for (line, expected) in [
            (0_usize, None),
            (1, Some(0)),
            (5, Some(1)),
            (7, Some(1)),
            (8, Some(4)),
            (9, Some(1)),
            (10, Some(3)),
            (11, Some(3)),
            (20, Some(0)),
            (21, None),
        ] {
            assert_eq!(
                line.checked_sub(1)
                    .and_then(|i| table.get(i))
                    .copied()
                    .flatten(),
                expected,
                "line {line}"
            );
        }
    }

    #[test]
    fn enclosing_table_matches_the_linear_scan_for_every_range_pair() {
        let ranges: Vec<_> = (1..=7)
            .flat_map(|start| (1..=7).map(move |end| (start, end)))
            .collect();
        for &a in &ranges {
            for &b in &ranges {
                let declarations = enclosing_declarations(&[a, b, (1, 6), (2, 4)]);
                let table = enclosing_table(&declarations, 8);
                for line in 1..=8 {
                    let expected = enclosing(&declarations, line as u32)
                        .map(|d| d.symbol.parse::<usize>().unwrap());
                    assert_eq!(
                        table[line - 1],
                        expected,
                        "ranges {a:?}, {b:?}, line {line}"
                    );
                }
            }
        }
    }

    #[test]
    fn enclosing_table_allocation_is_bounded_by_source_lines() {
        let declarations = enclosing_declarations(&[(1, u32::MAX), (2, 2), (5, u32::MAX)]);
        let table = enclosing_table(&declarations, 3);
        assert_eq!(table, [Some(0), Some(1), Some(0)]);
        assert!(enclosing_table(&declarations, 0).is_empty());
    }

    fn file(path: &str) -> ScopeFile {
        ScopeFile {
            path: path.into(),
            package: "pkg".into(),
        }
    }

    /// Symbol, first line and last line of each declaration.
    fn declared(path: &str, content: &str) -> Vec<(String, u32, u32)> {
        let read = read_one(&file(path), content).expect("a file that is read");
        let declarations = read.declarations.into_iter();
        declarations.map(|d| (d.symbol, d.start, d.end)).collect()
    }

    #[test]
    fn one_version_of_an_apex_file_gives_its_declarations_and_their_lines() {
        let source = "class Invoice {\n  Decimal total() {\n    return 1;\n  }\n}\n";

        let declarations = declared("pkg/classes/Invoice.cls", source);

        assert_eq!(
            declarations,
            [("Invoice".into(), 1, 5), ("Invoice.total()".into(), 2, 4)]
        );
    }

    #[test]
    fn a_declaration_stands_alone_if_nothing_else_is_on_its_first_and_last_lines() {
        let source = "public class Shared\n extends Base { Integer small() { return 1; }\n \
                      Integer plain() {\n  return 2;\n }\n static {\n } Integer after() { return 3; }\n \
                      Integer noted() { return 4; } // why\n Integer n, m;\n}\n";

        let read = read_one(&file("pkg/classes/Shared.cls"), source).unwrap();

        let alone: Vec<(&str, bool)> = read
            .declarations
            .iter()
            .map(|d| d.symbol.as_str())
            .zip(read.alone.iter().copied())
            .collect();
        assert_eq!(
            alone,
            [
                ("Shared", true),
                // The rest of the class header is in front of it.
                ("Shared.small()", false),
                ("Shared.plain()", true),
                // The end of an initializer is in front of it.
                ("Shared.after()", false),
                // A comment is after it, and a comment could be anything's.
                ("Shared.noted()", false),
                ("Shared.n", true),
                ("Shared.m", true),
            ]
        );
    }

    #[test]
    fn what_is_not_apex_is_not_known_to_stand_alone() {
        let flow = read_one(&file("pkg/flows/F.flow-meta.xml"), "<Flow>\n</Flow>\n");

        assert_eq!(flow.unwrap().alone, [false]);
    }

    #[test]
    fn one_version_of_a_metadata_file_is_declared_by_its_path() {
        let path = "pkg/objects/Invoice__c/fields/Total__c.field-meta.xml";

        let declarations = declared(path, "<CustomField>\n</CustomField>\n");

        assert_eq!(declarations, [("Invoice__c.Total__c".into(), 1, 2)]);
    }

    #[test]
    fn one_version_of_a_flow_is_declared_by_its_path() {
        let declarations = declared("pkg/flows/Welcome.flow-meta.xml", "<Flow>\n</Flow>\n");

        assert_eq!(declarations, [("Welcome".into(), 1, 2)]);
    }

    #[test]
    fn a_file_that_does_not_parse_cleanly_says_so_and_keeps_what_was_recovered() {
        let apex = read_one(&file("pkg/classes/A.cls"), "class A { void m( { } }");
        let xml = read_one(&file("pkg/flows/F.flow-meta.xml"), "<Flow><open>");
        let clean = read_one(&file("pkg/classes/B.cls"), "class B {}");

        let apex = apex.expect("an Apex file is read");
        assert!(apex.parse_error);
        assert_eq!(apex.declarations[0].symbol, "A");
        assert!(xml.expect("a Flow is read").parse_error);
        assert!(!clean.expect("an Apex file is read").parse_error);
    }

    #[test]
    fn a_file_of_a_bundle_declares_nothing_on_its_own() {
        let read = read_one(&file("pkg/lwc/hello/hello.js"), "export default 1;");

        assert_eq!(read.expect("a bundle file is read").declarations, []);
    }

    #[test]
    fn only_a_metadata_file_of_a_type_that_is_not_modelled_is_unmodelled() {
        assert_eq!(
            unmodelled("pkg/layouts/A.layout-meta.xml").as_deref(),
            Some("layout")
        );
        // A file of a bundle is modelled with its bundle, whatever its name.
        assert_eq!(unmodelled("pkg/lwc/hello/hello.js-meta.xml"), None);
        assert_eq!(unmodelled("pkg/classes/A.cls-meta.xml"), None);
        assert_eq!(unmodelled("pkg/flows/F.flow-meta.xml"), None);
        assert_eq!(unmodelled("pkg/notes.txt"), None);
    }

    #[test]
    fn permission_sets_profiles_and_records_are_the_files_read_entry_by_entry() {
        let entries = |path| read_entries(&file(path), b"<x>\n<y/>\n<z/>\n</x>\n".to_vec());

        for path in [
            "pkg/permissionsets/Billing.permissionset-meta.xml",
            "pkg/profiles/Admin.profile-meta.xml",
            "pkg/customMetadata/Plugin.Billing.md-meta.xml",
        ] {
            assert!(has_entries(path), "{path}");
            let read = entries(path).expect("a file with entries is read");
            assert_eq!(read.entries(), [(2, 2), (3, 3)], "{path}");
        }
        for path in ["pkg/classes/A.cls", "pkg/flows/F.flow-meta.xml"] {
            assert!(!has_entries(path), "{path}");
            assert!(entries(path).is_none(), "{path}");
        }
    }

    #[test]
    fn a_file_that_the_model_does_not_read_is_not_read() {
        assert!(reads("pkg/classes/A.cls") && reads("pkg/lwc/hello/hello.css"));
        for path in [
            "pkg/layouts/A.layout-meta.xml",
            "pkg/classes/A.cls-meta.xml",
            "pkg/notes.txt",
        ] {
            assert!(!reads(path), "{path}");
            assert!(read_one(&file(path), "<x/>").is_none(), "{path}");
        }
    }
}
