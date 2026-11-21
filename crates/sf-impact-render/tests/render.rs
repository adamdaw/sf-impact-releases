#[path = "../../sf-impact-tests/tests/support/mod.rs"]
mod support;
use serde_json::{Value, json};
use sf_impact_render::{COMMENT_BUDGET, render};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn fixture(name: &str) -> Value {
    let mut doc: Value = serde_json::from_slice(
        &fs::read(root().join(format!("fixtures/impact/{name}.json"))).unwrap(),
    )
    .unwrap();
    // Existing report bodies, with a coherent independently constructed envelope.
    let mut refs = support::refs_header();
    let mut changed = support::changed_header();
    refs.as_object_mut().unwrap().remove("row");
    changed.as_object_mut().unwrap().remove("row");
    doc["provenance"] = json!({"tool":"sf-impact-tests","version":"0.1.0","schema":1,"inputs":{"refs":refs,"changed":changed}});
    doc
}
fn output(doc: &Value, format: &str) -> String {
    render(&serde_json::to_vec(doc).unwrap(), format).unwrap()
}
fn golden(doc: &Value, name: &str) {
    for format in ["md", "gh-step"] {
        let expected =
            fs::read_to_string(root().join(format!("fixtures/render/{name}.{format}"))).unwrap();
        let actual = output(doc, format);
        assert_eq!(actual, expected, "{name}.{format}");
        assert_eq!(actual, output(doc, format), "deterministic {name}.{format}");
    }
}
#[test]
fn partial_reasons_and_selected_tests() {
    golden(&fixture("method"), "method");
}
#[test]
fn untested_and_empty_selection() {
    golden(&fixture("binding"), "binding");
}
#[test]
fn unpackaged_selection() {
    golden(&fixture("unpackaged"), "unpackaged");
}
#[test]
fn excluded_selection() {
    golden(&fixture("unpackaged-excluded"), "unpackaged-excluded");
}
#[test]
fn full_confidence() {
    let mut doc = fixture("method");
    doc["confidence"] = json!("full");
    doc["gaps"] = json!([]);
    golden(&doc, "full");
}
#[test]
fn an_unreached_collision_count_is_read_and_changes_nothing_rendered() {
    for name in ["method", "binding"] {
        let mut doc = fixture(name);
        doc["diagnostics"]["unreached_collision"] = json!(3);
        golden(&doc, name);
    }
}
#[test]
fn possible_only_is_counted_collapsed_and_explained() {
    let mut doc = fixture("method");
    doc["possible"] = fixture("unpackaged")["tests"].clone();
    doc["tests"] = json!([]);
    doc["possible"][0]["reasons"] = json!(["member_name_only"]);
    doc["references"].as_object_mut().unwrap().extend(
        fixture("unpackaged")["references"]
            .as_object()
            .unwrap()
            .clone(),
    );
    golden(&doc, "possible");
}
#[test]
fn user_text_is_inert_code_including_backticks_mentions_and_issues() {
    let mut doc = fixture("method");
    doc["tests"][0]["symbol"] = json!("`@octocat #1 GH-1 :smile: <b> $x$``");
    doc["tests"][0]["file"] = json!("a|b\n\u{202e}#2");
    doc["gaps"][0]["file"] = json!("[p](url)<script>&\r\n*gap*");
    let actual = output(&doc, "md");
    assert!(actual.contains("``` `@octocat #1 GH-1 :smile: <b> $x$`` ```"));
    assert!(actual.contains("`a|b��#2:1`"));
    assert!(!actual.contains("&#64;"));
    assert_eq!(
        actual,
        fs::read_to_string(root().join("fixtures/render/escaped.md")).unwrap()
    );
}
#[test]
fn step_rejects_invalid_runspecifiedtests_names_in_one_line() {
    for bad in [
        "",
        "1Test",
        "Outer.Inner",
        "A,B",
        "--target-org=prod",
        "A\r\nconfidence=full",
        "_Test",
        "A__B",
        "Test_",
        "Tést",
    ] {
        let mut doc = fixture("method");
        doc["tests"][0]["symbol"] = json!(bad);
        let result = cli(
            &serde_json::to_vec(&doc).unwrap(),
            &["--impact", "-", "--format", "gh-step"],
        );
        assert_eq!(result.status.code(), Some(1), "accepted {bad:?}");
        assert!(result.stdout.is_empty());
        let error = String::from_utf8(result.stderr).unwrap();
        assert_eq!(error.lines().count(), 1);
        assert!(error.contains(&format!("{bad:?}")), "{error}");
        assert!(error.contains("RunSpecifiedTests"));
    }
}
#[test]
fn convention_evidence_is_read_and_belongs_to_the_convention_reason() {
    // The golden's selected test has the reasons convention and direct.
    let mut doc = fixture("method");
    doc["tests"][0]["convention"] =
        json!([{"symbol":"ShapeServiceTest.Areas","file":doc["tests"][0]["file"]}]);
    golden(&doc, "method");
    doc["tests"][0]["reasons"] = json!(["direct"]);
    assert!(render(&serde_json::to_vec(&doc).unwrap(), "md").is_err());
    let mut doc = fixture("method");
    doc["tests"][0]["convention"] = json!([]);
    assert!(render(&serde_json::to_vec(&doc).unwrap(), "md").is_err());
}
#[test]
fn output_names_are_sorted_and_deduplicated() {
    let mut doc = fixture("method");
    let mut other = doc["tests"][0].clone();
    other["symbol"] = json!("A_Test");
    doc["tests"].as_array_mut().unwrap().push(other.clone());
    other["file"] = json!("another/A_Test.cls");
    doc["tests"].as_array_mut().unwrap().push(other);
    assert!(output(&doc, "gh-step").starts_with("tests=A_Test,ShapeServiceTest\n"));
    assert!(output(&doc, "gh-step").contains("selected_count=2\n"));
}
#[test]
fn section_caps_preserve_order_and_do_not_starve_changed_symbols() {
    let mut doc = fixture("method");
    let node = fixture("binding")["untested"][0].clone();
    doc["untested"] = json!(
        (0..1200)
            .map(|i| {
                let mut n = node.clone();
                n["symbol"] = json!(format!("Uncovered{i:04}"));
                n
            })
            .collect::<Vec<_>>()
    );
    let md = output(&doc, "md");
    let positions = [
        "| Head commit |",
        "### Partial confidence",
        "### Selected tests",
        "### Untested",
        "### Changed symbols",
        "<summary>Possible",
        "<summary>Direct referrers",
    ]
    .map(|s| md.find(s).unwrap());
    assert!(positions.windows(2).all(|p| p[0] < p[1]));
    let untested = &md[positions[3]..positions[4]];
    assert!(untested.len() <= 4000);
    let shown = untested
        .lines()
        .filter(|l| l.starts_with("- `Uncovered"))
        .count();
    assert!(shown > 0 && shown < 1200);
    assert!(untested.contains(&format!("\n\n**{} more not shown**", 1200 - shown)));
    assert!(md[positions[4]..positions[5]].contains("- `ShapeService.totalArea(list<shape>)`"));
    assert!(md.len() < COMMENT_BUDGET);
}

#[test]
fn possible_and_referrer_truncation_close_details() {
    let mut doc = fixture("method");
    doc["references"].as_object_mut().unwrap().extend(
        fixture("unpackaged")["references"]
            .as_object()
            .unwrap()
            .clone(),
    );
    let mut possible = fixture("unpackaged")["tests"][0].clone();
    possible["reasons"] = json!(["textual_path"]);
    doc["possible"] = json!(
        (0..1000)
            .map(|i| {
                let mut p = possible.clone();
                p["symbol"] = json!(format!("Maybe{i:04}"));
                p
            })
            .collect::<Vec<_>>()
    );
    let md = output(&doc, "md");
    assert!(md.len() <= COMMENT_BUDGET);
    assert!(md.contains("<summary>Possible tests (1000)"));
    assert_eq!(
        md.matches("<details>").count(),
        md.matches("</details>").count()
    );
    let shown = md.lines().filter(|l| l.starts_with("- `Maybe")).count();
    assert!(shown > 0 && shown < 1000);
    assert!(md.contains(&format!("{} more not shown", 1000 - shown)));
}
#[test]
fn repeated_reasons_are_grouped_counted_and_located_before_tests() {
    let mut doc = fixture("method");
    doc["gaps"]=json!((1..=1000).map(|line|json!({"reason":"unresolved","file":"src/Service.cls","line":line,"source":"refs"})).collect::<Vec<_>>());
    let md = output(&doc, "md");
    assert!(md.contains("### Partial confidence (1000 reasons)"));
    assert!(md.contains(
        "`unresolved` (refs), `src/Service.cls`: 1000 reasons; first lines: 1, 2, 3, 4, 5"
    ));
    assert!(md.contains("Full list: impact.json artifact"));
    assert!(md.len() < 4000);
    assert!(md.contains("ShapeServiceTest"));
    assert!(
        md.contains(
            doc["provenance"]["inputs"]["changed"]["head"]
                .as_str()
                .unwrap()
        )
    );
}
#[test]
fn size_never_refuses_a_valid_report_even_for_huge_groups_or_tests() {
    for field in ["tests", "gaps", "untested", "seeds"] {
        let mut doc = fixture("method");
        if field == "untested" {
            doc[field] = fixture("binding")[field].clone();
        }
        let key = if field == "gaps" { "file" } else { "symbol" };
        doc[field][0][key] = json!("🦀`".repeat(COMMENT_BUDGET));
        let md = output(&doc, "md");
        assert!(md.len() <= COMMENT_BUDGET);
        assert!(md.contains("more not shown"));
        assert!(md.contains("impact.json artifact"));
        assert!(
            md.contains(
                doc["provenance"]["inputs"]["changed"]["head"]
                    .as_str()
                    .unwrap()
            )
        );
        assert!(md.contains("partial"));
    }
}
fn cli(input: &[u8], args: &[&str]) -> std::process::Output {
    let child = Command::new(env!("CARGO_BIN_EXE_sf-impact-render"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    support::feed_and_wait(child, input)
}
#[test]
fn cli_reads_only_input_and_writes_requested_destination() {
    let doc = fixture("method");
    let bytes = serde_json::to_vec(&doc).unwrap();
    let path = std::env::temp_dir().join(format!("sf-impact-render-{}.md", std::process::id()));
    let args = [
        "--impact",
        "-",
        "--format",
        "md",
        "--out",
        path.to_str().unwrap(),
    ];
    let result = cli(&bytes, &args);
    assert!(result.status.success(), "{result:?}");
    assert!(result.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        fs::read_to_string(root().join("fixtures/render/method.md")).unwrap()
    );
    fs::remove_file(path).unwrap();
    let result = cli(
        &bytes,
        &["--impact", "-", "--format", "gh-step", "--out", "stdout"],
    );
    assert!(result.status.success());
    assert_eq!(
        result.stdout,
        fs::read(root().join("fixtures/render/method.gh-step")).unwrap()
    );
}
#[test]
fn malformed_inputs_are_refused_in_one_line_without_output() {
    for bytes in [b"{".as_slice(), b"{}", b"null", b"[]"] {
        let result = cli(bytes, &["--impact", "-", "--format", "md"]);
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        let err = String::from_utf8(result.stderr).unwrap();
        assert_eq!(err.lines().count(), 1);
        assert!(err.contains("re-run sf-impact-tests"));
    }
}
#[test]
fn missing_invalid_and_mismatched_fields_are_refused() {
    assert!(render(&serde_json::to_vec(&fixture("method")).unwrap(), "md").is_ok());
    let mutations = [
        ("/provenance", Value::Null),
        ("/provenance/schema", json!(2)),
        ("/provenance/inputs/refs/root", json!("elsewhere")),
        ("/provenance/inputs/refs/tree/tree", json!("f".repeat(40))),
        (
            "/provenance/inputs/changed/inputs/delta/head",
            json!("f".repeat(40)),
        ),
        ("/provenance/inputs/refs/config", json!("BAD")),
        ("/tests/0/reasons", json!([])),
        ("/seeds/0/referrers/0/parsed/0/references/0", json!(999999)),
        ("/seeds/0/counts/parsed", json!(999)),
        ("/confidence", json!("full")),
        ("/tests/0/line", json!(0)),
    ];
    for (pointer, value) in mutations {
        let mut doc = fixture("method");
        *doc.pointer_mut(pointer).unwrap() = value;
        assert!(
            render(&serde_json::to_vec(&doc).unwrap(), "md").is_err(),
            "accepted {pointer}"
        );
    }
    let mut doc = fixture("method");
    doc.as_object_mut().unwrap().remove("confidence");
    assert!(render(&serde_json::to_vec(&doc).unwrap(), "md").is_err());
    let mut doc = fixture("method");
    doc["unexpected"] = json!(true);
    assert!(render(&serde_json::to_vec(&doc).unwrap(), "md").is_err());
}
#[test]
fn errors_are_not_silently_rendered_as_a_successful_selection() {
    let mut doc = fixture("method");
    doc["errors"] = json!([{"name":"Missing","reason":"unknown_symbol"}]);
    for format in ["md", "gh-step"] {
        assert!(
            render(&serde_json::to_vec(&doc).unwrap(), format)
                .unwrap_err()
                .contains("errors")
        );
    }
}

#[test]
fn symbol_query_reports_commit_or_explicitly_unknown_head() {
    let mut doc = fixture("method");
    doc["provenance"]["inputs"]["changed"] = Value::Null;
    let head = doc["provenance"]["inputs"]["refs"]["tree"]["commit"]
        .as_str()
        .unwrap();
    assert!(output(&doc, "md").contains(head));
    doc["provenance"]["inputs"]["refs"]["tree"] = json!({"state":"unknown"});
    assert!(output(&doc, "md").contains("unknown (no commit in input)"));
}

#[test]
fn runtime_dependencies_do_not_include_jsonschema() {
    let manifest = fs::read_to_string(root().join("crates/sf-impact-render/Cargo.toml")).unwrap();
    assert!(
        !manifest
            .split("[dev-dependencies]")
            .next()
            .unwrap()
            .contains("jsonschema")
    );
}
#[test]
fn unverified_provenance_requires_its_reason_including_changed_inputs() {
    let mut doc = fixture("method");
    doc["provenance"]["inputs"]["refs"]["tree"] = json!({"state":"dirty"});
    let bytes = |doc: &Value| serde_json::to_vec(doc).unwrap();
    assert!(render(&bytes(&doc), "md").is_err());
    let gap = json!({"reason":"unverified_refs","file":"refs.jsonl","line":1,"source":"refs"});
    doc["gaps"].as_array_mut().unwrap().push(gap);
    assert!(render(&bytes(&doc), "md").is_ok());
    doc["provenance"]["inputs"]["changed"]["inputs"]["refs"]["tree"] = json!({"state":"unknown"});
    doc["provenance"]["inputs"]["changed"]["confidence"] = json!("partial");
    doc["provenance"]["inputs"]["changed"]["gaps"] = json!(["unverified_refs"]);
    assert!(render(&bytes(&doc), "md").is_err());
    doc["gaps"].as_array_mut().unwrap().push(
        json!({"reason":"unverified_refs","file":"changed.jsonl","line":1,"source":"changed"}),
    );
    assert!(render(&bytes(&doc), "md").is_ok());
}

#[test]
fn symbol_query_cannot_claim_unverified_changed_inputs() {
    let mut doc = fixture("method");
    doc["provenance"]["inputs"]["changed"] = Value::Null;
    assert!(render(&serde_json::to_vec(&doc).unwrap(), "md").is_ok());
    doc["gaps"].as_array_mut().unwrap().push(
        json!({"reason":"unverified_refs","file":"changed.jsonl","line":1,"source":"changed"}),
    );
    assert!(render(&serde_json::to_vec(&doc).unwrap(), "md").is_err());
}

#[test]
fn refusal_preserves_destination_and_is_one_line_even_for_hostile_provenance() {
    let mut doc = fixture("method");
    doc["provenance"]["inputs"]["refs"]["root"] = json!("other\nproject");
    let path = std::env::temp_dir().join(format!(
        "sf-impact-render-preserved-{}.md",
        std::process::id()
    ));
    fs::write(&path, "previous comment").unwrap();
    let result = cli(
        &serde_json::to_vec(&doc).unwrap(),
        &[
            "--impact",
            "-",
            "--format",
            "md",
            "--out",
            path.to_str().unwrap(),
        ],
    );
    assert_eq!(result.status.code(), Some(1));
    assert!(result.stdout.is_empty());
    assert_eq!(String::from_utf8(result.stderr).unwrap().lines().count(), 1);
    assert_eq!(fs::read_to_string(&path).unwrap(), "previous comment");
    fs::remove_file(path).unwrap();
}

#[test]
fn boundary_keeps_schema_checks_after_removing_runtime_validator() {
    let schema: Value =
        serde_json::from_str(include_str!("../../../schema/impact.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    fn visit(root: &Value, at: &str, pointers: &mut Vec<String>) {
        pointers.push(at.into());
        match root {
            Value::Object(map) => {
                for (k, v) in map {
                    visit(
                        v,
                        &format!("{at}/{}", k.replace('~', "~0").replace('/', "~1")),
                        pointers,
                    )
                }
            }
            Value::Array(array) => {
                for (i, v) in array.iter().enumerate() {
                    visit(v, &format!("{at}/{i}"), pointers)
                }
            }
            _ => {}
        }
    }
    for fixture_name in ["method", "binding", "unpackaged", "unpackaged-excluded"] {
        let doc = fixture(fixture_name);
        assert!(validator.is_valid(&doc));
        let mut pointers = Vec::new();
        visit(&doc, "", &mut pointers);
        for pointer in pointers {
            for wrong in [
                Value::Null,
                json!(-1),
                json!(false),
                json!([]),
                json!({}),
                json!("invalid"),
            ] {
                let mut candidate = doc.clone();
                *candidate.pointer_mut(&pointer).unwrap() = wrong;
                if !validator.is_valid(&candidate) {
                    assert!(
                        render(&serde_json::to_vec(&candidate).unwrap(), "md").is_err(),
                        "accepted invalid {fixture_name}{pointer}: {}",
                        candidate.pointer(&pointer).unwrap()
                    );
                }
            }
            if let Some(map) = doc.pointer(&pointer).unwrap().as_object() {
                for key in map.keys().chain(std::iter::once(&"extra".to_string())) {
                    let mut candidate = doc.clone();
                    let obj = candidate
                        .pointer_mut(&pointer)
                        .unwrap()
                        .as_object_mut()
                        .unwrap();
                    if key == "extra" {
                        obj.insert(key.clone(), json!(true));
                    } else {
                        obj.remove(key);
                    }
                    if !validator.is_valid(&candidate) {
                        assert!(
                            render(&serde_json::to_vec(&candidate).unwrap(), "md").is_err(),
                            "accepted invalid field {fixture_name}{pointer}/{key}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn capped_sections_keep_a_prefix_while_later_sections_use_their_own_budget() {
    let mut doc = fixture("method");
    let mut huge = fixture("binding")["untested"][0].clone();
    let mut small = huge.clone();
    huge["symbol"] = json!("x".repeat(4000));
    small["symbol"] = json!("LaterUncovered");
    doc["untested"] = json!([huge, small]);
    let md = output(&doc, "md");
    assert!(md.contains("\n\n**2 more not shown**"));
    assert!(
        !md.contains("LaterUncovered"),
        "a section must not skip an earlier row to display a later row"
    );
    assert!(
        md.contains("- `ShapeService.totalArea(list<shape>)`"),
        "later sections have independent budgets"
    );
}

/// Regression case: the 40-class NPSP sample, with as many selected tests
/// as asked for. It had 59 groups of reasons, 77 changed symbols, 6 untested
/// and 135 possible tests, and rows about this long.
fn npsp_40_shaped(selected: usize) -> Value {
    let mut doc = fixture("method");
    let numbered = |row: &Value, count: usize, name: &str| {
        let rows = (0..count).map(|i| {
            let mut row = row.clone();
            row["symbol"] = json!(format!("{name}{i:03}"));
            row["file"] = json!(format!("force-app/main/default/classes/{name}{i:03}.cls"));
            row
        });
        json!(rows.collect::<Vec<_>>())
    };
    doc["gaps"] = json!(
        (1..=59)
            .map(|i| json!({"reason":"unresolved","file":format!("force-app/main/default/classes/Service{i:02}.cls"),"line":i,"source":"refs"}))
            .collect::<Vec<_>>()
    );
    doc["tests"] = numbered(&doc["tests"][0], selected, "Selected_TEST");
    doc["seeds"] = numbered(&doc["seeds"][0], 77, "Changed");
    doc["untested"] = numbered(&fixture("binding")["untested"][0], 6, "Uncovered");
    let mut possible = fixture("unpackaged")["tests"][0].clone();
    possible["reasons"] = json!(["textual_path"]);
    doc["possible"] = numbered(&possible, 135, "Possible_TEST");
    doc["references"].as_object_mut().unwrap().extend(
        fixture("unpackaged")["references"]
            .as_object()
            .unwrap()
            .clone(),
    );
    doc
}
/// A section of the comment: from its heading to the heading after it.
fn section<'a>(md: &'a str, heading: &str, next: &str) -> &'a str {
    &md[md.find(heading).unwrap()..md.find(next).unwrap()]
}
fn listed(section: &str) -> usize {
    section.lines().filter(|l| l.starts_with("- ")).count()
}
#[test]
fn lower_sections_give_up_rows_from_the_last_before_a_selected_test_is_left_out() {
    let md = output(&npsp_40_shaped(260), "md");
    assert!(md.len() <= COMMENT_BUDGET);
    let selected = section(&md, "### Selected tests", "### Untested");
    assert_eq!(listed(selected), 260);
    assert!(!selected.contains("more not shown"));

    // With one selected test every lower section has the room of its own cap.
    let roomy = output(&npsp_40_shaped(1), "md");
    for (heading, next) in [
        ("### Untested", "### Changed symbols"),
        ("### Changed symbols", "<summary>Possible"),
        ("<summary>Possible", "<summary>Direct referrers"),
    ] {
        assert_eq!(
            section(&md, heading, next),
            section(&roomy, heading, next),
            "{heading} gave up rows before the referrers below it"
        );
    }
    // Each of the 77 changed symbols has the fixture's three referrers.
    let referrers = section(&md, "<summary>Direct referrers", "\nFull evidence");
    let at_cap = section(&roomy, "<summary>Direct referrers", "\nFull evidence");
    assert!(listed(referrers) < listed(at_cap));
    assert!(referrers.contains(&format!(
        "\n\n**{} more not shown**",
        231 - listed(referrers)
    )));
}
#[test]
fn selected_tests_are_left_out_only_when_they_alone_do_not_fit() {
    let md = output(&npsp_40_shaped(600), "md");
    assert!(md.len() <= COMMENT_BUDGET);
    let selected = section(&md, "### Selected tests", "### Untested");
    let row = selected.lines().find(|l| l.starts_with("- ")).unwrap();
    assert!(selected.contains(&format!(
        "\n\n**{} more not shown**",
        600 - listed(selected)
    )));
    // What the lower sections still list, with what is unused, has no room
    // for another selected test. Each says what it does not show.
    let lower = &md[md.find("### Untested").unwrap()..];
    let kept: usize = lower
        .lines()
        .filter(|l| l.starts_with("- "))
        .map(|l| l.len() + 1)
        .sum();
    assert!(
        kept + COMMENT_BUDGET - md.len() <= row.len(),
        "another selected test fits"
    );
    for heading in [
        "### Untested changed symbols (6)\n",
        "### Changed symbols (77)\n",
        "<summary>Possible tests (135) — not selected</summary>\n",
        "<summary>Direct referrers (parsed first)</summary>\n",
    ] {
        assert!(lower.contains(heading), "{heading}");
    }
    assert_eq!(lower.matches("more not shown").count(), 4);
    assert_eq!(
        md.matches("<details>").count(),
        md.matches("</details>").count()
    );
}
#[test]
fn an_out_that_is_a_link_is_refused_and_what_it_names_is_left_as_it_was() {
    let bytes = serde_json::to_vec(&fixture("method")).unwrap();
    let dir = std::env::temp_dir().join(format!("sf-impact-render-link-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let victim = dir.join("victim");
    fs::write(&victim, "a file outside\n").unwrap();
    let link = dir.join("comment.md");
    std::os::unix::fs::symlink(&victim, &link).unwrap();
    let args = ["--impact", "-", "--format", "md", "--out"];

    let result = cli(&bytes, &[&args[..], &[link.to_str().unwrap()]].concat());

    assert_eq!(result.status.code(), Some(1), "{result:?}");
    let said = String::from_utf8(result.stderr).unwrap();
    assert!(said.contains("is not a regular file"), "{said}");
    assert_eq!(fs::read_to_string(&victim).unwrap(), "a file outside\n");
    // The control: a path that is no link is written.
    let out = dir.join("written.md");
    let result = cli(&bytes, &[&args[..], &[out.to_str().unwrap()]].concat());
    assert!(result.status.success(), "{result:?}");
    assert_eq!(
        fs::read_to_string(&out).unwrap(),
        output(&fixture("method"), "md")
    );
    fs::remove_dir_all(&dir).unwrap();
}
#[test]
fn a_message_on_standard_error_writes_no_terminal_escape() {
    let name = "missing\u{1b}]8;;https://example.invalid\u{7}\u{1b}[2K\u{9b}.json";

    let result = cli(b"", &["--impact", name, "--format", "md"]);

    assert_eq!(result.status.code(), Some(1), "{result:?}");
    let said = String::from_utf8(result.stderr).unwrap();
    assert!(
        said.contains("missing\\u{1b}]8;;https://example.invalid\\u{7}"),
        "{said}"
    );
    assert!(!said.trim_end().chars().any(char::is_control), "{said:?}");
}
