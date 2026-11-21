use serde_json::Value;
use std::{fs, process::Command};
/// Commits a service and the class that tests it, changes the service, and
/// renders the impact of that change: the document, `gh-step` and `md`.
fn rendered(case: &str, test_class: &str, test_source: &str) -> (Value, String, String) {
    let root = std::env::temp_dir().join(format!(
        "sf-impact-render-pipeline-{}-{case}",
        std::process::id()
    ));
    fs::create_dir_all(root.join("app")).unwrap();
    fs::write(
        root.join("sfdx-project.json"),
        r#"{"packageDirectories":[{"path":"app"}]}"#,
    )
    .unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&root)
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
    };
    fs::write(
        root.join("app/Service.cls"),
        "public class Service { public static Integer value() { return 1; } }\n",
    )
    .unwrap();
    fs::write(root.join(format!("app/{test_class}.cls")), test_source).unwrap();
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    fs::write(
        root.join("app/Service.cls"),
        "public class Service { public static Integer value() { return 2; } }\n",
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "head"]);
    let refs = sf_impact_extract::extract_kept(&root, 1, false).unwrap().0;
    let delta = sf_impact_delta::delta(
        &root,
        false,
        &sf_impact_delta::Base::Commit("HEAD~1".into()),
        "HEAD",
    )
    .unwrap()
    .join("\n")
        + "\n";
    let changed = sf_impact_changed::changed(delta.as_bytes(), refs.as_slice(), false)
        .unwrap()
        .join("\n")
        + "\n";
    let doc = sf_impact_tests::impact(
        refs.as_slice(),
        Some(changed.as_bytes()),
        &[],
        2,
        &sf_impact_tests::Config::default(),
        false,
    )
    .unwrap();
    let input = root.join("impact.json");
    fs::write(&input, serde_json::to_vec(&doc).unwrap()).unwrap();
    // No source tree or git remains available to the renderer.
    fs::remove_dir_all(root.join("app")).unwrap();
    fs::remove_dir_all(root.join(".git")).unwrap();
    let run = |format| {
        let out = Command::new(env!("CARGO_BIN_EXE_sf-impact-render"))
            .current_dir(&root)
            .env("PATH", "")
            .args(["--impact", input.to_str().unwrap(), "--format", format])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8(out.stdout).unwrap()
    };
    let (step, md) = (run("gh-step"), run("md"));
    fs::remove_dir_all(root).unwrap();
    (doc, step, md)
}
#[test]
fn committed_change_through_all_five_stages() {
    let (doc, step, md) = rendered(
        "top-level",
        "ServiceTest",
        "@IsTest public class ServiceTest { @IsTest static void checks() { System.assertEquals(2, Service.value()); } }\n",
    );
    assert_eq!(
        step,
        "tests=ServiceTest\nconfidence=full\nselected_count=1\npossible_count=0\nuntested_count=0\nselection=selected\n"
    );
    assert!(md.contains("| full |"));
    assert!(
        md.contains(
            doc["provenance"]["inputs"]["changed"]["head"]
                .as_str()
                .unwrap()
        )
    );
    assert!(md.contains("ServiceTest"));
    assert!(md.contains("<!-- sf-impact -->"));
}
// Not valid Apex: the platform rejects a nested @IsTest class. The tools read
// it all the same, and must not hand a test run a dotted name.
#[test]
fn an_inner_test_class_reaches_runspecifiedtests_as_its_top_level_class() {
    let (doc, step, md) = rendered(
        "inner",
        "ServiceChecks",
        "@IsTest public class ServiceChecks { @IsTest private class Doubled { Integer twice() { return 2 * Service.value(); } } }\n",
    );
    assert_eq!(
        step,
        "tests=ServiceChecks\nconfidence=full\nselected_count=1\npossible_count=0\nuntested_count=0\nselection=selected\n"
    );
    assert_eq!(
        doc["tests"][0]["path"][0]["referrer"]["symbol"],
        "ServiceChecks.Doubled.twice()"
    );
    assert!(
        md.contains("- `ServiceChecks` (`app/ServiceChecks.cls:1`): `direct`; seed `Service`\n"),
        "{md}"
    );
}
