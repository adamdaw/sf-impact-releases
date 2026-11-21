//! Incoming references and test selection from input streams alone.
//!
//! Before anything is walked, the first row of each input is read, which
//! says what the file is of: refs that are not of the head that
//! changed.jsonl was made for are refused.
mod confidence;
mod graph;
mod output;
mod seeds;
mod walk;
use graph::{Graph, Node, is_type, word};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use sf_impact_common::{
    changed,
    pack::Pack,
    provenance::{self, Reader},
    rows::{Declaration, Reference, Row, fold},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, Write},
};
// Keys come from untrusted input; seed the hasher against collision floods.
type HashMap<K, V> = std::collections::HashMap<K, V, foldhash::fast::RandomState>;

#[derive(Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    pub test_suffixes: Vec<String>,
    pub exclude_unpackaged_tests: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            test_suffixes: ["Test", "Tests", "_Test", "_Tests"]
                .map(String::from)
                .into(),
            exclude_unpackaged_tests: false,
        }
    }
}
/// The rows of an input after its first, which says what the file is of
/// and is read before any other: lines count from 2.
fn read<T: DeserializeOwned>(
    input: impl BufRead,
    label: &str,
) -> impl Iterator<Item = Result<T, String>> {
    (2..).zip(input.lines()).map(move |(number, line)| {
        let line = line.map_err(|e| format!("{label}: line {number}: {e}"))?;
        serde_json::from_str(&line).map_err(|e| format!("{label}: line {number}: {e}"))
    })
}
fn canonical(values: &mut Vec<Value>) {
    values.sort_by_cached_key(Value::to_string);
    values.dedup();
}
fn reason(kind: &str, file: &str, line: Option<u32>, source: &str) -> Value {
    json!({"reason":kind,"file":file,"line":line,"source":source})
}
fn eligible(node: &Node, config: &Config) -> bool {
    !(config.exclude_unpackaged_tests && node.scope.as_deref() == Some("unpackagedMetadata"))
}

/// Convenience API for small callers/tests. The CLI streams typed output with
/// write_impact and does not build a serde_json::Value tree.
pub fn impact(
    refs: impl BufRead,
    changes: Option<&[u8]>,
    symbols: &[String],
    depth: usize,
    config: &Config,
    accept_unverified_refs: bool,
) -> Result<Value, String> {
    let mut bytes = Vec::new();
    write_impact(
        refs,
        changes,
        symbols,
        depth,
        config,
        accept_unverified_refs,
        &mut bytes,
    )?;
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
/// Reads the first row of each input, holds each to its schema, and holds
/// the refs to the head that changed.jsonl was made for. Gives the
/// confidence reasons that the inputs bring with them: refs that could not
/// be verified and were accepted, here or where changed.jsonl was made.
/// With symbols there is no head, and refs of any tree are read.
fn verified(
    refs: &mut &[u8],
    changes: &mut Option<&[u8]>,
    accept_unverified_refs: bool,
) -> Result<(provenance::Impact, Vec<Value>), String> {
    let reader = Reader {
        tool: "sf-impact-tests",
        version: env!("CARGO_PKG_VERSION"),
    };
    let Some(changes) = changes else {
        let refs = reader.read::<provenance::Refs>(refs, "re-run sf-impact-extract")?;
        return Ok((
            provenance::Impact {
                writer: provenance::Writer::of(&provenance::IMPACT, env!("CARGO_PKG_VERSION")),
                inputs: provenance::ImpactInputs {
                    refs,
                    changed: None,
                },
            },
            Vec::new(),
        ));
    };
    let of_changed: provenance::Changed = reader.read(changes, "re-run sf-impact-changed")?;
    let at_head = format!("re-run sf-impact-extract at {}", of_changed.head);
    let of_refs: provenance::Refs = reader.read(refs, &at_head)?;
    let head = provenance::Head {
        file: provenance::CHANGED.name,
        commit: &of_changed.head,
        tree: &of_changed.head_tree,
        root: &of_changed.root,
        scope: &of_changed.scope,
        config: &of_changed.config,
    };
    let mut unverified = Vec::new();
    if !of_refs.verify(&head, accept_unverified_refs)? {
        let refs = provenance::REFS.name;
        unverified.push(reason("unverified_refs", refs, Some(1), "refs"));
    }
    for gap in &of_changed.gaps {
        let changed = provenance::CHANGED.name;
        unverified.push(reason(&word(gap), changed, Some(1), "changed"));
    }
    Ok((
        provenance::Impact {
            writer: provenance::Writer::of(&provenance::IMPACT, env!("CARGO_PKG_VERSION")),
            inputs: provenance::ImpactInputs {
                refs: of_refs,
                changed: Some(of_changed),
            },
        },
        unverified,
    ))
}
/// Write one document. A false result means error rows were written.
/// `refs` is the refs pack that sf-impact-extract writes.
/// `accept_unverified_refs` says to walk refs of a tree that is no commit's,
/// which cannot be known to be the head that changed.jsonl was made for.
pub fn write_impact(
    refs: impl BufRead,
    changes: Option<&[u8]>,
    symbols: &[String],
    depth: usize,
    config: &Config,
    accept_unverified_refs: bool,
    sink: &mut impl Write,
) -> Result<bool, String> {
    let (ok, _kept) = write_impact_kept(
        refs,
        changes,
        symbols,
        depth,
        config,
        accept_unverified_refs,
        sink,
    )?;
    Ok(ok)
}
/// What [`write_impact_kept`] built to write its document, handed back
/// instead of freed: the graph, the rows and the walk.
pub struct Kept {
    _graph: Graph,
    _declarations: Vec<Declaration>,
    _rows: Vec<Row>,
    _walk: walk::Walk,
    _changed: Vec<changed::Row>,
}
/// [`write_impact`], handing back what it built rather than freeing it, so
/// that a process about to exit need not.
pub fn write_impact_kept(
    mut refs: impl BufRead,
    mut changes: Option<&[u8]>,
    symbols: &[String],
    depth: usize,
    config: &Config,
    accept_unverified_refs: bool,
    sink: &mut impl Write,
) -> Result<(bool, Kept), String> {
    if changes.is_some() == !symbols.is_empty() {
        return Err("give exactly one of changed input or symbols".into());
    }
    // Line 1 of a pack is its provenance row, read as a refs.jsonl's was;
    // the rest is decoded, and held to the pack's checks, after changed.
    let mut pack = Vec::new();
    let read_refs = refs.read_to_end(&mut pack);
    read_refs.map_err(|e| format!("{}: {e}", provenance::REFS.name))?;
    let (provenance, unverified) =
        verified(&mut pack.as_slice(), &mut changes, accept_unverified_refs)?;
    let changed: Vec<changed::Row> = match changes {
        Some(bytes) => read(bytes, "changed").collect::<Result<_, _>>()?,
        None => Vec::new(),
    };
    let refs = Pack::decode(&pack)?.rows;
    drop(pack);
    let mut declarations: Vec<Declaration> = Vec::new();
    let mut references: Vec<Reference> = Vec::with_capacity(refs.len());
    let mut rows = Vec::new();
    for row in refs {
        match row {
            Row::Declaration(d) => declarations.push(d),
            Row::Reference(r) => references.push(r),
            diagnostic => rows.push(diagnostic),
        }
    }
    declarations.sort_by(output::cmp_declaration);
    let mut ids = output::sort_references(&mut references);
    output::dedup_references(&mut references, &mut ids);
    let seeds::Prepared {
        extra,
        mut gaps,
        mut errors,
    } = seeds::prepare(&declarations, &changed)?;
    gaps.extend(unverified);
    let graph = Graph::new(&declarations, references, &ids, extra);
    let resolved = seeds::resolve(&graph, &changed, symbols, &mut gaps, &mut errors);
    let mut walk = walk::Walk::default();
    let mut seed_output = Vec::new();
    let mut used = BTreeSet::new();
    for (id, mut sources) in resolved.seeds {
        canonical(&mut sources);
        let direct = graph.incoming_to(id, true);
        walk.seed(&graph, id, &direct, depth, config);
        seed_output.push(output::seed(&graph, id, sources, &direct, &mut used));
    }
    walk.finish();
    let (unresolved, mut diagnostics) = confidence::collect(&graph, &rows, &walk, &mut gaps);
    used.extend(&unresolved);
    let tests = output::tests(&graph, &walk.selected, &mut used);
    let possible = output::tests(&graph, &walk.possible, &mut used);
    let mut collision_groups: BTreeMap<(String, &str), Vec<&Declaration>> = BTreeMap::new();
    for d in &declarations {
        let kind = d.kind.as_str();
        collision_groups
            .entry((fold(&d.symbol), if is_type(kind) { "class" } else { kind }))
            .or_default()
            .push(d);
    }
    // A collision is written when the walk reached one of its declarations,
    // the seeds included, as for unresolved and ambiguous names. The others
    // are counted, so that what is left out still shows.
    let reached = |d: &&Declaration| {
        let node = graph.nodes.binary_search(&Node::declaration(d));
        node.is_ok_and(|id| walk.reached.contains(&id))
    };
    let (collisions, unreached): (Vec<_>, Vec<_>) = collision_groups
        .into_values()
        .filter(|ds| ds.len() > 1)
        .partition(|ds| ds.iter().any(reached));
    if !unreached.is_empty() {
        diagnostics.insert("unreached_collision".into(), unreached.len());
    }
    let collisions = collisions
        .into_iter()
        .map(|ds| output::Collision {
            symbol: &ds[0].symbol,
            declarations: ds.into_iter().map(Into::into).collect(),
        })
        .collect();
    canonical(&mut gaps);
    canonical(&mut errors);
    let mut queries = resolved.queries;
    canonical(&mut queries);
    let ok = errors.is_empty();
    let report = output::Report {
        provenance,
        depth,
        seeds: seed_output,
        queries,
        tests,
        possible,
        references: used
            .into_iter()
            .map(|id| (id, &graph.edges[id].reference))
            .collect(),
        untested: walk.untested.iter().map(|&id| &graph.nodes[id]).collect(),
        confidence: if gaps.is_empty() && ok {
            "full"
        } else {
            "partial"
        },
        gaps,
        diagnostics,
        unresolved,
        collisions,
        errors,
    };
    serde_json::to_writer(&mut *sink, &report).map_err(|e| e.to_string())?;
    sink.write_all(b"\n").map_err(|e| e.to_string())?;
    drop(report);
    Ok((
        ok,
        Kept {
            _graph: graph,
            _declarations: declarations,
            _rows: rows,
            _walk: walk,
            _changed: changed,
        },
    ))
}
