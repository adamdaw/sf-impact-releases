//! The row that `refs.jsonl`, `delta.jsonl` and `changed.jsonl` each start
//! with: what wrote the file, and of what. A file that is kept, in a cache
//! or anywhere else, is used again only by a run that it is for, and this
//! row is how the tool that reads it knows. It holds no time, no name of a
//! machine and no path of one, so the same inputs still give the same bytes.
//!
//! A row is read in one place, [`Reader::read`], and is held there to all
//! that its schema says of it, not only to its shape: a tool that has read
//! a row has a valid one, and writes no part of an invalid one into its own.
//!
//! `schema/*.schema.json` is the contract; these types are a convenience
//! for the tools written in Rust and must match it.

use crate::changed::Confidence;
use crate::config::Config;
use crate::rows::{ScopeSource, fold};
use crate::scope;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::BufRead;

/// One of the three files that start with a provenance row.
pub struct File {
    pub name: &'static str,
    /// The tool that writes it.
    pub tool: &'static str,
    /// The version of its schema that these tools write and read. A file
    /// without the row is of the format before version 1.
    pub schema: u32,
}

/// Refs: the pack that sf-impact-extract writes, whose line 1 is this row
/// (`docs/pack.md`). Schema 2 is the first in a pack; refs.jsonl, its
/// projection, says the same. The file is named by its contract, refs.jsonl,
/// in what the tools say of it.
pub const REFS: File = File {
    name: "refs.jsonl",
    tool: "sf-impact-extract",
    schema: 2,
};
pub const DELTA: File = File {
    name: "delta.jsonl",
    tool: "sf-impact-delta",
    schema: 1,
};
pub const CHANGED: File = File {
    name: "changed.jsonl",
    tool: "sf-impact-changed",
    schema: 1,
};

/// The first row of a file, as a line of it.
pub fn line(provenance: impl Serialize) -> String {
    serde_json::to_string(&Row::Provenance(provenance))
        .expect("a row has only string keys and plain values")
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "row", rename_all = "snake_case")]
enum Row<P> {
    Provenance(P),
}

/// The first row of one of the three files.
pub trait Provenance: Serialize + DeserializeOwned {
    /// The file that it is the first row of.
    const FILE: File;

    fn writer(&self) -> &Writer;

    /// Every field that the schema requires of the row that is `given`,
    /// by its path from the row, a field before the fields in it. What is
    /// required of a tree depends on what it says it is.
    fn required(given: &Value) -> Vec<String>;

    /// The first field of the row that is not what the schema says it
    /// is, with why, if there is one.
    fn invalid(&self) -> Checked;
}

/// The fields that the schema of refs.jsonl requires of its first row,
/// each after `at`, which is where the row is: nowhere, or in another.
fn required_of_refs(at: &str, given: &Value) -> Vec<String> {
    let of_tree: &[&str] = match given["tree"]["state"].as_str() {
        Some("commit") => &["tree.commit", "tree.tree"],
        _ => &[],
    };
    let fields = ["tool", "version", "schema", "tree", "tree.state"].into_iter();
    let fields = fields.chain(of_tree.iter().copied()).chain(["root"]);
    let fields = fields.chain(REQUIRED_OF_SCOPE).chain(["config"]);
    fields.map(|field| format!("{at}{field}")).collect()
}

/// The fields that the schema of delta.jsonl requires of its first row,
/// each after `at`. Those of changed.jsonl start with the same.
fn required_of_delta(at: &str) -> Vec<String> {
    let fields = [
        "tool",
        "version",
        "schema",
        "base",
        "head",
        "head_tree",
        "root",
    ]
    .into_iter();
    let fields = fields.chain(REQUIRED_OF_SCOPE).chain(["config"]);
    fields.map(|field| format!("{at}{field}")).collect()
}

const REQUIRED_OF_SCOPE: [&str; 3] = [
    "scope",
    "scope.packageDirectories",
    "scope.unpackagedMetadata",
];

/// Whether an object in `given` lacks the field at a path. A field that
/// is null is there. A field of what is no object is not looked for: what
/// that is, null or otherwise, is for its own rule.
fn is_missing(given: &Value, field: &str) -> bool {
    let mut at = given;
    for name in field.split('.') {
        let Some(object) = at.as_object() else {
            return false;
        };
        match object.get(name) {
            Some(value) => at = value,
            None => return true,
        }
    }
    false
}

/// A field of a row that is not what the schema says it is.
#[derive(Debug, PartialEq, Eq)]
pub struct Invalid {
    /// Where the field is, from the row: `inputs.refs.tree.commit`.
    field: String,
    /// What is wrong with it, to follow its name.
    why: String,
}

pub type Checked = Result<(), Invalid>;

impl Invalid {
    fn new(field: &str, why: impl Into<String>) -> Invalid {
        Invalid {
            field: field.to_string(),
            why: why.into(),
        }
    }

    /// The same, of a row that is the field `parent` of another.
    fn within(self, parent: &str) -> Invalid {
        Invalid {
            field: format!("{parent}.{}", self.field),
            ..self
        }
    }
}

fn valid_if(valid: bool, field: &str, why: impl FnOnce() -> String) -> Checked {
    match valid {
        true => Ok(()),
        false => Err(Invalid::new(field, why())),
    }
}

/// The tool that reads a file.
pub struct Reader {
    pub tool: &'static str,
    pub version: &'static str,
}

impl Reader {
    /// Decode a complete JSON document without silently discarding fields or
    /// accepting a missing nullable field. As with provenance rows, its wire
    /// types must serialize optional fields only when they were present.
    pub fn document<T: Serialize + DeserializeOwned>(&self, given: &Value) -> Result<T, String> {
        crate::contract::decode(given)
    }

    /// Reads the first line of a file, which is its provenance row, and
    /// leaves the rest unread. The row is refused, in one line, unless it
    /// is all that its schema says: of the version of the schema that this
    /// tool knows, with no field that the schema does not have, and every
    /// field valid and consistent with the others. A file without the row
    /// was written before there was one, and nothing says what it is of.
    /// `fix` is what to run to have a file that can be read.
    pub fn read<P: Provenance>(&self, input: &mut impl BufRead, fix: &str) -> Result<P, String> {
        let File { name, tool, schema } = P::FILE;
        let Reader {
            tool: reader,
            version,
        } = self;
        let mut first = String::new();
        let read = input.read_line(&mut first);
        read.map_err(|e| format!("{name}: line 1: {e}"))?;
        let given = serde_json::from_str::<Value>(&first).unwrap_or_default();
        if given["row"] != "provenance" {
            return Err(format!(
                "{name} does not start with a provenance row, which it has as of schema version \
                 {schema}: an older {tool} wrote it, or it is not {name}; re-run {tool}"
            ));
        }
        // Before the rest: a row of another version may not have the
        // fields of this one.
        let of_schema = given["schema"].as_u64();
        if let Some(found) = of_schema.filter(|found| *found != u64::from(schema)) {
            return Err(format!(
                "{name} cannot be read: its `schema` is version {found} and {reader} {version} \
                 reads version {schema}; {fix}, with the {tool} that goes with this {reader}"
            ));
        }
        // What is required is there, whatever it is: a field that may
        // be null says so, and is not left out to say so.
        let required = P::required(&given).into_iter();
        if let Some(field) = required.into_iter().find(|field| is_missing(&given, field)) {
            return Err(format!(
                "{name}: its provenance row is not valid: `{field}` is missing, and schema \
                 version {schema} requires it; {fix}"
            ));
        }
        let provenance: P = match serde_json::from_str(&first) {
            Ok(Row::Provenance(provenance)) => provenance,
            Err(e) => return Err(format!("{name}: line 1: {e}")),
        };
        // What the types here did not take from the row, the schema does
        // not have in it.
        let kept = serde_json::to_value(Row::Provenance(&provenance)).expect("a row is plain");
        let unknown = unread(&given, &kept, "").map(|field| {
            let why = format!("is not what schema version {schema} has there");
            Invalid::new(&field, why)
        });
        let not_valid = |Invalid { field, why }| {
            format!("{name}: its provenance row is not valid: `{field}` {why}; {fix}")
        };
        // What wrote it, before what it wrote.
        let writer = provenance.writer();
        unknown.map_or(Ok(()), Err).map_err(not_valid)?;
        writer.invalid(&P::FILE).map_err(not_valid)?;
        provenance.invalid().map_err(not_valid)?;
        Ok(provenance)
    }
}

/// The first field of `given`, by its path, that `kept` does not have as
/// it is given.
pub(crate) fn unread(given: &Value, kept: &Value, at: &str) -> Option<String> {
    let (Value::Object(given), Value::Object(kept)) = (given, kept) else {
        return (given != kept).then(|| at.to_string());
    };
    given.iter().find_map(|(name, given)| {
        let at = match at {
            "" => name.clone(),
            _ => format!("{at}.{name}"),
        };
        match kept.get(name) {
            Some(kept) => unread(given, kept, &at),
            None => Some(at),
        }
    })
}

/// What wrote a file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Writer {
    pub tool: String,
    pub version: String,
    /// The version of the schema that the file is valid against.
    pub schema: u32,
}

impl Writer {
    /// The tool that writes `file`, at `version`.
    pub fn of(file: &File, version: &str) -> Writer {
        Writer {
            tool: file.tool.to_string(),
            version: version.to_string(),
            schema: file.schema,
        }
    }

    fn invalid(&self, file: &File) -> Checked {
        valid_if(!self.tool.is_empty(), "tool", || {
            "is empty: it names no tool".into()
        })?;
        // Three numbers, and whatever follows them after a `-` or a `+`.
        let numbers = self.version.split(['-', '+']).next().unwrap_or_default();
        let numbers: Vec<&str> = numbers.split('.').collect();
        let is_number =
            |part: &&str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
        valid_if(
            numbers.len() == 3 && numbers.iter().all(is_number),
            "version",
            || format!("is `{}`, which is no version as `0.1.0` is", self.version),
        )?;
        valid_if(self.schema == file.schema, "schema", || {
            format!("is version {} and not {}", self.schema, file.schema)
        })
    }
}

fn is_hex(text: &str) -> bool {
    text.bytes()
        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// The id of a git object, in full.
fn id(field: &str, id: &str) -> Checked {
    valid_if(matches!(id.len(), 40 | 64) && is_hex(id), field, || {
        format!("is `{id}`, which is not the id of a git object: 40 or 64 hexadecimal digits")
    })
}

/// Where a project is in its repository.
fn root(field: &str, root: &str) -> Checked {
    let is_normal = !root.is_empty() && scope::normalise(root).as_deref() == Some(root);
    valid_if(root == "." || is_normal, field, || {
        format!(
            "is `{root}`, which is not a path from the top of a repository as these are \
             written: `.` for the top, `/` between its parts, and nothing that leaves it"
        )
    })
}

fn config(field: &str, config: &str) -> Checked {
    valid_if(config.len() == 16 && is_hex(config), field, || {
        format!("is `{config}`, which is not the hash of a config: 16 hexadecimal digits")
    })
}

/// The first row of `refs.jsonl`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Refs {
    #[serde(flatten)]
    pub writer: Writer,
    pub tree: Tree,
    /// Where the project is in its repository, from the top of it: `.` at
    /// the top. `None` only for a tree that is `Unknown`: where no commit
    /// could be read, the place may not be known either.
    pub root: Option<String>,
    pub scope: Scope,
    /// [`config_hash`] of the config that the tree was read with.
    pub config: String,
}

/// The tree that refs were extracted from: what is known of the content
/// that was read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Tree {
    /// All that was read is as this commit has it.
    Commit {
        commit: String,
        /// The tree of the commit: another commit with this tree has the
        /// same content.
        tree: String,
    },
    /// A repository, and what was read is not what its commit has.
    Dirty,
    /// No commit could be read: no repository, or no git.
    Unknown,
}

impl Provenance for Refs {
    const FILE: File = REFS;

    fn writer(&self) -> &Writer {
        &self.writer
    }

    fn required(given: &Value) -> Vec<String> {
        required_of_refs("", given)
    }

    fn invalid(&self) -> Checked {
        self.writer.invalid(&REFS)?;
        match &self.tree {
            Tree::Commit { commit, tree } => {
                id("tree.commit", commit)?;
                id("tree.tree", tree)?;
            }
            Tree::Dirty | Tree::Unknown => {}
        }
        match (&self.tree, &self.root) {
            (_, Some(at)) => root("root", at)?,
            (Tree::Unknown, None) => {}
            (_, None) => {
                let why = "is null, and only a tree that is `unknown` can be at no place that \
                           is known";
                return Err(Invalid::new("root", why));
            }
        }
        self.scope.invalid()?;
        config("config", &self.config)
    }
}

/// The directories that a project file declares, as paths from the root.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    #[serde(rename = "packageDirectories")]
    pub package_directories: Vec<String>,
    #[serde(rename = "unpackagedMetadata")]
    pub unpackaged_metadata: Vec<String>,
}

impl Scope {
    /// What the content of `sfdx-project.json` declares, normalised:
    /// by the key that names it, sorted, each once. A path that leaves the
    /// root holds nothing in scope and is in neither list; a directory that
    /// both keys name is a package directory.
    pub fn declared(project: &[u8]) -> Result<Scope, serde_json::Error> {
        let mut scope = Scope::default();
        for (path, source) in scope::declared(project)? {
            let Some(directory) = scope::normalise(&path) else {
                continue;
            };
            match source {
                ScopeSource::PackageDirectories => scope.package_directories.push(directory),
                ScopeSource::UnpackagedMetadata => scope.unpackaged_metadata.push(directory),
            }
        }
        for directories in [
            &mut scope.package_directories,
            &mut scope.unpackaged_metadata,
        ] {
            directories.sort();
            directories.dedup();
        }
        let packages = &scope.package_directories;
        scope
            .unpackaged_metadata
            .retain(|directory| !packages.contains(directory));
        Ok(scope)
    }

    /// Every scope directory.
    pub fn directories(&self) -> Vec<String> {
        let both = self
            .package_directories
            .iter()
            .chain(&self.unpackaged_metadata);
        both.cloned().collect()
    }

    /// Whether the lists are as [`Scope::declared`] makes them.
    fn invalid(&self) -> Checked {
        let lists = [&self.package_directories, &self.unpackaged_metadata];
        let is_normal =
            |directory: &String| scope::normalise(directory).as_deref() == Some(directory.as_str());
        let is_as_declared = lists.iter().all(|list| {
            list.iter().all(is_normal) && list.windows(2).all(|pair| pair[0] < pair[1])
        });
        let in_both = |directory: &String| self.package_directories.contains(directory);
        valid_if(
            is_as_declared && !self.unpackaged_metadata.iter().any(in_both),
            "scope",
            || {
                "is not as the directories of a project file are written here: paths from the \
                 root that do not leave it, each list sorted and with each path once, and no \
                 path in both"
                    .into()
            },
        )
    }
}

/// A hash of the config as it takes effect, as 16 hexadecimal digits: of
/// the text `{"namespaces":[..],"textualAllowList":[..],"textualMinLength":n}`,
/// with no white space, the names in ASCII lower case, sorted, each once.
/// A config file that only says what the defaults are hashes as none does.
///
/// The hash is FNV-1a, 64 bits. It tells two configs apart; it is no
/// protection against a file that was made to deceive, and nor is the rest
/// of the row.
pub fn config_hash(config: &Config) -> String {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Effective {
        namespaces: Vec<String>,
        textual_allow_list: Vec<String>,
        textual_min_length: usize,
    }
    let names = |names: &[String]| {
        let mut names: Vec<String> = names.iter().map(|name| fold(name)).collect();
        names.sort();
        names.dedup();
        names
    };
    let effective = Effective {
        namespaces: names(&config.namespaces),
        textual_allow_list: names(&config.textual_allow_list),
        textual_min_length: config.textual_min_length,
    };
    let text = serde_json::to_string(&effective).expect("a config has plain values");
    format!("{:016x}", fnv_1a_64(text.as_bytes()))
}

fn fnv_1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The first row of `delta.jsonl`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delta {
    #[serde(flatten)]
    pub writer: Writer,
    /// The commit compared from.
    pub base: String,
    /// The commit compared to.
    pub head: String,
    /// The tree of the head.
    pub head_tree: String,
    /// Where the project is in the repository, from the top of it: `.` at
    /// the top.
    pub root: String,
    /// What the project file of the head declares; `None` if the head has
    /// none that can be read.
    pub scope: Option<Scope>,
    /// [`config_hash`] of the config of the head; `None` if the head has a
    /// config file that cannot be read.
    pub config: Option<String>,
}

impl Provenance for Delta {
    const FILE: File = DELTA;

    fn writer(&self) -> &Writer {
        &self.writer
    }

    fn required(_: &Value) -> Vec<String> {
        required_of_delta("")
    }

    fn invalid(&self) -> Checked {
        self.writer.invalid(&DELTA)?;
        id("base", &self.base)?;
        id("head", &self.head)?;
        id("head_tree", &self.head_tree)?;
        root("root", &self.root)?;
        self.scope.as_ref().map_or(Ok(()), Scope::invalid)?;
        let hash = self.config.as_deref();
        hash.map_or(Ok(()), |hash| config("config", hash))
    }
}

/// The first row of `changed.jsonl`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Changed {
    #[serde(flatten)]
    pub writer: Writer,
    pub base: String,
    pub head: String,
    pub head_tree: String,
    pub root: String,
    pub scope: Scope,
    pub config: String,
    /// `Partial` where the inputs were not verified to go together.
    pub confidence: Confidence,
    /// Why the confidence is partial; empty where it is full.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gaps: Vec<Gap>,
    pub inputs: Inputs,
}

/// The first rows of the two files that `changed.jsonl` was made of.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inputs {
    pub refs: Refs,
    pub delta: Delta,
}

/// A way in which a whole `changed.jsonl` may not be what its head changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gap {
    /// The refs were of a tree that is `dirty` or `unknown`, and were
    /// accepted as they were.
    UnverifiedRefs,
}

impl Provenance for Changed {
    const FILE: File = CHANGED;

    fn writer(&self) -> &Writer {
        &self.writer
    }

    fn required(given: &Value) -> Vec<String> {
        let mut required = required_of_delta("");
        required.extend(["confidence", "inputs", "inputs.refs"].map(String::from));
        required.extend(required_of_refs("inputs.refs.", &given["inputs"]["refs"]));
        required.push("inputs.delta".to_string());
        required.extend(required_of_delta("inputs.delta."));
        required
    }

    /// The row is valid if it is the row that the tool that wrote it
    /// writes for the two rows in `inputs`, which are held to the rules
    /// that they are read by.
    fn invalid(&self) -> Checked {
        self.writer.invalid(&CHANGED)?;
        id("base", &self.base)?;
        id("head", &self.head)?;
        id("head_tree", &self.head_tree)?;
        root("root", &self.root)?;
        self.scope.invalid()?;
        config("config", &self.config)?;
        let is_full = self.confidence == Confidence::Full;
        valid_if(is_full == self.gaps.is_empty(), "gaps", || match is_full {
            true => "says why the confidence is partial, and `confidence` is `full`".into(),
            false => "is absent, and `confidence` is `partial`: nothing says why".into(),
        })?;
        valid_if(self.gaps.len() <= 1, "gaps", || {
            "has a reason more than once".into()
        })?;

        let Inputs { refs, delta } = &self.inputs;
        let within = |parent: &'static str| move |invalid: Invalid| invalid.within(parent);
        refs.invalid().map_err(within("inputs.refs"))?;
        delta.invalid().map_err(within("inputs.delta"))?;
        let same = |field: &str, is_same: bool| {
            valid_if(is_same, &format!("inputs.delta.{field}"), || {
                format!("is not the `{field}` of the row, which is made of it")
            })
        };
        same("base", delta.base == self.base)?;
        same("head", delta.head == self.head)?;
        same("head_tree", delta.head_tree == self.head_tree)?;
        same("root", delta.root == self.root)?;
        same("scope", delta.scope.as_ref() == Some(&self.scope))?;
        same("config", delta.config.as_ref() == Some(&self.config))?;
        let head = Head {
            file: DELTA.name,
            commit: &self.head,
            tree: &self.head_tree,
            root: &self.root,
            scope: &self.scope,
            config: &self.config,
        };
        let verified = refs.fit(&head).map_err(|differs| {
            let Differs { field, refs, head } = differs;
            let why = format!("has the `{field}` {refs}, and the head of the row has {head}");
            Invalid::new("inputs.refs", why)
        })?;
        valid_if(verified == is_full, "inputs.refs", || match verified {
            true => "is of the head of the row, and `confidence` is `partial`".into(),
            false => "is of a tree that is no commit's, and `confidence` is `full`".into(),
        })
    }
}

/// Provenance carried inside the single impact.json document. No source tree
/// is needed to verify the chain again at the rendering boundary.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Impact {
    #[serde(flatten)]
    pub writer: Writer,
    pub inputs: ImpactInputs,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImpactInputs {
    pub refs: Refs,
    pub changed: Option<Changed>,
}
pub const IMPACT: File = File {
    name: "impact.json",
    tool: "sf-impact-tests",
    schema: 1,
};
impl Provenance for Impact {
    const FILE: File = IMPACT;
    fn writer(&self) -> &Writer {
        &self.writer
    }
    fn required(given: &Value) -> Vec<String> {
        let mut fields = [
            "tool",
            "version",
            "schema",
            "inputs",
            "inputs.refs",
            "inputs.changed",
        ]
        .map(String::from)
        .to_vec();
        fields.extend(required_of_refs("inputs.refs.", &given["inputs"]["refs"]));
        if !given["inputs"]["changed"].is_null() {
            fields.extend(
                Changed::required(&given["inputs"]["changed"])
                    .into_iter()
                    .map(|f| format!("inputs.changed.{f}")),
            );
        }
        fields
    }
    fn invalid(&self) -> Checked {
        self.writer.invalid(&IMPACT)?;
        let refs = &self.inputs.refs;
        refs.invalid().map_err(|e| e.within("inputs.refs"))?;
        if let Some(changed) = &self.inputs.changed {
            changed.invalid().map_err(|e| e.within("inputs.changed"))?;
            let head = Head {
                file: CHANGED.name,
                commit: &changed.head,
                tree: &changed.head_tree,
                root: &changed.root,
                scope: &changed.scope,
                config: &changed.config,
            };
            refs.fit(&head).map_err(|d| {
                Invalid::new(
                    &format!("inputs.refs.{}", d.field),
                    format!(
                        "is {} in refs.jsonl and {} in changed.jsonl",
                        d.refs, d.head
                    ),
                )
            })?;
        }
        Ok(())
    }
}

/// The head that refs have to be of, as the file read with them has it.
pub struct Head<'a> {
    /// The file that says so: `delta.jsonl` or `changed.jsonl`.
    pub file: &'a str,
    pub commit: &'a str,
    pub tree: &'a str,
    pub root: &'a str,
    pub scope: &'a Scope,
    pub config: &'a str,
}

/// A field of refs that is not what the head has for it.
struct Differs {
    field: &'static str,
    refs: String,
    head: String,
}

impl Refs {
    /// Whether these refs are known to be of `head`: of its commit, or of
    /// its tree under another commit, of the same project of the
    /// repository, with its scope and its config. `Ok(false)` for refs of
    /// a tree that is `dirty` or `unknown`, of which that cannot be known,
    /// if nothing that is known of them differs.
    fn fit(&self, head: &Head) -> Result<bool, Differs> {
        let differs = |field, refs: &str, head: &str| {
            let (refs, head) = (refs.to_string(), head.to_string());
            Err(Differs { field, refs, head })
        };
        let verified = match &self.tree {
            // One commit has one tree.
            Tree::Commit { commit, tree } if commit == head.commit && tree != head.tree => {
                return differs("tree", tree, head.tree);
            }
            Tree::Commit { commit, tree } if commit != head.commit && tree != head.tree => {
                return differs("commit", commit, head.commit);
            }
            Tree::Commit { .. } => true,
            Tree::Dirty | Tree::Unknown => false,
        };
        if let Some(root) = self.root.as_deref().filter(|root| *root != head.root) {
            return differs("root", root, head.root);
        }
        if self.scope != *head.scope {
            let shown = |scope: &Scope| serde_json::to_string(scope).expect("a scope is plain");
            return differs("scope", &shown(&self.scope), &shown(head.scope));
        }
        if self.config != head.config {
            return differs("config", &self.config, head.config);
        }
        Ok(verified)
    }

    /// Whether these refs are verified to be of `head`: see [`Refs::fit`].
    /// Refs that are known not to be are refused. Refs of a tree that is
    /// `dirty` or `unknown` cannot be known to be: they are refused too,
    /// unless `accept_unverified` says to use them as they are, and then
    /// the answer is `false`.
    pub fn verify(&self, head: &Head, accept_unverified: bool) -> Result<bool, String> {
        let Head { file, commit, .. } = head;
        let fix = format!("re-run sf-impact-extract at {commit}");
        let verified = self.fit(head).map_err(|differs| {
            let Differs { field, refs, head } = differs;
            format!(
                "refs.jsonl is not of the head of {file}: `{field}` is {refs} in refs.jsonl and \
                 {head} in {file}; {fix}"
            )
        })?;
        if verified || accept_unverified {
            return Ok(verified);
        }
        let (state, why) = match self.tree {
            Tree::Dirty => (
                "dirty",
                "what sf-impact-extract read is not what a commit has",
            ),
            _ => (
                "unknown",
                "sf-impact-extract could read no commit where it ran",
            ),
        };
        Err(format!(
            "refs.jsonl cannot be verified to be of the head of {file}: its tree is `{state}` \
             ({why}) and the head is {commit}; {fix}, or pass --accept-unverified-refs to use it \
             as it is"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hash_is_fnv_1a_of_64_bits() {
        // The published test vectors.
        assert_eq!(fnv_1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv_1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv_1a_64(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn a_config_is_hashed_as_the_text_of_what_takes_effect() {
        let config = Config {
            textual_min_length: 4,
            textual_allow_list: vec!["Money".into(), "money".into()],
            namespaces: vec!["npsp".into(), "Fflib".into()],
        };
        let text =
            r#"{"namespaces":["fflib","npsp"],"textualAllowList":["money"],"textualMinLength":4}"#;

        assert_eq!(
            config_hash(&config),
            format!("{:016x}", fnv_1a_64(text.as_bytes()))
        );
        assert_eq!(config_hash(&Config::default()), "4e7844462778448c");
    }

    #[test]
    fn the_declared_directories_are_normalised_by_key_sorted_and_each_once() {
        let project = br#"{"packageDirectories": [
            {"path": "plugins\\billing", "unpackagedMetadata": {"path": "./app"}},
            {"path": "./app/", "unpackagedMetadata": {"path": "unpackaged"}},
            {"path": "app"},
            {"path": "../elsewhere"},
            {"path": "/absolute"}
        ]}"#;

        let scope = Scope::declared(project).unwrap();

        assert_eq!(scope.package_directories, ["app", "plugins/billing"]);
        // `app` is a package directory, whatever else names it.
        assert_eq!(scope.unpackaged_metadata, ["unpackaged"]);
        assert_eq!(
            scope.directories(),
            ["app", "plugins/billing", "unpackaged"]
        );
        assert_eq!(scope.invalid(), Ok(()));
        assert!(Scope::declared(b"not JSON").is_err());
    }

    const READER: Reader = Reader {
        tool: "sf-impact-changed",
        version: "0.1.0",
    };

    fn refs() -> Refs {
        Refs {
            writer: Writer::of(&REFS, "0.1.0"),
            tree: Tree::Dirty,
            root: Some("packages/billing".into()),
            scope: Scope::default(),
            config: "0".repeat(16),
        }
    }

    #[test]
    fn a_row_is_written_with_what_kind_of_row_it_is_and_read_from_the_first_line_alone() {
        let line = line(refs());

        assert_eq!(
            line,
            r#"{"row":"provenance","tool":"sf-impact-extract","version":"0.1.0","schema":2,"tree":{"state":"dirty"},"root":"packages/billing","scope":{"packageDirectories":[],"unpackagedMetadata":[]},"config":"0000000000000000"}"#
        );
        let mut file = format!("{line}\nthe rest\n");
        let mut input = file.as_bytes();
        assert_eq!(READER.read::<Refs>(&mut input, "re-run"), Ok(refs()));
        assert_eq!(input, b"the rest\n");
        // A row of the file that is not this one, and no row at all.
        file = "{\"row\":\"declaration\"}\n".to_string();
        for old in [file.as_bytes(), b"", b"not JSON\n"] {
            let refused = READER.read::<Refs>(&mut &*old, "re-run").unwrap_err();
            assert!(refused.contains("schema version 2"), "{refused}");
        }
        // The row of another file: it lacks what this one has.
        let delta = r#"{"row":"provenance","tool":"sf-impact-delta","version":"0.1.0","schema":2}"#;
        let refused = READER
            .read::<Refs>(&mut delta.as_bytes(), "re-run")
            .unwrap_err();
        assert!(refused.starts_with("refs.jsonl: "), "{refused}");
        assert!(refused.contains("`tree` is missing"), "{refused}");
    }

    #[test]
    fn a_field_that_the_types_here_do_not_keep_is_found_by_its_path() {
        let given = serde_json::json!({"a": 1, "b": {"c": [1, 2], "d": null}});
        let mut kept = given.clone();

        assert_eq!(unread(&given, &kept, ""), None);
        kept["b"].as_object_mut().unwrap().remove("d");
        assert_eq!(unread(&given, &kept, "").as_deref(), Some("b.d"));
        kept["b"]["c"] = serde_json::json!([1]);
        assert_eq!(unread(&given, &kept, "").as_deref(), Some("b.c"));
        // What is kept and was not given took its default.
        kept = given.clone();
        kept["e"] = serde_json::json!([]);
        assert_eq!(unread(&given, &kept, ""), None);
    }

    #[test]
    fn a_field_that_is_null_is_there_and_one_that_is_left_out_is_missing() {
        let given = serde_json::json!({"root": null, "scope": null, "tree": {"state": "dirty"}});

        assert!(!is_missing(&given, "root"));
        assert!(is_missing(&given, "config"));
        assert!(!is_missing(&given, "tree.state"));
        assert!(is_missing(&given, "tree.commit"));
        // What is in a field that is null is not looked for.
        assert!(!is_missing(&given, "scope.packageDirectories"));
        // A tree is required to say what it says it is, and no more.
        let of_tree = |state: &str| -> Vec<String> {
            let given = serde_json::json!({"tree": {"state": state}});
            let required = required_of_refs("inputs.refs.", &given).into_iter();
            required.filter(|field| field.contains(".tree.")).collect()
        };
        assert_eq!(
            of_tree("commit"),
            [
                "inputs.refs.tree.state",
                "inputs.refs.tree.commit",
                "inputs.refs.tree.tree"
            ]
        );
        assert_eq!(of_tree("unknown"), ["inputs.refs.tree.state"]);
    }

    #[test]
    fn a_tree_is_at_a_place_in_a_repository_unless_it_is_unknown() {
        let invalid = |tree: Tree, root: Option<&str>| {
            let root = root.map(str::to_string);
            let refs = Refs {
                tree,
                root,
                ..refs()
            };
            refs.invalid().err().map(|invalid| invalid.field)
        };

        assert_eq!(invalid(Tree::Dirty, Some(".")), None);
        assert_eq!(invalid(Tree::Unknown, None), None);
        assert_eq!(invalid(Tree::Unknown, Some(".")), None);
        assert_eq!(invalid(Tree::Dirty, None).as_deref(), Some("root"));
        let unknown_at = invalid(Tree::Unknown, Some("/top"));
        assert_eq!(unknown_at.as_deref(), Some("root"));
        for not_a_root in ["", "/top", "a//b", "a/./b", "a/", "..", "C:\\a"] {
            let field = invalid(Tree::Dirty, Some(not_a_root));
            assert_eq!(field.as_deref(), Some("root"), "{not_a_root}");
        }
    }
}
