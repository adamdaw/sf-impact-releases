use crate::{
    HashMap,
    graph::{Graph, Link, Node, Step, Via},
    walk::Selection,
};
use serde::Serialize;
use serde_json::Value;
use sf_impact_common::rows::{Declaration, Reference, ScopeSource, Tier};
use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
};

/// Shared reference IDs are stable indices in the canonically sorted input.
/// Only rows used by direct referrers, witnesses or gaps are emitted.
#[derive(Serialize)]
pub struct Report<'a> {
    pub provenance: sf_impact_common::provenance::Impact,
    pub depth: usize,
    pub seeds: Vec<Seed<'a>>,
    pub queries: Vec<Value>,
    pub tests: Vec<Test<'a>>,
    pub possible: Vec<Test<'a>>,
    pub references: BTreeMap<usize, &'a Reference>,
    pub untested: Vec<&'a Node>,
    pub confidence: &'static str,
    pub gaps: Vec<Value>,
    pub diagnostics: BTreeMap<String, usize>,
    pub unresolved: Vec<usize>,
    pub collisions: Vec<Collision<'a>>,
    pub errors: Vec<Value>,
}
/// Declarations whose names collide. The members are in the alphabetical
/// order that a `serde_json::Value` map, which this replaced, writes.
#[derive(Serialize)]
pub struct Collision<'a> {
    pub declarations: Vec<CollisionDecl<'a>>,
    pub symbol: &'a str,
}
/// A declaration as a `Node` names it, members in alphabetical order.
#[derive(Serialize)]
pub struct CollisionDecl<'a> {
    pub file: &'a str,
    pub line: u32,
    pub package: &'a str,
    pub scope: Option<&'static str>,
    pub symbol: &'a str,
    #[serde(rename = "type")]
    pub kind: &'static str,
}
impl<'a> From<&'a Declaration> for CollisionDecl<'a> {
    fn from(d: &'a Declaration) -> Self {
        Self {
            file: &d.file,
            line: d.start,
            package: &d.package,
            scope: d.scope.map(ScopeSource::as_str),
            symbol: &d.symbol,
            kind: d.kind.as_str(),
        }
    }
}
#[derive(Serialize, Default)]
pub struct Counts {
    pub parsed: usize,
    pub textual: usize,
}
#[derive(Serialize)]
pub struct Seed<'a> {
    #[serde(flatten)]
    pub node: &'a Node,
    pub sources: Vec<Value>,
    pub counts: Counts,
    pub referrers: Vec<Group<'a>>,
}
#[derive(Serialize)]
pub struct Group<'a> {
    #[serde(rename = "type")]
    pub kind: String,
    pub parsed: Vec<Hit<'a>>,
    pub textual: Vec<Hit<'a>>,
}
#[derive(Serialize)]
pub struct Hit<'a> {
    pub referrer: Identity<'a>,
    pub references: Vec<usize>,
    pub routes: Vec<Route<'a>>,
    pub ambiguous: bool,
}
#[derive(Serialize)]
pub struct Route<'a> {
    pub target: Identity<'a>,
    pub via: Via,
    pub evidence: Vec<usize>,
}
type RouteKey = (usize, Via, Vec<usize>);
#[derive(Serialize)]
pub struct Identity<'a> {
    pub symbol: &'a str,
    pub file: &'a str,
}
impl<'a> From<&'a Node> for Identity<'a> {
    fn from(n: &'a Node) -> Self {
        Self {
            symbol: &n.symbol,
            file: &n.file,
        }
    }
}
#[derive(Serialize)]
pub struct Hop<'a> {
    pub target: Identity<'a>,
    pub referrer: Identity<'a>,
    pub reference: usize,
    pub via: Via,
    pub evidence: Vec<usize>,
    pub ambiguous: bool,
}
#[derive(Serialize)]
pub struct Test<'a> {
    #[serde(flatten)]
    pub node: &'a Node,
    pub reasons: Vec<&'static str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub convention: Vec<Identity<'a>>,
    pub seed: Identity<'a>,
    pub path: Vec<Hop<'a>>,
}
pub fn tests<'a>(
    graph: &'a Graph,
    selections: &BTreeMap<usize, Selection>,
    used: &mut BTreeSet<usize>,
) -> Vec<Test<'a>> {
    selections
        .iter()
        .map(|(&id, selection)| Test {
            node: &graph.nodes[id],
            reasons: selection.reasons.iter().copied().collect(),
            convention: selection
                .convention
                .iter()
                .map(|&test| (&graph.nodes[test]).into())
                .collect(),
            seed: (&graph.nodes[selection.seed]).into(),
            path: selection
                .path
                .iter()
                .map(|step| hop(graph, step, used))
                .collect(),
        })
        .collect()
}
fn hop<'a>(graph: &'a Graph, step: &Step, used: &mut BTreeSet<usize>) -> Hop<'a> {
    used.insert(step.link.edge);
    used.extend(&step.link.evidence);
    Hop {
        target: (&graph.nodes[step.link.target]).into(),
        referrer: (&graph.nodes[step.source]).into(),
        reference: step.link.edge,
        via: step.link.via,
        evidence: step.link.evidence.clone(),
        ambiguous: graph.ambiguous(step.link.edge),
    }
}
pub fn seed<'a>(
    graph: &'a Graph,
    id: usize,
    sources: Vec<Value>,
    links: &[Link],
    used: &mut BTreeSet<usize>,
) -> Seed<'a> {
    let mut seen = BTreeSet::new();
    let mut counts = Counts::default();
    let mut hits: BTreeMap<(usize, Tier), BTreeSet<usize>> = BTreeMap::new();
    let mut routes: BTreeMap<(usize, Tier), BTreeSet<RouteKey>> = BTreeMap::new();
    for link in links {
        let edge = &graph.edges[link.edge];
        used.insert(link.edge);
        used.extend(&link.evidence);
        if seen.insert(link.edge) {
            if edge.reference.tier == Tier::Parsed {
                counts.parsed += 1;
            } else {
                counts.textual += 1;
            }
        }
        for &source in &edge.sources {
            let key = (source, edge.reference.tier);
            hits.entry(key).or_default().insert(link.edge);
            routes
                .entry(key)
                .or_default()
                .insert((link.target, link.via, link.evidence.clone()));
        }
    }
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    for ((source, tier), references) in hits {
        let kind = graph.nodes[source].kind.clone();
        let hit = Hit {
            referrer: (&graph.nodes[source]).into(),
            ambiguous: references.iter().any(|&id| graph.ambiguous(id)),
            references: references.into_iter().collect(),
            routes: routes
                .remove(&(source, tier))
                .unwrap()
                .into_iter()
                .map(|(target, via, evidence)| Route {
                    target: (&graph.nodes[target]).into(),
                    via,
                    evidence,
                })
                .collect(),
        };
        let group = groups.entry(kind.clone()).or_insert_with(|| Group {
            kind,
            parsed: Vec::new(),
            textual: Vec::new(),
        });
        if tier == Tier::Parsed {
            group.parsed.push(hit);
        } else {
            group.textual.push(hit);
        }
    }
    Seed {
        node: &graph.nodes[id],
        sources,
        counts,
        referrers: groups.into_values().collect(),
    }
}

/// Sorts references into `serde_json::to_string` byte order, as
/// `sort_by_cached_key` did, ties keeping their input order.
///
/// Comparing two rows reads several separate strings of each, and many rows
/// share their leading members, so a direct sort spends its time waiting on
/// memory. Instead each distinct string and word is ranked once in
/// serialised order; strings write prefix-free values, so ranks order as the
/// values do. A row's key holds the ranks of its leading members and of
/// every member `cmp_same_shape` reads, so two rows of one shape without
/// candidates (nearly every tie) compare by ranks and numbers alone; any
/// other pair is compared in full. Slices too long for a key's `u32`
/// members are sorted by the comparator alone.
///
/// Gives each sorted row's ids, from its key's ranks, so that `Graph::new`
/// need not hash or compare those strings again.
pub fn sort_references(rows: &mut [Reference]) -> Vec<RowIds> {
    if !keys_fit(rows.len()) {
        rows.sort_by(cmp_reference);
        return row_ids(rows);
    }
    fn intern<'a>(ids: &mut HashMap<&'a str, u32>, s: &'a str) -> u32 {
        let next = ids.len() as u32;
        *ids.entry(s).or_insert(next)
    }
    fn text<'a>(ids: &mut HashMap<&'a str, u32>, v: &'a Option<String>) -> u32 {
        v.as_deref().map_or(u32::MAX, |s| intern(ids, s))
    }
    let mut ids = HashMap::default();
    let (mut tiers, mut kinds) = (Words::default(), Words::default());
    let (mut types, mut resolutions, mut details) =
        (Words::default(), Words::default(), Words::default());
    // Ids now, ranks below. `u32::MAX` and `ABSENT` mark a member a row
    // does not write; `null` follows every string, as `n` follows `"`.
    let mut keys: Vec<Key> = rows
        .iter()
        .enumerate()
        .map(|(index, r)| {
            let [
                referenced_type,
                resolution,
                detail,
                candidates,
                source_field,
            ] = shape(r);
            Key {
                tier: tiers.id(r.tier as u8, r.tier.as_str()),
                kind: kinds.id(r.kind as u8, r.kind.as_str()),
                referenced: intern(&mut ids, &r.referenced),
                referenced_type: r
                    .referenced_type
                    .map_or(ABSENT, |v| types.id(v as u8, v.as_str())),
                resolution: r
                    .resolution
                    .map_or(ABSENT, |v| resolutions.id(v as u8, v.as_str())),
                detail: r.detail.map_or(ABSENT, |v| details.id(v as u8, v.as_str())),
                referrer: text(&mut ids, &r.referrer),
                source_field: text(&mut ids, &r.source_field),
                package: intern(&mut ids, &r.package),
                file: intern(&mut ids, &r.file),
                line: r.line,
                column: r.column,
                shape: u8::from(referenced_type)
                    | u8::from(resolution) << 1
                    | u8::from(detail) << 2
                    | u8::from(candidates) << 3
                    | u8::from(source_field) << 4,
                index: index as u32,
            }
        })
        .collect();
    let mut distinct: Vec<(&str, u32)> = ids.into_iter().collect();
    distinct.sort_unstable_by(|a, b| cmp_json_str(a.0, b.0));
    let mut rank = vec![0; distinct.len()];
    for (position, (_, id)) in distinct.into_iter().enumerate() {
        rank[id as usize] = position as u32;
    }
    let rank = |id: u32| {
        if id == u32::MAX {
            id
        } else {
            rank[id as usize]
        }
    };
    let (tiers, kinds) = (tiers.ranks(), kinds.ranks());
    let (types, resolutions, details) = (types.ranks(), resolutions.ranks(), details.ranks());
    for key in &mut keys {
        key.tier = tiers[usize::from(key.tier)];
        key.kind = kinds[usize::from(key.kind)];
        key.referenced = rank(key.referenced);
        key.referenced_type = types[usize::from(key.referenced_type)];
        key.resolution = resolutions[usize::from(key.resolution)];
        key.detail = details[usize::from(key.detail)];
        key.referrer = rank(key.referrer);
        key.source_field = rank(key.source_field);
        key.package = rank(key.package);
        key.file = rank(key.file);
    }
    keys.sort_unstable_by(|a, b| {
        (a.tier, a.kind, a.referenced)
            .cmp(&(b.tier, b.kind, b.referenced))
            .then_with(|| {
                if a.shape == b.shape && a.shape & CANDIDATES == 0 {
                    cmp_same_shape_keys(a, b)
                } else {
                    cmp_reference(&rows[a.index as usize], &rows[b.index as usize])
                }
            })
            .then(a.index.cmp(&b.index))
    });
    let mut files: HashMap<u64, usize> = HashMap::default();
    let mut before: Option<&Key> = None;
    let ids = keys
        .iter()
        .map(|key| {
            let next = files.len();
            let file = u64::from(key.package) << 32 | u64::from(key.file);
            let row = RowIds {
                file: *files.entry(file).or_insert(next),
                same_referenced: before.is_some_and(|b| b.referenced == key.referenced),
                same_referrer: before.is_some_and(|b| b.referrer == key.referrer),
            };
            before = Some(key);
            row
        })
        .collect();
    // Row `order[i]` belongs at `i`. Earlier swaps moved any row from
    // before `i`, so follow where it went, as `sort_by_cached_key` does.
    let mut order: Vec<usize> = keys.into_iter().map(|key| key.index as usize).collect();
    for i in 0..rows.len() {
        let mut index = order[i];
        while index < i {
            index = order[index];
        }
        order[i] = index;
        rows.swap(i, index);
    }
    ids
}
/// What `Graph::new` compares of a row, without its strings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowIds {
    /// The row's package and file, numbered in first-seen order.
    pub file: usize,
    /// Whether the row's referenced, and its referrer, are the row before's.
    pub same_referenced: bool,
    pub same_referrer: bool,
}
/// Drops repeated rows, as `dedup` does, and their ids with them.
/// The numbers stay in first-seen order: a dropped row repeats the row
/// kept before it.
pub fn dedup_references(rows: &mut Vec<Reference>, ids: &mut Vec<RowIds>) {
    let keep: Vec<bool> = (0..rows.len())
        .map(|i| i == 0 || rows[i] != rows[i - 1])
        .collect();
    let mut kept = keep.iter();
    rows.retain(|_| *kept.next().unwrap());
    let mut kept = keep.iter();
    ids.retain(|_| *kept.next().unwrap());
}
/// The ids of rows in any order, from their strings.
pub fn row_ids(rows: &[Reference]) -> Vec<RowIds> {
    let mut files: HashMap<(&str, &str), usize> = HashMap::default();
    let mut before: Option<&Reference> = None;
    rows.iter()
        .map(|r| {
            let next = files.len();
            let row = RowIds {
                file: *files.entry((&r.package, &r.file)).or_insert(next),
                same_referenced: before.is_some_and(|b| b.referenced == r.referenced),
                same_referrer: before.is_some_and(|b| b.referrer == r.referrer),
            };
            before = Some(r);
            row
        })
        .collect()
}
/// Whether a key's `u32` members hold every row index and string id of a
/// slice of `rows` rows. Each row interns at most `STRINGS_PER_ROW` strings,
/// and `u32::MAX` marks a member a row does not write, so no id may reach it.
fn keys_fit(rows: usize) -> bool {
    rows <= u32::MAX as usize / STRINGS_PER_ROW
}
/// Referenced, referrer, source field, package and file.
const STRINGS_PER_ROW: usize = 5;
/// `cmp_same_shape` on two keys: the same members in the same order, each
/// by its rank, and the numbers as written.
fn cmp_same_shape_keys(a: &Key, b: &Key) -> Ordering {
    let members = |k: &Key| {
        (
            k.referenced_type,
            k.resolution,
            k.detail,
            k.referrer,
            k.source_field,
            k.package,
            k.file,
        )
    };
    members(a)
        .cmp(&members(b))
        .then_with(|| cmp_value(Json::Num(a.line), b',', Json::Num(b.line), b','))
        .then_with(|| cmp_value(Json::Num(a.column), b'}', Json::Num(b.column), b'}'))
}
/// A row's sort key: the ids, then the ranks, of its members.
struct Key {
    tier: u8,
    kind: u8,
    referenced: u32,
    referenced_type: u8,
    resolution: u8,
    detail: u8,
    referrer: u32,
    source_field: u32,
    package: u32,
    file: u32,
    line: u32,
    column: u32,
    /// `shape` as bits, in its order.
    shape: u8,
    index: u32,
}
/// The `shape` bit of a row with candidates, which its key cannot rank.
const CANDIDATES: u8 = 1 << 3;
/// The id and rank of a word a row does not write: no variant's index.
const ABSENT: u8 = u8::MAX;
/// The words of one enum seen so far, by variant index, to rank in
/// serialised order.
struct Words([Option<&'static str>; 256]);
impl Default for Words {
    fn default() -> Self {
        Self([None; 256])
    }
}
impl Words {
    fn id(&mut self, index: u8, word: &'static str) -> u8 {
        self.0[usize::from(index)] = Some(word);
        index
    }
    fn ranks(&self) -> [u8; 256] {
        let mut seen: Vec<(u8, &str)> = (0..=u8::MAX)
            .zip(self.0)
            .filter_map(|(index, word)| Some((index, word?)))
            .collect();
        seen.sort_unstable_by(|a, b| cmp_json_str(a.1, b.1));
        let mut ranks = [ABSENT; 256];
        for (rank, (index, _)) in seen.into_iter().enumerate() {
            ranks[usize::from(index)] = rank as u8;
        }
        ranks
    }
}

/// One member's value as serde_json writes it.
#[derive(Clone, Copy)]
enum Json<'a> {
    Null,
    Str(&'a str),
    Num(u32),
    Strs(&'a [String]),
}
type Member<'a> = Option<(&'static str, Json<'a>)>;

/// The members `serde_json::to_string` writes for a reference, in its order:
/// a skipped field is `None`. Naming every field makes a new one a compile
/// error here rather than a silent change of order.
fn members(r: &Reference) -> [Member<'_>; 13] {
    let Reference {
        tier,
        kind,
        referenced,
        referenced_type,
        resolution,
        detail,
        candidates,
        referrer,
        source_field,
        package,
        file,
        line,
        column,
    } = r;
    let word = |key, value: Option<&'static str>| value.map(|v| (key, Json::Str(v)));
    [
        Some(("tier", Json::Str(tier.as_str()))),
        Some(("kind", Json::Str(kind.as_str()))),
        Some(("referenced", Json::Str(referenced))),
        word("referenced_type", referenced_type.map(|v| v.as_str())),
        word("resolution", resolution.map(|v| v.as_str())),
        word("detail", detail.map(|v| v.as_str())),
        (!candidates.is_empty()).then_some(("candidates", Json::Strs(candidates))),
        Some((
            "referrer",
            referrer.as_deref().map_or(Json::Null, Json::Str),
        )),
        source_field
            .as_deref()
            .map(|v| ("source_field", Json::Str(v))),
        Some(("package", Json::Str(package))),
        Some(("file", Json::Str(file))),
        Some(("line", Json::Num(*line))),
        Some(("column", Json::Num(*column))),
    ]
}

/// The order of `serde_json::to_string(a)` and `(b)` as bytes, without
/// writing either. Two serialisations agree up to the first member where
/// they differ, and the first differing byte lies within that member, or
/// (for a number) on the byte after it.
fn cmp_reference(a: &Reference, b: &Reference) -> Ordering {
    // Most pairs differ early: compare the leading members directly.
    let leading = cmp_json_str(a.tier.as_str(), b.tier.as_str())
        .then_with(|| cmp_json_str(a.kind.as_str(), b.kind.as_str()))
        .then_with(|| cmp_json_str(&a.referenced, &b.referenced));
    if leading.is_ne() {
        return leading;
    }
    if shape(a) == shape(b) {
        return cmp_same_shape(a, b);
    }
    let (a, b) = (members(a), members(b));
    let mut a = a[3..].iter().flatten().peekable();
    let mut b = b[3..].iter().flatten().peekable();
    loop {
        let (key_a, value_a, key_b, value_b) = match (a.next(), b.next()) {
            (Some(&(key_a, value_a)), Some(&(key_b, value_b))) => (key_a, value_a, key_b, value_b),
            (None, None) => return Ordering::Equal,
            // The one that ends writes `}` where the other writes `,`.
            (None, Some(_)) => return Ordering::Greater,
            (Some(_), None) => return Ordering::Less,
        };
        if key_a != key_b {
            return cmp_json_str(key_a, key_b);
        }
        // What follows a value: `,` before another member, `}` at the end.
        let after = |more: bool| if more { b',' } else { b'}' };
        let order = cmp_value(
            value_a,
            after(a.peek().is_some()),
            value_b,
            after(b.peek().is_some()),
        );
        if order.is_ne() {
            return order;
        }
    }
}

/// Which optional members a reference writes.
fn shape(r: &Reference) -> [bool; 5] {
    [
        r.referenced_type.is_some(),
        r.resolution.is_some(),
        r.detail.is_some(),
        !r.candidates.is_empty(),
        r.source_field.is_some(),
    ]
}
/// Rows of one shape write the same keys, so after the leading members
/// their values alone decide, in member order. An absent value is absent
/// from both rows; `Null` stands for it here and compares `Equal`.
fn cmp_same_shape(a: &Reference, b: &Reference) -> Ordering {
    fn word(v: Option<&'static str>) -> Json<'static> {
        v.map_or(Json::Null, Json::Str)
    }
    fn text(v: &Option<String>) -> Json<'_> {
        v.as_deref().map_or(Json::Null, Json::Str)
    }
    fn value(a: Json, b: Json) -> Ordering {
        cmp_value(a, b',', b, b',')
    }
    value(
        word(a.referenced_type.map(|v| v.as_str())),
        word(b.referenced_type.map(|v| v.as_str())),
    )
    .then_with(|| {
        value(
            word(a.resolution.map(|v| v.as_str())),
            word(b.resolution.map(|v| v.as_str())),
        )
    })
    .then_with(|| {
        value(
            word(a.detail.map(|v| v.as_str())),
            word(b.detail.map(|v| v.as_str())),
        )
    })
    .then_with(|| value(Json::Strs(&a.candidates), Json::Strs(&b.candidates)))
    .then_with(|| value(text(&a.referrer), text(&b.referrer)))
    .then_with(|| value(text(&a.source_field), text(&b.source_field)))
    .then_with(|| cmp_json_str(&a.package, &b.package))
    .then_with(|| cmp_json_str(&a.file, &b.file))
    .then_with(|| value(Json::Num(a.line), Json::Num(b.line)))
    // The last member is followed by `}`.
    .then_with(|| cmp_value(Json::Num(a.column), b'}', Json::Num(b.column), b'}'))
}

/// Equal values are `Equal` whatever follows them. Unequal values differ
/// within their own bytes, except a number whose digits begin the other's:
/// then the byte after the shorter one decides.
fn cmp_value(a: Json, after_a: u8, b: Json, after_b: u8) -> Ordering {
    match (a, b) {
        (Json::Null, Json::Null) => Ordering::Equal,
        (Json::Str(a), Json::Str(b)) => cmp_json_str(a, b),
        (Json::Num(a), Json::Num(b)) if a == b => Ordering::Equal,
        (Json::Num(a), Json::Num(b)) => {
            let (mut buf_a, mut buf_b) = ([0; 11], [0; 11]);
            digits(a, after_a, &mut buf_a).cmp(digits(b, after_b, &mut buf_b))
        }
        (Json::Strs(a), Json::Strs(b)) => {
            for (a, b) in a.iter().zip(b) {
                let order = cmp_json_str(a, b);
                if order.is_ne() {
                    return order;
                }
            }
            // The shorter list writes `]` where the longer writes `,`.
            b.len().cmp(&a.len())
        }
        // Different kinds of value differ in their first byte.
        (a, b) => first_byte(a).cmp(&first_byte(b)),
    }
}
fn first_byte(value: Json) -> u8 {
    match value {
        Json::Null => b'n',
        Json::Str(_) => b'"',
        // Every digit lies between `"` and `[`, so any digit orders the same.
        Json::Num(_) => b'0',
        Json::Strs(_) => b'[',
    }
}
/// A number's decimal spelling, then the byte written after it.
fn digits(mut n: u32, after: u8, buf: &mut [u8; 11]) -> &[u8] {
    let mut start = buf.len() - 1;
    buf[start] = after;
    loop {
        start -= 1;
        buf[start] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            return &buf[start..];
        }
    }
}

/// The order of two strings as serde_json writes them, quotes and escapes
/// included. Escaping maps each byte on its own, so the strings order by
/// their first differing byte, where the shorter string's next byte is its
/// closing quote. Neither that quote nor two different escapes can begin
/// the same way, so those bytes alone decide.
fn cmp_json_str(a: &str, b: &str) -> Ordering {
    if a == b {
        return Ordering::Equal;
    }
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let at = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let (mut buf_a, mut buf_b) = ([0; 6], [0; 6]);
    escaped(a.get(at).copied(), &mut buf_a).cmp(escaped(b.get(at).copied(), &mut buf_b))
}
/// How serde_json writes a byte of a string; `None` is the closing quote.
fn escaped(byte: Option<u8>, buf: &mut [u8; 6]) -> &[u8] {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let short = match byte {
        None => return b"\"",
        Some(b'"') => b'"',
        Some(b'\\') => b'\\',
        Some(0x08) => b'b',
        Some(0x0c) => b'f',
        Some(b'\n') => b'n',
        Some(b'\r') => b'r',
        Some(b'\t') => b't',
        Some(c @ 0..0x20) => {
            *buf = [
                b'\\',
                b'u',
                b'0',
                b'0',
                HEX[usize::from(c >> 4)],
                HEX[usize::from(c & 0xf)],
            ];
            return buf;
        }
        Some(c) => {
            buf[0] = c;
            return &buf[..1];
        }
    };
    buf[..2].copy_from_slice(&[b'\\', short]);
    &buf[..2]
}

pub fn cmp_declaration(a: &Declaration, b: &Declaration) -> Ordering {
    (
        &a.symbol,
        a.kind.as_str(),
        &a.package,
        &a.file,
        a.start,
        a.scope.map(|v| v.as_str()),
    )
        .cmp(&(
            &b.symbol,
            b.kind.as_str(),
            &b.package,
            &b.file,
            b.start,
            b.scope.map(|v| v.as_str()),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_impact_common::rows::{DeclType, Detail, RefKind, ReferencedType, Resolution, Row};

    fn json(value: &impl Serialize) -> String {
        serde_json::to_string(value).unwrap()
    }
    /// Every row after the provenance line of each refs fixture.
    fn fixture_rows() -> Vec<Row> {
        let root = env!("CARGO_MANIFEST_DIR");
        let files = [
            format!("{root}/../../fixtures/refs.jsonl"),
            format!("{root}/tests/data/npsp-bdi-start.refs.jsonl"),
        ];
        files
            .iter()
            .flat_map(|path| {
                let text = std::fs::read_to_string(path).unwrap();
                let rows: Vec<Row> = text
                    .lines()
                    .skip(1)
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect();
                rows
            })
            .collect()
    }

    #[test]
    fn reference_order_is_serialised_order_for_every_fixture_pair() {
        let references: Vec<Reference> = fixture_rows()
            .into_iter()
            .filter_map(|row| match row {
                Row::Reference(r) => Some(r),
                _ => None,
            })
            .collect();
        // The control: the fixtures hold the shapes this test is about.
        assert!(references.len() > 650, "{}", references.len());
        assert!(references.iter().any(|r| !r.candidates.is_empty()));
        assert!(references.iter().any(|r| r.source_field.is_some()));
        assert!(references.iter().any(|r| r.referrer.is_none()));
        assert!(references.iter().any(|r| r.referenced_type.is_none()));
        let keys: Vec<String> = references.iter().map(json).collect();
        for (a, ka) in references.iter().zip(&keys) {
            for (b, kb) in references.iter().zip(&keys) {
                assert_eq!(cmp_reference(a, b), ka.cmp(kb), "{ka}\n{kb}");
            }
        }
        let mut by_key = references.clone();
        by_key.sort_by_cached_key(json);
        let mut by_cmp = references;
        by_cmp.sort_by(cmp_reference);
        assert!(by_key == by_cmp);
    }

    #[test]
    fn declaration_order_is_node_order_for_every_fixture_pair() {
        let declarations: Vec<Declaration> = fixture_rows()
            .into_iter()
            .filter_map(|row| match row {
                Row::Declaration(d) => Some(d),
                _ => None,
            })
            .collect();
        assert!(declarations.len() > 150, "{}", declarations.len());
        for a in &declarations {
            for b in &declarations {
                let expected = Node::declaration(a).cmp(&Node::declaration(b));
                assert_eq!(cmp_declaration(a, b), expected, "{a:?}\n{b:?}");
            }
        }
    }

    #[test]
    fn collision_bytes_are_the_value_map_bytes_for_every_fixture_declaration() {
        let mut declarations: Vec<Declaration> = fixture_rows()
            .into_iter()
            .filter_map(|row| match row {
                Row::Declaration(d) => Some(d),
                _ => None,
            })
            .collect();
        // A declaration without a scope, which writes `null`, and one with
        // characters that serde_json escapes.
        let mut odd = declarations[0].clone();
        odd.scope = None;
        odd.symbol = "a\"b\\c\n\u{1}é".into();
        declarations.push(odd);
        for d in &declarations {
            let typed = Collision {
                symbol: &d.symbol,
                declarations: vec![d.into(), d.into()],
            };
            let value = serde_json::json!({
                "symbol": d.symbol,
                "declarations": [Node::declaration(d), Node::declaration(d)],
            });
            assert_eq!(json(&typed), value.to_string());
        }
    }

    /// One string per byte that a serialiser treats differently, alone, after
    /// a common prefix, and before a common suffix.
    fn edge_strings() -> Vec<String> {
        let mut chars: Vec<char> = (0..=0x7f_u8).map(char::from).collect();
        chars.extend(['\u{80}', 'é', '€', '\u{ffff}', '😀']);
        let mut strings = vec![String::new()];
        for c in chars {
            strings.extend([c.to_string(), format!("a{c}"), format!("a{c}b")]);
        }
        strings
    }

    #[test]
    fn string_order_is_serialised_order_for_every_edge_pair() {
        let strings = edge_strings();
        let keys: Vec<String> = strings.iter().map(json).collect();
        for (a, ka) in strings.iter().zip(&keys) {
            for (b, kb) in strings.iter().zip(&keys) {
                assert_eq!(cmp_json_str(a, b), ka.cmp(kb), "{ka} {kb}");
            }
        }
    }

    /// A small deterministic generator: the same cases on every run.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: usize) -> usize {
            (self.next() % n as u64) as usize
        }
        fn pick<T: Copy>(&mut self, items: &[T]) -> T {
            items[self.below(items.len())]
        }
        fn chance(&mut self) -> bool {
            self.next() & 1 == 0
        }
        fn string(&mut self) -> String {
            const PIECES: &[&str] = &[
                "",
                "a",
                "A",
                "z",
                "_",
                "0",
                "9",
                " ",
                "!",
                "#",
                "/",
                "~",
                "\"",
                "\\",
                "\n",
                "\t",
                "\r",
                "\u{8}",
                "\u{c}",
                "\u{0}",
                "\u{1}",
                "\u{1f}",
                "\u{7f}",
                "é",
                "€",
                "😀",
                "Account",
                "Account.Name",
            ];
            (0..self.below(4)).map(|_| self.pick(PIECES)).collect()
        }
        fn number(&mut self) -> u32 {
            const EDGES: &[u32] = &[
                0, 1, 2, 9, 10, 11, 19, 20, 99, 100, 101, 999, 1000, 1001, 65535, 99999, 100000,
                4294967295,
            ];
            if self.chance() {
                self.pick(EDGES)
            } else {
                self.next() as u32
            }
        }
        fn optional<T>(&mut self, value: impl FnOnce(&mut Self) -> T) -> Option<T> {
            self.chance().then(|| value(self))
        }
    }
    const TIERS: &[Tier] = &[Tier::Parsed, Tier::Textual];
    // Words where one is a prefix of another, or nearly so.
    const KINDS: &[RefKind] = &[
        RefKind::Call,
        RefKind::Cmdt,
        RefKind::CmdtField,
        RefKind::CmdtType,
        RefKind::Type,
        RefKind::Text,
        RefKind::Lookup,
        RefKind::LookupFilter,
        RefKind::LwcObject,
    ];
    const TYPES: &[ReferencedType] = &[
        ReferencedType::Class,
        ReferencedType::Object,
        ReferencedType::ObjectField,
        ReferencedType::Field,
        ReferencedType::Flow,
        ReferencedType::Lwc,
    ];
    const RESOLUTIONS: &[Resolution] = &[
        Resolution::Declaration,
        Resolution::External,
        Resolution::Unresolved,
    ];
    const DETAILS: &[Detail] = &[
        Detail::AmbiguousOverload,
        Detail::AmbiguousReceiver,
        Detail::UnknownType,
        Detail::NoMember,
    ];
    /// Any combination of fields, in or out of contract: the order is defined
    /// for every value the type can hold.
    fn reference(rng: &mut Rng) -> Reference {
        Reference {
            tier: rng.pick(TIERS),
            kind: rng.pick(KINDS),
            referenced: rng.string(),
            referenced_type: rng.optional(|r| r.pick(TYPES)),
            resolution: rng.optional(|r| r.pick(RESOLUTIONS)),
            detail: rng.optional(|r| r.pick(DETAILS)),
            candidates: (0..rng.below(4)).map(|_| rng.string()).collect(),
            referrer: rng.optional(Rng::string),
            source_field: rng.optional(Rng::string),
            package: rng.string(),
            file: rng.string(),
            line: rng.number(),
            column: rng.number(),
        }
    }
    /// A copy with some fields drawn afresh, so most pairs share a long prefix.
    fn mutated(rng: &mut Rng, base: &Reference) -> Reference {
        let fresh = reference(rng);
        let mut v = base.clone();
        macro_rules! maybe {
            ($($field:ident),*) => {$(if rng.below(4) == 0 { v.$field = fresh.$field.clone(); })*};
        }
        maybe!(
            tier,
            kind,
            referenced,
            referenced_type,
            resolution,
            detail,
            candidates,
            referrer,
            source_field,
            package,
            file,
            line,
            column
        );
        if rng.below(4) == 0 && !v.candidates.is_empty() {
            // Candidates that differ only in their last element, or in length.
            if rng.chance() {
                v.candidates.pop();
            } else {
                let last = v.candidates.len() - 1;
                v.candidates[last] = rng.string();
            }
        }
        v
    }

    #[test]
    fn reference_order_is_serialised_order_for_generated_pairs() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for _ in 0..200_000 {
            let a = reference(&mut rng);
            let b = mutated(&mut rng, &a);
            let (ka, kb) = (json(&a), json(&b));
            assert_eq!(cmp_reference(&a, &b), ka.cmp(&kb), "{ka}\n{kb}");
            assert_eq!(cmp_reference(&b, &a), kb.cmp(&ka), "{kb}\n{ka}");
            assert_eq!(cmp_reference(&a, &a.clone()), Ordering::Equal, "{ka}");
        }
    }

    #[test]
    fn reference_order_is_serialised_order_for_every_optional_field_combination() {
        let base = Reference {
            tier: Tier::Parsed,
            kind: RefKind::Cmdt,
            referenced: "a".into(),
            referenced_type: None,
            resolution: None,
            detail: None,
            candidates: Vec::new(),
            referrer: None,
            source_field: None,
            package: "p".into(),
            file: "f".into(),
            line: 1,
            column: 1,
        };
        let shapes: Vec<Reference> = (0..64_u32)
            .flat_map(|mask| {
                let on = |bit: u32| mask & (1 << bit) != 0;
                let mut v = base.clone();
                v.referenced_type = on(0).then_some(ReferencedType::Object);
                v.resolution = on(1).then_some(Resolution::Declaration);
                v.detail = on(2).then_some(Detail::AmbiguousReceiver);
                v.candidates = if on(3) {
                    vec!["a".into(), "b".into()]
                } else {
                    Vec::new()
                };
                v.referrer = on(4).then(|| "r".into());
                v.source_field = on(5).then(|| "s".into());
                // Each shape also with an end position whose spelling is a prefix.
                let mut longer = v.clone();
                longer.column = 10;
                [v, longer]
            })
            .collect();
        for a in &shapes {
            for b in &shapes {
                assert_eq!(
                    cmp_reference(a, b),
                    json(a).cmp(&json(b)),
                    "{}\n{}",
                    json(a),
                    json(b)
                );
            }
        }
    }

    /// What the comparator replaced, as the oracle for the whole sort.
    fn sorted_by_serialised_key(rows: &[Reference]) -> Vec<Reference> {
        let mut rows = rows.to_vec();
        rows.sort_by_cached_key(json);
        rows
    }

    #[test]
    fn sort_references_is_the_serialised_key_sort_for_the_fixtures() {
        let mut rows: Vec<Reference> = fixture_rows()
            .into_iter()
            .filter_map(|row| match row {
                Row::Reference(r) => Some(r),
                _ => None,
            })
            .collect();
        rows.reverse();
        let expected = sorted_by_serialised_key(&rows);
        sort_references(&mut rows);
        assert!(rows == expected);
    }

    #[test]
    fn sort_references_is_the_serialised_key_sort_for_generated_rows() {
        let mut rng = Rng(0xd1b5_4a32_d192_ed03);
        for _ in 0..500 {
            // Rows that share leading members, plain and not, with repeats.
            let mut rows: Vec<Reference> = Vec::new();
            for _ in 0..rng.below(60) {
                let row = match rows.len() {
                    0 => reference(&mut rng),
                    n => {
                        let base = rows[rng.below(n)].clone();
                        match rng.below(4) {
                            0 => base,
                            1 => reference(&mut rng),
                            _ => mutated(&mut rng, &base),
                        }
                    }
                };
                rows.push(row);
            }
            // Half the rows without optional members, which sort by rank.
            for row in &mut rows {
                if rng.chance() {
                    row.referenced_type = None;
                    row.resolution = None;
                    row.detail = None;
                    row.candidates.clear();
                    row.source_field = None;
                }
            }
            let expected = sorted_by_serialised_key(&rows);
            sort_references(&mut rows);
            assert!(rows == expected, "{rows:#?}");
        }
    }

    /// Large groups of rows that tie on their leading members, as a real
    /// pack has them: the shapes the extractor writes, few referrers,
    /// packages and files (some needing escapes), positions whose spellings
    /// are prefixes of one another, and exact duplicates.
    fn large_group(rng: &mut Rng, n: usize) -> Vec<Reference> {
        let pool =
            |rng: &mut Rng, k: usize| -> Vec<String> { (0..k).map(|_| rng.string()).collect() };
        let (referenced, referrers, fields) = (pool(rng, 3), pool(rng, 12), pool(rng, 3));
        let (packages, files) = (pool(rng, 3), pool(rng, 8));
        const POSITIONS: &[u32] = &[1, 2, 9, 10, 11, 19, 100, 101, 1000, 4294967295];
        let any = |rng: &mut Rng, pool: &[String]| pool[rng.below(pool.len())].clone();
        let mut rows: Vec<Reference> = Vec::with_capacity(n);
        while rows.len() < n {
            if !rows.is_empty() && rng.below(8) == 0 {
                let repeat = rows[rng.below(rows.len())].clone();
                rows.push(repeat);
                continue;
            }
            let mut r = reference(rng);
            r.tier = if rng.below(4) == 0 {
                Tier::Textual
            } else {
                Tier::Parsed
            };
            r.kind = rng.pick(&[RefKind::Call, RefKind::Type, RefKind::Text]);
            r.referenced = any(rng, &referenced);
            // Plain, typed and resolved, then with a detail, a source field
            // or candidates on top.
            let shape = rng.below(8);
            r.referenced_type = (shape > 1).then(|| rng.pick(TYPES));
            r.resolution = (shape > 0).then(|| rng.pick(RESOLUTIONS));
            r.detail = (shape > 4).then(|| rng.pick(DETAILS));
            r.source_field = (shape == 4).then(|| any(rng, &fields));
            if shape != 7 {
                r.candidates.clear();
            }
            r.referrer = (rng.below(8) != 0).then(|| any(rng, &referrers));
            r.package = any(rng, &packages);
            r.file = any(rng, &files);
            r.line = rng.pick(POSITIONS);
            r.column = rng.pick(POSITIONS);
            rows.push(r);
        }
        rows
    }

    #[test]
    fn sort_references_is_the_comparator_sort_for_large_groups() {
        let mut rng = Rng(0x6a09_e667_f3bc_c908);
        for n in [2_000, 20_000, 60_000] {
            let mut rows = large_group(&mut rng, n);
            // The control: the groups are large, and hold every shape and
            // the ties this test is about.
            let heads: BTreeSet<_> = rows
                .iter()
                .map(|r| (r.tier.as_str(), r.kind.as_str(), r.referenced.clone()))
                .collect();
            assert!(
                heads.len() <= 18 && n / heads.len() >= 100,
                "{}",
                heads.len()
            );
            let shapes: BTreeSet<_> = rows.iter().map(shape).collect();
            assert_eq!(shapes.len(), 6, "{shapes:?}");
            assert!(rows.iter().any(|r| r.referrer.is_none()));
            let mut expected = rows.clone();
            expected.sort_by(cmp_reference);
            let distinct = expected.windows(2).filter(|w| w[0] != w[1]).count();
            assert!(distinct < n - n / 10, "{distinct} of {n}");
            sort_references(&mut rows);
            assert!(rows == expected, "{n} rows");
            // The comparator itself, on neighbours: serialised order.
            for w in expected.windows(2).step_by(97) {
                assert!(
                    json(&w[0]) <= json(&w[1]),
                    "{}\n{}",
                    json(&w[0]),
                    json(&w[1])
                );
            }
        }
    }

    /// Asserts that `ids` keep `RowIds`' promise for `rows`.
    fn assert_ids_follow_strings(rows: &[Reference], ids: &[RowIds]) {
        assert_eq!(ids.len(), rows.len());
        let mut seen: Vec<(&str, &str)> = Vec::new();
        for (i, (r, id)) in rows.iter().zip(ids).enumerate() {
            let file = (&*r.package, &*r.file);
            let first = seen.iter().position(|&f| f == file).unwrap_or_else(|| {
                seen.push(file);
                seen.len() - 1
            });
            assert_eq!(id.file, first, "row {i}: {r:?}");
            let before = i.checked_sub(1).map(|b| &rows[b]);
            assert_eq!(
                id.same_referenced,
                before.is_some_and(|b| b.referenced == r.referenced),
                "row {i}"
            );
            assert_eq!(
                id.same_referrer,
                before.is_some_and(|b| b.referrer == r.referrer),
                "row {i}"
            );
        }
    }

    /// Rows of `large_group` whose package and file each also come with
    /// another: the same path in two packages, and two paths in one.
    fn large_group_with_shared_paths(rng: &mut Rng) -> Vec<Reference> {
        let rows = large_group(rng, 2_000);
        let some = |same_file: bool| {
            rows.iter().any(|a| {
                rows.iter().any(|b| {
                    (a.file == b.file) == same_file && (a.package == b.package) != same_file
                })
            })
        };
        assert!(some(true) && some(false));
        rows
    }

    #[test]
    fn sort_references_numbers_files_and_marks_repeats_of_the_row_before() {
        let mut rows = large_group_with_shared_paths(&mut Rng(0xbb67_ae85_84ca_a73b));
        let ids = sort_references(&mut rows);
        assert_ids_follow_strings(&rows, &ids);
    }

    #[test]
    fn row_ids_number_files_and_mark_repeats_of_the_row_before_in_any_order() {
        let rows = large_group_with_shared_paths(&mut Rng(0x3c6e_f372_fe94_f82b));
        assert_ids_follow_strings(&rows, &row_ids(&rows));
    }

    #[test]
    fn dedup_references_drops_each_repeated_row_with_its_ids() {
        let mut rows = large_group(&mut Rng(0xa54f_f53a_5f1d_36f1), 2_000);
        let mut ids = sort_references(&mut rows);
        let mut expected: Vec<(Reference, RowIds)> =
            rows.iter().cloned().zip(ids.iter().copied()).collect();
        expected.dedup_by(|a, b| a.0 == b.0);
        // The control: there are repeats to drop.
        assert!(expected.len() < rows.len() - 100);
        dedup_references(&mut rows, &mut ids);
        assert_ids_follow_strings(&rows, &ids);
        let kept: Vec<(Reference, RowIds)> = rows.into_iter().zip(ids).collect();
        assert!(kept == expected);
    }

    #[test]
    fn keys_fit_while_every_row_index_and_string_id_is_below_u32_max() {
        let most = u32::MAX as usize / STRINGS_PER_ROW;
        assert!(keys_fit(0));
        assert!(keys_fit(most));
        // The last id the largest slice can intern is below the marker.
        assert!(most * STRINGS_PER_ROW - 1 < u32::MAX as usize);
        assert!(!keys_fit(most + 1));
        assert!(!keys_fit(u32::MAX as usize + 1));
        assert!(!keys_fit(usize::MAX));
    }

    /// Every variant of `E`, in declaration order, by its serde index.
    fn variants<E: serde::de::DeserializeOwned>() -> Vec<E> {
        use serde::de::{IntoDeserializer, value::Error};
        (0u32..)
            .map_while(|i| E::deserialize(IntoDeserializer::<Error>::into_deserializer(i)).ok())
            .collect()
    }

    /// `Words` ranks every variant of `E` as its serialised name orders.
    fn words_rank_as_serialised<E>(as_u8: fn(E) -> u8, as_str: fn(E) -> &'static str) -> usize
    where
        E: Copy + Serialize + serde::de::DeserializeOwned + std::fmt::Debug,
    {
        let all = variants::<E>();
        let mut words = Words::default();
        for (i, &v) in all.iter().enumerate() {
            assert_eq!(usize::from(as_u8(v)), i, "{v:?}");
            assert_eq!(json(&v), format!("\"{}\"", as_str(v)), "{v:?}");
            assert_eq!(words.id(as_u8(v), as_str(v)), as_u8(v));
        }
        let ranks = words.ranks();
        for &a in &all {
            for &b in &all {
                let by_rank = ranks[usize::from(as_u8(a))].cmp(&ranks[usize::from(as_u8(b))]);
                assert_eq!(by_rank, json(&a).cmp(&json(&b)), "{a:?} {b:?}");
            }
        }
        all.len()
    }

    #[test]
    fn words_rank_every_variant_of_the_ranked_enums_as_serialised() {
        // The control: `variants` finds every variant, not none. A new
        // variant changes a count here and is ranked above with the rest.
        assert_eq!(words_rank_as_serialised(|v: Tier| v as u8, Tier::as_str), 2);
        assert_eq!(
            words_rank_as_serialised(|v: RefKind| v as u8, RefKind::as_str),
            41
        );
        assert_eq!(
            words_rank_as_serialised(|v: ReferencedType| v as u8, ReferencedType::as_str),
            19
        );
        assert_eq!(
            words_rank_as_serialised(|v: Resolution| v as u8, Resolution::as_str),
            3
        );
        assert_eq!(
            words_rank_as_serialised(|v: Detail| v as u8, Detail::as_str),
            4
        );
    }

    #[test]
    fn declaration_order_is_node_order_for_generated_pairs() {
        let mut rng = Rng(0x2545_f491_4f6c_dd1d);
        const KINDS: &[DeclType] = &[DeclType::Class, DeclType::Enum, DeclType::EnumValue];
        const SCOPES: &[ScopeSource] = &[
            ScopeSource::PackageDirectories,
            ScopeSource::UnpackagedMetadata,
        ];
        let declaration = |rng: &mut Rng| Declaration {
            symbol: rng.string(),
            name: rng.string(),
            kind: rng.pick(KINDS),
            package: rng.string(),
            file: rng.string(),
            start: rng.number(),
            end: rng.number(),
            is_test: rng.chance(),
            annotations: Vec::new(),
            collision: rng.chance(),
            scope: rng.optional(|r| r.pick(SCOPES)),
        };
        for _ in 0..50_000 {
            let a = declaration(&mut rng);
            let mut b = a.clone();
            let fresh = declaration(&mut rng);
            match rng.below(6) {
                0 => b.symbol = fresh.symbol,
                1 => b.kind = fresh.kind,
                2 => b.package = fresh.package,
                3 => b.file = fresh.file,
                4 => b.start = fresh.start,
                _ => b.scope = fresh.scope,
            }
            let expected = Node::declaration(&a).cmp(&Node::declaration(&b));
            assert_eq!(cmp_declaration(&a, &b), expected, "{a:?}\n{b:?}");
        }
    }
}
