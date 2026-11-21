//! What `delta.jsonl` says: the two commits, the files that differ between
//! them and the text of their versions. All that this tool knows of a
//! repository it reads here, from a file. It runs no git.

use sf_impact_common::changed::{ChangedStatus, Side};
use sf_impact_common::delta::{Commit, File, Row, Version};
use sf_impact_common::lines::Hunk;
use std::collections::{HashMap, HashSet};
use std::io::BufRead;

/// One file that differs between the base and the head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub status: ChangedStatus,
    /// The file at base; `None` for one the difference adds.
    pub old: Option<Blob>,
    /// The file at head; `None` for one the difference deletes.
    pub new: Option<Blob>,
    pub content: Content,
    /// The topmost directory that came with the file at head, if one did.
    pub new_directory: Option<String>,
}

/// One version of a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Blob {
    /// Relative to the root, with `/` separators; `../` in front of a file
    /// of the repository that is outside the root.
    pub path: String,
    pub id: String,
    /// A file with content of its own: no symbolic link and no submodule,
    /// neither of which is read.
    pub regular: bool,
    pub binary: bool,
}

/// How the content of a file changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    /// It did not: the file moved, or only its mode changed.
    Same,
    /// It changed and there are no lines for the change: a version is
    /// binary, or the file became another kind of thing.
    Binary,
    /// The lines that differ; none for a file that is added or deleted,
    /// which has no other version to differ from.
    Lines(Vec<Hunk>),
}

pub struct Delta {
    pub base: Commit,
    pub head: Commit,
    /// In the order of the file.
    pub changes: Vec<Change>,
    /// The text of each blob that is text, by its id.
    texts: HashMap<String, String>,
    /// The path at head of each file whose path the base does not have.
    came: HashSet<String>,
}

impl Delta {
    /// Reads the rows of a delta after its first, which says what the
    /// file is of and is read before any other: lines count from 2.
    pub fn read(delta: impl BufRead) -> Result<Delta, String> {
        let (mut commits, mut changes) = (HashMap::new(), Vec::new());
        let (mut texts, mut came) = (HashMap::new(), HashSet::new());
        for (number, line) in (2..).zip(delta.lines()) {
            let line = line.map_err(|e| format!("line {number}: {e}"))?;
            let row = serde_json::from_str(&line).map_err(|e| format!("line {number}: {e}"));
            match row? {
                Row::Commit(commit) => {
                    commits.insert(commit.side, commit);
                }
                Row::File(file) => {
                    let is_new_path =
                        matches!(file.status, sf_impact_common::delta::DeltaStatus::Added)
                            || matches!(file.status, sf_impact_common::delta::DeltaStatus::Moved);
                    if let Some(head) = file.head.as_ref().filter(|_| is_new_path) {
                        came.insert(head.file.clone());
                    }
                    let change = change(file).map_err(|e| format!("line {number}: {e}"));
                    changes.push(change?);
                }
                Row::Blob(blob) => {
                    texts.insert(blob.id, blob.text);
                }
            }
        }
        let mut commit = |side: Side, name: &str| {
            let missing = format!("there is no commit row for the {name}: is this delta.jsonl?");
            commits.remove(&side).ok_or(missing)
        };
        Ok(Delta {
            base: commit(Side::Base, "base")?,
            head: commit(Side::Head, "head")?,
            changes,
            texts,
            came,
        })
    }

    /// The text of the blob with this id, if the delta gives it.
    pub fn text(&self, id: &str) -> Option<&str> {
        self.texts.get(id).map(String::as_str)
    }

    /// The text of a version of a file, if it is a file of text. A link
    /// has none, though the blob that says where it leads may be one.
    pub fn text_of(&self, blob: &Blob) -> Option<&str> {
        self.text(&blob.id).filter(|_| blob.regular)
    }

    /// Whether the head has a file at a path where the base had none.
    pub fn came(&self, path: &str) -> bool {
        self.came.contains(path)
    }

    /// The version at base of the file that was at `path` there, if it is
    /// not the same file at head.
    pub fn at_base(&self, path: &str) -> Option<&Blob> {
        let olds = self.changes.iter().filter_map(|change| change.old.as_ref());
        olds.into_iter().find(|old| old.path == path)
    }
}

/// A file row as this tool reads it: what became of the file, and whether
/// there are lines to map. A row that does not have the versions its
/// status has says nothing that can be mapped, and is refused.
fn change(file: File) -> Result<Change, String> {
    use sf_impact_common::delta::DeltaStatus::{Added, Deleted, Modified, Moved};
    // A file, a link or a submodule: what the first digits of a mode say.
    let kind = |version: &Version| version.mode.get(..3).map(str::to_string);
    let blob = |version: &Version| Blob {
        path: version.file.clone(),
        id: version.id.clone(),
        regular: version.is_regular(),
        binary: version.binary,
    };
    let lines = |version: &Version| match version.binary {
        true => Content::Binary,
        false => Content::Lines(Vec::new()),
    };
    let (status, content) = match (file.status, &file.base, &file.head) {
        (Added, None, Some(head)) => (ChangedStatus::Added, lines(head)),
        (Added, _, _) => return Err("an `added` file has a `head` and no `base`".to_string()),
        (Deleted, Some(base), None) => (ChangedStatus::Deleted, lines(base)),
        (Deleted, _, _) => return Err("a `deleted` file has a `base` and no `head`".to_string()),
        (Moved, Some(base), Some(head)) if base.id == head.id && kind(base) == kind(head) => {
            (ChangedStatus::Moved, Content::Same)
        }
        (Moved, _, _) => {
            let needs = "a `moved` file has a `base` and a `head` that are the same blob \
                         as the same kind of thing";
            return Err(needs.to_string());
        }
        // Before the blobs are compared: where a link leads is in a blob,
        // which can be the very blob of a file.
        (Modified, Some(base), Some(head)) if kind(base) != kind(head) => {
            (ChangedStatus::TypeChanged, Content::Binary)
        }
        (Modified, Some(base), Some(head)) if base.id == head.id => {
            (ChangedStatus::ModeChanged, Content::Same)
        }
        (Modified, Some(base), Some(head)) if base.binary || head.binary => {
            (ChangedStatus::Modified, Content::Binary)
        }
        (Modified, Some(_), Some(_)) => (ChangedStatus::Modified, Content::Lines(file.hunks)),
        (Modified, _, _) => return Err("a `modified` file has a `base` and a `head`".to_string()),
    };
    Ok(Change {
        status,
        old: file.base.as_ref().map(blob),
        new: file.head.as_ref().map(blob),
        content,
        new_directory: file.new_directory,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(file: &str) -> Change {
        let commit = |side: &str| {
            let id = "c".repeat(40);
            format!(
                r#"{{"row":"commit","side":"{side}","id":"{id}","project_file":null,"config_file":null}}"#
            )
        };
        let lines = format!("{}\n{}\n{file}\n", commit("base"), commit("head"));
        let mut delta = Delta::read(lines.as_bytes()).unwrap();
        delta.changes.remove(0)
    }

    fn version(mode: &str, id: &str, binary: bool) -> String {
        let id = id.repeat(40);
        format!(r#"{{"file":"a/F.cls","id":"{id}","mode":"{mode}","binary":{binary}}}"#)
    }

    fn modified(base: String, head: String) -> Change {
        read(&format!(
            r#"{{"row":"file","status":"modified","base":{base},"head":{head}}}"#
        ))
    }

    #[test]
    fn a_file_with_the_same_blob_and_another_mode_changed_its_mode_alone() {
        let change = modified(version("100644", "1", false), version("100755", "1", false));

        assert_eq!(
            (change.status, change.content),
            (ChangedStatus::ModeChanged, Content::Same)
        );
    }

    #[test]
    fn a_file_that_became_a_link_or_a_submodule_changed_its_type_and_has_no_lines() {
        let to_link = modified(version("100644", "1", false), version("120000", "2", false));
        let from_submodule = modified(version("160000", "1", false), version("100644", "2", false));

        assert_eq!(
            (to_link.status, to_link.content),
            (ChangedStatus::TypeChanged, Content::Binary)
        );
        assert_eq!(from_submodule.status, ChangedStatus::TypeChanged);
        assert!(!to_link.new.unwrap().regular && to_link.old.unwrap().regular);
    }

    #[test]
    fn a_file_that_became_a_link_with_the_blob_it_had_changed_its_type_not_its_mode() {
        // Where a link leads is in a blob, which can be the blob of a file.
        let to_link = modified(version("100644", "1", false), version("120000", "1", false));
        let to_file = modified(version("120000", "1", false), version("100755", "1", false));

        assert_eq!(
            (to_link.status, to_link.content),
            (ChangedStatus::TypeChanged, Content::Binary)
        );
        assert_eq!(
            (to_file.status, to_file.content),
            (ChangedStatus::TypeChanged, Content::Binary)
        );
    }

    /// What reading a delta with this one file row fails with.
    fn refused(file: &str) -> String {
        let commit = r#"{"row":"commit","side":"base","id":"cccccccccccccccccccccccccccccccccccccccc","project_file":null,"config_file":null}"#;
        let lines = format!("{commit}\n{}\n{file}\n", commit.replace("base", "head"));
        match Delta::read(lines.as_bytes()) {
            Ok(_) => panic!("{file} was read"),
            Err(error) => error,
        }
    }

    #[test]
    fn a_file_row_without_the_versions_its_status_has_is_refused_with_its_line() {
        let row = |status: &str, base: &str, head: &str| {
            format!(r#"{{"row":"file","status":"{status}","base":{base},"head":{head}}}"#)
        };
        let version = version("100644", "1", false);
        let other = version.replace(
            r#""id":"1111111111111111111111111111111111111111""#,
            r#""id":"2222222222222222222222222222222222222222""#,
        );

        for (status, base, head) in [
            ("modified", "null", "null"),
            ("modified", &version, "null"),
            ("modified", "null", &version),
            ("moved", &version, "null"),
            ("moved", "null", &version),
            ("added", "null", "null"),
            ("added", &version, &version),
            ("deleted", "null", "null"),
            ("deleted", &version, &version),
        ] {
            // The third row that is read is the fourth line of its file.
            let error = refused(&row(status, base, head));
            assert!(error.starts_with("line 4: "), "{status}: {error}");
            assert!(error.contains(status), "{error}");
        }
        // A file that moved is the same blob as the same kind of thing.
        let another_blob = refused(&row("moved", &version, &other));
        let another_kind = refused(&row(
            "moved",
            &version,
            &version.replace("100644", "120000"),
        ));
        assert!(another_blob.starts_with("line 4: "), "{another_blob}");
        assert!(another_kind.starts_with("line 4: "), "{another_kind}");
    }

    #[test]
    fn a_change_with_a_binary_version_has_no_lines_and_any_other_has_those_it_is_given() {
        let binary = modified(version("100644", "1", false), version("100644", "2", true));
        let text = read(&format!(
            r#"{{"row":"file","status":"modified","base":{},"head":{},"hunks":[{{"base_start":3,"base_lines":1,"head_start":3,"head_lines":2}}]}}"#,
            version("100644", "1", false),
            version("100644", "2", false)
        ));
        // A submodule at another commit: nothing to compare, and nothing binary.
        let submodule = modified(version("160000", "1", false), version("160000", "2", false));

        assert_eq!(
            (binary.status, binary.content),
            (ChangedStatus::Modified, Content::Binary)
        );
        let hunk = Hunk {
            base_start: 3,
            base_lines: 1,
            head_start: 3,
            head_lines: 2,
        };
        assert_eq!(text.content, Content::Lines(vec![hunk]));
        assert_eq!(
            (submodule.status, submodule.content),
            (ChangedStatus::Modified, Content::Lines(vec![]))
        );
    }

    #[test]
    fn a_file_that_came_or_went_is_binary_if_the_version_there_is() {
        let added = read(&format!(
            r#"{{"row":"file","status":"added","base":null,"head":{}}}"#,
            version("100644", "1", true)
        ));
        let deleted = read(&format!(
            r#"{{"row":"file","status":"deleted","base":{},"head":null}}"#,
            version("100644", "1", false)
        ));

        assert_eq!(
            (added.status, added.content),
            (ChangedStatus::Added, Content::Binary)
        );
        assert_eq!(
            (deleted.status, deleted.content),
            (ChangedStatus::Deleted, Content::Lines(vec![]))
        );
    }

    #[test]
    fn a_path_the_base_did_not_have_is_known_to_have_come() {
        let lines = [
            r#"{"row":"commit","side":"base","id":"cccccccccccccccccccccccccccccccccccccccc","project_file":null,"config_file":null}"#,
            r#"{"row":"commit","side":"head","id":"dddddddddddddddddddddddddddddddddddddddd","project_file":null,"config_file":null}"#,
            r#"{"row":"file","status":"added","base":null,"head":{"file":"new","id":"1111111111111111111111111111111111111111","mode":"100644","binary":false}}"#,
            r#"{"row":"file","status":"moved","base":{"file":"from","id":"2222222222222222222222222222222222222222","mode":"100644","binary":false},"head":{"file":"to","id":"2222222222222222222222222222222222222222","mode":"100644","binary":false}}"#,
            r#"{"row":"file","status":"modified","base":{"file":"same","id":"3333333333333333333333333333333333333333","mode":"100644","binary":false},"head":{"file":"same","id":"4444444444444444444444444444444444444444","mode":"100644","binary":false}}"#,
            r#"{"row":"blob","id":"2222222222222222222222222222222222222222","text":"moved\n"}"#,
        ];
        let delta = Delta::read(lines.join("\n").as_bytes()).unwrap();

        assert!(delta.came("new") && delta.came("to"));
        assert!(!delta.came("same") && !delta.came("from"));
        assert_eq!(
            delta.at_base("from").map(|blob| blob.id.as_str()),
            Some("2222222222222222222222222222222222222222")
        );
        assert_eq!(delta.at_base("to"), None);
        assert_eq!(
            delta.text("2222222222222222222222222222222222222222"),
            Some("moved\n")
        );
        assert_eq!(delta.text("3333333333333333333333333333333333333333"), None);
        // A link is not read, though where it leads is the text of a file.
        let file = delta.changes[1].new.clone().unwrap();
        let link = Blob {
            regular: false,
            ..file.clone()
        };
        assert_eq!(delta.text_of(&file), Some("moved\n"));
        assert_eq!(delta.text_of(&link), None);
    }
}
