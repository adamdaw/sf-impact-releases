//! What each element of a modelled metadata file is to this tool: read for
//! the name in it, known to hold no name, or neither.
//!
//! "Neither" is the default, and it is reported as an `unsupported_element`
//! row. An element is passed over in silence only because it is listed
//! here as holding no name of anything a repository can own. So a new
//! element, or one this tool has not got to, shows up in the output
//! instead of disappearing.
//!
//! Elements are named by their path below the root: `valueSet/valueSetName`.

use super::Kind;
use sf_impact_common::rows::{RefKind, ReferencedType};

pub enum Part {
    /// Holds no name: a flag, a number, a label, free text, a literal
    /// value. Nothing below it is looked at.
    Inert,
    /// Holds other elements, each looked at in its turn. With a name: the
    /// child that says what the element is about, which it has to have.
    Holder(Option<&'static str>),
    /// A formula: the names in it are found by the tokenizer.
    Formula,
    /// A field of a custom metadata record and the value it is set to.
    Assignment,
    /// One name.
    Name(Names),
}

/// What the one name in an element is.
#[derive(Clone, Copy)]
pub enum Names {
    /// The object a lookup points at.
    LookupTarget,
    /// A field written with its object: `Parcel__c.Weight__c`.
    Field(RefKind),
    /// A field of the object this file belongs to, written without it.
    OwnField(RefKind),
    /// A field in a lookup filter: of the object looked up, of the object
    /// the lookup is on (`$Source.`), or of a global.
    FilterField,
    GlobalValueSet,
    Granted(ReferencedType),
    /// A name that this file declares. It is read in phase one; here it
    /// only has to be there.
    Declared,
}

/// The elements of a permission set or a profile that grant access to
/// something: the element that holds one grant, the element in it that
/// names what is granted, and what kind of thing that is.
const GRANTS: [(&str, &str, ReferencedType); 9] = [
    ("classAccesses", "apexClass", ReferencedType::Class),
    ("customMetadataTypeAccesses", "name", ReferencedType::Object),
    (
        "customPermissions",
        "name",
        ReferencedType::CustomPermission,
    ),
    ("customSettingAccesses", "name", ReferencedType::Object),
    ("fieldPermissions", "field", ReferencedType::ObjectField),
    ("flowAccesses", "flow", ReferencedType::Flow),
    ("objectPermissions", "object", ReferencedType::Object),
    ("tabSettings", "tab", ReferencedType::Tab),
    ("tabVisibilities", "tab", ReferencedType::Tab),
];

/// What goes with a name in a grant: whether and how far it is granted.
const GRANT_FLAGS: [&str; 11] = [
    "allowCreate",
    "allowDelete",
    "allowEdit",
    "allowRead",
    "editable",
    "enabled",
    "modifyAllRecords",
    "readable",
    "viewAllFields",
    "viewAllRecords",
    "visibility",
];

/// What the element at `path` in a file of this kind is, or `None` if this
/// tool neither reads it nor knows it to hold no name.
pub fn part(kind: Kind, path: &str) -> Option<Part> {
    use Part::{Assignment, Formula, Holder, Inert, Name};
    Some(match kind {
        Kind::Object => match path {
            // An override names a page or a component in `content`, which
            // is not listed, and so is reported.
            "actionOverrides" => Holder(None),
            "actionOverrides/actionName"
            | "actionOverrides/comment"
            | "actionOverrides/formFactor"
            | "actionOverrides/skipRecordTypeSelect"
            | "actionOverrides/type" => Inert,
            // The object's own name field: a label and a format.
            "nameField" | "historyRetentionPolicy" => Inert,
            "allowInChatterGroups"
            | "customSettingsType"
            | "deploymentStatus"
            | "deprecated"
            | "description"
            | "enableActivities"
            | "enableBulkApi"
            | "enableChangeDataCapture"
            | "enableDataTranslation"
            | "enableEnhancedLookup"
            | "enableFeeds"
            | "enableHistory"
            | "enableLicensing"
            | "enablePublishStatusTracking"
            | "enableReports"
            | "enableSearch"
            | "enableSharing"
            | "enableStreamingApi"
            | "eventType"
            | "externalName"
            | "externalSharingModel"
            | "fullName"
            | "gender"
            | "household"
            | "label"
            | "pluralLabel"
            | "publishBehavior"
            | "recordTypeTrackFeedHistory"
            | "recordTypeTrackHistory"
            | "sharingModel"
            | "startsWith"
            | "visibility" => Inert,
            _ => return None,
        },
        Kind::Field => match path {
            "referenceTo" => Name(Names::LookupTarget),
            "formula" | "defaultValue" => Formula,
            "summarizedField" | "summaryForeignKey" | "summaryFilterItems/field" => {
                Name(Names::Field(RefKind::Summary))
            }
            "metadataRelationshipControllingField" => Name(Names::Field(RefKind::ControllingField)),
            "valueSet/controllingField" => Name(Names::OwnField(RefKind::ControllingField)),
            "valueSet/valueSetName" => Name(Names::GlobalValueSet),
            "lookupFilter/filterItems/field" | "lookupFilter/filterItems/valueField" => {
                Name(Names::FilterField)
            }
            "summaryFilterItems" | "lookupFilter/filterItems" => Holder(Some("field")),
            "valueSet" | "lookupFilter" => Holder(None),
            // The values of a picklist and which of them go with which
            // value of the controlling field: literals.
            "valueSet/restricted" | "valueSet/valueSetDefinition" | "valueSet/valueSettings" => {
                Inert
            }
            "summaryFilterItems/operation"
            | "summaryFilterItems/value"
            | "lookupFilter/active"
            | "lookupFilter/booleanFilter"
            | "lookupFilter/description"
            | "lookupFilter/errorMessage"
            | "lookupFilter/infoMessage"
            | "lookupFilter/isOptional"
            | "lookupFilter/filterItems/operation"
            | "lookupFilter/filterItems/value" => Inert,
            "businessStatus"
            | "caseSensitive"
            | "complianceGroup"
            | "deleteConstraint"
            | "deprecated"
            | "description"
            | "displayFormat"
            | "displayLocationInDecimal"
            | "encryptionScheme"
            | "escapeMarkup"
            | "externalDeveloperName"
            | "externalId"
            | "fieldManageability"
            | "formulaTreatBlanksAs"
            | "fullName"
            | "inlineHelpText"
            | "isAIPredictionField"
            | "isConvertLeadDisabled"
            | "isFilteringDisabled"
            | "isNameField"
            | "isSortingDisabled"
            | "label"
            | "length"
            | "maskChar"
            | "maskType"
            | "populateExistingRows"
            | "precision"
            | "relationshipLabel"
            | "relationshipName"
            | "relationshipOrder"
            | "reparentableMasterDetail"
            | "required"
            | "restrictedAdminField"
            | "scale"
            | "securityClassification"
            | "startingNumber"
            | "stripMarkup"
            | "summaryOperation"
            | "trackFeedHistory"
            | "trackHistory"
            | "trackTrending"
            | "translateData"
            | "type"
            | "unique"
            | "visibleLines"
            | "writeRequiresMasterRead" => Inert,
            _ => return None,
        },
        Kind::ValidationRule => match path {
            "errorConditionFormula" => Formula,
            "errorDisplayField" => Name(Names::OwnField(RefKind::DisplayField)),
            "active" | "description" | "errorMessage" | "fullName" => Inert,
            _ => return None,
        },
        Kind::PermissionSet | Kind::Profile => match path.split_once('/') {
            None => match GRANTS.iter().find(|(holder, _, _)| *holder == path) {
                Some((_, named_by, _)) => Holder(Some(named_by)),
                None => match path {
                    "custom" | "description" | "fullName" | "hasActivationRequired" | "label" => {
                        Inert
                    }
                    // Named by the platform alone: a user permission
                    // (`ViewSetup`; a repository's own are
                    // `customPermissions`, which are read) and a user
                    // licence. A permission set's `license` is not listed:
                    // it can be a permission set licence, which a
                    // repository can define, so it is reported.
                    "userPermissions" | "userLicense" => Inert,
                    _ => return None,
                },
            },
            Some((holder, within)) => {
                let (_, named_by, granted) =
                    GRANTS.iter().find(|(grant, _, _)| *grant == holder)?;
                match within {
                    _ if within == *named_by => Name(Names::Granted(*granted)),
                    _ if GRANT_FLAGS.contains(&within) => Inert,
                    _ => return None,
                }
            }
        },
        Kind::Record => match path {
            "values" => Assignment,
            "description" | "label" | "protected" => Inert,
            _ => return None,
        },
        Kind::Labels => match path {
            "labels" => Holder(Some("fullName")),
            "labels/fullName" => Name(Names::Declared),
            "labels/categories"
            | "labels/language"
            | "labels/protected"
            | "labels/shortDescription"
            | "labels/value" => Inert,
            _ => return None,
        },
        Kind::CustomPermission => match path {
            "description" | "isLicensed" | "label" => Inert,
            _ => return None,
        },
        Kind::MessageChannel => match path {
            "masterLabel" | "isExposed" | "description" => Inert,
            "lightningMessageFields" => Holder(None),
            // Payload keys are local to the message, not SObject fields.
            "lightningMessageFields/fieldName" | "lightningMessageFields/description" => Inert,
            _ => return None,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `read`, `inert`, `holder` or `unsupported`.
    fn disposition(kind: Kind, path: &str) -> &'static str {
        match part(kind, path) {
            Some(Part::Inert) => "inert",
            Some(Part::Holder(_)) => "holder",
            Some(Part::Formula | Part::Assignment | Part::Name(_)) => "read",
            None => "unsupported",
        }
    }

    #[test]
    fn an_element_nobody_has_listed_is_unsupported_not_inert() {
        for kind in [
            Kind::Object,
            Kind::Field,
            Kind::ValidationRule,
            Kind::PermissionSet,
            Kind::Profile,
            Kind::Record,
            Kind::Labels,
            Kind::CustomPermission,
        ] {
            assert_eq!(disposition(kind, "somethingNew"), "unsupported", "{kind:?}");
        }
    }

    #[test]
    fn an_element_is_what_it_is_only_at_its_own_path() {
        // `field` names a field in a grant and in a filter, and nothing
        // that this tool knows of at the top of a field file.
        assert_eq!(disposition(Kind::Field, "summaryFilterItems/field"), "read");
        assert_eq!(disposition(Kind::Field, "field"), "unsupported");
        assert_eq!(
            disposition(Kind::PermissionSet, "fieldPermissions/field"),
            "read"
        );
        assert_eq!(
            disposition(Kind::PermissionSet, "classAccesses/field"),
            "unsupported"
        );
    }

    #[test]
    fn the_flags_of_a_grant_are_inert_and_anything_else_in_it_is_not() {
        assert_eq!(
            disposition(Kind::Profile, "objectPermissions/allowRead"),
            "inert"
        );
        assert_eq!(
            disposition(Kind::Profile, "objectPermissions/somethingNew"),
            "unsupported"
        );
        // Named in the same way, but not a grant this tool reads.
        assert_eq!(
            disposition(Kind::Profile, "pageAccesses/enabled"),
            "unsupported"
        );
    }

    /// A user permission and a user licence are the platform's by
    /// definition. A permission set's licence can be one that a repository
    /// defines, so it is not passed over.
    #[test]
    fn what_only_the_platform_defines_is_inert_and_a_permission_sets_licence_is_not() {
        for kind in [Kind::PermissionSet, Kind::Profile] {
            assert_eq!(disposition(kind, "userPermissions"), "inert", "{kind:?}");
            assert_eq!(disposition(kind, "userLicense"), "inert", "{kind:?}");
            assert_eq!(disposition(kind, "license"), "unsupported", "{kind:?}");
            // A custom permission is the repository's, and is read.
            assert_eq!(
                disposition(kind, "customPermissions/name"),
                "read",
                "{kind:?}"
            );
        }
    }

    #[test]
    fn elements_that_name_other_components_are_not_listed_as_inert() {
        let reference_bearing = [
            (Kind::Object, "compactLayoutAssignment"),
            (Kind::Object, "searchLayouts"),
            (Kind::Object, "actionOverrides/content"),
            (Kind::Field, "referenceTargetField"),
            (Kind::Field, "businessOwnerGroup"),
            (Kind::Field, "picklist"),
            (Kind::PermissionSet, "applicationVisibilities"),
            (Kind::PermissionSet, "recordTypeVisibilities"),
            (Kind::PermissionSet, "license"),
            (Kind::Profile, "layoutAssignments"),
            (Kind::CustomPermission, "requiredPermission"),
        ];

        for (kind, path) in reference_bearing {
            assert_eq!(disposition(kind, path), "unsupported", "{kind:?} {path}");
        }
    }
}
