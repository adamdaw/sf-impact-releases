//! The refs pack, schema 2: the rows of `refs.jsonl`, exactly and in their
//! order, as a binary file, with an optional manifest of reuse facts.
//! `docs/pack.md` is the byte-level specification and
//! `schema/refs-pack.schema.json` the logical one; this module must match
//! them. A pack is a function of its content alone: equal content gives
//! equal bytes, and a reader refuses content that is not in canonical form.

use crate::diagnostic::shown;
use crate::rows::*;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

/// The version of the binary layout that this module writes and reads.
pub const VERSION: u64 = 2;
/// The version of the refs schema that line 1 of a pack names.
pub const SCHEMA: u64 = 2;

pub(crate) const MAGIC: &[u8] = b"SFIP";
/// The longest line 1 a pack may have: 64 KiB. Real packs' are 313 (LFC)
/// and 429 (NPSP) bytes.
const PROVENANCE_MAX: usize = 64 * 1024;
const LEVEL: i32 = 3;
const CHECKSUM: usize = 32;
// The decompression budget: what a section, and all of them, may expand to,
// from what is trusted, the bytes that are there. Real packs expand at most
// about 9.3 times per section and 4.5 times in all (NPSP and LFC).
pub(crate) const SECTION_RATIO: u64 = 32;
const PACK_RATIO: u64 = 16;
const BUDGET_FLOOR: u64 = 64 * 1024;
/// The largest window a frame may ask for: 4 MiB. A level-3 writer uses at
/// most 2 MiB.
const WINDOW_MAX: u64 = 4 << 20;
// The decoding budget: what decoding may allocate from the pack (see
// `Uses`), from the pack's length. Real packs take 91 (NPSP) and 135 (LFC)
// times their length; this is nearly three times the larger.
const ROW_RATIO: u64 = 384;
const ROW_FLOOR: u64 = 16 << 20;

pub(crate) const STRINGS: u8 = 1;
const FILES: u8 = 2;
const RECORDS: u8 = 3;
const PARSED: u8 = 4;
pub(crate) const TEXTUAL: u8 = 5;
pub(crate) const MANIFEST: u8 = 6;

/// Each row's tag in the file table.
const PARSE_ERROR: u8 = 0;
const SKIPPED_FILE: u8 = 1;
const UNMODELLED_FILE: u8 = 2;
const DECLARATION: u8 = 3;
const PARSED_REFERENCE: u8 = 4;
const TEXTUAL_REFERENCE: u8 = 5;
const UNSUPPORTED_ELEMENT: u8 = 6;

// The tag of a value is its index here: the schema's values, in its order.
#[rustfmt::skip]
const DECL_TYPES: [DeclType; 19] = { use DeclType::*; [
    Class, Interface, Enum, Trigger, Method, Constructor, Field, EnumValue, Object, ObjectField,
    ValidationRule, Label, CustomPermission, MessageChannel, PermissionSet, Profile,
    MetadataRecord, Flow, Lwc,
] };
#[rustfmt::skip]
pub(crate) const KINDS: [RefKind; 41] = { use RefKind::*; [
    Call, Construct, Type, Cast, Extends, Implements, StaticAccess, SoqlObject, SoqlField,
    DmlObject, DmlField, SobjectField, TriggerObject, Lookup, LookupFilter, Formula, Summary,
    ControllingField, DisplayField, ValueSet, Grant, Cmdt, CmdtField, CmdtType, FlowObject,
    FlowField, FlowAction, FlowSubflow, FlowComponent, FlowLabel, FlowCustomPermission,
    FlowCustomMetadata, LwcApex, LwcSchema, LwcLabel, LwcCustomPermission, LwcMessageChannel,
    LwcComponent, LwcTarget, LwcObject, Text,
] };
#[rustfmt::skip]
const REFERENCED_TYPES: [ReferencedType; 19] = { use ReferencedType::*; [
    Class, Method, Constructor, Field, EnumValue, Trigger, Object, ObjectField, ValidationRule,
    Label, CustomPermission, MessageChannel, PermissionSet, Profile, MetadataRecord, Flow, Lwc,
    Tab, GlobalValueSet,
] };
#[rustfmt::skip]
const RESOLUTIONS: [Resolution; 3] = { use Resolution::*; [Declaration, External, Unresolved] };
#[rustfmt::skip]
const DETAILS: [Detail; 4] =
    { use Detail::*; [AmbiguousOverload, AmbiguousReceiver, UnknownType, NoMember] };
#[rustfmt::skip]
const SOURCES: [ScopeSource; 2] = [ScopeSource::PackageDirectories, ScopeSource::UnpackagedMetadata];
#[rustfmt::skip]
const SCOPE_REASONS: [ScopeErrorReason; 3] =
    { use ScopeErrorReason::*; [Missing, NotADirectory, OutsideRoot] };
const SKIP_REASONS: [SkipReason; 1] = [SkipReason::Binary];
#[rustfmt::skip]
const UNSUPPORTED_REASONS: [UnsupportedReason; 2] =
    [UnsupportedReason::NotInterpreted, UnsupportedReason::NoName];

/// A pack's content: line 1, the rows, and the manifest if there is one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pack {
    /// The provenance row, as line 1 has it, without its line feed.
    pub provenance: String,
    pub rows: Vec<Row>,
    pub manifest: Option<Manifest>,
}

/// What a writer that reuses its last output needs that is not a row. No
/// output depends on it, and `--dump` does not print it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Manifest {
    /// Every file in scope, rows or not, in path order.
    pub files: Vec<FileEntry>,
    /// Every class, interface and enum, by file, and in a file in the
    /// order that phase one declared them.
    pub types: Vec<TypeDecl>,
    pub relationships: Vec<RelationshipFact>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FileEntry {
    pub package: String,
    pub path: String,
    /// The git blob id of the bytes that were read: 20 or 32 bytes.
    pub blob: Vec<u8>,
}

/// A class, interface or enum, as resolving a reference to it needs it:
/// what extract's phase one makes of its declaration. Type names are as
/// written; members are in source order, which resolving depends on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct TypeDecl {
    /// The index of the file in [`Manifest::files`].
    pub file: usize,
    pub symbol: String,
    pub superclass: Option<String>,
    pub interfaces: Vec<String>,
    pub methods: Vec<Callable>,
    pub constructors: Vec<Callable>,
    pub fields: Vec<Field>,
}

/// A method or a constructor of a [`TypeDecl`], which declares it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Callable {
    pub symbol: String,
    pub name: String,
    pub params: Vec<String>,
    /// `None` for `void` and for a constructor.
    pub returns: Option<String>,
}

/// A field or an enum value of a [`TypeDecl`], which declares it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Field {
    pub symbol: String,
    /// The name, case-folded: what an access is matched on.
    pub key: String,
    pub ty: String,
    pub is_static: bool,
    pub is_enum_value: bool,
}

/// A lookup or master-detail field, as following a relationship needs it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct RelationshipFact {
    /// The index of the file in [`Manifest::files`].
    pub file: usize,
    pub field: String,
    pub to: String,
    pub child_name: Option<String>,
    pub controlling: Option<String>,
}

impl Pack {
    /// The pack of a refs.jsonl whose line 1 says schema 2. Every row is
    /// held to refs.jsonl's own reader.
    pub fn from_jsonl(text: &str) -> Result<Pack, String> {
        let body = text
            .strip_suffix('\n')
            .ok_or("refs.jsonl: the last line has no line feed")?;
        let mut lines = body.split('\n');
        let provenance = lines.next().unwrap_or_default().to_string();
        let rows = lines.enumerate().map(|(at, line)| {
            serde_json::from_str(line).map_err(|e| format!("refs.jsonl: line {}: {e}", at + 2))
        });
        Ok(Pack {
            provenance,
            rows: rows.collect::<Result<_, _>>()?,
            manifest: None,
        })
    }

    /// The rows as the lines of refs.jsonl, line 1 first. The manifest is
    /// not part of refs.jsonl and is left out.
    pub fn to_jsonl(&self) -> Vec<String> {
        let rows = self.rows.iter().map(|row| {
            serde_json::to_string(row).expect("a row has only string keys and plain values")
        });
        std::iter::once(self.provenance.clone())
            .chain(rows)
            .collect()
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        self.encode_with(Strings::default())
    }

    /// [`Pack::encode`], from a string table that already holds `strings`:
    /// empty, except in a test of a writer that numbers strings wrongly.
    fn encode_with(&self, strings: Strings) -> Result<Vec<u8>, String> {
        check_provenance(&self.provenance)?;
        let errors = self.rows.iter().map_while(|row| match row {
            Row::ScopeError(error) => Some(error),
            _ => None,
        });
        let errors: Vec<&ScopeError> = errors.collect();
        let mut writer = Writer::with(strings, errors.iter().copied());
        for run in self.rows[errors.len()..].chunk_by(|a, b| located(a) == located(b)) {
            writer.push(FileRows::of(run)?)?;
        }
        writer.finish(&self.provenance, self.manifest.as_ref())
    }

    /// The content of a pack, refused unless it is all that docs/pack.md
    /// says: of this schema and version, intact, and in canonical form.
    pub fn decode(bytes: &[u8]) -> Result<Pack, String> {
        Self::decode_with(bytes, true)
    }
    /// As decode, but textual reference rows are read and checked, not kept.
    pub fn decode_without_textual(bytes: &[u8]) -> Result<Pack, String> {
        Self::decode_with(bytes, false)
    }
    fn decode_with(bytes: &[u8], keep_textual: bool) -> Result<Pack, String> {
        let (provenance, sections) = unframe(bytes)?;
        let mut uses = Uses {
            table: &[],
            next: 0,
            taken: 0,
            budget: budget(ROW_RATIO, bytes.len(), ROW_FLOOR),
        };
        let mut input = Input::new(&sections[0].1);
        let count = input.int()?;
        let mut table = Vec::with_capacity(uses.reserve::<String>(count)?);
        let mut seen = HashMap::with_capacity(uses.reserve_map::<String, u64>(count)?);
        for id in 0..count {
            // The table's copy and the map's, charged before either is made.
            let length = input.int()?;
            uses.take(length.saturating_mul(2))?;
            let s = input.string_of(length)?;
            if seen.insert(s.clone(), id).is_some() {
                return Err(format!(
                    "refs pack: the string `{}` is in the table twice",
                    Excerpt(&s)
                ));
            }
            table.push(s);
        }
        input.end()?;
        uses.table = &table;
        let mut files = Input::new(&sections[1].1);
        let mut records = Input::new(&sections[2].1);
        let mut parsed = Input::new(&sections[3].1);
        let mut textual = Input::new(&sections[4].1);
        let mut rows = Vec::with_capacity(uses.reserve::<Row>(row_count(&sections[1].1)?)?);
        for _ in 0..files.int()? {
            rows.push(Row::ScopeError(ScopeError {
                path: uses.get(records.int()?)?.into(),
                declared_in: untag(&SOURCES, records.byte()?, "declared_in")?,
                reason: untag(&SCOPE_REASONS, records.byte()?, "reason")?,
            }));
        }
        let count = files.int()?;
        let mut row_files: Vec<(&str, &str)> =
            Vec::with_capacity(uses.reserve::<(&str, &str)>(count)?);
        for _ in 0..count {
            let (package_id, path_id) = (files.int()?, files.int()?);
            let (package, path) = (uses.peek(package_id)?, uses.peek(path_id)?);
            if row_files.last().is_some_and(|(_, last)| *last >= path) {
                return Err(format!(
                    "refs pack: {}: files are out of order",
                    Excerpt(path)
                ));
            }
            row_files.push((package, path));
            let count = files.int()?;
            if count == 0 {
                return Err(format!("refs pack: {}: a file with no rows", Excerpt(path)));
            }
            let tags = files.take(count)?;
            let mut textuals_left = tags.iter().filter(|t| **t == TEXTUAL_REFERENCE).count();
            let (mut line, mut textual_line) = (0, 0);
            let (mut group_left, mut referrer, mut started) = (0, None, false);
            for &row_tag in tags {
                let located = |uses: &mut Uses<'_>| -> Result<(String, String), String> {
                    Ok((uses.get(package_id)?.into(), uses.get(path_id)?.into()))
                };
                let row = match row_tag {
                    PARSE_ERROR => {
                        let (package, file) = located(&mut uses)?;
                        let line = u32_of(records.int()?)?;
                        Row::ParseError(ParseError {
                            package,
                            file,
                            line,
                        })
                    }
                    SKIPPED_FILE => {
                        let (package, file) = located(&mut uses)?;
                        let reason = untag(&SKIP_REASONS, records.byte()?, "reason")?;
                        Row::SkippedFile(SkippedFile {
                            package,
                            file,
                            reason,
                        })
                    }
                    UNMODELLED_FILE => {
                        let (package, file) = located(&mut uses)?;
                        let suffix = uses.get(records.int()?)?.into();
                        Row::UnmodelledFile(UnmodelledFile {
                            package,
                            file,
                            suffix,
                        })
                    }
                    DECLARATION => {
                        let symbol = uses.get(records.int()?)?.into();
                        let name = uses.get(records.int()?)?.into();
                        let kind = untag(&DECL_TYPES, records.byte()?, "type")?;
                        let (package, file) = located(&mut uses)?;
                        let start = u32_of(records.int()?)?;
                        let end = undelta(&mut start.clone(), records.int()?)?;
                        let flags = records.byte()?;
                        let scope = match flags >> 2 {
                            0 => None,
                            tag @ 1..=2 => Some(SOURCES[usize::from(tag - 1)]),
                            _ => return Err(format!("refs pack: declaration flags {flags:#x}")),
                        };
                        let annotations = uses.list(&mut records)?;
                        Row::Declaration(Declaration {
                            symbol,
                            name,
                            kind,
                            package,
                            file,
                            start,
                            end,
                            is_test: flags & 1 != 0,
                            annotations,
                            collision: flags & 2 != 0,
                            scope,
                        })
                    }
                    UNSUPPORTED_ELEMENT => {
                        let (package, file) = located(&mut uses)?;
                        Row::UnsupportedElement(UnsupportedElement {
                            package,
                            file,
                            element: uses.get(records.int()?)?.into(),
                            reason: untag(&UNSUPPORTED_REASONS, records.byte()?, "reason")?,
                            line: u32_of(records.int()?)?,
                            column: u32_of(records.int()?)?,
                        })
                    }
                    PARSED_REFERENCE => {
                        let kind = untag(&KINDS, parsed.byte()?, "kind")?;
                        let referenced = uses.get(parsed.int()?)?.into();
                        let referenced_type = untag_opt(&REFERENCED_TYPES, parsed.byte()?)?;
                        let resolution = untag_opt(&RESOLUTIONS, parsed.byte()?)?;
                        let detail = untag_opt(&DETAILS, parsed.byte()?)?;
                        let candidates = uses.list(&mut parsed)?;
                        let referrer = uses.opt(parsed.int()?)?;
                        let source_field = uses.opt(parsed.int()?)?;
                        let (package, file) = located(&mut uses)?;
                        Row::Reference(Reference {
                            tier: Tier::Parsed,
                            kind,
                            referenced,
                            referenced_type,
                            resolution,
                            detail,
                            candidates,
                            referrer,
                            source_field,
                            package,
                            file,
                            line: undelta(&mut line, parsed.int()?)?,
                            column: u32_of(parsed.int()?)?,
                        })
                    }
                    TEXTUAL_REFERENCE => {
                        if group_left == 0 {
                            let next = textual.int()?;
                            if started && referrer == Some(next) {
                                return Err(format!(
                                    "refs pack: {}: two adjacent textual groups have the \
                                     same referrer",
                                    Excerpt(path)
                                ));
                            }
                            group_left = textual.int()?;
                            if group_left == 0 || group_left > textuals_left as u64 {
                                return Err(format!(
                                    "refs pack: {}: a textual group of {group_left} rows",
                                    Excerpt(path)
                                ));
                            }
                            (referrer, started) = (Some(next), true);
                        }
                        group_left -= 1;
                        textuals_left -= 1;
                        if !keep_textual {
                            let referenced = uses.get(textual.int()?)?;
                            uses.opt_str(referrer.expect("a group was read"))?;
                            let (_, file) = (uses.get(package_id)?, uses.get(path_id)?);
                            let line = undelta(&mut textual_line, textual.int()?)?;
                            let column = u32_of(textual.int()?)?;
                            let why =
                                crate::contract::nonempty(&[referenced, file]).and_then(|()| {
                                    crate::contract::require(
                                        line > 0 && column > 0,
                                        "reference position must be positive",
                                    )
                                });
                            why.map_err(|why| format!("refs pack: {}: {why}", Excerpt(file)))?;
                            continue;
                        }
                        let referenced = uses.get(textual.int()?)?.into();
                        let referrer = uses.opt(referrer.expect("a group was read"))?;
                        let (package, file) = located(&mut uses)?;
                        Row::Reference(Reference {
                            tier: Tier::Textual,
                            kind: RefKind::Text,
                            referenced,
                            referenced_type: None,
                            resolution: None,
                            detail: None,
                            candidates: Vec::new(),
                            referrer,
                            source_field: None,
                            package,
                            file,
                            line: undelta(&mut textual_line, textual.int()?)?,
                            column: u32_of(textual.int()?)?,
                        })
                    }
                    other => {
                        return Err(format!(
                            "refs pack: {}: row tag {other} is unknown",
                            Excerpt(path)
                        ));
                    }
                };
                valid(&row)?;
                rows.push(row);
            }
        }
        for section in [files, records, parsed, textual] {
            section.end()?;
        }
        let manifest = match sections.get(MANIFEST as usize - 1) {
            Some((_, payload)) => Some(Manifest::decode(payload, &mut uses, &row_files)?),
            None => None,
        };
        if uses.next != table.len() {
            return Err(format!(
                "refs pack: a string that nothing uses: `{}`",
                Excerpt(&table[uses.next])
            ));
        }
        Ok(Pack {
            provenance: provenance.to_string(),
            rows,
            manifest,
        })
    }
}

/// The rows of one file, encoded on their own, with the file's strings
/// numbered in the order of their first use in it. [`Writer`] numbers them
/// again for the whole pack as it joins the files in path order, so files
/// can be encoded apart, in parallel, and the pack is the one that
/// [`Pack::encode`] writes for the same rows.
pub struct FileRows {
    strings: Vec<String>,
    package: u64,
    path: u64,
    tags: Vec<u8>,
    records: Vec<u64>,
    parsed: Vec<u64>,
    textual: Vec<u64>,
}

// A value of a section before the pack's strings are numbered is an int,
// or the id in its file of a string (`ID`), or of an `opt` one (`OPT`). No
// int of a section comes near either bit: they are lines, columns, counts,
// tags and the differences of lines.
const ID: u64 = 1 << 63;
const OPT: u64 = 1 << 62;

/// A file's strings, each once, in first use.
#[derive(Default)]
struct Local<'a> {
    // Keys come from untrusted input; seed the hasher against collision floods.
    ids: HashMap<&'a str, u64, foldhash::fast::RandomState>,
    table: Vec<&'a str>,
}

impl<'a> Local<'a> {
    fn id(&mut self, s: &'a str) -> u64 {
        let next = self.table.len() as u64;
        ID | *self.ids.entry(s).or_insert_with(|| {
            self.table.push(s);
            next
        })
    }

    fn opt(&mut self, s: Option<&'a str>) -> u64 {
        s.map_or(0, |s| OPT | (self.id(s) & !ID))
    }
}

impl FileRows {
    /// The rows of one file, in their order: one or more, every one of the
    /// same file, and each one that the refs.jsonl reader would accept.
    pub fn of<'a>(rows: &'a [Row]) -> Result<FileRows, String> {
        let first = rows.first().and_then(located);
        let (package, path) = first
            .ok_or("refs pack: a file with no rows, or a scope_error row after a file's rows")?;
        let mut strings = Local::default();
        let (mut tags, mut records) = (Vec::with_capacity(rows.len()), Vec::new());
        let (mut parsed, mut textuals) = (Vec::new(), Vec::new());
        let mut line = 0;
        for row in rows {
            if located(row) != Some((package, path)) {
                return Err(format!(
                    "refs pack: {}: the rows of a file are not together in path order",
                    Excerpt(path)
                ));
            }
            valid(row)?;
            let located = |strings: &mut Local<'a>| {
                strings.id(package);
                strings.id(path);
            };
            match row {
                Row::ScopeError(_) => unreachable!("a scope error has no file"),
                Row::ParseError(error) => {
                    tags.push(PARSE_ERROR);
                    located(&mut strings);
                    records.push(error.line.into());
                }
                Row::SkippedFile(skipped) => {
                    tags.push(SKIPPED_FILE);
                    located(&mut strings);
                    records.push(tag(&SKIP_REASONS, &skipped.reason).into());
                }
                Row::UnmodelledFile(unmodelled) => {
                    tags.push(UNMODELLED_FILE);
                    located(&mut strings);
                    records.push(strings.id(&unmodelled.suffix));
                }
                Row::Declaration(d) => {
                    tags.push(DECLARATION);
                    records.push(strings.id(&d.symbol));
                    records.push(strings.id(&d.name));
                    records.push(tag(&DECL_TYPES, &d.kind).into());
                    located(&mut strings);
                    records.push(d.start.into());
                    records.push(delta(&mut d.start.clone(), d.end));
                    let scope = d.scope.map_or(0, |s| tag(&SOURCES, &s) + 1);
                    let flags = u8::from(d.is_test) | u8::from(d.collision) << 1 | scope << 2;
                    records.push(flags.into());
                    records.push(d.annotations.len() as u64);
                    for annotation in &d.annotations {
                        records.push(strings.id(annotation));
                    }
                }
                Row::UnsupportedElement(element) => {
                    tags.push(UNSUPPORTED_ELEMENT);
                    located(&mut strings);
                    records.push(strings.id(&element.element));
                    records.push(tag(&UNSUPPORTED_REASONS, &element.reason).into());
                    records.push(element.line.into());
                    records.push(element.column.into());
                }
                Row::Reference(r) if r.tier == Tier::Textual => {
                    tags.push(TEXTUAL_REFERENCE);
                    let referenced = strings.id(&r.referenced);
                    let referrer = strings.opt(r.referrer.as_deref());
                    located(&mut strings);
                    textuals.push((referrer, referenced, r.line, r.column));
                }
                Row::Reference(r) => {
                    tags.push(PARSED_REFERENCE);
                    parsed.push(tag(&KINDS, &r.kind).into());
                    parsed.push(strings.id(&r.referenced));
                    parsed.push(opt_tag(&REFERENCED_TYPES, r.referenced_type.as_ref()).into());
                    parsed.push(opt_tag(&RESOLUTIONS, r.resolution.as_ref()).into());
                    parsed.push(opt_tag(&DETAILS, r.detail.as_ref()).into());
                    parsed.push(r.candidates.len() as u64);
                    for candidate in &r.candidates {
                        parsed.push(strings.id(candidate));
                    }
                    parsed.push(strings.opt(r.referrer.as_deref()));
                    parsed.push(strings.opt(r.source_field.as_deref()));
                    located(&mut strings);
                    parsed.push(delta(&mut line, r.line));
                    parsed.push(r.column.into());
                }
            }
        }
        let mut textual = Vec::new();
        let mut line = 0;
        for group in textuals.chunk_by(|a, b| a.0 == b.0) {
            textual.push(group[0].0);
            textual.push(group.len() as u64);
            for &(_, referenced, at, column) in group {
                textual.push(referenced);
                textual.push(delta(&mut line, at));
                textual.push(column.into());
            }
        }
        let (package, path) = (strings.id(package) & !ID, strings.id(path) & !ID);
        Ok(FileRows {
            strings: strings.table.into_iter().map(str::to_owned).collect(),
            package,
            path,
            tags,
            records,
            parsed,
            textual,
        })
    }
}

/// Writes a pack: its scope errors, then the rows of each file, the files
/// in path order, then line 1 and the manifest when it is finished.
pub struct Writer {
    strings: Strings,
    scope_errors: u64,
    files: Vec<u8>,
    records: Vec<u8>,
    parsed: Vec<u8>,
    textual: Vec<u8>,
    row_files: Vec<(String, String)>,
}

impl Writer {
    /// A pack whose rows start with these scope errors, in this order.
    pub fn new<'a>(scope_errors: impl IntoIterator<Item = &'a ScopeError>) -> Writer {
        Writer::with(Strings::default(), scope_errors)
    }

    fn with<'a>(
        mut strings: Strings,
        scope_errors: impl IntoIterator<Item = &'a ScopeError>,
    ) -> Writer {
        let (mut records, mut count) = (Vec::new(), 0);
        for error in scope_errors {
            int(&mut records, strings.id(&error.path));
            records.push(tag(&SOURCES, &error.declared_in));
            records.push(tag(&SCOPE_REASONS, &error.reason));
            count += 1;
        }
        Writer {
            strings,
            scope_errors: count,
            files: Vec::new(),
            records,
            parsed: Vec::new(),
            textual: Vec::new(),
            row_files: Vec::new(),
        }
    }

    /// The rows of the next file, which comes after the last by path.
    pub fn push(&mut self, file: FileRows) -> Result<(), String> {
        let package = file.strings[file.package as usize].clone();
        let path = file.strings[file.path as usize].clone();
        if self.row_files.last().is_some_and(|(_, last)| *last >= path) {
            return Err(format!(
                "refs pack: {}: the rows of a file are not together in path order",
                Excerpt(&path)
            ));
        }
        let ids: Vec<u64> = file.strings.iter().map(|s| self.strings.id(s)).collect();
        let id = |value: u64| match value {
            _ if value & ID != 0 => ids[(value & !ID) as usize],
            _ if value & OPT != 0 => ids[(value & !OPT) as usize] + 1,
            _ => value,
        };
        int(&mut self.files, ids[file.package as usize]);
        int(&mut self.files, ids[file.path as usize]);
        int(&mut self.files, file.tags.len() as u64);
        self.files.extend_from_slice(&file.tags);
        for (section, values) in [
            (&mut self.records, &file.records),
            (&mut self.parsed, &file.parsed),
            (&mut self.textual, &file.textual),
        ] {
            for &value in values {
                int(section, id(value));
            }
        }
        self.row_files.push((package, path));
        Ok(())
    }

    /// The pack, with `provenance` as line 1 and the manifest if there is one.
    pub fn finish(self, provenance: &str, manifest: Option<&Manifest>) -> Result<Vec<u8>, String> {
        let Writer {
            mut strings,
            scope_errors,
            files: rows_of_files,
            records,
            parsed,
            textual,
            row_files,
        } = self;
        check_provenance(provenance)?;
        if provenance.contains('\n') {
            return Err("refs pack: line 1 has a line feed in it".into());
        }
        let mut files = Vec::with_capacity(rows_of_files.len() + 20);
        int(&mut files, scope_errors);
        int(&mut files, row_files.len() as u64);
        files.extend_from_slice(&rows_of_files);
        drop(rows_of_files);
        // The manifest's strings come after the rows', so it is written first.
        let manifest = match manifest {
            Some(manifest) => {
                let row_files: Vec<(&str, &str)> = row_files
                    .iter()
                    .map(|(p, f)| (p.as_str(), f.as_str()))
                    .collect();
                manifest.check(&row_files)?;
                Some((MANIFEST, manifest.encode(&mut strings)))
            }
            None => None,
        };
        let mut table = Vec::new();
        int(&mut table, strings.table.len() as u64);
        for s in &strings.table {
            string(&mut table, s);
        }
        let mut sections = vec![
            (STRINGS, table),
            (FILES, files),
            (RECORDS, records),
            (PARSED, parsed),
            (TEXTUAL, textual),
        ];
        sections.extend(manifest);
        frame(provenance, &sections)
    }
}

impl Manifest {
    fn encode(&self, strings: &mut Strings) -> Vec<u8> {
        let mut out = Vec::new();
        int(&mut out, self.files.len() as u64);
        for entry in &self.files {
            int(&mut out, strings.id(&entry.package));
            int(&mut out, strings.id(&entry.path));
            out.push(entry.blob.len() as u8);
            out.extend_from_slice(&entry.blob);
        }
        int(&mut out, self.types.len() as u64);
        let mut file = 0;
        for ty in &self.types {
            int(&mut out, (ty.file - file) as u64);
            file = ty.file;
            int(&mut out, strings.id(&ty.symbol));
            int(&mut out, strings.opt(ty.superclass.as_deref()));
            ids(&mut out, strings, &ty.interfaces);
            for callables in [&ty.methods, &ty.constructors] {
                int(&mut out, callables.len() as u64);
                for callable in callables {
                    int(&mut out, strings.id(&callable.symbol));
                    int(&mut out, strings.id(&callable.name));
                    ids(&mut out, strings, &callable.params);
                    int(&mut out, strings.opt(callable.returns.as_deref()));
                }
            }
            int(&mut out, ty.fields.len() as u64);
            for field in &ty.fields {
                int(&mut out, strings.id(&field.symbol));
                int(&mut out, strings.id(&field.key));
                int(&mut out, strings.id(&field.ty));
                out.push(u8::from(field.is_static) | u8::from(field.is_enum_value) << 1);
            }
        }
        int(&mut out, self.relationships.len() as u64);
        let mut file = 0;
        for r in &self.relationships {
            int(&mut out, (r.file - file) as u64);
            file = r.file;
            int(&mut out, strings.id(&r.field));
            int(&mut out, strings.id(&r.to));
            int(&mut out, strings.opt(r.child_name.as_deref()));
            int(&mut out, strings.opt(r.controlling.as_deref()));
        }
        out
    }

    fn decode(
        payload: &[u8],
        uses: &mut Uses<'_>,
        row_files: &[(&str, &str)],
    ) -> Result<Self, String> {
        let mut input = Input::new(payload);
        let count = input.int()?;
        let mut files = Vec::with_capacity(uses.reserve::<FileEntry>(count)?);
        for _ in 0..count {
            let package = uses.get(input.int()?)?.into();
            let path = uses.get(input.int()?)?.into();
            let length = input.byte()?;
            uses.take(length.into())?;
            let blob = input.take(length.into())?.to_vec();
            files.push(FileEntry {
                package,
                path,
                blob,
            });
        }
        let count = input.int()?;
        let mut types = Vec::with_capacity(uses.reserve::<TypeDecl>(count)?);
        let mut file = 0;
        for _ in 0..count {
            let file = file_of(&mut input, &mut file)?;
            let symbol = uses.get(input.int()?)?.into();
            let superclass = uses.opt(input.int()?)?;
            let interfaces = uses.list(&mut input)?;
            let methods = callables(&mut input, uses)?;
            let constructors = callables(&mut input, uses)?;
            let count = input.int()?;
            let mut fields = Vec::with_capacity(uses.reserve::<Field>(count)?);
            for _ in 0..count {
                let symbol = uses.get(input.int()?)?.into();
                let key = uses.get(input.int()?)?.into();
                let ty = uses.get(input.int()?)?.into();
                let flags = input.byte()?;
                if flags > 3 {
                    return Err(format!("refs pack: field flags {flags:#x}"));
                }
                fields.push(Field {
                    symbol,
                    key,
                    ty,
                    is_static: flags & 1 != 0,
                    is_enum_value: flags & 2 != 0,
                });
            }
            types.push(TypeDecl {
                file,
                symbol,
                superclass,
                interfaces,
                methods,
                constructors,
                fields,
            });
        }
        let count = input.int()?;
        let mut relationships = Vec::with_capacity(uses.reserve::<RelationshipFact>(count)?);
        let mut file = 0;
        for _ in 0..count {
            relationships.push(RelationshipFact {
                file: file_of(&mut input, &mut file)?,
                field: uses.get(input.int()?)?.into(),
                to: uses.get(input.int()?)?.into(),
                child_name: uses.opt(input.int()?)?,
                controlling: uses.opt(input.int()?)?,
            });
        }
        input.end()?;
        let manifest = Manifest {
            files,
            types,
            relationships,
        };
        manifest.check(row_files)?;
        Ok(manifest)
    }

    /// Canonical order, the strings that the schema requires not empty,
    /// and every file of the rows an entry. Writer and reader both hold a
    /// manifest to this.
    fn check(&self, row_files: &[(&str, &str)]) -> Result<(), String> {
        let length = self.files.first().map(|entry| entry.blob.len());
        let last = self.types.last().map(|t| t.file);
        let callables = || {
            let types = self.types.iter();
            types.flat_map(|t| t.methods.iter().chain(&t.constructors))
        };
        let fields = || self.types.iter().flat_map(|t| &t.fields);
        let last = last.max(self.relationships.last().map(|r| r.file));
        let problems = [
            (
                !self.files.is_sorted_by(|a, b| a.path < b.path),
                "entries are out of order",
            ),
            (
                !self.types.is_sorted_by_key(|t| t.file),
                "types are out of order by file",
            ),
            (
                !self.relationships.is_sorted_by(|a, b| a < b),
                "relationships are out of order",
            ),
            (
                self.files.iter().any(|e| e.path.is_empty()),
                "has an entry with an empty path",
            ),
            (
                self.types.iter().any(|t| t.symbol.is_empty())
                    || callables().any(|c| c.symbol.is_empty() || c.name.is_empty())
                    || fields().any(|f| f.symbol.is_empty() || f.key.is_empty()),
                "has a type, callable or field with an empty symbol, name or key",
            ),
            (
                self.relationships
                    .iter()
                    .any(|r| r.field.is_empty() || r.to.is_empty()),
                "has a relationship with an empty field or to",
            ),
            (
                self.files.iter().any(|e| Some(e.blob.len()) != length)
                    || length.is_some_and(|length| length != 20 && length != 32),
                "blob ids are not all 20 or all 32 bytes",
            ),
            (
                last.is_some_and(|file| file >= self.files.len()),
                "has a type or relationship of no entry",
            ),
        ];
        if let Some((_, why)) = problems.iter().find(|(failed, _)| *failed) {
            return Err(format!("refs pack: manifest {why}"));
        }
        let mut entries = self.files.iter();
        for (package, path) in row_files {
            if !entries.any(|entry| entry.path == *path && entry.package == *package) {
                return Err(format!(
                    "refs pack: {} has rows and no manifest entry",
                    Excerpt(path)
                ));
            }
        }
        Ok(())
    }
}

/// A list of methods or of constructors.
fn callables(input: &mut Input<'_>, uses: &mut Uses<'_>) -> Result<Vec<Callable>, String> {
    let count = input.int()?;
    let mut callables = Vec::with_capacity(uses.reserve::<Callable>(count)?);
    for _ in 0..count {
        callables.push(Callable {
            symbol: uses.get(input.int()?)?.into(),
            name: uses.get(input.int()?)?.into(),
            params: uses.list(input)?,
            returns: uses.opt(input.int()?)?,
        });
    }
    Ok(callables)
}

/// A list of strings, its length first, then the id of each.
fn ids(out: &mut Vec<u8>, strings: &mut Strings, list: &[String]) {
    int(out, list.len() as u64);
    for s in list {
        int(out, strings.id(s));
    }
}

/// The index of a type's or relationship's file: the previous one's,
/// `file`, plus a step.
fn file_of(input: &mut Input<'_>, file: &mut usize) -> Result<usize, String> {
    let step = usize::try_from(input.int()?).map_err(|_| "refs pack: a file index")?;
    *file = file.checked_add(step).ok_or("refs pack: a file index")?;
    Ok(*file)
}

/// The rows that a files section says the pack has: its scope errors and
/// each file's tags. Read ahead, so that the rows are charged and reserved
/// once, before any is decoded.
fn row_count(section: &[u8]) -> Result<u64, String> {
    let mut files = Input::new(section);
    let mut count = files.int()?;
    for _ in 0..files.int()? {
        files.int()?;
        files.int()?;
        let tags = files.int()?;
        files.take(tags)?;
        count = count.saturating_add(tags);
    }
    Ok(count)
}

/// The package and file of a row that has them: every row but a scope error.
pub fn located(row: &Row) -> Option<(&str, &str)> {
    let (package, file) = match row {
        Row::ScopeError(_) => return None,
        Row::ParseError(v) => (&v.package, &v.file),
        Row::SkippedFile(v) => (&v.package, &v.file),
        Row::UnmodelledFile(v) => (&v.package, &v.file),
        Row::Declaration(v) => (&v.package, &v.file),
        Row::Reference(v) => (&v.package, &v.file),
        Row::UnsupportedElement(v) => (&v.package, &v.file),
    };
    Some((package, file))
}

/// A row that the refs.jsonl reader would accept: its checks, and the
/// fields that each wire shape of a reference has.
fn valid(row: &Row) -> Result<(), String> {
    let why = match row {
        Row::Reference(r) => {
            let textual = r.tier == Tier::Textual;
            let only_parsed = r.referenced_type.is_some()
                || r.resolution.is_some()
                || r.detail.is_some()
                || !r.candidates.is_empty();
            let shape = if textual {
                !only_parsed && r.source_field.is_none()
            } else {
                r.resolution.is_some() && (r.source_field.is_none() || r.kind.is_metadata())
            };
            let presence = Presence {
                referenced_type: r.referenced_type.is_some(),
                detail: r.detail.is_some(),
                candidates: !r.candidates.is_empty(),
                source_field: r.source_field.is_some(),
            };
            match shape {
                true => r.check(presence),
                false => Err("a field that its tier does not have".into()),
            }
        }
        row => row.check(),
    };
    let file = located(row).map_or("", |(_, f)| f);
    why.map_err(|why| format!("refs pack: {}: {why}", Excerpt(file)))
}

fn check_provenance(line: &str) -> Result<(), String> {
    let row: Value = serde_json::from_str(line)
        .map_err(|e| format!("refs pack: line 1 is not a provenance row: {e}"))?;
    if row["row"] != "provenance" || row["tool"] != "sf-impact-extract" {
        return Err("refs pack: line 1 is not the provenance row of sf-impact-extract".into());
    }
    match row["schema"].as_u64() {
        Some(SCHEMA) => Ok(()),
        Some(other) => Err(format!(
            "refs pack: its `schema` is version {other}, and a pack is schema {SCHEMA}"
        )),
        None => Err("refs pack: line 1 has no `schema`".into()),
    }
}

/// What `ratio` times `bytes`, or `floor` if that is more, allows.
fn budget(ratio: u64, bytes: usize, floor: u64) -> u64 {
    (bytes as u64).saturating_mul(ratio).max(floor)
}

/// The window that a zstd frame's header asks for (RFC 8878, 3.1.1.1), or
/// `None` if it is not a zstd frame.
fn window(frame: &[u8]) -> Option<u64> {
    let header = frame.strip_prefix(&[0x28, 0xb5, 0x2f, 0xfd])?;
    let descriptor = *header.first()?;
    if descriptor & 0x20 == 0 {
        let byte = *header.get(1)?;
        let base = 1u64 << (10 + (byte >> 3));
        return Some(base + base / 8 * u64::from(byte & 7));
    }
    // A single segment: the window is the content size.
    let at = 1 + [0, 1, 2, 4][usize::from(descriptor & 3)];
    let width = [1, 2, 4, 8][usize::from(descriptor >> 6)];
    let size = header.get(at..at + width)?;
    let size = size
        .iter()
        .rev()
        .fold(0, |n, byte| n << 8 | u64::from(*byte));
    Some(if width == 2 { size + 256 } else { size })
}

/// A section's payload: one zstd frame, of the size it says, within its
/// budget, and whose header asks for a window of at most 4 MiB. Nothing is
/// allocated beyond it.
fn decompress(kind: u8, size: u64, frame: &[u8]) -> Result<Vec<u8>, String> {
    if size > budget(SECTION_RATIO, frame.len(), BUDGET_FLOOR) {
        return Err(format!(
            "refs pack: section {kind} says it expands to {size} bytes, over its \
             decompression budget"
        ));
    }
    if zstd::zstd_safe::find_frame_compressed_size(frame) != Ok(frame.len()) {
        return Err(format!(
            "refs pack: section {kind} is not exactly one zstd frame"
        ));
    }
    if window(frame).is_none_or(|window| window > WINDOW_MAX) {
        return Err(format!(
            "refs pack: section {kind}'s frame asks for a window over 4 MiB"
        ));
    }
    let payload = zstd::bulk::decompress(frame, size as usize)
        .map_err(|e| format!("refs pack: section {kind} does not decompress: {e}"))?;
    if payload.len() as u64 != size {
        return Err(format!(
            "refs pack: section {kind} is not of the size it says"
        ));
    }
    Ok(payload)
}

/// Each section's kind and payload, in pack order.
pub(crate) type Sections = Vec<(u8, Vec<u8>)>;

/// Line 1, the header, each section compressed, and the checksum. A
/// section that compresses past what a reader's decompression budget
/// allows is stored instead, and so is each section that compresses best
/// in turn while all of them together are past the pack's: what this
/// writes, its reader reads.
pub(crate) fn frame(provenance: &str, sections: &[(u8, Vec<u8>)]) -> Result<Vec<u8>, String> {
    if provenance.len() > PROVENANCE_MAX {
        return Err(over_provenance_max());
    }
    let mut frames = Vec::with_capacity(sections.len());
    for (_, payload) in sections {
        let frame = zstd::bulk::compress(payload, LEVEL).map_err(|e| format!("refs pack: {e}"))?;
        let within = payload.len() as u64 <= budget(SECTION_RATIO, frame.len(), BUDGET_FLOOR);
        frames.push(if within { frame } else { stored(payload) });
    }
    let total: u64 = sections
        .iter()
        .map(|(_, payload)| payload.len() as u64)
        .sum();
    loop {
        let mut out = [provenance.as_bytes(), b"\n", MAGIC].concat();
        int(&mut out, VERSION);
        for ((kind, payload), frame) in sections.iter().zip(&frames) {
            out.push(*kind);
            int(&mut out, payload.len() as u64);
            int(&mut out, frame.len() as u64);
            out.extend_from_slice(frame);
        }
        if total <= budget(PACK_RATIO, out.len() + CHECKSUM, BUDGET_FLOOR) {
            let checksum = Sha256::digest(&out);
            out.extend_from_slice(&checksum);
            return Ok(out);
        }
        // A stored frame is longer than its payload, so this ends with
        // every section stored at the latest.
        let best = sections
            .iter()
            .zip(&frames)
            .enumerate()
            .filter(|(_, ((_, payload), frame))| frame.len() < payload.len())
            .max_by(|(_, ((_, p), f)), (_, ((_, q), g))| {
                (p.len() as u128 * g.len() as u128).cmp(&(q.len() as u128 * f.len() as u128))
            })
            .map(|(at, _)| at)
            .expect("a pack whose frames are all stored is within its budget");
        frames[best] = stored(&sections[best].1);
    }
}

/// `payload` as one zstd frame of raw blocks, uncompressed (RFC 8878,
/// 3.1.1): its header gives the content size and a 128 KiB window, the
/// largest a block may be.
fn stored(payload: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 128 * 1024;
    // The magic number; an 8-byte content size and no single segment; the
    // window, 2^(10 + 7) bytes.
    let mut out = vec![0x28, 0xb5, 0x2f, 0xfd, 0xc0, 7 << 3];
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    let mut at = 0;
    loop {
        let end = payload.len().min(at + BLOCK);
        let last = end == payload.len();
        // Last_Block, then Block_Type 0 (raw), then Block_Size.
        let header = ((end - at) as u32) << 3 | u32::from(last);
        out.extend_from_slice(&header.to_le_bytes()[..3]);
        out.extend_from_slice(&payload[at..end]);
        if last {
            return out;
        }
        at = end;
    }
}

fn over_provenance_max() -> String {
    format!("refs pack: line 1 is over {PROVENANCE_MAX} bytes, its limit")
}

/// Line 1 and each section's payload, after the schema, the version, the
/// checksum, the sections' kinds and the budget are checked, in that order.
pub(crate) fn unframe(bytes: &[u8]) -> Result<(&str, Sections), String> {
    // Line 1 is looked for, and parsed, only within its limit.
    let lf = bytes
        .iter()
        .take(PROVENANCE_MAX + 1)
        .position(|b| *b == b'\n');
    let lf = match lf {
        Some(lf) => lf,
        None if bytes.len() > PROVENANCE_MAX => return Err(over_provenance_max()),
        None => {
            return Err(
                "refs pack: there is no line 1: it is empty, cut short or not a pack".into(),
            );
        }
    };
    let provenance = std::str::from_utf8(&bytes[..lf]).map_err(|_| "refs pack: line 1")?;
    check_provenance(provenance)?;
    let rest = &bytes[lf + 1..];
    let header = rest
        .strip_prefix(MAGIC)
        .ok_or("refs pack: no pack header after line 1")?;
    let mut input = Input::new(header);
    let version = input.int()?;
    if version != VERSION {
        return Err(format!(
            "refs pack: version {version} is unknown; this reads version {VERSION}"
        ));
    }
    let start = lf + 1 + MAGIC.len() + input.at;
    let Some(end) = bytes
        .len()
        .checked_sub(CHECKSUM)
        .filter(|end| *end >= start)
    else {
        return Err("refs pack: it is cut short: there is no checksum".into());
    };
    if Sha256::digest(&bytes[..end])[..] != bytes[end..] {
        return Err("refs pack: the checksum does not match: it is corrupt or cut short".into());
    }
    // Every section's header before any frame is decompressed.
    let expected = [STRINGS, FILES, RECORDS, PARSED, TEXTUAL];
    let mut input = Input::new(&bytes[start..end]);
    let (mut frames, mut kinds) = (Vec::new(), Vec::new());
    while input.at < input.bytes.len() {
        let (kind, size, length) = (input.byte()?, input.int()?, input.int()?);
        kinds.push(kind);
        // A seventh section is refused below: nothing beyond it is kept.
        if kinds.len() > expected.len() + 1 {
            break;
        }
        frames.push((kind, size, input.take(length)?));
    }
    if !kinds.starts_with(&expected) || kinds[expected.len()..].iter().any(|k| *k != MANIFEST) {
        return Err(format!(
            "refs pack: its sections are {kinds:?}, not {expected:?} and an optional manifest"
        ));
    }
    if kinds.len() > expected.len() + 1 {
        return Err("refs pack: a second manifest section".into());
    }
    let total = frames
        .iter()
        .map(|(_, size, _)| *size)
        .fold(0, u64::saturating_add);
    if total > budget(PACK_RATIO, bytes.len(), BUDGET_FLOOR) {
        return Err(format!(
            "refs pack: its sections say they expand to {total} bytes, over the pack's \
             decompression budget"
        ));
    }
    let sections = frames
        .into_iter()
        .map(|(kind, size, frame)| Ok((kind, decompress(kind, size, frame)?)));
    let sections = sections.collect::<Result<_, String>>()?;
    Ok((provenance, sections))
}

/// A pack's string as a diagnostic shows it: whole up to 128 bytes, and
/// past that its first 128 bytes (to a character) and its length, so that
/// no message copies a pack-sized string. Escaped as [`shown`] says.
struct Excerpt<'a>(&'a str);

impl std::fmt::Display for Excerpt<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const SHOWN: usize = 128;
        let s = self.0;
        match s.len() <= SHOWN {
            true => f.write_str(&shown(s)),
            false => {
                let prefix = &s[..s.floor_char_boundary(SHOWN)];
                write!(f, "{}… ({} bytes)", shown(prefix), s.len())
            }
        }
    }
}

/// The string table as it is written: each string once, in first use.
#[derive(Default)]
struct Strings {
    ids: HashMap<String, u64>,
    table: Vec<String>,
}

impl Strings {
    fn id(&mut self, s: &str) -> u64 {
        if let Some(id) = self.ids.get(s) {
            return *id;
        }
        let id = self.table.len() as u64;
        self.ids.insert(s.to_owned(), id);
        self.table.push(s.to_owned());
        id
    }

    fn opt(&mut self, s: Option<&str>) -> u64 {
        s.map_or(0, |s| self.id(s) + 1)
    }
}

/// The string table as it is read, held to first-use order: each id is
/// one already used or the next one. It counts what decoding allocates
/// from the pack against the decoding budget: the string table and the map
/// that finds its repeats, each string it hands out, and the room for every
/// list, rows included, charged before the list is reserved.
struct Uses<'a> {
    table: &'a [String],
    next: usize,
    taken: u64,
    budget: u64,
}

impl<'a> Uses<'a> {
    fn peek(&self, id: u64) -> Result<&'a str, String> {
        let table = self.table;
        let found = usize::try_from(id).ok().and_then(|id| table.get(id));
        found
            .map(String::as_str)
            .ok_or_else(|| format!("refs pack: string id {id} is out of range"))
    }

    fn get(&mut self, id: u64) -> Result<&'a str, String> {
        let s = self.peek(id)?;
        self.take(s.len() as u64)?;
        match (id as usize).cmp(&self.next) {
            std::cmp::Ordering::Less => {}
            std::cmp::Ordering::Equal => self.next += 1,
            std::cmp::Ordering::Greater => {
                return Err(format!(
                    "refs pack: strings are not in first-use order at `{}`",
                    Excerpt(s)
                ));
            }
        }
        Ok(s)
    }

    /// Room for `count` values of `T`, charged before it is reserved.
    fn reserve<T>(&mut self, count: u64) -> Result<usize, String> {
        self.take(count.saturating_mul(std::mem::size_of::<T>() as u64))?;
        usize::try_from(count).map_err(|_| "refs pack: a count does not fit memory".into())
    }

    /// Room for a hash map of `count` entries, charged before it is
    /// reserved. The standard map (hashbrown) has a power of two buckets,
    /// at most 7/8 full and at least 4, each an entry and a control byte,
    /// and 16 more control bytes: under 3 × count + 9 buckets in all.
    fn reserve_map<K, V>(&mut self, count: u64) -> Result<usize, String> {
        let buckets = count.saturating_mul(3).saturating_add(9);
        let bucket = std::mem::size_of::<(K, V)>() as u64 + 1;
        self.take(buckets.saturating_mul(bucket))?;
        usize::try_from(count).map_err(|_| "refs pack: a count does not fit memory".into())
    }

    /// A list of strings, its length first, then the id of each.
    fn list(&mut self, input: &mut Input<'_>) -> Result<Vec<String>, String> {
        let count = input.int()?;
        let mut list = Vec::with_capacity(self.reserve::<String>(count)?);
        for _ in 0..count {
            list.push(self.get(input.int()?)?.to_string());
        }
        Ok(list)
    }

    fn take(&mut self, bytes: u64) -> Result<(), String> {
        self.taken = self.taken.saturating_add(bytes);
        match self.taken <= self.budget {
            true => Ok(()),
            false => Err(format!(
                "refs pack: its rows would take over {} bytes, its decoding budget",
                self.budget
            )),
        }
    }

    fn opt_str(&mut self, id: u64) -> Result<Option<&'a str>, String> {
        match id {
            0 => Ok(None),
            id => Ok(Some(self.get(id - 1)?)),
        }
    }

    fn opt(&mut self, id: u64) -> Result<Option<String>, String> {
        match id {
            0 => Ok(None),
            id => Ok(Some(self.get(id - 1)?.to_string())),
        }
    }
}

pub(crate) struct Input<'a> {
    bytes: &'a [u8],
    pub(crate) at: usize,
}

impl<'a> Input<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Input { bytes, at: 0 }
    }

    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn take(&mut self, count: u64) -> Result<&'a [u8], String> {
        let end = usize::try_from(count)
            .ok()
            .and_then(|n| self.at.checked_add(n));
        let taken = end.and_then(|end| self.bytes.get(self.at..end));
        let taken = taken.ok_or("refs pack: a section ends in the middle of a value")?;
        self.at += taken.len();
        Ok(taken)
    }

    /// An unsigned LEB128 int, in its shortest form.
    pub(crate) fn int(&mut self) -> Result<u64, String> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = self.byte()?;
            let bits = u64::from(byte & 0x7f);
            if bits << shift >> shift != bits {
                return Err("refs pack: an int does not fit 64 bits".into());
            }
            value |= bits << shift;
            if byte & 0x80 == 0 {
                if byte == 0 && shift > 0 {
                    return Err("refs pack: an int is not in its shortest form".into());
                }
                return Ok(value);
            }
        }
        Err("refs pack: an int does not fit 64 bits".into())
    }

    /// A string, once its length has been read.
    pub(crate) fn string_of(&mut self, length: u64) -> Result<String, String> {
        let bytes = self.take(length)?;
        String::from_utf8(bytes.to_vec()).map_err(|_| "refs pack: a string is not UTF-8".into())
    }

    fn end(&self) -> Result<(), String> {
        match self.at == self.bytes.len() {
            true => Ok(()),
            false => Err("refs pack: a section has bytes after its last value".into()),
        }
    }
}

pub(crate) fn int(out: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        out.push(value as u8 | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

pub(crate) fn string(out: &mut Vec<u8>, s: &str) {
    int(out, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

/// `at` as a zigzag difference from `last`, which becomes `at`.
fn delta(last: &mut u32, at: u32) -> u64 {
    let difference = i64::from(at) - i64::from(*last);
    *last = at;
    ((difference << 1) ^ (difference >> 63)) as u64
}

fn undelta(last: &mut u32, zigzag: u64) -> Result<u32, String> {
    let difference = (zigzag >> 1) as i64 ^ -((zigzag & 1) as i64);
    let at = i64::from(*last).checked_add(difference).map(u32::try_from);
    *last = at
        .and_then(Result::ok)
        .ok_or("refs pack: a line out of range")?;
    Ok(*last)
}

fn u32_of(value: u64) -> Result<u32, String> {
    u32::try_from(value).map_err(|_| format!("refs pack: {value} does not fit 32 bits"))
}

fn tag<T: PartialEq>(all: &[T], value: &T) -> u8 {
    all.iter()
        .position(|v| v == value)
        .expect("every value has a tag") as u8
}

fn opt_tag<T: PartialEq>(all: &[T], value: Option<&T>) -> u8 {
    value.map_or(0, |value| tag(all, value) + 1)
}

fn untag<T: Copy>(all: &[T], tag: u8, what: &str) -> Result<T, String> {
    let found = all.get(usize::from(tag)).copied();
    found.ok_or_else(|| format!("refs pack: {what} tag {tag} is unknown"))
}

/// An opt tag: 0 for absent, otherwise a tag plus one.
fn untag_opt<T: Copy>(all: &[T], tag: u8) -> Result<Option<T>, String> {
    tag.checked_sub(1)
        .map(|tag| untag(all, tag, "value"))
        .transpose()
}

#[cfg(test)]
mod tests;
