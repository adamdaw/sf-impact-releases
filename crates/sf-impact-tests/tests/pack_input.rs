//! sf-impact-tests reads refs as the pack that sf-impact-extract writes:
//! a pack with a manifest as the same pack without one, and a pack that is
//! not as it was written, or not a pack, never.
#[path = "../../sf-impact-common/tests/support/pack.rs"]
mod pack;
mod support;
use serde_json::{Value, json};
use sf_impact_tests::{Config, impact};
use std::process::Command;

fn declaration(symbol: &str, is_test: bool) -> Value {
    json!({"row":"declaration","symbol":symbol,"name":symbol,"type":"class","package":"app",
        "file":format!("app/{symbol}.cls"),"start":1,"end":20,"is_test":is_test,
        "annotations":if is_test { json!(["IsTest"]) } else { json!([]) },"collision":false,
        "scope":"packageDirectories"})
}
fn refs() -> Vec<u8> {
    let reference = json!({"row":"reference","tier":"parsed","kind":"type","referenced":"Handler",
        "referenced_type":"class","resolution":"declaration","referrer":"HandlerTest",
        "package":"app","file":"app/HandlerTest.cls","line":5,"column":3});
    let rows = [
        declaration("Handler", false),
        declaration("HandlerTest", true),
        reference,
    ];
    pack::pack(&support::refs_header(), &rows)
}
fn changed() -> Vec<u8> {
    let symbol = json!({"row":"symbol","symbol":"Handler","type":"class","package":"app",
        "file":"app/Handler.cls","change":"modified","reason":"whole_file","collision":false});
    format!("{}\n{symbol}\n", support::changed_header()).into_bytes()
}
fn document(refs: &[u8]) -> Result<Value, String> {
    let changed = changed();
    impact(refs, Some(&changed), &[], 2, &Config::default(), false)
}

#[test]
fn a_pack_with_a_manifest_gives_the_document_that_the_same_pack_without_one_gives() {
    let plain = document(&refs()).unwrap();
    assert_eq!(plain["tests"][0]["symbol"], "HandlerTest", "{plain}");

    let with_manifest = document(&pack::with_manifest(&refs())).unwrap();

    assert_eq!(with_manifest, plain);
}

#[test]
fn a_malformed_manifest_is_refused_loudly() {
    let with_manifest = pack::with_manifest(&refs());
    assert!(document(&with_manifest).is_ok());

    for bad in [
        pack::two_manifests(&with_manifest),
        pack::corrupted(&with_manifest, 6, true),
    ] {
        let refused = document(&bad).unwrap_err();
        assert!(refused.contains("refs pack"), "{refused}");
    }
}

#[test]
fn a_pack_that_is_not_as_it_was_written_is_refused_loudly() {
    let refs = refs();
    assert!(document(&refs).is_ok());

    for (what, bad) in [
        ("truncated", pack::truncated(&refs)),
        ("corrupt section", pack::corrupted(&refs, 3, false)),
        ("corrupt section, resealed", pack::corrupted(&refs, 3, true)),
        ("wrong version", pack::of_version(&refs, 1)),
    ] {
        let refused = document(&bad).expect_err(what);
        assert!(refused.contains("refs pack"), "{what}: {refused}");
    }
}

#[test]
fn refs_jsonl_is_refused_by_its_schema_or_as_no_pack() {
    let jsonl = String::from_utf8(sf_impact_extract_dump(&refs())).unwrap();
    let schema_1 = jsonl.replacen(r#""schema":2,"#, r#""schema":1,"#, 1);

    let refused = document(schema_1.as_bytes()).unwrap_err();
    assert!(refused.contains("version 1"), "{refused}");
    let refused = document(jsonl.as_bytes()).unwrap_err();
    assert!(refused.contains("refs pack"), "{refused}");
}

#[test]
fn the_cli_refuses_a_truncated_pack_in_one_line_and_exits_1() {
    let dir = std::env::temp_dir().join(format!("sf-impact-pack-input-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (refs_path, changed_path) = (dir.join("refs.pack"), dir.join("changed.jsonl"));
    std::fs::write(&refs_path, pack::truncated(&refs())).unwrap();
    std::fs::write(&changed_path, changed()).unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_sf-impact-tests"))
        .args(["--refs".as_ref(), refs_path.as_os_str()])
        .args(["--changed".as_ref(), changed_path.as_os_str()])
        .output()
        .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert_eq!(stderr.lines().count(), 1, "{stderr}");
    assert!(stderr.contains("refs pack"), "{stderr}");
}

/// The rows of a pack as refs.jsonl, as `sf-impact-extract --dump` writes them.
fn sf_impact_extract_dump(pack: &[u8]) -> Vec<u8> {
    let pack = sf_impact_common::pack::Pack::decode(pack).unwrap();
    pack.to_jsonl()
        .iter()
        .map(|line| format!("{line}\n"))
        .collect::<String>()
        .into_bytes()
}
