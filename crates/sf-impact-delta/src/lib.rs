//! The library behind `sf-impact-delta`: what is not the same in two commits of
//! the repository that holds a Salesforce DX project, as the lines of
//! `delta.jsonl`. The rows are defined by `schema/delta.schema.json`.
//!
//! This is the stage that knows git, and it knows nothing of what a file
//! declares. It says which files differ, which of their lines, and what
//! the text of each version is, so that the stage after it reads no
//! repository.

pub mod git;

use git::{Entry, Git};
use sf_impact_common::changed::Side;
use sf_impact_common::config::{CONFIG_FILE, Config};
use sf_impact_common::delta::{Blob, Commit, DeltaStatus, File, Row, Version};
use sf_impact_common::provenance::{self, Scope, Writer};
use sf_impact_common::scope::PROJECT_FILE;
use sf_impact_common::{lines, text};
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// Where the comparison starts.
#[derive(Debug, PartialEq, Eq)]
pub enum Base {
    Commit(String),
    /// The merge base of this revision and the head.
    MergeBaseWith(String),
}

/// The files of a commit, by path from the top of the repository.
type Tree = BTreeMap<String, Entry>;

/// The lines of `delta.jsonl` for the project at `root`: what the file is
/// of, the two commits, the files that differ by path, then the text of
/// their versions by id.
/// `trust_repo` says to read the repository though another user owns it.
pub fn delta(
    root: &Path,
    trust_repo: bool,
    base: &Base,
    head: &str,
) -> Result<Vec<String>, String> {
    let mut git = Git::open(root, trust_repo)?;
    let (prefix, project_root) = (git.prefix.clone(), git.root());
    let head = git.commit(head)?;
    let base = match base {
        Base::Commit(revision) => git.commit(revision)?,
        Base::MergeBaseWith(revision) => git.merge_base(&git.commit(revision)?, &head)?,
    };
    let head_tree = git.tree_id(&head)?;
    let (was, now) = (git.tree(&base)?, git.tree(&head)?);

    // The content of every version that is a file of its own, each once.
    let mut contents: HashMap<&str, Option<String>> = HashMap::new();
    let mut files = Vec::new();
    for (status, old, new) in changes(&was, &now) {
        let old = old.map(|path| (path, &was[path]));
        let new = new.map(|path| (path, &now[path]));
        for (_, entry) in [old, new].into_iter().flatten() {
            if is_regular(entry) && !contents.contains_key(entry.id.as_str()) {
                contents.insert(&entry.id, text(&git.blob(&entry.id)?));
            }
        }
        let version = |(path, entry): (&str, &Entry)| Version {
            file: relative(&prefix, path),
            id: entry.id.clone(),
            mode: entry.mode.clone(),
            binary: is_regular(entry) && contents[entry.id.as_str()].is_none(),
        };
        let (base, head) = (old.map(version), new.map(version));
        // Lines are compared where there are two texts that differ.
        let hunks = match (old, new) {
            (Some((_, old)), Some((_, new))) if is_regular(old) && is_regular(new) => {
                match (&contents[old.id.as_str()], &contents[new.id.as_str()]) {
                    (Some(base), Some(head)) => lines::hunks(base.as_bytes(), head.as_bytes()),
                    _ => Vec::new(),
                }
            }
            _ => Vec::new(),
        };
        let new_directory = new.and_then(|(path, _)| new_directory(&was, path));
        files.push(File {
            status,
            base,
            head,
            hunks,
            new_directory: new_directory.map(|directory| relative(&prefix, directory)),
        });
    }
    let shown = |file: &File| {
        let version = file.head.as_ref().or(file.base.as_ref());
        version.map(|version| version.file.clone())
    };
    files.sort_by_key(shown);

    // The project file and the config file of each commit, changed or not:
    // what is in scope is said there, and the stage after this needs it.
    let named = |tree: &'_ Tree, name: &str| -> Option<String> {
        let entry = tree.get(&format!("{prefix}{name}"))?;
        is_regular(entry).then(|| entry.id.clone())
    };
    let commit = |side, id: &str, tree: &Tree| Commit {
        side,
        id: id.to_string(),
        project_file: named(tree, PROJECT_FILE),
        config_file: named(tree, CONFIG_FILE),
    };
    let commits = [
        commit(Side::Base, &base, &was),
        commit(Side::Head, &head, &now),
    ];
    let mut texts: BTreeMap<String, String> = BTreeMap::new();
    for commit in &commits {
        for id in [&commit.project_file, &commit.config_file]
            .into_iter()
            .flatten()
        {
            if let Some(text) = text(&git.blob(id)?) {
                texts.insert(id.clone(), text);
            }
        }
    }
    for (id, content) in &contents {
        if let Some(text) = content {
            texts.insert(id.to_string(), text.clone());
        }
    }

    // What refs have to be of to go with this delta: the head, with the
    // scope and the config that it has. A project file or a config that
    // cannot be read is for the stage that reads it to refuse.
    let [_, at_head] = &commits;
    let text_of = |id: &Option<String>| id.as_ref().and_then(|id| texts.get(id));
    let scope =
        text_of(&at_head.project_file).and_then(|project| Scope::declared(project.as_bytes()).ok());
    let config = match &at_head.config_file {
        None => Some(Config::default()),
        Some(id) => texts
            .get(id)
            .and_then(|config| serde_json::from_str(config).ok()),
    };
    let provenance = provenance::line(provenance::Delta {
        writer: Writer::of(&provenance::DELTA, env!("CARGO_PKG_VERSION")),
        base,
        head,
        head_tree,
        root: project_root,
        scope,
        config: config.as_ref().map(provenance::config_hash),
    });

    let commits = commits.into_iter().map(Row::Commit);
    let files = files.into_iter().map(Row::File);
    let blobs = texts
        .into_iter()
        .map(|(id, text)| Row::Blob(Blob { id, text }));
    let rows = commits.chain(files).chain(blobs).map(|row| {
        serde_json::to_string(&row).expect("a row has only string keys and plain values")
    });
    Ok(std::iter::once(provenance).chain(rows).collect())
}

/// What became of each file that is not the same in the two trees: its
/// path at base and its path at head.
///
/// No two files are taken for one because they are alike. A path that
/// only the base has is a file deleted, and a path that only the head has
/// a file added, with one exception that needs no judgement: the very same
/// blob under another path is a file that moved. Where several paths have
/// that blob, those with the same file name are paired first and the rest
/// in the order of their paths.
fn changes<'a>(
    was: &'a Tree,
    now: &'a Tree,
) -> Vec<(DeltaStatus, Option<&'a str>, Option<&'a str>)> {
    let mut changes = Vec::new();
    for (path, old) in was {
        if now.get(path).is_some_and(|new| new != old) {
            changes.push((
                DeltaStatus::Modified,
                Some(path.as_str()),
                Some(path.as_str()),
            ));
        }
    }
    let only_in = |tree: &'a Tree, other: &Tree| -> Vec<&'a str> {
        let paths = tree.keys().filter(|path| !other.contains_key(*path));
        paths.map(String::as_str).collect()
    };
    let (gone, mut come) = (only_in(was, now), only_in(now, was));
    // The same content as the same kind of thing: a link is no file.
    let same =
        |old: &Entry, new: &Entry| old.id == new.id && old.mode.get(..3) == new.mode.get(..3);
    let name = |path: &str| path.rsplit('/').next().map(str::to_string);
    let mut moved: BTreeMap<&str, &str> = BTreeMap::new();
    for by_name in [true, false] {
        for &old in &gone {
            if moved.contains_key(old) {
                continue;
            }
            let fits =
                |new: &&str| same(&was[old], &now[*new]) && (!by_name || name(new) == name(old));
            if let Some(at) = come.iter().position(fits) {
                moved.insert(old, come.remove(at));
            }
        }
    }
    for old in gone {
        match moved.get(old) {
            Some(new) => changes.push((DeltaStatus::Moved, Some(old), Some(*new))),
            None => changes.push((DeltaStatus::Deleted, Some(old), None)),
        }
    }
    changes.extend(
        come.into_iter()
            .map(|new| (DeltaStatus::Added, None, Some(new))),
    );
    changes
}

/// A file with content of its own: not a symbolic link, whose content is
/// where it leads, and not a submodule, which is a commit elsewhere.
fn is_regular(entry: &Entry) -> bool {
    entry.mode.starts_with("100")
}

/// A path from the top of the repository, as a path from the root, whose
/// own path from the top is `prefix`.
fn relative(prefix: &str, path: &str) -> String {
    match path.strip_prefix(prefix) {
        Some(below) => below.to_string(),
        None => "../".repeat(prefix.matches('/').count()) + path,
    }
}

/// The topmost directory on a path that a tree does not have.
fn new_directory<'a>(tree: &Tree, path: &'a str) -> Option<&'a str> {
    let has = |directory: &str| {
        let below = format!("{directory}/");
        let first = tree.range(below.clone()..).next();
        first.is_some_and(|(path, _)| path.starts_with(&below))
    };
    let directories = path.match_indices('/').map(|(at, _)| &path[..at]);
    directories.into_iter().find(|directory| !has(directory))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(files: &[(&str, &str)]) -> Tree {
        let entry = |id: &str| Entry {
            mode: "100644".into(),
            id: id.into(),
        };
        let files = files.iter();
        files
            .map(|(path, id)| (path.to_string(), entry(id)))
            .collect()
    }

    #[test]
    fn a_path_in_one_tree_only_is_deleted_or_added_and_in_both_modified_or_the_same() {
        let was = tree(&[("same", "1"), ("edited", "2"), ("gone", "3")]);
        let now = tree(&[("same", "1"), ("edited", "4"), ("come", "5")]);

        assert_eq!(
            changes(&was, &now),
            [
                (DeltaStatus::Modified, Some("edited"), Some("edited")),
                (DeltaStatus::Deleted, Some("gone"), None),
                (DeltaStatus::Added, None, Some("come")),
            ]
        );
    }

    #[test]
    fn a_link_with_the_content_of_a_file_is_not_the_file_moved() {
        let was = tree(&[("file", "1")]);
        let mut now = tree(&[("link", "1")]);
        now.get_mut("link").unwrap().mode = "120000".into();

        assert_eq!(
            changes(&was, &now),
            [
                (DeltaStatus::Deleted, Some("file"), None),
                (DeltaStatus::Added, None, Some("link"))
            ]
        );
    }

    #[test]
    fn a_path_from_the_top_is_given_from_the_root() {
        assert_eq!(relative("", "app/A.cls"), "app/A.cls");
        assert_eq!(relative("project/", "project/app/A.cls"), "app/A.cls");
        assert_eq!(relative("a/project/", "build.sh"), "../../build.sh");
    }

    #[test]
    fn the_topmost_directory_that_is_not_there_is_the_new_one() {
        let was = tree(&[("app/classes/A.cls", "1"), ("apple", "2")]);

        assert_eq!(new_directory(&was, "app/classes/B.cls"), None);
        assert_eq!(new_directory(&was, "app/lwc/x/x.js"), Some("app/lwc"));
        assert_eq!(new_directory(&was, "docs/a/b.md"), Some("docs"));
        assert_eq!(new_directory(&was, "top.md"), None);
        // A file of that name is no directory.
        assert_eq!(new_directory(&was, "apple/pie"), Some("apple"));
    }
}
