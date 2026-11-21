use crate::document::{self, Report};
use serde_json::Value;
use sf_impact_common::{
    changed::Confidence,
    provenance::{Reader, Tree},
    rows::Tier,
};
use std::collections::BTreeSet;

pub fn read(input: &[u8]) -> Result<Value, String> {
    match serde_json::from_slice::<Report>(input) {
        // Release the typed tree before building the renderer's JSON view.
        Ok(report) if !report.presence_lost() => {
            check(&report)?;
            drop(report);
            serde_json::from_slice(input).map_err(|e| e.to_string())
        }
        // Decoder and presence failures replay the old diagnostics, including
        // the last-key-wins policy for duplicate keys.
        _ => compatible(input),
    }
}

fn check(report: &Report) -> Result<(), String> {
    if !report.errors.is_empty() {
        return Err("errors are present; resolve them before rendering".into());
    }
    check_fields(report)?;
    if (report.confidence == Confidence::Full) != report.gaps.is_empty() {
        return Err("confidence disagrees with gaps".into());
    }
    let provenance = &report.provenance.value;
    if let Some(changed) = &provenance.inputs.changed {
        for (source, needed) in [
            (
                document::Input::Refs,
                matches!(provenance.inputs.refs.tree, Tree::Dirty | Tree::Unknown),
            ),
            (document::Input::Changed, !changed.gaps.is_empty()),
        ] {
            let name = if source == document::Input::Refs {
                "refs"
            } else {
                "changed"
            };
            let found = report.gaps.iter().any(|gap| {
                gap.reason == document::GapReason::UnverifiedRefs
                    && gap.source == source
                    && gap.file == format!("{name}.jsonl")
                    && gap.line.as_ref().is_some_and(|line| line.get() == 1)
            });
            if needed != found {
                return Err(format!("gaps disagree with inputs.{name} provenance"));
            }
        }
    } else if report
        .gaps
        .iter()
        .any(|gap| gap.reason == document::GapReason::UnverifiedRefs)
    {
        return Err("unverified_refs requires a changed input".into());
    }
    for id in report.references.keys() {
        if id.parse::<u64>().ok().map(|n| n.to_string()).as_ref() != Some(id) {
            return Err(format!("references key {id:?} is not an integer ID"));
        }
    }
    // Match the old JSON map traversal order, including evidence before the
    // reference of a hop. Multiple invalid IDs keep the same first diagnostic.
    let ids = |key: &str, ids: &[u64]| -> Result<(), String> {
        for id in ids {
            if !report.references.contains_key(&id.to_string()) {
                return Err(format!("{key}: missing reference {id}"));
            }
        }
        Ok(())
    };
    let paths = |tests: &[document::Test]| -> Result<(), String> {
        for test in tests {
            for hop in &test.path {
                ids("evidence", &hop.evidence)?;
                ids("reference", std::slice::from_ref(&hop.reference))?;
            }
        }
        Ok(())
    };
    paths(&report.possible)?;
    for seed in &report.seeds {
        for group in &seed.referrers {
            for hit in group.parsed.iter().chain(&group.textual) {
                ids("references", &hit.references)?;
                for route in &hit.routes {
                    ids("evidence", &route.evidence)?;
                }
            }
        }
    }
    paths(&report.tests)?;
    ids("unresolved", &report.unresolved)?;
    for seed in &report.seeds {
        for (name, tier, count) in [
            ("parsed", Tier::Parsed, seed.counts.parsed),
            ("textual", Tier::Textual, seed.counts.textual),
        ] {
            let mut distinct = BTreeSet::new();
            for group in &seed.referrers {
                let hits = if tier == Tier::Parsed {
                    &group.parsed
                } else {
                    &group.textual
                };
                for hit in hits {
                    for id in &hit.references {
                        if report.references[&id.to_string()].tier != tier {
                            return Err(format!(
                                "referrer tier {name} disagrees with reference {id}"
                            ));
                        }
                        distinct.insert(id);
                    }
                }
            }
            if count != distinct.len() as u64 {
                return Err(format!(
                    "seed counts.{name} disagrees with direct references"
                ));
            }
        }
    }
    let selected: BTreeSet<_> = report
        .tests
        .iter()
        .map(|test| (&test.symbol, &test.file))
        .collect();
    if report
        .possible
        .iter()
        .any(|test| selected.contains(&(&test.symbol, &test.file)))
    {
        return Err("selected and possible tests overlap".into());
    }
    Ok(())
}

// Direct Serde errors include input locations and follow wire key order. On a
// refusal, replay the old diagnostic order (and last-key-wins duplicate policy).
// Only this fallback retains both JSON and typed trees for the wire diff.
fn compatible(input: &[u8]) -> Result<Value, String> {
    let doc: Value = serde_json::from_slice(input).map_err(|e| e.to_string())?;
    let mut envelope = doc["provenance"].clone();
    let Some(object) = envelope.as_object_mut() else {
        return Err("missing provenance (older format or not impact.json)".into());
    };
    object.insert("row".into(), Value::String("provenance".into()));
    let line = envelope.to_string();
    let _: sf_impact_common::provenance::Impact = Reader {
        tool: "sf-impact-render",
        version: env!("CARGO_PKG_VERSION"),
    }
    .read(&mut line.as_bytes(), "re-run sf-impact-tests")?;
    if doc["errors"]
        .as_array()
        .is_some_and(|errors| !errors.is_empty())
    {
        return Err("errors are present; resolve them before rendering".into());
    }
    let report: Report = Reader {
        tool: "sf-impact-render",
        version: env!("CARGO_PKG_VERSION"),
    }
    .document(&doc)?;
    check(&report)?;
    Ok(doc)
}

fn check_fields(report: &Report) -> Result<(), String> {
    use sf_impact_common::{changed, rows::Tier};
    for (key, tests, allowed) in [
        (
            "tests",
            &report.tests,
            &[
                "changed",
                "convention",
                "direct",
                "caller",
                "trigger_dml",
                "platform_entry",
            ][..],
        ),
        (
            "possible",
            &report.possible,
            &["textual_path", "lwc_textual", "member_name_only"][..],
        ),
    ] {
        for test in tests {
            let unique: BTreeSet<_> = test.reasons.iter().map(String::as_str).collect();
            if test.reasons.is_empty()
                || unique.len() != test.reasons.len()
                || unique.iter().any(|r| !allowed.contains(r))
                || (key == "possible" && test.path.is_empty())
                || (test.convention.is_some() && !unique.contains("convention"))
            {
                return Err(format!("{key}: invalid reasons or witness path"));
            }
        }
    }
    let candidates_valid = |c: &[String]| {
        c.is_empty()
            || (c.len() >= 2
                && c.iter().all(|s| !s.is_empty())
                && c.iter().collect::<BTreeSet<_>>().len() == c.len())
    };
    for reference in report.references.values() {
        if reference.file.is_empty()
            || reference.line == 0
            || reference.column == 0
            || (reference.tier == Tier::Parsed) != reference.resolution.is_some()
            || reference
                .source_field
                .as_ref()
                .is_some_and(|s| s.is_empty())
            || !candidates_valid(&reference.candidates)
        {
            return Err("invalid reference fields".into());
        }
    }
    for seed in &report.seeds {
        for source in &seed.sources {
            use crate::document::Source;
            let invalid = match source {
                Source::Query(_) => false,
                Source::Changed(changed::Row::File(_)) => true,
                Source::Changed(changed::Row::Touched(t)) => {
                    t.referrer.is_empty()
                        || t.referenced.is_empty()
                        || t.file.is_empty()
                        || t.line == 0
                        || !matches!(
                            t.kind,
                            sf_impact_common::rows::RefKind::Grant
                                | sf_impact_common::rows::RefKind::Cmdt
                                | sf_impact_common::rows::RefKind::CmdtField
                                | sf_impact_common::rows::RefKind::CmdtType
                        )
                        || !candidates_valid(&t.candidates)
                }
                Source::Changed(changed::Row::Symbol(s)) => {
                    s.symbol.is_empty()
                        || s.file.is_empty()
                        || s.previous_file.as_ref().is_some_and(|s| s.is_empty())
                        || s.start == Some(0)
                        || s.end == Some(0)
                        || s.start.is_some() != s.end.is_some()
                        || (s.reason == changed::Reason::Hunk
                            && (s.hunks.is_empty() || s.start.is_none()))
                        || (s.reason == changed::Reason::Removed
                            && (s.change != changed::Change::Removed
                                || s.start.is_none()
                                || !s.hunks.is_empty()))
                }
            };
            if invalid {
                return Err("invalid changed source fields".into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod differential {
    use serde_json::{Value, json};
    use std::collections::BTreeSet;

    // Frozen pre-T5 reader and wire types are the compatibility oracle.
    mod document {
        use serde::{Deserialize, Serialize};
        use sf_impact_common::{changed, provenance, rows};
        use std::{collections::BTreeMap, num::NonZeroU64};

        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct Report {
            pub provenance: provenance::Impact,
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
            // A report containing errors is refused before decoding this field.
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
            pub line: Option<NonZeroU64>,
            pub scope: Option<rows::ScopeSource>,
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
            pub line: Option<NonZeroU64>,
            pub scope: Option<rows::ScopeSource>,
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
            parsed: u64,
            textual: u64,
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct Group {
            #[serde(rename = "type")]
            kind: Kind,
            parsed: Vec<Hit>,
            textual: Vec<Hit>,
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct Hit {
            referrer: Identity,
            references: Vec<u64>,
            routes: Vec<Route>,
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
            evidence: Vec<u64>,
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
            reference: u64,
            via: Via,
            evidence: Vec<u64>,
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
            pub line: Option<NonZeroU64>,
            pub scope: Option<rows::ScopeSource>,
            pub reasons: Vec<String>,
            #[serde(default, skip_serializing_if = "Vec::is_empty")]
            pub convention: Vec<Identity>,
            pub seed: Identity,
            pub path: Vec<Hop>,
        }
        #[derive(Serialize, Deserialize)]
        #[serde(deny_unknown_fields)]
        pub struct Gap {
            reason: GapReason,
            file: String,
            line: Option<NonZeroU64>,
            source: Input,
        }
        #[derive(Serialize, Deserialize)]
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
            // Schema 2 (T8): the impact contract's seventh key.
            UnreachedCollision,
        }
    }

    mod old {
        use crate::{rows, text};
        use serde_json::Value;
        use sf_impact_common::provenance::{self, Reader, Tree};
        use std::collections::BTreeSet;

        pub fn read(input: &[u8]) -> Result<Value, String> {
            let doc: Value = serde_json::from_slice(input).map_err(|e| e.to_string())?;
            let mut envelope = doc["provenance"].clone();
            let Some(object) = envelope.as_object_mut() else {
                return Err("missing provenance (older format or not impact.json)".into());
            };
            object.insert("row".into(), Value::String("provenance".into()));
            let line = envelope.to_string();
            let provenance: provenance::Impact = Reader {
                tool: "sf-impact-render",
                version: env!("CARGO_PKG_VERSION"),
            }
            .read(&mut line.as_bytes(), "re-run sf-impact-tests")?;
            if doc["errors"]
                .as_array()
                .is_some_and(|errors| !errors.is_empty())
            {
                return Err("errors are present; resolve them before rendering".into());
            }
            let report: super::document::Report = Reader {
                tool: "sf-impact-render",
                version: env!("CARGO_PKG_VERSION"),
            }
            .document(&doc)?;
            check_fields(&report, &doc)?;
            if (doc["confidence"] == "full") != rows(&doc, "gaps").is_empty() {
                return Err("confidence disagrees with gaps".into());
            }
            if let Some(changed) = &provenance.inputs.changed {
                for (source, needed) in [
                    (
                        "refs",
                        matches!(provenance.inputs.refs.tree, Tree::Dirty | Tree::Unknown),
                    ),
                    ("changed", !changed.gaps.is_empty()),
                ] {
                    let found = rows(&doc, "gaps").iter().any(|g| {
                        g["reason"] == "unverified_refs"
                            && g["source"] == source
                            && g["file"] == format!("{source}.jsonl")
                            && g["line"] == 1
                    });
                    if needed != found {
                        return Err(format!("gaps disagree with inputs.{source} provenance"));
                    }
                }
            } else if rows(&doc, "gaps")
                .iter()
                .any(|gap| gap["reason"] == "unverified_refs")
            {
                return Err("unverified_refs requires a changed input".into());
            }
            let references = doc["references"].as_object().expect("validated map");
            for id in references.keys() {
                if id.parse::<u64>().ok().map(|n| n.to_string()).as_ref() != Some(id) {
                    return Err(format!("references key {id:?} is not an integer ID"));
                }
            }
            check_ids(&doc, references)?;
            for seed in rows(&doc, "seeds") {
                for tier in ["parsed", "textual"] {
                    let mut ids = BTreeSet::new();
                    for group in rows(seed, "referrers") {
                        for hit in rows(group, tier) {
                            for id in rows(hit, "references") {
                                if references[&id.to_string()]["tier"] != tier {
                                    return Err(format!(
                                        "referrer tier {tier} disagrees with reference {id}"
                                    ));
                                }
                                ids.insert(id.as_u64().expect("validated ID"));
                            }
                        }
                    }
                    if seed["counts"][tier].as_u64() != Some(ids.len() as u64) {
                        return Err(format!(
                            "seed counts.{tier} disagrees with direct references"
                        ));
                    }
                }
            }
            let selected: BTreeSet<_> = rows(&doc, "tests")
                .iter()
                .map(|t| (text(&t["symbol"]), text(&t["file"])))
                .collect();
            if rows(&doc, "possible")
                .iter()
                .any(|t| selected.contains(&(text(&t["symbol"]), text(&t["file"]))))
            {
                return Err("selected and possible tests overlap".into());
            }
            Ok(doc)
        }
        fn check_ids(
            value: &Value,
            references: &serde_json::Map<String, Value>,
        ) -> Result<(), String> {
            match value {
                Value::Object(object) => {
                    for (key, value) in object {
                        if key == "reference"
                            || (["references", "evidence", "unresolved"].contains(&key.as_str())
                                && value.is_array())
                        {
                            let ids = if key == "reference" {
                                std::slice::from_ref(value)
                            } else {
                                value.as_array().unwrap()
                            };
                            for id in ids {
                                if !references.contains_key(&id.to_string()) {
                                    return Err(format!("{key}: missing reference {id}"));
                                }
                            }
                        } else {
                            check_ids(value, references)?;
                        }
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        check_ids(value, references)?;
                    }
                }
                _ => {}
            }
            Ok(())
        }

        fn check_fields(report: &super::document::Report, doc: &Value) -> Result<(), String> {
            use sf_impact_common::{changed, rows::Tier};
            for (key, allowed) in [
                (
                    "tests",
                    &[
                        "changed",
                        "convention",
                        "direct",
                        "caller",
                        "trigger_dml",
                        "platform_entry",
                    ][..],
                ),
                (
                    "possible",
                    &["textual_path", "lwc_textual", "member_name_only"][..],
                ),
            ] {
                for test in rows(doc, key) {
                    let reasons = rows(test, "reasons");
                    let unique: BTreeSet<_> = reasons.iter().map(text).collect();
                    if reasons.is_empty()
                        || unique.len() != reasons.len()
                        || unique.iter().any(|r| !allowed.contains(r))
                        || (key == "possible" && rows(test, "path").is_empty())
                        || (test.get("convention").is_some() && !unique.contains("convention"))
                    {
                        return Err(format!("{key}: invalid reasons or witness path"));
                    }
                }
            }
            let candidates_valid = |c: &[String]| {
                c.is_empty()
                    || (c.len() >= 2
                        && c.iter().all(|s| !s.is_empty())
                        && c.iter().collect::<BTreeSet<_>>().len() == c.len())
            };
            for reference in report.references.values() {
                if reference.file.is_empty()
                    || reference.line == 0
                    || reference.column == 0
                    || (reference.tier == Tier::Parsed) != reference.resolution.is_some()
                    || reference
                        .source_field
                        .as_ref()
                        .is_some_and(|s| s.is_empty())
                    || !candidates_valid(&reference.candidates)
                {
                    return Err("invalid reference fields".into());
                }
            }
            for seed in &report.seeds {
                for source in &seed.sources {
                    use super::document::Source;
                    let invalid = match source {
                        Source::Query(_) => false,
                        Source::Changed(changed::Row::File(_)) => true,
                        Source::Changed(changed::Row::Touched(t)) => {
                            t.referrer.is_empty()
                                || t.referenced.is_empty()
                                || t.file.is_empty()
                                || t.line == 0
                                || !matches!(
                                    t.kind,
                                    sf_impact_common::rows::RefKind::Grant
                                        | sf_impact_common::rows::RefKind::Cmdt
                                        | sf_impact_common::rows::RefKind::CmdtField
                                        | sf_impact_common::rows::RefKind::CmdtType
                                )
                                || !candidates_valid(&t.candidates)
                        }
                        Source::Changed(changed::Row::Symbol(s)) => {
                            s.symbol.is_empty()
                                || s.file.is_empty()
                                || s.previous_file.as_ref().is_some_and(|s| s.is_empty())
                                || s.start == Some(0)
                                || s.end == Some(0)
                                || s.start.is_some() != s.end.is_some()
                                || (s.reason == changed::Reason::Hunk
                                    && (s.hunks.is_empty() || s.start.is_none()))
                                || (s.reason == changed::Reason::Removed
                                    && (s.change != changed::Change::Removed
                                        || s.start.is_none()
                                        || !s.hunks.is_empty()))
                        }
                    };
                    if invalid {
                        return Err("invalid changed source fields".into());
                    }
                }
            }
            Ok(())
        }
    }

    fn paths(v: &Value, at: String, out: &mut Vec<String>) {
        out.push(at.clone());
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    paths(
                        v,
                        format!("{at}/{}", k.replace('~', "~0").replace('/', "~1")),
                        out,
                    );
                }
            }
            Value::Array(a) => {
                for (i, v) in a.iter().enumerate() {
                    paths(v, format!("{at}/{i}"), out);
                }
            }
            _ => {}
        }
    }

    fn outcome(result: Result<Value, String>) -> (u8, String) {
        match result {
            Ok(_) => (0, String::new()),
            Err(e) => (
                1,
                format!(
                    "sf-impact-render: impact.json: {}; re-run sf-impact-tests\n",
                    e.replace(['\r', '\n'], " ")
                ),
            ),
        }
    }

    // Keep null at every path, plus a shape-specific replacement. ID positions
    // also exercise an existing ID (0), which can name the wrong reference tier,
    // as well as a missing ID. Deletions and unknown fields visit every object.
    fn replacements(at: &str, value: &Value) -> Vec<Value> {
        let field = at.rsplit('/').next().unwrap();
        let parent = at.rsplit('/').nth(1).unwrap_or("");
        if value.is_number()
            && (field == "reference" || ["references", "evidence", "unresolved"].contains(&parent))
        {
            return vec![Value::Null, json!(0), json!(u64::MAX)];
        }
        let other = match value {
            Value::Null => json!(0),
            Value::Bool(value) => json!(!value),
            Value::Number(_) if ["line", "column"].contains(&field) => json!(0),
            Value::Number(_) => json!(1.5),
            Value::String(_)
                if [
                    "kind",
                    "type",
                    "tier",
                    "reason",
                    "source",
                    "confidence",
                    "row",
                    "resolution",
                    "detail",
                    "scope",
                ]
                .contains(&field) =>
            {
                json!("bogus")
            }
            Value::String(_) => json!(""),
            Value::Array(_) if ["references", "evidence", "unresolved"].contains(&field) => {
                json!([u64::MAX])
            }
            Value::Array(values) if !values.is_empty() => json!([]),
            Value::Array(_) => json!({}),
            Value::Object(_) => json!([]),
        };
        vec![Value::Null, other]
    }

    #[test]
    fn mutated_documents_preserve_exit_code_and_stderr() {
        let mut checked = 0;
        let mut visited = 0;
        let mut values = BTreeSet::new();
        for input in [
            include_str!("../../../fixtures/impact/method.json"),
            include_str!("../../../fixtures/impact/binding.json"),
            include_str!("../../../fixtures/impact/unpackaged.json"),
            include_str!("../../../fixtures/impact/unpackaged-excluded.json"),
        ] {
            let doc: Value = serde_json::from_str(input).unwrap();
            let mut locations = Vec::new();
            paths(&doc, String::new(), &mut locations);
            let mut check = |candidate: &Value, label: &str| {
                let bytes = serde_json::to_vec(candidate).unwrap();
                assert_eq!(
                    outcome(old::read(&bytes)),
                    outcome(super::read(&bytes)),
                    "{label}"
                );
                checked += 1;
            };
            check(&doc, "control");
            // Keep the remaining integer boundaries and boolean replacement
            // kinds once per document instead of at every unrelated path.
            for wrong in [json!(0), json!(1), json!(-1), json!(u64::MAX), json!(false)] {
                values.insert(wrong.to_string());
                let mut candidate = doc.clone();
                candidate["depth"] = wrong;
                check(&candidate, "depth boundary");
            }
            visited += locations.len();
            for at in locations {
                for wrong in replacements(&at, doc.pointer(&at).unwrap()) {
                    values.insert(wrong.to_string());
                    let mut candidate = doc.clone();
                    *candidate.pointer_mut(&at).unwrap() = wrong;
                    check(&candidate, &at);
                }
                if let Some(map) = doc.pointer(&at).unwrap().as_object() {
                    for key in map.keys().chain(std::iter::once(&"unknown".to_owned())) {
                        let mut candidate = doc.clone();
                        let object = candidate.pointer_mut(&at).unwrap().as_object_mut().unwrap();
                        if key == "unknown" {
                            object.insert(key.clone(), json!(true));
                        } else {
                            object.remove(key);
                        }
                        check(&candidate, &format!("{at}/{key}"));
                    }
                }
            }
        }
        for retained in [
            Value::Null,
            json!(0),
            json!(1),
            json!(-1),
            json!(u64::MAX),
            json!([u64::MAX]),
            json!(1.5),
            json!(false),
            json!({}),
            json!([]),
            json!(""),
            json!("bogus"),
        ] {
            assert!(
                values.contains(&retained.to_string()),
                "replacement value {retained} was not used in any generated case"
            );
        }
        eprintln!("differential: {checked} mutated documents and controls across {visited} paths");
    }

    #[test]
    fn selected_and_possible_overlap_preserves_exit_code_and_stderr() {
        let mut doc: Value =
            serde_json::from_str(include_str!("../../../fixtures/impact/unpackaged.json")).unwrap();
        let mut possible = doc["tests"][0].clone();
        // A valid possible witness must pass the earlier reasons/path check
        // before the same identity can exercise the overlap check.
        possible["reasons"] = json!(["textual_path"]);
        doc["possible"] = json!([possible]);
        let bytes = serde_json::to_vec(&doc).unwrap();
        let expected = old::read(&bytes);
        assert_eq!(
            expected.as_ref().unwrap_err(),
            "selected and possible tests overlap"
        );
        assert_eq!(outcome(expected), outcome(super::read(&bytes)));
    }

    #[test]
    fn dirty_refs_without_matching_gap_preserves_exit_code_and_stderr() {
        let mut doc: Value =
            serde_json::from_str(include_str!("../../../fixtures/impact/method.json")).unwrap();
        doc["provenance"]["inputs"]["refs"]["tree"] = json!({"state": "dirty"});
        let bytes = serde_json::to_vec(&doc).unwrap();
        let expected = old::read(&bytes);
        assert_eq!(
            expected.as_ref().unwrap_err(),
            "gaps disagree with inputs.refs provenance"
        );
        assert_eq!(outcome(expected), outcome(super::read(&bytes)));
    }

    #[test]
    fn absent_optional_fields_preserve_exit_code_and_stderr() {
        let mut checked = 0;
        let mut exercised = BTreeSet::new();
        for input in [
            include_str!("../../../fixtures/impact/method.json"),
            include_str!("../../../fixtures/impact/binding.json"),
            include_str!("../../../fixtures/impact/unpackaged.json"),
            include_str!("../../../fixtures/impact/unpackaged-excluded.json"),
        ] {
            let doc: Value = serde_json::from_str(input).unwrap();
            let mut locations = Vec::new();
            paths(&doc, String::new(), &mut locations);
            let mut check = |candidate: &Value, label: &str| {
                let bytes = serde_json::to_vec(candidate).unwrap();
                assert_eq!(
                    outcome(old::read(&bytes)),
                    outcome(super::read(&bytes)),
                    "{label}"
                );
                checked += 1;
            };
            // This must reach the wire diff, without another provenance error
            // masking an extra row key as the multiple-error control does.
            let mut candidate = doc.clone();
            candidate["provenance"]["row"] = json!("provenance");
            check(&candidate, "added provenance.row");
            assert!(old::read(&serde_json::to_vec(&candidate).unwrap()).is_err());
            for at in locations {
                let Some(object) = doc.pointer(&at).unwrap().as_object() else {
                    continue;
                };
                let keys: &[&str] = if object.contains_key("reasons") && object.contains_key("path")
                {
                    &["convention"]
                } else if object.contains_key("tier") {
                    &["candidates", "detail"]
                } else if object.get("row") == Some(&json!("symbol")) {
                    &["previous_file"]
                } else if object.get("row") == Some(&json!("touched")) {
                    &["candidates"]
                } else {
                    &[]
                };
                for key in keys {
                    if object.contains_key(*key) {
                        continue;
                    }
                    exercised.insert(*key);
                    for value in [
                        Value::Null,
                        json!([]),
                        json!(false),
                        json!(0),
                        json!(""),
                        json!("bogus"),
                        json!(["A", "B"]),
                        json!([{ "symbol": "A", "file": "app/A.cls" }]),
                    ] {
                        let mut candidate = doc.clone();
                        candidate
                            .pointer_mut(&at)
                            .unwrap()
                            .as_object_mut()
                            .unwrap()
                            .insert((*key).into(), value);
                        check(&candidate, &format!("added {at}/{key}"));
                        if *key == "convention"
                            && candidate.pointer(&format!("{at}/{key}")) == Some(&json!([]))
                        {
                            assert!(
                                old::read(&serde_json::to_vec(&candidate).unwrap()).is_err(),
                                "explicit empty convention"
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(
            exercised,
            BTreeSet::from(["convention", "candidates", "detail", "previous_file"])
        );
        eprintln!("differential: {checked} absent optional field and provenance.row cases");
    }

    #[test]
    fn valid_fixtures_use_the_typed_path() {
        for input in [
            include_str!("../../../fixtures/impact/method.json"),
            include_str!("../../../fixtures/impact/binding.json"),
            include_str!("../../../fixtures/impact/unpackaged.json"),
            include_str!("../../../fixtures/impact/unpackaged-excluded.json"),
        ] {
            let report: crate::document::Report = serde_json::from_slice(input.as_bytes()).unwrap();
            assert!(!report.presence_lost());
            super::check(&report).unwrap();
        }
    }

    #[test]
    fn duplicates_keep_last_value_and_error_precedence() {
        let doc: Value =
            serde_json::from_str(include_str!("../../../fixtures/impact/unpackaged.json")).unwrap();
        let encoded = doc.to_string();
        let mut locations = Vec::new();
        paths(&doc, String::new(), &mut locations);
        let mut checked = 0;
        for at in locations {
            let original = doc.pointer(&at).unwrap();
            if let Some(object) = original.as_object() {
                for (key, value) in object {
                    for first in [value.clone(), json!("wrong first value")] {
                        let duplicated = format!(
                            "{{{}:{first},{}",
                            serde_json::to_string(key).unwrap(),
                            &original.to_string()[1..]
                        );
                        let candidate = encoded.replacen(&original.to_string(), &duplicated, 1);
                        assert_eq!(
                            outcome(old::read(candidate.as_bytes())),
                            outcome(super::read(candidate.as_bytes())),
                            "duplicate {at}/{key}"
                        );
                        checked += 1;
                    }
                }
            }
        }
        for (pointer, value) in [
            ("/provenance/row", json!("ignored by provenance reader")),
            ("/depth", json!("bad depth")),
            ("/tests/0/path/0/evidence", json!([u64::MAX])),
        ] {
            let mut candidate = doc.clone();
            if pointer == "/provenance/row" {
                candidate["provenance"]["row"] = value;
            } else {
                *candidate.pointer_mut(pointer).unwrap() = value;
            }
            // The provenance diagnostic must still precede these body errors.
            candidate["provenance"]["schema"] = json!(99);
            let bytes = serde_json::to_vec(&candidate).unwrap();
            assert_eq!(
                outcome(old::read(&bytes)),
                outcome(super::read(&bytes)),
                "multiple errors {pointer}"
            );
        }
        eprintln!("differential: {checked} duplicate-key documents");
    }
}
