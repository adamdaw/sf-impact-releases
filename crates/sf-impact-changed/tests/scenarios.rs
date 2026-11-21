//! Runs the stages on real git repositories. Each test copies the project
//! in `fixtures/changed/base` into a temporary repository, commits it as the
//! base, makes one change and commits that as the head. It then extracts
//! the head with the `sf-impact-extract` library, takes the delta of the two
//! commits with the `sf-impact-delta` library, and runs the `sf-impact-changed` binary
//! on the two files.
//!
//! What a scenario is about is asserted in the test. The whole output is
//! also compared with `fixtures/changed/<scenario>.jsonl` and checked
//! against `schema/changed.schema.json`. To accept a deliberate change:
//! `UPDATE_GOLDEN=1 cargo test -p sf-impact-changed --test scenarios`

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
/// same content is the same commit on every machine: the first row of
/// each output names the commits it is of.
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

/// A git repository in a temporary directory, removed on drop. The project
/// is in `project`, which is the repository itself unless a test says
/// otherwise.
struct Repo {
    scenario: &'static str,
    dir: PathBuf,
    /// The project's directory within the repository; empty for the root.
    project: &'static str,
}

impl Repo {
    /// The base project, committed.
    fn new(scenario: &'static str) -> Repo {
        Repo::within(scenario, "")
    }

    /// The base project in the directory `project` of the repository.
    fn within(scenario: &'static str, project: &'static str) -> Repo {
        let name = format!("sf-impact-changed-{}-{scenario}", std::process::id());
        let repo = Repo {
            scenario,
            dir: env::temp_dir().join(name),
            project,
        };
        let _ = fs::remove_dir_all(&repo.dir);
        fs::create_dir_all(repo.dir.join("repo")).unwrap();
        run(git(&repo.root()).args([
            "init",
            "--quiet",
            "--initial-branch=main",
            "--object-format=sha1",
        ]));
        copy(
            &repository().join("fixtures/changed/base"),
            &repo.root().join(project),
        );
        repo.commit("base");
        repo
    }

    fn root(&self) -> PathBuf {
        self.dir.join("repo")
    }

    /// A path in the project.
    fn path(&self, file: &str) -> PathBuf {
        self.root().join(self.project).join(file)
    }

    fn write(&self, file: &str, content: impl AsRef<[u8]>) {
        let path = self.path(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// Replaces the one occurrence of `from` in `file`.
    fn replace(&self, file: &str, from: &str, to: &str) {
        let content = fs::read_to_string(self.path(file)).unwrap();
        assert_eq!(content.matches(from).count(), 1, "{from:?} in {file}");
        self.write(file, content.replace(from, to));
    }

    fn remove(&self, file: &str) {
        fs::remove_file(self.path(file)).unwrap();
    }

    fn rename(&self, from: &str, to: &str) {
        let to = self.path(to);
        fs::create_dir_all(to.parent().unwrap()).unwrap();
        fs::rename(self.path(from), to).unwrap();
    }

    fn commit(&self, message: &str) {
        run(git(&self.root()).args(["add", "--all"]));
        run(git(&self.root()).args(["commit", "--quiet", "--message", message]));
    }

    /// Commits the change as the head and runs `sf-impact-changed` from the
    /// commit before it to it. The output is checked against the
    /// scenario's golden file and the schema.
    fn changed(&self) -> Vec<Value> {
        self.commit("head");
        let output = self.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
        check_golden(self.scenario, &output);
        rows(&output)
    }

    /// Extracts the project as it is on disk, takes the delta between two
    /// commits, `--base <rev>` or `--merge-base <rev>` and `--head <rev>`,
    /// and runs `sf-impact-changed` on the two. What they are is left in
    /// `refs.pack` and `delta.jsonl` beside the repository.
    fn run_changed(&self, revisions: &[&str], extra: &[&str]) -> String {
        let refs = self.dir.join("refs.pack");
        fs::write(&refs, pack_of(&self.path(""))).unwrap();
        let base = match revisions {
            ["--base", base, "--head", _] => sf_impact_delta::Base::Commit(base.to_string()),
            ["--merge-base", with, "--head", _] => {
                sf_impact_delta::Base::MergeBaseWith(with.to_string())
            }
            other => panic!("not two revisions: {other:?}"),
        };
        let lines =
            sf_impact_delta::delta(&self.path(""), false, &base, revisions[3]).expect("the delta");
        let delta = lines.join("\n") + "\n";
        check_schema("delta", self.scenario, &delta);
        fs::write(self.dir.join("delta.jsonl"), delta).unwrap();
        let mut changed = self.sf_impact_changed();
        changed.args(extra);
        String::from_utf8(run(&mut changed).stdout).expect("the output is UTF-8")
    }

    /// The binary, on the two files that `run_changed` left.
    fn sf_impact_changed(&self) -> Command {
        let mut changed = Command::new(env!("CARGO_BIN_EXE_sf-impact-changed"));
        changed
            .arg("--delta")
            .arg(self.dir.join("delta.jsonl"))
            .arg("--refs")
            .arg(self.dir.join("refs.pack"));
        changed
    }
}

/// The refs pack that sf-impact-extract writes for the project at `root`.
fn pack_of(root: &Path) -> Vec<u8> {
    let extracted = sf_impact_extract::extract_kept(root, 1, false);
    extracted.expect("the head extracts").0
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The rows of an output that say what changed: all but the first, which
/// says what the file is of.
fn rows(output: &str) -> Vec<Value> {
    let mut rows = lines(output);
    let first = rows.remove(0);
    assert_eq!(first["row"], "provenance", "{first}");
    rows
}

fn lines(output: &str) -> Vec<Value> {
    let rows = output.lines().map(serde_json::from_str);
    rows.collect::<Result<_, _>>().expect("every line is JSON")
}

/// Every row of an output is valid against the schema of its file.
fn check_schema(file: &str, scenario: &str, output: &str) {
    let schema = repository().join(format!("schema/{file}.schema.json"));
    let schema = Schema::parse(&fs::read_to_string(schema).expect("the schema exists"));
    for (number, row) in lines(output).iter().enumerate() {
        let errors = schema.errors(row);
        assert!(
            errors.is_empty(),
            "{scenario}, {file} line {}: {errors:?}",
            number + 1
        );
    }
}

/// The output is the scenario's golden file, and every row of it is valid.
fn check_golden(scenario: &str, output: &str) {
    let golden = repository().join(format!("fixtures/changed/{scenario}.jsonl"));
    if env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&golden, output).expect("the golden file is writable");
    }
    let expected = fs::read_to_string(&golden).unwrap_or_else(|e| panic!("{golden:?}: {e}"));
    assert_eq!(
        output, expected,
        "the output differs from fixtures/changed/{scenario}.jsonl. \
         If the change is intended, rerun with UPDATE_GOLDEN=1 and review the diff."
    );

    check_schema("changed", scenario, &expected);
}

/// The golden files are the examples of the schema, so between them they
/// have to show every kind of row and every value that says why.
#[test]
fn the_golden_files_show_every_alternative_of_the_schema() {
    let directory = repository().join("fixtures/changed");
    let mut seen = std::collections::BTreeSet::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path
            .extension()
            .is_none_or(|extension| extension != "jsonl")
        {
            continue;
        }
        for row in rows(&fs::read_to_string(&path).unwrap()) {
            for field in ["row", "status", "change", "reason", "side", "kind"] {
                if let Some(value) = row[field].as_str() {
                    seen.insert(format!("{field}={value}"));
                }
            }
            for gap in row["gaps"].as_array().into_iter().flatten() {
                seen.insert(format!("gap={}", gap.as_str().unwrap()));
            }
        }
    }

    let seen: Vec<&str> = seen.iter().map(String::as_str).collect();
    assert_eq!(
        seen,
        [
            "change=added",
            "change=modified",
            "change=removed",
            "gap=no_declaration",
            "gap=parse_error",
            "gap=scope_changed",
            "gap=unmodelled_file",
            "gap=unsupported_element",
            "kind=cmdt",
            "kind=cmdt_field",
            "kind=cmdt_type",
            "kind=grant",
            "reason=hunk",
            "reason=removed",
            "reason=whole_file",
            "row=file",
            "row=symbol",
            "row=touched",
            "side=base",
            "side=head",
            "status=added",
            "status=deleted",
            "status=mode_changed",
            "status=modified",
            "status=moved",
            "status=type_changed",
        ]
    );
}

/// The rows of one kind, each reduced to the fields named.
fn fields(rows: &[Value], kind: &str, names: &[&str]) -> Vec<Value> {
    let of_kind = rows.iter().filter(|row| row["row"] == kind);
    of_kind
        .map(|row| Value::Array(names.iter().map(|name| row[*name].clone()).collect()))
        .collect()
}

/// Symbol, change and reason of every symbol row.
fn symbols(rows: &[Value]) -> Vec<Value> {
    fields(rows, "symbol", &["symbol", "change", "reason"])
}

/// File, status and confidence of every file row.
fn files(rows: &[Value]) -> Vec<Value> {
    fields(rows, "file", &["file", "status", "confidence"])
}

/// Side, kind and name of every touched row.
fn touched(rows: &[Value]) -> Vec<Value> {
    fields(rows, "touched", &["side", "kind", "referenced"])
}

fn the_one<'a>(rows: &'a [Value], kind: &str) -> &'a Value {
    let mut of_kind = rows.iter().filter(|row| row["row"] == kind);
    let row = of_kind.next().unwrap_or_else(|| panic!("a {kind} row"));
    assert!(of_kind.next().is_none(), "one {kind} row");
    row
}

const INVOICE: &str = "force-app/classes/Invoice.cls";
const PERMISSION_SET: &str = "force-app/permissionsets/Billing_Admin.permissionset-meta.xml";
const RECORD: &str = "force-app/customMetadata/Plugin.Billing.md-meta.xml";

// Apex: a hunk marks the innermost declaration around it.

#[test]
fn an_edit_inside_a_method_marks_the_method() {
    let repo = Repo::new("method-edit");
    repo.replace(
        INVOICE,
        "return amount + tax();",
        "return amount + tax() + 1;",
    );

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [json!(["Invoice.total()", "modified", "hunk"])]
    );
    let total = the_one(&rows, "symbol");
    assert_eq!((&total["start"], &total["end"]), (&json!(10), &json!(12)));
    assert_eq!(
        total["hunks"],
        json!([{"base_start": 11, "base_lines": 1, "head_start": 11, "head_lines": 1}])
    );
    assert_eq!(files(&rows), [json!([INVOICE, "modified", "full"])]);
}

#[test]
fn an_edit_outside_every_member_marks_the_class() {
    let repo = Repo::new("class-level-edit");
    repo.replace(
        INVOICE,
        "public with sharing class",
        "public without sharing class",
    );

    let rows = repo.changed();

    assert_eq!(symbols(&rows), [json!(["Invoice", "modified", "hunk"])]);
}

#[test]
fn an_edit_of_a_field_marks_the_field() {
    let repo = Repo::new("field-edit");
    repo.replace(INVOICE, "RATE = 0.2;", "RATE = 0.25;");

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [json!(["Invoice.RATE", "modified", "hunk"])]
    );
}

#[test]
fn an_edit_in_an_inner_class_marks_the_inner_class_or_its_member() {
    let repo = Repo::new("inner-class");
    repo.replace(
        INVOICE,
        "public class Line {",
        "public virtual class Line {",
    );
    repo.replace(INVOICE, "return quantity * 1.0;", "return quantity * 2.0;");

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [
            json!(["Invoice.Line", "modified", "hunk"]),
            json!(["Invoice.Line.amount()", "modified", "hunk"]),
        ]
    );
}

#[test]
fn a_line_above_the_class_marks_the_class_its_file_is_for() {
    let repo = Repo::new("header-comment");
    repo.replace(
        INVOICE,
        "public with sharing",
        "// Billing.\npublic with sharing",
    );

    let rows = repo.changed();

    assert_eq!(symbols(&rows), [json!(["Invoice", "modified", "hunk"])]);
}

#[test]
fn an_added_method_is_added() {
    let repo = Repo::new("added-method");
    let tax = "    public Decimal tax() {";
    let discount = "    public Decimal discount() {\n        return 0;\n    }\n\n";
    repo.replace(INVOICE, tax, &format!("{discount}{tax}"));

    let rows = repo.changed();

    // The blank line that sets the method apart is a line of the class.
    assert_eq!(
        symbols(&rows),
        [
            json!(["Invoice", "modified", "hunk"]),
            json!(["Invoice.discount()", "added", "hunk"]),
        ]
    );
}

#[test]
fn a_removed_method_is_removed_and_marks_the_class_it_was_in() {
    let repo = Repo::new("removed-method");
    let round =
        "    public Decimal round(Decimal value) {\n        return value.setScale(2);\n    }\n\n";
    repo.replace(INVOICE, round, "");

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [
            json!(["Invoice", "modified", "hunk"]),
            json!(["Invoice.round(decimal)", "removed", "removed"]),
        ]
    );
    // The evidence of a removal is where it was declared at base.
    let removed = &rows[2];
    assert_eq!(
        (&removed["start"], &removed["end"]),
        (&json!(18), &json!(20))
    );
}

#[test]
fn an_annotation_taken_from_above_a_method_marks_the_method_it_was_on() {
    let repo = Repo::new("removed-annotation");
    repo.replace(
        "force-app/classes/InvoiceHandler.cls",
        "    @TestVisible\n",
        "",
    );

    let rows = repo.changed();

    // The line was the method's first at base. At head the gap it leaves
    // is above the method, in the class.
    assert_eq!(
        symbols(&rows),
        [
            json!(["InvoiceHandler", "modified", "hunk"]),
            json!(["InvoiceHandler.handle()", "modified", "hunk"]),
        ]
    );
}

#[test]
fn a_changed_signature_is_one_overload_removed_and_another_added() {
    let repo = Repo::new("overload-signature");
    repo.replace(
        INVOICE,
        "round(Decimal value) {",
        "round(Decimal value, Integer places) {",
    );

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [
            json!(["Invoice.round(decimal)", "removed", "removed"]),
            json!(["Invoice.round(decimal,integer)", "added", "hunk"]),
        ]
    );
}

#[test]
fn a_name_that_changes_only_in_case_is_the_same_symbol() {
    let repo = Repo::new("case-only");
    repo.replace(INVOICE, "public Decimal tax() {", "public Decimal Tax() {");

    let rows = repo.changed();

    // Apex compares names without regard to case: nothing was removed.
    assert_eq!(
        symbols(&rows),
        [json!(["Invoice.Tax()", "modified", "hunk"])]
    );
}

#[test]
fn a_change_of_white_space_alone_is_a_change() {
    let repo = Repo::new("whitespace-only");
    repo.replace(
        INVOICE,
        "        return amount * RATE;",
        "            return  amount * RATE;",
    );

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [json!(["Invoice.tax()", "modified", "hunk"])]
    );
}

// Whole files.

#[test]
fn an_added_file_adds_everything_it_declares() {
    let repo = Repo::new("added-file");
    repo.write(
        "force-app/classes/Refund.cls",
        "public class Refund {\n    public void issue() {\n    }\n}\n",
    );

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [
            json!(["Refund", "added", "whole_file"]),
            json!(["Refund.issue()", "added", "whole_file"]),
        ]
    );
    assert_eq!(
        files(&rows),
        [json!(["force-app/classes/Refund.cls", "added", "full"])]
    );
}

#[test]
fn a_deleted_file_removes_everything_it_declared() {
    let repo = Repo::new("deleted-file");
    repo.remove("force-app/classes/PaymentHandler.cls");
    repo.remove("force-app/classes/PaymentHandler.cls-meta.xml");

    let rows = repo.changed();

    // The class is gone with its file, and with the companion that was its.
    let (class, companion) = (
        "force-app/classes/PaymentHandler.cls",
        "force-app/classes/PaymentHandler.cls-meta.xml",
    );
    assert_eq!(
        symbols_by_file(&rows),
        [
            json!(["PaymentHandler", "removed", "whole_file", class]),
            json!(["PaymentHandler.handle()", "removed", "whole_file", class]),
            json!(["PaymentHandler", "removed", "whole_file", companion]),
        ]
    );
    assert_eq!(
        files(&rows),
        [
            json!(["force-app/classes/PaymentHandler.cls", "deleted", "full"]),
            json!([
                "force-app/classes/PaymentHandler.cls-meta.xml",
                "deleted",
                "full"
            ]),
        ]
    );
}

#[test]
fn a_file_that_moved_without_a_change_changes_no_symbol() {
    let repo = Repo::new("moved-unchanged");
    repo.rename(INVOICE, "force-app/classes/billing/Invoice.cls");

    let rows = repo.changed();

    assert_eq!(symbols(&rows), [] as [Value; 0]);
    let file = the_one(&rows, "file");
    assert_eq!(file["file"], "force-app/classes/billing/Invoice.cls");
    assert_eq!(file["previous_file"], INVOICE);
    assert_eq!(
        (&file["status"], &file["confidence"]),
        (&json!("moved"), &json!("full"))
    );
}

#[test]
fn a_file_renamed_with_its_class_removes_the_old_names_and_adds_the_new() {
    let repo = Repo::new("moved-renamed-class");
    repo.rename(
        "force-app/classes/PaymentHandler.cls",
        "force-app/classes/RefundHandler.cls",
    );
    repo.replace(
        "force-app/classes/RefundHandler.cls",
        "class PaymentHandler",
        "class RefundHandler",
    );

    let rows = repo.changed();

    // Nothing says that the two files are one: neither their content,
    // which no stage compares for likeness, nor what they declare.
    assert_eq!(
        symbols(&rows),
        [
            json!(["PaymentHandler", "removed", "whole_file"]),
            json!(["PaymentHandler.handle()", "removed", "whole_file"]),
            json!(["RefundHandler", "added", "whole_file"]),
            json!(["RefundHandler.handle()", "added", "whole_file"]),
        ]
    );
    assert_eq!(
        files(&rows),
        [
            json!(["force-app/classes/PaymentHandler.cls", "deleted", "full"]),
            json!(["force-app/classes/RefundHandler.cls", "added", "full"]),
        ]
    );
}

#[test]
fn an_edit_in_a_file_that_moved_marks_what_it_edits_and_nothing_else() {
    let repo = Repo::new("moved-edited");
    repo.rename(INVOICE, "force-app/classes/billing/Invoice.cls");
    repo.replace(
        "force-app/classes/billing/Invoice.cls",
        "return amount * RATE;",
        "return 0;",
    );

    let rows = repo.changed();

    // The delta has a file deleted and a file added. They declare the
    // same class in the same package, so they are compared as one file.
    let moved_to = "force-app/classes/billing/Invoice.cls";
    assert_eq!(
        fields(
            &rows,
            "symbol",
            &["symbol", "change", "reason", "file", "previous_file"]
        ),
        [json!([
            "Invoice.tax()",
            "modified",
            "hunk",
            moved_to,
            INVOICE
        ])]
    );
    assert_eq!(
        files(&rows),
        [
            json!([INVOICE, "deleted", "full"]),
            json!([moved_to, "added", "full"]),
        ]
    );
}

#[test]
fn a_file_moved_to_another_package_is_removed_from_one_and_added_to_the_other() {
    let repo = Repo::new("moved-package");
    repo.rename(
        "force-app/classes/PaymentHandler.cls",
        "plugins/billing/classes/PaymentHandler.cls",
    );

    let rows = repo.changed();

    assert_eq!(
        fields(
            &rows,
            "symbol",
            &["symbol", "package", "change", "collision"]
        ),
        [
            json!(["PaymentHandler", "force-app", "removed", true]),
            json!(["PaymentHandler.handle()", "force-app", "removed", true]),
            json!(["PaymentHandler", "plugins/billing", "added", false]),
            json!(["PaymentHandler.handle()", "plugins/billing", "added", false]),
        ]
    );
}

#[test]
fn a_change_of_mode_alone_changes_no_symbol_and_leaves_no_gap() {
    use std::os::unix::fs::PermissionsExt;
    let repo = Repo::new("mode-only");
    let bundle_file = "force-app/lwc/invoiceList/invoiceList.js";
    let layout = "force-app/layouts/Invoice__c-Invoice Layout.layout-meta.xml";
    for file in [INVOICE, bundle_file, layout] {
        fs::set_permissions(repo.path(file), fs::Permissions::from_mode(0o755)).unwrap();
    }

    let rows = repo.changed();

    // Not the bundle, though any other change to its file marks it; and
    // not partial, though the layout is not modelled.
    assert_eq!(
        files(&rows),
        [
            json!([INVOICE, "mode_changed", "full"]),
            json!([layout, "mode_changed", "full"]),
            json!([bundle_file, "mode_changed", "full"]),
        ]
    );
    assert_eq!(rows.len(), 3);
}

#[test]
fn a_file_at_base_that_did_not_parse_cleanly_lowers_the_confidence() {
    let repo = Repo::new("parse-error-at-base");
    repo.replace(INVOICE, "return amount * RATE;", "return amount * (RATE;");
    repo.commit("a syntax error");
    repo.replace(INVOICE, "return amount * (RATE;", "return amount * RATE;");

    let rows = repo.changed();

    assert_eq!(the_one(&rows, "file")["gaps"], json!(["parse_error"]));
}

#[test]
fn a_latin1_change_marks_the_whole_file_and_keeps_the_gap() {
    let repo = Repo::new("latin1-whole-file-gap");
    let file = "force-app/classes/Latin.cls";
    repo.write(
        file,
        b"public class Latin { // caf\xe9\n    Integer a() { return 1; }\n}\n",
    );
    repo.commit("Latin-1 at base");
    repo.write(
        file,
        b"public class Latin { // caf\xe9\n    Integer a() { return 2; }\n}\n",
    );
    repo.commit("Latin-1 at head");
    let output = repo.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
    check_schema("changed", "latin1-whole-file-gap", &output);
    let rows = rows(&output);
    assert_eq!(
        symbols(&rows),
        [
            json!(["Latin", "modified", "whole_file"]),
            json!(["Latin.a()", "modified", "whole_file"]),
        ]
    );
    let file = the_one(&rows, "file");
    assert_eq!(file["binary"], true);
    assert_eq!(file["confidence"], "partial");
    assert_eq!(file["gaps"], json!(["parse_error"]));
}

#[test]
fn a_utf16_change_marks_only_the_changed_members() {
    let repo = Repo::new("binary");
    // UTF-16 is decoded before comparing lines.
    let utf16 = |source: &str| -> Vec<u8> {
        let units = source.encode_utf16().flat_map(u16::to_le_bytes);
        [0xFF, 0xFE].into_iter().chain(units).collect()
    };
    repo.write(
        "force-app/classes/Wide.cls",
        utf16("public class Wide {\n    void a() {}\n}\n"),
    );
    repo.commit("a class in UTF-16");
    repo.write(
        "force-app/classes/Wide.cls",
        utf16("public class Wide {\n    void b() {}\n}\n"),
    );

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [
            json!(["Wide.a()", "removed", "removed"]),
            json!(["Wide.b()", "added", "hunk"]),
        ]
    );
    assert!(the_one(&rows, "file").get("binary").is_none());
}

#[test]
fn utf8_and_utf16_versions_are_compared_as_text() {
    let repo = Repo::new("binary-one-side");
    let utf16 = |source: &str| -> Vec<u8> {
        let units = source.encode_utf16().flat_map(u16::to_le_bytes);
        [0xFF, 0xFE].into_iter().chain(units).collect()
    };
    let (was_wide, now_wide) = (
        "force-app/classes/WasWide.cls",
        "force-app/classes/NowWide.cls",
    );
    repo.write(
        was_wide,
        utf16("public class WasWide {\n    void a() {}\n}\n"),
    );
    repo.write(now_wide, "public class NowWide {\n    void a() {}\n}\n");
    repo.commit("one class in UTF-16 and one in UTF-8");
    repo.write(was_wide, "public class WasWide {\n    void b() {}\n}\n");
    repo.write(
        now_wide,
        utf16("public class NowWide {\n    void b() {}\n}\n"),
    );

    let rows = repo.changed();

    assert_eq!(
        fields(&rows, "file", &["file", "binary"]),
        [json!([now_wide, null]), json!([was_wide, null])]
    );
    assert_eq!(
        symbols(&rows),
        [
            json!(["NowWide.a()", "removed", "removed"]),
            json!(["NowWide.b()", "added", "hunk"]),
            json!(["WasWide.a()", "removed", "removed"]),
            json!(["WasWide.b()", "added", "hunk"]),
        ]
    );
}

#[test]
fn a_change_to_the_companion_of_a_class_marks_the_class() {
    let repo = Repo::new("companion");
    repo.replace("force-app/classes/Invoice.cls-meta.xml", "62.0", "63.0");

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [json!(["Invoice", "modified", "whole_file"])]
    );
    assert_eq!(
        the_one(&rows, "symbol")["file"],
        "force-app/classes/Invoice.cls-meta.xml"
    );
    assert_eq!(the_one(&rows, "file")["confidence"], "full");
}

// Metadata: the file is the component.

#[test]
fn an_edit_of_a_flow_marks_the_flow() {
    let repo = Repo::new("flow-edit");
    repo.replace("force-app/flows/Welcome.flow-meta.xml", "Active", "Draft");

    let rows = repo.changed();

    assert_eq!(symbols(&rows), [json!(["Welcome", "modified", "hunk"])]);
    assert_eq!(the_one(&rows, "symbol")["type"], "flow");
}

#[test]
fn an_edit_of_one_label_marks_that_label() {
    let repo = Repo::new("label-edit");
    repo.replace(
        "force-app/labels/CustomLabels.labels-meta.xml",
        "Goodbye",
        "So long",
    );

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [json!(["Label.Farewell", "modified", "hunk"])]
    );
}

#[test]
fn an_edit_of_a_file_of_a_bundle_marks_the_bundle() {
    let repo = Repo::new("lwc-js-edit");
    repo.replace(
        "force-app/lwc/invoiceList/invoiceList.js",
        "total = 0;",
        "total = 1;",
    );
    repo.replace(
        "force-app/lwc/invoiceList/invoiceList.html",
        "<p>",
        "<p class=\"total\">",
    );

    let rows = repo.changed();

    assert_eq!(
        fields(
            &rows,
            "symbol",
            &["symbol", "type", "file", "change", "reason"]
        ),
        [
            json!([
                "invoiceList",
                "lwc",
                "force-app/lwc/invoiceList/invoiceList.html",
                "modified",
                "whole_file"
            ]),
            json!([
                "invoiceList",
                "lwc",
                "force-app/lwc/invoiceList/invoiceList.js",
                "modified",
                "whole_file"
            ]),
        ]
    );
    // Line evidence all the same.
    assert_eq!(
        rows[3]["hunks"],
        json!([{"base_start": 4, "base_lines": 1, "head_start": 4, "head_lines": 1}])
    );
}

#[test]
fn a_new_bundle_is_added_and_a_deleted_one_removed() {
    let repo = Repo::new("lwc-added-removed");
    repo.write(
        "force-app/lwc/refund/refund.js",
        "export default class Refund {}\n",
    );
    repo.write(
        "force-app/lwc/refund/refund.html",
        "<template></template>\n",
    );
    for file in [
        "invoiceList.js",
        "invoiceList.html",
        "invoiceList.js-meta.xml",
    ] {
        repo.remove(&format!("force-app/lwc/invoiceList/{file}"));
    }

    let rows = repo.changed();

    let mut changes = fields(&rows, "symbol", &["symbol", "change"]);
    changes.dedup();
    assert_eq!(
        changes,
        [
            json!(["invoiceList", "removed"]),
            json!(["refund", "added"])
        ]
    );
}

#[test]
fn a_file_that_leaves_a_bundle_marks_the_bundle_it_leaves() {
    let repo = Repo::new("lwc-file-deleted");
    repo.remove("force-app/lwc/invoiceList/invoiceList.html");

    let rows = repo.changed();

    assert_eq!(
        fields(&rows, "symbol", &["symbol", "file", "change", "reason"]),
        [json!([
            "invoiceList",
            "force-app/lwc/invoiceList/invoiceList.html",
            "modified",
            "whole_file"
        ])]
    );
    // The lines of the bundle's declaration are in another file.
    assert_eq!(the_one(&rows, "symbol").get("start"), None);
}

// Permission sets, profiles and records: the names in the entries touched.

#[test]
fn a_changed_flag_of_a_grant_touches_what_the_grant_names() {
    let repo = Repo::new("grant-flag");
    repo.replace(
        PERMISSION_SET,
        "<editable>false</editable>",
        "<editable>true</editable>",
    );

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [json!(["Billing_Admin", "modified", "hunk"])]
    );
    assert_eq!(
        touched(&rows),
        [
            json!(["base", "grant", "Invoice__c.Total__c"]),
            json!(["head", "grant", "Invoice__c.Total__c"]),
        ]
    );
    let head = &rows[3];
    assert_eq!(head["referrer"], "Billing_Admin");
    assert_eq!(head["referenced_type"], "object_field");
    assert_eq!(
        (&head["file"], &head["line"]),
        (&json!(PERMISSION_SET), &json!(9))
    );
}

#[test]
fn a_new_grant_is_touched_at_head_and_a_revoked_one_at_base() {
    let repo = Repo::new("grant-added-revoked");
    let class_access = "    <classAccesses>\n        <apexClass>Invoice</apexClass>\n        <enabled>true</enabled>\n    </classAccesses>\n";
    repo.replace(PERMISSION_SET, class_access, "");
    let flow_access = "    <flowAccesses>\n        <enabled>true</enabled>\n        <flow>Welcome</flow>\n    </flowAccesses>\n";
    repo.replace(
        PERMISSION_SET,
        "    <hasActivationRequired>",
        &format!("{flow_access}    <hasActivationRequired>"),
    );

    let rows = repo.changed();

    assert_eq!(
        touched(&rows),
        [
            json!(["base", "grant", "Invoice"]),
            json!(["head", "grant", "Welcome"])
        ]
    );
}

#[test]
fn a_changed_value_of_a_record_touches_the_name_it_had_and_the_name_it_has() {
    let repo = Repo::new("cmdt-value");
    repo.replace(RECORD, "InvoiceHandler", "PaymentHandler");

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [json!(["Plugin.Billing", "modified", "hunk"])]
    );
    assert_eq!(
        touched(&rows),
        [
            json!(["base", "cmdt_type", "Plugin__mdt"]),
            json!(["base", "cmdt_field", "Plugin__mdt.Handler__c"]),
            json!(["base", "cmdt", "InvoiceHandler"]),
            json!(["head", "cmdt_type", "Plugin__mdt"]),
            json!(["head", "cmdt_field", "Plugin__mdt.Handler__c"]),
            json!(["head", "cmdt", "PaymentHandler"]),
        ]
    );
    // The type is marked from its own line, the first, which did not
    // change: what queries `Plugin__mdt` is reached through it.
    let type_at_head = &rows[5];
    assert_eq!(
        (&type_at_head["referenced_type"], &type_at_head["line"]),
        (&json!("object"), &json!(1))
    );
}

#[test]
fn a_new_record_touches_what_its_own_name_names() {
    let repo = Repo::new("cmdt-added");
    let record = fs::read_to_string(repo.path(RECORD)).unwrap();
    repo.write(
        "force-app/customMetadata/Plugin.PaymentHandler.md-meta.xml",
        record,
    );

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [json!(["Plugin.PaymentHandler", "added", "whole_file"])]
    );
    assert_eq!(
        touched(&rows),
        [
            json!(["head", "cmdt", "PaymentHandler"]),
            json!(["head", "cmdt_type", "Plugin__mdt"]),
            json!(["head", "cmdt_field", "Plugin__mdt.Handler__c"]),
            json!(["head", "cmdt", "InvoiceHandler"]),
        ]
    );
}

#[test]
fn a_deleted_permission_set_touches_every_grant_it_held() {
    let repo = Repo::new("grant-file-deleted");
    repo.remove(PERMISSION_SET);

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [json!(["Billing_Admin", "removed", "whole_file"])]
    );
    assert_eq!(
        touched(&rows),
        [
            json!(["base", "grant", "Invoice"]),
            json!(["base", "grant", "Invoice__c.Total__c"]),
        ]
    );
}

#[test]
fn a_name_read_at_base_that_head_no_longer_declares_is_unresolved() {
    let repo = Repo::new("grant-of-deleted-class");
    let class_access = "    <classAccesses>\n        <apexClass>Invoice</apexClass>\n        <enabled>true</enabled>\n    </classAccesses>\n";
    repo.replace(PERMISSION_SET, class_access, "");
    repo.remove(INVOICE);
    repo.remove("force-app/classes/Invoice.cls-meta.xml");
    repo.remove("force-app/classes/InvoiceHandler.cls");

    let rows = repo.changed();

    assert_eq!(
        fields(
            &rows,
            "touched",
            &["side", "referenced", "referenced_type", "resolution"]
        ),
        [json!(["base", "Invoice", "class", "unresolved"])]
    );
}

#[test]
fn a_name_at_base_in_a_namespace_of_the_config_file_is_external() {
    let repo = Repo::new("grant-in-namespace");
    let class_access = "    <classAccesses>\n        <apexClass>npsp__Pricing</apexClass>\n        <enabled>true</enabled>\n    </classAccesses>\n";
    repo.write("sf-impact.json", "{\"namespaces\": [\"npsp\"]}\n");
    repo.replace(
        PERMISSION_SET,
        "    <fieldPermissions>",
        &format!("{class_access}    <fieldPermissions>"),
    );
    repo.commit("a grant of a class of an installed package");
    repo.replace(PERMISSION_SET, class_access, "");

    let rows = repo.changed();

    assert_eq!(
        fields(&rows, "touched", &["side", "referenced", "resolution"]),
        [json!(["base", "npsp__Pricing", "external"])]
    );
}

#[test]
fn a_changed_entry_that_was_not_read_lowers_the_confidence() {
    let repo = Repo::new("grant-unsupported");
    let assignment = "    <layoutAssignments>\n        <layout>Invoice__c-Invoice Layout</layout>\n    </layoutAssignments>\n";
    repo.replace(
        PERMISSION_SET,
        "    <label>",
        &format!("{assignment}    <label>"),
    );

    let rows = repo.changed();

    let file = the_one(&rows, "file");
    assert_eq!(file["confidence"], "partial");
    assert_eq!(file["gaps"], json!(["unsupported_element"]));
}

// What the model does not cover, and what is out of scope.

#[test]
fn a_change_to_an_unmodelled_file_lowers_the_confidence() {
    let repo = Repo::new("unmodelled-file");
    let layout = "force-app/layouts/Invoice__c-Invoice Layout.layout-meta.xml";
    repo.replace(layout, "Information", "Details");

    let rows = repo.changed();

    assert_eq!(rows.len(), 1);
    assert_eq!(files(&rows), [json!([layout, "modified", "partial"])]);
    assert_eq!(rows[0]["gaps"], json!(["unmodelled_file"]));
    assert_eq!(rows[0]["package"], "force-app");
}

#[test]
fn a_deleted_unmodelled_file_lowers_the_confidence_too() {
    let repo = Repo::new("unmodelled-deleted");
    let layout = "force-app/layouts/Invoice__c-Invoice Layout.layout-meta.xml";
    repo.remove(layout);

    let rows = repo.changed();

    assert_eq!(files(&rows), [json!([layout, "deleted", "partial"])]);
    assert_eq!(rows[0]["gaps"], json!(["unmodelled_file"]));
}

#[test]
fn a_change_in_scope_to_a_file_that_declares_nothing_lowers_the_confidence() {
    let repo = Repo::new("no-declaration");
    repo.write("force-app/staticresources/rates.json", "{}\n");

    let rows = repo.changed();

    assert_eq!(
        files(&rows),
        [json!([
            "force-app/staticresources/rates.json",
            "added",
            "partial"
        ])]
    );
    assert_eq!(rows[0]["gaps"], json!(["no_declaration"]));
}

#[test]
fn a_change_outside_the_package_directories_is_listed_as_out_of_scope() {
    let repo = Repo::new("out-of-scope");
    repo.write("docs/notes.md", "Changed.\n");
    repo.write("scripts/deploy.cls", "public class NotInScope {}\n");

    let rows = repo.changed();

    assert_eq!(
        fields(&rows, "file", &["file", "status", "package", "confidence"]),
        [
            json!(["docs/notes.md", "modified", null, "full"]),
            json!(["scripts/deploy.cls", "added", null, "full"]),
        ]
    );
    assert_eq!(rows.len(), 2);
}

#[test]
fn a_file_deleted_from_a_package_directory_that_is_deleted_with_it_is_in_scope() {
    let repo = Repo::new("package-directory-removed");
    repo.remove("plugins/billing/classes/Ledger.cls");
    repo.remove("plugins/billing/classes/Ledger.cls-meta.xml");
    repo.replace(
        "sfdx-project.json",
        ",\n    { \"path\": \"plugins/billing\" }",
        "",
    );

    let rows = repo.changed();

    assert_eq!(
        fields(&rows, "symbol", &["symbol", "package", "change"]),
        [
            json!(["Ledger", "plugins/billing", "removed"]),
            json!(["Ledger.post()", "plugins/billing", "removed"]),
            json!(["Ledger", "plugins/billing", "removed"]),
        ]
    );
    // The scope moved, and here every file that left it is in the diff.
    assert_eq!(
        project_row(&rows),
        json!([null, "partial", ["scope_changed"]])
    );
    assert_eq!(
        fields(&rows, "file", &["file", "package"]),
        [
            json!(["plugins/billing/classes/Ledger.cls", "plugins/billing"]),
            json!([
                "plugins/billing/classes/Ledger.cls-meta.xml",
                "plugins/billing"
            ]),
            json!(["sfdx-project.json", null]),
        ]
    );
}

#[test]
fn a_file_that_becomes_a_symbolic_link_no_longer_declares_what_it_did() {
    let repo = Repo::new("type-changed");
    repo.remove("force-app/classes/PaymentHandler.cls");
    std::os::unix::fs::symlink(
        "InvoiceHandler.cls",
        repo.path("force-app/classes/PaymentHandler.cls"),
    )
    .unwrap();

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [
            json!(["PaymentHandler", "removed", "removed"]),
            json!(["PaymentHandler.handle()", "removed", "removed"]),
        ]
    );
    assert_eq!(the_one(&rows, "file")["status"], "type_changed");
}

/// A class and a field of one line each, without a line end: what a link
/// leads to is kept in a blob, and a link that "leads to" this text is the
/// very blob of the file.
const TINY: [(&str, &str); 2] = [
    ("force-app/classes/Tiny.cls", "public class Tiny {}"),
    (
        "force-app/objects/Invoice__c/fields/Tiny__c.field-meta.xml",
        "<CustomField><fullName>Tiny__c</fullName><type>Text</type></CustomField>",
    ),
];

#[test]
fn a_file_that_becomes_a_link_with_the_blob_it_had_no_longer_declares_what_it_did() {
    let repo = Repo::new("type-changed-same-blob");
    for (file, content) in TINY {
        repo.write(file, content);
    }
    repo.commit("a class and a field of one line");
    for (file, content) in TINY {
        repo.remove(file);
        std::os::unix::fs::symlink(content, repo.path(file)).unwrap();
    }

    let rows = repo.changed();

    // The same blob with another mode, and not a change of mode alone.
    assert_eq!(
        symbols(&rows),
        [
            json!(["Tiny", "removed", "removed"]),
            json!(["Invoice__c.Tiny__c", "removed", "removed"]),
        ]
    );
    assert_eq!(
        files(&rows),
        [
            json!([TINY[0].0, "type_changed", "full"]),
            json!([TINY[1].0, "type_changed", "full"]),
        ]
    );
}

#[test]
fn a_link_that_becomes_a_file_with_the_blob_it_had_declares_what_the_file_does() {
    let repo = Repo::new("type-changed-same-blob-to-file");
    for (file, content) in TINY {
        fs::create_dir_all(repo.path(file).parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(content, repo.path(file)).unwrap();
    }
    repo.commit("two links that lead nowhere");
    for (file, content) in TINY {
        repo.remove(file);
        repo.write(file, content);
    }

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [
            json!(["Tiny", "added", "whole_file"]),
            json!(["Invoice__c.Tiny__c", "added", "whole_file"]),
        ]
    );
    assert_eq!(
        files(&rows),
        [
            json!([TINY[0].0, "type_changed", "full"]),
            json!([TINY[1].0, "type_changed", "full"]),
        ]
    );
}

#[test]
fn a_class_declared_in_two_packages_is_marked_where_it_changed_and_flagged() {
    let repo = Repo::new("collision");
    let copy = fs::read_to_string(repo.path("force-app/classes/PaymentHandler.cls")).unwrap();
    repo.write("plugins/billing/classes/PaymentHandler.cls", copy);
    repo.commit("the same class in a second package");
    repo.replace(
        "plugins/billing/classes/PaymentHandler.cls",
        "paid = paid + 1;",
        "paid = paid + 2;",
    );

    let rows = repo.changed();

    // Only the declaration whose file changed, and it says there is another.
    assert_eq!(
        fields(
            &rows,
            "symbol",
            &["symbol", "package", "change", "collision"]
        ),
        [json!([
            "PaymentHandler.handle()",
            "plugins/billing",
            "modified",
            true
        ])]
    );
}

#[test]
fn a_submodule_in_scope_is_listed_and_nothing_is_read_from_it() {
    let repo = Repo::new("submodule");
    // A submodule is a commit of another repository: this one has no
    // object by that id.
    let pin = |commit: &str, message: &str| {
        let entry = format!("160000,{commit},force-app/vendor");
        run(git(&repo.root()).args(["update-index", "--add", "--cacheinfo", &entry]));
        run(git(&repo.root()).args(["commit", "--quiet", "--message", message]));
    };
    pin("1111111111111111111111111111111111111111", "a submodule");
    pin("2222222222222222222222222222222222222222", "head");

    let output = repo.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
    check_golden(repo.scenario, &output);

    let rows = rows(&output);
    assert_eq!(
        files(&rows),
        [json!(["force-app/vendor", "modified", "partial"])]
    );
    assert_eq!(rows[0]["gaps"], json!(["no_declaration"]));
}

#[test]
fn a_file_that_does_not_parse_cleanly_lowers_the_confidence() {
    let repo = Repo::new("parse-error");
    repo.replace(INVOICE, "return amount * RATE;", "return amount * (RATE;");

    let rows = repo.changed();

    let file = the_one(&rows, "file");
    assert_eq!(file["confidence"], "partial");
    assert_eq!(file["gaps"], json!(["parse_error"]));
}

// Paths.

#[test]
fn a_path_with_a_space_is_read_as_it_is() {
    let repo = Repo::new("path-with-space");
    repo.replace(
        "force-app/shipping area/classes/Parcel.cls",
        "return 1;",
        "return 2;",
    );

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [json!(["Parcel.weight()", "modified", "hunk"])]
    );
    assert_eq!(
        files(&rows),
        [json!([
            "force-app/shipping area/classes/Parcel.cls",
            "modified",
            "full"
        ])]
    );
}

#[test]
fn a_new_bundle_under_a_path_with_a_space_is_added() {
    let repo = Repo::new("lwc-added-under-space");
    repo.write(
        "force-app/shipping area/lwc/tracker/tracker.js",
        "export default class Tracker {}\n",
    );

    let rows = repo.changed();

    assert_eq!(symbols(&rows), [json!(["tracker", "added", "whole_file"])]);
}

#[test]
fn a_path_that_is_not_ascii_is_read_as_it_is() {
    let repo = Repo::new("path-not-ascii");
    let path = "force-app/expédition/classes/Colis.cls";
    repo.write(path, "public class Colis {\n}\n");
    repo.commit("a class under a directory with an accent");
    repo.replace(path, "public class", "public virtual class");

    let rows = repo.changed();

    assert_eq!(symbols(&rows), [json!(["Colis", "modified", "hunk"])]);
    assert_eq!(the_one(&rows, "file")["file"], path);
}

#[test]
fn a_project_below_the_top_of_its_repository_has_paths_from_its_own_root() {
    let repo = Repo::within("project-in-subdirectory", "salesforce");
    repo.replace(INVOICE, "return amount * RATE;", "return 0;");
    fs::write(repo.root().join("build.sh"), "#!/bin/sh\n").unwrap();

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [json!(["Invoice.tax()", "modified", "hunk"])]
    );
    assert_eq!(
        fields(&rows, "file", &["file", "package"]),
        [json!(["../build.sh", null]), json!([INVOICE, "force-app"])]
    );
}

// The command line.

#[test]
fn the_merge_base_of_a_branch_stands_in_for_the_base() {
    let repo = Repo::new("merge-base");
    run(git(&repo.root()).args(["switch", "--quiet", "--create", "feature"]));
    repo.replace(INVOICE, "return amount * RATE;", "return 0;");
    repo.commit("feature");
    run(git(&repo.root()).args(["switch", "--quiet", "main"]));
    repo.replace(INVOICE, "RATE = 0.2;", "RATE = 0.3;");
    repo.commit("main moves on");
    run(git(&repo.root()).args(["switch", "--quiet", "feature"]));

    let from_merge_base = repo.run_changed(&["--merge-base", "main", "--head", "feature"], &[]);
    let from_main = repo.run_changed(&["--base", "main", "--head", "feature"], &[]);

    // Only what the branch did, not the reverse of what main did since.
    assert_eq!(
        symbols(&rows(&from_merge_base)),
        [json!(["Invoice.tax()", "modified", "hunk"])]
    );
    assert_eq!(symbols(&rows(&from_main)).len(), 2);
}

#[test]
fn the_output_can_go_to_a_file() {
    let repo = Repo::new("out-file");
    repo.replace(INVOICE, "return amount * RATE;", "return 0;");
    repo.commit("head");
    let out = repo.dir.join("changed.jsonl");

    let printed = repo.run_changed(
        &["--base", "HEAD~1", "--head", "HEAD"],
        &["--out", out.to_str().unwrap()],
    );

    assert_eq!(printed, "");
    let written = fs::read_to_string(out).unwrap();
    assert_eq!(
        symbols(&rows(&written)),
        [json!(["Invoice.tax()", "modified", "hunk"])]
    );
}

#[test]
fn the_refs_can_come_from_standard_input() {
    let repo = Repo::new("refs-from-stdin");
    repo.replace(INVOICE, "return amount * RATE;", "return 0;");
    repo.commit("head");
    let refs_file = repo.dir.join("piped.pack");
    fs::write(&refs_file, pack_of(&repo.path(""))).unwrap();

    let lines = sf_impact_delta::delta(
        &repo.path(""),
        false,
        &sf_impact_delta::Base::Commit("HEAD~1".into()),
        "HEAD",
    );
    let delta_file = repo.dir.join("delta.jsonl");
    fs::write(&delta_file, lines.unwrap().join("\n") + "\n").unwrap();

    let output = run(Command::new(env!("CARGO_BIN_EXE_sf-impact-changed"))
        .args(["--refs", "-", "--delta"])
        .arg(delta_file)
        .stdin(fs::File::open(refs_file).unwrap()));

    let output = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        symbols(&rows(&output)),
        [json!(["Invoice.tax()", "modified", "hunk"])]
    );
}

#[test]
fn a_submodule_where_a_class_would_be_is_not_read_and_lowers_the_confidence() {
    let repo = Repo::new("submodule-named-as-a-class");
    // A path that the model reads, and no file of text at it: the commit
    // of another repository, which this one does not have.
    let pin = |commit: &str, message: &str| {
        let entry = format!("160000,{commit},force-app/classes/Vendor.cls");
        run(git(&repo.root()).args(["update-index", "--add", "--cacheinfo", &entry]));
        run(git(&repo.root()).args(["commit", "--quiet", "--message", message]));
    };
    pin("1111111111111111111111111111111111111111", "a submodule");
    pin("2222222222222222222222222222222222222222", "head");

    let output = repo.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
    check_golden(repo.scenario, &output);

    let rows = rows(&output);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        fields(&rows, "file", &["file", "status", "confidence", "gaps"]),
        [json!([
            "force-app/classes/Vendor.cls",
            "modified",
            "partial",
            ["no_declaration"]
        ])]
    );
}

#[test]
fn the_same_diff_gives_the_same_bytes() {
    let repo = Repo::new("deterministic");
    repo.replace(INVOICE, "return amount * RATE;", "return 0;");
    repo.replace(
        PERMISSION_SET,
        "<editable>false</editable>",
        "<editable>true</editable>",
    );
    repo.remove("force-app/classes/PaymentHandler.cls");
    repo.commit("head");

    let first = repo.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
    let second = repo.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);

    assert!(!first.is_empty());
    assert_eq!(first, second);
}

// Regression cases: each of these was a probe that found a change reported
// with full confidence and without the symbol it changes.

const PROJECT: &str = "sfdx-project.json";

/// The row of the project file: its package, confidence and gaps.
fn project_row(rows: &[Value]) -> Value {
    let row = rows.iter().find(|row| row["file"] == PROJECT);
    let row = row.expect("the project file has a row");
    json!([row["package"], row["confidence"], row["gaps"]])
}

#[test]
fn a_directory_that_enters_scope_with_no_file_of_it_changed_lowers_the_confidence() {
    let repo = Repo::new("scope-added");
    repo.write(
        "outside/classes/Extra.cls",
        "public class Extra {\n    public void run() {\n    }\n}\n",
    );
    repo.commit("a class that no package directory holds");
    repo.replace(
        PROJECT,
        "{ \"path\": \"plugins/billing\" }",
        "{ \"path\": \"plugins/billing\" },\n    { \"path\": \"outside\" }",
    );

    let rows = repo.changed();

    // `Extra` is in scope at head and was not at base, and no file of it
    // is in the diff: the rows cannot say so, and the project file does.
    assert_eq!(
        project_row(&rows),
        json!([null, "partial", ["scope_changed"]])
    );
    assert_eq!(rows.len(), 1);
}

#[test]
fn a_directory_that_leaves_scope_with_no_file_of_it_changed_lowers_the_confidence() {
    let repo = Repo::new("scope-removed");
    repo.replace(PROJECT, ",\n    { \"path\": \"plugins/billing\" }", "");

    let rows = repo.changed();

    assert_eq!(
        project_row(&rows),
        json!([null, "partial", ["scope_changed"]])
    );
}

#[test]
fn a_nested_package_directory_changes_what_its_files_are_and_lowers_the_confidence() {
    let repo = Repo::new("scope-nested");
    repo.replace(
        PROJECT,
        "{ \"path\": \"plugins/billing\" }",
        "{ \"path\": \"plugins/billing\" },\n    { \"path\": \"force-app/classes\" }",
    );

    let rows = repo.changed();

    assert_eq!(
        project_row(&rows),
        json!([null, "partial", ["scope_changed"]])
    );
}

#[test]
fn a_project_file_that_declares_the_same_scope_differently_leaves_no_gap() {
    let repo = Repo::new("scope-same");
    repo.replace(
        PROJECT,
        "\"sourceApiVersion\": \"62.0\"",
        "\"sourceApiVersion\": \"63.0\"",
    );
    repo.replace(
        PROJECT,
        "{ \"path\": \"plugins/billing\" }",
        "{ \"path\": \"./plugins\\\\billing/\" }",
    );

    let rows = repo.changed();

    assert_eq!(project_row(&rows), json!([null, "full", null]));
}

const SHARED: &str = "force-app/classes/Shared.cls";

#[test]
fn a_line_that_two_methods_share_marks_both_and_the_class_around_them() {
    let repo = Repo::new("shared-line-signature");
    repo.write(
        SHARED,
        "public class Shared {\n Integer first() { return 1; } Integer second() {\n   return 2;\n }\n}\n",
    );
    repo.commit("two methods that meet on a line");
    repo.replace(SHARED, "Integer second()", "Decimal second()");

    let rows = repo.changed();

    // The line cannot say which of them changed. It was `second`.
    assert_eq!(
        symbols(&rows),
        [
            json!(["Shared", "modified", "hunk"]),
            json!(["Shared.first()", "modified", "hunk"]),
            json!(["Shared.second()", "modified", "hunk"]),
        ]
    );
}

#[test]
fn a_class_header_that_shares_its_line_with_a_method_marks_the_class_too() {
    let repo = Repo::new("shared-line-class-header");
    repo.write(
        SHARED,
        "public with sharing class Shared { Integer small() { return 1; }\n Integer second() {\n  return 2;\n }\n}\n",
    );
    repo.commit("a method on the line of the class header");
    repo.replace(SHARED, "with sharing", "without sharing");

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [
            json!(["Shared", "modified", "hunk"]),
            json!(["Shared.small()", "modified", "hunk"]),
        ]
    );
}

#[test]
fn a_class_header_that_goes_on_to_the_line_of_a_member_marks_the_class() {
    let repo = Repo::new("boundary-class-header");
    repo.write(
        "force-app/classes/Base.cls",
        "public virtual class Base {\n}\n",
    );
    repo.write(
        "force-app/classes/Other.cls",
        "public virtual class Other {\n}\n",
    );
    repo.write(
        SHARED,
        "public class Shared\n extends Base { Integer small() { return 1; }\n Integer second() { return 2; }\n}\n",
    );
    repo.commit("a class header that ends on the line of a method");
    repo.replace(SHARED, "extends Base", "extends Other");

    let rows = repo.changed();

    // Only `small` starts or ends on the line. What changed is the class.
    assert_eq!(
        symbols(&rows),
        [
            json!(["Shared", "modified", "hunk"]),
            json!(["Shared.small()", "modified", "hunk"]),
        ]
    );
}

#[test]
fn an_initializer_that_ends_on_the_line_of_a_member_marks_the_class() {
    let repo = Repo::new("boundary-initializer");
    repo.write(
        SHARED,
        "public class Shared {\n static {\n System.debug(1); } Integer small() { return 1; }\n Integer second() { return 2; }\n}\n",
    );
    repo.commit("an initializer that ends on the line of a method");
    repo.replace(SHARED, "debug(1)", "debug(2)");

    let rows = repo.changed();

    // An initializer is declared by nothing but its class.
    assert_eq!(
        symbols(&rows),
        [
            json!(["Shared", "modified", "hunk"]),
            json!(["Shared.small()", "modified", "hunk"]),
        ]
    );
}

#[test]
fn an_initializer_taken_from_the_last_line_of_a_member_marks_the_class() {
    let repo = Repo::new("boundary-removed-initializer");
    repo.write(
        SHARED,
        "public class Shared {\n Integer small() {\n  return 1; } static { System.debug(1); }\n Integer second() { return 2; }\n}\n",
    );
    repo.commit("an initializer on the last line of a method");
    repo.replace(
        SHARED,
        "  return 1; } static { System.debug(1); }\n",
        "  return 1;\n }\n",
    );

    let rows = repo.changed();

    // At head the method has its lines to itself. The line it ended on at
    // base, which is gone, held the initializer too.
    assert_eq!(
        symbols(&rows),
        [
            json!(["Shared", "modified", "hunk"]),
            json!(["Shared.small()", "modified", "hunk"]),
        ]
    );
}

const COMPANION: &str = "force-app/classes/Invoice.cls-meta.xml";

/// Symbol, change, reason and file of every symbol row.
fn symbols_by_file(rows: &[Value]) -> Vec<Value> {
    fields(rows, "symbol", &["symbol", "change", "reason", "file"])
}

#[test]
fn two_files_with_the_same_content_that_change_alike_each_have_their_lines() {
    let repo = Repo::new("same-blobs");
    // Every companion in the project is the same bytes, before and after.
    let other = "force-app/classes/InvoiceHandler.cls-meta.xml";
    repo.replace(COMPANION, "62.0", "63.0");
    repo.replace(other, "62.0", "63.0");

    let rows = repo.changed();

    let hunk = json!([{"base_start": 3, "base_lines": 1, "head_start": 3, "head_lines": 1}]);
    assert_eq!(
        fields(&rows, "symbol", &["symbol", "file", "hunks"]),
        [
            json!(["Invoice", COMPANION, hunk]),
            json!(["InvoiceHandler", other, hunk]),
        ]
    );
    assert_eq!(
        fields(&rows, "file", &["binary"]),
        [json!([null]), json!([null])]
    );
}

#[test]
fn a_companion_that_is_added_to_a_class_marks_the_class() {
    let repo = Repo::new("companion-added");
    let companion = fs::read_to_string(repo.path(COMPANION)).unwrap();
    repo.remove(COMPANION);
    repo.commit("a class without its companion");
    repo.write(COMPANION, companion);

    let rows = repo.changed();

    assert_eq!(
        symbols_by_file(&rows),
        [json!(["Invoice", "modified", "whole_file", COMPANION])]
    );
    assert_eq!(files(&rows), [json!([COMPANION, "added", "full"])]);
}

#[test]
fn a_companion_that_is_deleted_from_a_class_marks_the_class() {
    let repo = Repo::new("companion-deleted");
    repo.remove(COMPANION);

    let rows = repo.changed();

    assert_eq!(
        symbols_by_file(&rows),
        [json!(["Invoice", "modified", "whole_file", COMPANION])]
    );
    assert_eq!(files(&rows), [json!([COMPANION, "deleted", "full"])]);
}

#[test]
fn a_utf16_companion_change_marks_the_class() {
    let repo = Repo::new("companion-binary");
    let companion = fs::read_to_string(repo.path(COMPANION)).unwrap();
    let utf16 = |text: &str| -> Vec<u8> {
        let units = text.encode_utf16().flat_map(u16::to_le_bytes);
        [0xFF, 0xFE].into_iter().chain(units).collect()
    };
    repo.write(COMPANION, utf16(&companion.replace("62.0", "63.0")));

    let rows = repo.changed();

    assert_eq!(
        symbols_by_file(&rows),
        [json!(["Invoice", "modified", "whole_file", COMPANION])]
    );
    assert!(the_one(&rows, "file").get("binary").is_none());
}

#[test]
fn a_companion_renamed_to_another_class_marks_the_class_it_left_and_the_class_it_joined() {
    let repo = Repo::new("companion-renamed-owner");
    repo.write("force-app/classes/Refund.cls", "public class Refund {\n}\n");
    repo.commit("a class without a companion");
    let (from, to) = (
        "force-app/classes/PaymentHandler.cls-meta.xml",
        "force-app/classes/Refund.cls-meta.xml",
    );
    repo.rename(from, to);

    let rows = repo.changed();

    assert_eq!(
        symbols_by_file(&rows),
        [
            json!(["PaymentHandler", "modified", "whole_file", from]),
            json!(["Refund", "modified", "whole_file", to]),
        ]
    );
    assert_eq!(the_one(&rows, "file")["status"], "moved");
}

#[test]
fn a_companion_that_moves_with_its_class_and_does_not_change_marks_nothing() {
    let repo = Repo::new("companion-moved");
    repo.rename(INVOICE, "force-app/classes/billing/Invoice.cls");
    repo.rename(COMPANION, "force-app/classes/billing/Invoice.cls-meta.xml");

    let rows = repo.changed();

    assert_eq!(symbols(&rows), [] as [Value; 0]);
    assert_eq!(
        fields(&rows, "file", &["status", "confidence"]),
        [json!(["moved", "full"]), json!(["moved", "full"])]
    );
}

#[test]
fn a_class_and_its_companion_that_move_and_change_are_still_the_class_and_its_companion() {
    let repo = Repo::new("moved-with-companion");
    let (class, companion) = (
        "force-app/classes/billing/Invoice.cls",
        "force-app/classes/billing/Invoice.cls-meta.xml",
    );
    repo.rename(INVOICE, class);
    repo.rename(COMPANION, companion);
    repo.replace(class, "return amount * RATE;", "return 0;");
    repo.replace(companion, "62.0", "63.0");

    let rows = repo.changed();

    // Four files in the delta, two deleted and two added. Nothing is
    // removed: the class is the class, and the companion is its companion.
    assert_eq!(
        fields(
            &rows,
            "symbol",
            &["symbol", "change", "reason", "file", "previous_file"]
        ),
        [
            json!(["Invoice.tax()", "modified", "hunk", class, INVOICE]),
            json!(["Invoice", "modified", "whole_file", companion, COMPANION]),
        ]
    );
    assert_eq!(
        fields(&rows, "file", &["status", "confidence"]),
        [
            json!(["deleted", "full"]),
            json!(["deleted", "full"]),
            json!(["added", "full"]),
            json!(["added", "full"]),
        ]
    );
}

#[test]
fn a_companion_of_no_class_lowers_the_confidence() {
    let repo = Repo::new("companion-orphan");
    let companion = fs::read_to_string(repo.path(COMPANION)).unwrap();
    repo.write("force-app/classes/Ghost.cls-meta.xml", companion);

    let rows = repo.changed();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["gaps"], json!(["no_declaration"]));
}

const HANDLER: &str = "force-app/classes/PaymentHandler.cls";

#[test]
fn an_apex_file_renamed_into_a_bundle_removes_what_it_declared_and_marks_the_bundle() {
    let repo = Repo::new("rename-apex-to-lwc");
    let into = "force-app/lwc/invoiceList/source.txt";
    repo.rename(HANDLER, into);

    let rows = repo.changed();

    assert_eq!(
        symbols_by_file(&rows),
        [
            json!(["PaymentHandler", "removed", "removed", HANDLER]),
            json!(["PaymentHandler.handle()", "removed", "removed", HANDLER]),
            json!(["invoiceList", "modified", "whole_file", into]),
        ]
    );
    assert_eq!(the_one(&rows, "file")["status"], "moved");
}

#[test]
fn a_bundle_file_renamed_into_an_apex_file_marks_the_bundle_and_adds_what_it_declares() {
    let repo = Repo::new("rename-lwc-to-apex");
    let (from, to) = (
        "force-app/lwc/invoiceList/source.txt",
        "force-app/classes/Mover.cls",
    );
    repo.write(
        from,
        "public class Mover {\n    public void run() {\n    }\n}\n",
    );
    repo.commit("Apex in a file of a bundle");
    repo.rename(from, to);

    let rows = repo.changed();

    assert_eq!(
        symbols_by_file(&rows),
        [
            json!(["Mover", "added", "whole_file", to]),
            json!(["Mover.run()", "added", "whole_file", to]),
            json!(["invoiceList", "modified", "whole_file", from]),
        ]
    );
}

#[test]
fn an_apex_file_renamed_to_a_companion_removes_what_it_declared_and_lowers_the_confidence() {
    let repo = Repo::new("rename-apex-to-companion");
    let companion = "force-app/classes/PaymentHandler.cls-meta.xml";
    repo.remove(companion);
    repo.commit("a class without its companion");
    repo.rename(HANDLER, companion);

    let rows = repo.changed();

    assert_eq!(
        symbols_by_file(&rows),
        [
            json!(["PaymentHandler", "removed", "removed", HANDLER]),
            json!(["PaymentHandler.handle()", "removed", "removed", HANDLER]),
        ]
    );
    // The companion at head is of a class that is no longer there.
    assert_eq!(the_one(&rows, "file")["gaps"], json!(["no_declaration"]));
}

#[test]
fn an_apex_file_renamed_to_an_unmodelled_file_removes_what_it_declared_and_lowers_the_confidence() {
    let repo = Repo::new("rename-apex-to-unmodelled");
    repo.rename(HANDLER, "force-app/layouts/Moved.layout-meta.xml");

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [
            json!(["PaymentHandler", "removed", "removed"]),
            json!(["PaymentHandler.handle()", "removed", "removed"]),
        ]
    );
    assert_eq!(the_one(&rows, "file")["gaps"], json!(["unmodelled_file"]));
}

#[test]
fn a_removed_value_of_a_record_marks_the_type_of_the_record_at_base_and_at_head() {
    let repo = Repo::new("cmdt-value-removed");
    let values = "    <values>\n        <field>Handler__c</field>\n        <value xsi:type=\"xsd:string\">InvoiceHandler</value>\n    </values>\n";
    repo.replace(RECORD, values, "");

    let rows = repo.changed();

    assert_eq!(
        touched(&rows),
        [
            json!(["base", "cmdt_type", "Plugin__mdt"]),
            json!(["base", "cmdt_field", "Plugin__mdt.Handler__c"]),
            json!(["base", "cmdt", "InvoiceHandler"]),
            json!(["head", "cmdt_type", "Plugin__mdt"]),
        ]
    );
}

#[test]
fn a_changed_value_of_a_record_keeps_what_the_name_of_the_record_binds() {
    let repo = Repo::new("cmdt-developer-name");
    // Bound by its own name, as a framework that loads `Plugin.<class>` does.
    let named = "force-app/customMetadata/Plugin.InvoiceHandler.md-meta.xml";
    let record = fs::read_to_string(repo.path(RECORD)).unwrap();
    repo.write(named, record.replace("InvoiceHandler", "Unbound"));
    repo.commit("a record named after a class");
    repo.replace(named, "<label>Billing</label>", "<label>Invoices</label>");

    let rows = repo.changed();

    assert_eq!(
        touched(&rows),
        [
            json!(["base", "cmdt", "InvoiceHandler"]),
            json!(["base", "cmdt_type", "Plugin__mdt"]),
            json!(["head", "cmdt", "InvoiceHandler"]),
            json!(["head", "cmdt_type", "Plugin__mdt"]),
        ]
    );
}

#[test]
fn a_relationship_field_declared_twice_keeps_what_each_declaration_makes_of_a_base_value() {
    let repo = Repo::new("cmdt-divergent-controlling");
    let field = |text: &str| format!("<CustomField>\n{text}\n</CustomField>\n");
    let target = |controlling: &str| {
        field(&format!(
            "<type>MetadataRelationship</type>\n<referenceTo>FieldDefinition</referenceTo>\n\
             <metadataRelationshipControllingField>Plugin__mdt.{controlling}</metadataRelationshipControllingField>"
        ))
    };
    let object = "<type>MetadataRelationship</type>\n<referenceTo>EntityDefinition</referenceTo>";
    let fields_of_type = "force-app/objects/Plugin__mdt/fields";
    // The same field, which says which object it is about by another field
    // in each of the two packages that declare it.
    repo.write(
        &format!("{fields_of_type}/Target__c.field-meta.xml"),
        target("Left__c"),
    );
    repo.write(
        "plugins/billing/objects/Plugin__mdt/fields/Target__c.field-meta.xml",
        target("Right__c"),
    );
    repo.write(
        &format!("{fields_of_type}/Left__c.field-meta.xml"),
        field(object),
    );
    repo.write(
        &format!("{fields_of_type}/Right__c.field-meta.xml"),
        field(object),
    );
    for name in ["LeftObject__c", "RightObject__c"] {
        let directory = format!("force-app/objects/{name}");
        repo.write(
            &format!("{directory}/{name}.object-meta.xml"),
            "<CustomObject>\n<label>Object</label>\n</CustomObject>\n",
        );
        for text in ["Before__c", "After__c"] {
            let path = format!("{directory}/fields/{text}.field-meta.xml");
            repo.write(&path, field("<type>Text</type>"));
        }
    }
    let routing = "force-app/customMetadata/Plugin.Routing.md-meta.xml";
    let value = |field: &str, value: &str| {
        format!(" <values>\n  <field>{field}</field>\n  <value>{value}</value>\n </values>\n")
    };
    repo.write(
        routing,
        format!(
            "<CustomMetadata>\n{}{}{}</CustomMetadata>\n",
            value("Left__c", "LeftObject__c"),
            value("Right__c", "RightObject__c"),
            value("Target__c", "Before__c"),
        ),
    );
    repo.commit("a relationship field declared twice");
    repo.replace(routing, "Before__c", "After__c");

    let rows = repo.changed();

    assert_eq!(
        touched(&rows),
        [
            json!(["base", "cmdt_type", "Plugin__mdt"]),
            json!(["base", "cmdt_field", "Plugin__mdt.Target__c"]),
            json!(["base", "cmdt", "LeftObject__c.Before__c"]),
            json!(["base", "cmdt", "RightObject__c.Before__c"]),
            json!(["head", "cmdt_type", "Plugin__mdt"]),
            json!(["head", "cmdt_field", "Plugin__mdt.Target__c"]),
            json!(["head", "cmdt", "LeftObject__c.After__c"]),
            json!(["head", "cmdt", "RightObject__c.After__c"]),
        ]
    );
}

#[test]
fn a_submodule_that_is_a_package_directory_is_in_scope_and_lowers_the_confidence() {
    let repo = Repo::new("submodule-at-package-root");
    repo.replace(
        PROJECT,
        "{ \"path\": \"plugins/billing\" }",
        "{ \"path\": \"plugins/billing\" },\n    { \"path\": \"vendor\" }",
    );
    let pin = |commit: &str, message: &str| {
        let entry = format!("160000,{commit},vendor");
        run(git(&repo.root()).args(["add", PROJECT]));
        run(git(&repo.root()).args(["update-index", "--add", "--cacheinfo", &entry]));
        run(git(&repo.root()).args(["commit", "--quiet", "--message", message]));
    };
    pin(
        "1111111111111111111111111111111111111111",
        "a package that is a submodule",
    );
    pin("2222222222222222222222222222222222222222", "head");

    let output = repo.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
    check_golden(repo.scenario, &output);

    // What it holds changed with the commit it is pinned to, unread.
    assert_eq!(
        fields(
            &rows(&output),
            "file",
            &["file", "package", "confidence", "gaps"]
        ),
        [json!(["vendor", "vendor", "partial", ["no_declaration"]])]
    );
}

#[test]
fn a_symbolic_link_that_is_a_package_directory_is_in_scope_and_is_not_followed() {
    let repo = Repo::new("symlink-at-package-root");
    repo.replace(
        PROJECT,
        "{ \"path\": \"plugins/billing\" }",
        "{ \"path\": \"plugins/billing\" },\n    { \"path\": \"linked\" }",
    );
    std::os::unix::fs::symlink("docs", repo.path("linked")).unwrap();
    repo.commit("a package that is a symbolic link");
    fs::remove_file(repo.path("linked")).unwrap();
    std::os::unix::fs::symlink("force-app", repo.path("linked")).unwrap();

    let rows = repo.changed();

    assert_eq!(
        fields(&rows, "file", &["file", "package", "confidence", "gaps"]),
        [json!(["linked", "linked", "partial", ["no_declaration"]])]
    );
    // Nothing of `force-app`, which the link now leads to.
    assert_eq!(rows.len(), 1);
}

/// A class whose one method calls `steps` in order, a line each.
fn steps(steps: &str) -> String {
    let calls: String = steps
        .split(' ')
        .map(|step| format!("        {step}();\n"))
        .collect();
    format!("public class Steps {{\n    public void run() {{\n{calls}    }}\n}}\n")
}

/// A class of methods that each have a comment above them.
fn documented(methods: &[&str]) -> String {
    let methods: Vec<String> = methods
        .iter()
        .map(|name| {
            format!(
                "    /**\n     * {name}.\n     */\n    void {name}() {{\n        go();\n    }}\n"
            )
        })
        .collect();
    format!("public class Documented {{\n{}}}\n", methods.join("\n"))
}

/// Every setting here changes what `git diff` prints for the two commits
/// of this repository, or how. None of them may change the rows.
#[test]
fn the_configuration_of_git_does_not_change_the_output() {
    let repo = Repo::within("git-configuration", "project");
    // Four files that move to new names with an edit: renames that git
    // finds by comparing content, which a rename limit can turn off.
    let class = |name: &str, body: &str| {
        format!(
            "public class {name} {{\n    public Integer first() {{\n        return {body};\n    }}\n\n    \
             public Integer second() {{\n        Integer total = 0;\n        total = total + 1;\n        \
             return total;\n    }}\n}}\n"
        )
    };
    for number in 0..4 {
        let name = format!("Target{number}");
        repo.write(&format!("force-app/old/Old{number}.cls"), class(&name, "1"));
    }
    // Lines that the algorithms pair differently, and a method whose
    // comment the indent heuristic moves the hunk around.
    repo.write("force-app/classes/Steps.cls", steps("a b c a b b a"));
    repo.write(
        "force-app/classes/Documented.cls",
        documented(&["first", "third"]),
    );
    // A file that moves with an edit and with other line ends. As text it
    // is nearly the same file; byte for byte, every line of it differs.
    // Whether git pairs the two depends on which it takes the file for.
    let wound: String = (1..=12)
        .map(|n| format!("    public Integer step{n}() {{\n        return {n};\n    }}\n"))
        .collect();
    let wound = format!("public class Wound {{\n{wound}}}\n");
    repo.write("force-app/old/Wound.cls", wound.replace('\n', "\r\n"));
    let pin = |commit: &str, message: &str| {
        let entry = format!("160000,{commit},project/force-app/vendor");
        run(git(&repo.root()).args(["add", "--all"]));
        run(git(&repo.root()).args(["update-index", "--add", "--cacheinfo", &entry]));
        run(git(&repo.root()).args(["commit", "--quiet", "--message", message]));
    };
    pin("1111111111111111111111111111111111111111", "base");
    for number in 0..4 {
        let name = format!("Target{number}");
        repo.remove(&format!("force-app/old/Old{number}.cls"));
        repo.write(
            &format!("force-app/new/New{number}.cls"),
            class(&name, "100"),
        );
    }
    repo.remove("force-app/old/Wound.cls");
    repo.write(
        "force-app/new/Wound.cls",
        wound.replace("return 7;", "return 70;"),
    );
    repo.write("force-app/classes/Steps.cls", steps("c b a b a c"));
    repo.write(
        "force-app/classes/Documented.cls",
        documented(&["first", "second", "third"]),
    );
    repo.write(
        "force-app/classes/Caf\u{e9}.cls",
        "public class Cafe {\n}\n",
    );
    fs::write(repo.root().join("outside.txt"), "Not in the project.\n").unwrap();
    pin("2222222222222222222222222222222222222222", "head");
    let changed = || repo.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
    let configure = |key: &str, value: &str| {
        run(git(&repo.root()).args(["config", key, value]));
    };
    let plain = changed();
    let delta = fs::read(repo.dir.join("delta.jsonl")).unwrap();
    // The two files that are read have to be the same too.
    let same = |what: &str| {
        assert!(changed() == plain, "changed.jsonl, with {what}");
        assert!(
            fs::read(repo.dir.join("delta.jsonl")).unwrap() == delta,
            "delta.jsonl, with {what}"
        );
    };
    // A diff driver for Apex files, for git to take a text conversion
    // from, or the word that they are binary, if it is let: by the
    // repository's attributes and by the user's.
    let user_attributes = repo.dir.join("attributes");
    for attributes in [&repo.root().join(".git/info/attributes"), &user_attributes] {
        fs::write(attributes, "*.cls diff=apex\n").unwrap();
    }
    configure("core.attributesFile", user_attributes.to_str().unwrap());
    same("a diff driver in the attributes");
    for (key, value) in [
        ("diff.renameLimit", "1"),
        ("diff.renames", "false"),
        ("diff.algorithm", "histogram"),
        ("diff.indentHeuristic", "false"),
        ("diff.context", "9"),
        ("diff.interHunkContext", "9"),
        ("diff.noprefix", "true"),
        ("diff.mnemonicPrefix", "true"),
        ("diff.relative", "true"),
        ("diff.ignoreSubmodules", "all"),
        ("core.quotePath", "true"),
        ("color.ui", "always"),
        ("color.diff", "always"),
        ("diff.external", "false"),
        ("diff.apex.textconv", "echo"),
        // What git takes for binary: by size, by a driver, by attributes.
        ("core.bigFileThreshold", "1"),
        ("diff.apex.binary", "true"),
    ] {
        configure(key, value);
        same(&format!("{key} = {value}"));
    }
    // Attributes that are in no commit: of the user, of the repository,
    // and of a working tree.
    fs::write(&user_attributes, "*.cls -diff\n").unwrap();
    same("-diff in the user's attributes");
    fs::write(repo.root().join(".git/info/attributes"), "*.cls -diff\n").unwrap();
    same("-diff in info/attributes");
    fs::write(repo.root().join(".gitattributes"), "*.cls -diff\n").unwrap();
    same("an untracked .gitattributes");
    // `sf-impact-changed` runs no git, and what its caller was told about git
    // is nothing to it. What it does to `sf-impact-delta`, which runs git, is in
    // that tool's own tests.
    let told = repo
        .sf_impact_changed()
        .env("GIT_DIR", repo.dir.join("no-such-repository"))
        .env("GIT_COMMON_DIR", repo.root().join(".git"))
        .env("GIT_OBJECT_DIRECTORY", repo.dir.join("no-such-objects"))
        .env(
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            repo.dir.join("no-such-objects"),
        )
        .env("GIT_CONFIG_PARAMETERS", "'diff.renames=false'")
        .env("PATH", "")
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&told.stdout) == plain,
        "with git variables: {told:?}"
    );

    check_golden(repo.scenario, &plain);
    let rows = rows(&plain);
    assert_eq!(
        fields(&rows, "file", &["file", "status"]),
        [
            json!(["../outside.txt", "added"]),
            json!(["force-app/classes/Caf\u{e9}.cls", "added"]),
            json!(["force-app/classes/Documented.cls", "modified"]),
            json!(["force-app/classes/Steps.cls", "modified"]),
            json!(["force-app/new/New0.cls", "added"]),
            json!(["force-app/new/New1.cls", "added"]),
            json!(["force-app/new/New2.cls", "added"]),
            json!(["force-app/new/New3.cls", "added"]),
            json!(["force-app/new/Wound.cls", "added"]),
            json!(["force-app/old/Old0.cls", "deleted"]),
            json!(["force-app/old/Old1.cls", "deleted"]),
            json!(["force-app/old/Old2.cls", "deleted"]),
            json!(["force-app/old/Old3.cls", "deleted"]),
            json!(["force-app/old/Wound.cls", "deleted"]),
            json!(["force-app/vendor", "modified"]),
        ]
    );
}

#[test]
fn a_permission_set_that_only_moves_touches_nothing() {
    let repo = Repo::new("grant-file-moved");
    let moved = "force-app/permissionsets/billing/Billing_Admin.permissionset-meta.xml";
    repo.rename(PERMISSION_SET, moved);

    let rows = repo.changed();

    assert_eq!(rows.len(), 1);
    assert_eq!(files(&rows), [json!([moved, "moved", "full"])]);
}

#[test]
fn a_deleted_companion_of_no_class_lowers_the_confidence() {
    let repo = Repo::new("companion-orphan-deleted");
    let companion = fs::read_to_string(repo.path(COMPANION)).unwrap();
    repo.write("force-app/classes/Ghost.cls-meta.xml", companion);
    repo.commit("a companion of no class");
    repo.remove("force-app/classes/Ghost.cls-meta.xml");

    let rows = repo.changed();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["gaps"], json!(["no_declaration"]));
}

#[test]
fn a_file_of_a_bundle_that_does_not_parse_cleanly_lowers_the_confidence() {
    let repo = Repo::new("lwc-parse-error");
    let script = "force-app/lwc/invoiceList/invoiceList.js";
    repo.write(script, "import x from \"unterminated\n");

    let rows = repo.changed();

    assert_eq!(
        symbols(&rows),
        [json!(["invoiceList", "modified", "whole_file"])]
    );
    assert_eq!(the_one(&rows, "file")["gaps"], json!(["parse_error"]));
}

#[test]
fn a_file_of_a_bundle_that_did_not_parse_cleanly_at_base_lowers_the_confidence() {
    let repo = Repo::new("lwc-parse-error-at-base");
    let script = "force-app/lwc/invoiceList/invoiceList.js";
    let clean = fs::read_to_string(repo.path(script)).unwrap();
    repo.write(script, "import x from \"unterminated\n");
    repo.commit("a script that does not parse");
    repo.write(script, clean);

    let rows = repo.changed();

    assert_eq!(the_one(&rows, "file")["gaps"], json!(["parse_error"]));
}

// What the two inputs are of.

/// What `sf-impact-changed` says where it refuses the two files that
/// `run_changed` left: one line, and nothing written.
fn refusal(repo: &Repo) -> String {
    let output = repo
        .sf_impact_changed()
        .output()
        .expect("sf-impact-changed runs");
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(output.stdout.is_empty(), "{output:?}");
    let said = String::from_utf8(output.stderr).unwrap();
    assert_eq!(said.lines().count(), 1, "{said}");
    said
}

#[test]
fn refs_of_a_working_tree_with_a_change_that_is_not_committed_are_refused_unless_accepted() {
    let repo = Repo::new("dirty-working-tree");
    repo.replace(INVOICE, "return amount * RATE;", "return 0;");
    repo.commit("head");
    // After the head: a change that no commit has.
    repo.replace(INVOICE, "return 0;", "return 1;");
    let head =
        String::from_utf8(run(git(&repo.root()).args(["rev-parse", "HEAD"])).stdout).unwrap();

    let accepted = repo.run_changed(
        &["--base", "HEAD~1", "--head", "HEAD"],
        &["--accept-unverified-refs"],
    );

    // The same two files, without the word that accepts them.
    let said = refusal(&repo);
    assert!(said.contains("`dirty`"), "{said}");
    assert!(
        said.contains(&format!("re-run sf-impact-extract at {}", head.trim())),
        "{said}"
    );
    assert!(said.contains("--accept-unverified-refs"), "{said}");
    // Accepted, they are mapped, and the first row says what they were.
    check_schema("changed", repo.scenario, &accepted);
    let first = lines(&accepted).remove(0);
    assert_eq!(first["confidence"], "partial");
    assert_eq!(first["gaps"], json!(["unverified_refs"]));
    assert_eq!(first["inputs"]["refs"]["tree"], json!({"state": "dirty"}));
    assert_eq!(first["head"], head.trim());
    assert_eq!(
        symbols(&rows(&accepted)),
        [json!(["Invoice.tax()", "modified", "hunk"])]
    );
}

#[test]
fn refs_of_the_commit_before_the_head_are_refused_and_told_which_commit_to_extract() {
    let repo = Repo::new("stale-refs");
    repo.replace(INVOICE, "return amount * RATE;", "return 0;");
    repo.commit("the commit that the refs are of");
    repo.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
    let stale = fs::read(repo.dir.join("refs.pack")).unwrap();
    repo.replace(INVOICE, "return 0;", "return 1;");
    repo.commit("head");
    repo.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
    // The refs that were kept, with the delta of the commit after them.
    fs::write(repo.dir.join("refs.pack"), stale).unwrap();
    let id = |revision: &str| {
        let said = run(git(&repo.root()).args(["rev-parse", revision])).stdout;
        String::from_utf8(said).unwrap().trim().to_string()
    };

    let said = refusal(&repo);

    assert!(said.contains("`commit`"), "{said}");
    assert!(
        said.contains(&id("HEAD~1")) && said.contains(&id("HEAD")),
        "{said}"
    );
    assert!(
        said.contains(&format!("re-run sf-impact-extract at {}", id("HEAD"))),
        "{said}"
    );
    // Nor with the word that accepts refs that cannot be verified: these can.
    let output = repo
        .sf_impact_changed()
        .arg("--accept-unverified-refs")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
}

#[test]
fn refs_of_a_head_that_was_committed_anew_with_the_same_tree_are_of_the_head() {
    let repo = Repo::new("same-tree");
    repo.replace(INVOICE, "return amount * RATE;", "return 0;");
    repo.commit("head");
    let plain = repo.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
    let kept = fs::read(repo.dir.join("refs.pack")).unwrap();
    // The same content under another commit, as after a commit is amended.
    run(git(&repo.root()).args([
        "commit",
        "--quiet",
        "--amend",
        "--message",
        "head, said otherwise",
    ]));
    repo.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
    fs::write(repo.dir.join("refs.pack"), kept).unwrap();

    let output = run(&mut repo.sf_impact_changed());

    let output = String::from_utf8(output.stdout).unwrap();
    let (first, was) = (lines(&output).remove(0), lines(&plain).remove(0));
    assert_eq!(first["confidence"], "full");
    assert_ne!(first["head"], was["head"]);
    assert_eq!(first["inputs"]["refs"]["tree"]["commit"], was["head"]);
    assert_eq!(rows(&output), rows(&plain));
}

#[test]
fn refs_of_another_project_of_the_same_repository_are_refused() {
    // Two projects in one repository, with the same files at the same
    // paths from each root, and a change to the second.
    let one = Repo::within("two-projects", "one");
    let two = Repo {
        scenario: one.scenario,
        dir: one.dir.clone(),
        project: "two",
    };
    copy(&repository().join("fixtures/changed/base"), &two.path(""));
    two.commit("a second project");
    two.replace(INVOICE, "return amount * RATE;", "return 0;");
    two.commit("head");
    let of_two = two.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
    assert_eq!(
        symbols(&rows(&of_two)),
        [json!(["Invoice.tax()", "modified", "hunk"])]
    );
    // The refs of the first, at the same commit, with the delta of the second.
    let of_one = sf_impact_extract::extract(&one.path(""), 1, false).expect("the head extracts");
    assert!(of_one[0].contains(r#""state":"commit""#), "{}", of_one[0]);
    fs::write(one.dir.join("refs.pack"), pack_of(&one.path(""))).unwrap();

    let said = refusal(&two);

    assert!(said.contains("`root`"), "{said}");
    assert!(
        said.contains("is one in refs.jsonl and two in delta.jsonl"),
        "{said}"
    );
    let output = two
        .sf_impact_changed()
        .arg("--accept-unverified-refs")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
}

#[test]
fn a_parameter_respelling_is_modified() {
    let repo = Repo::new("parameter-respelling");
    let path = "force-app/classes/Account.cls";
    repo.write(
        path,
        "public class Account {\n    public void process(System.List<Id> ids) {}\n}\n",
    );
    repo.commit("qualified parameter");
    repo.replace(path, "System.List<Id>", "Id[]");
    repo.commit("array parameter");
    let output = repo.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
    check_schema("changed", "parameter-respelling", &output);
    assert_eq!(
        symbols(&rows(&output)),
        [json!(["Account.process(list<id>)", "modified", "hunk"])]
    );
}

#[test]
fn an_out_that_is_a_link_is_refused_and_what_it_names_is_left_as_it_was() {
    let repo = Repo::new("out-link");
    repo.write("force-app/classes/Extra.cls", "public class Extra {\n}\n");
    repo.commit("head");
    let rows = repo.run_changed(&["--base", "HEAD~1", "--head", "HEAD"], &[]);
    let victim = repo.dir.join("victim");
    fs::write(&victim, "a file outside\n").unwrap();
    let link = repo.dir.join("changed.jsonl");
    std::os::unix::fs::symlink(&victim, &link).unwrap();

    let output = repo.sf_impact_changed().arg("--out").arg(&link).output();

    let output = output.unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let message = String::from_utf8(output.stderr).unwrap();
    assert!(message.contains("is not a regular file"), "{message}");
    assert_eq!(fs::read_to_string(&victim).unwrap(), "a file outside\n");
    // The control: a path that is no link is written.
    let out = repo.dir.join("written.jsonl");
    let output = repo.sf_impact_changed().arg("--out").arg(&out).output();
    assert!(output.unwrap().status.success());
    assert_eq!(fs::read_to_string(&out).unwrap(), rows);
}
