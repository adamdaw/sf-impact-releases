//! `sf-impact-changed` on a `delta.jsonl` written by hand: no repository and no
//! git. Each test says what differs between two commits in the rows that
//! `sf-impact-delta` would write, extracts the head from a directory of files,
//! and reads what the difference is mapped onto.
//!
//! These are the mapping rules one at a time. `scenarios.rs` runs the same
//! rules through real repositories.

#[path = "../../sf-impact-common/tests/support/pack.rs"]
mod pack;
#[path = "../../sf-impact-common/tests/support/schema.rs"]
mod schema;

use schema::Schema;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::{env, fs};

const PROJECT: &str = r#"{"packageDirectories": [{"path": "app"}, {"path": "plugin"}]}"#;
const INVOICE: &str = "public class Invoice {\n    public Decimal total() {\n        return 1;\n    }\n\n    \
                       public Decimal tax() {\n        return 2;\n    }\n}\n";
const COMPANION: &str =
    "<ApexClass>\n    <apiVersion>62.0</apiVersion>\n    <status>Active</status>\n</ApexClass>\n";
const CLASS: &str = "app/classes/Invoice.cls";

/// The head of every delta that is written here, and its tree.
const HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";
const HEAD_TREE: &str = "dddddddddddddddddddddddddddddddddddddddd";

/// What `sf-impact-delta` would write for two commits, row by row.
#[derive(Default)]
struct Delta {
    files: Vec<Value>,
    /// The id of each text that is a version of a file.
    ids: BTreeMap<String, String>,
}

impl Delta {
    /// The id of the blob with this text: the same text, the same blob.
    fn id(&mut self, text: &str) -> String {
        let next = format!("{:040x}", self.ids.len() + 1);
        self.ids.entry(text.to_string()).or_insert(next).clone()
    }

    fn version(&mut self, path: &str, text: &str) -> Value {
        json!({"file": path, "id": self.id(text), "mode": "100644", "binary": false})
    }

    fn added(&mut self, path: &str, text: &str) -> &mut Value {
        let head = self.version(path, text);
        self.file(json!({"row": "file", "status": "added", "base": null, "head": head}))
    }

    fn deleted(&mut self, path: &str, text: &str) -> &mut Value {
        let base = self.version(path, text);
        self.file(json!({"row": "file", "status": "deleted", "base": base, "head": null}))
    }

    /// A file changed in place, with the hunks between its versions:
    /// `[base_start, base_lines, head_start, head_lines]` each.
    fn modified(&mut self, path: &str, base: &str, head: &str, hunks: &[[u32; 4]]) -> &mut Value {
        let (base, head) = (self.version(path, base), self.version(path, head));
        let hunks: Vec<Value> = hunks.iter().map(|hunk| hunk_of(*hunk)).collect();
        self.file(json!({"row": "file", "status": "modified", "base": base, "head": head, "hunks": hunks}))
    }

    fn moved(&mut self, from: &str, to: &str, text: &str) -> &mut Value {
        let (base, head) = (self.version(from, text), self.version(to, text));
        self.file(json!({"row": "file", "status": "moved", "base": base, "head": head}))
    }

    fn file(&mut self, row: Value) -> &mut Value {
        self.files.push(row);
        self.files.last_mut().unwrap()
    }

    /// The lines of `delta.jsonl`, for two commits with these project files.
    fn lines(mut self, base_project: &str, head_project: &str) -> String {
        let commit = |side: &str, project: String| json!({"row": "commit", "side": side, "id": HEAD, "project_file": project, "config_file": null});
        let mut rows = vec![
            provenance(head_project),
            commit("base", self.id(base_project)),
            commit("head", self.id(head_project)),
        ];
        rows.append(&mut self.files);
        let blobs = self.ids.iter();
        rows.extend(blobs.map(|(text, id)| json!({"row": "blob", "id": id, "text": text})));
        rows.iter().map(|row| format!("{row}\n")).collect()
    }
}

/// The first row of a delta whose head has this project file and no config.
fn provenance(head_project: &str) -> Value {
    let scope = sf_impact_common::provenance::Scope::declared(head_project.as_bytes()).ok();
    let config = sf_impact_common::provenance::config_hash(&Default::default());
    json!({
        "row": "provenance", "tool": "sf-impact-delta", "version": env!("CARGO_PKG_VERSION"), "schema": 1,
        "base": HEAD, "head": HEAD, "head_tree": HEAD_TREE, "root": ".", "scope": scope, "config": config,
    })
}

fn hunk_of([base_start, base_lines, head_start, head_lines]: [u32; 4]) -> Value {
    json!({
        "base_start": base_start, "base_lines": base_lines,
        "head_start": head_start, "head_lines": head_lines,
    })
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn delta_schema() -> Schema {
    let schema = repository().join("schema/delta.schema.json");
    Schema::parse(&fs::read_to_string(schema).expect("the schema exists"))
}

/// The refs pack of a head that has these files and the project file. The
/// files are in no repository and the deltas are of no commit: the first
/// row is made to say that the tree is the head of the deltas written here.
fn refs(name: &str, project: &str, head: &[(&str, &str)]) -> Vec<u8> {
    let root = env::temp_dir().join(format!("sf-impact-mapping-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    for (file, content) in [&[("sfdx-project.json", project)][..], head].concat() {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    let mut lines = sf_impact_extract::extract(&root, 1, false).expect("the head extracts");
    fs::remove_dir_all(&root).unwrap();
    let mut first: Value = serde_json::from_str(&lines[0]).expect("the first line is JSON");
    first["tree"] = json!({"state": "commit", "commit": HEAD, "tree": HEAD_TREE});
    first["root"] = json!(".");
    lines[0] = first.to_string();
    pack::of_jsonl(&(lines.join("\n") + "\n"))
}

/// The rows of `changed.jsonl` for a delta and a head, each checked
/// against the schema: all but the first, which says what the file is of.
fn changed(name: &str, delta: Delta, head: &[(&str, &str)]) -> Vec<Value> {
    changed_between(name, delta, PROJECT, PROJECT, head)
}

fn changed_between(
    name: &str,
    delta: Delta,
    base_project: &str,
    head_project: &str,
    head: &[(&str, &str)],
) -> Vec<Value> {
    let delta = delta.lines(base_project, head_project);
    for line in delta.lines() {
        let row: Value = serde_json::from_str(line).expect("every line is JSON");
        let errors = delta_schema().errors(&row);
        assert!(errors.is_empty(), "{name}: the delta {row}: {errors:?}");
    }
    let refs = refs(name, head_project, head);
    let lines = sf_impact_changed::changed(delta.as_bytes(), refs.as_slice(), false)
        .expect("the delta is mapped");
    let schema = repository().join("schema/changed.schema.json");
    let schema = Schema::parse(&fs::read_to_string(schema).expect("the schema exists"));
    let mut rows: Vec<Value> = lines
        .iter()
        .map(|line| serde_json::from_str(line).expect("every line is JSON"))
        .collect();
    for row in &rows {
        let errors = schema.errors(row);
        assert!(errors.is_empty(), "{name}: {row}: {errors:?}");
    }
    let first = rows.remove(0);
    assert_eq!(
        (&first["row"], &first["confidence"]),
        (&json!("provenance"), &json!("full"))
    );
    rows
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

/// File, status, confidence and gaps of every file row.
fn files(rows: &[Value]) -> Vec<Value> {
    fields(rows, "file", &["file", "status", "confidence", "gaps"])
}

/// Side, kind and name of every touched row.
fn touched(rows: &[Value]) -> Vec<Value> {
    fields(rows, "touched", &["side", "kind", "referenced"])
}

// Lines onto declarations.

#[test]
fn a_hunk_inside_a_method_marks_the_method() {
    let edited = INVOICE.replace("return 2;", "return 3;");
    let mut delta = Delta::default();
    delta.modified(CLASS, INVOICE, &edited, &[[7, 1, 7, 1]]);

    let rows = changed("method", delta, &[(CLASS, &edited)]);

    assert_eq!(
        symbols(&rows),
        [json!(["Invoice.tax()", "modified", "hunk"])]
    );
    assert_eq!(rows[1]["hunks"], json!([hunk_of([7, 1, 7, 1])]));
    assert_eq!(files(&rows), [json!([CLASS, "modified", "full", null])]);
}

#[test]
fn a_line_a_member_shares_with_its_class_marks_the_class_too() {
    let shared = "app/classes/Shared.cls";
    let base = "public class Shared\n extends Base { Integer small() { return 1; }\n Integer second() { return 2; }\n}\n";
    let head = base.replace("extends Base", "extends Other");
    let mut delta = Delta::default();
    delta.modified(shared, base, &head, &[[2, 1, 2, 1]]);

    let rows = changed("shared-line", delta, &[(shared, &head)]);

    assert_eq!(
        symbols(&rows),
        [
            json!(["Shared", "modified", "hunk"]),
            json!(["Shared.small()", "modified", "hunk"]),
        ]
    );
}

#[test]
fn a_line_that_is_gone_marks_what_it_was_a_line_of_at_base() {
    let handler = "app/classes/Handler.cls";
    let base = "public class Handler {\n    @TestVisible\n    private void handle() {\n    }\n}\n";
    let head = base.replace("    @TestVisible\n", "");
    let mut delta = Delta::default();
    delta.modified(handler, base, &head, &[[2, 1, 1, 0]]);

    let rows = changed("removed-line", delta, &[(handler, &head)]);

    // At head only a gap above the method, in the class.
    assert_eq!(
        symbols(&rows),
        [
            json!(["Handler", "modified", "hunk"]),
            json!(["Handler.handle()", "modified", "hunk"]),
        ]
    );
}

// Files that moved. The delta pairs no two files that differ.

#[test]
fn a_symbol_that_goes_on_in_an_added_file_is_compared_with_the_deleted_file_it_was_in() {
    let (moved_to, edited) = (
        "app/billing/Invoice.cls",
        INVOICE.replace("return 2;", "return 3;"),
    );
    let mut delta = Delta::default();
    delta.deleted(CLASS, INVOICE);
    delta.added(moved_to, &edited);

    let rows = changed("moved-and-edited", delta, &[(moved_to, &edited)]);

    // The class is the same class: its package, kind and name. So only
    // what differs between the two files is changed.
    assert_eq!(
        fields(
            &rows,
            "symbol",
            &[
                "symbol",
                "change",
                "reason",
                "file",
                "previous_file",
                "hunks"
            ]
        ),
        [json!([
            "Invoice.tax()",
            "modified",
            "hunk",
            moved_to,
            CLASS,
            [hunk_of([7, 1, 7, 1])]
        ])]
    );
    assert_eq!(
        files(&rows),
        [
            json!([moved_to, "added", "full", null]),
            json!([CLASS, "deleted", "full", null]),
        ]
    );
}

#[test]
fn files_that_declare_other_symbols_are_one_removed_and_one_added() {
    let mut delta = Delta::default();
    delta.deleted("app/classes/Old.cls", "public class Old {\n}\n");
    delta.added("app/classes/New.cls", "public class New {\n}\n");

    let rows = changed(
        "renamed-class",
        delta,
        &[("app/classes/New.cls", "public class New {\n}\n")],
    );

    assert_eq!(
        symbols(&rows),
        [
            json!(["New", "added", "whole_file"]),
            json!(["Old", "removed", "whole_file"]),
        ]
    );
}

#[test]
fn a_symbol_that_could_have_gone_on_in_either_of_two_files_is_removed_and_added() {
    let edited = INVOICE.replace("return 2;", "return 3;");
    let (one, other) = ("app/billing/Invoice.cls", "app/copies/Invoice.cls");
    let mut delta = Delta::default();
    delta.deleted(CLASS, INVOICE);
    delta.added(one, &edited);
    delta.added(other, INVOICE);

    let rows = changed("moved-twice", delta, &[(one, &edited), (other, INVOICE)]);

    // Which of the two the deleted file went on in is not known, and
    // none is chosen: nothing is compared, and nothing is left out.
    assert_eq!(
        fields(
            &rows,
            "symbol",
            &["symbol", "file", "change", "reason", "collision"]
        ),
        [
            json!(["Invoice", one, "added", "whole_file", true]),
            json!(["Invoice.total()", one, "added", "whole_file", true]),
            json!(["Invoice.tax()", one, "added", "whole_file", true]),
            json!(["Invoice", CLASS, "removed", "whole_file", true]),
            json!(["Invoice.total()", CLASS, "removed", "whole_file", true]),
            json!(["Invoice.tax()", CLASS, "removed", "whole_file", true]),
            json!(["Invoice", other, "added", "whole_file", true]),
            json!(["Invoice.total()", other, "added", "whole_file", true]),
            json!(["Invoice.tax()", other, "added", "whole_file", true]),
        ]
    );
}

#[test]
fn what_is_new_in_a_file_that_went_on_was_in_no_file_before() {
    let moved_to = "app/billing/Invoice.cls";
    let discount = "\n    public Decimal discount() {\n        return 0;\n    }\n}\n";
    let grown = format!("{}{discount}", INVOICE.strip_suffix("}\n").unwrap());
    let mut delta = Delta::default();
    delta.deleted(CLASS, INVOICE);
    delta.added(moved_to, &grown);

    let rows = changed("moved-and-grown", delta, &[(moved_to, &grown)]);

    assert_eq!(
        fields(
            &rows,
            "symbol",
            &["symbol", "change", "reason", "previous_file"]
        ),
        [
            // The blank line that sets the method apart.
            json!(["Invoice", "modified", "hunk", CLASS]),
            json!(["Invoice.discount()", "added", "hunk", null]),
        ]
    );
}

#[test]
fn a_reading_that_falls_short_lowers_the_confidence_of_both_halves_of_a_file_that_moved() {
    let moved_to = "app/billing/Invoice.cls";
    let broken = INVOICE.replace("return 2;", "return (2;");
    let mut delta = Delta::default();
    delta.deleted(CLASS, INVOICE);
    delta.added(moved_to, &broken);

    let rows = changed("moved-and-broken", delta, &[(moved_to, &broken)]);

    // What is removed from the one is what is not found in the other.
    assert_eq!(
        files(&rows),
        [
            json!([moved_to, "added", "partial", ["parse_error"]]),
            json!([CLASS, "deleted", "partial", ["parse_error"]]),
        ]
    );
}

#[test]
fn a_file_that_moved_and_changed_without_lines_to_compare_marks_all_it_declares() {
    let (from, to) = ("app/classes/Wide.cls", "app/billing/Wide.cls");
    let (base, head) = (
        "public class Wide {\n    void a() {}\n}\n",
        "public class Wide {\n    void b() {}\n}\n",
    );
    let mut delta = Delta::default();
    delta.deleted(from, base)["base"]["binary"] = json!(true);
    delta.added(to, head)["head"]["binary"] = json!(true);

    let rows = changed("moved-binary", delta, &[(to, head)]);

    assert_eq!(
        fields(&rows, "symbol", &["symbol", "change", "reason", "file"]),
        [
            json!(["Wide", "modified", "whole_file", to]),
            json!(["Wide.b()", "added", "whole_file", to]),
            json!(["Wide.a()", "removed", "removed", from]),
        ]
    );
}

/// A labels file with these labels.
fn labels(names: &[&str]) -> String {
    let label = |name: &&str| {
        format!(
            "    <labels>\n        <fullName>{name}</fullName>\n        <value>{name}</value>\n    </labels>\n"
        )
    };
    let labels: String = names.iter().map(label).collect();
    format!("<CustomLabels>\n{labels}</CustomLabels>\n")
}

#[test]
fn a_file_whose_symbols_went_on_in_two_files_went_on_in_neither() {
    let (whole, one, two) = (
        "app/labels/CustomLabels.labels-meta.xml",
        "app/a/CustomLabels.labels-meta.xml",
        "app/b/CustomLabels.labels-meta.xml",
    );
    let mut delta = Delta::default();
    delta.deleted(whole, &labels(&["One", "Two"]));
    delta.added(one, &labels(&["One"]));
    delta.added(two, &labels(&["Two"]));
    let tree = [(one, labels(&["One"])), (two, labels(&["Two"]))];
    let tree: Vec<(&str, &str)> = tree
        .iter()
        .map(|(path, text)| (*path, text.as_str()))
        .collect();

    let rows = changed("labels-split", delta, &tree);

    assert_eq!(
        fields(&rows, "symbol", &["symbol", "change", "reason", "file"]),
        [
            json!(["Label.One", "added", "whole_file", one]),
            json!(["Label.Two", "added", "whole_file", two]),
            json!(["Label.One", "removed", "whole_file", whole]),
            json!(["Label.Two", "removed", "whole_file", whole]),
        ]
    );
}

#[test]
fn a_file_that_one_of_two_files_went_on_in_is_not_the_file_a_third_went_on_in() {
    let (first, second, third, joined) = (
        "app/x/CustomLabels.labels-meta.xml",
        "app/y/CustomLabels.labels-meta.xml",
        "app/z/CustomLabels.labels-meta.xml",
        "app/labels/CustomLabels.labels-meta.xml",
    );
    let mut delta = Delta::default();
    // `One` was declared twice, and `Two` once, in a file of its own.
    delta.deleted(first, &labels(&["One"]));
    delta.deleted(second, &labels(&["One", "Other"]));
    delta.deleted(third, &labels(&["Two"]));
    delta.added(joined, &labels(&["One", "Two"]));

    let rows = changed(
        "labels-joined",
        delta,
        &[(joined, &labels(&["One", "Two"]))],
    );

    // Which `One` the new file's is, is not known. So it is not known to
    // be the file that had `Two` either, though only one file had `Two`.
    assert_eq!(
        fields(&rows, "symbol", &["symbol", "change", "reason", "file"]),
        [
            json!(["Label.One", "added", "whole_file", joined]),
            json!(["Label.Two", "added", "whole_file", joined]),
            json!(["Label.One", "removed", "whole_file", first]),
            json!(["Label.One", "removed", "whole_file", second]),
            json!(["Label.Other", "removed", "whole_file", second]),
            json!(["Label.Two", "removed", "whole_file", third]),
        ]
    );
}

#[test]
fn a_file_that_moved_as_it_is_within_its_package_changes_no_symbol() {
    let moved_to = "app/billing/Invoice.cls";
    let mut delta = Delta::default();
    delta.moved(CLASS, moved_to, INVOICE);

    let rows = changed("moved", delta, &[(moved_to, INVOICE)]);

    assert_eq!(rows.len(), 1);
    assert_eq!(
        fields(
            &rows,
            "file",
            &["file", "previous_file", "status", "confidence"]
        ),
        [json!([moved_to, CLASS, "moved", "full"])]
    );
}

#[test]
fn a_file_that_moved_to_another_package_is_removed_from_one_and_added_to_the_other() {
    let moved_to = "plugin/classes/Invoice.cls";
    let mut delta = Delta::default();
    delta.moved(CLASS, moved_to, INVOICE);

    let rows = changed("moved-package", delta, &[(moved_to, INVOICE)]);

    assert_eq!(
        fields(
            &rows,
            "symbol",
            &["symbol", "package", "change", "reason", "collision"]
        ),
        [
            json!(["Invoice", "app", "removed", "removed", true]),
            json!(["Invoice.total()", "app", "removed", "removed", true]),
            json!(["Invoice.tax()", "app", "removed", "removed", true]),
            json!(["Invoice", "plugin", "added", "whole_file", false]),
            json!(["Invoice.total()", "plugin", "added", "whole_file", false]),
            json!(["Invoice.tax()", "plugin", "added", "whole_file", false]),
        ]
    );
}

#[test]
fn a_file_that_goes_on_in_another_package_is_another_file() {
    let goes_on_in = "plugin/classes/Invoice.cls";
    let edited = INVOICE.replace("return 2;", "return 3;");
    let mut delta = Delta::default();
    delta.deleted(CLASS, INVOICE);
    delta.added(goes_on_in, &edited);

    let rows = changed("edited-to-package", delta, &[(goes_on_in, &edited)]);

    // A symbol is a symbol of its package: nothing here is modified.
    assert_eq!(
        fields(&rows, "symbol", &["symbol", "package", "change", "reason"]),
        [
            json!(["Invoice", "app", "removed", "whole_file"]),
            json!(["Invoice.total()", "app", "removed", "whole_file"]),
            json!(["Invoice.tax()", "app", "removed", "whole_file"]),
            json!(["Invoice", "plugin", "added", "whole_file"]),
            json!(["Invoice.total()", "plugin", "added", "whole_file"]),
            json!(["Invoice.tax()", "plugin", "added", "whole_file"]),
        ]
    );
}

#[test]
fn a_changed_file_whose_directory_came_into_scope_is_added_whole() {
    let narrower = r#"{"packageDirectories": [{"path": "app"}]}"#;
    let class = "plugin/classes/Invoice.cls";
    let edited = INVOICE.replace("return 2;", "return 3;");
    let mut delta = Delta::default();
    delta.modified("sfdx-project.json", narrower, PROJECT, &[[1, 1, 1, 1]]);
    delta.modified(class, INVOICE, &edited, &[[7, 1, 7, 1]]);

    let rows = changed_between("into-scope", delta, narrower, PROJECT, &[(class, &edited)]);

    // Its base version was no file of the project: there is nothing that
    // the lines of the change are a change to.
    assert_eq!(
        symbols(&rows),
        [
            json!(["Invoice", "added", "whole_file"]),
            json!(["Invoice.total()", "added", "whole_file"]),
            json!(["Invoice.tax()", "added", "whole_file"]),
        ]
    );
}

#[test]
fn a_file_that_moved_into_a_bundle_no_longer_declares_what_it_did_and_marks_the_bundle() {
    let moved_to = "app/lwc/widget/source.txt";
    let mut delta = Delta::default();
    delta.moved(CLASS, moved_to, INVOICE);
    let head = [
        (
            "app/lwc/widget/widget.js",
            "export default class Widget {}\n",
        ),
        (moved_to, INVOICE),
    ];

    let rows = changed("apex-to-bundle", delta, &head);

    assert_eq!(
        fields(&rows, "symbol", &["symbol", "change", "reason", "file"]),
        [
            json!(["Invoice", "removed", "removed", CLASS]),
            json!(["Invoice.total()", "removed", "removed", CLASS]),
            json!(["Invoice.tax()", "removed", "removed", CLASS]),
            json!(["widget", "modified", "whole_file", moved_to]),
        ]
    );
}

// Companions and bundles: files that are a part of what is declared.

const COMPANION_PATH: &str = "app/classes/Invoice.cls-meta.xml";

#[test]
fn a_companion_that_comes_or_goes_marks_the_class_it_is_the_companion_of() {
    let mut added = Delta::default();
    added.added(COMPANION_PATH, COMPANION);
    let mut deleted = Delta::default();
    deleted.deleted(COMPANION_PATH, COMPANION);

    let added = changed(
        "companion-added",
        added,
        &[(CLASS, INVOICE), (COMPANION_PATH, COMPANION)],
    );
    let deleted = changed("companion-deleted", deleted, &[(CLASS, INVOICE)]);

    let invoice = [json!(["Invoice", "modified", "whole_file", COMPANION_PATH])];
    assert_eq!(
        fields(&added, "symbol", &["symbol", "change", "reason", "file"]),
        invoice
    );
    assert_eq!(
        fields(&deleted, "symbol", &["symbol", "change", "reason", "file"]),
        invoice
    );
}

#[test]
fn a_companion_that_moves_with_its_class_and_changes_marks_the_class_by_its_lines() {
    let (class_to, companion_to) = (
        "app/billing/Invoice.cls",
        "app/billing/Invoice.cls-meta.xml",
    );
    let bumped = COMPANION.replace("62.0", "63.0");
    let mut delta = Delta::default();
    delta.moved(CLASS, class_to, INVOICE);
    delta.deleted(COMPANION_PATH, COMPANION);
    delta.added(companion_to, &bumped);

    let rows = changed(
        "companion-moved",
        delta,
        &[(class_to, INVOICE), (companion_to, &bumped)],
    );

    // Not the class removed with the companion it had: the companion is
    // still the companion of `Invoice` in `app`.
    assert_eq!(
        fields(
            &rows,
            "symbol",
            &[
                "symbol",
                "change",
                "reason",
                "file",
                "previous_file",
                "hunks"
            ]
        ),
        [json!([
            "Invoice",
            "modified",
            "whole_file",
            companion_to,
            COMPANION_PATH,
            [hunk_of([2, 1, 2, 1])]
        ])]
    );
}

#[test]
fn a_bundle_is_added_if_its_first_file_comes_with_its_directory() {
    let script = "app/lwc/refund/refund.js";
    let content = "export default class Refund {}\n";
    let mut with_directory = Delta::default();
    with_directory.added(script, content)["new_directory"] = json!("app/lwc/refund");
    let mut into_directory = Delta::default();
    into_directory.added(script, content);

    // A directory of its own inside a bundle that was there.
    let test = "app/lwc/refund/__tests__/refund.test.js";
    let mut with_directory_inside = Delta::default();
    with_directory_inside.added(test, "test('refund', () => {});\n")["new_directory"] =
        json!("app/lwc/refund/__tests__");
    let both = [(script, content), (test, "test('refund', () => {});\n")];

    let added = changed("bundle-added", with_directory, &[(script, content)]);
    let modified = changed("bundle-modified", into_directory, &[(script, content)]);
    let inside = changed("bundle-inside", with_directory_inside, &both);

    assert_eq!(symbols(&added), [json!(["refund", "added", "whole_file"])]);
    assert_eq!(
        symbols(&modified),
        [json!(["refund", "modified", "whole_file"])]
    );
    assert_eq!(
        symbols(&inside),
        [json!(["refund", "modified", "whole_file"])]
    );
}

#[test]
fn a_file_of_a_bundle_that_does_not_parse_cleanly_lowers_the_confidence() {
    let script = "app/lwc/widget/widget.js";
    let broken = "import x from \"unterminated\n";
    let mut delta = Delta::default();
    delta.modified(
        script,
        "export default class Widget {}\n",
        broken,
        &[[1, 1, 1, 1]],
    );

    let rows = changed("bundle-parse-error", delta, &[(script, broken)]);

    assert_eq!(
        symbols(&rows),
        [json!(["widget", "modified", "whole_file"])]
    );
    assert_eq!(
        files(&rows),
        [json!([script, "modified", "partial", ["parse_error"]])]
    );
}

#[test]
fn a_changed_flow_with_an_element_that_was_not_read_lowers_the_confidence() {
    let flow = "app/flows/Invoice.flow-meta.xml";
    let plain = "<Flow>\n<label>Invoice</label>\n</Flow>\n";
    let unread =
        "<Flow>\n<label>Invoice</label>\n<sourceTemplate>ns__T</sourceTemplate>\n</Flow>\n";
    let mut delta = Delta::default();
    delta.modified(flow, plain, unread, &[[2, 0, 3, 1]]);

    let rows = changed("flow-unsupported", delta, &[(flow, unread)]);

    assert_eq!(symbols(&rows), [json!(["Invoice", "modified", "hunk"])]);
    assert_eq!(
        files(&rows),
        [json!([
            flow,
            "modified",
            "partial",
            ["unsupported_element"]
        ])]
    );
}

#[test]
fn a_flow_with_an_element_that_was_not_read_at_base_only_is_read_in_full() {
    // The gap is of what the changed file holds now, as for a parse error
    // in refs.jsonl: the base version declares the same Flow either way.
    let flow = "app/flows/Invoice.flow-meta.xml";
    let unread =
        "<Flow>\n<label>Invoice</label>\n<sourceTemplate>ns__T</sourceTemplate>\n</Flow>\n";
    let plain = "<Flow>\n<label>Invoice</label>\n</Flow>\n";
    let mut delta = Delta::default();
    delta.modified(flow, unread, plain, &[[3, 1, 2, 0]]);

    let rows = changed("flow-unsupported-base", delta, &[(flow, plain)]);

    assert_eq!(files(&rows), [json!([flow, "modified", "full", null])]);
}

// Records: what they are by their own name.

#[test]
fn a_changed_value_of_a_record_marks_its_type_at_both_versions() {
    let record = "app/customMetadata/Plugin.Billing.md-meta.xml";
    let base = "<CustomMetadata>\n    <label>Billing</label>\n    <values>\n        <field>Handler__c</field>\n        \
                <value>InvoiceHandler</value>\n    </values>\n</CustomMetadata>\n";
    let head = base.replace("InvoiceHandler", "PaymentHandler");
    let mut delta = Delta::default();
    delta.modified(record, base, &head, &[[5, 1, 5, 1]]);
    let tree = [
        (record, head.as_str()),
        (
            "app/classes/InvoiceHandler.cls",
            "public class InvoiceHandler {\n}\n",
        ),
        (
            "app/classes/PaymentHandler.cls",
            "public class PaymentHandler {\n}\n",
        ),
        (
            "app/objects/Plugin__mdt/Plugin__mdt.object-meta.xml",
            "<CustomObject>\n</CustomObject>\n",
        ),
    ];

    let rows = changed("record", delta, &tree);

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
}

// Scope, and what the model does not cover.

#[test]
fn a_project_file_that_declares_other_directories_lowers_the_confidence() {
    let wider =
        r#"{"packageDirectories": [{"path": "app"}, {"path": "plugin"}, {"path": "outside"}]}"#;
    let mut delta = Delta::default();
    delta.modified("sfdx-project.json", PROJECT, wider, &[[1, 1, 1, 1]]);

    let rows = changed_between("scope", delta, PROJECT, wider, &[(CLASS, INVOICE)]);

    assert_eq!(
        fields(&rows, "file", &["file", "package", "confidence", "gaps"]),
        [json!([
            "sfdx-project.json",
            null,
            "partial",
            ["scope_changed"]
        ])]
    );
}

#[test]
fn a_submodule_or_a_link_that_is_a_package_directory_is_in_scope_and_is_not_read() {
    let entry = |mode: &str, id: &str| json!({"file": "plugin", "id": id.repeat(40), "mode": mode, "binary": false});
    let mut submodule = Delta::default();
    submodule.file(json!({
        "row": "file", "status": "modified",
        "base": entry("160000", "1"), "head": entry("160000", "2"),
    }));
    let mut link = Delta::default();
    link.file(json!({
        "row": "file", "status": "modified",
        "base": entry("120000", "3"), "head": entry("120000", "4"),
    }));

    let submodule = changed("submodule-root", submodule, &[(CLASS, INVOICE)]);
    let link = changed("link-root", link, &[(CLASS, INVOICE)]);

    let partial = [json!([
        "plugin",
        "plugin",
        "modified",
        "partial",
        ["no_declaration"]
    ])];
    let shown = ["file", "package", "status", "confidence", "gaps"];
    assert_eq!(fields(&submodule, "file", &shown), partial);
    assert_eq!(fields(&link, "file", &shown), partial);
}

#[test]
fn a_file_the_model_does_not_cover_lowers_the_confidence_and_one_out_of_scope_does_not() {
    let layout = "app/layouts/Invoice__c-Invoice Layout.layout-meta.xml";
    let mut delta = Delta::default();
    delta.modified(
        layout,
        "<Layout>\n</Layout>\n",
        "<Layout>\n\n</Layout>\n",
        &[[1, 0, 2, 1]],
    );
    delta.added("docs/notes.md", "Notes.\n");
    delta.added("app/README.md", "Notes.\n");
    // A file that is not read and is no text: nothing of it was to be
    // read, so that it has no text is no reading that fell short.
    let logo = "app/staticresources/logo.png";
    let image =
        |id: &str| json!({"file": logo, "id": id.repeat(40), "mode": "100644", "binary": true});
    delta
        .file(json!({"row": "file", "status": "modified", "base": image("e"), "head": image("f")}));
    let tree = [
        (layout, "<Layout>\n\n</Layout>\n"),
        ("app/README.md", "Notes.\n"),
        (logo, "\0"),
    ];

    let rows = changed("out-of-model", delta, &tree);

    assert_eq!(
        fields(&rows, "file", &["file", "package", "confidence", "gaps"]),
        [
            json!(["app/README.md", "app", "partial", ["no_declaration"]]),
            json!([layout, "app", "partial", ["unmodelled_file"]]),
            json!([logo, "app", "partial", ["no_declaration"]]),
            json!(["docs/notes.md", null, "full", null]),
        ]
    );
    assert_eq!(rows.len(), 4);
}

// Versions that have no lines, or no text.

#[test]
fn a_binary_change_marks_everything_in_the_file_and_is_still_read_for_what_is_gone() {
    let wide = "app/classes/Wide.cls";
    let (base, head) = (
        "public class Wide {\n    void a() {}\n}\n",
        "public class Wide {\n    void b() {}\n}\n",
    );
    // Supplied binary rows still mark the whole file, even with blob text.
    let mut delta = Delta::default();
    let row = delta.modified(wide, base, head, &[]);
    row.as_object_mut().unwrap().remove("hunks");
    row["base"]["binary"] = json!(true);
    row["head"]["binary"] = json!(true);

    let rows = changed("binary", delta, &[(wide, head)]);

    assert_eq!(
        symbols(&rows),
        [
            json!(["Wide", "modified", "whole_file"]),
            json!(["Wide.a()", "removed", "removed"]),
            json!(["Wide.b()", "added", "whole_file"]),
        ]
    );
    assert_eq!(
        fields(&rows, "file", &["binary", "confidence"]),
        [json!([true, "full"])]
    );
}

#[test]
fn a_version_that_is_read_and_has_no_text_lowers_the_confidence() {
    let edited = INVOICE.replace("return 2;", "return 3;");
    let mut delta = Delta::default();
    // The base version is no text in any encoding that is read: its blob
    // has no row.
    let head = delta.version(CLASS, &edited);
    delta.file(json!({
        "row": "file", "status": "modified",
        "base": {"file": CLASS, "id": "f".repeat(40), "mode": "100644", "binary": true},
        "head": head,
    }));

    let rows = changed("no-text", delta, &[(CLASS, &edited)]);

    // What the base declared is not known: nothing is known to be gone,
    // and nothing is taken to be new.
    assert_eq!(
        files(&rows),
        [json!([CLASS, "modified", "partial", ["parse_error"]])]
    );
    assert_eq!(
        symbols(&rows),
        [
            json!(["Invoice", "modified", "whole_file"]),
            json!(["Invoice.total()", "modified", "whole_file"]),
            json!(["Invoice.tax()", "modified", "whole_file"]),
        ]
    );
}

// The two files it reads, and nothing else.

/// A delta that changes `Invoice.tax()`, and the refs pack of its head.
fn tax_changed(name: &str) -> (String, Vec<u8>) {
    let edited = INVOICE.replace("return 2;", "return 3;");
    let mut delta = Delta::default();
    delta.modified(CLASS, INVOICE, &edited, &[[7, 1, 7, 1]]);
    let refs = refs(name, PROJECT, &[(CLASS, &edited)]);
    (delta.lines(PROJECT, PROJECT), refs)
}

#[test]
fn a_pack_with_a_manifest_maps_as_the_same_pack_without_one() {
    let (delta, refs) = tax_changed("manifest");
    let plain = sf_impact_changed::changed(delta.as_bytes(), refs.as_slice(), false).unwrap();
    assert!(
        plain.iter().any(|line| line.contains("Invoice.tax()")),
        "{plain:?}"
    );

    let with_manifest = pack::with_manifest(&refs);
    let mapped = sf_impact_changed::changed(delta.as_bytes(), with_manifest.as_slice(), false);

    assert_eq!(mapped.unwrap(), plain);
    for bad in [
        pack::two_manifests(&with_manifest),
        pack::corrupted(&with_manifest, 6, true),
    ] {
        let refused = sf_impact_changed::changed(delta.as_bytes(), bad.as_slice(), false);
        let refused = refused.unwrap_err();
        assert!(refused.contains("refs pack"), "{refused}");
    }
}

#[test]
fn a_pack_that_is_not_as_it_was_written_is_refused_loudly() {
    let (delta, refs) = tax_changed("tampered");
    assert!(sf_impact_changed::changed(delta.as_bytes(), refs.as_slice(), false).is_ok());

    for (what, bad) in [
        ("truncated", pack::truncated(&refs)),
        ("corrupt section", pack::corrupted(&refs, 3, false)),
        ("corrupt section, resealed", pack::corrupted(&refs, 3, true)),
        ("wrong version", pack::of_version(&refs, 1)),
    ] {
        let refused = sf_impact_changed::changed(delta.as_bytes(), bad.as_slice(), false);
        let refused = refused.expect_err(what);
        assert!(refused.contains("refs pack"), "{what}: {refused}");
    }
}

#[test]
fn a_delta_without_its_commits_or_a_head_without_its_project_file_is_an_error() {
    let refs = refs("errors", PROJECT, &[(CLASS, INVOICE)]);
    let changed =
        |delta: &str| sf_impact_changed::changed(delta.as_bytes(), refs.as_slice(), false);
    let commit = |side: &str, project: Value| json!({"row": "commit", "side": side, "id": HEAD, "project_file": project, "config_file": null});
    let first = provenance(PROJECT);
    let mut without_scope = first.clone();
    without_scope["scope"] = json!(null);

    let no_commits = changed(&format!("{first}\n")).unwrap_err();
    let no_project = changed(&format!(
        "{without_scope}\n{}\n{}\n",
        commit("base", json!(null)),
        commit("head", json!(null))
    ));
    let not_a_row = changed(&format!("{first}\n{{\"row\": \"something else\"}}\n")).unwrap_err();

    assert!(no_commits.contains("commit"), "{no_commits}");
    assert!(no_project.unwrap_err().contains("sfdx-project.json"));
    assert!(not_a_row.starts_with("the delta: line 2: "), "{not_a_row}");
}

#[test]
fn a_file_row_with_no_version_is_refused_by_the_schema_and_by_the_tool_which_writes_no_row() {
    let refs = refs("no-version", PROJECT, &[(CLASS, INVOICE)]);
    let no_version = json!({"row": "file", "status": "modified", "base": null, "head": null});
    let mut delta = Delta::default();
    delta.file(no_version.clone());
    let delta = delta.lines(PROJECT, PROJECT);
    let files = [("delta.jsonl", delta.as_bytes()), ("refs.pack", &refs)];
    let directory = env::temp_dir().join(format!(
        "sf-impact-mapping-{}-no-version",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    for (name, content) in files {
        fs::write(directory.join(name), content).unwrap();
    }

    let output = Command::new(env!("CARGO_BIN_EXE_sf-impact-changed"))
        .arg("--delta")
        .arg(directory.join("delta.jsonl"))
        .arg("--refs")
        .arg(directory.join("refs.pack"))
        .output()
        .unwrap();
    fs::remove_dir_all(&directory).unwrap();

    assert!(!delta_schema().errors(&no_version).is_empty());
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty(), "{output:?}");
    let said = String::from_utf8(output.stderr).unwrap();
    assert!(said.contains("the delta: line 4: "), "{said}");
}

#[test]
fn the_schema_of_the_delta_has_a_file_row_with_the_versions_its_status_has_and_no_other() {
    let mut delta = Delta::default();
    let version = delta.version(CLASS, INVOICE);
    let row = |status: &str, base: &Value, head: &Value| json!({"row": "file", "status": status, "base": base, "head": head});
    let (there, none) = (&version, &Value::Null);

    for (status, base, head, valid) in [
        ("added", none, there, true),
        ("added", there, there, false),
        ("added", none, none, false),
        ("deleted", there, none, true),
        ("deleted", there, there, false),
        ("deleted", none, none, false),
        ("modified", there, there, true),
        ("modified", there, none, false),
        ("modified", none, there, false),
        ("moved", there, there, true),
        ("moved", there, none, false),
        ("moved", none, there, false),
    ] {
        let errors = delta_schema().errors(&row(status, base, head));
        assert_eq!(
            errors.is_empty(),
            valid,
            "{status} {base} {head}: {errors:?}"
        );
    }
}

#[test]
fn the_binary_reads_the_delta_and_the_refs_from_files_or_one_of_them_from_standard_input() {
    let edited = INVOICE.replace("return 2;", "return 3;");
    let mut delta = Delta::default();
    delta.modified(CLASS, INVOICE, &edited, &[[7, 1, 7, 1]]);
    let directory = env::temp_dir().join(format!(
        "sf-impact-mapping-{}-cli-files",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    let (delta_file, refs_file) = (directory.join("delta.jsonl"), directory.join("refs.pack"));
    fs::write(&delta_file, delta.lines(PROJECT, PROJECT)).unwrap();
    fs::write(&refs_file, refs("cli", PROJECT, &[(CLASS, &edited)])).unwrap();
    let sf_impact_changed = || Command::new(env!("CARGO_BIN_EXE_sf-impact-changed"));

    let from_files = sf_impact_changed()
        .arg("--delta")
        .arg(&delta_file)
        .arg("--refs")
        .arg(&refs_file)
        .output()
        .unwrap();
    let from_a_pipe = sf_impact_changed()
        .args(["--delta", "-", "--refs"])
        .arg(&refs_file)
        .stdin(fs::File::open(&delta_file).unwrap())
        .output()
        .unwrap();
    let both_piped = sf_impact_changed()
        .args(["--delta", "-", "--refs", "-"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    // It is told nothing about git, and what git is told does not reach it.
    let with_git_variables = sf_impact_changed()
        .env("GIT_DIR", "/no/such/repository")
        .env("PATH", "")
        .arg("--delta")
        .arg(&delta_file)
        .arg("--refs")
        .arg(&refs_file)
        .output()
        .unwrap();
    fs::remove_dir_all(&directory).unwrap();

    let output = String::from_utf8(from_files.stdout).unwrap();
    assert!(from_files.status.success());
    assert_eq!(output.lines().count(), 3, "{output}");
    assert!(output.contains("\"symbol\":\"Invoice.tax()\""), "{output}");
    assert_eq!(String::from_utf8(from_a_pipe.stdout).unwrap(), output);
    assert_eq!(
        String::from_utf8(with_git_variables.stdout).unwrap(),
        output
    );
    assert_eq!(both_piped.status.code(), Some(2));
    assert!(both_piped.stdout.is_empty());
}

#[test]
fn the_binary_writes_its_lines_whole_to_out_and_a_reader_that_stops_early_is_not_an_error() {
    let edited = INVOICE.replace("return 2;", "return 3;");
    let mut delta = Delta::default();
    delta.modified(CLASS, INVOICE, &edited, &[[7, 1, 7, 1]]);
    let directory =
        env::temp_dir().join(format!("sf-impact-mapping-{}-cli-out", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let (delta_file, refs_file) = (directory.join("delta.jsonl"), directory.join("refs.pack"));
    let out_file = directory.join("changed.jsonl");
    fs::write(&delta_file, delta.lines(PROJECT, PROJECT)).unwrap();
    fs::write(
        &refs_file,
        refs("cli-out-head", PROJECT, &[(CLASS, &edited)]),
    )
    .unwrap();
    let sf_impact_changed = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sf-impact-changed"));
        command
            .arg("--delta")
            .arg(&delta_file)
            .arg("--refs")
            .arg(&refs_file);
        command
    };

    let to_stdout = sf_impact_changed().output().unwrap();
    let to_file = sf_impact_changed()
        .arg("--out")
        .arg(&out_file)
        .output()
        .unwrap();
    let written = fs::read(&out_file).unwrap();
    // The delta comes on stdin after the reader is gone, so not a line can
    // be written before stdout is closed.
    let mut child = Command::new(env!("CARGO_BIN_EXE_sf-impact-changed"))
        .args(["--delta", "-", "--refs"])
        .arg(&refs_file)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let mut stdin = child.stdin.take().unwrap();
    use std::io::Write;
    stdin.write_all(&fs::read(&delta_file).unwrap()).unwrap();
    drop(stdin);
    let stopped_early = child.wait_with_output().unwrap();
    fs::remove_dir_all(&directory).unwrap();

    assert!(to_stdout.status.success(), "{to_stdout:?}");
    assert_eq!(
        String::from_utf8(to_stdout.stdout.clone())
            .unwrap()
            .lines()
            .count(),
        3
    );
    assert!(to_file.status.success(), "{to_file:?}");
    assert!(to_file.stdout.is_empty(), "{to_file:?}");
    assert_eq!(written, to_stdout.stdout);
    assert_eq!(stopped_early.status.code(), Some(0), "{stopped_early:?}");
    assert!(stopped_early.stderr.is_empty(), "{stopped_early:?}");
}

// A valid single textual reference whose column is changed to zero inside
// the compressed section, then reframed with a valid checksum.
fn textual_with_zero_column(refs: &[u8]) -> Vec<u8> {
    let (_, section) = pack::sections(refs)
        .into_iter()
        .find(|(kind, _)| *kind == 5)
        .unwrap();
    let mut at = section.start + 1;
    fn int(bytes: &[u8], at: &mut usize) -> usize {
        let (mut value, mut shift) = (0, 0);
        loop {
            let byte = bytes[*at];
            *at += 1;
            value |= usize::from(byte & 127) << shift;
            if byte & 128 == 0 {
                return value;
            }
            shift += 7;
        }
    }
    fn put(out: &mut Vec<u8>, mut value: usize) {
        while value >= 128 {
            out.push((value as u8) | 128);
            value >>= 7;
        }
        out.push(value as u8);
    }
    let size = int(refs, &mut at);
    let _compressed_size = int(refs, &mut at);
    let mut payload = zstd::bulk::decompress(&refs[at..section.end], size).unwrap();
    let mut field = 0;
    for _ in 0..4 {
        int(&payload, &mut field);
    }
    assert_eq!(payload[field], 1);
    payload[field] = 0;
    let compressed = zstd::bulk::compress(&payload, 3).unwrap();
    let mut result = refs[..section.start].to_vec();
    result.push(5);
    put(&mut result, payload.len());
    put(&mut result, compressed.len());
    result.extend(compressed);
    result.extend_from_slice(&refs[section.end..]);
    pack::resealed(result)
}

fn changed_refusal(name: &str, delta: &str, refs: &[u8]) -> std::process::Output {
    let dir = env::temp_dir().join(format!("sf-impact-refusal-{}-{name}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("delta"), delta).unwrap();
    fs::write(dir.join("refs"), refs).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sf-impact-changed"))
        .args([
            "--delta",
            dir.join("delta").to_str().unwrap(),
            "--refs",
            dir.join("refs").to_str().unwrap(),
        ])
        .output()
        .unwrap();
    fs::remove_dir_all(dir).unwrap();
    output
}

#[test]
fn a_textual_reference_with_a_zero_column_is_refused() {
    let (delta, original) = tax_changed("bad-textual");
    let first: Value =
        serde_json::from_slice(original.split(|b| *b == b'\n').next().unwrap()).unwrap();
    let refs = pack::pack(
        &first,
        &[
            json!({"row":"reference", "tier":"textual", "kind":"text", "referenced":"Invoice", "referrer":null, "package":"app", "file":"app/classes/Invoice.cls", "line":1, "column":1}),
        ],
    );
    let output = changed_refusal("textual", &delta, &textual_with_zero_column(&refs));
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "sf-impact-changed: refs pack: app/classes/Invoice.cls: reference position must be positive\n"
    );
}

#[test]
fn a_truncated_pack_is_refused_at_the_checksum() {
    let (delta, refs) = tax_changed("truncated-command");
    let output = changed_refusal("truncated", &delta, &pack::truncated(&refs));
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "sf-impact-changed: refs pack: the checksum does not match: it is corrupt or cut short\n"
    );
}
