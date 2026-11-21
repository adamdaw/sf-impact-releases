//! Refs packs for tests: rows written by hand, put in the order that a pack
//! keeps them in, and packs altered after they were written.
#![allow(dead_code)]

use serde_json::Value;
use sf_impact_common::pack::{FileEntry, Manifest, Pack, TypeDecl};
use sha2::{Digest, Sha256};

/// The pack of `provenance` and `rows`. A pack keeps the rows of a file
/// together and the files in path order, after the scope errors, so the
/// rows are put in that order first; the rows of one file keep theirs.
pub fn pack(provenance: &Value, rows: &[Value]) -> Vec<u8> {
    try_pack(provenance, rows).unwrap_or_else(|e| panic!("{e}"))
}

/// [`pack`], or why the pack's writer refuses the rows.
pub fn try_pack(provenance: &Value, rows: &[Value]) -> Result<Vec<u8>, String> {
    let mut rows: Vec<&Value> = rows.iter().collect();
    rows.sort_by_key(|row| match row["row"].as_str() {
        Some("scope_error") => (0, String::new()),
        _ => (1, row["file"].as_str().unwrap_or_default().to_string()),
    });
    let text: String = std::iter::once(provenance)
        .chain(rows)
        .map(|row| format!("{row}\n"))
        .collect();
    Pack::from_jsonl(&text)?.encode()
}

/// The pack of a refs.jsonl, written as the pipeline writes it.
pub fn of_jsonl(text: &str) -> Vec<u8> {
    let pack = Pack::from_jsonl(text).unwrap_or_else(|e| panic!("{e}"));
    pack.encode().unwrap_or_else(|e| panic!("{e}"))
}

/// The same pack with its checksum made right again for the bytes before
/// it, so that a reader finds what was altered and not only the checksum.
pub fn resealed(mut pack: Vec<u8>) -> Vec<u8> {
    let body = pack.len() - 32;
    pack.truncate(body);
    let checksum = Sha256::digest(&pack);
    pack.extend_from_slice(&checksum);
    pack
}

/// Where the header (`SFIP` and the version) starts: after line 1.
pub fn header(pack: &[u8]) -> usize {
    pack.iter().position(|b| *b == b'\n').unwrap() + 1
}

/// Each section's kind and where it is in the pack, its header included.
pub fn sections(pack: &[u8]) -> Vec<(u8, std::ops::Range<usize>)> {
    let int = |at: &mut usize| {
        let (mut value, mut shift) = (0usize, 0);
        loop {
            let byte = pack[*at];
            *at += 1;
            value |= usize::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return value;
            }
            shift += 7;
        }
    };
    // `SFIP` and a one-byte version.
    let mut at = header(pack) + 5;
    let mut sections = Vec::new();
    while at < pack.len() - 32 {
        let start = at;
        let kind = pack[at];
        at += 1;
        let _size = int(&mut at);
        let length = int(&mut at);
        at += length;
        sections.push((kind, start..at));
    }
    sections
}

/// The same rows with a manifest: an entry for every file that has rows,
/// one for a file that has none, and a type whose names no row has.
pub fn with_manifest(pack: &[u8]) -> Vec<u8> {
    let mut pack = Pack::decode(pack).unwrap_or_else(|e| panic!("{e}"));
    let mut files: Vec<FileEntry> = Vec::new();
    for line in pack.to_jsonl().iter().skip(1) {
        let row: Value = serde_json::from_str(line).unwrap();
        let (Some(package), Some(path)) = (row["package"].as_str(), row["file"].as_str()) else {
            continue;
        };
        if files.last().is_none_or(|entry| entry.path != path) {
            let (package, path) = (package.to_string(), path.to_string());
            let blob = vec![7; 20];
            files.push(FileEntry {
                package,
                path,
                blob,
            });
        }
    }
    let package = files
        .first()
        .map_or("app".into(), |entry| entry.package.clone());
    files.push(FileEntry {
        path: format!("{package}/classes/NoRows.cls"),
        package,
        blob: vec![9; 20],
    });
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let types = vec![TypeDecl {
        file: files.len() - 1,
        symbol: "NoRows".into(),
        superclass: Some("ManifestOnlyType".into()),
        interfaces: Vec::new(),
        methods: Vec::new(),
        constructors: Vec::new(),
        fields: Vec::new(),
    }];
    pack.manifest = Some(Manifest {
        files,
        types,
        relationships: Vec::new(),
    });
    pack.encode().unwrap_or_else(|e| panic!("{e}"))
}

/// Line 1 as it was written, and half of what follows it.
pub fn truncated(pack: &[u8]) -> Vec<u8> {
    let header = header(pack);
    pack[..header + (pack.len() - header) / 2].to_vec()
}

/// One byte of the frame of the section of this kind changed: with the
/// checksum left as it was, or made right again for the changed bytes.
pub fn corrupted(pack: &[u8], kind: u8, reseal: bool) -> Vec<u8> {
    let (_, at) = sections(pack)
        .into_iter()
        .find(|(k, _)| *k == kind)
        .unwrap();
    let mut pack = pack.to_vec();
    pack[at.end - 2] ^= 0xff;
    match reseal {
        true => resealed(pack),
        false => pack,
    }
}

/// The same pack, saying that its layout is of another version.
pub fn of_version(pack: &[u8], version: u8) -> Vec<u8> {
    let mut pack = pack.to_vec();
    let at = header(&pack) + 4;
    pack[at] = version;
    resealed(pack)
}

/// A pack with a manifest, with its manifest section there twice.
pub fn two_manifests(pack: &[u8]) -> Vec<u8> {
    let (_, at) = sections(pack).into_iter().find(|(k, _)| *k == 6).unwrap();
    let mut out = pack[..at.end].to_vec();
    out.extend_from_slice(&pack[at]);
    out.extend_from_slice(&pack[pack.len() - 32..]);
    resealed(out)
}
