//! What `sf-impact-tests` asks of its inputs before it walks anything: each
//! starts with a provenance row, and the refs are of the head that
//! changed.jsonl was made for. The inputs are written by hand, so each
//! test changes one field of one row.
#[path = "../../sf-impact-common/tests/support/pack.rs"]
mod pack;
#[path = "../../sf-impact-common/tests/support/schema.rs"]
mod schema;
mod support;

use serde_json::{Value, json};
use sf_impact_common::pack::Pack;
use std::{
    env, fs,
    path::PathBuf,
    process::{Command, Output},
};
use support::{CONFIG, HEAD, HEAD_TREE, changed_header, refs_header};

const OTHER: &str = "c333333333333333333333333333333333333333";
const OTHER_TREE: &str = "c444444444444444444444444444444444444444";

fn refs_rows() -> Vec<Value> {
    let declaration = |symbol: &str, is_test: bool| {
        json!({"row":"declaration","symbol":symbol,"name":symbol,"type":"class","package":"app",
            "file":format!("app/{symbol}.cls"),"start":1,"end":9,"is_test":is_test,
            "annotations":[],"collision":false,"scope":"packageDirectories"})
    };
    vec![
        declaration("Invoice", false),
        declaration("InvoiceTest", true),
    ]
}
fn changed_rows() -> Vec<Value> {
    vec![
        json!({"row":"file","file":"app/Invoice.cls","status":"modified","package":"app","confidence":"full"}),
        json!({"row":"symbol","symbol":"Invoice","type":"class","package":"app","file":"app/Invoice.cls",
            "change":"modified","reason":"whole_file","collision":false}),
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

/// A directory for the inputs of one test, removed on drop.
struct Inputs(PathBuf);
impl Inputs {
    fn new(name: &str, refs: &[u8], changed: &str) -> Inputs {
        let dir = env::temp_dir().join(format!(
            "sf-impact-tests-provenance-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("refs.pack"), refs).unwrap();
        fs::write(dir.join("changed.jsonl"), changed).unwrap();
        Inputs(dir)
    }
    fn with(name: &str, refs_header: &Value, changed_header: &Value) -> Inputs {
        Inputs::new(
            name,
            &refs_file(Some(refs_header), &refs_rows()),
            &lines(Some(changed_header), &changed_rows()),
        )
    }
    fn run(&self, seeds: &[&str], extra: &[&str]) -> Output {
        let mut impact = Command::new(env!("CARGO_BIN_EXE_sf-impact-tests"));
        impact.arg("--refs").arg(self.0.join("refs.pack"));
        if seeds.is_empty() {
            impact.arg("--changed").arg(self.0.join("changed.jsonl"));
        } else {
            impact.arg("--symbol").args(seeds);
        }
        impact.args(extra).output().expect("sf-impact-tests runs")
    }
    /// The document written for changed.jsonl, valid against the schema.
    fn document(&self, extra: &[&str]) -> Value {
        document(self.run(&[], extra))
    }
    /// What the tool says where it refuses: nothing written, the status of
    /// a failure, and one line.
    fn refusal(&self, seeds: &[&str], extra: &[&str]) -> String {
        let output = self.run(seeds, extra);
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let said = String::from_utf8(output.stderr).unwrap();
        assert_eq!(said.lines().count(), 1, "one line: {said}");
        said
    }
}
impl Drop for Inputs {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn document(output: Output) -> Value {
    assert!(output.status.success(), "{output:?}");
    let document: Value = serde_json::from_slice(&output.stdout).unwrap();
    let schema = schema::Schema::parse(include_str!("../../../schema/impact.schema.json"));
    assert_eq!(schema.errors(&document), Vec::<String>::new());
    document
}
fn tests(document: &Value) -> Vec<&str> {
    let tests = document["tests"].as_array().unwrap().iter();
    tests.map(|test| test["symbol"].as_str().unwrap()).collect()
}
fn unverified(document: &Value) -> Vec<&Value> {
    let reasons = document["gaps"].as_array().unwrap().iter();
    reasons
        .filter(|reason| reason["reason"] == "unverified_refs")
        .collect()
}

#[test]
fn refs_of_the_head_that_changed_was_made_for_are_walked_at_full_confidence() {
    let inputs = Inputs::with("matching", &refs_header(), &changed_header());

    let document = inputs.document(&[]);

    assert_eq!(tests(&document), ["InvoiceTest"]);
    assert_eq!(document["confidence"], "full");
    assert_eq!(document["gaps"], json!([]));
}

#[test]
fn refs_of_the_same_tree_under_another_commit_are_of_the_head() {
    let mut refs = refs_header();
    refs["tree"]["commit"] = json!(OTHER);
    let inputs = Inputs::with("same-tree", &refs, &changed_header());

    assert_eq!(inputs.document(&[])["confidence"], "full");
}

#[test]
fn refs_that_do_not_describe_the_head_of_changed_are_refused_in_one_line() {
    let other_scope =
        json!({"packageDirectories": ["app", "plugins/billing"], "unpackagedMetadata": []});
    // The field, what the refs have for it, and what the refusal shows of
    // each side.
    let mismatches = [
        (
            "commit",
            json!({"tree": {"state": "commit", "commit": OTHER, "tree": OTHER_TREE}}),
            [OTHER, HEAD],
        ),
        // One commit has one tree.
        (
            "tree",
            json!({"tree": {"state": "commit", "commit": HEAD, "tree": OTHER_TREE}}),
            [OTHER_TREE, HEAD_TREE],
        ),
        // Another project of the same repository, at the same commit.
        (
            "root",
            json!({"root": "packages/shipping"}),
            ["packages/shipping", "packages/billing"],
        ),
        (
            "scope",
            json!({"scope": other_scope}),
            [r#"["app","plugins/billing"]"#, r#"["app"]"#],
        ),
        (
            "config",
            json!({"config": "fedcba9876543210"}),
            ["fedcba9876543210", CONFIG],
        ),
        ("schema", json!({"schema": 1}), ["1", "2"]),
    ];

    for (field, in_refs, shown) in mismatches {
        let mut refs = refs_header();
        for (name, value) in in_refs.as_object().unwrap() {
            refs[name] = value.clone();
        }
        let inputs = Inputs::with(field, &refs, &changed_header());

        for extra in [&[][..], &["--accept-unverified-refs"]] {
            let said = inputs.refusal(&[], extra);

            assert!(said.starts_with("sf-impact-tests: refs.jsonl"), "{said}");
            assert!(said.contains(&format!("`{field}`")), "{field}: {said}");
            for value in shown {
                assert!(said.contains(value), "{field}: {value} in {said}");
            }
            let fix = format!("re-run sf-impact-extract at {HEAD}");
            assert!(said.contains(&fix), "{field}: {said}");
        }
    }
}

#[test]
fn a_changed_of_another_schema_version_is_refused_in_one_line() {
    let mut changed = changed_header();
    changed["schema"] = json!(2);
    let inputs = Inputs::with("schema", &refs_header(), &changed);

    let said = inputs.refusal(&[], &[]);

    assert!(said.starts_with("sf-impact-tests: changed.jsonl"), "{said}");
    assert!(said.contains("`schema`") && said.contains('2'), "{said}");
    assert!(said.contains("re-run sf-impact-changed"), "{said}");
}

#[test]
fn refs_of_a_tree_that_is_no_commit_are_refused_unless_told_to_accept_them() {
    for state in ["dirty", "unknown"] {
        let mut refs = refs_header();
        refs["tree"] = json!({"state": state});
        if state == "unknown" {
            refs["root"] = json!(null);
        }
        let inputs = Inputs::with(state, &refs, &changed_header());

        let said = inputs.refusal(&[], &[]);

        assert!(said.starts_with("sf-impact-tests: refs.jsonl"), "{said}");
        assert!(said.contains(&format!("`{state}`")), "{said}");
        assert!(
            said.contains(&format!("re-run sf-impact-extract at {HEAD}")),
            "{said}"
        );
        assert!(said.contains("--accept-unverified-refs"), "{said}");

        // Told to, it walks them, and the document says why it is not to
        // be relied on in full.
        let document = inputs.document(&["--accept-unverified-refs"]);

        assert_eq!(tests(&document), ["InvoiceTest"]);
        assert_eq!(document["confidence"], "partial");
        assert_eq!(
            unverified(&document),
            [&json!({"reason":"unverified_refs","file":"refs.jsonl","line":1,"source":"refs"})]
        );
    }
}

#[test]
fn a_changed_that_was_made_of_unverified_refs_lowers_the_confidence_of_what_reads_it() {
    // Verified refs now, but changed.jsonl was mapped onto refs that were not.
    let mut changed = changed_header();
    changed["confidence"] = json!("partial");
    changed["gaps"] = json!(["unverified_refs"]);
    changed["inputs"]["refs"]["tree"] = json!({"state": "dirty"});
    let inputs = Inputs::with("made-of-unverified", &refs_header(), &changed);

    let document = inputs.document(&[]);

    assert_eq!(document["confidence"], "partial");
    assert_eq!(
        unverified(&document),
        [&json!({"reason":"unverified_refs","file":"changed.jsonl","line":1,"source":"changed"})]
    );
}

#[test]
fn an_input_without_its_provenance_row_is_refused_and_told_which_version_has_one() {
    let (refs, changed) = (
        refs_file(Some(&refs_header()), &refs_rows()),
        lines(Some(&changed_header()), &changed_rows()),
    );
    let (old_refs, old_changed) = (refs_file(None, &refs_rows()), lines(None, &changed_rows()));

    for (name, refs, changed, file, tool, version) in [
        (
            "old-refs",
            &*old_refs,
            &*changed,
            "refs.jsonl",
            "sf-impact-extract",
            2,
        ),
        (
            "old-changed",
            &*refs,
            &*old_changed,
            "changed.jsonl",
            "sf-impact-changed",
            1,
        ),
        (
            "empty-refs",
            &[][..],
            &*changed,
            "refs.jsonl",
            "sf-impact-extract",
            2,
        ),
        (
            "empty-changed",
            &*refs,
            "",
            "changed.jsonl",
            "sf-impact-changed",
            1,
        ),
    ] {
        let said = Inputs::new(name, refs, changed).refusal(&[], &[]);

        assert!(
            said.starts_with(&format!("sf-impact-tests: {file}")),
            "{name}: {said}"
        );
        assert!(said.contains("provenance"), "{name}: {said}");
        let version = format!("schema version {version}");
        assert!(said.contains(&version), "{name}: {said}");
        assert!(said.contains(&format!("re-run {tool}")), "{name}: {said}");
    }
}

#[test]
fn a_named_symbol_is_looked_up_in_refs_of_any_tree_that_have_their_provenance_row() {
    // There is no head to hold the refs to: nothing is verified, and
    // nothing is said to be unverified.
    let mut refs = refs_header();
    refs["tree"] = json!({"state": "dirty"});
    let dirty = Inputs::with("symbol-dirty", &refs, &changed_header());

    let document = document(dirty.run(&["Invoice"], &[]));

    assert_eq!(tests(&document), ["InvoiceTest"]);
    assert_eq!(document["confidence"], "full");

    // Refs of an older format are still refused.
    let old = Inputs::new("symbol-old", &refs_file(None, &refs_rows()), "");
    let said = old.refusal(&["Invoice"], &[]);
    assert!(said.contains("schema version 2"), "{said}");
}

#[test]
fn the_same_two_files_give_the_same_bytes_wherever_they_are() {
    let here = Inputs::with("here", &refs_header(), &changed_header());
    let there = Inputs::with("there", &refs_header(), &changed_header());

    let first = here.run(&[], &[]).stdout;

    assert!(!first.is_empty());
    assert_eq!(here.run(&[], &[]).stdout, first);
    assert_eq!(there.run(&[], &[]).stdout, first);
}

/// A first row with the value at `pointer` replaced: `/inputs/refs/schema`.
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
fn a_changed_whose_first_row_is_not_what_the_schema_says_is_refused() {
    let other = "c333333333333333333333333333333333333333";
    // The value that is not valid, where it is, and the field that the
    // refusal names.
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
        // The confidence and the reasons for it say the same.
        ("/confidence", json!("partial"), "gaps"),
        ("/gaps", json!(["unverified_refs"]), "gaps"),
        ("/gaps", json!([]), "gaps"),
        // A confidence that is neither: the refusal names it.
        ("/confidence", json!("certain"), "certain"),
        // The first rows of what it was made of are held to the same rules.
        ("/inputs/refs/schema", json!(1), "inputs.refs.schema"),
        ("/inputs/refs/tool", json!(""), "inputs.refs.tool"),
        (
            "/inputs/refs/tree/commit",
            json!(""),
            "inputs.refs.tree.commit",
        ),
        (
            "/inputs/refs/unexpected",
            json!("x"),
            "inputs.refs.unexpected",
        ),
        ("/inputs/delta/schema", json!(2), "inputs.delta.schema"),
        ("/inputs/delta/base", json!(""), "inputs.delta.base"),
        ("/inputs/delta/scope", json!(null), "inputs.delta.scope"),
        // And to what the row says of itself: it was made of them.
        ("/inputs/delta/head", json!(other), "inputs.delta.head"),
        ("/inputs/delta/base", json!(other), "inputs.delta.base"),
        (
            "/inputs/delta/root",
            json!("packages/shipping"),
            "inputs.delta.root",
        ),
        (
            "/inputs/refs/config",
            json!("fedcba9876543210"),
            "inputs.refs",
        ),
        (
            "/inputs/refs/root",
            json!("packages/shipping"),
            "inputs.refs",
        ),
        (
            "/inputs/refs/tree",
            json!({"state": "dirty"}),
            "inputs.refs",
        ),
    ];

    for (pointer, value, field) in malformed {
        let changed = with(changed_header(), pointer, value.clone());
        let inputs = Inputs::with("malformed-changed", &refs_header(), &changed);

        for extra in [&[][..], &["--accept-unverified-refs"]] {
            let said = inputs.refusal(&[], extra);

            let case = format!("{pointer} = {value}: {said}");
            assert!(said.starts_with("sf-impact-tests: changed.jsonl"), "{case}");
            assert!(said.contains(&format!("`{field}`")), "{case}");
        }
    }
}

#[test]
fn a_changed_that_says_it_is_partial_for_refs_that_it_shows_were_verified_is_refused() {
    let mut changed = changed_header();
    changed["confidence"] = json!("partial");
    changed["gaps"] = json!(["unverified_refs"]);
    let inputs = Inputs::with("partial-of-verified", &refs_header(), &changed);

    let said = inputs.refusal(&[], &[]);

    assert!(said.starts_with("sf-impact-tests: changed.jsonl"), "{said}");
    assert!(said.contains("`inputs.refs`"), "{said}");
}

#[test]
fn refs_whose_first_row_is_not_what_the_schema_says_are_refused_with_changed_or_with_a_symbol() {
    let malformed = [
        ("/tool", json!(""), "tool"),
        ("/version", json!("0.not-semver"), "version"),
        ("/tree/commit", json!(""), "tree.commit"),
        ("/tree/tree", json!(""), "tree.tree"),
        ("/root", json!("/home/someone/project"), "root"),
        ("/config", json!(""), "config"),
        ("/unexpected", json!("x"), "unexpected"),
    ];

    for (pointer, value, field) in malformed {
        let refs = with(refs_header(), pointer, value.clone());
        let inputs = Inputs::with("malformed-refs", &refs, &changed_header());

        for seeds in [&[][..], &["Invoice"]] {
            let said = inputs.refusal(seeds, &[]);

            let case = format!("{pointer} = {value}: {said}");
            assert!(said.starts_with("sf-impact-tests: refs.jsonl"), "{case}");
            assert!(said.contains(&format!("`{field}`")), "{case}");
        }
    }
}

/// A first row without the field at `pointer`: `/inputs/refs/root`.
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
    let accepted = Inputs::with("unknown-null-root", &unknown, &changed_header());
    assert_eq!(
        accepted.document(&["--accept-unverified-refs"])["confidence"],
        "partial"
    );
    assert!(accepted.run(&["Invoice"], &[]).status.success());

    let omitted = Inputs::with(
        "unknown-no-root",
        &without(unknown, "/root"),
        &changed_header(),
    );
    // With changed.jsonl, told to accept what cannot be verified or not,
    // and with a symbol, where no head is held to at all.
    for (seeds, extra) in [
        (&[][..], &[][..]),
        (&[], &["--accept-unverified-refs"]),
        (&["Invoice"], &[]),
        (&["Invoice"], &["--accept-unverified-refs"]),
    ] {
        let said = omitted.refusal(seeds, extra);
        assert!(said.starts_with("sf-impact-tests: refs.jsonl"), "{said}");
        assert!(said.contains("`root` is missing"), "{said}");
    }

    for (pointer, field) in [
        ("/tool", "tool"),
        ("/version", "version"),
        ("/schema", "schema"),
        ("/tree", "tree"),
        ("/tree/state", "tree.state"),
        ("/root", "root"),
        ("/scope", "scope"),
        ("/scope/unpackagedMetadata", "scope.unpackagedMetadata"),
        ("/config", "config"),
    ] {
        let refs = without(refs_header(), pointer);
        let inputs = Inputs::with("refs-without", &refs, &changed_header());
        for seeds in [&[][..], &["Invoice"]] {
            let said = inputs.refusal(seeds, &["--accept-unverified-refs"]);

            assert!(
                said.starts_with("sf-impact-tests: refs.jsonl"),
                "{pointer}: {said}"
            );
            assert!(
                said.contains(&format!("`{field}` is missing")),
                "{pointer}: {said}"
            );
        }
    }
}

#[test]
fn a_changed_without_a_field_that_the_schema_requires_is_refused_though_it_may_be_null() {
    // Made of refs of a tree that is unknown and at no place that is
    // known: the row of those refs has a `root` that is null.
    let mut partial = changed_header();
    partial["confidence"] = json!("partial");
    partial["gaps"] = json!(["unverified_refs"]);
    partial["inputs"]["refs"]["tree"] = json!({"state": "unknown"});
    partial["inputs"]["refs"]["root"] = json!(null);
    let accepted = Inputs::with("nested-null-root", &refs_header(), &partial);
    assert_eq!(accepted.document(&[])["confidence"], "partial");
    let omitted = without(partial, "/inputs/refs/root");
    let said = Inputs::with("nested-no-root", &refs_header(), &omitted).refusal(&[], &[]);
    assert!(said.starts_with("sf-impact-tests: changed.jsonl"), "{said}");
    assert!(said.contains("`inputs.refs.root` is missing"), "{said}");

    // So it is for every field that the schema requires, of the row and
    // of the two rows in it.
    let own = [
        "tool",
        "version",
        "schema",
        "base",
        "head",
        "head_tree",
        "root",
        "scope",
        "config",
    ];
    let of_refs = [
        "tool", "version", "schema", "tree", "root", "scope", "config",
    ];
    let top = own.iter().map(|field| field.to_string());
    let top = top.chain(["confidence", "inputs", "inputs/refs", "inputs/delta"].map(String::from));
    let nested_refs = of_refs.iter().map(|field| format!("inputs/refs/{field}"));
    let nested_delta = own.iter().map(|field| format!("inputs/delta/{field}"));
    let deeper = [
        "inputs/refs/tree/state",
        "inputs/delta/scope/packageDirectories",
        "scope/unpackagedMetadata",
    ];
    for path in top
        .chain(nested_refs)
        .chain(nested_delta)
        .chain(deeper.map(String::from))
    {
        let changed = without(changed_header(), &format!("/{path}"));
        let inputs = Inputs::with("changed-without", &refs_header(), &changed);

        let said = inputs.refusal(&[], &["--accept-unverified-refs"]);

        let field = path.replace('/', ".");
        assert!(
            said.starts_with("sf-impact-tests: changed.jsonl"),
            "{path}: {said}"
        );
        assert!(
            said.contains(&format!("`{field}` is missing")),
            "{path}: {said}"
        );
    }
}

#[test]
fn a_changed_that_another_tool_wrote_to_the_same_schema_is_read() {
    // Any stage can be replaced: a row has to name what wrote it, and
    // need not name these tools. All else is held to as it is of theirs.
    let mut changed = with(changed_header(), "/tool", json!("another-changed"));
    changed["inputs"]["delta"]["tool"] = json!("another-delta");
    let inputs = Inputs::with("other-producers", &refs_header(), &changed);

    let document = inputs.document(&[]);

    assert_eq!(tests(&document), ["InvoiceTest"]);
    assert_eq!(document["confidence"], "full");
    assert!(inputs.run(&["Invoice"], &[]).status.success());
}

#[test]
fn a_refs_pack_is_of_sf_impact_extract_and_one_of_another_tool_is_refused() {
    // As of schema 2, refs are a pack, and a pack's line 1 is the
    // provenance row of sf-impact-extract (docs/pack.md). One that names
    // another tool is refused, with changed or with a symbol.
    let refs = refs_file(Some(&refs_header()), &refs_rows());
    let other = with(refs_header(), "/tool", json!("sf-impact-other")).to_string();
    let line_1 = refs.iter().position(|b| *b == b'\n').unwrap();
    let mut of_other = other.into_bytes();
    of_other.extend_from_slice(&refs[line_1..]);
    let changed = lines(Some(&changed_header()), &changed_rows());
    let inputs = Inputs::new("other-producer", &pack::resealed(of_other), &changed);

    for seeds in [&[][..], &["Invoice"]] {
        let said = inputs.refusal(seeds, &[]);

        assert!(said.contains("refs pack"), "{said}");
        assert!(said.contains("sf-impact-extract"), "{said}");
    }
}
