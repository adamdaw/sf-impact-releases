//! What `sf-impact-changed` asks of its two inputs before it maps anything: each
//! starts with a provenance row, and the refs are of the head of the delta.
//! The inputs are written by hand, so each test changes one field of one
//! row. That `sf-impact-extract` and `sf-impact-delta` write rows that pass is shown by
//! the scenarios, on real repositories.

#[path = "../../sf-impact-common/tests/support/pack.rs"]
mod pack;
#[path = "../../sf-impact-common/tests/support/schema.rs"]
mod schema;

use schema::Schema;
use serde_json::{Value, json};
use sf_impact_common::pack::Pack;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::{env, fs};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const BASE: &str = "b000000000000000000000000000000000000000";
const HEAD: &str = "a111111111111111111111111111111111111111";
const HEAD_TREE: &str = "a222222222222222222222222222222222222222";
const OTHER: &str = "c333333333333333333333333333333333333333";
const OTHER_TREE: &str = "c444444444444444444444444444444444444444";
const CONFIG: &str = "0123456789abcdef";

fn scope() -> Value {
    json!({"packageDirectories": ["app"], "unpackagedMetadata": []})
}

/// The first row of the refs that `sf-impact-extract` wrote at the head.
fn refs_header() -> Value {
    json!({
        "row": "provenance", "tool": "sf-impact-extract", "version": VERSION, "schema": 2,
        "tree": {"state": "commit", "commit": HEAD, "tree": HEAD_TREE},
        "root": "packages/billing", "scope": scope(), "config": CONFIG,
    })
}

/// The first row of a delta.jsonl from the base to the head.
fn delta_header() -> Value {
    json!({
        "row": "provenance", "tool": "sf-impact-delta", "version": VERSION, "schema": 1,
        "base": BASE, "head": HEAD, "head_tree": HEAD_TREE,
        "root": "packages/billing", "scope": scope(), "config": CONFIG,
    })
}

const INVOICE: &str =
    "public class Invoice {\n    public Decimal total() {\n        return 1;\n    }\n}\n";

/// The rows of a delta in which one line of one method changed.
fn delta_rows() -> Vec<Value> {
    let project = r#"{"packageDirectories": [{"path": "app"}]}"#;
    let commit = |side: &str, id: &str| json!({"row": "commit", "side": side, "id": id, "project_file": "f".repeat(40), "config_file": null});
    let version = |id: &str| json!({"file": "app/classes/Invoice.cls", "id": id.repeat(40), "mode": "100644", "binary": false});
    vec![
        commit("base", BASE),
        commit("head", HEAD),
        json!({
            "row": "file", "status": "modified", "base": version("1"), "head": version("2"),
            "hunks": [{"base_start": 3, "base_lines": 1, "head_start": 3, "head_lines": 1}],
        }),
        json!({"row": "blob", "id": "1".repeat(40), "text": INVOICE.replace("return 1;", "return 0;")}),
        json!({"row": "blob", "id": "2".repeat(40), "text": INVOICE}),
        json!({"row": "blob", "id": "f".repeat(40), "text": project}),
    ]
}

/// The rows of the refs of that head.
fn refs_rows() -> Vec<Value> {
    let declaration = |symbol: &str, name: &str, kind: &str, start: u32, end: u32| {
        json!({
            "row": "declaration", "symbol": symbol, "name": name, "type": kind, "package": "app",
            "file": "app/classes/Invoice.cls", "start": start, "end": end, "scope": "packageDirectories",
            "is_test": false, "annotations": [], "collision": false,
        })
    };
    vec![
        declaration("Invoice", "Invoice", "class", 1, 5),
        declaration("Invoice.total()", "total", "method", 2, 4),
    ]
}

fn lines(header: Option<&Value>, rows: &[Value]) -> String {
    let rows = header.into_iter().chain(rows);
    rows.map(|row| format!("{row}\n")).collect()
}

/// The refs: the pack of the first row and the rows, or, for a first row
/// that a pack cannot have (of another schema or tool, or none), the lines
/// of refs.jsonl, which the tool reads line 1 of, and refuses, all the same.
fn refs_file(header: Option<&Value>, rows: &[Value]) -> Vec<u8> {
    let text = lines(header, rows);
    let pack = Pack::from_jsonl(&text).and_then(|pack| pack.encode());
    pack.unwrap_or_else(|_| text.into_bytes())
}

/// A directory for the two inputs of one test, removed on drop.
struct Inputs(PathBuf);

impl Inputs {
    fn new(name: &str, delta: &str, refs: &[u8]) -> Inputs {
        let dir = env::temp_dir().join(format!(
            "sf-impact-changed-provenance-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("delta.jsonl"), delta).unwrap();
        fs::write(dir.join("refs.pack"), refs).unwrap();
        Inputs(dir)
    }

    /// The inputs with these two first rows, and the rows that go with them.
    fn with(name: &str, delta_header: &Value, refs_header: &Value) -> Inputs {
        let delta = lines(Some(delta_header), &delta_rows());
        let refs = refs_file(Some(refs_header), &refs_rows());
        Inputs::new(name, &delta, &refs)
    }

    fn run(&self, extra: &[&str]) -> Output {
        let mut changed = Command::new(env!("CARGO_BIN_EXE_sf-impact-changed"));
        changed.arg("--delta").arg(self.0.join("delta.jsonl"));
        changed.arg("--refs").arg(self.0.join("refs.pack"));
        changed
            .args(extra)
            .output()
            .expect("sf-impact-changed runs")
    }

    /// The rows written, each valid against the schema.
    fn rows(&self, extra: &[&str]) -> Vec<Value> {
        let output = self.run(extra);
        assert!(output.status.success(), "{output:?}");
        let schema = repository().join("schema/changed.schema.json");
        let schema = Schema::parse(&fs::read_to_string(schema).expect("the schema exists"));
        let written = String::from_utf8(output.stdout).expect("the output is UTF-8");
        let rows = written
            .lines()
            .map(|line| -> Value { serde_json::from_str(line).expect("a line is JSON") });
        let rows: Vec<Value> = rows.collect();
        for row in &rows {
            assert_eq!(schema.errors(row), Vec::<String>::new(), "{row}");
        }
        rows
    }

    /// What the tool says where it refuses: nothing written, the status of
    /// a failure, and one line.
    fn refusal(&self, extra: &[&str]) -> String {
        let output = self.run(extra);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let said = String::from_utf8(output.stderr).expect("the message is UTF-8");
        assert_eq!(said.lines().count(), 1, "one line: {said}");
        said
    }
}

impl Drop for Inputs {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A row without the field that says what kind of row it is.
fn without_row(row: &Value) -> Value {
    let mut row = row.clone();
    row.as_object_mut().unwrap().remove("row");
    row
}

fn symbols(rows: &[Value]) -> Vec<&str> {
    let symbols = rows.iter().filter(|row| row["row"] == "symbol");
    symbols.map(|row| row["symbol"].as_str().unwrap()).collect()
}

#[test]
fn refs_of_the_head_of_the_delta_are_mapped_and_the_output_says_what_it_was_made_of() {
    let inputs = Inputs::with("matching", &delta_header(), &refs_header());

    let rows = inputs.rows(&[]);

    assert_eq!(
        rows[0],
        json!({
            "row": "provenance", "tool": "sf-impact-changed", "version": VERSION, "schema": 1,
            "base": BASE, "head": HEAD, "head_tree": HEAD_TREE, "root": "packages/billing",
            "scope": scope(), "config": CONFIG, "confidence": "full",
            "inputs": {"refs": without_row(&refs_header()), "delta": without_row(&delta_header())},
        })
    );
    assert_eq!(symbols(&rows), ["Invoice.total()"]);
    let provenance = rows.iter().filter(|row| row["row"] == "provenance");
    assert_eq!(provenance.count(), 1);
}

#[test]
fn refs_of_the_same_tree_under_another_commit_are_of_the_head() {
    // The head rebased, amended or squashed into another commit: the same
    // content, so the same refs.
    let mut refs = refs_header();
    refs["tree"]["commit"] = json!(OTHER);
    let inputs = Inputs::with("same-tree", &delta_header(), &refs);

    let rows = inputs.rows(&[]);

    assert_eq!(rows[0]["confidence"], "full");
    assert_eq!(symbols(&rows), ["Invoice.total()"]);
}

/// One field of the first row of the refs that is not what the delta has.
struct Mismatch {
    name: &'static str,
    /// The field that the refusal names.
    field: &'static str,
    in_refs: Value,
    /// What the refusal shows for each side.
    shown: [&'static str; 2],
}

#[test]
fn refs_that_do_not_describe_the_head_of_the_delta_are_refused_in_one_line() {
    let other_scope =
        json!({"packageDirectories": ["app", "plugins/billing"], "unpackagedMetadata": []});
    let mismatches = [
        Mismatch {
            name: "commit",
            field: "commit",
            in_refs: json!({"tree": {"state": "commit", "commit": OTHER, "tree": OTHER_TREE}}),
            shown: [OTHER, HEAD],
        },
        // One commit has one tree: refs that name the head and another
        // tree are not of the head.
        Mismatch {
            name: "tree-of-the-same-commit",
            field: "tree",
            in_refs: json!({"tree": {"state": "commit", "commit": HEAD, "tree": OTHER_TREE}}),
            shown: [OTHER_TREE, HEAD_TREE],
        },
        // Another project of the same repository, at the same commit.
        Mismatch {
            name: "root",
            field: "root",
            in_refs: json!({"root": "packages/shipping"}),
            shown: ["packages/shipping", "packages/billing"],
        },
        Mismatch {
            name: "scope",
            field: "scope",
            in_refs: json!({"scope": other_scope}),
            shown: [r#"["app","plugins/billing"]"#, r#"["app"]"#],
        },
        Mismatch {
            name: "config",
            field: "config",
            in_refs: json!({"config": "fedcba9876543210"}),
            shown: ["fedcba9876543210", CONFIG],
        },
        Mismatch {
            name: "schema",
            field: "schema",
            in_refs: json!({"schema": 1}),
            shown: ["1", "2"],
        },
    ];

    for mismatch in mismatches {
        let mut refs = refs_header();
        for (field, value) in mismatch.in_refs.as_object().unwrap() {
            refs[field] = value.clone();
        }
        let inputs = Inputs::with(mismatch.name, &delta_header(), &refs);

        // Not with the word that accepts refs that cannot be verified,
        // either: these were, and are of something else.
        for extra in [&[][..], &["--accept-unverified-refs"]] {
            let said = inputs.refusal(extra);

            let name = mismatch.name;
            assert!(
                said.starts_with("sf-impact-changed: refs.jsonl"),
                "{name}: {said}"
            );
            assert!(
                said.contains(&format!("`{}`", mismatch.field)),
                "{name}: {said}"
            );
            for value in mismatch.shown {
                assert!(said.contains(value), "{name}: {value} in {said}");
            }
            let fix = format!("re-run sf-impact-extract at {HEAD}");
            assert!(said.contains(&fix), "{name}: {said}");
        }
    }
}

#[test]
fn a_delta_of_another_schema_version_is_refused_in_one_line() {
    let mut delta = delta_header();
    delta["schema"] = json!(2);
    let inputs = Inputs::with("schema", &delta, &refs_header());

    let said = inputs.refusal(&[]);

    assert!(said.starts_with("sf-impact-changed: delta.jsonl"), "{said}");
    assert!(said.contains("`schema`") && said.contains('2'), "{said}");
    assert!(said.contains("re-run sf-impact-delta"), "{said}");
}

#[test]
fn refs_of_a_minor_version_that_is_not_this_one_are_read() {
    let (major, _) = VERSION.split_once('.').unwrap();
    let mut refs = refs_header();
    refs["version"] = json!(format!("{major}.999.0"));
    let inputs = Inputs::with("minor", &delta_header(), &refs);

    assert_eq!(symbols(&inputs.rows(&[])), ["Invoice.total()"]);
}

#[test]
fn refs_of_a_tree_that_is_no_commit_are_refused_unless_told_to_accept_them() {
    for state in ["dirty", "unknown"] {
        let mut refs = refs_header();
        refs["tree"] = json!({"state": state});
        if state == "unknown" {
            refs["root"] = json!(null);
        }
        let inputs = Inputs::with(state, &delta_header(), &refs);

        let said = inputs.refusal(&[]);

        assert!(said.starts_with("sf-impact-changed: refs.jsonl"), "{said}");
        assert!(said.contains(&format!("`{state}`")), "{said}");
        assert!(said.contains(HEAD), "{said}");
        assert!(
            said.contains(&format!("re-run sf-impact-extract at {HEAD}")),
            "{said}"
        );
        assert!(said.contains("--accept-unverified-refs"), "{said}");

        // Told to, it maps them, and the output says that it could not
        // verify what it was given.
        let rows = inputs.rows(&["--accept-unverified-refs"]);

        assert_eq!(rows[0]["confidence"], "partial");
        assert_eq!(rows[0]["gaps"], json!(["unverified_refs"]));
        assert_eq!(rows[0]["inputs"]["refs"]["tree"], json!({"state": state}));
        assert_eq!(rows[0]["inputs"]["refs"]["root"], refs["root"]);
        assert_eq!(symbols(&rows), ["Invoice.total()"]);
    }
}

#[test]
fn refs_that_are_verified_are_not_made_less_by_the_word_that_accepts_others() {
    let inputs = Inputs::with("accepting", &delta_header(), &refs_header());

    assert_eq!(inputs.rows(&["--accept-unverified-refs"]), inputs.rows(&[]));
}

#[test]
fn an_input_without_its_provenance_row_is_refused_and_told_which_version_has_one() {
    let (delta, refs) = (
        lines(Some(&delta_header()), &delta_rows()),
        refs_file(Some(&refs_header()), &refs_rows()),
    );
    let (old_delta, old_refs) = (lines(None, &delta_rows()), refs_file(None, &refs_rows()));

    for (name, delta, refs, file, tool, version) in [
        (
            "old-refs",
            &*delta,
            &*old_refs,
            "refs.jsonl",
            "sf-impact-extract",
            2,
        ),
        (
            "old-delta",
            &*old_delta,
            &*refs,
            "delta.jsonl",
            "sf-impact-delta",
            1,
        ),
        (
            "empty-refs",
            &*delta,
            &[][..],
            "refs.jsonl",
            "sf-impact-extract",
            2,
        ),
        (
            "empty-delta",
            "",
            &*refs,
            "delta.jsonl",
            "sf-impact-delta",
            1,
        ),
    ] {
        let said = Inputs::new(name, delta, refs).refusal(&[]);

        assert!(
            said.starts_with(&format!("sf-impact-changed: {file}")),
            "{name}: {said}"
        );
        assert!(said.contains("provenance"), "{name}: {said}");
        let version = format!("schema version {version}");
        assert!(said.contains(&version), "{name}: {said}");
        assert!(said.contains(&format!("re-run {tool}")), "{name}: {said}");
    }
}

#[test]
fn the_inputs_are_verified_before_anything_else_is_read_from_them() {
    // A delta that cannot be read past its first row, and refs of another
    // commit: what is said is that the two do not go together.
    let delta = lines(Some(&delta_header()), &[]) + "not a row\n";
    let mut refs = refs_header();
    refs["tree"] = json!({"state": "commit", "commit": OTHER, "tree": OTHER_TREE});
    let inputs = Inputs::new("first", &delta, &refs_file(Some(&refs), &refs_rows()));

    let said = inputs.refusal(&[]);

    assert!(said.contains(OTHER) && said.contains(HEAD), "{said}");
    assert!(!said.contains("line 2"), "{said}");
}

#[test]
fn the_same_two_files_give_the_same_bytes_wherever_they_are() {
    let here = Inputs::with("here", &delta_header(), &refs_header());
    let there = Inputs::with("there", &delta_header(), &refs_header());

    let first = here.run(&[]).stdout;

    assert!(String::from_utf8_lossy(&first).starts_with(r#"{"row":"provenance","#));
    assert_eq!(here.run(&[]).stdout, first);
    assert_eq!(there.run(&[]).stdout, first);
}

/// A first row with the value at `pointer` replaced: `/tree/commit`.
fn with(mut row: Value, pointer: &str, value: Value) -> Value {
    match row.pointer_mut(pointer) {
        Some(at) => *at = value,
        None => {
            let (parent, name) = pointer.rsplit_once('/').unwrap();
            let parent = if parent.is_empty() {
                &mut row
            } else {
                row.pointer_mut(parent).expect("the parent is there")
            };
            parent[name] = value;
        }
    }
    row
}

#[test]
fn unknown_provenance_keys_name_only_their_actual_path() {
    for (pointer, path) in [
        ("/scope/tree", "scope.tree"),
        ("/tree/schema", "tree.schema"),
    ] {
        let refs = with(refs_header(), pointer, json!(1));
        let inputs = Inputs::with("unknown-provenance-path", &delta_header(), &refs);
        let said = inputs.refusal(&[]);
        assert_eq!(
            said.trim(),
            format!(
                "sf-impact-changed: refs.jsonl: its provenance row is not valid: `{path}` is not what schema version 2 has there; re-run sf-impact-extract at {HEAD}"
            )
        );
    }
}

#[test]
fn refs_whose_first_row_is_not_what_the_schema_says_are_refused_and_nothing_of_it_is_written() {
    let (hex, not_hex) = ("f".repeat(40), "z".repeat(40));
    // The value that is not valid, where it is, and the field that the
    // refusal names.
    let malformed = [
        ("/tool", json!(""), "tool"),
        ("/version", json!(""), "version"),
        ("/version", json!("0.not-semver"), "version"),
        ("/version", json!("1"), "version"),
        ("/tree/commit", json!(""), "tree.commit"),
        ("/tree/tree", json!(""), "tree.tree"),
        ("/tree/commit", json!(not_hex), "tree.commit"),
        ("/tree/tree", json!(&hex[..39]), "tree.tree"),
        // A state that is none of the four: the refusal names it.
        ("/tree/state", json!("clean"), "clean"),
        ("/tree/extra", json!(1), "tree.extra"),
        (
            "/tree",
            json!({"state": "dirty", "commit": HEAD}),
            "tree.commit",
        ),
        ("/root", json!(""), "root"),
        ("/root", json!("/home/someone/project"), "root"),
        ("/root", json!("packages/../billing"), "root"),
        ("/root", json!("packages\\billing"), "root"),
        ("/root", json!("packages/billing/"), "root"),
        ("/root", json!(null), "root"),
        ("/scope/packageDirectories", json!(["b", "a"]), "scope"),
        ("/scope/packageDirectories", json!(["a", "a"]), "scope"),
        ("/scope/packageDirectories", json!(["../a"]), "scope"),
        ("/scope/unpackagedMetadata", json!(["app"]), "scope"),
        ("/scope/more", json!([]), "scope.more"),
        ("/config", json!(""), "config"),
        ("/config", json!("0123456789abcdeg"), "config"),
        ("/config", json!("0123456789abcde"), "config"),
        ("/unexpected", json!("x"), "unexpected"),
    ];

    for (pointer, value, field) in malformed {
        let refs = with(refs_header(), pointer, value.clone());
        let inputs = Inputs::with("malformed-refs", &delta_header(), &refs);

        for extra in [&[][..], &["--accept-unverified-refs"]] {
            let said = inputs.refusal(extra);

            let case = format!("{pointer} = {value}: {said}");
            assert!(said.starts_with("sf-impact-changed: refs.jsonl"), "{case}");
            assert!(said.contains(&format!("`{field}`")), "{case}");
            if pointer != "/tree/state" {
                assert!(said.contains("re-run sf-impact-extract"), "{case}");
            }
        }
    }
}

#[test]
fn a_delta_whose_first_row_is_not_what_the_schema_says_is_refused() {
    let malformed = [
        ("/tool", json!(""), "tool"),
        ("/version", json!("0.x.1"), "version"),
        ("/base", json!(""), "base"),
        ("/head", json!("HEAD"), "head"),
        ("/head_tree", json!(""), "head_tree"),
        ("/root", json!("/home/someone/project"), "root"),
        ("/scope/packageDirectories", json!(["b", "a"]), "scope"),
        ("/config", json!("no hash"), "config"),
        ("/unexpected", json!("x"), "unexpected"),
    ];

    for (pointer, value, field) in malformed {
        let delta = with(delta_header(), pointer, value.clone());
        let inputs = Inputs::with("malformed-delta", &delta, &refs_header());

        let said = inputs.refusal(&[]);

        let case = format!("{pointer} = {value}: {said}");
        assert!(said.starts_with("sf-impact-changed: delta.jsonl"), "{case}");
        assert!(said.contains(&format!("`{field}`")), "{case}");
    }
    // A delta is always of a place in a repository: one that is of none
    // is not read as a delta at all.
    let nowhere = with(delta_header(), "/root", json!(null));
    let said = Inputs::with("delta-nowhere", &nowhere, &refs_header()).refusal(&[]);
    assert!(
        said.starts_with("sf-impact-changed: delta.jsonl: line 1: "),
        "{said}"
    );
}

#[test]
fn refs_of_a_tree_that_is_no_commit_and_of_another_project_are_refused_though_accepted() {
    // What is known of them is held to the delta, with the word or without.
    for state in ["dirty", "unknown"] {
        let mut refs = refs_header();
        refs["tree"] = json!({"state": state});
        // Of the same project, they are accepted: a tree that is unknown
        // may still be known to be at a place.
        let same = Inputs::with("same-root", &delta_header(), &refs);
        assert_eq!(
            same.rows(&["--accept-unverified-refs"])[0]["confidence"],
            "partial"
        );
        refs["root"] = json!("packages/shipping");
        let inputs = Inputs::with("other-root", &delta_header(), &refs);

        let said = inputs.refusal(&["--accept-unverified-refs"]);

        assert!(
            said.contains("`root`") && said.contains("packages/shipping"),
            "{said}"
        );
    }
}

#[test]
fn a_row_of_another_schema_version_is_refused_for_its_version_whatever_else_it_has() {
    // A later version need not have the fields of this one: what is said
    // is that it is another version, not that a field is missing.
    let later = json!({"row": "provenance", "tool": "sf-impact-extract", "version": VERSION, "schema": 3, "of": {}});
    let inputs = Inputs::with("later-schema", &delta_header(), &later);

    let said = inputs.refusal(&[]);

    assert!(said.starts_with("sf-impact-changed: refs.jsonl"), "{said}");
    assert!(said.contains("`schema` is version 3"), "{said}");
    assert!(said.contains("reads version 2"), "{said}");
    assert!(
        said.contains(&format!("re-run sf-impact-extract at {HEAD}")),
        "{said}"
    );
}

/// A first row without the field at `pointer`: `/tree/state`.
fn without(mut row: Value, pointer: &str) -> Value {
    let (parent, name) = pointer.rsplit_once('/').unwrap();
    let parent = if parent.is_empty() {
        &mut row
    } else {
        row.pointer_mut(parent).expect("the parent is there")
    };
    let removed = parent.as_object_mut().unwrap().remove(name);
    assert!(removed.is_some(), "{pointer} was there");
    row
}

#[test]
fn refs_without_a_field_that_the_schema_requires_are_refused_though_it_may_be_null() {
    // A tree that is unknown may be at no place that is known: `root` is
    // then null. It is still there to say so.
    let unknown = with(
        with(refs_header(), "/tree", json!({"state": "unknown"})),
        "/root",
        json!(null),
    );
    let accepted = Inputs::with("unknown-null-root", &delta_header(), &unknown);
    assert_eq!(
        accepted.rows(&["--accept-unverified-refs"])[0]["confidence"],
        "partial"
    );
    let omitted = Inputs::with(
        "unknown-no-root",
        &delta_header(),
        &without(unknown, "/root"),
    );
    for extra in [&[][..], &["--accept-unverified-refs"]] {
        let said = omitted.refusal(extra);
        assert!(said.starts_with("sf-impact-changed: refs.jsonl"), "{said}");
        assert!(
            said.contains("`root`") && said.contains("missing"),
            "{said}"
        );
    }

    // So it is for every field that the schema requires.
    for (pointer, field) in [
        ("/tool", "tool"),
        ("/version", "version"),
        ("/schema", "schema"),
        ("/tree", "tree"),
        ("/tree/state", "tree.state"),
        ("/tree/commit", "tree.commit"),
        ("/tree/tree", "tree.tree"),
        ("/root", "root"),
        ("/scope", "scope"),
        ("/scope/packageDirectories", "scope.packageDirectories"),
        ("/scope/unpackagedMetadata", "scope.unpackagedMetadata"),
        ("/config", "config"),
    ] {
        let refs = without(refs_header(), pointer);
        let said = Inputs::with("refs-without", &delta_header(), &refs)
            .refusal(&["--accept-unverified-refs"]);

        assert!(
            said.starts_with("sf-impact-changed: refs.jsonl"),
            "{pointer}: {said}"
        );
        assert!(
            said.contains(&format!("`{field}` is missing")),
            "{pointer}: {said}"
        );
    }
}

#[test]
fn a_delta_without_a_field_that_the_schema_requires_is_refused_though_it_may_be_null() {
    // The scope and the config of a head may be null, where it has none
    // that can be read. That is said by the delta, and refused here for
    // what it is; a delta that does not say is no delta.
    let no_scope = with(delta_header(), "/scope", json!(null));
    let said = Inputs::with("delta-null-scope", &no_scope, &refs_header()).refusal(&[]);
    assert!(
        said.contains("sfdx-project.json") && !said.contains("missing"),
        "{said}"
    );

    for (pointer, field) in [
        ("/tool", "tool"),
        ("/version", "version"),
        ("/schema", "schema"),
        ("/base", "base"),
        ("/head", "head"),
        ("/head_tree", "head_tree"),
        ("/root", "root"),
        ("/scope", "scope"),
        ("/scope/packageDirectories", "scope.packageDirectories"),
        ("/config", "config"),
    ] {
        let delta = without(delta_header(), pointer);
        let said = Inputs::with("delta-without", &delta, &refs_header()).refusal(&[]);

        assert!(
            said.starts_with("sf-impact-changed: delta.jsonl"),
            "{pointer}: {said}"
        );
        assert!(
            said.contains(&format!("`{field}` is missing")),
            "{pointer}: {said}"
        );
    }
}

#[test]
fn a_delta_that_another_tool_wrote_to_the_same_schema_is_read() {
    // Any stage can be replaced: a row has to name what wrote it, and
    // need not name these tools. All else is held to as it is of theirs.
    let delta = with(delta_header(), "/tool", json!("another-delta"));
    let inputs = Inputs::with("other-producers", &delta, &refs_header());

    let rows = inputs.rows(&[]);

    assert_eq!(rows[0]["confidence"], "full");
    assert_eq!(rows[0]["inputs"]["delta"]["tool"], "another-delta");
    assert_eq!(symbols(&rows), ["Invoice.total()"]);
}

#[test]
fn a_refs_pack_is_of_sf_impact_extract_and_one_of_another_tool_is_refused() {
    // As of schema 2, refs are a pack, and a pack's line 1 is the
    // provenance row of sf-impact-extract (docs/pack.md). One that names
    // another tool is refused, whatever else it holds.
    let refs = refs_file(Some(&refs_header()), &refs_rows());
    let other = with(refs_header(), "/tool", json!("sf-impact-other")).to_string();
    let line_1 = refs.iter().position(|b| *b == b'\n').unwrap();
    let mut of_other = other.into_bytes();
    of_other.extend_from_slice(&refs[line_1..]);
    let inputs = Inputs::new(
        "other-producer",
        &lines(Some(&delta_header()), &delta_rows()),
        &pack::resealed(of_other),
    );

    let said = inputs.refusal(&[]);

    assert!(said.contains("refs pack"), "{said}");
    assert!(said.contains("sf-impact-extract"), "{said}");
}
