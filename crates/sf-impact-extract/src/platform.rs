//! Names that belong to the Apex platform: its namespaces and the classes of
//! the `System` and `Schema` namespaces, which code uses without a qualifier.
//!
//! A reference that is wholly the platform's can never be a reference into
//! the repository (a repository type of the same name is found first and
//! wins, as in Apex), so it is not reported. The list is curated, not
//! complete. A platform name missing from it shows up as an `unresolved`
//! reference, which errs on the side of saying too much.
//!
//! A platform name at the head of a dotted name does not make the whole name
//! the platform's: `Schema.Invoice__c` is an object in the repository, and
//! `Label.Greeting` is a custom label. `Index::foreign` decides that; this
//! module only knows the names. `Label` is left out of the list because
//! every member of it is a custom label.

/// Lower-case and sorted, for binary search.
const NAMES: &[&str] = &[
    "access",
    "accesslevel",
    "accesstype",
    "address",
    "aggregateresult",
    "apexpages",
    "applauncher",
    "approval",
    "assert",
    "assertexception",
    "asyncexception",
    "asyncinfo",
    "asyncoptions",
    "aurahandledexception",
    "auth",
    "blob",
    "boolean",
    "businesshours",
    "cache",
    "callable",
    "calloutexception",
    "canvas",
    "cases",
    "chatteranswers",
    "childrelationship",
    "collator",
    "commercebuygrp",
    "commerceextension",
    "commerceorders",
    "commercepayments",
    "commercetax",
    "comparable",
    "comparator",
    "compression",
    "connectapi",
    "context",
    "continuation",
    "cookie",
    "crypto",
    "database",
    "datacloud",
    "dataretrieval",
    "datasource",
    "dataweave",
    "dataweavescriptexception",
    "date",
    "datetime",
    "decimal",
    "describefieldresult",
    "describesobjectresult",
    "displaytype",
    "dmlexception",
    "dom",
    "domain",
    "domaincreator",
    "domainparser",
    "domaintype",
    "double",
    "emailexception",
    "emailmessages",
    "encodingutil",
    "eventbus",
    "exception",
    "externalobjectexception",
    "externalservice",
    "featuremanagement",
    "fieldset",
    "fieldsetmember",
    "finalexception",
    "finalizer",
    "finalizercontext",
    "flow",
    "flowexception",
    "formula",
    "formulaeval",
    "functions",
    "handledexception",
    "http",
    "httpcalloutmock",
    "httprequest",
    "httpresponse",
    "id",
    "ideas",
    "illegalargumentexception",
    "installhandler",
    "integer",
    "invalidparametervalueexception",
    "invocable",
    "isvpartners",
    "iterable",
    "iterator",
    "json",
    "jsonexception",
    "jsongenerator",
    "jsonparser",
    "jsontoken",
    "kbmanagement",
    "limitexception",
    "limits",
    "list",
    "listexception",
    "location",
    "logginglevel",
    "long",
    "lxscheduler",
    "map",
    "matcher",
    "math",
    "mathexception",
    "messaging",
    "metadata",
    "multistaticresourcecalloutmock",
    "network",
    "noaccessexception",
    "nodatafoundexception",
    "nosuchelementexception",
    "nullpointerexception",
    "object",
    "orglimit",
    "orglimits",
    "packaging",
    "pagereference",
    "pattern",
    "picklistentry",
    "placequote",
    "pref_center",
    "process",
    "queryexception",
    "queueable",
    "queueablecontext",
    "quickaction",
    "quiddity",
    "recordtypeinfo",
    "reports",
    "request",
    "requiredfeaturemissingexception",
    "resetpasswordresult",
    "restcontext",
    "restrequest",
    "restresponse",
    "revsalestrxn",
    "revsignaling",
    "richmessaging",
    "roundingmode",
    "sandboxcontext",
    "sandboxpostcopy",
    "savepoint",
    "schedulable",
    "schedulablecontext",
    "schema",
    "search",
    "searchexception",
    "security",
    "securityexception",
    "selectoption",
    "serializationexception",
    "set",
    "sfc",
    "sfdc_checkout",
    "sfdc_surveys",
    "site",
    "slack",
    "soaptype",
    "sobject",
    "sobjectaccessdecision",
    "sobjectexception",
    "sobjectfield",
    "sobjecttype",
    "staticresourcecalloutmock",
    "string",
    "stringexception",
    "stubprovider",
    "support",
    "system",
    "territorymgmt",
    "test",
    "time",
    "timezone",
    "trigger",
    "triggeroperation",
    "txnsecurity",
    "type",
    "typeexception",
    "unexpectedexception",
    "uninstallhandler",
    "unsupportedoperationexception",
    "url",
    "userinfo",
    "usermanagement",
    "userprovisioning",
    "uuid",
    "version",
    "visualeditor",
    "visualforceexception",
    "wave",
    "webservicecallout",
    "webservicemock",
    "xmlexception",
    "xmlstreamreader",
    "xmlstreamwriter",
];

/// `name` is a single lower-case segment: a type name or a leading qualifier.
pub fn is_platform(name: &str) -> bool {
    NAMES.binary_search(&name).is_ok()
}

/// The classes of the `Schema` namespace. Any other name written after
/// `Schema.` is an SObject type: `Schema.Account`, `Schema.Invoice__c`.
pub fn is_schema_class(name: &str) -> bool {
    matches!(
        name,
        "childrelationship"
            | "datacategory"
            | "datacategorygroupsobjecttypepair"
            | "describecolorresult"
            | "describedatacategorygroupresult"
            | "describedatacategorygroupstructureresult"
            | "describefieldresult"
            | "describeiconresult"
            | "describesobjectresult"
            | "describetabresult"
            | "describetabsetresult"
            | "displaytype"
            | "fielddescribeoptions"
            | "fieldset"
            | "fieldsetmember"
            | "picklistentry"
            | "recordtypeinfo"
            | "sobjectdescribeoptions"
            | "soaptype"
            | "sobjectfield"
            | "sobjecttype"
    )
}

/// Methods every Apex object or enum has without declaring them.
pub fn is_universal_method(name: &str) -> bool {
    matches!(
        name,
        "equals" | "hashcode" | "tostring" | "values" | "name" | "ordinal" | "valueof"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_sorted_and_lower_case_so_binary_search_finds_every_name() {
        assert!(NAMES.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(
            NAMES
                .iter()
                .all(|name| !name.chars().any(|c| c.is_ascii_uppercase()))
        );
    }

    #[test]
    fn system_classes_and_namespaces_are_platform_but_sobjects_are_not() {
        assert!(is_platform("string"));
        assert!(is_platform("schema"));
        assert!(!is_platform("account"));
    }
}
