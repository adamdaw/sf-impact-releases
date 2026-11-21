//! The library behind `sf-impact-changed`: maps the difference between two
//! commits, as `delta.jsonl` has it, onto the symbols of a Salesforce DX
//! project, as the refs pack has them at head, and gives the lines of
//! `changed.jsonl`: one changed file, changed symbol or touched name each.
//! The rows are defined by `schema/changed.schema.json`.
//!
//! It reads those two files and nothing else: no repository, no working
//! tree, no environment. The same two files give the same lines.
//!
//! Before it maps anything it reads the first row of each, which says
//! what the file is of, and refuses refs that are not of the head of the
//! delta.

mod delta;
mod head;
mod map;

use delta::Delta;
use head::Head;
use map::Mapper;
use sf_impact_common::changed::{Confidence, Row};
use sf_impact_common::config::{CONFIG_FILE, Config};
use sf_impact_common::pack::Pack;
use sf_impact_common::provenance::{self, Gap, Inputs, Reader, Writer};
use sf_impact_common::scope::{self, PROJECT_FILE};
use std::collections::BTreeSet;
use std::io::BufRead;

/// The lines of `changed.jsonl`: what the file is of, then its rows,
/// sorted. `refs` is the refs pack that sf-impact-extract writes.
/// `accept_unverified_refs` says to map onto refs of a tree that is no
/// commit's, which cannot be known to be the head of the delta.
pub fn changed(
    mut delta: impl BufRead,
    mut refs: impl BufRead,
    accept_unverified_refs: bool,
) -> Result<Vec<String>, String> {
    // Before anything else: what each file is of, whether it says so as
    // its schema has it, and whether the two go together.
    let version = env!("CARGO_PKG_VERSION");
    let reader = Reader {
        tool: "sf-impact-changed",
        version,
    };
    let of_delta: provenance::Delta = reader.read(&mut delta, "re-run sf-impact-delta")?;
    let at_head = format!("re-run sf-impact-extract at {}", of_delta.head);
    // Line 1 of a pack is its provenance row, read as a refs.jsonl's was;
    // the rest is decoded, and held to the pack's checks, below.
    let mut pack = Vec::new();
    let read = refs.read_to_end(&mut pack);
    read.map_err(|e| format!("{}: {e}", provenance::REFS.name))?;
    let of_refs: provenance::Refs = reader.read(&mut pack.as_slice(), &at_head)?;
    let provenance = verified(version, of_delta, of_refs, accept_unverified_refs)?;

    let delta = Delta::read(delta).map_err(|e| format!("the delta: {e}"))?;
    // Scope and config are those of the two commits, as the delta has
    // them: at head for a file's path at head, at base for its path at base.
    let head_dirs = match scope_dirs(&delta, &delta.head.project_file)? {
        Some(dirs) => dirs,
        None => return Err(format!("the head commit has no {PROJECT_FILE} at the root")),
    };
    let base_dirs = scope_dirs(&delta, &delta.base.project_file)?.unwrap_or_default();
    let config: Config = match text(&delta, &delta.head.config_file, CONFIG_FILE)? {
        Some(config) => serde_json::from_str(config).map_err(|e| format!("{CONFIG_FILE}: {e}"))?,
        None => Config::default(),
    };
    let rows = Pack::decode_without_textual(&pack)?.rows;
    drop(pack);
    let head = Head::read(rows, &config.namespaces);

    let dirs = |dirs: &[String]| -> BTreeSet<String> { dirs.iter().cloned().collect() };
    let mapper = Mapper {
        delta: &delta,
        base_dirs: &base_dirs,
        head_dirs: &head_dirs,
        scope_changed: dirs(&base_dirs) != dirs(&head_dirs),
        head: &head,
    };
    let rows = mapper.rows();
    let mut keyed: Vec<_> = rows
        .iter()
        .map(|row| (position(row), to_line(row)))
        .collect();
    keyed.sort();
    let rows = keyed.into_iter().map(|(_, line)| line);
    Ok(std::iter::once(provenance::line(provenance))
        .chain(rows)
        .collect())
}

/// What `changed.jsonl` is of, if its two inputs go together: the refs
/// of the head of the delta, or accepted though that cannot be known.
/// Both rows are valid: what is taken from them is.
fn verified(
    version: &str,
    delta: provenance::Delta,
    refs: provenance::Refs,
    accept_unverified_refs: bool,
) -> Result<provenance::Changed, String> {
    let unreadable = |file: &str| {
        format!("delta.jsonl: the head commit has no {file} at the root that can be read")
    };
    let scope = delta
        .scope
        .clone()
        .ok_or_else(|| unreadable(PROJECT_FILE))?;
    let config = delta
        .config
        .clone()
        .ok_or_else(|| unreadable(CONFIG_FILE))?;
    let head = provenance::Head {
        file: provenance::DELTA.name,
        commit: &delta.head,
        tree: &delta.head_tree,
        root: &delta.root,
        scope: &scope,
        config: &config,
    };
    let (confidence, gaps) = match refs.verify(&head, accept_unverified_refs)? {
        true => (Confidence::Full, Vec::new()),
        false => (Confidence::Partial, vec![Gap::UnverifiedRefs]),
    };
    Ok(provenance::Changed {
        writer: Writer::of(&provenance::CHANGED, version),
        base: delta.base.clone(),
        head: delta.head.clone(),
        head_tree: delta.head_tree.clone(),
        root: delta.root.clone(),
        scope,
        config,
        confidence,
        gaps,
        inputs: Inputs { refs, delta },
    })
}

/// The text of a file that a commit row names, if the commit has the file.
fn text<'a>(delta: &'a Delta, id: &Option<String>, name: &str) -> Result<Option<&'a str>, String> {
    let Some(id) = id else {
        return Ok(None);
    };
    let text = delta.text(id);
    text.map(Some)
        .ok_or_else(|| format!("{name} is the blob {id}, which the delta has no text for"))
}

/// The scope directories that a commit's `sfdx-project.json` declares, as
/// paths from the root. `None` if the commit has no such file. A declared
/// path that leaves the root holds nothing that is in scope.
fn scope_dirs(delta: &Delta, project_file: &Option<String>) -> Result<Option<Vec<String>>, String> {
    let Some(project) = text(delta, project_file, PROJECT_FILE)? else {
        return Ok(None);
    };
    let declared =
        scope::declared(project.as_bytes()).map_err(|e| format!("{PROJECT_FILE}: {e}"))?;
    let dirs = declared
        .iter()
        .filter_map(|(path, _)| scope::normalise(path));
    Ok(Some(dirs.collect()))
}

/// Sort key: the rows of a file together, its `file` row first, then its
/// symbols by line, then the names touched at base and at head by line.
/// The serialised row breaks ties, which makes the order total.
fn position(row: &Row) -> (String, u8, u32) {
    match row {
        Row::File(file) => (file.file.clone(), 0, 0),
        Row::Symbol(symbol) => (symbol.file.clone(), 1, symbol.start.unwrap_or(0)),
        Row::Touched(touched) => (touched.file.clone(), 2 + touched.side as u8, touched.line),
    }
}

fn to_line(row: &Row) -> String {
    serde_json::to_string(row).expect("a row has only string keys and plain values")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rows_of_a_file_are_together_and_in_the_order_of_the_file() {
        let row = |line: &str| -> Row { serde_json::from_str(line).unwrap() };
        let file = row(
            r#"{"row":"file","file":"a/P.permissionset-meta.xml","status":"modified","package":"a","confidence":"full"}"#,
        );
        let symbol = row(
            r#"{"row":"symbol","symbol":"P","type":"permission_set","package":"a","file":"a/P.permissionset-meta.xml","change":"modified","reason":"hunk","start":1,"end":9,"hunks":[{"base_start":1,"base_lines":1,"head_start":1,"head_lines":1}],"collision":false}"#,
        );
        let touched = |side: &str, line: u32| {
            row(&format!(
                r#"{{"row":"touched","referrer":"P","side":"{side}","kind":"grant","referenced":"A","resolution":"unresolved","package":"a","file":"a/P.permissionset-meta.xml","line":{line}}}"#
            ))
        };
        let earlier_file = row(
            r#"{"row":"file","file":"a/A.cls","status":"deleted","package":"a","confidence":"full"}"#,
        );
        let in_order = [
            earlier_file,
            file,
            symbol,
            touched("base", 7),
            touched("head", 3),
            touched("head", 5),
        ];

        let mut sorted = in_order.to_vec();
        sorted.reverse();
        sorted.sort_by_key(position);

        assert_eq!(sorted, in_order);
    }
}
