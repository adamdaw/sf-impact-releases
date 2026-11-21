//! The rows of `delta.jsonl`. `schema/delta.schema.json` is the contract;
//! these types are a convenience for the tools written in Rust and must
//! match it.

use crate::changed::Side;
use crate::lines::Hunk;
use serde::{Deserialize, Serialize};

/// One line of `delta.jsonl`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "row", rename_all = "snake_case")]
pub enum Row {
    Commit(Commit),
    File(File),
    Blob(Blob),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "row", rename_all = "snake_case")]
enum WireRow {
    Commit(Commit),
    File(File),
    Blob(Blob),
}

/// One of the two commits that were compared.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Commit {
    pub side: Side,
    /// The commit, by its full id.
    pub id: String,
    /// The blob that is `sfdx-project.json` at the root in this commit.
    pub project_file: Option<String>,
    /// The blob that is `sf-impact.json` at the root in this commit.
    pub config_file: Option<String>,
}

/// One file that is not the same in the two commits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub status: DeltaStatus,
    /// The file at base; `None` for one that is added.
    pub base: Option<Version>,
    /// The file at head; `None` for one that is deleted.
    pub head: Option<Version>,
    /// The lines that differ, where both versions are files of text.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hunks: Vec<Hunk>,
    /// Of a file whose path the base does not have: the topmost directory
    /// on that path that the base does not have either.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_directory: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaStatus {
    /// At head only.
    Added,
    /// At base only.
    Deleted,
    /// At the same path in both, as another blob or with another mode.
    Modified,
    /// The same blob under another path.
    Moved,
}

/// A file as one commit has it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Version {
    /// Relative to the root, with `/` separators; `../` in front of a file
    /// of the repository that is outside the root.
    pub file: String,
    /// The blob; for a submodule, the commit it is at.
    pub id: String,
    /// The mode of the tree entry: `100644`, `100755`, `120000` for a
    /// symbolic link, `160000` for a submodule.
    pub mode: String,
    /// True for a regular file that is not text.
    /// A file is text if and only if it is valid UTF-8, or UTF-16 (LE or BE) with a BOM that decodes without error, and the decoded content contains no NUL (U+0000).
    pub binary: bool,
}

impl Version {
    /// A file with content of its own: not a symbolic link or a submodule.
    pub fn is_regular(&self) -> bool {
        self.mode.starts_with("100")
    }
}

/// The content of a blob that is text.
/// A file is text if and only if it is valid UTF-8, or UTF-16 (LE or BE) with a BOM that decodes without error, and the decoded content contains no NUL (U+0000).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Blob {
    pub id: String,
    pub text: String,
}

impl<'de> Deserialize<'de> for Row {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let value = serde_json::Value::deserialize(d)?;
        let wire: WireRow = crate::contract::decode(&value).map_err(D::Error::custom)?;
        let row = match wire {
            WireRow::Commit(v) => Row::Commit(v),
            WireRow::File(v) => Row::File(v),
            WireRow::Blob(v) => Row::Blob(v),
        };
        row.check().map_err(D::Error::custom)?;
        Ok(row)
    }
}
impl Row {
    fn check(&self) -> Result<(), String> {
        use crate::contract::{id, nonempty, require};
        match self {
            Self::Commit(v) => {
                id(&v.id)?;
                for value in [&v.project_file, &v.config_file].into_iter().flatten() {
                    id(value)?;
                }
                Ok(())
            }
            Self::Blob(v) => id(&v.id),
            Self::File(v) => {
                let (base, head) = match v.status {
                    DeltaStatus::Added => (false, true),
                    DeltaStatus::Deleted => (true, false),
                    DeltaStatus::Modified | DeltaStatus::Moved => (true, true),
                };
                require(
                    v.base.is_some() == base && v.head.is_some() == head,
                    &format!("{:?} status disagrees with base/head", v.status).to_lowercase(),
                )?;
                for version in [&v.base, &v.head].into_iter().flatten() {
                    nonempty(&[&version.file])?;
                    id(&version.id)?;
                    require(version.mode.chars().count() >= 6, "invalid file mode")?;
                }
                if let Some(path) = &v.new_directory {
                    nonempty(&[path])?;
                }
                Ok(())
            }
        }
    }
}
