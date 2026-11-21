//! The report's wire types. Nullable fields are required; optional fields are
//! omitted on serialization. Presence is checked by validate::read.
use serde::{Deserialize, Serialize};
use sf_impact_common::{changed, provenance, rows};
use std::{collections::BTreeMap, num::NonZeroU64};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub provenance: VerifiedImpact,
    pub depth: u64,
    pub seeds: Vec<Seed>,
    pub queries: Vec<Query>,
    pub tests: Vec<Test>,
    pub possible: Vec<Test>,
    pub references: BTreeMap<String, rows::Reference>,
    pub untested: Vec<Node>,
    pub confidence: changed::Confidence,
    pub gaps: Vec<Gap>,
    pub diagnostics: BTreeMap<DiagnosticKind, u64>,
    pub unresolved: Vec<u64>,
    pub collisions: Vec<Collision>,
    // Any nonempty errors list is refused, regardless of its contents.
    pub errors: Vec<serde_json::Value>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub symbol: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    pub package: String,
    pub file: String,
    #[serde(default)]
    pub line: Nullable<NonZeroU64>,
    #[serde(default)]
    pub scope: Nullable<rows::ScopeSource>,
}
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum Kind {
    Declaration(rows::DeclType),
    Other(OtherKind),
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OtherKind {
    File,
    Unknown,
    Tab,
    GlobalValueSet,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Query {
    name: String,
    ambiguous: bool,
    matches: Vec<Node>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Collision {
    symbol: String,
    declarations: Vec<Node>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Seed {
    pub symbol: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    pub package: String,
    pub file: String,
    #[serde(default)]
    pub line: Nullable<NonZeroU64>,
    #[serde(default)]
    pub scope: Nullable<rows::ScopeSource>,
    pub sources: Vec<Source>,
    pub counts: Counts,
    pub referrers: Vec<Group>,
}
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum Source {
    Query(SymbolQuery),
    Changed(changed::Row),
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolQuery {
    source: Symbol,
    name: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Symbol {
    Symbol,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Counts {
    pub parsed: u64,
    pub textual: u64,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    #[serde(rename = "type")]
    kind: Kind,
    pub parsed: Vec<Hit>,
    pub textual: Vec<Hit>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hit {
    referrer: Identity,
    pub references: Vec<u64>,
    pub routes: Vec<Route>,
    ambiguous: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    symbol: String,
    file: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Route {
    target: Identity,
    via: Via,
    pub evidence: Vec<u64>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    Reference,
    Member,
    Dispatch,
    OwnerBinding,
    TriggerDml,
    PlatformEntry,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hop {
    target: Identity,
    referrer: Identity,
    pub reference: u64,
    via: Via,
    pub evidence: Vec<u64>,
    ambiguous: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Test {
    pub symbol: String,
    #[serde(rename = "type")]
    pub kind: Kind,
    pub package: String,
    pub file: String,
    #[serde(default)]
    pub line: Nullable<NonZeroU64>,
    #[serde(default)]
    pub scope: Nullable<rows::ScopeSource>,
    pub reasons: Vec<String>,
    #[serde(
        default,
        skip_serializing_if = "empty_convention",
        deserialize_with = "convention"
    )]
    pub convention: Option<Vec<Identity>>,
    pub seed: Identity,
    pub path: Vec<Hop>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gap {
    pub reason: GapReason,
    pub file: String,
    #[serde(default)]
    pub line: Nullable<NonZeroU64>,
    pub source: Input,
}
#[derive(Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Input {
    Refs,
    Changed,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum GapReason {
    UnverifiedRefs,
    UnmodelledFile,
    NoDeclaration,
    ParseError,
    UnsupportedElement,
    ScopeChanged,
    MissingDeclaration,
    Unresolved,
    Ambiguous,
    ScopeError,
    SkippedFile,
    TriggerEntry,
    DynamicBinding,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticKind {
    Unresolved,
    Ambiguous,
    ParseError,
    SkippedFile,
    UnsupportedElement,
    UnmodelledFile,
    UnreachedCollision,
}

impl Report {
    pub fn presence_lost(&self) -> bool {
        let node = |node: &Node| node.line.missing() || node.scope.missing();
        self.provenance.extra_row
            || self
                .collisions
                .iter()
                .any(|collision| collision.declarations.iter().any(node))
            || self
                .queries
                .iter()
                .any(|query| query.matches.iter().any(node))
            || self.untested.iter().any(node)
            || self
                .seeds
                .iter()
                .any(|seed| seed.line.missing() || seed.scope.missing())
            || self.tests.iter().chain(&self.possible).any(|test| {
                test.line.missing()
                    || test.scope.missing()
                    || test.convention.as_ref().is_some_and(Vec::is_empty)
            })
            || self.gaps.iter().any(|gap| gap.line.missing())
    }
}

// Retain missing versus null without a second document tree. Serialization of
// Missing mirrors the old Option wire types for compatibility diagnostics.
#[derive(Default)]
pub enum Nullable<T> {
    #[default]
    Missing,
    Present(Option<T>),
}
impl<T: Serialize> Serialize for Nullable<T> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Missing => s.serialize_none(),
            Self::Present(value) => value.serialize(s),
        }
    }
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Nullable<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Option::deserialize(d).map(Self::Present)
    }
}
impl<T> Nullable<T> {
    fn missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
    pub fn as_ref(&self) -> Option<&T> {
        match self {
            Self::Present(value) => value.as_ref(),
            Self::Missing => None,
        }
    }
}
fn convention<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Vec<Identity>>, D::Error> {
    Vec::deserialize(d).map(Some)
}
fn empty_convention(value: &Option<Vec<Identity>>) -> bool {
    value.as_ref().is_none_or(Vec::is_empty)
}

#[derive(Serialize)]
pub struct VerifiedImpact {
    #[serde(flatten)]
    pub value: provenance::Impact,
    #[serde(skip)]
    extra_row: bool,
}
impl<'de> Deserialize<'de> for VerifiedImpact {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let mut envelope = serde_json::Value::deserialize(d)?;
        let object = envelope.as_object_mut().ok_or_else(|| {
            D::Error::custom("missing provenance (older format or not impact.json)")
        })?;
        let extra_row = object
            .insert("row".into(), serde_json::Value::String("provenance".into()))
            .is_some();
        let line = envelope.to_string();
        let value = provenance::Reader {
            tool: "sf-impact-render",
            version: env!("CARGO_PKG_VERSION"),
        }
        .read(&mut line.as_bytes(), "re-run sf-impact-tests")
        .map_err(D::Error::custom)?;
        Ok(Self { value, extra_row })
    }
}

#[cfg(test)]
mod tests {
    use super::{DiagnosticKind, GapReason};
    use serde::Serialize;
    use serde_json::Value;
    use std::collections::BTreeSet;

    #[test]
    fn gap_reasons_and_diagnostic_kinds_match_schema_in_both_directions() {
        fn names<T: Serialize>(values: impl IntoIterator<Item = T>) -> BTreeSet<String> {
            values
                .into_iter()
                .map(|v| serde_json::to_string(&v).unwrap())
                .collect()
        }
        let gaps = [
            GapReason::UnverifiedRefs,
            GapReason::UnmodelledFile,
            GapReason::NoDeclaration,
            GapReason::ParseError,
            GapReason::UnsupportedElement,
            GapReason::ScopeChanged,
            GapReason::MissingDeclaration,
            GapReason::Unresolved,
            GapReason::Ambiguous,
            GapReason::ScopeError,
            GapReason::SkippedFile,
            GapReason::TriggerEntry,
            GapReason::DynamicBinding,
        ]
        .map(|reason| match reason {
            GapReason::UnverifiedRefs
            | GapReason::UnmodelledFile
            | GapReason::NoDeclaration
            | GapReason::ParseError
            | GapReason::UnsupportedElement
            | GapReason::ScopeChanged
            | GapReason::MissingDeclaration
            | GapReason::Unresolved
            | GapReason::Ambiguous
            | GapReason::ScopeError
            | GapReason::SkippedFile
            | GapReason::TriggerEntry
            | GapReason::DynamicBinding => reason,
        });
        let diagnostics = [
            DiagnosticKind::Unresolved,
            DiagnosticKind::Ambiguous,
            DiagnosticKind::ParseError,
            DiagnosticKind::SkippedFile,
            DiagnosticKind::UnsupportedElement,
            DiagnosticKind::UnmodelledFile,
            DiagnosticKind::UnreachedCollision,
        ]
        .map(|kind| match kind {
            DiagnosticKind::Unresolved
            | DiagnosticKind::Ambiguous
            | DiagnosticKind::ParseError
            | DiagnosticKind::SkippedFile
            | DiagnosticKind::UnsupportedElement
            | DiagnosticKind::UnmodelledFile
            | DiagnosticKind::UnreachedCollision => kind,
        });
        let schema: Value =
            serde_json::from_str(include_str!("../../../schema/impact.schema.json")).unwrap();
        let reasons = schema.pointer("/$defs/gap/properties/reason/enum").unwrap();
        let kinds = &schema["properties"]["diagnostics"]["properties"];
        assert_eq!(names(gaps), names(reasons.as_array().unwrap()));
        assert_eq!(names(diagnostics), names(kinds.as_object().unwrap().keys()));
    }
}
