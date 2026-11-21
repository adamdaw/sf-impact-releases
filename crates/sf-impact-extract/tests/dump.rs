//! `sf-impact-extract --dump`: the rows of a refs pack, as the refs.jsonl
//! they were made of, which `--format jsonl` writes.

use sf_impact_common::pack::Pack;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::{env, fs};

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn extract(args: &[&str]) -> Output {
    let binary = env!("CARGO_BIN_EXE_sf-impact-extract");
    Command::new(binary)
        .args(args)
        .output()
        .expect("the binary runs")
}

/// `--dump` of `pack`, written to a file of its own for the run.
fn dump(pack: &[u8], name: &str) -> Output {
    let path = env::temp_dir().join(format!("sf-impact-dump-{}-{name}.pack", std::process::id()));
    fs::write(&path, pack).unwrap();
    let output = extract(&["--dump", path.to_str().unwrap()]);
    fs::remove_file(&path).unwrap();
    output
}

/// Every refs.jsonl the fixtures give: the golden file, and the output of
/// extract on each fixture project.
fn every_refs() -> Vec<(String, String)> {
    let golden = fs::read_to_string(repository().join("fixtures/refs.jsonl")).unwrap();
    let mut all = vec![("golden".to_string(), golden)];
    for project in ["fixtures/project", "fixtures/changed/base"] {
        let root = repository().join(project);
        let output = extract(&["--root", root.to_str().unwrap(), "--format", "jsonl"]);
        assert!(output.status.success(), "{project}: {output:?}");
        all.push((
            project.replace('/', "-"),
            String::from_utf8(output.stdout).unwrap(),
        ));
    }
    all
}

#[test]
fn the_dump_of_a_pack_is_the_refs_jsonl_it_was_made_of() {
    for (name, refs) in every_refs() {
        let pack = Pack::from_jsonl(&refs).unwrap().encode().unwrap();

        let output = dump(&pack, &name);

        assert!(output.status.success(), "{name}: {output:?}");
        let projection = String::from_utf8(output.stdout).unwrap();
        assert!(projection == refs, "{name}: the dump differs");
        let again = Pack::from_jsonl(&projection).unwrap().encode().unwrap();
        assert!(
            again == pack,
            "{name}: pack -> JSONL -> pack changed the bytes"
        );
    }
}

#[test]
fn a_pack_that_is_not_intact_is_refused_with_nothing_written() {
    let refs = fs::read_to_string(repository().join("fixtures/refs.jsonl")).unwrap();
    let pack = Pack::from_jsonl(&refs).unwrap().encode().unwrap();

    let output = dump(&pack[..pack.len() - 1], "cut");

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("checksum"), "{stderr}");
}

#[path = "../../sf-impact-common/tests/support/schema.rs"]
mod schema;

/// A pack's content, written as the one JSON object that
/// schema/refs-pack.schema.json describes.
fn content(pack: &Pack) -> serde_json::Value {
    let lines = pack.to_jsonl();
    let parse = |line: &String| serde_json::from_str::<serde_json::Value>(line).unwrap();
    let mut content = serde_json::json!({
        "provenance": parse(&lines[0]),
        "rows": lines[1..].iter().map(parse).collect::<Vec<_>>(),
    });
    if let Some(manifest) = &pack.manifest {
        content["manifest"] = serde_json::to_value(manifest).unwrap();
    }
    content
}

#[test]
fn the_content_of_a_pack_is_valid_against_its_logical_schema() {
    use sf_impact_common::pack::{
        Callable, Field, FileEntry, Manifest, RelationshipFact, TypeDecl,
    };
    let schema = schema::Schema::parse(
        &fs::read_to_string(repository().join("schema/refs-pack.schema.json")).unwrap(),
    );
    let refs = fs::read_to_string(repository().join("fixtures/refs.jsonl")).unwrap();
    let mut pack = Pack::from_jsonl(&refs).unwrap();
    let mut files: Vec<FileEntry> = Vec::new();
    for line in &pack.to_jsonl()[1..] {
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        let (Some(package), Some(path)) = (row["package"].as_str(), row["file"].as_str()) else {
            continue;
        };
        if files.last().is_none_or(|last| last.path != path) {
            let (package, path) = (package.to_string(), path.to_string());
            files.push(FileEntry {
                package,
                path,
                blob: vec![7; 20],
            });
        }
    }
    pack.manifest = Some(Manifest {
        files,
        types: vec![TypeDecl {
            file: 0,
            symbol: "Pack".into(),
            superclass: None,
            interfaces: vec!["Schedulable".into()],
            methods: vec![Callable {
                symbol: "Pack.run(Integer)".into(),
                name: "run".into(),
                params: vec!["Integer".into()],
                returns: None,
            }],
            constructors: Vec::new(),
            fields: vec![Field {
                symbol: "Pack.LEVEL".into(),
                key: "level".into(),
                ty: "String".into(),
                is_static: true,
                is_enum_value: false,
            }],
        }],
        relationships: vec![RelationshipFact {
            file: 0,
            field: "Pack.Carrier__c".into(),
            to: "Carrier__c".into(),
            child_name: None,
            controlling: None,
        }],
    });
    pack.encode().expect("a valid pack");

    assert_eq!(schema.errors(&content(&pack)), Vec::<String>::new());
    // The control: the schema does refuse what a pack cannot hold.
    pack.manifest.as_mut().unwrap().files[0].blob = vec![7; 7];
    assert!(!schema.errors(&content(&pack)).is_empty());
}

#[test]
fn extract_writes_a_pack_by_default_and_its_dump_with_format_jsonl() {
    let root = repository().join("fixtures/project");
    let root = root.to_str().unwrap();

    let pack = extract(&["--root", root]);
    let jsonl = extract(&["--root", root, "--format", "jsonl"]);
    let to_file = env::temp_dir().join(format!("sf-impact-format-{}.pack", std::process::id()));
    let written = extract(&["--root", root, "--out", to_file.to_str().unwrap()]);

    assert!(pack.status.success(), "{pack:?}");
    assert!(jsonl.status.success(), "{jsonl:?}");
    assert!(written.status.success(), "{written:?}");
    assert!(
        fs::read(&to_file).unwrap() == pack.stdout,
        "--out writes what stdout has"
    );
    fs::remove_file(&to_file).unwrap();
    let decoded = Pack::decode(&pack.stdout).expect("the default output is a pack");
    // The fixture is in a repository, so its pack has a manifest.
    assert!(decoded.manifest.is_some());
    assert!(dump(&pack.stdout, "format").stdout == jsonl.stdout);
    let first = String::from_utf8(jsonl.stdout).unwrap();
    assert!(first.starts_with(r#"{"row":"provenance","tool":"sf-impact-extract","#));
    assert!(first.lines().next().unwrap().contains(r#""schema":2,"#));
}

#[test]
fn an_unknown_format_is_refused() {
    let output = extract(&["--root", "repo", "--format", "xml"]);

    assert_eq!(output.status.code(), Some(2));
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("--format"), "{stderr}");
}
