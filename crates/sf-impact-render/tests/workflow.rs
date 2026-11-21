//! Execute the documented shell steps with controlled inputs and a Salesforce stub.
use std::{fs, path::PathBuf, process::Command};
fn render_doc() -> String {
    fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/render.md"))
        .unwrap()
}
fn script(name: &str) -> String {
    let doc = render_doc();
    let section = doc
        .split("```yaml\nname: Salesforce impact example")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    assert!(
        !section.lines().any(|line| line.starts_with("    if:")),
        "a fork must not skip the whole job"
    );
    assert!(!section.contains("pull_request_target"));
    assert!(!section.contains("actions/cache"));
    let step = section
        .split(&format!("      - name: {name}\n"))
        .nth(1)
        .expect("documented step")
        .split("      - ")
        .next()
        .unwrap();
    step.split("        run: |\n")
        .nth(1)
        .unwrap()
        .lines()
        .map(|l| format!("{}\n", l.strip_prefix("          ").unwrap_or(l)))
        .collect()
}
fn dir(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("sf-impact-workflow-{}-{name}", std::process::id()));
    fs::create_dir_all(&p).unwrap();
    p
}
#[test]
fn fork_job_runs_and_fails_visibly() {
    let out = Command::new("bash")
        .args(["-e", "-c", &script("Reject fork pull requests visibly")])
        .env("HEAD_REPO", "fork/project")
        .env("BASE_REPO", "owner/project")
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Fork PR"));
}
#[test]
fn rendering_failure_writes_a_fresh_head_stamped_comment_and_fails() {
    let p = dir("fallback");
    fs::write(p.join("impact.json"), "{}").unwrap();
    fs::write(p.join("comment.md"), "stale comment").unwrap();
    let bin = PathBuf::from(env!("CARGO_BIN_EXE_sf-impact-render"));
    let path = format!(
        "{}:{}",
        bin.parent().unwrap().display(),
        std::env::var("PATH").unwrap()
    );
    let out = Command::new("bash")
        .args(["-e", "-c", &script("Render")])
        .env("PATH", path)
        .env("RUNNER_TEMP", &p)
        .env("GITHUB_OUTPUT", p.join("outputs"))
        .env("HEAD_SHA", "abc123")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let body = fs::read_to_string(p.join("comment.md")).unwrap();
    assert!(body.starts_with("<!-- sf-impact -->"));
    assert!(body.contains("abc123") && body.contains("render failed: see artifact"));
    // A failed step still posts. A cancelled run was superseded by a newer
    // head and must not post "render failed" for the old one.
    let r = render_doc();
    assert!(r.contains(
        "      - name: Post or update the impact comment\n        if: ${{ !cancelled() && github.event.pull_request.head.repo.full_name == github.repository }}\n"
    ));
    fs::remove_dir_all(p).unwrap();
}
#[test]
fn workflow_validates_full_and_partial_skips_out_of_scope_and_fails_uncovered() {
    let script = script("Apply this example's gate and validate the selected tests");
    let p = dir("gate");
    fs::write(
        p.join("sf"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" >> \"$RUNNER_TEMP/sf-args\"\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(p.join("sf"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    for (selection, confidence, in_scope, success, called) in [
        ("selected", "full", true, true, true),
        ("selected", "partial", true, true, true),
        ("empty", "full", false, true, false),
        ("empty", "full", true, false, false),
    ] {
        let _ = fs::remove_file(p.join("sf-args"));
        fs::write(
            p.join("changed.jsonl"),
            format!(
                "{{\"row\":\"file\",\"package\":{},\"file\":\"force-app/Uncovered.cls\"}}\n",
                if in_scope { "\"force-app\"" } else { "null" }
            ),
        )
        .unwrap();
        fs::write(
            p.join("impact.json"),
            r#"{"untested":[{"symbol":"Uncovered","file":"force-app/Uncovered.cls"}]}"#,
        )
        .unwrap();
        let out = Command::new("bash")
            .args(["-e", "-c", &script])
            .env("RUNNER_TEMP", &p)
            .env("SELECTION", selection)
            .env("CONFIDENCE", confidence)
            .env(
                "TESTS",
                if called {
                    "ServiceTest,AnotherTest"
                } else {
                    ""
                },
            )
            .env("SF_INSTANCE_URL", "https://example.com")
            .env("SF_ACCESS_TOKEN", "stub")
            .env(
                "PATH",
                format!("{}:{}", p.display(), std::env::var("PATH").unwrap()),
            )
            .output()
            .unwrap();
        let log = String::from_utf8_lossy(&out.stdout);
        assert_eq!(
            out.status.success(),
            success,
            "{selection} {confidence}: {out:?}"
        );
        assert_eq!(p.join("sf-args").exists(), called);
        if called {
            let args = fs::read_to_string(p.join("sf-args")).unwrap();
            assert!(args.contains("RunSpecifiedTests\n--tests\nServiceTest\nAnotherTest\n"));
        } else if success {
            assert!(log.contains("No Apex/metadata in scope"));
        } else {
            assert!(log.contains("Uncovered") && log.contains("force-app/Uncovered.cls"));
        }
    }
    fs::remove_dir_all(p).unwrap();
}
