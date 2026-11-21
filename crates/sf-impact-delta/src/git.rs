//! The one place where these tools run git, and all that is asked of it:
//! which commit a name is and which tree a commit has (`rev-parse`), where
//! two commits part (`merge-base`), what a commit's tree holds (`ls-tree`)
//! and what a blob is (`cat-file`). Those four read objects and nothing
//! else. Whatever could make two machines disagree about two commits, the
//! configuration of a user or a repository, attributes, diff drivers,
//! rename detection, belongs to `git diff`, which is not run. A git that
//! cannot be told not to fetch is asked one thing more: which objects the
//! repository has (`rev-list`).
//!
//! `sf-impact-extract` asks the same of the commit that is checked out, before
//! it reads a file, to say which tree it read: it compares what it reads
//! with what the commit holds itself, and asks git nothing after.
//!
//! Git is run without any `GIT_*` variable of the caller's and without
//! the user's and the system's configuration, so that the repository it
//! reads is the one the root is in, read as it is: no replacement object
//! and no graft stands in for what a commit is.
//!
//! Git reads the objects the repository has and fetches none. A clone
//! that was not given all its objects (a partial clone) is promised the
//! rest by a remote, and git fetches one when it is asked to read it,
//! which writes to the repository and runs whatever the repository says
//! to reach the remote with. So git is told not to (`--no-lazy-fetch`,
//! which it has had since 2.45), and an object that is not there is an
//! error. A git that cannot be told is only asked to read what is known
//! to be there, in any repository: whether a repository is promised
//! objects is for git to say, in more ways than one, and is not asked.
//! See [`Git::has`].

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command, Output, Stdio};

/// A file as a tree has it: not a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// `100644`, `100755`, `120000` for a link, `160000` for a submodule.
    pub mode: String,
    /// The blob; for a submodule, the commit it is at.
    pub id: String,
}

/// A repository, seen from a directory in it.
pub struct Git<'a> {
    root: &'a Path,
    /// The root's path below the top of the repository, with a `/` after
    /// it; empty where the root is the top.
    pub prefix: String,
    /// What git is told before every command: not to fetch, and to read
    /// a repository of another owner if this one is to be trusted.
    options: Vec<OsString>,
    /// Git cannot be told not to fetch: each object is looked for before
    /// git is asked to read it.
    looks_first: bool,
    objects: Objects,
}

/// What is said where git could not read an object.
const FETCHES_NOTHING: &str = "This tool fetches nothing: an object that the repository does not have has to be \
     fetched first. If a commit is not there, fetch the history that has it. In a partial \
     clone, fetch the trees and the files of the two commits: `git diff <base> <head>` \
     fetches the files that differ";

fn or_fetch_first(error: String) -> String {
    format!("{error} | {FETCHES_NOTHING}")
}

impl<'a> Git<'a> {
    /// The repository that `root` is in. `trust` says to read it though
    /// another user owns it, which git refuses to on its own.
    pub fn open(root: &'a Path, trust: bool) -> Result<Git<'a>, String> {
        // First, so that a directory in no repository is refused once.
        // It reads no object.
        let show_prefix = ["rev-parse", "--show-prefix"];
        let mut options = vec![OsString::from("--no-lazy-fetch")];
        let mut first = run(root, &options, &show_prefix)?;
        // A git that does not know the option says so as it does of any
        // command line it cannot read.
        let can_be_told = first.status.code() != Some(129);
        if !can_be_told {
            options.clear();
            first = run(root, &options, &show_prefix)?;
        }
        let prefix = match said(&show_prefix, first) {
            Ok(prefix) => prefix,
            Err(refusal) if trust => {
                let (exception, prefix) = trusted(root, &options, &show_prefix).ok_or(refusal)?;
                options = exception;
                prefix
            }
            Err(refusal) => return Err(refusal),
        };
        Ok(Git {
            root,
            prefix,
            looks_first: !can_be_told,
            objects: Objects::open(root, &options)?,
            options,
        })
    }

    /// The id of the commit that `revision` names.
    pub fn commit(&self, revision: &str) -> Result<String, String> {
        // Nothing that names a commit begins so, and git would take it
        // for an option.
        if revision.starts_with('-') {
            return Err(format!("a revision does not begin with `-`: {revision}"));
        }
        let commit = format!("{revision}^{{commit}}");
        self.has(&["--no-walk", &commit])?;
        self.git(&["rev-parse", "--verify", &commit])
    }

    /// The id of the tree that a commit has.
    pub fn tree_id(&self, commit: &str) -> Result<String, String> {
        // That the commit is there does not say that its tree is, and a
        // tree that is not would be fetched to be named.
        self.has(&["--objects", "--no-walk", commit])?;
        self.git(&["rev-parse", "--verify", &format!("{commit}^{{tree}}")])
    }

    /// Where the root is in the repository, from the top of it, with `/`
    /// between its parts: `.` where it is the top.
    pub fn root(&self) -> String {
        match self.prefix.trim_end_matches('/') {
            "" => ".".to_string(),
            below => below.to_string(),
        }
    }

    /// The commit that two commits both come from.
    pub fn merge_base(&self, one: &str, other: &str) -> Result<String, String> {
        // The history of both is walked to find it.
        self.has(&[one, other])?;
        self.git(&["merge-base", one, other])
    }

    /// Every file of a commit, by its path from the top of the repository.
    pub fn tree(&self, commit: &str) -> Result<BTreeMap<String, Entry>, String> {
        // Its trees are read here, and its files after this.
        self.has(&["--objects", "--no-walk", commit])?;
        let args = ["ls-tree", "-r", "-z", "--full-tree", commit];
        let listed =
            output(&args, run(self.root, &self.options, &args)?).map_err(or_fetch_first)?;
        // `<mode> <type> <id>`, a tab, the path as it is, and a NUL.
        let records = listed.split(|byte| *byte == 0);
        let entries = records.filter(|record| !record.is_empty()).map(|record| {
            // A path is given as the text it is or not at all. Bytes that
            // are no text would have to be changed to be written, and two
            // paths that are changed alike would be taken for one.
            let record = std::str::from_utf8(record).map_err(|_| {
                let record = String::from_utf8_lossy(record);
                let path = record.split_once('\t').map_or(&*record, |(_, path)| path);
                format!(
                    "the commit {commit} has a file whose path is not UTF-8, and such a \
                     path is not read: {}",
                    sf_impact_common::diagnostic::shown(path)
                )
            })?;
            let unreadable = || format!("git ls-tree gave a record that is not one: {record:?}");
            let (entry, path) = record.split_once('\t').ok_or_else(unreadable)?;
            let fields: Vec<&str> = entry.split(' ').collect();
            let [mode, _type, id] = fields[..] else {
                return Err(unreadable());
            };
            let (mode, id) = (mode.to_string(), id.to_string());
            Ok((path.to_string(), Entry { mode, id }))
        });
        entries.collect()
    }

    /// The content of a blob.
    pub fn blob(&mut self, id: &str) -> Result<Vec<u8>, String> {
        let content = self.objects.read(id)?;
        content.ok_or_else(|| absent(id))
    }

    /// What git says of objects, or what went wrong, which may be that
    /// one is not there.
    fn git(&self, args: &[&str]) -> Result<String, String> {
        said(args, run(self.root, &self.options, args)?).map_err(or_fetch_first)
    }

    /// Refuses unless the repository has all that `rev-list` comes by on
    /// this walk, where that has to be known before git is asked to read
    /// it: git cannot be told not to fetch, and would fetch what is not
    /// there if the repository is promised it. `rev-list --missing=print`
    /// is how a git of any age is asked what is there: it lists what is
    /// missing and fetches nothing.
    fn has(&self, walk: &[&str]) -> Result<(), String> {
        if !self.looks_first {
            return Ok(());
        }
        let mut args = vec!["rev-list", "--missing=print"];
        args.extend(walk);
        let listed = self.git(&args)?;
        match listed.lines().find_map(|line| line.strip_prefix('?')) {
            None => Ok(()),
            Some(id) => Err(format!(
                "{} | This git cannot be told not to fetch (`--no-lazy-fetch` came with git \
                 2.45), so a repository is read only if it has every file of both commits",
                absent(id)
            )),
        }
    }
}

fn absent(id: &str) -> String {
    or_fetch_first(format!("the repository does not have the object {id}"))
}

/// `git` in `dir`, told nothing by its caller's environment or by any
/// configuration but the repository's own, and reading the commits as
/// they are: not as a replacement object or a graft says they are.
fn command(dir: &Path, options: &[OsString]) -> Command {
    let mut git = Command::new("git");
    git.current_dir(dir);
    for (name, _) in std::env::vars_os() {
        if name.as_encoded_bytes().starts_with(b"GIT_") {
            git.env_remove(name);
        }
    }
    git.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        // A file that cannot be: of one that can be read, even an empty
        // one, git says on every run that grafts are no longer to be used.
        .env("GIT_GRAFT_FILE", "/dev/null/grafts");
    git.args(options);
    git
}

fn run(dir: &Path, options: &[OsString], args: &[&str]) -> Result<Output, String> {
    let output = command(dir, options).args(args).output();
    output.map_err(|e| format!("git could not be run: {e}"))
}

/// What a git that was run wrote, or what it said went wrong.
fn output(args: &[&str], output: Output) -> Result<Vec<u8>, String> {
    if output.status.success() {
        return Ok(output.stdout);
    }
    let said = String::from_utf8_lossy(&output.stderr);
    let mut message = format!(
        "git {} failed: {}",
        args.join(" "),
        said.trim_end().replace('\n', " | ")
    );
    // The exception git asks for is made in the user's configuration,
    // which is not read here.
    if said.contains("dubious ownership") {
        message.push_str(
            " | Git is run here without the user's and the system's configuration, so a \
             `safe.directory` exception made there does not apply. Run it as the owner of \
             the repository, or give `--trust-repo` to have this one repository read \
             though another user owns it",
        );
    }
    Err(message)
}

/// What a git that was run said, without the line end.
fn said(args: &[&str], run: Output) -> Result<String, String> {
    let said = String::from_utf8(output(args, run)?)
        .map_err(|_| format!("git {} said what is not UTF-8", args.join(" ")))?;
    Ok(said.trim_end_matches('\n').to_string())
}

/// The options with which git reads the repository that `dir` is in
/// though another user owns it, and what it then says when `asked`.
///
/// Git reads such a repository if it is told, in so many words, that the
/// directory at the top of it is safe. It does not say where the top of a
/// repository it will not read is, so each directory from `dir` upwards
/// is offered in turn: git takes the one that is the top of the
/// repository it found, and refuses every other. The exception is given
/// on the command line, for that one directory, and is written nowhere.
fn trusted(dir: &Path, options: &[OsString], asked: &[&str]) -> Option<(Vec<OsString>, String)> {
    let dir = dir.canonicalize().ok()?;
    dir.ancestors().find_map(|top| {
        let mut exception = OsString::from("safe.directory=");
        exception.push(top);
        let mut options = options.to_vec();
        options.extend([OsString::from("-c"), exception]);
        let said = said(asked, run(&dir, &options, asked).ok()?).ok()?;
        Some((options, said))
    })
}

/// A `git cat-file --batch` that is asked for one object after another:
/// one process, however many files changed.
struct Objects {
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Objects {
    fn open(dir: &Path, options: &[OsString]) -> Result<Objects, String> {
        let mut child = command(dir, options)
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("git could not be run: {e}"))?;
        let (input, output) = (child.stdin.take(), child.stdout.take());
        Ok(Objects {
            input: input.expect("the input is piped"),
            output: BufReader::new(output.expect("the output is piped")),
        })
    }

    /// The content of the object with this id; `None` if there is none.
    fn read(&mut self, id: &str) -> Result<Option<Vec<u8>>, String> {
        let failed = |e: std::io::Error| format!("git cat-file failed on {id}: {e}");
        writeln!(self.input, "{id}").map_err(failed)?;
        self.input.flush().map_err(failed)?;
        // `<id> <type> <size>`, the content, and a line end; or
        // `<id> missing`.
        let mut header = String::new();
        if self.output.read_line(&mut header).map_err(failed)? == 0 {
            return Err(format!("git cat-file ended without an answer for {id}"));
        }
        let header = header.trim_end();
        if header.ends_with(" missing") {
            return Ok(None);
        }
        let size = header.rsplit(' ').next().and_then(|size| size.parse().ok());
        let size: usize = size.ok_or_else(|| format!("git cat-file said of {id}: {header}"))?;
        let mut content = vec![0; size + 1];
        self.output.read_exact(&mut content).map_err(failed)?;
        content.truncate(size);
        Ok(Some(content))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_is_not_given_what_its_caller_was_told_about_git() {
        let git = command(Path::new("."), &[]);

        let set: Vec<_> = git
            .get_envs()
            .filter(|(_, value)| value.is_some())
            .collect();
        let names: Vec<_> = set.iter().map(|(name, _)| name.to_str().unwrap()).collect();
        assert_eq!(
            names,
            [
                "GIT_CONFIG_GLOBAL",
                "GIT_CONFIG_NOSYSTEM",
                "GIT_GRAFT_FILE",
                "GIT_NO_REPLACE_OBJECTS"
            ]
        );
    }
}
