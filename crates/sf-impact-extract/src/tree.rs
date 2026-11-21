//! Which tree was read, for the first row of `refs.jsonl`: the commit that
//! is checked out, if all that was read is as that commit has it.
//!
//! Git is asked, through the one module that runs it, which commit is
//! checked out and what that commit holds, before a file is read, and is
//! asked nothing after. What is read is then held to the commit here, by
//! the id of the very bytes that the rows are made of: a file that another
//! process changes while this one runs is the commit's only if what was
//! read of it is. Nothing is written, and the index is not asked: what git
//! calls clean is not the question. A file outside the scope is not read
//! and does not count; a file in scope that git ignores is read, and is
//! not the commit's.

use crate::blob::Format;
use sf_impact_common::config::CONFIG_FILE;
use sf_impact_common::diagnostic::shown;
use sf_impact_common::provenance::{Scope, Tree};
use sf_impact_common::scope::{self, PROJECT_FILE};
use sf_impact_delta::git::Git;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The commit that is checked out where a project is, and what it holds
/// of the project.
pub struct Checkout {
    commit: String,
    /// The tree of the commit.
    tree: String,
    /// Where the project is in the repository, from the top of it: `.`
    /// where it is the top.
    pub root: String,
    /// How the repository names its objects.
    pub format: Format,
    /// Every file of the commit under the root, by its path from the
    /// root, with the id of its content. A link is not followed and a
    /// submodule is not the commit's content: neither is a file.
    files: BTreeMap<String, String>,
}

impl Checkout {
    /// What is checked out at `root`, as git has it now: asked before a
    /// file of the project is read. `trust_repo` says to read the
    /// repository though another user owns it.
    pub fn of(root: &Path, trust_repo: bool) -> Result<Checkout, String> {
        let git = Git::open(root, trust_repo)?;
        let commit = git.commit("HEAD")?;
        let tree = git.tree_id(&commit)?;
        let format = Format::of(&commit)
            .ok_or_else(|| format!("the commit {commit} has an id of no known length"))?;
        let held = git.tree(&commit)?;
        let files = held.into_iter().filter_map(|(path, entry)| {
            let path = path.strip_prefix(&git.prefix)?;
            entry
                .mode
                .starts_with("100")
                .then(|| (path.to_string(), entry.id))
        });
        Ok(Checkout {
            files: files.collect(),
            root: git.root(),
            commit,
            tree,
            format,
        })
    }

    /// The tree that was read, where `read` is every file that was, by
    /// its path from the root, with the id of the bytes that were read of
    /// it: the project file, the config if there is one, and the files of
    /// the scope. With it, what to tell the caller where that is not the
    /// commit.
    pub fn read_as(self, scope: &Scope, read: &BTreeMap<String, String>) -> (Tree, Option<String>) {
        // What the commit has that would be read.
        let directories = scope.directories();
        let is_read = |path: &str| {
            path == PROJECT_FILE
                || path == CONFIG_FILE
                || scope::package_of(&directories, path).is_some()
        };
        let in_commit = |path: &str| self.files.get(path).filter(|_| is_read(path));
        let of_commit = self.files.keys().filter(|path| is_read(path));
        let paths: BTreeSet<&String> = read.keys().chain(of_commit).collect();
        let difference =
            paths
                .into_iter()
                .find_map(|path| match (read.get(path), in_commit(path)) {
                    (Some(read), Some(held)) if read == held => None,
                    (Some(_), Some(_)) => Some(format!("{} is not as it is in", shown(path))),
                    (Some(_), None) => Some(format!("{} was read and is no file of", shown(path))),
                    (None, _) => Some(format!("{} was not read and is a file of", shown(path))),
                });
        let Checkout { commit, tree, .. } = self;
        match difference {
            None => (Tree::Commit { commit, tree }, None),
            Some(difference) => {
                let note = format!(
                    "the tree is `dirty` to what reads refs.jsonl: {difference} the commit \
                     {commit} that is checked out"
                );
                (Tree::Dirty, Some(note))
            }
        }
    }
}

/// What to tell the caller where no commit could be read, for git's reason.
pub fn unknown(why: &str) -> String {
    format!("the tree is `unknown` to what reads refs.jsonl: no commit could be read for it: {why}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(files: &[(&str, &str)]) -> BTreeMap<String, String> {
        let files = files.iter();
        files
            .map(|(path, id)| (path.to_string(), id.to_string()))
            .collect()
    }

    /// The state of the tree for what was read, at a commit that has
    /// a project file, a class in scope and a file that is in none.
    fn state(read: &[(&str, &str)]) -> (Tree, Option<String>) {
        let checkout = Checkout {
            commit: "c".repeat(40),
            tree: "d".repeat(40),
            root: ".".into(),
            format: Format::Sha1,
            files: files(&[
                ("sfdx-project.json", "1"),
                ("app/A.cls", "2"),
                ("README.md", "3"),
            ]),
        };
        let scope = Scope {
            package_directories: vec!["app".into()],
            unpackaged_metadata: Vec::new(),
        };
        checkout.read_as(&scope, &files(read))
    }

    #[test]
    fn the_tree_is_the_commit_if_the_bytes_that_were_read_are_those_the_commit_has() {
        let (tree, note) = state(&[("sfdx-project.json", "1"), ("app/A.cls", "2")]);

        let (commit, tree_id) = ("c".repeat(40), "d".repeat(40));
        assert_eq!(
            tree,
            Tree::Commit {
                commit,
                tree: tree_id
            }
        );
        assert_eq!(note, None);
    }

    #[test]
    fn the_tree_is_dirty_for_any_file_that_was_read_otherwise_or_not_at_all() {
        let said = |read: &[(&str, &str)]| {
            let (tree, note) = state(read);
            assert_eq!(tree, Tree::Dirty);
            note.expect("a note")
        };

        // Other bytes than the commit has, whatever is on disk by now.
        let edited = said(&[("sfdx-project.json", "1"), ("app/A.cls", "9")]);
        assert!(edited.contains("app/A.cls is not as it is in"), "{edited}");
        let project = said(&[("sfdx-project.json", "9"), ("app/A.cls", "2")]);
        assert!(project.contains("sfdx-project.json is not as"), "{project}");
        // A file that the commit does not have, in scope or the config.
        let added = said(&[
            ("sfdx-project.json", "1"),
            ("app/A.cls", "2"),
            ("app/B.cls", "4"),
        ]);
        assert!(added.contains("app/B.cls was read and is no"), "{added}");
        let config = said(&[
            ("sfdx-project.json", "1"),
            ("app/A.cls", "2"),
            ("sf-impact.json", "5"),
        ]);
        assert!(config.contains("sf-impact.json was read"), "{config}");
        // A file of the commit that was not among those read.
        let missing = said(&[("sfdx-project.json", "1")]);
        assert!(missing.contains("app/A.cls was not read"), "{missing}");
    }

    #[test]
    fn a_path_in_the_note_cannot_start_a_line_of_its_own() {
        let (_, note) = state(&[
            ("sfdx-project.json", "1"),
            ("app/A.cls", "2"),
            ("app/B\n::error::x\u{1b}[2K.cls", "4"),
        ]);

        let note = note.expect("a note");
        assert!(
            note.contains("app/B\\n::error::x\\u{1b}[2K.cls was read"),
            "{note}"
        );
        assert!(!note.chars().any(char::is_control), "{note:?}");
    }
}
