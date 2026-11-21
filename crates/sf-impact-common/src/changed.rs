//! The rows of `changed.jsonl`. `schema/changed.schema.json` is the
//! contract; these types are a convenience for the tools written in Rust
//! and must match it.

pub use crate::lines::Hunk;
use crate::rows::{DeclType, RefKind, ReferencedType, Resolution};
use serde::{Deserialize, Serialize};

/// One line of `changed.jsonl`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "row", rename_all = "snake_case")]
pub enum Row {
    File(File),
    Symbol(Symbol),
    Touched(Touched),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "row", rename_all = "snake_case")]
enum WireRow {
    File(File),
    Symbol(Symbol),
    Touched(Touched),
}

/// One file the diff changes, whatever became of it. Every changed file
/// has exactly one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    /// The path at head; of a deleted file, the path at base.
    pub file: String,
    /// The path at base of a file that moved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_file: Option<String>,
    pub status: ChangedStatus,
    /// The content changed and git has no lines for the change.
    #[serde(default, skip_serializing_if = "is_false")]
    pub binary: bool,
    /// The scope directory the file is under; `None` if it is out of scope.
    pub package: Option<String>,
    pub confidence: Confidence,
    /// Why the confidence is partial; empty where it is full.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<Gap>,
}

/// Unlike the public nullable package, the wire records whether its key was
/// supplied. Missing and null must keep their distinct contract meanings.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireFile {
    file: String,
    #[serde(default, deserialize_with = "crate::contract::previous_file")]
    previous_file: Option<String>,
    status: ChangedStatus,
    #[serde(default, deserialize_with = "binary")]
    binary: bool,
    #[serde(default, deserialize_with = "crate::contract::present_nullable")]
    package: Option<Option<String>>,
    confidence: Confidence,
    #[serde(default, deserialize_with = "crate::contract::gaps")]
    gaps: Vec<Gap>,
}
impl<'de> Deserialize<'de> for File {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let wire = WireFile::deserialize(d)?;
        let package = wire
            .package
            .ok_or_else(|| D::Error::custom("`package` is missing, unknown or invalid"))?;
        Ok(Self {
            file: wire.file,
            previous_file: wire.previous_file,
            status: wire.status,
            binary: wire.binary,
            package,
            confidence: wire.confidence,
            gaps: wire.gaps,
        })
    }
}

fn binary<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let value = bool::deserialize(d)?;
    if !value {
        return Err(serde::de::Error::custom(
            "`binary` is missing, unknown or invalid",
        ));
    }
    Ok(value)
}

fn is_false(flag: &bool) -> bool {
    !flag
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangedStatus {
    Added,
    Deleted,
    Modified,
    /// The same content under another path.
    Moved,
    /// The content is the same and the file mode is not.
    ModeChanged,
    /// A file became a symbolic link or a submodule, or the reverse.
    TypeChanged,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Full,
    Partial,
}

/// A way in which the symbol rows of a changed file may not be all that
/// its change touches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gap {
    /// A metadata component of a type that is not modelled.
    UnmodelledFile,
    /// Nothing that the model knows is declared in the file.
    NoDeclaration,
    /// A version of the file did not parse cleanly.
    ParseError,
    /// A changed entry, or a changed Flow, holds an element that was not read.
    UnsupportedElement,
    /// The project file declares other scope directories than it did:
    /// files that did not change are in scope, out of it, or in another
    /// package, and have no rows.
    ScopeChanged,
}

/// A symbol the diff adds, modifies or removes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Symbol {
    pub symbol: String,
    #[serde(rename = "type")]
    pub kind: DeclType,
    pub package: String,
    /// The changed file that is the evidence: at base for `Removed`.
    pub file: String,
    pub change: Change,
    pub reason: Reason,
    /// The lines of the declaration in `file`, where it has some there.
    #[serde(
        default,
        deserialize_with = "crate::contract::start",
        skip_serializing_if = "Option::is_none"
    )]
    pub start: Option<u32>,
    #[serde(
        default,
        deserialize_with = "crate::contract::end",
        skip_serializing_if = "Option::is_none"
    )]
    pub end: Option<u32>,
    /// The file that declared the symbol at base, where that is another
    /// file than `file`, deleted while this one was added: the base lines
    /// of `hunks` are lines of it.
    #[serde(
        default,
        deserialize_with = "crate::contract::previous_file",
        skip_serializing_if = "Option::is_none"
    )]
    pub previous_file: Option<String>,
    /// The hunks of `file` that mark the symbol.
    #[serde(
        default,
        deserialize_with = "crate::contract::hunks",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub hunks: Vec<Hunk>,
    /// At head another declaration, or for `Removed` any declaration, has
    /// this name and is the same kind of thing.
    pub collision: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Added,
    Modified,
    Removed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Changed lines fall inside the declaration.
    Hunk,
    /// The file as a whole marks the symbol.
    WholeFile,
    /// Declared in the base version of a file that is still there.
    Removed,
}

/// A name in a changed entry of a permission set, a profile or a custom
/// metadata record: a grant or a binding that the diff touches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Touched {
    /// The permission set, profile or record.
    pub referrer: String,
    pub side: Side,
    pub kind: RefKind,
    pub referenced: String,
    #[serde(
        default,
        deserialize_with = "crate::contract::referenced_type",
        skip_serializing_if = "Option::is_none"
    )]
    pub referenced_type: Option<ReferencedType>,
    pub resolution: Resolution,
    /// Every declaration the name may be, where it is more than one.
    #[serde(
        default,
        deserialize_with = "crate::contract::candidates_list",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub candidates: Vec<String>,
    pub package: String,
    pub file: String,
    pub line: u32,
}

/// Which version of the file a name was read from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Base,
    Head,
}

impl<'de> Deserialize<'de> for Row {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let raw = crate::contract::raw_row(d)?;
        let wire: WireRow = serde_json::from_str(raw.get())
            .map_err(|e| D::Error::custom(crate::contract::row_error(raw.get(), e)))?;
        let row = match wire {
            WireRow::File(v) => Row::File(v),
            WireRow::Symbol(v) => Row::Symbol(v),
            WireRow::Touched(v) => Row::Touched(v),
        };
        row.check().map_err(D::Error::custom)?;
        Ok(row)
    }
}
impl Row {
    fn check(&self) -> Result<(), String> {
        match self {
            Self::File(v) => v.check(),
            Self::Symbol(v) => v.check(),
            Self::Touched(v) => v.check(),
        }
    }
}
impl File {
    fn check(&self) -> Result<(), String> {
        use crate::contract::{nonempty, require};
        let v = self;
        nonempty(&[&v.file])?;
        require(
            (v.status == ChangedStatus::Moved) == v.previous_file.is_some(),
            "previous_file belongs to moved files",
        )?;
        if let Some(path) = &v.previous_file {
            nonempty(&[path])?;
        }
        require(
            (v.confidence == Confidence::Partial) == !v.gaps.is_empty(),
            "confidence disagrees with gaps",
        )?;
        require(
            (1..v.gaps.len()).all(|i| !v.gaps[..i].contains(&v.gaps[i])),
            "gaps repeat",
        )
    }
}

impl Symbol {
    fn check(&self) -> Result<(), String> {
        use crate::contract::{nonempty, require};
        let v = self;
        nonempty(&[&v.symbol, &v.file])?;
        if let Some(path) = &v.previous_file {
            nonempty(&[path])?;
        }
        require(
            v.start.is_some() == v.end.is_some() && v.start != Some(0) && v.end != Some(0),
            "invalid declaration span",
        )?;
        if v.reason == Reason::Hunk {
            require(
                !v.hunks.is_empty() && v.start.is_some(),
                "hunk requires hunks and span",
            )?;
        }
        if v.reason == Reason::Removed {
            require(
                v.change == Change::Removed && v.start.is_some() && v.hunks.is_empty(),
                "removed requires removed change and span without hunks",
            )?;
        }
        Ok(())
    }
}

impl Touched {
    fn check(&self) -> Result<(), String> {
        use crate::contract::{candidates, nonempty, require};
        let v = self;
        nonempty(&[&v.referrer, &v.referenced, &v.file])?;
        require(v.line > 0, "line must be positive")?;
        require(
            matches!(
                v.kind,
                RefKind::Grant | RefKind::Cmdt | RefKind::CmdtField | RefKind::CmdtType
            ),
            "invalid touched kind",
        )?;
        if !v.candidates.is_empty() {
            candidates(&v.candidates)?;
        }
        Ok(())
    }
}
