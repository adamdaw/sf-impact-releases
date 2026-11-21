//! Shared by the `sf-impact-*` tools: the row types of `refs.jsonl`, `delta.jsonl`
//! and `changed.jsonl`, the provenance row that each starts with, the
//! comparison of two texts line by line, the optional config file, and the
//! reading of `sfdx-project.json` into a set of in-scope files. The tools exchange files, not Rust types; see
//! `schema/`.

pub mod allocator;
pub mod changed;
pub mod config;
pub mod delta;
pub mod diagnostic;
pub mod files;
pub mod lines;
pub mod pack;
pub mod provenance;
pub mod rows;
pub mod scope;
pub mod types;

mod contract;

// Linked for its C symbols alone: on musl, every command's memcmp and bcmp.
extern crate sf_impact_memcmp;

/// A file is text if and only if it is valid UTF-8, or UTF-16 (LE or BE) with a BOM that decodes without error, and the decoded content contains no NUL (U+0000).
/// An optional UTF-8 BOM is preserved; a UTF-16 BOM is removed.
pub fn text(bytes: &[u8]) -> Option<String> {
    let decoded = match bytes.get(..2) {
        Some([0xff, 0xfe] | [0xfe, 0xff]) => {
            let to_unit = if bytes[0] == 0xff {
                u16::from_le_bytes
            } else {
                u16::from_be_bytes
            };
            let (pairs, odd) = bytes[2..].as_chunks::<2>();
            if !odd.is_empty() {
                return None;
            }
            let units: Vec<u16> = pairs.iter().map(|pair| to_unit(*pair)).collect();
            String::from_utf16(&units).ok()
        }
        _ => std::str::from_utf8(bytes).ok().map(str::to_owned),
    };
    decoded.filter(|text| !text.contains('\0'))
}
