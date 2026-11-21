//! CLI exit status, stdout digest and exact stderr captured from a pre-publication release build.
//!
//! As of schema 2 the refs are a pack. The inputs are the goldens' with the
//! refs' `schema` 2 wherever they hold it, and the document is held to its
//! golden digest after that one token is put back. A refs row that a pack
//! cannot hold is refused before the CLI runs, by the pack's writer, and
//! the earlier release refused it too, but for duplicate keys (T1) and for a file whose
//! rows name two packages, which a pack cannot hold.
#[path = "../../sf-impact-common/tests/support/golden.rs"]
mod golden;
#[path = "../../sf-impact-common/tests/support/pack.rs"]
mod pack;
mod support;
use serde_json::Value;
use sf_impact_common::pack::Pack;
use std::process::{Command, Stdio};

const SCHEMA_1: &str = r#""tool":"sf-impact-extract","version":"0.1.0","schema":1"#;
const SCHEMA_2: &str = r#""tool":"sf-impact-extract","version":"0.1.0","schema":2"#;

/// A first row with the refs it names of schema 2, its keys in the order
/// the goldens have them.
fn of_schema_2(header: &str, refs: &[&str]) -> String {
    let mut header: Value = serde_json::from_str(header).unwrap();
    let at = refs.iter().fold(&mut header, |at, key| &mut at[*key]);
    assert_eq!(at["schema"], 1);
    at["schema"] = 2.into();
    header.to_string()
}

#[test]
fn cli_matches_golden_outcomes_except_duplicate_keys() {
    let goldens = golden::load();
    let refs_header = of_schema_2(&goldens.refs_header, &[]);
    let changed_header = of_schema_2(&goldens.changed_header, &["inputs", "refs"]);
    let refs = format!("{refs_header}\n{}\n", goldens.declaration);
    let directory =
        std::env::temp_dir().join(format!("decoder-cli-goldens-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let refs_path = directory.join("refs.pack");
    std::fs::write(&refs_path, pack::of_jsonl(&refs)).unwrap();
    let (mut run, mut refused, mut two_packages) = (0, 0, 0);
    for case in &goldens.cases {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sf-impact-tests"));
        let input = if case.changed {
            command.args(["--refs", refs_path.to_str().unwrap(), "--changed", "-"]);
            format!("{changed_header}\n{}\n", case.line).into_bytes()
        } else {
            command.args(["--refs", "-", "--symbol", "fflib_Application"]);
            // The rows as they are written, in the order a pack keeps.
            let mut lines = [&goldens.declaration, &case.line];
            let file = |line: &str| {
                let row = serde_json::from_str::<Value>(line).unwrap_or_default();
                (
                    row["row"] != "scope_error",
                    row["file"].as_str().map(str::to_owned),
                )
            };
            lines.sort_by_key(|line| file(line));
            let text = format!("{refs_header}\n{}\n{}\n", lines[0], lines[1]);
            let packed = Pack::from_jsonl(&text).and_then(|pack| pack.encode());
            match packed {
                Ok(pack) => pack,
                Err(why) => {
                    refused += 1;
                    let row: Value = serde_json::from_str(&case.line).unwrap_or_default();
                    let declaration: Value = serde_json::from_str(&goldens.declaration).unwrap();
                    if let Some(field) = &case.duplicate {
                        // As the CLI refuses a duplicate key: T1's exception.
                        let duplicate = format!("duplicate field `{field}`");
                        assert!(why.contains(&duplicate), "{}: {why}", case.id);
                    } else if case.cli.status == 0
                        && row["file"] == declaration["file"]
                        && row["package"].is_string()
                        && row["package"] != declaration["package"]
                    {
                        // The one exception of schema 2: a pack holds each
                        // file once, of one package, which extract always
                        // writes; the earlier release read a file of two packages.
                        assert!(why.contains("not together in path order"), "{why}");
                        two_packages += 1;
                    } else {
                        // As the earlier release did: refused, and nothing written.
                        assert_eq!(case.cli.status, 1, "{}: {}", case.id, case.line);
                        assert_eq!(case.decoder.status, 1, "{}", case.id);
                    }
                    continue;
                }
            }
        };
        let child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = support::feed_and_wait(child, &input);
        let stdout = String::from_utf8(output.stdout).unwrap();
        case.check(
            &case.cli,
            output.status.code().unwrap(),
            stdout.replace(SCHEMA_2, SCHEMA_1).as_bytes(),
            &output.stderr,
        );
        run += 1;
    }
    std::fs::remove_dir_all(directory).unwrap();
    // Both kinds of case are there: those the CLI ran, and refs rows that
    // the pack's writer refused; four of those name a second package.
    assert!(run > 1988 && refused > 0, "{run} run, {refused} refused");
    assert_eq!(two_packages, 4);
}
