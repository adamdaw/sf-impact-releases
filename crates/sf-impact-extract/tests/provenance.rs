//! The first row of `refs.jsonl`: what wrote the file, of which tree, with
//! which scope and config. Each test makes a small repository in a
//! temporary directory and runs the `sf-impact-extract` binary on it.

#[path = "../../sf-impact-common/tests/support/schema.rs"]
mod schema;

use schema::Schema;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::{env, fs};

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `git` as the tests run it: without the configuration of whoever runs
/// them, with a fixed identity and a fixed date, so that the same content
/// is the same commit on every machine.
fn git(dir: &Path) -> Command {
    let mut git = Command::new("git");
    git.current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "A Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_NAME", "A Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
    git
}

fn run(command: &mut Command) -> Output {
    let output = command.output().expect("the command starts");
    assert!(output.status.success(), "{command:?} failed: {output:?}");
    output
}

const PROJECT: &str = r#"{"packageDirectories": [
    {"path": "plugins\\billing"},
    {"path": "./app/", "unpackagedMetadata": {"path": "unpackaged"}},
    {"path": "../elsewhere"}
]}"#;
const ALPHA: &str = "public class Alpha {\n}\n";
const CLASS: &str = "app/classes/Alpha.cls";

/// A project in a temporary directory, removed on drop: a project file,
/// one class in each scope directory and a file that is in none.
struct Project {
    dir: PathBuf,
    /// The project's directory within `dir/repo`; empty for the top.
    within: &'static str,
}

impl Project {
    /// Not in any repository.
    fn files(name: &str) -> Project {
        Project::files_within(name, "")
    }

    fn files_within(name: &str, within: &'static str) -> Project {
        let dir = env::temp_dir().join(format!(
            "sf-impact-extract-provenance-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let project = Project { dir, within };
        project.write("sfdx-project.json", PROJECT);
        project.write(CLASS, ALPHA);
        project.write(
            "plugins/billing/classes/Beta.cls",
            "public class Beta {\n}\n",
        );
        project.write(
            "unpackaged/classes/AlphaTest.cls",
            "@IsTest\nclass AlphaTest {\n}\n",
        );
        project.write("README.md", "Not in scope.\n");
        project
    }

    /// The top of a repository whose one commit is these files.
    fn committed(name: &str) -> Project {
        Project::committed_within(name, "")
    }

    /// The same, with the project in a directory of the repository.
    fn committed_within(name: &str, within: &'static str) -> Project {
        let project = Project::files_within(name, within);
        run(git(&project.top()).args([
            "init",
            "--quiet",
            "--initial-branch=main",
            "--object-format=sha1",
        ]));
        project.commit("base");
        project
    }

    fn top(&self) -> PathBuf {
        self.dir.join("repo")
    }

    fn root(&self) -> PathBuf {
        self.top().join(self.within)
    }

    fn write(&self, file: &str, content: &str) {
        let path = self.root().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn commit(&self, message: &str) {
        run(git(&self.top()).args(["add", "--all"]));
        run(git(&self.top()).args(["commit", "--quiet", "--message", message]));
    }

    /// What git says of a revision: an id.
    fn id(&self, revision: &str) -> String {
        let said = run(git(&self.top()).args(["rev-parse", revision])).stdout;
        String::from_utf8(said).unwrap().trim_end().to_string()
    }

    fn sf_impact_extract(&self) -> Command {
        let mut extract = Command::new(env!("CARGO_BIN_EXE_sf-impact-extract"));
        extract.arg("--root").arg(self.root());
        extract.args(["--format", "jsonl"]);
        extract
    }

    fn output(&self) -> String {
        String::from_utf8(run(&mut self.sf_impact_extract()).stdout).expect("the output is UTF-8")
    }

    /// The first row that is written: the provenance of the file.
    fn provenance(&self) -> Value {
        provenance(&self.output())
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn provenance(output: &str) -> Value {
    let first = output.lines().next().expect("a first line");
    let row: Value = serde_json::from_str(first).expect("the first line is JSON");
    assert_eq!(row["row"], "provenance", "{row}");
    row
}

/// What `sf-impact-extract` tells its caller, beside the file, when it runs.
fn notes(extract: &mut Command) -> String {
    String::from_utf8(run(extract).stderr).expect("the notes are UTF-8")
}

#[test]
fn the_file_starts_with_what_wrote_it_of_which_tree_with_which_scope_and_config() {
    let project = Project::committed("clean");

    let output = project.output();

    let row = provenance(&output);
    assert_eq!(
        row,
        json!({
            "row": "provenance",
            "tool": "sf-impact-extract",
            "version": env!("CARGO_PKG_VERSION"),
            "schema": 2,
            "tree": {"state": "commit", "commit": project.id("HEAD"), "tree": project.id("HEAD^{tree}")},
            // Where the project is in its repository: here, at the top.
            "root": ".",
            // Normalised, sorted, and without the path that leaves the root.
            "scope": {"packageDirectories": ["app", "plugins/billing"], "unpackagedMetadata": ["unpackaged"]},
            "config": row["config"],
        })
    );
    let schema = repository().join("schema/refs.schema.json");
    let schema = Schema::parse(&fs::read_to_string(schema).expect("the schema exists"));
    assert_eq!(schema.errors(&row), Vec::<String>::new());
    // One such row, and the rows of the tree after it as they were.
    let rows = output
        .lines()
        .filter(|line| line.contains(r#""row":"provenance""#));
    assert_eq!(rows.count(), 1);
    assert!(
        output
            .lines()
            .nth(1)
            .unwrap()
            .contains(r#""row":"scope_error""#)
    );
    assert!(output.contains(r#""symbol":"Alpha""#));
}

#[test]
fn the_same_commit_gives_the_same_bytes_in_another_directory_at_another_time() {
    let (here, there) = (
        Project::committed("here"),
        Project::committed_within("there", "project"),
    );

    let first = here.output();

    assert_eq!(provenance(&first)["tree"]["state"], "commit");
    assert_eq!(here.output(), first);
    // The commits are not the same, with the project in a directory of
    // the other: what is the same is all that the row does not say.
    let in_a_directory = there.output();
    assert_eq!(
        first.lines().skip(1).collect::<Vec<_>>(),
        in_a_directory.lines().skip(1).collect::<Vec<_>>()
    );
    let same = Project::committed("here-again");
    assert_eq!(same.output(), first);
    // No path of the machine, and nothing else that names it or the time.
    let row = provenance(&first);
    let mut fields: Vec<&str> = row
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort();
    assert_eq!(
        fields,
        [
            "config", "root", "row", "schema", "scope", "tool", "tree", "version"
        ]
    );
    assert!(
        !first
            .lines()
            .next()
            .unwrap()
            .contains(here.dir.file_name().unwrap().to_str().unwrap())
    );
}

#[test]
fn a_project_in_a_directory_of_its_repository_is_of_the_commit_too() {
    let project = Project::committed_within("subdirectory", "project");
    // Outside the project, and so nothing that is read.
    fs::write(project.top().join("NOTES.md"), "Not in the project.\n").unwrap();

    let row = project.provenance();

    assert_eq!(row["tree"]["commit"], project.id("HEAD"), "{row}");
    // Which project of the repository it is, from the top of it.
    assert_eq!(row["root"], "project");
}

#[test]
fn two_projects_of_one_commit_are_told_apart_by_where_they_are_in_the_repository() {
    // The same files at the same paths from each root, in one commit.
    let one = Project::files_within("two-projects", "one");
    let two = Project {
        dir: one.dir.clone(),
        within: "packages/two",
    };
    for (file, content) in [("sfdx-project.json", PROJECT), (CLASS, ALPHA)] {
        two.write(file, content);
    }
    run(git(&one.top()).args([
        "init",
        "--quiet",
        "--initial-branch=main",
        "--object-format=sha1",
    ]));
    one.commit("two projects");

    let (first, second) = (one.provenance(), two.provenance());

    assert_eq!(first["tree"], second["tree"]);
    assert_eq!(
        (&first["root"], &second["root"]),
        (&json!("one"), &json!("packages/two"))
    );
    // `two` is removed with `one`: they are in one directory.
    std::mem::forget(two);
}

#[test]
fn a_project_whose_package_directory_is_its_root_has_its_project_file_and_config_in_scope() {
    let project = Project::files("at-root");
    project.write(
        "sfdx-project.json",
        "{\"packageDirectories\": [{\"path\": \".\"}]}\n",
    );
    project.write("sf-impact.json", "{\"textualAllowList\": [\"Alpha\"]}\n");
    run(git(&project.top()).args([
        "init",
        "--quiet",
        "--initial-branch=main",
        "--object-format=sha1",
    ]));
    project.commit("base");

    let output = project.output();

    // Both are files of the scope, searched like any other, and the tree
    // is the commit: each is held to it once, by what was read of it.
    let row = provenance(&output);
    assert_eq!(row["tree"]["commit"], project.id("HEAD"), "{row}");
    assert_eq!(row["scope"]["packageDirectories"], json!([""]));
    let in_config = r#""referenced":"Alpha","referrer":null,"package":"","file":"sf-impact.json""#;
    assert!(output.contains(in_config), "{output}");
    // A change to either is a change to a file in scope and to what the
    // scope or the config is: the tree is dirty once, for the one file.
    project.write(
        "sf-impact.json",
        "{\"textualAllowList\": [\"Alpha\", \"Beta\"]}\n",
    );
    let output = run(&mut project.sf_impact_extract());
    let row = provenance(std::str::from_utf8(&output.stdout).unwrap());
    assert_eq!(row["tree"], json!({"state": "dirty"}));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("sf-impact.json is not as it is in")
    );
}

#[test]
fn a_repository_whose_objects_are_named_by_sha_256_is_read_as_its_commit_too() {
    let project = Project::files("sha-256");
    // Longer than a block of the hash, of a length that is no multiple of
    // one, and of no length at all.
    let long = format!(
        "public class Long {{\n{}}}\n",
        "    // A line.\n".repeat(500)
    );
    project.write("app/classes/Long.cls", &long);
    project.write("app/classes/Empty.cls", "");
    run(git(&project.top()).args([
        "init",
        "--quiet",
        "--initial-branch=main",
        "--object-format=sha256",
    ]));
    project.commit("base");

    let tree = project.provenance()["tree"].clone();

    assert_eq!(tree["commit"], project.id("HEAD"), "{tree}");
    assert_eq!(tree["commit"].as_str().unwrap().len(), 64);
    project.write(
        "app/classes/Long.cls",
        &long.replacen("A line", "a line", 1),
    );
    assert_eq!(project.provenance()["tree"], json!({"state": "dirty"}));
}

/// A directory to put first in `PATH`, with a `git` that writes `content`
/// to a file of the project whenever it is run, and then is git: another
/// process that changes the file while `sf-impact-extract` runs, at the moment
/// that it asks git anything, whenever that is.
fn git_that_writes(project: &Project, file: &str, content: &str) -> std::ffi::OsString {
    use std::os::unix::fs::PermissionsExt;
    let path = env::var_os("PATH").unwrap();
    let real = env::split_paths(&path)
        .map(|directory| directory.join("git"))
        .find(|git| git.is_file());
    let bin = project.dir.join("writing-git");
    fs::create_dir_all(&bin).unwrap();
    fs::write(project.dir.join("written"), content).unwrap();
    let script = format!(
        "#!/bin/sh\ncat '{}' > '{}'\nexec '{}' \"$@\"\n",
        project.dir.join("written").display(),
        project.root().join(file).display(),
        real.expect("git is in PATH").display()
    );
    fs::write(bin.join("git"), script).unwrap();
    fs::set_permissions(bin.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
    env::join_paths([bin].into_iter().chain(env::split_paths(&path))).unwrap()
}

#[test]
fn rows_of_a_file_that_was_not_the_commits_when_it_was_read_are_never_said_to_be_of_the_commit() {
    const STALE: &str = "public class Alpha {\n    Integer stale;\n}\n";
    // The file is restored to what the commit has while the tool runs,
    // and the other way about: made stale while it runs.
    for (name, on_disk, written) in [("restored", STALE, ALPHA), ("made-stale", ALPHA, STALE)] {
        let project = Project::committed(name);
        project.write(CLASS, on_disk);
        let racing = git_that_writes(&project, CLASS, written);

        let output = run(project.sf_impact_extract().env("PATH", &racing));

        // Whichever content was read, the tree is what that content is:
        // rows of the stale file never go with the name of the commit.
        let output = String::from_utf8(output.stdout).unwrap();
        let read_stale = output.contains(r#""symbol":"Alpha.stale""#);
        let state = provenance(&output)["tree"]["state"].clone();
        assert_eq!(state, if read_stale { "dirty" } else { "commit" }, "{name}");
        let left = fs::read_to_string(project.root().join(CLASS)).unwrap();
        assert_eq!(left, written, "{name}: the other process ran");
    }
}

#[test]
fn a_tree_whose_files_in_scope_are_not_those_of_the_commit_is_dirty() {
    type Change = fn(&Project);
    let changes: [(&str, &str, Change); 7] = [
        ("edited", CLASS, |project| {
            project.write(CLASS, "public class Alpha {\n    Integer one;\n}\n")
        }),
        ("untracked", "app/classes/Gamma.cls", |project| {
            project.write("app/classes/Gamma.cls", "class Gamma {\n}\n")
        }),
        ("ignored", "app/classes/Scratch.cls", |project| {
            // Git calls such a tree clean. What was read is still not the commit.
            fs::write(project.top().join(".git/info/exclude"), "Scratch.cls\n").unwrap();
            project.write("app/classes/Scratch.cls", "class Scratch {\n}\n");
        }),
        ("deleted", CLASS, |project| {
            fs::remove_file(project.root().join(CLASS)).unwrap()
        }),
        (
            "unpackaged",
            "unpackaged/classes/AlphaTest.cls",
            |project| {
                project.write(
                    "unpackaged/classes/AlphaTest.cls",
                    "@IsTest\nclass AlphaTest {\n    // Edited.\n}\n",
                )
            },
        ),
        ("project-file", "sfdx-project.json", |project| {
            project.write(
                "sfdx-project.json",
                &PROJECT.replace("{\"path\": \"../elsewhere\"}", "{\"path\": \"more\"}"),
            )
        }),
        ("config-file", "sf-impact.json", |project| {
            project.write("sf-impact.json", "{\"namespaces\": [\"npsp\"]}\n")
        }),
    ];

    for (name, file, change) in changes {
        let project = Project::committed(name);
        change(&project);

        let mut extract = project.sf_impact_extract();
        let output = run(&mut extract);

        let row = provenance(std::str::from_utf8(&output.stdout).unwrap());
        assert_eq!(row["tree"], json!({"state": "dirty"}), "{name}");
        // Which file it is, is said to whoever runs it: git may not say.
        let note = String::from_utf8(output.stderr).unwrap();
        let about: Vec<&str> = note
            .lines()
            .filter(|line| line.contains("`dirty`"))
            .collect();
        assert_eq!(about.len(), 1, "{name}: {note}");
        assert!(about[0].contains(file), "{name}: {note}");
        assert!(about[0].contains(&project.id("HEAD")), "{name}: {note}");
    }
}

#[test]
fn what_is_not_read_does_not_make_the_tree_dirty() {
    let project = Project::committed("out-of-scope");
    project.write("README.md", "Edited, and still not in scope.\n");
    project.write("refs.jsonl", "What an earlier run wrote.\n");
    project.write("elsewhere/classes/Delta.cls", "class Delta {\n}\n");
    // A mode alone is nothing that is read.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            project.root().join(CLASS),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }

    let mut extract = project.sf_impact_extract();
    let output = run(&mut extract);

    let row = provenance(std::str::from_utf8(&output.stdout).unwrap());
    assert_eq!(row["tree"]["commit"], project.id("HEAD"), "{row}");
    assert_eq!(
        String::from_utf8(output.stderr)
            .unwrap()
            .lines()
            .filter(|line| line.contains("note"))
            .count(),
        0
    );
}

#[test]
fn a_directory_of_the_commit_that_is_not_on_disk_makes_the_tree_dirty() {
    // As in a sparse checkout: the commit has a package that was not read.
    let project = Project::committed("sparse");
    fs::remove_dir_all(project.root().join("plugins")).unwrap();

    assert_eq!(project.provenance()["tree"], json!({"state": "dirty"}));
}

#[test]
fn a_file_that_git_converts_on_checkout_is_not_the_bytes_of_the_commit() {
    // The commit has line feeds; the working tree is given carriage
    // returns with them. What is read is what is on disk.
    let project = Project::committed("converted");
    project.write(".gitattributes", "*.cls text eol=crlf\n");
    project.commit("line ends by attribute");
    fs::remove_file(project.root().join(CLASS)).unwrap();
    run(git(&project.top()).args(["checkout", "--", CLASS]));
    let on_disk = fs::read_to_string(project.root().join(CLASS)).unwrap();
    assert!(on_disk.contains("\r\n"), "{on_disk:?}");

    assert_eq!(project.provenance()["tree"], json!({"state": "dirty"}));
}

#[test]
fn a_tree_that_git_names_no_commit_for_is_unknown() {
    let no_repository = Project::files("no-repository");
    let no_commit = Project::files("no-commit");
    run(git(&no_commit.top()).args(["init", "--quiet", "--initial-branch=main"]));

    for project in [&no_repository, &no_commit] {
        let mut extract = project.sf_impact_extract();
        let output = run(&mut extract);

        let row = provenance(std::str::from_utf8(&output.stdout).unwrap());
        assert_eq!(row["tree"], json!({"state": "unknown"}));
        // In no repository that is known, it is at no place in one.
        assert_eq!(row["root"], json!(null));
        let note = String::from_utf8(output.stderr).unwrap();
        assert_eq!(
            note.lines()
                .filter(|line| line.contains("`unknown`"))
                .count(),
            1,
            "{note}"
        );
    }

    // Nor is it known where there is no git to ask.
    let committed = Project::committed("no-git");
    let row = provenance(
        std::str::from_utf8(&run(committed.sf_impact_extract().env("PATH", "")).stdout).unwrap(),
    );
    assert_eq!(row["tree"], json!({"state": "unknown"}));
}

#[test]
fn the_config_is_hashed_as_it_takes_effect_and_not_as_it_is_written() {
    let hash = |name: &str, config: Option<&str>| {
        let project = Project::files(name);
        if let Some(config) = config {
            project.write("sf-impact.json", config);
        }
        project.provenance()["config"]
            .as_str()
            .expect("a hash")
            .to_string()
    };
    let default = hash("no-config", None);

    // 16 hexadecimal digits.
    assert_eq!(default.len(), 16);
    assert!(
        default.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "{default}"
    );
    // A file that says what the defaults are, and keys of other tools.
    assert_eq!(hash("empty-config", Some("{}")), default);
    assert_eq!(
        hash(
            "default-config",
            Some(r#"{"textualMinLength": 6, "testSuffixes": ["_T"]}"#)
        ),
        default
    );
    // What changes the rows changes the hash.
    let namespaces = hash("namespaces", Some(r#"{"namespaces": ["npsp", "Fflib"]}"#));
    assert_ne!(namespaces, default);
    assert_ne!(
        hash("min-length", Some(r#"{"textualMinLength": 4}"#)),
        default
    );
    assert_ne!(
        hash("allow-list", Some(r#"{"textualAllowList": ["Money"]}"#)),
        default
    );
    // Names are a set, and compare without case.
    assert_eq!(
        hash(
            "namespaces-again",
            Some(r#"{"namespaces": ["fflib", "NPSP", "npsp"]}"#)
        ),
        namespaces
    );
}

#[test]
fn a_clone_without_its_trees_is_unknown_and_nothing_is_fetched_by_any_git() {
    use std::os::unix::fs::PermissionsExt;
    let origin = Project::committed("treeless-origin");
    run(git(&origin.top()).args(["config", "uploadpack.allowFilter", "true"]));
    let path = env::var_os("PATH").unwrap();
    let real = env::split_paths(&path)
        .map(|directory| directory.join("git"))
        .find(|git| git.is_file())
        .expect("git is in PATH");
    // A `git` from before it could be told not to fetch: it does not know
    // the option, and fetches what a partial clone lacks when asked for it.
    let old = origin.dir.join("old-git");
    fs::create_dir_all(&old).unwrap();
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = --no-lazy-fetch ]; then\n  exit 129\nfi\nexec '{}' \"$@\"\n",
        real.display()
    );
    fs::write(old.join("git"), script).unwrap();
    fs::set_permissions(old.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
    let old_git = env::join_paths([old].into_iter().chain(env::split_paths(&path))).unwrap();

    for (name, path) in [("modern", &path), ("old", &old_git)] {
        // The commit is there and its trees are not; the files are on disk.
        let clone = Project::files(&format!("treeless-{name}"));
        run(git(&origin.dir)
            .args(["clone", "--quiet", "--filter=tree:0", "--no-checkout"])
            .arg(format!("file://{}", origin.top().display()))
            .arg(clone.dir.join("clone")));
        fs::rename(clone.dir.join("clone/.git"), clone.top().join(".git")).unwrap();
        // The remote is reached only by a program, which leaves a mark.
        let (program, mark) = (clone.dir.join("fetch-program"), clone.dir.join("fetched"));
        let script = format!("#!/bin/sh\necho ran >> '{}'\nexit 1\n", mark.display());
        fs::write(&program, script).unwrap();
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
        run(git(&clone.top()).args(["config", "remote.origin.url", "ssh://example.invalid/r"]));
        run(git(&clone.top())
            .args(["config", "core.sshCommand"])
            .arg(&program));
        let before = snapshot(&clone.top());

        let output = run(clone.sf_impact_extract().env("PATH", path));

        let row = provenance(std::str::from_utf8(&output.stdout).unwrap());
        assert_eq!(row["tree"], json!({"state": "unknown"}), "{name}");
        assert!(
            !mark.exists(),
            "{name}: git ran the program it fetches with"
        );
        assert!(
            before == snapshot(&clone.top()),
            "{name}: the clone was written to"
        );
    }
}

/// Every file under a directory with its content: what is there to be changed.
fn snapshot(directory: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            match entry.file_type().unwrap().is_dir() {
                true => pending.push(entry.path()),
                false => found.push((entry.path(), fs::read(entry.path()).unwrap())),
            }
        }
    }
    found.sort();
    found
}

#[test]
fn nothing_is_written_into_the_repository_and_what_its_caller_was_told_about_git_is_not_heard() {
    let project = Project::committed("touch-nothing");
    let commit = project.id("HEAD");
    // An index that is older than the files, as after a checkout by
    // another tool: git would write it anew to say that the tree is clean.
    project.write(CLASS, ALPHA);
    let before = snapshot(&project.top());

    let elsewhere = project.dir.join("no-such-repository");
    let row = provenance(
        std::str::from_utf8(
            &run(project
                .sf_impact_extract()
                .env("GIT_DIR", &elsewhere)
                .env("GIT_WORK_TREE", &elsewhere)
                .env("GIT_INDEX_FILE", elsewhere.join("index")))
            .stdout,
        )
        .unwrap(),
    );

    assert_eq!(row["tree"]["commit"], commit, "{row}");
    assert!(
        before == snapshot(&project.top()),
        "the repository was written to"
    );
}

#[test]
fn a_repository_of_another_owner_is_read_only_when_told_to_trust_it() {
    use std::os::unix::fs::PermissionsExt;
    let project = Project::committed("trust-repo");
    // A `git` that takes every repository to be another user's, as git
    // does in a container whose workspace was made by someone else.
    let path = env::var_os("PATH").unwrap();
    let real = env::split_paths(&path)
        .map(|directory| directory.join("git"))
        .find(|git| git.is_file());
    let bin = project.dir.join("other-owner");
    fs::create_dir_all(&bin).unwrap();
    let script = format!(
        "#!/bin/sh\nexport GIT_TEST_ASSUME_DIFFERENT_OWNER=1\nexec '{}' \"$@\"\n",
        real.expect("git is in PATH").display()
    );
    fs::write(bin.join("git"), script).unwrap();
    fs::set_permissions(bin.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
    let other_owner = env::join_paths([bin].into_iter().chain(env::split_paths(&path))).unwrap();

    // Not without being told to: the tree is unknown, and the note says how.
    let mut untrusted = project.sf_impact_extract();
    let output = run(untrusted.env("PATH", &other_owner));
    let row = provenance(std::str::from_utf8(&output.stdout).unwrap());
    assert_eq!(row["tree"], json!({"state": "unknown"}));
    assert!(notes(project.sf_impact_extract().env("PATH", &other_owner)).contains("--trust-repo"));

    let mut trusted = project.sf_impact_extract();
    let output = run(trusted.env("PATH", &other_owner).arg("--trust-repo"));
    let row = provenance(std::str::from_utf8(&output.stdout).unwrap());
    assert_eq!(row["tree"]["commit"], project.id("HEAD"), "{row}");
}
