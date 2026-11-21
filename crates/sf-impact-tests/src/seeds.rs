use crate::{
    graph::{Graph, Node, bare, is_type, word},
    reason,
};
use serde_json::{Value, json};
use sf_impact_common::{
    changed,
    rows::{Declaration, Resolution, fold},
};
use std::collections::BTreeMap;

fn touched_node(t: &changed::Touched, name: &str) -> Node {
    Node {
        symbol: name.into(),
        kind: t
            .referenced_type
            .as_ref()
            .map(word)
            .unwrap_or_else(|| "unknown".into()),
        package: t.package.clone(),
        file: t.file.clone(),
        line: Some(t.line),
        scope: None,
    }
}
fn changed_node(s: &changed::Symbol) -> Node {
    Node {
        symbol: s.symbol.clone(),
        kind: word(&s.kind),
        package: s.package.clone(),
        file: s.file.clone(),
        line: s.start,
        scope: None,
    }
}

pub struct Prepared {
    pub extra: Vec<Node>,
    pub gaps: Vec<Value>,
    pub errors: Vec<Value>,
}

pub fn prepare(declarations: &[Declaration], changed: &[changed::Row]) -> Result<Prepared, String> {
    let mut gaps = Vec::new();
    let mut errors = Vec::new();
    let mut extra = Vec::new();
    for row in changed {
        match row {
            changed::Row::File(f) if f.confidence == changed::Confidence::Partial => {
                if f.gaps.is_empty() {
                    return Err(format!(
                        "changed: partial file {} has no gaps",
                        sf_impact_common::diagnostic::shown(&f.file)
                    ));
                }
                for gap in &f.gaps {
                    gaps.push(reason(&word(gap), &f.file, None, "changed"));
                }
            }
            changed::Row::Symbol(s) => {
                if s.change == changed::Change::Removed
                    || !declarations.iter().any(|d| {
                        fold(&d.symbol) == fold(&s.symbol)
                            && d.package == s.package
                            && d.kind == s.kind
                            && (d.file == s.file || s.reason == changed::Reason::WholeFile)
                    })
                {
                    if s.change == changed::Change::Removed {
                        extra.push(changed_node(s));
                    } else {
                        errors.push(json!({"name":s.symbol,"reason":"missing_declaration","file":s.file,"line":s.start}));
                        gaps.push(reason("missing_declaration", &s.file, s.start, "changed"));
                    }
                }
            }
            changed::Row::Touched(t) => {
                for name in std::iter::once(&t.referenced).chain(&t.candidates) {
                    if !declarations.iter().any(|d| {
                        fold(&d.symbol) == fold(name)
                            && t.referenced_type.is_none_or(|ty| {
                                word(&d.kind) == word(&ty)
                                    || (word(&ty) == "class" && is_type(&word(&d.kind)))
                            })
                    }) {
                        extra.push(touched_node(t, name));
                    }
                }
            }
            _ => {}
        }
    }
    // A removed member still has a class for convention selection. This
    // identity is reconstructed from the qualified symbol, not a source file.
    for member in extra.clone() {
        if matches!(
            member.kind.as_str(),
            "method" | "constructor" | "field" | "enum_value"
        ) && let Some((class, _)) = bare(&member.symbol).rsplit_once('.')
            && !declarations.iter().any(|d| {
                fold(&d.symbol) == fold(class)
                    && d.package == member.package
                    && d.file == member.file
            })
        {
            extra.push(Node {
                symbol: class.into(),
                kind: "class".into(),
                line: None,
                ..member
            });
        }
    }
    Ok(Prepared {
        extra,
        gaps,
        errors,
    })
}
pub struct Seeds {
    pub seeds: BTreeMap<usize, Vec<Value>>,
    pub queries: Vec<Value>,
}
pub fn resolve(
    graph: &Graph,
    changed: &[changed::Row],
    symbols: &[String],
    gaps: &mut Vec<Value>,
    errors: &mut Vec<Value>,
) -> Seeds {
    let mut seeds: BTreeMap<usize, Vec<Value>> = BTreeMap::new();
    let mut queries = Vec::new();

    for name in symbols {
        let matches = graph.lookup(name);
        if matches.is_empty() {
            errors.push(json!({"name":name,"reason":"unknown_symbol"}));
        }
        queries.push(json!({"name":name,"ambiguous":matches.len()>1,"matches":matches.iter().map(|&i|&graph.nodes[i]).collect::<Vec<_>>()}));
        for id in matches {
            seeds
                .entry(id)
                .or_default()
                .push(json!({"source":"symbol","name":name}));
        }
    }
    for row in changed {
        match row {
            changed::Row::Symbol(s) => {
                let mut matches: Vec<_> = graph
                    .lookup(&s.symbol)
                    .into_iter()
                    .filter(|&i| {
                        let n = &graph.nodes[i];
                        fold(&n.symbol) == fold(&s.symbol)
                            && n.package == s.package
                            && n.kind == word(&s.kind)
                            && (n.file == s.file
                                || s.reason == changed::Reason::WholeFile
                                || s.change == changed::Change::Removed)
                    })
                    .collect();
                if s.change == changed::Change::Removed || matches.is_empty() {
                    matches = graph
                        .nodes
                        .iter()
                        .enumerate()
                        .filter(|(_, n)| **n == changed_node(s))
                        .map(|(i, _)| i)
                        .collect();
                } else if matches.iter().any(|&i| graph.nodes[i].file == s.file) {
                    matches.retain(|&i| graph.nodes[i].file == s.file);
                }
                for id in matches {
                    seeds
                        .entry(id)
                        .or_default()
                        .push(serde_json::to_value(row).unwrap());
                }
            }
            changed::Row::Touched(t) => {
                let names: Vec<_> = std::iter::once(&t.referenced)
                    .chain(&t.candidates)
                    .collect();
                for name in names {
                    let matches: Vec<_> = graph
                        .lookup(name)
                        .into_iter()
                        .filter(|&i| {
                            t.referenced_type.is_none_or(|kind| {
                                let kind = word(&kind);
                                graph.nodes[i].kind == kind
                                    || (kind == "class" && is_type(&graph.nodes[i].kind))
                            })
                        })
                        .collect();
                    for id in matches {
                        seeds
                            .entry(id)
                            .or_default()
                            .push(serde_json::to_value(row).unwrap());
                    }
                }
                if t.resolution == Resolution::Unresolved {
                    gaps.push(reason("unresolved", &t.file, Some(t.line), "changed"));
                }
            }
            _ => {}
        }
    }
    Seeds { seeds, queries }
}
