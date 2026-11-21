//! Runs `sf-impact-delta` on real git repositories. Each test makes a small
//! repository in a temporary directory, commits a base, makes one change,
//! commits it as the head and runs the binary from the one to the other.
//!
//! What a test is about is asserted in the test. The whole output is also
//! compared with `fixtures/delta/<test>.jsonl` and checked against
//! `schema/delta.schema.json`. To accept a deliberate change:
//! `UPDATE_GOLDEN=1 cargo test -p sf-impact-delta --test delta`

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

/// `git` as the tests run it: without the configuration and the hooks of
/// whoever runs them, with a fixed identity and a fixed date, so that the
/// same content is the same commit on every machine.
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

fn said(command: &mut Command) -> String {
    let output = run(command);
    String::from_utf8(output.stdout)
        .expect("the output is UTF-8")
        .trim_end()
        .to_string()
}

const PROJECT: &str = "{\"packageDirectories\": [{\"path\": \"app\"}]}\n";
const ALPHA: &str = "public class Alpha {\n    public Integer first() {\n        return 1;\n    }\n\n    \
                     public Integer second() {\n        return 2;\n    }\n}\n";
const CLASS: &str = "app/classes/Alpha.cls";

/// A git repository in a temporary directory, removed on drop. Its base
/// is a project file and one class.
struct Repo {
    name: &'static str,
    dir: PathBuf,
    /// The project's directory within the repository; empty for the top.
    project: &'static str,
}

impl Repo {
    fn new(name: &'static str) -> Repo {
        Repo::within(name, "")
    }

    fn within(name: &'static str, project: &'static str) -> Repo {
        let repo = Repo {
            name,
            dir: env::temp_dir().join(format!("sf-impact-delta-{}-{name}", std::process::id())),
            project,
        };
        let _ = fs::remove_dir_all(&repo.dir);
        fs::create_dir_all(repo.top()).unwrap();
        run(git(&repo.top()).args([
            "init",
            "--quiet",
            "--initial-branch=main",
            "--object-format=sha1",
        ]));
        repo.write("sfdx-project.json", PROJECT);
        repo.write(CLASS, ALPHA);
        repo.commit("base");
        repo
    }

    /// The top of the repository.
    fn top(&self) -> PathBuf {
        self.dir.join("repo")
    }

    /// The project: what `--root` is given.
    fn root(&self) -> PathBuf {
        self.top().join(self.project)
    }

    fn write(&self, file: &str, content: impl AsRef<[u8]>) {
        let path = self.root().join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn replace(&self, file: &str, from: &str, to: &str) {
        let content = fs::read_to_string(self.root().join(file)).unwrap();
        assert_eq!(content.matches(from).count(), 1, "{from:?} in {file}");
        self.write(file, content.replace(from, to));
    }

    fn remove(&self, file: &str) {
        fs::remove_file(self.root().join(file)).unwrap();
    }

    fn rename(&self, from: &str, to: &str) {
        let to = self.root().join(to);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::rename(self.root().join(from), to).unwrap();
    }

    fn commit(&self, message: &str) {
        run(git(&self.top()).args(["add", "--all"]));
        run(git(&self.top()).args(["commit", "--quiet", "--message", message]));
    }

    /// What git says of a revision or of a file in one: an id.
    fn id(&self, revision: &str) -> String {
        said(git(&self.top()).args(["rev-parse", revision]))
    }

    /// The binary, on this repository's project.
    fn sf_impact_delta(&self) -> Command {
        let mut delta = Command::new(env!("CARGO_BIN_EXE_sf-impact-delta"));
        delta.arg("--root").arg(self.root());
        delta
    }

    /// The output from the commit before the last to the last.
    fn output(&self) -> String {
        let output = run(self
            .sf_impact_delta()
            .args(["--base", "HEAD~1", "--head", "HEAD"]));
        String::from_utf8(output.stdout).expect("the output is UTF-8")
    }

    /// Commits the change as the head and runs `sf-impact-delta` from the commit
    /// before it. The output is checked against the test's golden file and
    /// the schema.
    fn delta(&self) -> Vec<Value> {
        self.commit("head");
        let output = self.output();
        check_golden(self.name, &output);
        rows(&output)
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn rows(output: &str) -> Vec<Value> {
    let rows = output.lines().map(serde_json::from_str);
    rows.collect::<Result<_, _>>().expect("every line is JSON")
}

fn check_golden(name: &str, output: &str) {
    let golden = repository().join(format!("fixtures/delta/{name}.jsonl"));
    if env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&golden, output).expect("the golden file is writable");
    }
    let expected = fs::read_to_string(&golden).unwrap_or_else(|e| panic!("{golden:?}: {e}"));
    assert_eq!(
        output, expected,
        "the output differs from fixtures/delta/{name}.jsonl. \
         If the change is intended, rerun with UPDATE_GOLDEN=1 and review the diff."
    );
    check_schema(name, output);
}

fn check_schema(name: &str, output: &str) {
    let schema = repository().join("schema/delta.schema.json");
    let schema = Schema::parse(&fs::read_to_string(schema).expect("the schema exists"));
    for (number, row) in rows(output).iter().enumerate() {
        let errors = schema.errors(row);
        assert!(errors.is_empty(), "{name} line {}: {errors:?}", number + 1);
    }
}

/// Status, path at base and path at head of every file row.
fn files(rows: &[Value]) -> Vec<Value> {
    let files = rows.iter().filter(|row| row["row"] == "file");
    files
        .map(|row| json!([row["status"], row["base"]["file"], row["head"]["file"]]))
        .collect()
}

fn the_file(rows: &[Value]) -> &Value {
    let mut files = rows.iter().filter(|row| row["row"] == "file");
    let file = files.next().expect("a file row");
    assert!(files.next().is_none(), "one file row");
    file
}

/// The text the output gives for a blob, if it gives any.
fn text<'a>(rows: &'a [Value], id: &Value) -> Option<&'a str> {
    let mut blobs = rows
        .iter()
        .filter(|row| row["row"] == "blob" && row["id"] == *id);
    let blob = blobs.next()?;
    assert!(blobs.next().is_none(), "one row for the blob {id}");
    blob["text"].as_str()
}

fn hunk(base_start: u32, base_lines: u32, head_start: u32, head_lines: u32) -> Value {
    json!({
        "base_start": base_start, "base_lines": base_lines,
        "head_start": head_start, "head_lines": head_lines,
    })
}

// The four things that can become of a file.

#[test]
fn a_file_changed_in_place_has_its_hunks_and_the_text_of_both_versions() {
    let repo = Repo::new("modified");
    repo.replace(CLASS, "return 2;", "return 20;");

    let rows = repo.delta();

    assert_eq!(files(&rows), [json!(["modified", CLASS, CLASS])]);
    let file = the_file(&rows);
    assert_eq!(file["hunks"], json!([hunk(7, 1, 7, 1)]));
    assert_eq!(file["base"]["id"], repo.id(&format!("HEAD~1:{CLASS}")));
    assert_eq!(file["head"]["id"], repo.id(&format!("HEAD:{CLASS}")));
    assert_eq!(file["base"]["mode"], "100644");
    assert_eq!(
        (&file["base"]["binary"], &file["head"]["binary"]),
        (&json!(false), &json!(false))
    );
    assert_eq!(text(&rows, &file["base"]["id"]), Some(ALPHA));
    assert_eq!(
        text(&rows, &file["head"]["id"]),
        Some(ALPHA.replace("return 2;", "return 20;").as_str())
    );
}

#[test]
fn an_added_file_has_no_base_and_a_deleted_file_no_head() {
    let repo = Repo::new("added-and-deleted");
    repo.write("app/classes/Beta.cls", "public class Beta {\n}\n");
    repo.remove(CLASS);

    let rows = repo.delta();

    assert_eq!(
        files(&rows),
        [
            json!(["deleted", CLASS, null]),
            json!(["added", null, "app/classes/Beta.cls"]),
        ]
    );
    // Neither has lines that differ from another version: there is none.
    assert!(rows.iter().all(|row| row.get("hunks").is_none()));
    assert_eq!(text(&rows, &rows[3]["base"]["id"]), Some(ALPHA));
    assert_eq!(
        text(&rows, &rows[4]["head"]["id"]),
        Some("public class Beta {\n}\n")
    );
}

#[test]
fn the_same_blob_under_another_path_is_a_file_that_moved() {
    let repo = Repo::new("moved");
    repo.rename(CLASS, "app/billing/Alpha.cls");

    let rows = repo.delta();

    assert_eq!(
        files(&rows),
        [json!(["moved", CLASS, "app/billing/Alpha.cls"])]
    );
    let file = the_file(&rows);
    assert_eq!(file["base"]["id"], file["head"]["id"]);
    assert_eq!(file.get("hunks"), None);
    assert_eq!(text(&rows, &file["head"]["id"]), Some(ALPHA));
}

#[test]
fn a_file_that_moves_and_changes_is_one_deleted_and_one_added() {
    let repo = Repo::new("moved-and-edited");
    repo.rename(CLASS, "app/billing/Alpha.cls");
    repo.replace("app/billing/Alpha.cls", "return 2;", "return 20;");

    let rows = repo.delta();

    // No two files are taken for one because they are alike.
    assert_eq!(
        files(&rows),
        [
            json!(["added", null, "app/billing/Alpha.cls"]),
            json!(["deleted", CLASS, null]),
        ]
    );
}

#[test]
fn files_of_one_blob_that_move_are_paired_by_name_first_and_then_in_order() {
    let repo = Repo::new("moved-same-blob");
    let companion = "<ApexClass/>\n";
    for file in ["one/A.xml", "one/B.xml", "one/C.xml", "one/D.xml"] {
        repo.write(file, companion);
    }
    repo.commit("four files that are the same blob");
    // `B` and `A` each go to a directory of their own, in an order that
    // is not the order of the files they were; `C` takes another name;
    // `D` is deleted.
    repo.rename("one/B.xml", "first/B.xml");
    repo.rename("one/A.xml", "second/A.xml");
    repo.rename("one/C.xml", "third/E.xml");
    repo.remove("one/D.xml");

    let rows = repo.delta();

    assert_eq!(
        files(&rows),
        [
            json!(["moved", "one/B.xml", "first/B.xml"]),
            json!(["deleted", "one/D.xml", null]),
            json!(["moved", "one/A.xml", "second/A.xml"]),
            json!(["moved", "one/C.xml", "third/E.xml"]),
        ]
    );
    // The text of the blob they all are is given once.
    assert_eq!(
        rows.iter().filter(|row| row["text"] == companion).count(),
        1
    );
}

#[test]
fn a_change_of_mode_alone_is_the_same_blob_with_another_mode() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new("mode-only");
    fs::set_permissions(repo.root().join(CLASS), fs::Permissions::from_mode(0o755)).unwrap();

    let rows = repo.delta();

    assert_eq!(files(&rows), [json!(["modified", CLASS, CLASS])]);
    let file = the_file(&rows);
    assert_eq!(file["base"]["id"], file["head"]["id"]);
    assert_eq!(
        (&file["base"]["mode"], &file["head"]["mode"]),
        (&json!("100644"), &json!("100755"))
    );
    assert_eq!(file.get("hunks"), None);
}

// What is text, and what is binary.

#[test]
fn not_text_on_either_side_has_no_hunks() {
    let repo = Repo::new("binary");
    let with_nul_at = |at: usize| {
        let mut bytes = "line\n".repeat(2000).into_bytes();
        bytes[at] = 0;
        bytes
    };
    repo.write("app/was-not-text.dat", with_nul_at(0));
    repo.write("app/becomes-not-text.dat", "text\n");
    repo.write("app/stays-not-text.dat", with_nul_at(7999));
    repo.write("app/late-nul.dat", with_nul_at(8000));
    repo.commit("files of bytes");
    repo.write("app/was-not-text.dat", "text\n");
    repo.write("app/becomes-not-text.dat", b"caf\xe9\n");
    repo.write("app/stays-not-text.dat", b"caf\xe9\n");
    let mut late = with_nul_at(8000);
    late[1] = b'I';
    repo.write("app/late-nul.dat", late);

    let rows = repo.delta();

    let binary = |row: &Value| {
        json!([
            row["head"]["file"],
            row["base"]["binary"],
            row["head"]["binary"]
        ])
    };
    let files: Vec<&Value> = rows.iter().filter(|row| row["row"] == "file").collect();
    assert_eq!(
        files.iter().map(|row| binary(row)).collect::<Vec<_>>(),
        [
            json!(["app/becomes-not-text.dat", false, true]),
            json!(["app/late-nul.dat", true, true]),
            json!(["app/stays-not-text.dat", true, true]),
            json!(["app/was-not-text.dat", true, false]),
        ]
    );
    // Lines are compared only where both versions are text.
    let hunks: Vec<bool> = files.iter().map(|row| row.get("hunks").is_some()).collect();
    assert_eq!(hunks, [false, false, false, false]);
}

#[test]
fn text_in_utf16_is_compared_as_decoded_text() {
    let repo = Repo::new("utf-16");
    let utf16 = |source: &str| -> Vec<u8> {
        let units = source.encode_utf16().flat_map(u16::to_le_bytes);
        [0xFF, 0xFE].into_iter().chain(units).collect()
    };
    repo.write(
        "app/classes/Wide.cls",
        utf16("public class Wide {\n    void a() {}\n}\n"),
    );
    repo.commit("a class in UTF-16");
    repo.write(
        "app/classes/Wide.cls",
        utf16("public class Wide {\n    void b() {}\n}\n"),
    );

    let rows = repo.delta();

    let file = the_file(&rows);
    assert_eq!(
        (&file["base"]["binary"], &file["head"]["binary"]),
        (&json!(false), &json!(false))
    );
    assert_eq!(file["hunks"], json!([hunk(2, 1, 2, 1)]));
    // As the extractor reads it: without its byte order mark.
    assert_eq!(
        text(&rows, &file["head"]["id"]),
        Some("public class Wide {\n    void b() {}\n}\n")
    );
}

#[test]
fn bytes_that_are_no_text_have_no_text_row() {
    let repo = Repo::new("not-text");
    repo.write("app/image.png", b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR");
    // No NUL, and not UTF-8 either: Latin-1.
    repo.write("app/latin1.txt", b"caf\xe9\n");

    let rows = repo.delta();

    assert_eq!(
        files(&rows),
        [
            json!(["added", null, "app/image.png"]),
            json!(["added", null, "app/latin1.txt"]),
        ]
    );
    assert_eq!(
        rows.iter().filter(|row| row["row"] == "blob").count(),
        1,
        "the project file"
    );
    assert_eq!(
        (&rows[3]["head"]["binary"], &rows[4]["head"]["binary"]),
        (&json!(true), &json!(true))
    );
}

#[test]
fn a_line_end_that_changes_changes_its_line() {
    let repo = Repo::new("crlf-to-lf");
    repo.write(CLASS, ALPHA.replace('\n', "\r\n"));
    repo.commit("a class with CRLF line ends");
    repo.write(CLASS, ALPHA.replace("return 2;", "return 20;"));

    let rows = repo.delta();

    // Every line, not the one that was edited: nothing is ignored.
    assert_eq!(files(&rows), [json!(["modified", CLASS, CLASS])]);
    assert_eq!(the_file(&rows)["hunks"], json!([hunk(1, 9, 1, 9)]));
}

// What is not a file with content of its own.

/// A submodule at `path`, pinned to a commit that this repository does not
/// have, as the commit of a submodule is not.
fn pin(repo: &Repo, path: &str, commit: &str) {
    // The directory of a submodule that is not checked out: empty.
    fs::create_dir_all(repo.top().join(path)).unwrap();
    let entry = format!("160000,{commit},{path}");
    run(git(&repo.top()).args(["update-index", "--add", "--cacheinfo", &entry]));
}

#[test]
fn a_submodule_is_the_commit_it_is_at_and_is_not_read() {
    let repo = Repo::new("submodule");
    pin(
        &repo,
        "app/vendor",
        "1111111111111111111111111111111111111111",
    );
    repo.commit("a submodule");
    pin(
        &repo,
        "app/vendor",
        "2222222222222222222222222222222222222222",
    );

    let rows = repo.delta();

    assert_eq!(
        files(&rows),
        [json!(["modified", "app/vendor", "app/vendor"])]
    );
    let file = the_file(&rows);
    assert_eq!(
        (&file["base"], &file["head"]),
        (
            &json!({"file": "app/vendor", "id": "1111111111111111111111111111111111111111", "mode": "160000", "binary": false}),
            &json!({"file": "app/vendor", "id": "2222222222222222222222222222222222222222", "mode": "160000", "binary": false}),
        )
    );
    assert_eq!(file.get("hunks"), None);
    assert_eq!(text(&rows, &file["head"]["id"]), None);
}

#[test]
fn a_symbolic_link_is_a_link_and_is_not_followed() {
    let repo = Repo::new("symlink");
    std::os::unix::fs::symlink("classes", repo.root().join("app/link")).unwrap();
    repo.commit("a link");
    // The link is turned elsewhere, and the class becomes a link.
    fs::remove_file(repo.root().join("app/link")).unwrap();
    std::os::unix::fs::symlink("../outside", repo.root().join("app/link")).unwrap();
    repo.remove(CLASS);
    std::os::unix::fs::symlink("Other.cls", repo.root().join(CLASS)).unwrap();

    let rows = repo.delta();

    assert_eq!(
        files(&rows),
        [
            json!(["modified", CLASS, CLASS]),
            json!(["modified", "app/link", "app/link"])
        ]
    );
    assert_eq!(
        (&rows[3]["base"]["mode"], &rows[3]["head"]["mode"]),
        (&json!("100644"), &json!("120000"))
    );
    assert_eq!(
        (&rows[4]["base"]["mode"], &rows[4]["head"]["mode"]),
        (&json!("120000"), &json!("120000"))
    );
    // Where a link leads is not content to compare or to give.
    assert!(rows.iter().all(|row| row.get("hunks").is_none()));
    assert_eq!(text(&rows, &rows[3]["head"]["id"]), None);
    assert_eq!(text(&rows, &rows[4]["head"]["id"]), None);
    assert_eq!(text(&rows, &rows[3]["base"]["id"]), Some(ALPHA));
}

// Paths.

#[test]
fn a_path_is_given_as_it_is_whatever_characters_it_has() {
    let repo = Repo::new("paths");
    repo.write(
        "app/shipping area/Parcel \"one\".cls",
        "public class Parcel {\n}\n",
    );
    repo.write("app/exp\u{e9}dition/Colis.cls", "public class Colis {\n}\n");

    let rows = repo.delta();

    assert_eq!(
        files(&rows),
        [
            json!(["added", null, "app/exp\u{e9}dition/Colis.cls"]),
            json!(["added", null, "app/shipping area/Parcel \"one\".cls"]),
        ]
    );
}

#[test]
fn a_project_below_the_top_of_its_repository_has_paths_from_its_own_root() {
    let repo = Repo::within("project-in-subdirectory", "salesforce");
    repo.replace(CLASS, "return 2;", "return 20;");
    fs::write(repo.top().join("build.sh"), "#!/bin/sh\n").unwrap();

    let rows = repo.delta();

    assert_eq!(
        files(&rows),
        [
            json!(["added", null, "../build.sh"]),
            json!(["modified", CLASS, CLASS])
        ]
    );
    // The project file is the one at the root, not at the top.
    assert_eq!(
        rows[1]["project_file"],
        repo.id("HEAD~1:salesforce/sfdx-project.json")
    );
}

#[test]
fn a_file_that_comes_with_a_new_directory_says_which() {
    let repo = Repo::new("new-directory");
    repo.write("app/lwc/refund/__tests__/data/one.json", "{}\n");
    repo.write("app/classes/Beta.cls", "public class Beta {\n}\n");
    repo.rename(CLASS, "app/billing/core/Alpha.cls");

    let rows = repo.delta();

    let directories = rows.iter().filter(|row| row["row"] == "file");
    let directories: Vec<Value> = directories
        .map(|row| json!([row["head"]["file"], row["new_directory"]]))
        .collect();
    assert_eq!(
        directories,
        [
            // The topmost that is new, not the nearest.
            json!(["app/billing/core/Alpha.cls", "app/billing"]),
            // `app/classes` was there.
            json!(["app/classes/Beta.cls", null]),
            json!(["app/lwc/refund/__tests__/data/one.json", "app/lwc"]),
        ]
    );
}

// The two commits.

#[test]
fn the_two_commits_are_named_with_the_project_and_config_files_they_have() {
    let repo = Repo::new("commits");
    repo.write("sf-impact.json", "{\"namespaces\": [\"npsp\"]}\n");
    repo.replace(CLASS, "return 2;", "return 20;");

    let rows = repo.delta();

    assert_eq!(
        rows[1],
        json!({
            "row": "commit", "side": "base", "id": repo.id("HEAD~1"),
            "project_file": repo.id("HEAD~1:sfdx-project.json"), "config_file": null,
        })
    );
    assert_eq!(
        rows[2],
        json!({
            "row": "commit", "side": "head", "id": repo.id("HEAD"),
            "project_file": repo.id("HEAD:sfdx-project.json"),
            "config_file": repo.id("HEAD:sf-impact.json"),
        })
    );
    // The project file did not change, and its text is given all the same.
    assert_eq!(files(&rows).len(), 2);
    assert_eq!(text(&rows, &rows[2]["project_file"]), Some(PROJECT));
    assert_eq!(
        text(&rows, &rows[2]["config_file"]),
        Some("{\"namespaces\": [\"npsp\"]}\n")
    );
}

#[test]
fn a_merge_commit_as_the_head_is_compared_as_the_tree_it_is() {
    let repo = Repo::new("merge-head");
    run(git(&repo.top()).args(["switch", "--quiet", "--create", "feature"]));
    repo.replace(CLASS, "return 2;", "return 20;");
    repo.commit("feature");
    run(git(&repo.top()).args(["switch", "--quiet", "main"]));
    repo.write("app/classes/Beta.cls", "public class Beta {\n}\n");
    repo.commit("main moves on");
    run(git(&repo.top()).args([
        "merge",
        "--quiet",
        "--no-ff",
        "--message",
        "merge",
        "feature",
    ]));

    // From the first parent: what the merge brought to main.
    let output = repo.output();
    check_golden(repo.name, &output);
    assert_eq!(files(&rows(&output)), [json!(["modified", CLASS, CLASS])]);

    // From before either side: both.
    let output = run(repo
        .sf_impact_delta()
        .args(["--base", "HEAD~2", "--head", "HEAD"]));
    let output = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        files(&rows(&output)),
        [
            json!(["modified", CLASS, CLASS]),
            json!(["added", null, "app/classes/Beta.cls"]),
        ]
    );
}

#[test]
fn the_merge_base_of_a_branch_stands_in_for_a_base_that_is_no_ancestor() {
    let repo = Repo::new("merge-base");
    run(git(&repo.top()).args(["switch", "--quiet", "--create", "feature"]));
    repo.replace(CLASS, "return 2;", "return 20;");
    repo.commit("feature");
    run(git(&repo.top()).args(["switch", "--quiet", "main"]));
    repo.write("app/classes/Beta.cls", "public class Beta {\n}\n");
    repo.commit("main moves on");

    let from_merge_base =
        run(repo
            .sf_impact_delta()
            .args(["--merge-base", "main", "--head", "feature"]));
    let from_merge_base = String::from_utf8(from_merge_base.stdout).unwrap();
    let from_main = run(repo
        .sf_impact_delta()
        .args(["--base", "main", "--head", "feature"]));
    let from_main = rows(&String::from_utf8(from_main.stdout).unwrap());

    check_golden(repo.name, &from_merge_base);
    let from_merge_base = rows(&from_merge_base);
    // Only what the branch did; and the base is said to be where it left.
    assert_eq!(files(&from_merge_base), [json!(["modified", CLASS, CLASS])]);
    assert_eq!(from_merge_base[1]["id"], repo.id("main~1"));
    // As given, the base is main, and what main did since is undone.
    assert_eq!(
        files(&from_main),
        [
            json!(["modified", CLASS, CLASS]),
            json!(["deleted", "app/classes/Beta.cls", null]),
        ]
    );
    assert_eq!(from_main[1]["id"], repo.id("main"));
}

// Repositories that keep their objects elsewhere, or not all of them.

/// A repository with one edit to the class, and the output for it.
fn edited(name: &'static str) -> (Repo, String) {
    let repo = Repo::new(name);
    repo.replace(CLASS, "return 2;", "return 20;");
    repo.write("app/classes/Beta.cls", "public class Beta {\n}\n");
    repo.commit("head");
    let output = repo.output();
    assert!(!output.is_empty());
    (repo, output)
}

/// The output of the binary on another repository of the same commits.
fn output_in(directory: &Path) -> String {
    let output = run(Command::new(env!("CARGO_BIN_EXE_sf-impact-delta"))
        .args(["--base", "HEAD~1", "--head", "HEAD", "--root"])
        .arg(directory));
    String::from_utf8(output.stdout).unwrap()
}

/// A clone of a repository that was not given all of it: `filter` says
/// what was left out.
fn partial_clone(repo: &Repo, name: &str, filter: &str, checkout: &[&str]) -> PathBuf {
    run(git(&repo.top()).args(["config", "uploadpack.allowFilter", "true"]));
    let clone = repo.dir.join(name);
    let origin = format!("file://{}", repo.top().display());
    run(git(&repo.dir)
        .args(["clone", "--quiet", filter])
        .args(checkout)
        .arg(&origin)
        .arg(&clone));
    clone
}

/// Makes the remote of a clone one that git can only reach by running a
/// program of the repository's choosing, which leaves a mark if it is
/// run. The mark is the path returned.
fn fetches_by_running_a_program(repo: &Repo, clone: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let (program, mark) = (repo.dir.join("fetch-program"), repo.dir.join("fetched"));
    let script = format!("#!/bin/sh\necho ran >> '{}'\nexit 1\n", mark.display());
    fs::write(&program, script).unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    run(git(clone).args([
        "config",
        "remote.origin.url",
        "ssh://example.invalid/repository",
    ]));
    run(git(clone).args(["config", "core.sshCommand"]).arg(&program));
    mark
}

/// A run in `directory` that is to fail: what it said. Nothing may be
/// written by it, into the clone or out of it, and no program may be run.
fn refused_in(directory: &Path, mark: &Path, path: Option<&Path>, revisions: &[&str]) -> String {
    let before = snapshot(directory);
    let mut delta = Command::new(env!("CARGO_BIN_EXE_sf-impact-delta"));
    if let Some(path) = path {
        delta.env("PATH", path);
    }
    let (code, message) = failure(delta.args(revisions).arg("--root").arg(directory));
    assert_eq!(code, Some(1), "{message}");
    assert!(!mark.exists(), "git ran the program it fetches with");
    assert!(
        snapshot(directory) == before,
        "a file under {directory:?} changed"
    );
    message
}

#[test]
fn a_clone_without_its_blobs_is_refused_and_nothing_is_fetched() {
    let (repo, _) = edited("partial-clone");
    let blobs = ["HEAD", "HEAD~1"].map(|commit| repo.id(&format!("{commit}:{CLASS}")));
    let without_blobs = partial_clone(&repo, "blobless", "--filter=blob:none", &["--no-checkout"]);
    let without_trees = partial_clone(&repo, "treeless", "--filter=tree:0", &["--no-checkout"]);
    let between = ["--base", "HEAD~1", "--head", "HEAD"];

    for clone in [&without_blobs, &without_trees] {
        let mark = fetches_by_running_a_program(&repo, clone);
        let message = refused_in(clone, &mark, None, &between);
        assert!(message.contains("fetches nothing"), "{message}");
        // What git said, without advice of its about what it was told
        // here: where to look for grafts.
        assert!(!message.contains("hint:"), "{message}");
    }
    // The object that is not there is named.
    let mark = repo.dir.join("fetched");
    let message = refused_in(&without_blobs, &mark, None, &between);
    assert!(blobs.iter().any(|blob| message.contains(blob)), "{message}");
}

#[test]
fn a_clone_that_was_given_the_files_that_differ_is_read_and_fetches_nothing() {
    let (repo, output) = edited("partial-clone-with-its-files");
    let clone = partial_clone(&repo, "clone", "--filter=blob:none", &[]);
    // What the README says to do first: a diff fetches the files it reads.
    run(git(&clone).args(["diff", "HEAD~1", "HEAD"]));
    let mark = fetches_by_running_a_program(&repo, &clone);
    let before = snapshot(&clone);

    assert_eq!(output_in(&clone), output);
    assert!(!mark.exists(), "git ran the program it fetches with");
    assert!(snapshot(&clone) == before, "a file of the clone changed");
}

/// A directory to put first in `PATH`, with a `git` that is the real one
/// from before it had `--no-lazy-fetch`: it does not know the option, and
/// fetches what a partial clone lacks whenever it is asked to read it.
fn git_that_cannot_be_told_not_to_fetch(repo: &Repo) -> std::ffi::OsString {
    use std::os::unix::fs::PermissionsExt;
    let path = env::var_os("PATH").unwrap();
    let real = env::split_paths(&path)
        .map(|directory| directory.join("git"))
        .find(|git| git.is_file())
        .expect("git is in PATH");
    let bin = repo.dir.join("old-git");
    fs::create_dir_all(&bin).unwrap();
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = --no-lazy-fetch ]; then\n  echo 'unknown option: --no-lazy-fetch' >&2\n  \
         exit 129\nfi\nexec '{}' \"$@\"\n",
        real.display()
    );
    fs::write(bin.join("git"), script).unwrap();
    fs::set_permissions(bin.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
    env::join_paths([bin].into_iter().chain(env::split_paths(&path))).unwrap()
}

#[test]
fn a_git_that_cannot_be_told_not_to_fetch_reads_a_repository_only_if_all_of_it_is_there() {
    let repo = Repo::new("old-git");
    repo.replace(CLASS, "return 1;", "return 10;");
    repo.commit("an earlier change");
    repo.replace(CLASS, "return 2;", "return 20;");
    repo.commit("head");
    let output = repo.output();
    let old_git = git_that_cannot_be_told_not_to_fetch(&repo);
    let with_old_git = |directory: &Path, revisions: &[&str]| {
        let output = run(Command::new(env!("CARGO_BIN_EXE_sf-impact-delta"))
            .env("PATH", &old_git)
            .args(revisions)
            .arg("--root")
            .arg(directory));
        String::from_utf8(output.stdout).unwrap()
    };
    let between = ["--base", "HEAD~1", "--head", "HEAD"];
    let from_where_they_part = ["--merge-base", "HEAD~1", "--head", "HEAD"];

    // A repository that has all its objects is read as by any git.
    assert_eq!(with_old_git(&repo.top(), &between), output);
    assert_eq!(with_old_git(&repo.top(), &from_where_they_part), output);

    // One that lacks a file of either commit is not read.
    let cold = partial_clone(&repo, "cold", "--filter=blob:none", &["--no-checkout"]);
    let mark = fetches_by_running_a_program(&repo, &cold);
    let message = refused_in(&cold, &mark, Some(old_git.as_ref()), &between);
    assert!(message.contains("fetches nothing"), "{message}");
    assert!(message.contains("does not have the object"), "{message}");

    // Nor is one that lacks the trees: the tree of a commit that is there
    // is not fetched to be named or to be listed.
    let treeless = partial_clone(&repo, "treeless", "--filter=tree:0", &["--no-checkout"]);
    let mark = fetches_by_running_a_program(&repo, &treeless);
    let message = refused_in(&treeless, &mark, Some(old_git.as_ref()), &between);
    assert!(message.contains("fetches nothing"), "{message}");
    assert!(message.contains("does not have the object"), "{message}");

    // Nor is one that git takes to be promised objects by a setting
    // other than the one that says so: here the filter alone is left.
    let filter_only = partial_clone(
        &repo,
        "filter-only",
        "--filter=blob:none",
        &["--no-checkout"],
    );
    run(git(&filter_only).args(["config", "--unset", "remote.origin.promisor"]));
    let mark = fetches_by_running_a_program(&repo, &filter_only);
    let trusted = ["--trust-repo", "--base", "HEAD~1", "--head", "HEAD"];
    for revisions in [&between[..], &trusted[..]] {
        let message = refused_in(&filter_only, &mark, Some(old_git.as_ref()), revisions);
        assert!(message.contains("fetches nothing"), "{message}");
    }

    // One that has every file of both commits is.
    let whole = partial_clone(&repo, "whole", "--filter=blob:none", &[]);
    run(git(&whole).args(["diff", "HEAD~1", "HEAD"]));
    let mark = fetches_by_running_a_program(&repo, &whole);
    let before = snapshot(&whole);
    assert_eq!(with_old_git(&whole, &between), output);
    assert_eq!(with_old_git(&whole, &from_where_they_part), output);
    assert!(!mark.exists(), "git ran the program it fetches with");
    assert!(snapshot(&whole) == before, "a file of the clone changed");

    // Where two commits part is looked for along their history, which
    // has to be there too: here the first commit is not.
    let lacking = repo.dir.join("lacking");
    run(git(&repo.dir)
        .args(["clone", "--quiet"])
        .arg(repo.top())
        .arg(&lacking));
    run(git(&lacking).args(["config", "remote.origin.promisor", "true"]));
    let mark = fetches_by_running_a_program(&repo, &lacking);
    let first = repo.id("HEAD~2");
    let object = lacking.join(".git/objects").join(&first[..2]);
    fs::remove_file(object.join(&first[2..])).unwrap();
    let message = refused_in(
        &lacking,
        &mark,
        Some(old_git.as_ref()),
        &from_where_they_part,
    );
    assert!(message.contains("fetches nothing"), "{message}");
    // Nor is a commit that is not there fetched to be compared from.
    let from_the_first = ["--base", "HEAD~2", "--head", "HEAD"];
    let message = refused_in(&lacking, &mark, Some(old_git.as_ref()), &from_the_first);
    assert!(message.contains("fetches nothing"), "{message}");
}

#[test]
fn a_shallow_clone_a_linked_working_tree_and_borrowed_objects_are_read_alike() {
    let (repo, output) = edited("object-stores");
    let origin = format!("file://{}", repo.top().display());
    let shallow = repo.dir.join("shallow");
    run(git(&repo.dir)
        .args(["clone", "--quiet", "--depth=2"])
        .arg(&origin)
        .arg(&shallow));
    let linked = repo.dir.join("linked");
    run(git(&repo.top())
        .args(["worktree", "add", "--quiet", "--detach"])
        .arg(&linked));
    // A clone that has no objects of its own: `objects/info/alternates`.
    let borrowing = repo.dir.join("borrowing");
    run(git(&repo.dir)
        .args(["clone", "--quiet", "--shared"])
        .arg(repo.top())
        .arg(&borrowing));

    assert_eq!(output_in(&shallow), output);
    assert_eq!(output_in(&linked), output);
    assert_eq!(output_in(&borrowing), output);
}

// What is around the repository does not change the output.

/// Every file under a directory with its bytes, `.git` included.
fn snapshot(directory: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut pending = vec![directory.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                files.push((entry.path(), fs::read(entry.path()).unwrap()));
            }
        }
    }
    files.sort();
    files
}

#[test]
fn nothing_is_written_into_the_repository_or_beside_it() {
    let (repo, _) = edited("writes-nothing");
    let before = snapshot(&repo.dir);

    repo.output();

    assert!(
        snapshot(&repo.dir) == before,
        "a file under {:?} changed",
        repo.dir
    );
}

/// A class whose one method calls `steps` in order, a line each: lines
/// that algorithms pair differently.
fn steps(steps: &str) -> String {
    let calls: String = steps
        .split(' ')
        .map(|step| format!("        {step}();\n"))
        .collect();
    format!("public class Steps {{\n    public void run() {{\n{calls}    }}\n}}\n")
}

/// Every setting here changes what `git diff` says of the two commits of
/// this repository, or which repository git reads. None may change a byte
/// of the output.
#[test]
fn neither_the_configuration_of_git_nor_its_environment_changes_the_output() {
    let repo = Repo::within("configuration", "project");
    // A file that moves with an edit and with other line ends, files that
    // move unchanged, lines that can be paired in several ways, a binary
    // file, a submodule and a file outside the project.
    repo.write("app/old/Wound.cls", ALPHA.replace('\n', "\r\n"));
    repo.write("app/old/Same.cls", "public class Same {\n}\n");
    repo.write("app/classes/Steps.cls", steps("a b c a b b a"));
    repo.write("app/logo.png", b"\x89PNG\0one");
    pin(
        &repo,
        "project/app/vendor",
        "1111111111111111111111111111111111111111",
    );
    repo.commit("base");
    repo.remove("app/old/Wound.cls");
    repo.write(
        "app/new/Wound.cls",
        ALPHA.replace("return 2;", "return 20;"),
    );
    repo.rename("app/old/Same.cls", "app/new/Same.cls");
    repo.write("app/classes/Steps.cls", steps("c b a b a c"));
    repo.write("app/logo.png", b"\x89PNG\0two");
    repo.write("app/classes/Caf\u{e9}.cls", "public class Cafe {\n}\n");
    fs::write(repo.top().join("outside.txt"), "Not in the project.\n").unwrap();
    pin(
        &repo,
        "project/app/vendor",
        "2222222222222222222222222222222222222222",
    );
    repo.commit("head");

    let plain = repo.output();
    check_golden(repo.name, &plain);
    assert_eq!(
        files(&rows(&plain)),
        [
            json!(["added", null, "../outside.txt"]),
            json!(["added", null, "app/classes/Caf\u{e9}.cls"]),
            json!(["modified", "app/classes/Steps.cls", "app/classes/Steps.cls"]),
            json!(["modified", "app/logo.png", "app/logo.png"]),
            json!(["moved", "app/old/Same.cls", "app/new/Same.cls"]),
            json!(["added", null, "app/new/Wound.cls"]),
            json!(["deleted", "app/old/Wound.cls", null]),
            json!(["modified", "app/vendor", "app/vendor"]),
        ]
    );

    // The configuration of the repository, of the user and of the system.
    let user_config = repo.dir.join("gitconfig");
    let user_attributes = repo.dir.join("attributes");
    fs::write(&user_config, "").unwrap();
    let configure = |key: &str, value: &str| {
        run(git(&repo.top()).args(["config", key, value]));
        run(git(&repo.top())
            .args(["config", "--file"])
            .arg(&user_config)
            .args([key, value]));
    };
    let with_user_config = || {
        let output = run(repo
            .sf_impact_delta()
            .env("GIT_CONFIG_GLOBAL", &user_config)
            .env("GIT_CONFIG_SYSTEM", &user_config)
            .args(["--base", "HEAD~1", "--head", "HEAD"]));
        String::from_utf8(output.stdout).unwrap()
    };
    configure("core.attributesFile", user_attributes.to_str().unwrap());
    for (key, value) in [
        ("diff.renames", "false"),
        ("diff.renames", "copies"),
        ("diff.renameLimit", "1"),
        ("diff.algorithm", "histogram"),
        ("diff.indentHeuristic", "false"),
        ("diff.context", "9"),
        ("diff.interHunkContext", "9"),
        ("diff.noprefix", "true"),
        ("diff.mnemonicPrefix", "true"),
        ("diff.relative", "true"),
        ("diff.ignoreSubmodules", "all"),
        ("diff.submodule", "diff"),
        ("diff.suppressBlankEmpty", "true"),
        ("diff.wordRegex", "."),
        ("diff.dirstat", "files,0"),
        ("diff.external", "false"),
        ("diff.orderFile", "/dev/null"),
        ("diff.outputIndicatorNew", "X"),
        ("diff.outputIndicatorOld", "Y"),
        ("diff.outputIndicatorContext", "Z"),
        ("diff.apex.binary", "true"),
        ("diff.apex.textconv", "echo"),
        ("diff.apex.command", "false"),
        ("diff.apex.algorithm", "histogram"),
        ("diff.apex.xfuncname", "^public"),
        ("core.quotePath", "true"),
        ("core.autocrlf", "true"),
        ("core.safecrlf", "true"),
        ("core.eol", "crlf"),
        ("core.bigFileThreshold", "1"),
        ("core.precomposeUnicode", "true"),
        ("color.ui", "always"),
        ("color.diff", "always"),
    ] {
        configure(key, value);
        assert!(with_user_config() == plain, "with {key} = {value}");
    }

    // Attributes: of the repository, of the user and of the working tree.
    let attributes = [
        repo.top().join(".git/info/attributes"),
        user_attributes,
        repo.top().join(".gitattributes"),
        repo.root().join("app/.gitattributes"),
    ];
    for content in [
        "*.cls diff=apex\n",
        "*.cls -diff\n",
        "* binary\n",
        "*.cls text eol=crlf\n",
    ] {
        for file in &attributes {
            fs::write(file, content).unwrap();
            assert!(with_user_config() == plain, "with {content:?} in {file:?}");
        }
    }

    // The environment: where git is told to find a repository, its
    // objects and its configuration.
    let elsewhere = Repo::new("configuration-elsewhere");
    let dot_git = repo.top().join(".git");
    let environments: [&[(&str, PathBuf)]; 12] = [
        &[("GIT_COMMON_DIR", dot_git.clone())],
        &[("GIT_DIR", dot_git.clone()), ("GIT_WORK_TREE", repo.top())],
        // Another repository altogether, which has neither commit.
        &[("GIT_DIR", elsewhere.top().join(".git"))],
        &[("GIT_OBJECT_DIRECTORY", dot_git.join("objects"))],
        &[("GIT_OBJECT_DIRECTORY", PathBuf::from(".git/objects"))],
        &[("GIT_OBJECT_DIRECTORY", elsewhere.top().join(".git/objects"))],
        &[(
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            elsewhere.top().join(".git/objects"),
        )],
        &[(
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            PathBuf::from("no/such/objects"),
        )],
        &[
            ("GIT_INDEX_FILE", repo.dir.join("no-index")),
            ("GIT_NAMESPACE", PathBuf::from("elsewhere")),
        ],
        &[
            ("GIT_ATTR_SOURCE", PathBuf::from("HEAD~1")),
            ("GIT_EXTERNAL_DIFF", PathBuf::from("false")),
        ],
        &[
            ("GIT_CONFIG_COUNT", PathBuf::from("1")),
            ("GIT_CONFIG_KEY_0", PathBuf::from("diff.renames")),
            ("GIT_CONFIG_VALUE_0", PathBuf::from("false")),
        ],
        &[(
            "GIT_CONFIG_PARAMETERS",
            PathBuf::from("'core.bigFileThreshold=1'"),
        )],
    ];
    for environment in environments {
        let mut delta = repo.sf_impact_delta();
        for (name, value) in environment {
            delta.env(name, value);
        }
        let output = run(delta.args(["--base", "HEAD~1", "--head", "HEAD"]));
        assert!(
            String::from_utf8(output.stdout).unwrap() == plain,
            "with {environment:?}"
        );
    }

    // An object that the repository is told to read as another: the class
    // at head as the class it was at base.
    let (was, is) = (
        repo.id("HEAD~1:project/app/classes/Steps.cls"),
        repo.id("HEAD:project/app/classes/Steps.cls"),
    );
    run(git(&repo.top()).args(["replace", &is, &was]));
    assert!(with_user_config() == plain, "with a replacement object");

    // A graft: the head told to come from another commit than it does,
    // which would make that commit `HEAD~1`.
    let graft = format!("{} {}\n", repo.id("HEAD"), repo.id("HEAD~2"));
    fs::write(repo.top().join(".git/info/grafts"), graft).unwrap();
    assert!(with_user_config() == plain, "with a graft");
}

#[test]
fn the_same_two_commits_give_the_same_bytes_in_a_fixed_order() {
    let (repo, output) = edited("order");

    assert_eq!(repo.output(), output);
    // What the file is of, the commits, the files by path, then the
    // blobs by id.
    let rows = rows(&output);
    let kinds: Vec<&str> = rows
        .iter()
        .map(|row| row["row"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        [
            "provenance",
            "commit",
            "commit",
            "file",
            "file",
            "blob",
            "blob",
            "blob",
            "blob"
        ]
    );
    let ids: Vec<&str> = rows[5..]
        .iter()
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert!(ids.is_sorted(), "{ids:?}");
}

// The command line, and what goes wrong.

#[test]
fn the_output_can_go_to_a_file() {
    let (repo, output) = edited("out-file");
    let out = repo.dir.join("delta.jsonl");

    let printed = run(repo
        .sf_impact_delta()
        .args(["--base", "HEAD~1", "--head", "HEAD", "--out"])
        .arg(&out));

    assert!(printed.stdout.is_empty());
    assert_eq!(fs::read_to_string(out).unwrap(), output);
}

/// The exit code and what was said, where the run is to fail.
fn failure(command: &mut Command) -> (Option<i32>, String) {
    let output = command.output().unwrap();
    assert!(
        output.stdout.is_empty(),
        "nothing is written when a run fails"
    );
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn a_revision_that_does_not_exist_is_an_error_and_writes_nothing() {
    let repo = Repo::new("bad-revision");

    let (code, message) =
        failure(
            repo.sf_impact_delta()
                .args(["--base", "no-such-branch", "--head", "HEAD"]),
        );

    assert_eq!(code, Some(1));
    assert!(message.contains("no-such-branch"), "{message}");
}

#[test]
fn a_revision_that_would_be_read_as_an_option_is_refused() {
    let repo = Repo::new("option-revision");

    let (code, message) =
        failure(
            repo.sf_impact_delta()
                .args(["--base", "HEAD", "--head", "--output=/tmp/x"]),
        );

    assert_eq!(code, Some(1));
    assert!(
        message.contains("does not begin with `-`: --output=/tmp/x"),
        "{message}"
    );
}

#[test]
fn a_directory_that_is_in_no_repository_is_an_error() {
    let repo = Repo::new("no-repository");
    // A directory whose `.git` leads nowhere, whatever is above it.
    let outside = repo.dir.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join(".git"), "gitdir: /no/such/repository\n").unwrap();

    let (code, message) = failure(
        Command::new(env!("CARGO_BIN_EXE_sf-impact-delta"))
            .args(["--base", "HEAD", "--head", "HEAD", "--root"])
            .arg(&outside),
    );

    assert_eq!(code, Some(1));
    assert!(
        message.contains("git rev-parse --show-prefix failed"),
        "{message}"
    );
    assert!(message.contains("repository"), "{message}");
}

#[test]
fn a_path_that_is_not_utf_8_is_refused_and_not_taken_for_another() {
    use std::os::unix::ffi::OsStrExt;
    let repo = Repo::new("path-not-utf-8");
    // Two classes whose names differ in one byte that no text has. They
    // are put in the tree as git has them, whatever the file system takes.
    let names: [&[u8]; 2] = [b"project/\xfe.cls", b"project/\xff.cls"];
    for (name, class) in names.iter().zip(["Alpha", "Beta"]) {
        repo.write("class", format!("public class {class} {{\n}}\n"));
        let blob = said(git(&repo.top()).args(["hash-object", "-w", "class"]));
        let mut entry = format!("100644,{blob},").into_bytes();
        entry.extend_from_slice(name);
        run(git(&repo.top())
            .args(["update-index", "--add", "--cacheinfo"])
            .arg(std::ffi::OsStr::from_bytes(&entry)));
    }
    repo.remove("class");
    run(git(&repo.top()).args(["commit", "--quiet", "--message", "two such paths"]));
    for name in names {
        run(git(&repo.top())
            .args(["update-index", "--force-remove"])
            .arg(std::ffi::OsStr::from_bytes(name)));
    }
    run(git(&repo.top()).args(["commit", "--quiet", "--message", "head"]));

    let (code, message) = failure(
        repo.sf_impact_delta()
            .args(["--base", "HEAD~1", "--head", "HEAD"]),
    );

    assert_eq!(code, Some(1));
    assert!(message.contains("not UTF-8"), "{message}");
    assert!(message.contains("project/\u{fffd}.cls"), "{message}");
}

#[test]
fn a_blob_that_the_repository_lacks_is_an_error_not_an_empty_file() {
    let repo = Repo::new("missing-object");
    repo.replace(CLASS, "return 2;", "return 20;");
    repo.commit("head");
    let blob = repo.id(&format!("HEAD:{CLASS}"));
    let object = repo
        .top()
        .join(".git/objects")
        .join(&blob[..2])
        .join(&blob[2..]);
    fs::remove_file(object).unwrap();

    let (code, message) = failure(
        repo.sf_impact_delta()
            .args(["--base", "HEAD~1", "--head", "HEAD"]),
    );

    assert_eq!(code, Some(1));
    assert!(message.contains(&blob), "{message}");
}

#[test]
fn a_repository_git_will_not_read_for_its_owner_says_why_an_exception_does_not_apply() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new("dubious-ownership");
    // A `git` that refuses as git does in a repository of another owner.
    let bin = repo.dir.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(
        bin.join("git"),
        "#!/bin/sh\necho \"fatal: detected dubious ownership in repository at '/x'\" >&2\nexit 128\n",
    )
    .unwrap();
    fs::set_permissions(bin.join("git"), fs::Permissions::from_mode(0o755)).unwrap();

    let (code, message) = failure(
        repo.sf_impact_delta()
            .env("PATH", &bin)
            .args(["--base", "HEAD", "--head", "HEAD"]),
    );

    assert_eq!(code, Some(1));
    assert!(message.contains("dubious ownership"), "{message}");
    assert!(message.contains("safe.directory"), "{message}");
}

/// A directory to put first in `PATH`, with a `git` that takes every
/// repository to be another user's, as git does in a container whose
/// workspace was made by someone else. It notes what it was asked.
fn git_in_a_repository_of_another_owner(repo: &Repo) -> (std::ffi::OsString, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let path = env::var_os("PATH").unwrap();
    let real = env::split_paths(&path)
        .map(|directory| directory.join("git"))
        .find(|git| git.is_file())
        .expect("git is in PATH");
    let (bin, asked) = (repo.dir.join("other-owner"), repo.dir.join("asked"));
    fs::create_dir_all(&bin).unwrap();
    let script = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexport GIT_TEST_ASSUME_DIFFERENT_OWNER=1\n\
         exec '{}' \"$@\"\n",
        asked.display(),
        real.display()
    );
    fs::write(bin.join("git"), script).unwrap();
    fs::set_permissions(bin.join("git"), fs::Permissions::from_mode(0o755)).unwrap();
    let path = env::join_paths([bin].into_iter().chain(env::split_paths(&path))).unwrap();
    (path, asked)
}

#[test]
fn a_repository_of_another_owner_is_read_only_when_told_to_trust_it() {
    let repo = Repo::within("trust-repo", "project");
    repo.replace(CLASS, "return 2;", "return 20;");
    repo.commit("head");
    let output = repo.output();
    let (other_owner, asked) = git_in_a_repository_of_another_owner(&repo);
    let between = ["--base", "HEAD~1", "--head", "HEAD"];
    let before = snapshot(&repo.dir.join("repo"));

    // Not without being told to: git refuses, and the error says how.
    let (code, message) = failure(
        repo.sf_impact_delta()
            .env("PATH", &other_owner)
            .args(between),
    );
    assert_eq!(code, Some(1));
    assert!(message.contains("dubious ownership"), "{message}");
    assert!(message.contains("--trust-repo"), "{message}");

    // Told to, it is read as any other, from the project and from a
    // linked working tree of it. Its configuration is then read, and
    // names a program wherever git takes one: none is run.
    let linked = repo.dir.join("linked");
    run(git(&repo.top())
        .args(["worktree", "add", "--quiet", "--detach"])
        .arg(&linked));
    let mark = fetches_by_running_a_program(&repo, &repo.top());
    for setting in [
        "core.fsmonitor",
        "core.gitProxy",
        "core.alternateRefsCommand",
        "diff.external",
        "credential.helper",
    ] {
        run(git(&repo.top())
            .args(["config", setting])
            .arg(repo.dir.join("fetch-program")));
    }
    let before_linked = snapshot(&repo.dir.join("repo"));
    for root in [repo.root(), linked.join("project")] {
        let trusted = run(Command::new(env!("CARGO_BIN_EXE_sf-impact-delta"))
            .env("PATH", &other_owner)
            .arg("--trust-repo")
            .args(between)
            .arg("--root")
            .arg(&root));
        assert_eq!(String::from_utf8(trusted.stdout).unwrap(), output);
    }
    assert!(before_linked == snapshot(&repo.dir.join("repo")));
    assert!(before.len() <= before_linked.len());
    assert!(
        !mark.exists(),
        "git ran a program that the repository names"
    );

    // What git was told to trust is that one repository, by the path of
    // its top, and never every repository.
    let asked = fs::read_to_string(asked).unwrap();
    let trusted: std::collections::BTreeSet<&str> = asked
        .lines()
        .flat_map(|line| line.split(' '))
        .filter_map(|word| word.strip_prefix("safe.directory="))
        .collect();
    let tops = [repo.top(), linked].map(|top| fs::canonicalize(top).unwrap());
    let reads: Vec<&str> = asked
        .lines()
        .filter(|line| line.contains("cat-file"))
        .collect();
    assert!(!trusted.contains("*"), "{trusted:?}");
    assert_eq!(reads.len(), 2, "{asked}");
    for (read, top) in reads.iter().zip(&tops) {
        let exception = format!("safe.directory={}", top.display());
        assert!(read.contains(&exception), "{read}");
    }
    for directory in &trusted {
        let is_above = tops.iter().any(|top| top.starts_with(directory));
        let is_below = tops.iter().any(|top| Path::new(directory).starts_with(top));
        assert!(is_above || is_below, "{directory} is on the way to no top");
    }
}

#[test]
fn a_trusted_repository_that_lacks_objects_is_refused_and_runs_no_program() {
    let (repo, _) = edited("trust-repo-partial");
    let clone = partial_clone(&repo, "clone", "--filter=blob:none", &["--no-checkout"]);
    let mark = fetches_by_running_a_program(&repo, &clone);
    let (other_owner, _) = git_in_a_repository_of_another_owner(&repo);

    let message = refused_in(
        &clone,
        &mark,
        Some(other_owner.as_ref()),
        &["--trust-repo", "--base", "HEAD~1", "--head", "HEAD"],
    );

    assert!(message.contains("fetches nothing"), "{message}");
}

#[test]
fn a_command_line_that_is_not_understood_is_refused_with_the_usage() {
    let repo = Repo::new("usage");

    let (code, message) = failure(repo.sf_impact_delta().args(["--head", "HEAD"]));

    assert_eq!(code, Some(2));
    assert!(
        message.contains("--base or --merge-base is required"),
        "{message}"
    );
    assert!(message.contains("usage: sf-impact-delta"), "{message}");
}

// What the file says of itself.

#[test]
fn the_delta_starts_with_what_wrote_it_of_which_commits_with_which_scope_and_config() {
    let repo = Repo::new("provenance");
    repo.write(
        "sfdx-project.json",
        r#"{"packageDirectories": [
            {"path": "plugins\\billing"},
            {"path": "./app/", "unpackagedMetadata": {"path": "unpackaged"}},
            {"path": "../elsewhere"}
        ]}"#,
    );
    repo.commit("head");

    let output = repo.output();

    check_schema(repo.name, &output);
    let rows = rows(&output);
    assert_eq!(
        rows[0],
        json!({
            "row": "provenance",
            "tool": "sf-impact-delta",
            "version": env!("CARGO_PKG_VERSION"),
            "schema": 1,
            "base": repo.id("HEAD~1"),
            "head": repo.id("HEAD"),
            "head_tree": repo.id("HEAD^{tree}"),
            // Where the project is in its repository: here, at the top.
            "root": ".",
            // Of the head, normalised, sorted, and without the path that
            // leaves the root.
            "scope": {"packageDirectories": ["app", "plugins/billing"], "unpackagedMetadata": ["unpackaged"]},
            "config": rows[0]["config"],
        })
    );
    let config = rows[0]["config"].as_str().expect("a hash");
    assert_eq!(config.len(), 16, "{config}");
    // One such row, and the rows of the commits after it as they were.
    let kinds: Vec<&str> = rows
        .iter()
        .map(|row| row["row"].as_str().unwrap())
        .collect();
    assert_eq!(kinds[..3], ["provenance", "commit", "commit"]);
    assert_eq!(
        kinds.iter().filter(|kind| **kind == "provenance").count(),
        1
    );
}

#[test]
fn the_delta_of_a_project_below_the_top_of_its_repository_says_where_it_is() {
    let repo = Repo::within("provenance-root", "packages/billing");
    repo.replace(CLASS, "return 2;", "return 20;");
    repo.commit("head");

    let first = rows(&repo.output()).remove(0);

    assert_eq!(first["root"], "packages/billing");
}

#[test]
fn the_same_two_commits_give_the_same_bytes_in_another_directory() {
    let edited = |name: &'static str| {
        let repo = Repo::new(name);
        repo.replace(CLASS, "return 2;", "return 20;");
        repo.commit("head");
        repo
    };
    let (here, there) = (edited("here"), edited("there"));

    let first = here.output();

    assert!(first.starts_with(r#"{"row":"provenance","#), "{first}");
    assert_eq!(here.output(), first);
    assert_eq!(there.output(), first);
}

#[test]
fn a_head_whose_project_file_or_config_cannot_be_read_says_so_in_its_provenance() {
    let provenance = |repo: &Repo| {
        repo.commit("head");
        let output = repo.output();
        check_schema(repo.name, &output);
        rows(&output).remove(0)
    };
    let no_file = Repo::new("default-config");
    no_file.replace(CLASS, "return 2;", "return 20;");
    let default = provenance(&no_file)["config"].clone();
    assert_eq!(default, "4e7844462778448c");

    // The config that takes effect, not the file as it is written.
    let written = Repo::new("written-config");
    written.write("sf-impact.json", "{\"textualMinLength\": 6}\n");
    assert_eq!(provenance(&written)["config"], default);
    let other = Repo::new("other-config");
    other.write("sf-impact.json", "{\"namespaces\": [\"npsp\"]}\n");
    assert_ne!(provenance(&other)["config"], default);

    // Neither is refused here: what cannot be read is for the stage that
    // reads it to say. The row has nothing to hold refs to.
    let no_config = Repo::new("unreadable-config");
    no_config.write("sf-impact.json", "not JSON\n");
    assert_eq!(provenance(&no_config)["config"], Value::Null);
    let no_project = Repo::new("no-project-file");
    no_project.remove("sfdx-project.json");
    assert_eq!(provenance(&no_project)["scope"], Value::Null);
    let not_json = Repo::new("unreadable-project-file");
    not_json.write("sfdx-project.json", "not JSON\n");
    assert_eq!(provenance(&not_json)["scope"], Value::Null);
}

#[test]
fn a_path_that_is_not_utf_8_is_shown_escaped_and_starts_no_line() {
    use std::os::unix::ffi::OsStrExt;
    let repo = Repo::new("path-not-utf-8-escaped");
    let name: &[u8] = b"project/A\n::error::injected\x1b[2K\xff.cls";
    repo.write("class", "public class Alpha {\n}\n");
    let blob = said(git(&repo.top()).args(["hash-object", "-w", "class"]));
    let mut entry = format!("100644,{blob},").into_bytes();
    entry.extend_from_slice(name);
    run(git(&repo.top())
        .args(["update-index", "--add", "--cacheinfo"])
        .arg(std::ffi::OsStr::from_bytes(&entry)));
    repo.remove("class");
    run(git(&repo.top()).args(["commit", "--quiet", "--message", "head"]));

    let (code, message) = failure(
        repo.sf_impact_delta()
            .args(["--base", "HEAD~1", "--head", "HEAD"]),
    );

    assert_eq!(code, Some(1));
    assert!(
        message.contains("project/A\\n::error::injected\\u{1b}[2K\u{fffd}.cls"),
        "{message}"
    );
    assert!(!message.contains('\u{1b}'), "{message}");
    let starts = |line: &str| line.trim_start().starts_with("::");
    assert!(!message.lines().any(starts), "{message}");
}

#[test]
fn an_out_that_is_a_link_is_refused_and_what_it_names_is_left_as_it_was() {
    let repo = Repo::new("out-link");
    repo.write(CLASS, "public class Alpha {\n}\n");
    repo.commit("head");
    let victim = repo.dir.join("victim");
    fs::write(&victim, "a file outside\n").unwrap();
    let link = repo.dir.join("delta.jsonl");
    std::os::unix::fs::symlink(&victim, &link).unwrap();
    let delta = |out: &Path| {
        repo.sf_impact_delta()
            .args(["--base", "HEAD~1", "--head", "HEAD", "--out"])
            .arg(out)
            .output()
            .unwrap()
    };

    let output = delta(&link);

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let message = String::from_utf8(output.stderr).unwrap();
    assert!(message.contains("is not a regular file"), "{message}");
    assert_eq!(fs::read_to_string(&victim).unwrap(), "a file outside\n");
    // The control: a path that is no link is written.
    let out = repo.dir.join("written.jsonl");
    assert!(delta(&out).status.success());
    assert_eq!(fs::read_to_string(&out).unwrap(), repo.output());
}
