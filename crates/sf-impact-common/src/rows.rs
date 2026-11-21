//! The rows of `refs.jsonl`. `schema/refs.schema.json` is the contract; these
//! types are a convenience for the tools written in Rust and must match it.

use serde::{Deserialize, Serialize};
mod reference;
pub(crate) use reference::Presence;

/// One line of `refs.jsonl`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "row", rename_all = "snake_case")]
pub enum Row {
    ScopeError(ScopeError),
    ParseError(ParseError),
    SkippedFile(SkippedFile),
    UnmodelledFile(UnmodelledFile),
    Declaration(Declaration),
    Reference(Reference),
    UnsupportedElement(UnsupportedElement),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "row", rename_all = "snake_case")]
enum WireRow {
    ScopeError(ScopeError),
    ParseError(ParseError),
    SkippedFile(SkippedFile),
    UnmodelledFile(UnmodelledFile),
    Declaration(Declaration),
    Reference(Reference),
    UnsupportedElement(UnsupportedElement),
}

/// A directory named in `sfdx-project.json` that could not be read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeError {
    /// The path as written in `sfdx-project.json`.
    pub path: String,
    pub declared_in: ScopeSource,
    pub reason: ScopeErrorReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScopeSource {
    #[serde(rename = "packageDirectories")]
    PackageDirectories,
    #[serde(rename = "unpackagedMetadata")]
    UnpackagedMetadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeErrorReason {
    Missing,
    NotADirectory,
    OutsideRoot,
}

/// An Apex file the grammar could not parse cleanly, or a metadata file
/// that is not well-formed XML.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParseError {
    pub package: String,
    pub file: String,
    /// Line of the first error.
    pub line: u32,
}

/// A file in scope that the textual tier could not search.
/// A file is text if and only if it is valid UTF-8, or UTF-16 (LE or BE) with a BOM that decodes without error, and the decoded content contains no NUL (U+0000).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkippedFile {
    pub package: String,
    pub file: String,
    pub reason: SkipReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// A file that is not text.
    /// A file is text if and only if it is valid UTF-8, or UTF-16 (LE or BE) with a BOM that decodes without error, and the decoded content contains no NUL (U+0000).
    Binary,
}

/// A metadata file whose type the parsed tier does not model. It is still
/// searched by the textual tier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnmodelledFile {
    pub package: String,
    pub file: String,
    /// What stands between the component's name and `-meta.xml`: `layout`.
    pub suffix: String,
}

/// An element of a modelled metadata file that was not read. What it
/// refers to, if anything, is not in the output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsupportedElement {
    pub package: String,
    pub file: String,
    /// The element's path below the root: `valueSet/valueSetName`.
    pub element: String,
    pub reason: UnsupportedReason,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnsupportedReason {
    /// The tool does not interpret the element, and does not know it to
    /// hold no name.
    NotInterpreted,
    /// The element should name something and names nothing.
    NoName,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    pub symbol: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: DeclType,
    pub package: String,
    pub file: String,
    pub start: u32,
    pub end: u32,
    pub is_test: bool,
    pub annotations: Vec<String>,
    pub collision: bool,
    /// Where the scope directory was declared. Every row has it; `None`
    /// only while sf-impact-extract builds the row, before it knows.
    #[serde(
        default,
        deserialize_with = "crate::contract::scope",
        skip_serializing_if = "Option::is_none"
    )]
    pub scope: Option<ScopeSource>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeclType {
    Class,
    Interface,
    Enum,
    Trigger,
    Method,
    Constructor,
    Field,
    EnumValue,
    // Declared by metadata files.
    Object,
    ObjectField,
    ValidationRule,
    Label,
    CustomPermission,
    MessageChannel,
    PermissionSet,
    Profile,
    MetadataRecord,
    Flow,
    Lwc,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Reference {
    pub tier: Tier,
    pub kind: RefKind,
    pub referenced: String,
    /// What kind of thing `referenced` is, where the source or the
    /// declaration it resolves to says. Absent where neither does.
    #[serde(
        default,
        deserialize_with = "crate::contract::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub referenced_type: Option<ReferencedType>,
    /// Present on `parsed` rows only.
    #[serde(
        default,
        deserialize_with = "crate::contract::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub resolution: Option<Resolution>,
    #[serde(
        default,
        deserialize_with = "crate::contract::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub detail: Option<Detail>,
    #[serde(
        default,
        deserialize_with = "crate::contract::candidates_list",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub candidates: Vec<String>,
    pub referrer: Option<String>,
    /// On a `cmdt` row: the field of the record whose value this is.
    #[serde(
        default,
        deserialize_with = "crate::contract::present",
        skip_serializing_if = "Option::is_none"
    )]
    pub source_field: Option<String>,
    pub package: String,
    pub file: String,
    pub line: u32,
    pub column: u32,
}

/// What kind of thing a reference names. Apex types share one namespace,
/// so `Class` stands for an interface or an enum too. `Tab` and
/// `GlobalValueSet` are named by metadata but not declared by this tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferencedType {
    Class,
    Method,
    Constructor,
    Field,
    EnumValue,
    Trigger,
    Object,
    ObjectField,
    ValidationRule,
    Label,
    CustomPermission,
    MessageChannel,
    PermissionSet,
    Profile,
    MetadataRecord,
    Flow,
    Lwc,
    Tab,
    GlobalValueSet,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Parsed,
    Textual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefKind {
    Call,
    Construct,
    Type,
    Cast,
    Extends,
    Implements,
    StaticAccess,
    SoqlObject,
    SoqlField,
    DmlObject,
    DmlField,
    SobjectField,
    TriggerObject,
    // Read from metadata files.
    Lookup,
    LookupFilter,
    Formula,
    Summary,
    ControllingField,
    DisplayField,
    ValueSet,
    Grant,
    Cmdt,
    CmdtField,
    CmdtType,
    FlowObject,
    FlowField,
    FlowAction,
    FlowSubflow,
    FlowComponent,
    FlowLabel,
    FlowCustomPermission,
    FlowCustomMetadata,
    LwcApex,
    LwcSchema,
    LwcLabel,
    LwcCustomPermission,
    LwcMessageChannel,
    LwcComponent,
    LwcTarget,
    LwcObject,
    Text,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Resolution {
    Declaration,
    External,
    Unresolved,
}

/// Why a `parsed` reference is less exact than a single declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Detail {
    AmbiguousOverload,
    AmbiguousReceiver,
    UnknownType,
    NoMember,
}

/// Each word is the one `serde` writes for the value.
impl ScopeSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PackageDirectories => "packageDirectories",
            Self::UnpackagedMetadata => "unpackagedMetadata",
        }
    }
}
impl DeclType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Interface => "interface",
            Self::Enum => "enum",
            Self::Trigger => "trigger",
            Self::Method => "method",
            Self::Constructor => "constructor",
            Self::Field => "field",
            Self::EnumValue => "enum_value",
            Self::Object => "object",
            Self::ObjectField => "object_field",
            Self::ValidationRule => "validation_rule",
            Self::Label => "label",
            Self::CustomPermission => "custom_permission",
            Self::MessageChannel => "message_channel",
            Self::PermissionSet => "permission_set",
            Self::Profile => "profile",
            Self::MetadataRecord => "metadata_record",
            Self::Flow => "flow",
            Self::Lwc => "lwc",
        }
    }
}
impl ReferencedType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Class => "class",
            Self::Method => "method",
            Self::Constructor => "constructor",
            Self::Field => "field",
            Self::EnumValue => "enum_value",
            Self::Trigger => "trigger",
            Self::Object => "object",
            Self::ObjectField => "object_field",
            Self::ValidationRule => "validation_rule",
            Self::Label => "label",
            Self::CustomPermission => "custom_permission",
            Self::MessageChannel => "message_channel",
            Self::PermissionSet => "permission_set",
            Self::Profile => "profile",
            Self::MetadataRecord => "metadata_record",
            Self::Flow => "flow",
            Self::Lwc => "lwc",
            Self::Tab => "tab",
            Self::GlobalValueSet => "global_value_set",
        }
    }
}
impl Tier {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Parsed => "parsed",
            Self::Textual => "textual",
        }
    }
}
impl RefKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Call => "call",
            Self::Construct => "construct",
            Self::Type => "type",
            Self::Cast => "cast",
            Self::Extends => "extends",
            Self::Implements => "implements",
            Self::StaticAccess => "static_access",
            Self::SoqlObject => "soql_object",
            Self::SoqlField => "soql_field",
            Self::DmlObject => "dml_object",
            Self::DmlField => "dml_field",
            Self::SobjectField => "sobject_field",
            Self::TriggerObject => "trigger_object",
            Self::Lookup => "lookup",
            Self::LookupFilter => "lookup_filter",
            Self::Formula => "formula",
            Self::Summary => "summary",
            Self::ControllingField => "controlling_field",
            Self::DisplayField => "display_field",
            Self::ValueSet => "value_set",
            Self::Grant => "grant",
            Self::Cmdt => "cmdt",
            Self::CmdtField => "cmdt_field",
            Self::CmdtType => "cmdt_type",
            Self::FlowObject => "flow_object",
            Self::FlowField => "flow_field",
            Self::FlowAction => "flow_action",
            Self::FlowSubflow => "flow_subflow",
            Self::FlowComponent => "flow_component",
            Self::FlowLabel => "flow_label",
            Self::FlowCustomPermission => "flow_custom_permission",
            Self::FlowCustomMetadata => "flow_custom_metadata",
            Self::LwcApex => "lwc_apex",
            Self::LwcSchema => "lwc_schema",
            Self::LwcLabel => "lwc_label",
            Self::LwcCustomPermission => "lwc_custom_permission",
            Self::LwcMessageChannel => "lwc_message_channel",
            Self::LwcComponent => "lwc_component",
            Self::LwcTarget => "lwc_target",
            Self::LwcObject => "lwc_object",
            Self::Text => "text",
        }
    }
}
impl Resolution {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Declaration => "declaration",
            Self::External => "external",
            Self::Unresolved => "unresolved",
        }
    }
}
impl Detail {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AmbiguousOverload => "ambiguous_overload",
            Self::AmbiguousReceiver => "ambiguous_receiver",
            Self::UnknownType => "unknown_type",
            Self::NoMember => "no_member",
        }
    }
}

/// Names compare case-insensitively, as Apex does. Identifiers are ASCII.
pub fn fold(name: &str) -> String {
    name.to_ascii_lowercase()
}

impl<'de> Deserialize<'de> for Row {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let raw = crate::contract::raw_row(d)?;
        if reference::is_reference(raw.get()).map_err(D::Error::custom)? {
            return Reference::decode_row(raw.get())
                .map(Row::Reference)
                .map_err(D::Error::custom);
        }
        let wire: WireRow = serde_json::from_str(raw.get())
            .map_err(|e| D::Error::custom(crate::contract::row_error(raw.get(), e)))?;
        let row = match wire {
            WireRow::ScopeError(v) => Row::ScopeError(v),
            WireRow::ParseError(v) => Row::ParseError(v),
            WireRow::SkippedFile(v) => Row::SkippedFile(v),
            WireRow::UnmodelledFile(v) => Row::UnmodelledFile(v),
            WireRow::Declaration(v) => Row::Declaration(v),
            WireRow::Reference(v) => Row::Reference(v),
            WireRow::UnsupportedElement(v) => Row::UnsupportedElement(v),
        };
        row.check().map_err(D::Error::custom)?;
        Ok(row)
    }
}
impl Row {
    pub(crate) fn check(&self) -> Result<(), String> {
        use crate::contract::{nonempty, require};
        match self {
            Self::ScopeError(_) | Self::Reference(_) => Ok(()),
            Self::ParseError(v) => {
                nonempty(&[&v.file])?;
                require(v.line > 0, "parse error line must be positive")
            }
            Self::SkippedFile(v) => nonempty(&[&v.file]),
            Self::UnmodelledFile(v) => nonempty(&[&v.file, &v.suffix]),
            Self::UnsupportedElement(v) => {
                nonempty(&[&v.file, &v.element])?;
                require(v.line > 0 && v.column > 0, "position must be positive")
            }
            Self::Declaration(v) => {
                nonempty(&[&v.symbol, &v.name, &v.file])?;
                require(
                    v.start > 0 && v.end > 0,
                    "declaration position must be positive",
                )?;
                require(
                    v.annotations.iter().all(|a| !a.is_empty()),
                    "annotation names must not be empty",
                )?;
                require(v.scope.is_some(), "missing field `scope`")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn serde_word(value: impl Serialize) -> String {
        serde_json::to_value(value)
            .unwrap()
            .as_str()
            .unwrap()
            .to_owned()
    }
    #[test]
    fn as_str_is_the_serialised_word() {
        for v in [
            ScopeSource::PackageDirectories,
            ScopeSource::UnpackagedMetadata,
        ] {
            assert_eq!(v.as_str(), serde_word(v));
        }
        for v in [
            DeclType::Class,
            DeclType::Interface,
            DeclType::Enum,
            DeclType::Trigger,
            DeclType::Method,
            DeclType::Constructor,
            DeclType::Field,
            DeclType::EnumValue,
            DeclType::Object,
            DeclType::ObjectField,
            DeclType::ValidationRule,
            DeclType::Label,
            DeclType::CustomPermission,
            DeclType::MessageChannel,
            DeclType::PermissionSet,
            DeclType::Profile,
            DeclType::MetadataRecord,
            DeclType::Flow,
            DeclType::Lwc,
        ] {
            assert_eq!(v.as_str(), serde_word(v));
        }
        for v in [
            ReferencedType::Class,
            ReferencedType::Method,
            ReferencedType::Constructor,
            ReferencedType::Field,
            ReferencedType::EnumValue,
            ReferencedType::Trigger,
            ReferencedType::Object,
            ReferencedType::ObjectField,
            ReferencedType::ValidationRule,
            ReferencedType::Label,
            ReferencedType::CustomPermission,
            ReferencedType::MessageChannel,
            ReferencedType::PermissionSet,
            ReferencedType::Profile,
            ReferencedType::MetadataRecord,
            ReferencedType::Flow,
            ReferencedType::Lwc,
            ReferencedType::Tab,
            ReferencedType::GlobalValueSet,
        ] {
            assert_eq!(v.as_str(), serde_word(v));
        }
        for v in [Tier::Parsed, Tier::Textual] {
            assert_eq!(v.as_str(), serde_word(v));
        }
        for v in [
            RefKind::Call,
            RefKind::Construct,
            RefKind::Type,
            RefKind::Cast,
            RefKind::Extends,
            RefKind::Implements,
            RefKind::StaticAccess,
            RefKind::SoqlObject,
            RefKind::SoqlField,
            RefKind::DmlObject,
            RefKind::DmlField,
            RefKind::SobjectField,
            RefKind::TriggerObject,
            RefKind::Lookup,
            RefKind::LookupFilter,
            RefKind::Formula,
            RefKind::Summary,
            RefKind::ControllingField,
            RefKind::DisplayField,
            RefKind::ValueSet,
            RefKind::Grant,
            RefKind::Cmdt,
            RefKind::CmdtField,
            RefKind::CmdtType,
            RefKind::FlowObject,
            RefKind::FlowField,
            RefKind::FlowAction,
            RefKind::FlowSubflow,
            RefKind::FlowComponent,
            RefKind::FlowLabel,
            RefKind::FlowCustomPermission,
            RefKind::FlowCustomMetadata,
            RefKind::LwcApex,
            RefKind::LwcSchema,
            RefKind::LwcLabel,
            RefKind::LwcCustomPermission,
            RefKind::LwcMessageChannel,
            RefKind::LwcComponent,
            RefKind::LwcTarget,
            RefKind::LwcObject,
            RefKind::Text,
        ] {
            assert_eq!(v.as_str(), serde_word(v));
        }
        for v in [
            Resolution::Declaration,
            Resolution::External,
            Resolution::Unresolved,
        ] {
            assert_eq!(v.as_str(), serde_word(v));
        }
        for v in [
            Detail::AmbiguousOverload,
            Detail::AmbiguousReceiver,
            Detail::UnknownType,
            Detail::NoMember,
        ] {
            assert_eq!(v.as_str(), serde_word(v));
        }
    }
}
