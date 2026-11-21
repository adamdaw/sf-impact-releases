//! Real fixture project -> extract -> delta -> changed -> impact binary.
#[path = "../../sf-impact-common/tests/support/pack.rs"]
mod pack;
#[path = "../../sf-impact-common/tests/support/schema.rs"]
mod schema;
use serde_json::{Value, json};
use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
struct Project(PathBuf);
impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_COMMITTER_DATE", "2000-01-01T00:00:00Z")
        .env("GIT_AUTHOR_NAME", "Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.com")
        .env("GIT_COMMITTER_NAME", "Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.com")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    String::from_utf8(out.stdout).unwrap()
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
fn pipeline(case: &str, path: &str, before: &str, after: &str, config: Option<Value>) -> Value {
    let project =
        Project(env::temp_dir().join(format!("sf-impact-tests-{}-{case}", std::process::id())));
    fs::create_dir_all(&project.0).unwrap();
    if case == "binding" {
        // Keep this golden about one changed binding. The other pipelines use
        // the complete project; this one copies the relevant existing files.
        let fixture = repository().join("fixtures/project");
        for path in [
            "force-app/declarative/classes/ShipmentTracker.cls",
            "force-app/declarative/classes/ShipmentAudit.cls",
            "force-app/declarative/customMetadata/Shipping_Plugin.Tracking.md-meta.xml",
            "force-app/declarative/objects/Shipping_Plugin__mdt/Shipping_Plugin__mdt.object-meta.xml",
            "force-app/declarative/objects/Shipping_Plugin__mdt/fields/Handler__c.field-meta.xml",
        ] {
            let target = project.0.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(fixture.join(path), target).unwrap();
        }
        // Binding resolution needs class declarations, not the unrelated
        // SOQL/DML examples in ShipmentTracker's body.
        for class in ["ShipmentTracker", "ShipmentAudit"] {
            let path = project
                .0
                .join(format!("force-app/declarative/classes/{class}.cls"));
            let text = fs::read_to_string(&path).unwrap();
            let header = text
                .lines()
                .find(|line| line.contains("public with sharing class"))
                .unwrap();
            fs::write(path, format!("{header}\n}}\n")).unwrap();
        }
        fs::write(
            project.0.join("sfdx-project.json"),
            r#"{"packageDirectories":[{"path":"force-app"}]}"#,
        )
        .unwrap();
    } else {
        copy(&repository().join("fixtures/project"), &project.0);
    }
    git(&project.0, &["init", "--quiet", "--initial-branch=main"]);
    git(&project.0, &["add", "."]);
    git(&project.0, &["commit", "-qm", "base"]);
    let base = git(&project.0, &["rev-parse", "HEAD"]);
    let file = project.0.join(path);
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains(before));
    fs::write(file, text.replace(before, after)).unwrap();
    git(&project.0, &["add", "."]);
    git(&project.0, &["commit", "-qm", "head"]);
    let refs = sf_impact_extract::extract_kept(&project.0, 1, false)
        .unwrap()
        .0;
    let delta = sf_impact_delta::delta(
        &project.0,
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
    fs::write(project.0.join("refs.pack"), refs).unwrap();
    fs::write(project.0.join("changed.jsonl"), changed).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_sf-impact-tests"));
    command
        .current_dir(&project.0)
        .args(["--refs", "refs.pack", "--changed", "changed.jsonl"]);
    if let Some(config) = config {
        fs::write(project.0.join("config.json"), config.to_string()).unwrap();
        command.args(["--config", "config.json"]);
    }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{output:?}");
    // impact.json is compact: one line, and its only newline ends it.
    assert_eq!(
        output.stdout.iter().position(|&b| b == b'\n'),
        Some(output.stdout.len() - 1),
        "impact.json is not one compact line"
    );
    let actual: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        actual["seeds"].as_array().is_some_and(|s| !s.is_empty()),
        "{actual}"
    );
    let schema = schema::Schema::parse(
        &fs::read_to_string(repository().join("schema/impact.schema.json")).unwrap(),
    );
    assert_eq!(schema.errors(&actual), Vec::<String>::new());
    let golden = repository().join(format!("fixtures/impact/{case}.json"));
    if env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&golden, &output.stdout).unwrap();
    }
    let expected: Value = serde_json::from_slice(&fs::read(golden).unwrap()).unwrap();
    assert_eq!(schema.errors(&expected), Vec::<String>::new());
    assert_eq!(actual, expected);
    actual
}
#[test]
fn method_change_selects_a_test_with_its_evidence() {
    let out = pipeline(
        "method",
        "force-app/main/default/classes/ShapeService.cls",
        "Decimal total = 0;",
        "Decimal total = 1;",
        None,
    );
    assert!(
        out["tests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["symbol"] == "ShapeServiceTest")
    );
}
#[test]
fn metadata_binding_change_seeds_the_bound_class_and_cmdt_type() {
    let out = pipeline(
        "binding",
        "force-app/declarative/customMetadata/Shipping_Plugin.Tracking.md-meta.xml",
        "shipmenttracker",
        "ShipmentAudit",
        None,
    );
    let seeds = out["seeds"].as_array().unwrap();
    assert_eq!(
        seeds
            .iter()
            .map(|s| s["symbol"].as_str().unwrap())
            .collect::<Vec<_>>(),
        [
            "ShipmentAudit",
            "ShipmentTracker",
            "Shipping_Plugin.Tracking",
            "Shipping_Plugin__mdt",
            "Shipping_Plugin__mdt.Handler__c"
        ]
    );
    assert_eq!(out["tests"], json!([]));
    assert_eq!(out["possible"], json!([]));
    let audit = &seeds[0];
    assert_eq!(audit["counts"]["parsed"], 1);
    assert!(
        audit["referrers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["type"] == "metadata_record"
                && g["parsed"][0]["referrer"]["symbol"] == "Shipping_Plugin.Tracking")
    );
    assert_eq!(out["confidence"], "partial");
    assert!(
        out["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|g| g["reason"] == "dynamic_binding")
    );
}
#[test]
fn unpackaged_test_is_selected_from_extracted_provenance() {
    let out = pipeline(
        "unpackaged",
        "plugins/Billing/classes/BillingPlugin.cls",
        "public class BillingPlugin",
        "public with sharing class BillingPlugin",
        None,
    );
    assert!(
        out["tests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["symbol"] == "ExtraTests" && t["scope"] == "unpackagedMetadata")
    );
}
#[test]
fn config_excludes_the_same_unpackaged_test() {
    let out = pipeline(
        "unpackaged-excluded",
        "plugins/Billing/classes/BillingPlugin.cls",
        "public class BillingPlugin",
        "public with sharing class BillingPlugin",
        Some(json!({"excludeUnpackagedTests":true})),
    );
    assert!(
        !out["tests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["symbol"] == "ExtraTests")
    );
}
#[test]
fn a_working_tree_that_is_not_its_commit_goes_through_only_when_accepted_and_says_so() {
    let project =
        Project(env::temp_dir().join(format!("sf-impact-tests-{}-dirty", std::process::id())));
    copy(&repository().join("fixtures/project"), &project.0);
    git(&project.0, &["init", "--quiet", "--initial-branch=main"]);
    git(&project.0, &["add", "."]);
    git(&project.0, &["commit", "-qm", "base"]);
    let class = project
        .0
        .join("force-app/main/default/classes/ShapeService.cls");
    let text = fs::read_to_string(&class).unwrap();
    fs::write(
        &class,
        text.replace("Decimal total = 0;", "Decimal total = 1;"),
    )
    .unwrap();
    git(&project.0, &["add", "."]);
    git(&project.0, &["commit", "-qm", "head"]);
    // After the head: a change that no commit has.
    fs::write(
        &class,
        text.replace("Decimal total = 0;", "Decimal total = 2;"),
    )
    .unwrap();
    let refs = sf_impact_extract::extract_kept(&project.0, 1, false)
        .unwrap()
        .0;
    let base = sf_impact_delta::Base::Commit("HEAD~1".into());
    let delta = sf_impact_delta::delta(&project.0, false, &base, "HEAD")
        .unwrap()
        .join("\n")
        + "\n";
    let refused = sf_impact_changed::changed(delta.as_bytes(), refs.as_slice(), false).unwrap_err();
    assert!(refused.contains("`dirty`"), "{refused}");
    let changed = sf_impact_changed::changed(delta.as_bytes(), refs.as_slice(), true).unwrap();
    fs::write(project.0.join("refs.pack"), refs).unwrap();
    fs::write(project.0.join("changed.jsonl"), changed.join("\n") + "\n").unwrap();
    let impact = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_sf-impact-tests"))
            .current_dir(&project.0)
            .args(["--refs", "refs.pack", "--changed", "changed.jsonl"])
            .args(extra)
            .output()
            .unwrap()
    };

    let refused = impact(&[]);
    let accepted = impact(&["--accept-unverified-refs"]);

    assert_eq!(refused.status.code(), Some(1), "{refused:?}");
    assert!(refused.stdout.is_empty(), "{refused:?}");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("--accept-unverified-refs"));
    assert!(accepted.status.success(), "{accepted:?}");
    let document: Value = serde_json::from_slice(&accepted.stdout).unwrap();
    let schema = schema::Schema::parse(
        &fs::read_to_string(repository().join("schema/impact.schema.json")).unwrap(),
    );
    assert_eq!(schema.errors(&document), Vec::<String>::new());
    assert_eq!(document["confidence"], "partial");
    let reasons = document["gaps"].as_array().unwrap().iter();
    let unverified: Vec<&Value> = reasons
        .filter(|r| r["reason"] == "unverified_refs")
        .collect();
    // Said by this stage of its refs, and by the stage before it of its own.
    assert_eq!(
        unverified,
        [
            &json!({"reason":"unverified_refs","file":"changed.jsonl","line":1,"source":"changed"}),
            &json!({"reason":"unverified_refs","file":"refs.jsonl","line":1,"source":"refs"}),
        ]
    );
    assert!(
        document["tests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["symbol"] == "ShapeServiceTest")
    );
}
#[test]
fn refs_of_another_project_of_the_same_repository_are_refused_at_both_stages() {
    // Two projects in one repository, each declaring `app` with a class
    // `Service` at the same path. Only the test of the second calls it.
    let repo = Project(env::temp_dir().join(format!(
        "sf-impact-tests-{}-two-projects",
        std::process::id()
    )));
    let service = |result: u32| {
        format!(
            "public class Service {{\n public static Integer beta() {{\n  return {result};\n }}\n}}\n"
        )
    };
    for (project, call) in [("one", "System.assert(true);"), ("two", "Service.beta();")] {
        let root = repo.0.join(project);
        fs::create_dir_all(root.join("app/classes")).unwrap();
        fs::write(
            root.join("sfdx-project.json"),
            r#"{"packageDirectories":[{"path":"app"}]}"#,
        )
        .unwrap();
        fs::write(root.join("app/classes/Service.cls"), service(1)).unwrap();
        let test = format!(
            "@IsTest public class VerificationTest {{\n @IsTest static void checkIt() {{\n  {call}\n }}\n}}\n"
        );
        fs::write(root.join("app/classes/VerificationTest.cls"), test).unwrap();
    }
    git(&repo.0, &["init", "--quiet", "--initial-branch=main"]);
    git(&repo.0, &["add", "."]);
    git(&repo.0, &["commit", "-qm", "base"]);
    fs::write(repo.0.join("two/app/classes/Service.cls"), service(2)).unwrap();
    git(&repo.0, &["add", "."]);
    git(&repo.0, &["commit", "-qm", "head"]);
    let refs = |project: &str| {
        let extracted = sf_impact_extract::extract_kept(&repo.0.join(project), 1, false);
        extracted.unwrap().0
    };
    let (of_one, of_two) = (refs("one"), refs("two"));
    let base = sf_impact_delta::Base::Commit("HEAD~1".into());
    let delta = sf_impact_delta::delta(&repo.0.join("two"), false, &base, "HEAD")
        .unwrap()
        .join("\n")
        + "\n";

    // The stage that maps refuses the refs of the first project.
    let refused =
        sf_impact_changed::changed(delta.as_bytes(), of_one.as_slice(), false).unwrap_err();
    assert!(refused.contains("`root`"), "{refused}");
    assert!(
        refused.contains("is one in refs.jsonl and two in delta.jsonl"),
        "{refused}"
    );
    // And so does the stage that walks, given what was mapped onto the second.
    let changed = sf_impact_changed::changed(delta.as_bytes(), of_two.as_slice(), false).unwrap();
    fs::write(repo.0.join("changed.jsonl"), changed.join("\n") + "\n").unwrap();
    fs::write(repo.0.join("one.pack"), of_one).unwrap();
    fs::write(repo.0.join("two.pack"), of_two).unwrap();
    let impact = |refs: &str| {
        Command::new(env!("CARGO_BIN_EXE_sf-impact-tests"))
            .current_dir(&repo.0)
            .args(["--changed", "changed.jsonl", "--refs", refs])
            .output()
            .unwrap()
    };

    let (wrong, right) = (impact("one.pack"), impact("two.pack"));

    assert_eq!(wrong.status.code(), Some(1), "{wrong:?}");
    assert!(wrong.stdout.is_empty(), "{wrong:?}");
    let said = String::from_utf8(wrong.stderr).unwrap();
    assert_eq!(said.lines().count(), 1, "{said}");
    assert!(
        said.contains("is one in refs.jsonl and two in changed.jsonl"),
        "{said}"
    );
    assert!(right.status.success(), "{right:?}");
    let document: Value = serde_json::from_slice(&right.stdout).unwrap();
    let selected: Vec<&Value> = document["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| &t["symbol"])
        .collect();
    assert_eq!(selected, ["VerificationTest"]);
    assert_eq!(document["confidence"], "full");
}

#[test]
fn latin1_at_both_commits_is_binary_skipped_and_a_gap() {
    let project = Project(env::temp_dir().join(format!("sf-impact-latin1-{}", std::process::id())));
    fs::create_dir_all(project.0.join("app/classes")).unwrap();
    fs::write(
        project.0.join("sfdx-project.json"),
        r#"{"packageDirectories":[{"path":"app"}]}"#,
    )
    .unwrap();
    let path = "app/classes/Latin.cls";
    let mut bytes = b"public class Latin { // ".to_vec();
    bytes.extend(0x80..=0xff);
    bytes.extend(b"\n}\n");
    fs::write(project.0.join(path), &bytes).unwrap();
    git(&project.0, &["init", "--quiet", "--initial-branch=main"]);
    git(&project.0, &["add", "."]);
    git(&project.0, &["commit", "-qm", "base"]);
    let base = git(&project.0, &["rev-parse", "HEAD"]);
    bytes.extend(b"// head\n");
    fs::write(project.0.join(path), bytes).unwrap();
    git(&project.0, &["add", "."]);
    git(&project.0, &["commit", "-qm", "head"]);
    let refs = sf_impact_extract::extract(&project.0, 1, false).unwrap();
    let rows: Vec<Value> = refs
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert!(
        rows.iter()
            .any(|row| row["row"] == "skipped_file" && row["file"] == path),
        "{rows:?}"
    );
    let delta = sf_impact_delta::delta(
        &project.0,
        false,
        &sf_impact_delta::Base::Commit(base.trim().into()),
        "HEAD",
    )
    .unwrap();
    let rows: Vec<Value> = delta
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let file = rows.iter().find(|row| row["row"] == "file").unwrap();
    assert_eq!(file["base"]["binary"], true);
    assert_eq!(file["head"]["binary"], true);
    let refs = pack::of_jsonl(&(refs.join("\n") + "\n"));
    let delta = delta.join("\n") + "\n";
    let changed = sf_impact_changed::changed(delta.as_bytes(), refs.as_slice(), false)
        .unwrap()
        .join("\n")
        + "\n";
    fs::write(project.0.join("refs.pack"), refs).unwrap();
    fs::write(project.0.join("changed.jsonl"), changed).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sf-impact-tests"))
        .current_dir(&project.0)
        .args(["--refs", "refs.pack", "--changed", "changed.jsonl"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let impact: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(
        impact["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|gap| gap.to_string().contains(path)),
        "{impact}"
    );
}

#[test]
fn delta_and_extract_agree_on_encoding_boundaries() {
    let project =
        Project(env::temp_dir().join(format!("sf-impact-encodings-{}", std::process::id())));
    fs::create_dir_all(project.0.join("app")).unwrap();
    fs::write(
        project.0.join("sfdx-project.json"),
        r#"{"packageDirectories":[{"path":"app"}]}"#,
    )
    .unwrap();
    let cases: [(&str, &[u8], bool); 13] = [
        ("le", &[0xff, 0xfe, 0xe9, 0], true),
        ("be", &[0xfe, 0xff, 0, 0xe9], true),
        ("unmarked-le", &[0xe9, 0], false),
        ("unmarked-be", &[0, 0xe9], false),
        ("invalid-le", &[0xff, 0xfe, 0, 0xd8], false),
        ("invalid-be", &[0xfe, 0xff, 0xd8, 0], false),
        ("odd-le", &[0xff, 0xfe, 0x43], false),
        ("odd-be", &[0xfe, 0xff, 0x43], false),
        ("nul-utf8", b"Circle\0", false),
        ("nul-le", &[0xff, 0xfe, 0x43, 0, 0, 0], false),
        ("nul-be", &[0xfe, 0xff, 0, 0x43, 0, 0], false),
        ("utf32-le", &[0xff, 0xfe, 0, 0, 0x43, 0, 0, 0], false),
        (
            "mangled-png",
            b"\xef\xbf\xbdPNG\r\n\x1a\n\0\0\0\rIHDR",
            false,
        ),
    ];
    git(&project.0, &["init", "--quiet", "--initial-branch=main"]);
    git(&project.0, &["add", "."]);
    git(&project.0, &["commit", "-qm", "base"]);
    let base = git(&project.0, &["rev-parse", "HEAD"]);
    for (name, bytes, _) in cases {
        fs::write(project.0.join(format!("app/{name}.txt")), bytes).unwrap();
    }
    git(&project.0, &["add", "."]);
    git(&project.0, &["commit", "-qm", "head"]);
    let refs: Vec<Value> = sf_impact_extract::extract(&project.0, 1, false)
        .unwrap()
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let delta: Vec<Value> = sf_impact_delta::delta(
        &project.0,
        false,
        &sf_impact_delta::Base::Commit(base.trim().into()),
        "HEAD",
    )
    .unwrap()
    .iter()
    .map(|line| serde_json::from_str(line).unwrap())
    .collect();
    let refs_pack = pack::of_jsonl(
        &(refs
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"),
    );
    let delta_jsonl = delta
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let changed = sf_impact_changed::changed(delta_jsonl.as_bytes(), refs_pack.as_slice(), false)
        .unwrap()
        .join("\n")
        + "\n";
    fs::write(project.0.join("refs.pack"), refs_pack).unwrap();
    fs::write(project.0.join("changed.jsonl"), changed).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sf-impact-tests"))
        .current_dir(&project.0)
        .args(["--refs", "refs.pack", "--changed", "changed.jsonl"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let impact: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(impact["diagnostics"]["skipped_file"], 11);
    for (name, _, is_text) in cases {
        let path = format!("app/{name}.txt");
        let file = delta
            .iter()
            .find(|row| row["row"] == "file" && row["head"]["file"] == path)
            .unwrap();
        if !is_text {
            assert!(
                impact["gaps"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|gap| gap["file"] == path),
                "{name}: {impact}"
            );
        }
        assert_eq!(file["head"]["binary"], !is_text, "{name}");
        assert_eq!(
            refs.iter()
                .any(|row| row["row"] == "skipped_file" && row["file"] == path),
            !is_text,
            "{name}"
        );
        assert_eq!(
            delta
                .iter()
                .any(|row| row["row"] == "blob" && row["id"] == file["head"]["id"]),
            is_text,
            "{name}"
        );
    }
}
