//! The independently implemented schema validator and production readers must agree.
use serde_json::{Value, json};
use sf_impact_common::{
    changed, delta,
    provenance::{self, Reader},
    rows,
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

fn reader(kind: &str, value: &Value) -> bool {
    read_result(kind, value).is_ok()
}

fn read_result(kind: &str, value: &Value) -> Result<(), String> {
    let reader = Reader {
        tool: "contract-test",
        version: env!("CARGO_PKG_VERSION"),
    };
    let line = value.to_string();
    if value["row"] == "provenance" {
        return match kind {
            "refs" => reader
                .read::<provenance::Refs>(&mut line.as_bytes(), "re-run")
                .map(|_| ()),
            "delta" => reader
                .read::<provenance::Delta>(&mut line.as_bytes(), "re-run")
                .map(|_| ()),
            "changed" => reader
                .read::<provenance::Changed>(&mut line.as_bytes(), "re-run")
                .map(|_| ()),
            _ => unreachable!(),
        };
    }
    match kind {
        "refs" => serde_json::from_value::<rows::Row>(value.clone())
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "delta" => serde_json::from_value::<delta::Row>(value.clone())
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "changed" => serde_json::from_value::<changed::Row>(value.clone())
            .map(|_| ())
            .map_err(|e| e.to_string()),
        "impact" => sf_impact_render::render(line.as_bytes(), "md").map(|_| ()),
        _ => unreachable!(),
    }
}

/// Only reader-only normative relations may disagree, and only in the
/// schema-accepts/reader-refuses direction. New reader errors fail the test.
fn mutation_agrees(kind: &str, pointer: &str, schema: bool, result: &Result<(), String>) -> bool {
    schema == result.is_ok()
        || (schema
            && result
                .as_ref()
                .err()
                .and_then(|e| reader_only_rule(kind, pointer, e))
                .is_some())
}

fn reader_only_rule(kind: &str, pointer: &str, error: &str) -> Option<&'static str> {
    if pointer.split('/').any(|part| part == "scope")
        && error.contains("is not as the directories of a project file are written here:")
    {
        return Some("normalised, strictly sorted and disjoint scope lists");
    }
    if matches!(kind, "changed" | "impact") {
        for field in ["base", "head", "head_tree", "root", "scope", "config"] {
            if error.contains(&format!(
                "is not the `{field}` of the row, which is made of it"
            )) {
                return Some("changed provenance equals inputs.delta");
            }
            if error.contains(&format!("has the `{field}`"))
                && error.contains("and the head of the row has")
            {
                return Some("refs fit the head of changed provenance");
            }
            if error.contains(&format!("`inputs.refs.{field}`"))
                && error.contains("in refs.jsonl and")
                && error.contains("in changed.jsonl")
            {
                return Some("impact inputs agree on the changed head");
            }
        }
    }
    if kind != "impact" {
        return None;
    }
    let error = error
        .strip_prefix("impact.json: ")?
        .strip_suffix("; re-run sf-impact-tests")?;
    match error {
        "confidence disagrees with gaps" => Some("impact confidence agrees with its reasons"),
        "seed counts.parsed disagrees with direct references"
        | "seed counts.textual disagrees with direct references" => {
            Some("impact counts agree with distinct direct references")
        }
        _ => {
            let id = error
                .strip_prefix("references: missing reference ")
                .or_else(|| error.strip_prefix("referrer tier textual disagrees with reference "))
                .or_else(|| error.strip_prefix("referrer tier parsed disagrees with reference "))?;
            id.parse::<u64>()
                .ok()
                .map(|_| "impact IDs exist and agree with referrer tiers")
        }
    }
}

#[test]
fn agreement_rejects_both_directions_of_unlisted_disagreement() {
    assert!(
        !mutation_agrees("refs", "", false, &Ok(())),
        "reader cannot accept a schema-invalid mutation"
    );
    assert!(
        !mutation_agrees("refs", "", true, &Err("unknown field `new`".into())),
        "unknown reader-only rules cannot be skipped"
    );
    assert!(mutation_agrees("impact", "", false, &Err("impact.json: seed counts.parsed disagrees with direct references; re-run sf-impact-tests".into())));
    assert!(mutation_agrees("impact", "", true, &Err("impact.json: seed counts.parsed disagrees with direct references; re-run sf-impact-tests".into())));
}

#[test]
fn scope_exceptions_require_a_scope_mutation() {
    let error = "is not as the directories of a project file are written here:";
    for pointer in ["/tree", "/root", "/not_scope/packageDirectories"] {
        assert!(
            reader_only_rule("refs", pointer, error).is_none(),
            "{pointer}"
        );
    }
    for pointer in [
        "/scope",
        "/scope/packageDirectories/0",
        "/inputs/refs/scope/packageDirectories",
    ] {
        assert!(
            reader_only_rule("refs", pointer, error).is_some(),
            "{pointer}"
        );
    }
}

fn files(at: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(at).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(&path, out);
        } else {
            out.push(path);
        }
    }
}
#[test]
fn committed_fixtures_and_invalid_variants_agree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut paths = vec![];
    files(&root.join("fixtures"), &mut paths);
    files(&root.join("crates/sf-impact-tests/tests/data"), &mut paths);
    paths.sort();
    let mut disagreements = vec![];
    let mut count = 0;
    let mut representatives = BTreeSet::new();
    for kind in ["refs", "delta", "changed", "impact"] {
        let schema: Value = serde_json::from_str(
            &fs::read_to_string(root.join(format!("schema/{kind}.schema.json"))).unwrap(),
        )
        .unwrap();
        let validator = jsonschema::draft202012::new(&schema).unwrap();
        let mut check = |label: &str, pointer: &str, value: &Value, valid: Option<bool>| {
            count += 1;
            let schema = validator.is_valid(value);
            let result = read_result(kind, value);
            let rust = result.is_ok();
            let agrees = match valid {
                Some(expected) => schema == expected && rust == expected,
                None => mutation_agrees(kind, pointer, schema, &result),
            };
            if !agrees {
                disagreements.push(format!(
                    "{kind} {label}: expected {valid:?}, schema={schema}, reader={rust}, error={result:?}"
                ));
            }
        };
        for path in &paths {
            let name = path.to_string_lossy();
            let matches = if kind == "refs" {
                name.ends_with("refs.jsonl")
            } else if kind == "impact" {
                name.contains("/fixtures/impact/") && name.ends_with(".json")
            } else {
                name.contains(&format!("/fixtures/{kind}/")) && name.ends_with(".jsonl")
            };
            if !matches {
                continue;
            }
            let text = fs::read_to_string(path).unwrap();
            let values: Vec<Value> = if kind == "impact" {
                vec![serde_json::from_str(&text).unwrap()]
            } else {
                text.lines()
                    .map(|l| serde_json::from_str(l).unwrap())
                    .collect()
            };
            for (i, value) in values.iter().enumerate() {
                check(&format!("{name}:{}", i + 1), "", value, Some(true));
                // Exercise each structural/conditional shape without repeating the
                // same mutation thousands of times in the large captured sample.
                let signature = format!(
                    "{kind}:{}",
                    [
                        "row",
                        "tier",
                        "kind",
                        "status",
                        "reason",
                        "detail",
                        "confidence"
                    ]
                    .map(|key| value[key].to_string())
                    .join(":")
                );
                if representatives.insert(signature) {
                    for (pointer, candidate) in mutations(value) {
                        check(&format!("mutation {pointer}"), &pointer, &candidate, None);
                    }
                }
                let mut unknown = value.clone();
                unknown["unknown"] = json!(true);
                check("unknown field", "", &unknown, Some(false));
            }
            if kind != "impact" && values[0]["row"] != "provenance" {
                continue;
            }
            let mut first = values[0].clone();
            let envelope = if kind == "impact" {
                &mut first["provenance"]
            } else {
                &mut first
            };
            envelope["version"] = json!("bad");
            check("bad version", "", &first, Some(false));
            let mut first = values[0].clone();
            let envelope = if kind == "impact" {
                &mut first["provenance"]
            } else {
                &mut first
            };
            // The file's own version, spelled as a float: 1.0 or 2.0.
            let version = envelope["schema"].as_u64().unwrap() as f64;
            envelope["schema"] = json!(version);
            // Draft 2020-12 defines integer mathematically; this lexical
            // distinction is the sole expected exception here.
            assert!(
                validator.is_valid(&first),
                "1.0 is mathematically integer: {kind} {name}: {:?}",
                validator
                    .iter_errors(&first)
                    .map(|e| e.to_string())
                    .collect::<Vec<_>>()
            );
            assert!(
                !reader(kind, &first),
                "reader refuses floating schema tokens"
            );
        }
        if kind == "refs" {
            let base = json!({"row":"reference","tier":"parsed","kind":"call","referenced":"A.m()","referenced_type":"method","resolution":"declaration","referrer":null,"package":"app","file":"app/A.cls","line":1,"column":1});
            check("parsed control", "", &base, Some(true));
            // A declaration says what kind of thing it is. A name that is not
            // declared, or candidates of different kinds, need not.
            let mut v = base.clone();
            v.as_object_mut().unwrap().remove("referenced_type");
            check("declaration without referenced_type", "", &v, Some(false));
            for wrong in [
                Value::Null,
                json!("widget"),
                json!("interface"),
                json!(["method"]),
            ] {
                let mut v = base.clone();
                v["referenced_type"] = wrong;
                check("referenced_type of the wrong shape", "", &v, Some(false));
            }
            let mut v = base.clone();
            v.as_object_mut().unwrap().remove("referenced_type");
            v["resolution"] = json!("unresolved");
            check("unresolved without referenced_type", "", &v, Some(true));
            let mut v = base.clone();
            v.as_object_mut().unwrap().remove("referenced_type");
            v["detail"] = json!("ambiguous_receiver");
            v["candidates"] = json!(["A", "A.A()"]);
            check("candidates without referenced_type", "", &v, Some(true));
            let mut v = base.clone();
            v["tier"] = json!("textual");
            v["kind"] = json!("text");
            v.as_object_mut().unwrap().remove("resolution");
            check("textual with referenced_type", "", &v, Some(false));
            let mut v = base.clone();
            v["tier"] = json!("textual");
            v["kind"] = json!("text");
            check("wrong reference shape", "", &v, Some(false));
            let mut v = base;
            v["detail"] = json!("ambiguous_receiver");
            check("if/then candidates missing", "", &v, Some(false));
            let refs = fs::read_to_string(root.join("fixtures/refs.jsonl")).unwrap();
            let declaration: Value = refs
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .find(|row| row["row"] == "declaration")
                .unwrap();
            let mut v = declaration.clone();
            v.as_object_mut().unwrap().remove("scope");
            check("declaration without scope", "", &v, Some(false));
            let mut v = declaration;
            v["scope"] = Value::Null;
            check("declaration with null scope", "", &v, Some(false));
            let parse_error: Value = refs
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .find(|row| row["row"] == "parse_error")
                .unwrap();
            check("parse_error control", "", &parse_error, Some(true));
            let mut v = parse_error;
            v["errors"] = json!(1);
            check("parse_error with an errors count", "", &v, Some(false));
            let base: Value = serde_json::from_str(refs.lines().next().unwrap()).unwrap();
            let mut v = base.clone();
            v["tree"] = json!({"state":"tree","tree":"a".repeat(40)});
            check("tree known by its tree alone", "", &v, Some(false));
            for id in [
                "a".repeat(39),
                "A".repeat(40),
                format!("{}\n", "a".repeat(40)),
            ] {
                let mut v = base.clone();
                v["tree"] = json!({"state":"commit","commit":id,"tree":"a".repeat(40)});
                check("invalid object id", "", &v, Some(false));
            }
        }
    }
    eprintln!("checked {count} contract cases");
    assert!(disagreements.is_empty(), "{}", disagreements.join("\n"));
}

/// Wrong values, absent required keys and unknown nested fields exercise the
/// independent schema rather than duplicating its implementation in the test.
fn mutations(value: &Value) -> Vec<(String, Value)> {
    fn visit(value: &Value, at: String, paths: &mut Vec<String>) {
        paths.push(at.clone());
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    visit(
                        value,
                        format!("{at}/{}", key.replace('~', "~0").replace('/', "~1")),
                        paths,
                    );
                }
            }
            Value::Array(array) => {
                for (i, value) in array.iter().enumerate() {
                    visit(value, format!("{at}/{i}"), paths);
                }
            }
            _ => {}
        }
    }
    let mut paths = vec![];
    visit(value, String::new(), &mut paths);
    let mut variants = vec![];
    for pointer in paths {
        for wrong in [
            Value::Null,
            json!(-1),
            json!(4294967296u64),
            json!(0),
            json!(false),
            json!([]),
            json!({}),
            json!(""),
            json!("invalid"),
        ] {
            let mut candidate = value.clone();
            *candidate.pointer_mut(&pointer).unwrap() = wrong;
            variants.push((pointer.clone(), candidate));
        }
        if let Some(map) = value.pointer(&pointer).unwrap().as_object() {
            for key in map.keys().chain(std::iter::once(&"unknown".to_string())) {
                let mut candidate = value.clone();
                let object = candidate
                    .pointer_mut(&pointer)
                    .unwrap()
                    .as_object_mut()
                    .unwrap();
                if key == "unknown" {
                    object.insert(key.clone(), json!(true));
                } else {
                    object.remove(key);
                }
                variants.push((format!("{pointer}/{key}"), candidate));
            }
        }
    }
    variants
}

#[test]
fn patterns_match_the_readers_edges() {
    let schema: Value =
        serde_json::from_str(include_str!("../../../schema/refs.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let first: Value = serde_json::from_str(
        include_str!("../../../fixtures/refs.jsonl")
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    for (key, values) in [
        (
            "root",
            vec![
                "", "..", "/app", "a//b", "a/./b", "a/../b", "a/", "C:/a", "a\\b", "a\n\\b",
            ],
        ),
        ("version", vec!["1", "1.2", "0.x.0", "1.2.3.4", "1.2.3\n"]),
        (
            "config",
            vec!["0123456789abcde", "0123456789abcdeF", "0123456789abcdef0"],
        ),
    ] {
        for value in values {
            let mut candidate = first.clone();
            candidate[key] = json!(value);
            assert!(
                !validator.is_valid(&candidate),
                "schema accepts {key}={value:?}"
            );
            assert!(
                !reader("refs", &candidate),
                "reader accepts {key}={value:?}"
            );
        }
    }
    for (key, values) in [
        ("root", vec![".", "a", "a/b", "a:b/c", "a\nb"]),
        (
            "version",
            vec![
                "0.1.0",
                "0.01.00",
                "0.1.0-",
                "0.1.0+",
                "0.1.0-\nsuffix",
                "1.0.0",
            ],
        ),
    ] {
        for value in values {
            let mut candidate = first.clone();
            candidate[key] = json!(value);
            assert!(
                validator.is_valid(&candidate),
                "schema refuses {key}={value:?}"
            );
            assert!(reader("refs", &candidate), "reader refuses {key}={value:?}");
        }
    }
    for length in [40, 64] {
        let mut candidate = first.clone();
        candidate["tree"] =
            json!({"state":"commit", "commit":"a".repeat(length), "tree":"b".repeat(length)});
        assert!(validator.is_valid(&candidate));
        assert!(reader("refs", &candidate));
    }
}

#[test]
fn normative_prose_exceptions_are_explicit() {
    let refs_schema: Value =
        serde_json::from_str(include_str!("../../../schema/refs.schema.json")).unwrap();
    let refs_validator = jsonschema::draft202012::new(&refs_schema).unwrap();
    let first: Value = serde_json::from_str(
        include_str!("../../../fixtures/refs.jsonl")
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    for scope in [
        json!({"packageDirectories":["b","a"],"unpackagedMetadata":[]}),
        json!({"packageDirectories":["./app"],"unpackagedMetadata":[]}),
        json!({"packageDirectories":["app"],"unpackagedMetadata":["app"]}),
    ] {
        let mut candidate = first.clone();
        candidate["scope"] = scope;
        assert!(refs_validator.is_valid(&candidate));
        assert!(!reader("refs", &candidate));
    }
    let schema: Value =
        serde_json::from_str(include_str!("../../../schema/changed.schema.json")).unwrap();
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let first: Value = serde_json::from_str(
        include_str!("../../../fixtures/changed/whitespace-only.jsonl")
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    for pointer in ["/inputs/delta/base", "/inputs/refs/config"] {
        let mut candidate = first.clone();
        *candidate.pointer_mut(pointer).unwrap() = json!(if pointer.ends_with("base") {
            "0".repeat(40)
        } else {
            "0".repeat(16)
        });
        assert!(validator.is_valid(&candidate));
        assert!(!reader("changed", &candidate), "inconsistent {pointer}");
    }
    let mut candidate = first;
    candidate["inputs"]["refs"]["tree"] = json!({"state":"dirty"});
    assert!(validator.is_valid(&candidate));
    assert!(
        !reader("changed", &candidate),
        "full confidence requires verified refs"
    );
}

#[test]
fn impact_references_have_the_same_three_shapes_as_refs() {
    let impact: Value =
        serde_json::from_str(include_str!("../../../schema/impact.schema.json")).unwrap();
    let schema = json!({"$schema":"https://json-schema.org/draft/2020-12/schema", "$defs":impact["$defs"], "$ref":"#/$defs/reference"});
    let validator = jsonschema::draft202012::new(&schema).unwrap();
    let mut seen = BTreeSet::new();
    for line in include_str!("../../../fixtures/refs.jsonl").lines() {
        let mut value: Value = serde_json::from_str(line).unwrap();
        if value["row"] != "reference" {
            continue;
        }
        value.as_object_mut().unwrap().remove("row");
        assert!(validator.is_valid(&value));
        assert!(serde_json::from_value::<rows::Reference>(value.clone()).is_ok());
        if seen.insert(format!(
            "{}:{}:{}",
            value["tier"], value["kind"], value["detail"]
        )) {
            for (pointer, candidate) in mutations(&value) {
                assert_eq!(
                    validator.is_valid(&candidate),
                    serde_json::from_value::<rows::Reference>(candidate.clone()).is_ok(),
                    "reference {pointer}: {candidate}"
                );
            }
        }
    }
}
