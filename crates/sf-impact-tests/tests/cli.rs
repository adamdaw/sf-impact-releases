#[path = "../../sf-impact-common/tests/support/pack.rs"]
mod pack;
mod support;
use serde_json::Value;
use std::{
    io::Write,
    process::{Command, Stdio},
};
fn run(args: &[&str], input: impl AsRef<[u8]>) -> std::process::Output {
    let child = Command::new(env!("CARGO_BIN_EXE_sf-impact-tests"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    support::feed_and_wait(child, input.as_ref())
}
#[test]
fn cli_requires_exactly_one_seed_input_and_rejects_bad_depth() {
    for args in [
        vec![],
        vec!["--refs", "-"],
        vec!["--refs", "-", "--changed", "-"],
        vec!["--refs", "-", "--symbol", "X", "--changed", "x"],
        vec!["--refs", "-", "--symbol", "X", "--depth", "-1"],
        vec!["--refs", "-", "--symbol", "X", "--wat"],
    ] {
        let out = run(&args, "");
        assert!(!out.status.success(), "{args:?}");
        assert!(!out.stderr.is_empty());
    }
}
#[test]
fn unknown_symbol_writes_an_error_document_and_exits_unsuccessfully() {
    let refs = pack::pack(&support::refs_header(), &[]);
    let out = run(&["--refs", "-", "--symbol", "Missing"], &refs);
    assert!(!out.status.success());
    let document: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(document["errors"][0]["name"], "Missing");
    assert_eq!(document["depth"], 2);
}
#[test]
fn multiple_names_and_stdout_work_without_a_project() {
    let input = r#"{"row":"declaration","symbol":"A","name":"A","type":"class","package":"app","file":"app/A.cls","start":1,"end":1,"is_test":false,"annotations":[],"collision":false,"scope":"packageDirectories"}
{"row":"declaration","symbol":"B","name":"B","type":"class","package":"app","file":"app/B.cls","start":1,"end":1,"is_test":false,"annotations":[],"collision":false,"scope":"packageDirectories"}
"#;
    let input = pack::of_jsonl(&format!("{}\n{input}", support::refs_header()));
    let out = run(
        &[
            "--refs", "-", "--symbol", "a", "b", "--depth", "0", "--out", "stdout",
        ],
        &input,
    );
    assert!(out.status.success(), "{out:?}");
    let document: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(document["seeds"].as_array().unwrap().len(), 2);
    assert_eq!(document["depth"], 0);
}
#[test]
fn refs_that_are_no_pack_are_refused_with_their_reason() {
    // refs.jsonl, as --format jsonl writes it: line 1 is read, and the rest
    // is no pack.
    let refs = format!("{}\n{{}}\n", support::refs_header());
    let out = run(&["--refs", "-", "--symbol", "A"], &refs);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("refs pack"));
}

#[test]
fn equivalent_parameter_spellings_resolve_to_the_same_seed() {
    let refs = fixture_refs();
    let mut expected = None;
    for symbol in [
        "FlowInvoiceAction.run(List<Id>)",
        "FlowInvoiceAction.run(Id[])",
        "FlowInvoiceAction.run(System.List<Id>)",
    ] {
        let out = run(&["--refs", "-", "--symbol", symbol], &refs);
        assert!(out.status.success(), "{symbol}: {out:?}");
        let document: Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(document["seeds"].as_array().unwrap().len(), 1);
        assert_eq!(
            document["seeds"][0]["symbol"],
            "FlowInvoiceAction.run(list<id>)"
        );
        if let Some(expected) = &expected {
            assert_eq!(&document["seeds"], expected);
        } else {
            expected = Some(document["seeds"].clone());
        }
    }
}

/// The refs pack of the fixture project.
fn fixture_refs() -> Vec<u8> {
    pack::of_jsonl(include_str!("../../../fixtures/refs.jsonl"))
}
const FIXTURE_SYMBOL: &str = "Invoice";

/// The document the library writes for the fixture, byte for byte.
fn fixture_document() -> Vec<u8> {
    let mut bytes = Vec::new();
    let symbols = [sf_impact_common::types::normal_symbol(FIXTURE_SYMBOL)];
    let config = sf_impact_tests::Config::default();
    let ok = sf_impact_tests::write_impact(
        fixture_refs().as_slice(),
        None,
        &symbols,
        2,
        &config,
        false,
        &mut bytes,
    );
    assert_eq!(ok, Ok(true));
    bytes
}

#[test]
fn the_document_is_written_whole_to_out_and_to_stdout() {
    let expected = fixture_document();
    let directory =
        std::env::temp_dir().join(format!("sf-impact-tests-cli-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let out_file = directory.join("impact.json");
    let out_path = out_file.to_str().unwrap();

    let to_file = run(
        &["--refs", "-", "--symbol", FIXTURE_SYMBOL, "--out", out_path],
        fixture_refs(),
    );
    let written = std::fs::read(&out_file).unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
    let to_stdout = run(&["--refs", "-", "--symbol", FIXTURE_SYMBOL], fixture_refs());

    assert!(to_file.status.success(), "{to_file:?}");
    assert!(to_file.stdout.is_empty() && to_file.stderr.is_empty());
    assert!(to_stdout.status.success(), "{to_stdout:?}");
    assert!(to_stdout.stderr.is_empty());
    assert!(expected.len() > 8192, "larger than one write buffer");
    assert!(
        written == expected,
        "--out differs from the library's document"
    );
    assert!(
        to_stdout.stdout == expected,
        "stdout differs from the library's document"
    );
}

#[test]
fn a_reader_that_stops_early_is_not_an_error() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sf-impact-tests"))
        .args(["--refs", "-", "--symbol", FIXTURE_SYMBOL])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The reader is gone before a byte is written: the refs come after.
    drop(child.stdout.take());
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(&fixture_refs()).unwrap();
    drop(stdin);

    let out = child.wait_with_output().unwrap();

    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(out.stderr.is_empty(), "{out:?}");
}

#[test]
fn an_out_that_is_a_device_is_refused_with_its_reason() {
    // A document that would succeed, small enough to stay in the write
    // buffer: the failure is the flush's.
    let input = r#"{"row":"declaration","symbol":"A","name":"A","type":"class","package":"app","file":"app/A.cls","start":1,"end":1,"is_test":false,"annotations":[],"collision":false,"scope":"packageDirectories"}"#;
    let input = pack::of_jsonl(&format!("{}\n{input}\n", support::refs_header()));
    let args = ["--refs", "-", "--symbol", "A", "--depth", "0"];
    assert!(run(&args, &input).status.success());

    // A device is no file to replace: refused before anything is written.
    let out = run(&[&args[..], &["--out", "/dev/full"]].concat(), &input);

    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let said = String::from_utf8(out.stderr).unwrap();
    assert!(said.starts_with("sf-impact-tests: "), "{said}");
    assert!(said.contains("/dev/full: is not a regular file"), "{said}");
}

#[test]
fn an_out_or_a_config_that_is_a_link_is_refused_and_what_it_names_is_left_as_it_was() {
    let dir = std::env::temp_dir().join(format!("sf-impact-tests-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let victim = dir.join("victim");
    std::fs::write(&victim, "{}").unwrap();
    let link = dir.join("impact.json");
    std::os::unix::fs::symlink(&victim, &link).unwrap();
    let input = r#"{"row":"declaration","symbol":"A","name":"A","type":"class","package":"app","file":"app/A.cls","start":1,"end":1,"is_test":false,"annotations":[],"collision":false,"scope":"packageDirectories"}"#;
    let input = pack::of_jsonl(&format!("{}\n{input}\n", support::refs_header()));
    let args = ["--refs", "-", "--symbol", "A", "--depth", "0"];
    let (link, victim_path) = (link.to_str().unwrap(), victim.to_str().unwrap());

    let out = run(&[&args[..], &["--out", link]].concat(), &input);

    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let said = String::from_utf8(out.stderr).unwrap();
    assert!(said.contains("is not a regular file"), "{said}");
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "{}");
    // A config is read only if it is a regular file of its own.
    let out = run(&[&args[..], &["--config", link]].concat(), &input);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let said = String::from_utf8(out.stderr).unwrap();
    assert!(said.contains("is not a regular file"), "{said}");
    // The control: the file itself is read, and a path that is no link
    // is written.
    let written = dir.join("written.json");
    let written = written.to_str().unwrap();
    let out = run(
        &[&args[..], &["--config", victim_path, "--out", written]].concat(),
        &input,
    );
    assert!(out.status.success(), "{out:?}");
    assert!(std::fs::read(written).unwrap().starts_with(b"{"));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The tool in `dir`, as `run` runs it, and under a limit of no byte of a
/// file written if `no_file_bytes`: `sh` ignores the signal that the limit
/// would end the process with, so a write fails with an error instead.
fn run_in(
    dir: &std::path::Path,
    args: &[&str],
    input: impl AsRef<[u8]>,
    no_file_bytes: bool,
) -> std::process::Output {
    let tool = env!("CARGO_BIN_EXE_sf-impact-tests");
    let mut command = match no_file_bytes {
        true => {
            let mut sh = Command::new("sh");
            sh.args(["-c", "trap '' XFSZ; ulimit -f 0; exec \"$0\" \"$@\"", tool]);
            sh
        }
        false => Command::new(tool),
    };
    let child = command
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    support::feed_and_wait(child, input.as_ref())
}

#[test]
fn a_child_that_exits_before_reading_its_input_is_not_a_failure_of_the_helper() {
    // More than a pipe holds, so the write cannot finish before the exit.
    let input = vec![b'x'; 1 << 20];
    let child = Command::new("true")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let out = support::feed_and_wait(child, &input);

    assert!(out.status.success(), "{out:?}");
}

fn entries(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn a_refs_pack() -> Vec<u8> {
    let input = r#"{"row":"declaration","symbol":"A","name":"A","type":"class","package":"app","file":"app/A.cls","start":1,"end":1,"is_test":false,"annotations":[],"collision":false,"scope":"packageDirectories"}"#;
    pack::of_jsonl(&format!("{}\n{input}\n", support::refs_header()))
}

#[test]
fn an_out_below_a_link_is_refused_and_nothing_is_made_where_the_link_goes() {
    let dir = std::env::temp_dir().join(format!("sf-impact-tests-parent-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (work, outside) = (dir.join("work"), dir.join("outside"));
    std::fs::create_dir_all(work.join("real")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::os::unix::fs::symlink("../outside", work.join("parent")).unwrap();
    std::os::unix::fs::symlink("real", work.join("nested")).unwrap();
    let args = ["--refs", "-", "--symbol", "A", "--depth", "0", "--out"];
    let refs = a_refs_pack();
    let absolute = work.join("parent/impact.json");

    for out in [
        "parent/impact.json",
        "nested/impact.json",
        absolute.to_str().unwrap(),
    ] {
        let run = run_in(&work, &[&args[..], &[out]].concat(), &refs, false);

        assert_eq!(run.status.code(), Some(1), "{out}: {run:?}");
        let said = String::from_utf8(run.stderr).unwrap();
        assert!(
            said.contains("has a symbolic link in its directory"),
            "{said}"
        );
    }
    assert_eq!(entries(&outside), Vec::<String>::new());
    assert_eq!(entries(&work.join("real")), Vec::<String>::new());
    // The control: a directory that is no link is written, relative and
    // absolute, and the current directory may be reached through a link.
    let real = work.join("real");
    std::os::unix::fs::symlink("work", dir.join("via")).unwrap();
    let through = dir.join("via/real/b.json");
    for out in ["real/a.json", through.to_str().unwrap()] {
        let run = run_in(&work, &[&args[..], &[out]].concat(), &refs, false);
        assert!(run.status.success(), "{out}: {run:?}");
    }
    assert_eq!(entries(&real), ["a.json", "b.json"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn an_out_that_cannot_be_written_leaves_what_was_there_and_no_temporary_file() {
    let dir = std::env::temp_dir().join(format!("sf-impact-tests-full-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("impact.json"), "before").unwrap();
    let args = [
        "--refs",
        "-",
        "--symbol",
        "A",
        "--depth",
        "0",
        "--out",
        "impact.json",
    ];
    let refs = a_refs_pack();

    let run = run_in(&dir, &args, &refs, true);

    assert_eq!(run.status.code(), Some(1), "{run:?}");
    let said = String::from_utf8(run.stderr).unwrap();
    assert!(said.starts_with("sf-impact-tests: "), "{said}");
    assert!(said.contains("File too large"), "{said}");
    assert_eq!(std::fs::read(dir.join("impact.json")).unwrap(), b"before");
    assert_eq!(entries(&dir), ["impact.json"], "no temporary file is left");
    // The control: without the limit the same command replaces the file.
    let run = run_in(&dir, &args, &refs, false);
    assert!(run.status.success(), "{run:?}");
    assert!(
        std::fs::read(dir.join("impact.json"))
            .unwrap()
            .starts_with(b"{")
    );
    assert_eq!(entries(&dir), ["impact.json"]);
    std::fs::remove_dir_all(&dir).unwrap();
}
