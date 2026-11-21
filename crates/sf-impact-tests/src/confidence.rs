use crate::{graph::Graph, reason, walk::Walk};
use serde_json::Value;
use sf_impact_common::rows::{Resolution, Row, Tier};
use std::collections::BTreeMap;

pub fn collect(
    graph: &Graph,
    rows: &[Row],
    walk: &Walk,
    gaps: &mut Vec<Value>,
) -> (Vec<usize>, BTreeMap<String, usize>) {
    let mut unresolved = Vec::new();
    let mut diagnostics = BTreeMap::new();
    for (id, edge) in graph.edges.iter().enumerate() {
        let r = &edge.reference;
        let kind = if r.resolution == Some(Resolution::Unresolved) {
            Some("unresolved")
        } else if r.tier == Tier::Parsed && graph.ambiguous(id) {
            Some("ambiguous")
        } else {
            None
        };
        if let Some(kind) = kind {
            if graph.relevant(r, &walk.reached) {
                gaps.push(reason(kind, &r.file, Some(r.line), "refs"));
                if kind == "unresolved" {
                    unresolved.push(id);
                }
            } else {
                *diagnostics.entry(kind.into()).or_default() += 1;
            }
        }
    }
    let apex_files: std::collections::BTreeSet<_> = walk
        .reached
        .iter()
        .chain(walk.selected.keys())
        .map(|&id| &graph.nodes[id].file)
        .filter(|f| f.ends_with(".cls") || f.ends_with(".trigger"))
        .collect();
    for row in rows {
        let (kind, file, line, relevant) = match row {
            Row::ScopeError(e) => ("scope_error", &e.path, None, true),
            Row::ParseError(e) => (
                "parse_error",
                &e.file,
                Some(e.line),
                apex_files.contains(&e.file),
            ),
            Row::SkippedFile(e) => ("skipped_file", &e.file, None, apex_files.contains(&e.file)),
            Row::UnsupportedElement(e) => (
                "unsupported_element",
                &e.file,
                Some(e.line),
                apex_files.contains(&e.file),
            ),
            Row::UnmodelledFile(e) => ("unmodelled_file", &e.file, None, false),
            _ => continue,
        };
        if relevant {
            gaps.push(reason(kind, file, line, "refs"));
        } else {
            *diagnostics.entry(kind.into()).or_default() += 1;
        }
    }
    for &id in walk.triggers.difference(&walk.tested_triggers) {
        let n = &graph.nodes[id];
        gaps.push(reason("trigger_entry", &n.file, n.line, "refs"));
    }
    for &id in walk.bindings.difference(&walk.tested_bindings) {
        let r = &graph.edges[id].reference;
        gaps.push(reason("dynamic_binding", &r.file, Some(r.line), "refs"));
    }
    (unresolved, diagnostics)
}
