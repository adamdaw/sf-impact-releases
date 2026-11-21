use crate::{HashMap, output::RowIds};
use serde::Serialize;
use sf_impact_common::rows::{DeclType, Declaration, RefKind, Reference, Resolution, Tier, fold};
use std::collections::{BTreeMap, BTreeSet};

/// File and line distinguish declarations even when their API identities collide.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Node {
    pub symbol: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub package: String,
    pub file: String,
    pub line: Option<u32>,
    pub scope: Option<String>,
}
impl Node {
    pub fn declaration(d: &Declaration) -> Self {
        Self {
            symbol: d.symbol.clone(),
            kind: d.kind.as_str().to_owned(),
            package: d.package.clone(),
            file: d.file.clone(),
            line: Some(d.start),
            scope: d.scope.map(|s| s.as_str().to_owned()),
        }
    }
}
pub fn word(value: &impl Serialize) -> String {
    serde_json::to_value(value)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string()
}
pub fn bare(symbol: &str) -> &str {
    symbol.split('(').next().unwrap()
}
pub fn short(symbol: &str) -> &str {
    bare(symbol).rsplit('.').next().unwrap()
}
pub fn is_type(kind: &str) -> bool {
    matches!(kind, "class" | "interface" | "enum")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    Reference,
    Member,
    Dispatch,
    OwnerBinding,
    TriggerDml,
    PlatformEntry,
}

/// An edge index and small identities, never cloned reference rows or nodes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Link {
    pub edge: usize,
    pub target: usize,
    pub via: Via,
    pub evidence: Vec<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Step {
    pub link: Link,
    pub source: usize,
}

pub struct Edge {
    pub reference: Reference,
    /// Empty for a grouped textual row: its targets are its word's.
    targets: Vec<usize>,
    word: Option<usize>,
    pub sources: Vec<usize>,
}
/// A textual word is resolved once. Every row naming it shares its targets
/// and is listed once here, not once per target in `incoming`.
struct Word {
    targets: Vec<usize>,
    edges: Vec<usize>,
}
/// Whether two consecutive rows resolve to the same targets: every input to
/// that is equal. `same_referenced` says so of what they name.
fn same_targets(a: &Reference, b: &Reference, same_referenced: bool) -> bool {
    a.tier == b.tier
        && a.resolution == b.resolution
        && a.referenced_type == b.referenced_type
        && same_referenced
        && a.candidates == b.candidates
}
/// Declarations by package, then file, then folded symbol.
type Locations = HashMap<String, HashMap<String, HashMap<String, Vec<usize>>>>;
/// What the per-reference work needs of one distinct (package, file) of a
/// reference, found once so that work never reads or compares the paths.
struct RefFile<'a> {
    file: String,
    /// The declarations in this file, by folded symbol.
    symbols: Option<&'a HashMap<String, Vec<usize>>>,
    /// The file nodes made for this file, by symbol.
    file_nodes: HashMap<String, usize>,
}
/// A reusable buffer for `fold`: the same lowercase, without allocating.
#[derive(Default)]
struct Folded(String);
impl Folded {
    fn of(&mut self, name: &str) -> &str {
        self.0.clear();
        self.0.push_str(name);
        self.0.make_ascii_lowercase();
        &self.0
    }
}
/// Names are indexed once. Traversal touches only incoming edges, never a
/// source tree, and a cycle cannot enqueue a node twice for one seed.
pub struct Graph {
    pub nodes: Vec<Node>,
    pub tests: BTreeSet<usize>,
    invocable: BTreeSet<usize>,
    pub edges: Vec<Edge>,
    /// Incoming edges with targets of their own; see `incoming_edges`.
    incoming: Vec<Vec<usize>>,
    words: Vec<Word>,
    /// The word whose targets include a node: its own short name, if any.
    node_word: Vec<Option<usize>>,
    outgoing: Vec<Vec<usize>>,
    dml: BTreeMap<String, Vec<usize>>,
    pub owners: Vec<Option<usize>>,
    pub members: Vec<Vec<usize>>,
    file_types: BTreeMap<String, BTreeSet<usize>>,
    names: HashMap<String, Vec<usize>>,
    exact: HashMap<String, Vec<usize>>,
}
impl Graph {
    /// `ids` are the references' `RowIds`, one per row in the same order.
    pub fn new(
        declarations: &[Declaration],
        references: Vec<Reference>,
        ids: &[RowIds],
        extra: Vec<Node>,
    ) -> Self {
        assert_eq!(references.len(), ids.len());
        let mut nodes: Vec<_> = declarations
            .iter()
            .map(Node::declaration)
            .chain(extra)
            .collect();
        nodes.sort();
        nodes.dedup();
        let mut tests = BTreeSet::new();
        for d in declarations
            .iter()
            .filter(|d| d.is_test && d.kind == DeclType::Class)
        {
            tests.insert(nodes.binary_search(&Node::declaration(d)).unwrap());
        }
        let invocable = declarations
            .iter()
            .filter(|d| {
                d.annotations
                    .iter()
                    .any(|a| a.eq_ignore_ascii_case("InvocableMethod"))
            })
            .map(|d| nodes.binary_search(&Node::declaration(d)).unwrap())
            .collect();
        let mut graph = Self {
            nodes,
            tests,
            invocable,
            edges: Vec::with_capacity(references.len()),
            incoming: Vec::new(),
            words: Vec::new(),
            node_word: Vec::new(),
            outgoing: Vec::new(),
            dml: BTreeMap::new(),
            owners: Vec::new(),
            members: Vec::new(),
            file_types: BTreeMap::new(),
            names: HashMap::default(),
            exact: HashMap::default(),
        };
        for (id, node) in graph.nodes.iter().enumerate() {
            graph.exact.entry(fold(&node.symbol)).or_default().push(id);
            for name in BTreeSet::from([
                fold(&node.symbol),
                fold(bare(&node.symbol)),
                fold(short(&node.symbol)),
            ]) {
                graph.names.entry(name).or_default().push(id);
            }
        }
        // Resolve sources by their file as well as their symbol. LWC auxiliary
        // files share the bundle declaration, anchored in one other file.
        let mut locations = Locations::default();
        for (id, node) in graph.nodes.iter().enumerate() {
            locations
                .entry(node.package.clone())
                .or_default()
                .entry(node.file.clone())
                .or_default()
                .entry(fold(&node.symbol))
                .or_default()
                .push(id);
        }
        // Each reference's (package, file), by its number in first-seen order.
        let mut ref_files: Vec<RefFile> = Vec::new();
        for (r, id) in references.iter().zip(ids) {
            if id.file == ref_files.len() {
                ref_files.push(RefFile {
                    file: r.file.clone(),
                    symbols: locations.get(&r.package).and_then(|f| f.get(&r.file)),
                    file_nodes: HashMap::default(),
                });
            }
        }
        let mut word_ids: HashMap<String, Option<usize>> = HashMap::default();
        let mut folded = Folded::default();
        graph.node_word = vec![None; graph.nodes.len()];
        for (reference, id) in references.into_iter().zip(ids) {
            let file = id.file;
            let mut targets = BTreeSet::new();
            let mut grouped = None;
            let resolvable = reference.resolution != Some(Resolution::External)
                && reference.resolution != Some(Resolution::Unresolved);
            // Rows sort by what they name, so most name what the row before
            // did, and so have its targets.
            let previous = graph
                .edges
                .last()
                .filter(|last| same_targets(&last.reference, &reference, id.same_referenced));
            if let Some(last) = previous {
                grouped = last.word;
                targets.extend(&last.targets);
            } else if resolvable
                && reference.tier == Tier::Textual
                && reference.candidates.is_empty()
                && reference.referenced_type.is_none()
            {
                // The same set the general loop below would build, once per word.
                let name = folded.of(&reference.referenced);
                grouped = match word_ids.get(name) {
                    Some(&word) => word,
                    None => {
                        let ids: Vec<usize> = graph
                            .names
                            .get(name)
                            .into_iter()
                            .flatten()
                            .copied()
                            .filter(|&id| short(&graph.nodes[id].symbol).eq_ignore_ascii_case(name))
                            .collect();
                        let word = (!ids.is_empty()).then(|| {
                            let word_id = graph.words.len();
                            for &id in &ids {
                                graph.node_word[id] = Some(word_id);
                            }
                            graph.words.push(Word {
                                targets: ids,
                                edges: Vec::new(),
                            });
                            word_id
                        });
                        word_ids.insert(name.to_owned(), word);
                        word
                    }
                };
            } else if resolvable {
                let names = if reference.candidates.is_empty() {
                    vec![reference.referenced.as_str()]
                } else {
                    reference.candidates.iter().map(String::as_str).collect()
                };
                for name in names {
                    let index = if reference.tier == Tier::Textual {
                        &graph.names
                    } else {
                        &graph.exact
                    };
                    for &id in index.get(folded.of(name)).into_iter().flatten() {
                        let node = &graph.nodes[id];
                        // Textual rows match declaration *names*, not qualified
                        // symbols. Parsed metadata supplies a type namespace.
                        if reference.tier == Tier::Textual
                            && !short(&node.symbol).eq_ignore_ascii_case(name)
                        {
                            continue;
                        }
                        if let Some(kind) = reference.referenced_type {
                            let kind = kind.as_str();
                            if node.kind != kind && !(kind == "class" && is_type(&node.kind)) {
                                continue;
                            }
                        }
                        targets.insert(id);
                    }
                }
            }
            // Rows naming one thing then sort by referrer and file, so the row
            // before often has this row's sources too.
            let previous = graph
                .edges
                .len()
                .checked_sub(1)
                .filter(|&last| ids[last].file == file && id.same_referrer);
            let mut sources = match previous {
                Some(last) => graph.edges[last].sources.clone(),
                None => reference
                    .referrer
                    .as_ref()
                    .and_then(|name| ref_files[file].symbols?.get(folded.of(name)))
                    .cloned()
                    .unwrap_or_default(),
            };
            if sources.is_empty() {
                if let Some(name) = &reference.referrer {
                    sources = graph
                        .exact
                        .get(folded.of(name))
                        .into_iter()
                        .flatten()
                        .copied()
                        .filter(|&id| {
                            graph.nodes[id].kind == "lwc"
                                && graph.nodes[id].package == reference.package
                        })
                        .collect();
                }
                if sources.is_empty() {
                    let symbol = reference.referrer.as_deref().unwrap_or(&reference.file);
                    let file_nodes = &mut ref_files[file].file_nodes;
                    // Files without declarations can still refer to a seed.
                    let id = match file_nodes.get(symbol) {
                        Some(&id) => id,
                        None => {
                            file_nodes.insert(symbol.to_owned(), graph.nodes.len());
                            graph.nodes.push(Node {
                                symbol: symbol.to_owned(),
                                kind: "file".into(),
                                package: reference.package.clone(),
                                file: reference.file.clone(),
                                line: None,
                                scope: None,
                            });
                            graph.nodes.len() - 1
                        }
                    };
                    sources.push(id);
                }
            }
            if let Some(word_id) = grouped {
                graph.words[word_id].edges.push(graph.edges.len());
            }
            graph.edges.push(Edge {
                reference,
                targets: targets.into_iter().collect(),
                word: grouped,
                sources,
            });
        }
        drop(word_ids);
        graph.node_word.resize(graph.nodes.len(), None);
        graph.incoming = vec![Vec::new(); graph.nodes.len()];
        graph.outgoing = vec![Vec::new(); graph.nodes.len()];
        for (edge_id, edge) in graph.edges.iter().enumerate() {
            for &source in &edge.sources {
                graph.outgoing[source].push(edge_id);
            }
            if edge.reference.kind == RefKind::DmlObject && edge.reference.tier == Tier::Parsed {
                graph
                    .dml
                    .entry(fold(&edge.reference.referenced))
                    .or_default()
                    .push(edge_id);
            }
            for &target in &edge.targets {
                graph.incoming[target].push(edge_id);
            }
        }
        graph.owners = vec![None; graph.nodes.len()];
        graph.members = vec![Vec::new(); graph.nodes.len()];
        for (id, node) in graph.nodes.iter().enumerate() {
            let symbols = locations.get(&node.package).and_then(|f| f.get(&node.file));
            let mut name = bare(&node.symbol);
            while let Some((parent, _)) = name.rsplit_once('.') {
                if let Some(owner) = symbols
                    .and_then(|s| s.get(folded.of(parent)))
                    .and_then(|ids| ids.iter().find(|&&i| is_type(&graph.nodes[i].kind)))
                {
                    if graph.owners[id].is_none() {
                        graph.owners[id] = Some(*owner);
                    }
                    graph.members[*owner].push(id);
                }
                name = parent;
            }
        }
        // Textual rows already represent whole-word, case-insensitive matches.
        // Parsed references into a type also establish that type in the file.
        // A textual member word alone must never establish its owner.
        // Only `member_name_only` reads these, and only for a file holding an
        // `@IsTest` class, so other files are left out.
        for &test in &graph.tests {
            graph
                .file_types
                .entry(graph.nodes[test].file.clone())
                .or_default();
        }
        for (id, node) in graph.nodes.iter().enumerate() {
            if is_type(&node.kind)
                && let Some(named) = graph.file_types.get_mut(&node.file)
            {
                named.insert(id);
            }
        }
        // A textual target only names itself, so a word contributes its types.
        let word_types: Vec<Vec<usize>> = graph
            .words
            .iter()
            .map(|word| {
                word.targets
                    .iter()
                    .copied()
                    .filter(|&id| is_type(&graph.nodes[id].kind))
                    .collect()
            })
            .collect();
        // Each distinct reference file's entry, found once rather than per edge.
        let mut file_types: Vec<_> = std::mem::take(&mut graph.file_types).into_iter().collect();
        let slots: Vec<Option<usize>> = ref_files
            .iter()
            .map(|f| {
                file_types
                    .binary_search_by(|(name, _)| name.cmp(&f.file))
                    .ok()
            })
            .collect();
        for (edge, id) in graph.edges.iter().zip(ids) {
            let Some(slot) = slots[id.file] else {
                continue;
            };
            let named = &mut file_types[slot].1;
            if let Some(word) = edge.word {
                named.extend(&word_types[word]);
                continue;
            }
            for &target in &edge.targets {
                let mut id = Some(target);
                while let Some(node) = id {
                    if is_type(&graph.nodes[node].kind) {
                        named.insert(node);
                    }
                    if edge.reference.tier == Tier::Textual {
                        break;
                    }
                    id = graph.owners[node];
                }
            }
        }
        graph.file_types = file_types.into_iter().collect();
        graph
    }
    /// The targets of an edge, in ascending id, grouped or not.
    pub fn targets(&self, edge: usize) -> &[usize] {
        let edge = &self.edges[edge];
        match edge.word {
            Some(word) => &self.words[word].targets,
            None => &edge.targets,
        }
    }
    /// Every edge with a target at this node: its own edges, then its word's.
    fn incoming_edges(&self, target: usize) -> impl Iterator<Item = usize> + '_ {
        let word = self.node_word[target].map(|word| &self.words[word].edges);
        self.incoming[target]
            .iter()
            .chain(word.into_iter().flatten())
            .copied()
    }
    pub fn lookup(&self, name: &str) -> Vec<usize> {
        self.names.get(&fold(name)).cloned().unwrap_or_default()
    }
    /// The class to name in a test run for a node in an `@IsTest` class.
    /// Apex does not allow a nested `@IsTest` class, and a run cannot be
    /// given the name of one: source that has one selects the class it is in,
    /// so that no dotted name is emitted. Witnesses keep the node itself.
    pub fn test_owner(&self, mut id: usize) -> Option<usize> {
        loop {
            if self.tests.contains(&id) {
                return Some(self.top_level(id));
            }
            id = self.owners[id]?;
        }
    }
    pub fn top_level(&self, mut id: usize) -> usize {
        while let Some(owner) = self.owners[id] {
            id = owner;
        }
        id
    }
    pub fn class_owner(&self, mut id: usize) -> Option<usize> {
        loop {
            if self.nodes[id].kind == "class" {
                return Some(id);
            }
            id = self.owners[id]?;
        }
    }
    /// Only seeds expand to members, same-signature supertypes and targeted
    /// owner bindings. An otherwise uncalled method seed takes parsed owner
    /// entry references. A reached member never opens its containing type.
    pub fn incoming_to(&self, id: usize, seed: bool) -> Vec<Link> {
        let mut targets = vec![(id, Via::Reference, Vec::new())];
        if seed && is_type(&self.nodes[id].kind) {
            targets.extend(
                self.members[id]
                    .iter()
                    .map(|&m| (m, Via::Member, Vec::new())),
            );
        }
        if seed {
            for (member, _, _) in targets.clone() {
                targets.extend(self.dispatch(member));
            }
        }
        let mut links = BTreeSet::new();
        for (target, via, evidence) in &targets {
            for edge in self.incoming_edges(*target) {
                links.insert(Link {
                    edge,
                    target: *target,
                    via: *via,
                    evidence: evidence.clone(),
                });
            }
        }
        if seed
            && self.nodes[id].kind == "method"
            && let Some(owner) = self.owners[id]
        {
            for edge in self.incoming_edges(owner) {
                let e = &self.edges[edge];
                let binding = matches!(e.reference.kind, RefKind::Cmdt | RefKind::CmdtField)
                    || (self.invocable.contains(&id) && e.reference.kind == RefKind::FlowAction)
                    || (e.reference.tier == Tier::Textual
                        && e.sources
                            .iter()
                            .any(|&s| self.nodes[s].kind == "metadata_record"));
                if binding {
                    links.insert(Link {
                        edge,
                        target: owner,
                        via: Via::OwnerBinding,
                        evidence: Vec::new(),
                    });
                }
            }
            if !links
                .iter()
                .any(|link| self.edges[link.edge].reference.tier == Tier::Parsed)
            {
                // The platform's interfaces are omitted by extraction. Its
                // entry methods are exercised by constructing/running the owner.
                // Explicit constructors have their own reference targets.
                for target in
                    std::iter::once(owner).chain(self.members[owner].iter().copied().filter(|&m| {
                        self.nodes[m].kind == "constructor" && self.owners[m] == Some(owner)
                    }))
                {
                    // A word's edges are all textual, so own edges suffice.
                    for &edge in &self.incoming[target] {
                        if self.edges[edge].reference.tier == Tier::Parsed {
                            links.insert(Link {
                                edge,
                                target,
                                via: Via::PlatformEntry,
                                evidence: Vec::new(),
                            });
                        }
                    }
                }
            }
        }
        if self.nodes[id].kind == "trigger" {
            for &binding in &self.outgoing[id] {
                let r = &self.edges[binding].reference;
                if r.kind != RefKind::TriggerObject || r.tier != Tier::Parsed {
                    continue;
                }
                for &edge in self.dml.get(&fold(&r.referenced)).into_iter().flatten() {
                    links.insert(Link {
                        edge,
                        target: id,
                        via: Via::TriggerDml,
                        evidence: vec![binding],
                    });
                }
            }
        }
        links.into_iter().collect()
    }
    /// First-hop member text needs an owner (or enclosing/dispatch type) in
    /// that same file. Type-name text itself needs no extra corroboration.
    pub fn member_name_only(&self, seed: usize, link: &Link) -> bool {
        if !matches!(
            self.nodes[link.target].kind.as_str(),
            "method" | "constructor" | "field" | "enum_value"
        ) {
            return false;
        }
        let Some(named) = self.file_types.get(&self.edges[link.edge].reference.file) else {
            return true;
        };
        let owns_named = |mut id: usize| loop {
            if is_type(&self.nodes[id].kind) && named.contains(&id) {
                return true;
            }
            let Some(owner) = self.owners[id] else {
                return false;
            };
            id = owner;
        };
        !owns_named(link.target)
            && !owns_named(seed)
            && !link.evidence.iter().any(|&e| {
                self.targets(e)
                    .iter()
                    .chain(&self.edges[e].sources)
                    .any(|&id| owns_named(id))
            })
    }
    fn dispatch(&self, member: usize) -> Vec<(usize, Via, Vec<usize>)> {
        if self.nodes[member].kind != "method" {
            return Vec::new();
        }
        let Some(owner) = self.owners[member] else {
            return Vec::new();
        };
        let signature = &self.nodes[member].symbol[bare(&self.nodes[owner].symbol).len() + 1..];
        let mut stack = vec![(owner, Vec::new())];
        let mut seen = BTreeSet::from([owner]);
        let mut found = Vec::new();
        while let Some((node, path)) = stack.pop() {
            for &edge in &self.outgoing[node] {
                if !matches!(
                    self.edges[edge].reference.kind,
                    RefKind::Extends | RefKind::Implements
                ) {
                    continue;
                }
                for &parent in self.targets(edge) {
                    if !seen.insert(parent) {
                        continue;
                    }
                    let mut proof = path.clone();
                    proof.push(edge);
                    let name = fold(&format!("{}.{}", self.nodes[parent].symbol, signature));
                    for &method in self.exact.get(&name).into_iter().flatten() {
                        if self.nodes[method].file == self.nodes[parent].file
                            && self.nodes[method].kind == "method"
                        {
                            found.push((method, Via::Dispatch, proof.clone()));
                        }
                    }
                    stack.push((parent, proof));
                }
            }
        }
        found
    }
    pub fn ambiguous(&self, edge: usize) -> bool {
        let targets = self.targets(edge).len();
        let edge = &self.edges[edge];
        targets > 1
            || edge.sources.len() > 1
            || !edge.reference.candidates.is_empty()
            || edge.reference.detail.is_some()
    }
    pub fn relevant(&self, reference: &Reference, reached: &BTreeSet<usize>) -> bool {
        let mut names: Vec<&str> = std::iter::once(reference.referenced.as_str())
            .chain(reference.candidates.iter().map(String::as_str))
            .collect();
        if reference.kind == RefKind::Call {
            names.push(short(&reference.referenced));
        }
        names.into_iter().any(|name| {
            self.lookup(name).into_iter().any(|id| {
                reached.contains(&id)
                    && reference.referenced_type.is_none_or(|kind| {
                        let kind = kind.as_str();
                        self.nodes[id].kind == kind
                            || (kind == "class" && is_type(&self.nodes[id].kind))
                    })
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::{dedup_references, row_ids, sort_references};
    use sf_impact_common::rows::ReferencedType;

    fn declaration(symbol: &str, kind: DeclType, package: &str, file: &str) -> Declaration {
        Declaration {
            symbol: symbol.into(),
            name: short(symbol).into(),
            kind,
            package: package.into(),
            file: file.into(),
            start: 1,
            end: 1,
            is_test: false,
            annotations: Vec::new(),
            collision: false,
            scope: None,
        }
    }
    fn reference(
        tier: Tier,
        referenced: &str,
        referrer: Option<&str>,
        package: &str,
        file: &str,
    ) -> Reference {
        Reference {
            tier,
            kind: if tier == Tier::Parsed {
                RefKind::Call
            } else {
                RefKind::Text
            },
            referenced: referenced.into(),
            referenced_type: None,
            resolution: (tier == Tier::Parsed).then_some(Resolution::Declaration),
            detail: None,
            candidates: Vec::new(),
            referrer: referrer.map(Into::into),
            source_field: None,
            package: package.into(),
            file: file.into(),
            line: 1,
            column: 1,
        }
    }
    /// Each node as (symbol, kind, package, file), in id order.
    fn nodes(graph: &Graph) -> Vec<(&str, &str, &str, &str)> {
        graph
            .nodes
            .iter()
            .map(|n| (&*n.symbol, &*n.kind, &*n.package, &*n.file))
            .collect()
    }
    /// The graph of references in the order given, with `row_ids`' ids.
    fn graph(declarations: &[Declaration], references: Vec<Reference>) -> Graph {
        let ids = row_ids(&references);
        Graph::new(declarations, references, &ids, Vec::new())
    }
    fn id(graph: &Graph, symbol: &str, package: &str) -> usize {
        graph
            .nodes
            .iter()
            .position(|n| n.symbol == symbol && n.package == package)
            .unwrap()
    }

    #[test]
    fn a_referrer_resolves_case_insensitively_within_its_own_package_and_file() {
        let declarations = [
            declaration("Svc", DeclType::Class, "a", "a/Svc.cls"),
            declaration("Svc.run()", DeclType::Method, "a", "a/Svc.cls"),
            declaration("Svc", DeclType::Class, "b", "b/Svc.cls"),
            declaration("Svc.run()", DeclType::Method, "b", "b/Svc.cls"),
        ];
        let references = vec![
            // Same package and file, other case: that file's declaration.
            reference(Tier::Parsed, "Svc", Some("SVC.RUN()"), "b", "b/Svc.cls"),
            reference(Tier::Parsed, "Svc", Some("svc.run()"), "a", "a/Svc.cls"),
            // Same package, another file: no declaration there.
            reference(Tier::Parsed, "Svc", Some("Svc.run()"), "a", "a/Other.cls"),
            // Another package with the same file path: none either.
            reference(Tier::Parsed, "Svc", Some("Svc.run()"), "c", "a/Svc.cls"),
            // A second row from the first file-node referrer shares its node.
            reference(Tier::Parsed, "Svc", Some("Svc.run()"), "a", "a/Other.cls"),
            // Another case of that referrer is a node of its own.
            reference(Tier::Parsed, "Svc", Some("SVC.run()"), "a", "a/Other.cls"),
            // No referrer: the file is the source.
            reference(Tier::Parsed, "Svc", None, "a", "a/Other.cls"),
        ];
        let graph = graph(&declarations, references);
        let sources: Vec<&[usize]> = graph.edges.iter().map(|e| &*e.sources).collect();
        assert_eq!(
            sources,
            [
                &[id(&graph, "Svc.run()", "b")][..],
                &[id(&graph, "Svc.run()", "a")],
                &[4],
                &[5],
                &[4],
                &[6],
                &[7],
            ]
        );
        assert_eq!(
            nodes(&graph)[4..],
            [
                ("Svc.run()", "file", "a", "a/Other.cls"),
                ("Svc.run()", "file", "c", "a/Svc.cls"),
                ("SVC.run()", "file", "a", "a/Other.cls"),
                ("a/Other.cls", "file", "a", "a/Other.cls"),
            ]
        );
        // Owners resolve case-insensitively too, within the file.
        let method = id(&graph, "Svc.run()", "a");
        assert_eq!(graph.owners[method], Some(id(&graph, "Svc", "a")));
        assert_eq!(
            graph.members[id(&graph, "Svc", "b")],
            [id(&graph, "Svc.run()", "b")]
        );
    }

    #[test]
    fn an_unmatched_referrer_falls_back_to_a_same_package_lwc_of_that_name() {
        let declarations = [
            declaration("myCard", DeclType::Lwc, "a", "lwc/myCard/myCard.js"),
            declaration("myCard", DeclType::Lwc, "b", "lwc/myCard/myCard.js"),
        ];
        let references = vec![
            // The bundle's auxiliary file, its referrer in another case.
            reference(
                Tier::Textual,
                "x",
                Some("MYCARD"),
                "a",
                "lwc/myCard/myCard.html",
            ),
        ];
        let graph = graph(&declarations, references);
        assert_eq!(graph.edges[0].sources, [id(&graph, "myCard", "a")]);
        assert_eq!(graph.nodes.len(), 2);
    }

    #[test]
    fn targets_match_names_case_insensitively_and_textual_rows_by_short_name() {
        let declarations = [
            declaration("Svc", DeclType::Class, "a", "a/Svc.cls"),
            declaration("Svc.run()", DeclType::Method, "a", "a/Svc.cls"),
            declaration("Other.Run()", DeclType::Method, "a", "a/Other.cls"),
            declaration("Other", DeclType::Class, "a", "a/Other.cls"),
        ];
        let mut candidates = reference(Tier::Textual, "ignored", None, "a", "a/T.cls");
        candidates.candidates = vec!["RUN".into(), "svc".into()];
        let references = vec![
            // Parsed: exact symbol, any case.
            reference(Tier::Parsed, "svc.RUN()", None, "a", "a/T.cls"),
            // Textual words, grouped: the short name, any case.
            reference(Tier::Textual, "RUN", None, "a", "a/T.cls"),
            reference(Tier::Textual, "run", None, "a", "a/T.cls"),
            reference(Tier::Textual, "nothing", None, "a", "a/T.cls"),
            reference(Tier::Textual, "SVC", None, "a", "a/T.cls"),
            // Textual with candidates: each candidate's short name.
            candidates,
        ];
        let graph = graph(&declarations, references);
        let run = [id(&graph, "Other.Run()", "a"), id(&graph, "Svc.run()", "a")];
        let svc = id(&graph, "Svc", "a");
        let targets: Vec<Vec<usize>> = (0..graph.edges.len())
            .map(|e| graph.targets(e).to_vec())
            .collect();
        let mut both = vec![run[0], run[1], svc];
        both.sort();
        assert_eq!(
            targets,
            [
                vec![run[1]],
                run.to_vec(),
                run.to_vec(),
                vec![],
                vec![svc],
                both,
            ]
        );
        // One word per distinct folded name, in first-seen order.
        let words: Vec<Option<usize>> = graph.edges.iter().map(|e| e.word).collect();
        assert_eq!(words, [None, Some(0), Some(0), None, Some(1), None]);
        assert_eq!(graph.words[0].edges, [1, 2]);
    }

    #[test]
    fn a_row_shares_the_previous_rows_sources_only_for_its_package_file_and_referrer() {
        let declarations = [
            declaration("Svc", DeclType::Class, "a", "a/Svc.cls"),
            declaration("Svc.run()", DeclType::Method, "a", "a/Svc.cls"),
        ];
        // Consecutive rows, each differing from the one before in one way.
        let references = vec![
            reference(Tier::Parsed, "Svc", Some("Svc.run()"), "a", "a/Svc.cls"),
            reference(Tier::Parsed, "Svc", Some("Svc.run()"), "a", "a/Other.cls"),
            reference(Tier::Parsed, "Svc", Some("Svc.run()"), "a", "a/Svc.cls"),
            reference(Tier::Parsed, "Svc", Some("Svc.run()"), "c", "a/Svc.cls"),
            reference(Tier::Parsed, "Svc", Some("SVC.RUN()"), "c", "a/Svc.cls"),
            reference(Tier::Parsed, "Svc", Some("SVC.RUN()"), "a", "a/Svc.cls"),
            reference(Tier::Parsed, "Svc", None, "a", "a/Svc.cls"),
            reference(Tier::Parsed, "Svc", None, "a", "a/Svc.cls"),
        ];
        let graph = graph(&declarations, references);
        let run = id(&graph, "Svc.run()", "a");
        let sources: Vec<&[usize]> = graph.edges.iter().map(|e| &*e.sources).collect();
        assert_eq!(
            sources,
            [&[run][..], &[2], &[run], &[3], &[4], &[run], &[5], &[5]]
        );
        assert_eq!(
            nodes(&graph)[2..],
            [
                ("Svc.run()", "file", "a", "a/Other.cls"),
                ("Svc.run()", "file", "c", "a/Svc.cls"),
                ("SVC.RUN()", "file", "c", "a/Svc.cls"),
                ("a/Svc.cls", "file", "a", "a/Svc.cls"),
            ]
        );
    }

    #[test]
    fn a_row_shares_the_previous_rows_targets_only_when_it_names_the_same_thing_alike() {
        let declarations = [
            declaration("Svc", DeclType::Class, "a", "a/Svc.cls"),
            declaration("Svc.run()", DeclType::Method, "a", "a/Svc.cls"),
            declaration("Other.svc()", DeclType::Method, "a", "a/Other.cls"),
        ];
        let typed = |kind| {
            let mut r = reference(Tier::Parsed, "Svc", None, "a", "a/T.cls");
            r.referenced_type = Some(kind);
            r
        };
        let mut external = reference(Tier::Parsed, "Svc", None, "a", "a/T.cls");
        external.resolution = Some(Resolution::External);
        let mut candidates = reference(Tier::Textual, "Svc", None, "a", "a/T.cls");
        candidates.candidates = vec!["run".into()];
        let mut textual = reference(Tier::Textual, "Svc", None, "a", "a/T.cls");
        textual.resolution = Some(Resolution::Declaration);
        // Consecutive rows naming "Svc", each differing from the one before.
        let references = vec![
            reference(Tier::Parsed, "Svc", None, "a", "a/T.cls"),
            typed(ReferencedType::Method),
            external,
            reference(Tier::Parsed, "Svc", None, "a", "a/T.cls"),
            // Only the tier differs from the row before.
            textual,
            reference(Tier::Textual, "Svc", None, "a", "a/T.cls"),
            candidates,
            reference(Tier::Textual, "Svc", None, "a", "a/T.cls"),
            reference(Tier::Textual, "SVC", None, "a", "a/T.cls"),
            reference(Tier::Parsed, "Svc", None, "a", "a/T.cls"),
            // Only what is named differs from the row before.
            reference(Tier::Parsed, "Svc.run()", None, "a", "a/T.cls"),
            reference(Tier::Textual, "Svc", None, "a", "a/T.cls"),
            reference(Tier::Textual, "run", None, "a", "a/T.cls"),
        ];
        let graph = graph(&declarations, references);
        let svc = id(&graph, "Svc", "a");
        let run = id(&graph, "Svc.run()", "a");
        let mut word = vec![svc, id(&graph, "Other.svc()", "a")];
        word.sort();
        let targets: Vec<Vec<usize>> = (0..graph.edges.len())
            .map(|e| graph.targets(e).to_vec())
            .collect();
        assert_eq!(
            targets,
            [
                vec![svc],
                vec![],
                vec![],
                vec![svc],
                word.clone(),
                word.clone(),
                vec![run],
                word.clone(),
                word.clone(),
                vec![svc],
                vec![run],
                word,
                vec![run],
            ]
        );
        let words: Vec<Option<usize>> = graph.edges.iter().map(|e| e.word).collect();
        let grouped = Some(0);
        let none = None;
        assert_eq!(
            words,
            [
                none,
                none,
                none,
                none,
                grouped,
                grouped,
                none,
                grouped,
                grouped,
                none,
                none,
                grouped,
                Some(1),
            ]
        );
        assert_eq!(graph.words[0].edges, [4, 5, 7, 8, 11]);
        assert_eq!(graph.words[1].edges, [12]);
    }

    #[test]
    fn sorted_rows_share_sources_only_within_their_package_file_and_referrer() {
        let declarations = [
            declaration("Svc", DeclType::Class, "a", "a/Svc.cls"),
            declaration("Svc.run()", DeclType::Method, "a", "a/Svc.cls"),
        ];
        let mut references = vec![
            reference(Tier::Parsed, "Svc", None, "c", "a/Svc.cls"),
            reference(Tier::Parsed, "Svc", Some("Svc.run()"), "c", "a/Svc.cls"),
            reference(Tier::Parsed, "Svc", Some("Svc.run()"), "a", "a/Svc.cls"),
            reference(Tier::Parsed, "Svc", Some("Svc.run()"), "a", "a/Svc.cls"),
        ];
        let mut ids = sort_references(&mut references);
        dedup_references(&mut references, &mut ids);
        // Sorted and without the repeat: the same path in package "a", then
        // in "c", then in "c" without a referrer.
        let rows: Vec<(&str, Option<&str>)> = references
            .iter()
            .map(|r| (&*r.package, r.referrer.as_deref()))
            .collect();
        assert_eq!(
            rows,
            [
                ("a", Some("Svc.run()")),
                ("c", Some("Svc.run()")),
                ("c", None)
            ]
        );
        let graph = Graph::new(&declarations, references, &ids, Vec::new());
        let sources: Vec<&[usize]> = graph.edges.iter().map(|e| &*e.sources).collect();
        assert_eq!(sources, [&[id(&graph, "Svc.run()", "a")][..], &[2], &[3]]);
        assert_eq!(
            nodes(&graph)[2..],
            [
                ("Svc.run()", "file", "c", "a/Svc.cls"),
                ("a/Svc.cls", "file", "c", "a/Svc.cls"),
            ]
        );
    }

    #[test]
    fn a_test_files_types_come_from_rows_in_that_file_path_in_any_package() {
        let mut test = declaration("SvcTest", DeclType::Class, "a", "t/SvcTest.cls");
        test.is_test = true;
        let declarations = [
            test,
            declaration("Svc", DeclType::Class, "a", "a/Svc.cls"),
            declaration("Svc.run()", DeclType::Method, "a", "a/Svc.cls"),
            declaration("Util", DeclType::Class, "a", "u/Util.cls"),
            declaration("Other", DeclType::Class, "a", "o/Other.cls"),
            declaration("Thing", DeclType::Class, "a", "h/Thing.cls"),
        ];
        let references = vec![
            // A textual word contributes its types.
            reference(Tier::Textual, "Util", None, "a", "t/SvcTest.cls"),
            // A parsed member contributes its owner.
            reference(Tier::Parsed, "Svc.run()", None, "a", "t/SvcTest.cls"),
            // The same path in another package counts too.
            reference(Tier::Parsed, "Other", None, "b", "t/SvcTest.cls"),
            // Rows in a file without a test class are left out.
            reference(Tier::Parsed, "Thing", None, "a", "a/Svc.cls"),
        ];
        let graph = graph(&declarations, references);
        let named = BTreeSet::from(["SvcTest", "Svc", "Util", "Other"].map(|s| id(&graph, s, "a")));
        assert_eq!(
            graph.file_types,
            BTreeMap::from([("t/SvcTest.cls".to_owned(), named)])
        );
    }
}
