//! Each reference shape has its own closed wire type.
use super::*;
use serde::de::Error;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Parsed {
    tier: Tier,
    kind: RefKind,
    referenced: String,
    #[serde(default, deserialize_with = "crate::contract::present_nullable")]
    referenced_type: Option<Option<ReferencedType>>,
    resolution: Resolution,
    #[serde(default, deserialize_with = "crate::contract::present_nullable")]
    detail: Option<Option<Detail>>,
    #[serde(default, deserialize_with = "crate::contract::present")]
    candidates: Option<Vec<String>>,
    #[serde(deserialize_with = "crate::contract::nullable")]
    referrer: Option<String>,
    package: String,
    file: String,
    line: u32,
    column: u32,
}
impl From<Parsed> for Reference {
    fn from(v: Parsed) -> Self {
        Self {
            tier: v.tier,
            kind: v.kind,
            referenced: v.referenced,
            referenced_type: v.referenced_type.flatten(),
            resolution: Some(v.resolution),
            detail: v.detail.flatten(),
            candidates: v.candidates.unwrap_or_default(),
            referrer: v.referrer,
            source_field: None,
            package: v.package,
            file: v.file,
            line: v.line,
            column: v.column,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Textual {
    tier: Tier,
    kind: RefKind,
    referenced: String,
    #[serde(deserialize_with = "crate::contract::nullable")]
    referrer: Option<String>,
    package: String,
    file: String,
    line: u32,
    column: u32,
}
impl From<Textual> for Reference {
    fn from(v: Textual) -> Self {
        Self {
            tier: v.tier,
            kind: v.kind,
            referenced: v.referenced,
            referenced_type: None,
            resolution: None,
            detail: None,
            candidates: Vec::new(),
            referrer: v.referrer,
            source_field: None,
            package: v.package,
            file: v.file,
            line: v.line,
            column: v.column,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    tier: Tier,
    kind: RefKind,
    referenced: String,
    #[serde(default, deserialize_with = "crate::contract::present_nullable")]
    referenced_type: Option<Option<ReferencedType>>,
    resolution: Resolution,
    #[serde(default, deserialize_with = "crate::contract::present_nullable")]
    detail: Option<Option<Detail>>,
    #[serde(default, deserialize_with = "crate::contract::present")]
    candidates: Option<Vec<String>>,
    #[serde(deserialize_with = "crate::contract::nullable")]
    referrer: Option<String>,
    #[serde(default, deserialize_with = "crate::contract::present_nullable")]
    source_field: Option<Option<String>>,
    package: String,
    file: String,
    line: u32,
    column: u32,
}
impl From<Metadata> for Reference {
    fn from(v: Metadata) -> Self {
        Self {
            tier: v.tier,
            kind: v.kind,
            referenced: v.referenced,
            referenced_type: v.referenced_type.flatten(),
            resolution: Some(v.resolution),
            detail: v.detail.flatten(),
            candidates: v.candidates.unwrap_or_default(),
            referrer: v.referrer,
            source_field: v.source_field.flatten(),
            package: v.package,
            file: v.file,
            line: v.line,
            column: v.column,
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct Presence {
    pub(crate) referenced_type: bool,
    pub(crate) detail: bool,
    pub(crate) candidates: bool,
    pub(crate) source_field: bool,
}

// A probe only chooses the closed wire shape; the selected decoder validates
// every field. Invalid kinds keep the old reader's first diagnostic.
#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    kind: Option<RefKind>,
    #[serde(default, deserialize_with = "textual")]
    tier: bool,
}
fn textual<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    struct IsTextual;
    impl<'de> serde::de::Visitor<'de> for IsTextual {
        type Value = bool;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("any tier")
        }
        fn visit_str<E: serde::de::Error>(self, s: &str) -> Result<bool, E> {
            Ok(s == "textual")
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<bool, E> {
            Ok(false)
        }
        fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<bool, E> {
            Ok(false)
        }
        fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<bool, E> {
            Ok(false)
        }
        fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<bool, E> {
            Ok(false)
        }
        fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<bool, E> {
            Ok(false)
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<bool, A::Error> {
            while a.next_element::<serde::de::IgnoredAny>()?.is_some() {}
            Ok(false)
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut a: A) -> Result<bool, A::Error> {
            while a
                .next_entry::<serde::de::IgnoredAny, serde::de::IgnoredAny>()?
                .is_some()
            {}
            Ok(false)
        }
    }
    d.deserialize_any(IsTextual)
}
pub(super) fn is_reference(line: &str) -> Result<bool, String> {
    #[derive(Deserialize)]
    #[serde(tag = "row", rename_all = "snake_case")]
    enum Tag {
        ScopeError,
        ParseError,
        SkippedFile,
        UnmodelledFile,
        Declaration,
        Reference,
        UnsupportedElement,
    }
    serde_json::from_str::<Tag>(line)
        .map(|v| matches!(v, Tag::Reference))
        .map_err(|e| crate::contract::row_error(line, e))
}
impl<'de> Deserialize<'de> for Reference {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = crate::contract::raw_row(d)?;
        Self::decode(raw.get(), false).map_err(D::Error::custom)
    }
}
impl Reference {
    pub(super) fn decode_row(line: &str) -> Result<Self, String> {
        Self::decode(line, true)
    }
    fn decode(line: &str, row: bool) -> Result<Self, String> {
        fn read<T: serde::de::DeserializeOwned>(line: &str, row: bool) -> Result<T, String> {
            #[derive(Deserialize)]
            #[serde(tag = "row", rename_all = "snake_case")]
            enum Envelope<T> {
                Reference(T),
            }
            if row {
                serde_json::from_str::<Envelope<T>>(line)
                    .map(|Envelope::Reference(v)| v)
                    .map_err(|e| crate::contract::row_error(line, e))
            } else {
                serde_json::from_str(line).map_err(|e| crate::contract::row_error(line, e))
            }
        }
        let probe: Probe = read(line, row)?;
        let kind = probe
            .kind
            .ok_or("invalid type: null, expected string or map")?;
        let (reference, presence): (Reference, Presence) = if probe.tier {
            let v: Textual = read(line, row)?;
            (v.into(), Presence::default())
        } else if kind.is_metadata() {
            let v: Metadata = read(line, row)?;
            let presence = Presence {
                referenced_type: v.referenced_type.is_some(),
                detail: v.detail.is_some(),
                candidates: v.candidates.is_some(),
                source_field: v.source_field.is_some(),
            };
            (v.into(), presence)
        } else {
            let v: Parsed = read(line, row)?;
            let presence = Presence {
                referenced_type: v.referenced_type.is_some(),
                detail: v.detail.is_some(),
                candidates: v.candidates.is_some(),
                source_field: false,
            };
            (v.into(), presence)
        };
        reference.check(presence)?;
        Ok(reference)
    }
    /// Recognize only the canonical, unescaped textual shape without allocating.
    /// Other spellings fall through to the full decoder, which validates them.
    pub fn is_textual_row(line: &str) -> bool {
        fn string(s: &str, nonempty: bool) -> Option<&str> {
            for (i, c) in s.bytes().enumerate() {
                if c == b'"' {
                    return (!nonempty || i > 0).then_some(&s[i + 1..]);
                }
                if c < 0x20 || c == b'\\' {
                    return None;
                }
            }
            None
        }
        fn position(s: &str) -> Option<&str> {
            if !matches!(s.as_bytes().first(), Some(b'1'..=b'9')) {
                return None;
            }
            let mut value = 0_u32;
            let mut end = 0;
            for c in s.bytes().take_while(u8::is_ascii_digit) {
                value = value.checked_mul(10)?.checked_add(u32::from(c - b'0'))?;
                end += 1;
            }
            Some(&s[end..])
        }
        fn tail(line: &str) -> Option<()> {
            let s = line.strip_prefix(
                r#"{"row":"reference","tier":"textual","kind":"text","referenced":""#,
            )?;
            let s = string(s, true)?.strip_prefix(r#","referrer":"#)?;
            let s = if let Some(s) = s.strip_prefix("null") {
                s
            } else {
                string(s.strip_prefix('"')?, false)?
            };
            let s = string(s.strip_prefix(r#","package":""#)?, false)?;
            let s = string(s.strip_prefix(r#","file":""#)?, true)?;
            let s = position(s.strip_prefix(r#","line":"#)?)?;
            let s = position(s.strip_prefix(r#","column":"#)?)?;
            (s == "}").then_some(())
        }
        tail(line).is_some()
    }

    pub(crate) fn check(&self, presence: Presence) -> Result<(), String> {
        use crate::contract::{nonempty, require};
        nonempty(&[&self.referenced, &self.file])?;
        require(
            self.line > 0 && self.column > 0,
            "reference position must be positive",
        )?;
        let textual = self.tier == Tier::Textual;
        require(
            textual == (self.kind == RefKind::Text),
            "reference tier disagrees with kind",
        )?;
        if textual {
            return Ok(());
        }
        if presence.referenced_type {
            require(
                self.referenced_type.is_some(),
                "referenced_type cannot be null",
            )?;
        }
        if presence.detail {
            require(self.detail.is_some(), "detail cannot be null")?;
        }
        if self.kind.is_metadata() {
            self.check_metadata(presence)?;
        } else {
            require(
                self.resolution != Some(Resolution::Declaration)
                    || presence.candidates
                    || self.referenced_type.is_some(),
                "a declared referenced_type is required",
            )?;
        }
        self.check_candidates(presence)
    }

    fn check_metadata(&self, presence: Presence) -> Result<(), String> {
        use crate::contract::{nonempty, require};
        require(
            self.detail.is_none() || self.detail == Some(Detail::AmbiguousReceiver),
            "invalid metadata detail",
        )?;
        require(
            self.referrer.as_ref().is_some_and(|s| !s.is_empty()),
            "metadata referrer is required",
        )?;
        require(
            self.kind == RefKind::Formula || self.referenced_type.is_some(),
            "metadata referenced_type is required",
        )?;
        require(
            (self.kind == RefKind::Cmdt) == presence.source_field,
            "source_field belongs only to cmdt",
        )?;
        if self.kind == RefKind::Cmdt {
            nonempty(&[self.source_field.as_deref().unwrap_or_default()])?;
        }
        Ok(())
    }

    fn check_candidates(&self, presence: Presence) -> Result<(), String> {
        use crate::contract::{candidates, require};
        let ambiguous = matches!(
            self.detail,
            Some(Detail::AmbiguousOverload | Detail::AmbiguousReceiver)
        );
        require(
            ambiguous == presence.candidates,
            "candidates require an ambiguous detail",
        )?;
        if ambiguous {
            candidates(&self.candidates)?;
        }
        Ok(())
    }
}

impl RefKind {
    pub(crate) fn is_metadata(self) -> bool {
        matches!(
            self,
            RefKind::Lookup
                | RefKind::LookupFilter
                | RefKind::Formula
                | RefKind::Summary
                | RefKind::ControllingField
                | RefKind::DisplayField
                | RefKind::ValueSet
                | RefKind::Grant
                | RefKind::Cmdt
                | RefKind::CmdtField
                | RefKind::CmdtType
        )
    }
}
