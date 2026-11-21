//! A shape the extractor does not model reaches the
//! impact document. In a changed Flow an unsupported_element is a gap of
//! that file, as a parse_error is; in a Flow that did not change it is only
//! counted. A JS parse_error makes the changed bundle file a gap.
use serde_json::Value;
use std::{env, fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
}

/// The impact document for one file changed from `before` to `after`, in a
/// project with one Apex test so that the run has something to select and
/// the `unchanged` files, which are the same at both commits.
fn impact(case: &str, unchanged: &[(&str, &str)], path: &str, before: &str, after: &str) -> Value {
    let root = env::temp_dir().join(format!("sf-impact-loud-{}-{case}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    let write = |path: &str, text: &str| {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    };
    write(
        "sfdx-project.json",
        r#"{"packageDirectories":[{"path":"app"}]}"#,
    );
    write(
        "app/classes/ExampleTest.cls",
        "@IsTest\nprivate class ExampleTest {\n@IsTest static void runs() {}\n}\n",
    );
    for (path, text) in unchanged {
        write(path, text);
    }
    write(path, before);
    git(&root, &["init", "--quiet", "--initial-branch=main"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "base"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    write(path, after);
    git(&root, &["add", "."]);
    git(&root, &["commit", "-qm", "head"]);
    let refs = sf_impact_extract::extract_kept(&root, 1, false).unwrap().0;
    let delta = sf_impact_delta::delta(
        &root,
        false,
        &sf_impact_delta::Base::Commit(base.trim().into()),
        "HEAD",
    )
    .unwrap()
    .join("\n")
        + "\n";
    let changed = sf_impact_changed::changed(delta.as_bytes(), refs.as_slice(), false)
        .unwrap()
        .join("\n")
        + "\n";
    fs::write(root.join("refs.pack"), refs).unwrap();
    fs::write(root.join("changed.jsonl"), changed).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sf-impact-tests"))
        .current_dir(&root)
        .args(["--refs", "refs.pack", "--changed", "changed.jsonl"])
        .output()
        .unwrap();
    fs::remove_dir_all(&root).unwrap();
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap()
}

const FLOW: &str = "app/flows/Example.flow-meta.xml";

/// The Flow before each change: only control metadata, nothing unmodelled.
const PLAIN: &str = "<Flow>\n<label>Example</label>\n</Flow>\n";

fn unsupported(impact: &Value) -> &Value {
    &impact["diagnostics"]["unsupported_element"]
}

#[test]
fn plain_flow_change_counts_no_unsupported_element() {
    let after = PLAIN.replace("Example<", "Renamed<");
    assert!(unsupported(&impact("plain", &[], FLOW, PLAIN, &after)).is_null());
}

#[test]
fn flow_shapes_that_are_not_modelled_make_the_changed_flow_a_gap() {
    for (case, element) in [
        (
            "wrapper",
            "<futureMap><assignToFieldReference>Industry</assignToFieldReference></futureMap>",
        ),
        (
            "style",
            "<screens><fields><name>Body</name><styleProperties><futureBorder><stringValue>thin</stringValue></futureBorder></styleProperties></fields></screens>",
        ),
        ("leaf", "<sourceTemplate>ns__Template_Flow</sourceTemplate>"),
    ] {
        let after = PLAIN.replace("</Flow>", &format!("{element}\n</Flow>"));
        let out = impact(case, &[], FLOW, PLAIN, &after);
        assert_eq!(unsupported(&out), 1, "{case}: {out}");
        assert_eq!(out["confidence"], "partial", "{case}: {out}");
        let gaps = out["gaps"].as_array().unwrap();
        assert!(
            gaps.iter().any(|g| g["file"] == FLOW
                && g["reason"] == "unsupported_element"
                && g["source"] == "changed"),
            "{case}: {out}"
        );
    }
}

#[test]
fn type_only_reexport_makes_the_changed_bundle_file_a_gap() {
    let path = "app/lwc/example/example.js";
    let out = impact(
        "reexport",
        &[],
        path,
        "export default class Example {}\n",
        "export type { U } from 'c/other';\nexport default class Example {}\n",
    );
    let gaps = out["gaps"].as_array().unwrap();
    assert!(
        gaps.iter()
            .any(|g| g["file"] == path && g["reason"] == "parse_error"),
        "{out}"
    );
}

#[test]
fn unsupported_element_in_a_flow_that_did_not_change_is_only_counted() {
    let other = (
        "app/flows/Other.flow-meta.xml",
        "<Flow>\n<label>Other</label>\n<sourceTemplate>ns__Template_Flow</sourceTemplate>\n</Flow>\n",
    );
    let after = PLAIN.replace("Example<", "Renamed<");
    let out = impact("unchanged", &[other], FLOW, PLAIN, &after);
    assert_eq!(unsupported(&out), 1, "{out}");
    assert_eq!(out["confidence"], "full", "{out}");
    assert_eq!(out["gaps"], serde_json::json!([]), "{out}");
}
