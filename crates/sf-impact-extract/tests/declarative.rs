//! Declarative metadata in the fixture project: objects, fields, validation
//! rules, permission sets, profiles, custom metadata, labels and custom
//! permissions, and the Apex that names them. The fixture files are under
//! `declarative/` in two package directories.
//!
//! These tests read the output as JSON, as any consumer of `refs.jsonl`
//! would, and spell out the rows they expect.

#[path = "../../sf-impact-common/tests/support/schema.rs"]
mod schema;

use schema::Schema;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

const APP: &str = "force-app/declarative";
const PLUGIN: &str = "plugins/Billing/declarative";

static ROWS: LazyLock<Vec<Value>> = LazyLock::new(|| {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/project");
    let output = Command::new(env!("CARGO_BIN_EXE_sf-impact-extract"))
        .arg("--root")
        .arg(root)
        .args(["--format", "jsonl"])
        .output()
        .expect("sf-impact-extract runs");
    assert!(
        output.status.success(),
        "sf-impact-extract failed: {output:?}"
    );
    let text = String::from_utf8(output.stdout).expect("output is UTF-8");
    let rows = text.lines().map(serde_json::from_str);
    rows.collect::<Result<_, _>>().expect("every line is JSON")
});

fn text<'a>(row: &'a Value, key: &str) -> &'a str {
    row[key].as_str().unwrap_or_default()
}

/// The rows of one kind (`declaration`, `reference`, ..) for one file.
fn rows_of(file: &str, kind: &str) -> std::vec::IntoIter<&'static Value> {
    let rows = ROWS.iter();
    let found: Vec<_> = rows
        .filter(|row| text(row, "file") == file && text(row, "row") == kind)
        .collect();
    found.into_iter()
}

/// The declarations in `file`, each as `type symbol lines`.
fn declared(file: &str) -> Vec<String> {
    let line = |row: &Value| {
        format!(
            "{} {} {}-{}",
            text(row, "type"),
            text(row, "symbol"),
            row["start"],
            row["end"]
        )
    };
    rows_of(file, "declaration").map(line).collect()
}

/// The parsed references in `file`, in output order, each as
/// `kind referenced [type] resolution`, then the detail and candidates if
/// it has them, then `<- source field` if it has one.
fn parsed(file: &str) -> Vec<String> {
    let line = |row: &Value| {
        let mut line = format!("{} {}", text(row, "kind"), text(row, "referenced"));
        for key in ["referenced_type", "resolution", "detail"] {
            if let Some(value) = row[key].as_str() {
                line = format!("{line} {value}");
            }
        }
        if let Some(candidates) = row["candidates"].as_array() {
            let candidates: Vec<&str> = candidates.iter().filter_map(Value::as_str).collect();
            line = format!("{line} [{}]", candidates.join(", "));
        }
        if let Some(field) = row["source_field"].as_str() {
            line = format!("{line} <- {field}");
        }
        line
    };
    rows_of(file, "reference")
        .filter(|row| text(row, "tier") == "parsed")
        .map(line)
        .collect()
}

/// The elements of `file` that were not read, in output order, each as
/// `element reason line`.
fn not_read(file: &str) -> Vec<String> {
    let line = |row: &Value| {
        format!(
            "{} {} {}",
            text(row, "element"),
            text(row, "reason"),
            row["line"]
        )
    };
    rows_of(file, "unsupported_element").map(line).collect()
}

fn in_app(path: &str) -> String {
    format!("{APP}/{path}")
}

fn in_plugin(path: &str) -> String {
    format!("{PLUGIN}/{path}")
}

fn field(object: &str, name: &str) -> String {
    in_app(&format!("objects/{object}/fields/{name}.field-meta.xml"))
}

// Declarations.

#[test]
fn an_object_and_its_fields_are_declared_under_their_api_names() {
    let object = declared(&in_app("objects/Shipment__c/Shipment__c.object-meta.xml"));
    let weight = declared(&field("Shipment__c", "Weight__c"));

    assert_eq!(object, ["object Shipment__c 1-6"]);
    assert_eq!(weight, ["object_field Shipment__c.Weight__c 1-8"]);
}

#[test]
fn a_field_of_a_standard_object_is_declared_without_declaring_the_object() {
    let field = declared(&field("Account", "Region__c"));
    let account_is_declared = ROWS
        .iter()
        .any(|row| text(row, "row") == "declaration" && text(row, "symbol") == "Account");

    assert_eq!(field, ["object_field Account.Region__c 1-7"]);
    assert!(!account_is_declared);
}

#[test]
fn a_validation_rule_is_declared_under_its_object() {
    let rule = declared(&in_app(
        "objects/Shipment__c/validationRules/Delivered_Needs_Weight.validationRule-meta.xml",
    ));

    assert_eq!(
        rule,
        ["validation_rule Shipment__c.Delivered_Needs_Weight 1-15"]
    );
}

#[test]
fn each_label_in_the_labels_file_is_declared_as_apex_names_it() {
    let labels = declared(&in_app("labels/CustomLabels.labels-meta.xml"));

    assert_eq!(
        labels,
        [
            "label Label.Shipment_Late 3-9",
            "label Label.Shipment_Title 10-16"
        ]
    );
}

#[test]
fn custom_permissions_permission_sets_and_metadata_records_are_declared() {
    let permission = declared(&in_app(
        "customPermissions/Waive_Shipping_Fee.customPermission-meta.xml",
    ));
    let permission_set = declared(&in_app("permissionsets/Shipping.permissionset-meta.xml"));
    let record = declared(&in_app(
        "customMetadata/Shipping_Plugin.Tracking.md-meta.xml",
    ));

    assert_eq!(permission, ["custom_permission Waive_Shipping_Fee 1-5"]);
    assert_eq!(permission_set, ["permission_set Shipping 1-74"]);
    assert_eq!(record, ["metadata_record Shipping_Plugin.Tracking 1-21"]);
}

#[test]
fn a_permission_set_saved_as_utf16_is_declared_and_its_grants_are_read() {
    let file = "force-app/main/default/permissionsets/Billing.permissionset-meta.xml";

    assert_eq!(declared(file), ["permission_set Billing 1-8"]);
    assert_eq!(parsed(file), ["grant BillingPlugin class declaration"]);
}

// Names declared more than once.

#[test]
fn a_profile_split_across_two_package_directories_is_a_collision_that_keeps_both() {
    let files = [
        in_app("profiles/Admin.profile-meta.xml"),
        in_plugin("profiles/Admin.profile-meta.xml"),
    ];

    for file in files {
        let profile = rows_of(&file, "declaration").next().expect("declared");
        assert_eq!(text(profile, "symbol"), "Admin");
        assert_eq!(profile["collision"], true, "{file}");
    }
}

#[test]
fn a_field_copied_into_a_second_package_directory_resolves_as_one() {
    let copy = rows_of(
        &in_plugin("objects/Shipment__c/fields/Weight__c.field-meta.xml"),
        "declaration",
    )
    .next()
    .expect("declared");
    let permission_set = parsed(&in_app("permissionsets/Shipping.permissionset-meta.xml"));

    assert_eq!(copy["collision"], true);
    assert!(
        permission_set
            .contains(&"grant Shipment__c.Weight__c object_field declaration".to_string()),
        "{permission_set:?}"
    );
}

#[test]
fn a_traversal_through_a_lookup_declared_twice_to_different_objects_names_both_ends() {
    let formula = parsed(&field("Shipment__c", "Payer_Region__c"));

    assert_eq!(
        formula,
        [
            // Both rows are at the start of the path, where rows sort by
            // their text.
            "formula Region__c object_field declaration ambiguous_receiver \
             [Account.Region__c, Contact.Region__c]",
            "formula Shipment__c.Payer__c object_field declaration",
        ]
    );
}

#[test]
fn a_field_that_only_one_end_of_such_a_lookup_declares_leaves_the_other_end_unresolved() {
    let formula = parsed(&field("Shipment__c", "Payer_No__c"));

    // `Payer__r.No__c`: `Account` has the field. Whether `Contact` has it
    // cannot be told from the repository, and it is not passed over.
    assert_eq!(
        formula,
        [
            "formula Account.No__c object_field declaration",
            "formula Contact.No__c object_field unresolved",
            "formula Shipment__c.Payer__c object_field declaration",
        ]
    );
}

#[test]
fn a_field_that_neither_end_of_such_a_lookup_declares_is_unresolved_at_both() {
    let formula = parsed(&field("Shipment__c", "Payer_Name__c"));

    assert_eq!(
        formula,
        [
            "formula Account.Name object_field unresolved",
            "formula Contact.Name object_field unresolved",
            "formula Shipment__c.Payer__c object_field declaration",
        ]
    );
}

// Fields.

#[test]
fn a_lookup_references_the_object_it_points_at() {
    let custom = parsed(&field("Shipment__c", "Carrier__c"));
    let master_detail = parsed(&field("Parcel__c", "Shipment__c"));
    let standard = parsed(&field("Shipment__c", "Payer__c"));

    assert_eq!(custom, ["lookup Carrier__c object declaration"]);
    assert_eq!(master_detail, ["lookup Shipment__c object declaration"]);
    // No metadata for `Account` is in the repository.
    assert_eq!(standard, ["lookup Account object unresolved"]);
}

#[test]
fn a_formula_references_each_field_on_a_relationship_path_and_its_globals() {
    let formula = parsed(&field("Shipment__c", "Carrier_Region__c"));

    assert_eq!(
        formula,
        [
            "formula Carrier__c.Region__c object_field declaration",
            "formula Shipment__c.Carrier__c object_field declaration",
            "formula Shipping_Settings__c.Default_Region__c object_field declaration",
            "formula Carrier__c.Region__c object_field declaration",
            "formula Shipment__c.Carrier__c object_field declaration",
            "formula Label.Shipment_Title label declaration",
            "formula Shipment__c.Owner.Alias object_field unresolved",
        ]
    );
}

#[test]
fn a_name_in_a_formula_comment_or_string_is_left_to_the_textual_tier() {
    let file = field("Shipment__c", "Carrier_Region__c");
    let textual: Vec<&str> = rows_of(&file, "reference")
        .filter(|row| text(row, "tier") == "textual")
        .map(|row| text(row, "referenced"))
        .collect();

    // In the comment and in the string, and so not among the parsed rows above.
    assert!(textual.contains(&"Delivered__c"), "{textual:?}");
    assert!(textual.contains(&"Weight__c"), "{textual:?}");
    assert!(!parsed(&file).iter().any(|row| row.contains("Delivered__c")));
}

#[test]
fn a_formula_reference_is_located_where_the_name_is_written() {
    let label = rows_of(&field("Shipment__c", "Carrier_Region__c"), "reference")
        .find(|row| {
            text(row, "referenced") == "Label.Shipment_Title" && text(row, "tier") == "parsed"
        })
        .expect("the label is referenced");

    // Line 6 of the file; the `&amp;` before it is five bytes in the source.
    assert_eq!(
        (label["line"].as_u64(), label["column"].as_u64()),
        (Some(6), Some(32))
    );
    assert_eq!(text(label, "referrer"), "Shipment__c.Carrier_Region__c");
}

#[test]
fn a_default_value_is_a_formula() {
    let with_setting = parsed(&field("Shipment__c", "Region__c"));
    let literal = parsed(&field("Shipment__c", "Delivered__c"));

    assert_eq!(
        with_setting,
        ["formula Shipping_Settings__c.Default_Region__c object_field declaration"]
    );
    assert_eq!(literal, [] as [&str; 0]);
}

#[test]
fn a_roll_up_summary_references_the_fields_it_sums_filters_and_joins_on() {
    let summary = parsed(&field("Shipment__c", "Parcel_Weight__c"));

    assert_eq!(
        summary,
        [
            "summary Parcel__c.Weight__c object_field declaration",
            "summary Parcel__c.Fragile__c object_field declaration",
            "summary Parcel__c.Shipment__c object_field declaration",
        ]
    );
}

#[test]
fn a_validation_rule_references_what_its_condition_names() {
    let rule = parsed(&in_app(
        "objects/Shipment__c/validationRules/Delivered_Needs_Weight.validationRule-meta.xml",
    ));

    assert_eq!(
        rule,
        [
            "formula Shipment__c.Delivered__c object_field declaration",
            "formula Shipment__c.Weight__c object_field declaration",
            "formula Shipment__c.Weight__c object_field declaration",
            "formula Waive_Shipping_Fee custom_permission declaration",
            "formula Shipping_Plugin.Tracking metadata_record declaration",
            "formula Shipping_Plugin__mdt.Enabled__c object_field declaration",
            "formula Profile.Name object_field unresolved",
            // Not a global this tool knows: reported as written, not dropped.
            "formula $Mystery.Thing unresolved",
            "display_field Shipment__c.Weight__c object_field declaration",
        ]
    );
}

#[test]
fn a_picklist_references_its_controlling_field_and_its_global_value_set() {
    let picklist = parsed(&field("Shipment__c", "Mode__c"));

    assert_eq!(
        picklist,
        [
            "controlling_field Shipment__c.Go__c object_field declaration",
            // The value set's file is in scope, but value sets are not declared.
            "value_set Modes global_value_set unresolved",
        ]
    );
}

#[test]
fn a_lookup_filter_references_the_fields_it_compares() {
    let lookup = parsed(&field("Shipment__c", "Depot__c"));

    assert_eq!(
        lookup,
        [
            "lookup_filter Carrier__c.Region__c object_field declaration",
            // `$Source.Kg__c`: a field of the object the lookup is on.
            "lookup_filter Shipment__c.Kg__c object_field declaration",
            "lookup_filter Carrier__c.Name object_field unresolved",
            "lookup Carrier__c object declaration",
        ]
    );
}

#[test]
fn a_validation_rule_references_the_field_its_error_is_shown_on() {
    let rule = parsed(&in_app(
        "objects/Shipment__c/validationRules/Heavy_Needs_Go.validationRule-meta.xml",
    ));

    assert_eq!(
        rule,
        [
            "formula Shipment__c.Kg__c object_field declaration",
            "formula Shipment__c.Go__c object_field declaration",
            "display_field Shipment__c.Go__c object_field declaration",
        ]
    );
}

#[test]
fn a_metadata_relationship_field_references_the_field_that_controls_it() {
    let relationship = parsed(&field("Shipping_Plugin__mdt", "By__c"));

    assert_eq!(
        relationship,
        [
            "controlling_field Shipping_Plugin__mdt.On__c object_field declaration",
            "lookup FieldDefinition object unresolved",
        ]
    );
}

#[test]
fn a_part_of_an_object_file_that_is_not_interpreted_is_a_row() {
    let object = not_read(&in_app("objects/Carrier__c/Carrier__c.object-meta.xml"));

    // An override that names a Lightning page, a compact layout and search
    // layouts. The override that names nothing is not among them.
    assert_eq!(
        object,
        [
            "actionOverrides/content not_interpreted 9",
            "compactLayoutAssignment not_interpreted 13",
            "searchLayouts not_interpreted 16",
        ]
    );
}

#[test]
fn only_what_can_name_something_and_is_not_interpreted_is_reported_as_not_read() {
    let reported: Vec<String> = ROWS
        .iter()
        .filter(|row| text(row, "row") == "unsupported_element")
        .map(|row| format!("{} in {}", text(row, "element"), text(row, "file")))
        .collect();

    // Every modelled file in the fixture is full of labels, types, flags
    // and lengths. These are all that is reported: the parts of objects,
    // permission sets and profiles that are not read, and one structure in
    // a Flow that the Flow reader does not know.
    assert_eq!(
        reported,
        [
            "actionOverrides/content in force-app/declarative/objects/Carrier__c/Carrier__c.object-meta.xml",
            "compactLayoutAssignment in force-app/declarative/objects/Carrier__c/Carrier__c.object-meta.xml",
            "searchLayouts in force-app/declarative/objects/Carrier__c/Carrier__c.object-meta.xml",
            "license in force-app/declarative/permissionsets/Shipping.permissionset-meta.xml",
            "futureInvoiceSettings in force-app/flow-lwc/flows/InvoiceGlobals.flow-meta.xml",
            "layoutAssignments in plugins/Billing/declarative/profiles/Admin.profile-meta.xml",
            "pageAccesses in plugins/Billing/declarative/profiles/Admin.profile-meta.xml",
        ]
    );
}

/// `Go__c`, `Kg__c`, `No__c`, `On__c`, `By__c`, `Do__c`, `Modes` and `Pack`
/// are shorter than the textual tier's minimum, so nothing but structure
/// reports them.
#[test]
fn the_short_names_these_tests_rely_on_are_not_found_by_the_textual_tier() {
    let short = [
        "Go__c", "Kg__c", "No__c", "On__c", "By__c", "Do__c", "Modes", "Pack",
    ];
    let textual: Vec<&str> = ROWS
        .iter()
        .filter(|row| text(row, "tier") == "textual")
        .map(|row| text(row, "referenced"))
        .filter(|word| short.iter().any(|name| name.eq_ignore_ascii_case(word)))
        .collect();
    let parsed_names: Vec<&str> = ROWS
        .iter()
        .filter(|row| text(row, "tier") == "parsed")
        .map(|row| text(row, "referenced"))
        .collect();

    assert_eq!(textual, [] as [&str; 0]);
    for name in [
        "Shipment__c.Go__c",
        "Shipment__c.Kg__c",
        "Account.No__c",
        "Modes",
        "Pack",
    ] {
        assert!(parsed_names.contains(&name), "{name}");
    }
}

// Permission sets and profiles.

#[test]
fn a_permission_set_grants_classes_objects_fields_flows_tabs_and_custom_permissions() {
    let grants = parsed(&in_app("permissionsets/Shipping.permissionset-meta.xml"));

    assert_eq!(
        grants,
        [
            "grant ShipmentTracker class declaration",
            "grant Retired_Service class unresolved",
            "grant npsp__Pricing class external",
            "grant Shipping_Plugin__mdt object declaration",
            "grant Waive_Shipping_Fee custom_permission declaration",
            "grant Shipping_Settings__c object declaration",
            "grant Shipment__c.Weight__c object_field declaration",
            "grant Account.Industry object_field unresolved",
            "grant Shipment_Reminder flow declaration",
            "grant Shipment__c object declaration",
            "grant Account object unresolved",
            "grant Shipment__c tab unresolved",
            // A tab of the platform's is granted like any other.
            "grant standard-Account tab unresolved",
        ]
    );
}

#[test]
fn a_part_of_a_permission_set_or_profile_that_is_not_interpreted_is_a_row() {
    let permission_set = not_read(&in_app("permissionsets/Shipping.permissionset-meta.xml"));
    let profile = not_read(&in_plugin("profiles/Admin.profile-meta.xml"));

    // A licence, a layout and a Visualforce page are named in them. A
    // permission-set licence can be one that a repository defines.
    assert_eq!(permission_set, ["license not_interpreted 43"]);
    assert_eq!(
        profile,
        [
            "layoutAssignments not_interpreted 8",
            "pageAccesses not_interpreted 11"
        ]
    );
}

/// `ViewSetup` is a permission of the platform's, and `Salesforce` a user
/// licence: no repository declares either, and neither is a gap.
#[test]
fn a_platform_permission_and_a_user_licence_name_nothing_a_repository_can_hold() {
    let permission_set = in_app("permissionsets/Shipping.permissionset-meta.xml");
    let profile = in_plugin("profiles/Admin.profile-meta.xml");
    let said_about = |file: &str, word: &str| -> Vec<String> {
        let rows = ROWS.iter().filter(|row| text(row, "file") == file);
        rows.filter(|row| text(row, "tier") != "textual")
            .filter(|row| row.to_string().contains(word))
            .map(|row| text(row, "row").to_string())
            .collect()
    };

    // The control: the same question finds the grant of a class.
    assert_eq!(
        said_about(&permission_set, "ShipmentTracker"),
        ["reference"]
    );
    assert_eq!(
        said_about(&permission_set, "userPermissions"),
        [] as [&str; 0]
    );
    assert_eq!(said_about(&permission_set, "ViewSetup"), [] as [&str; 0]);
    assert_eq!(said_about(&profile, "userLicense"), [] as [&str; 0]);
}

#[test]
fn a_partial_profile_grants_what_it_holds() {
    let grants = parsed(&in_app("profiles/Admin.profile-meta.xml"));

    assert_eq!(
        grants,
        [
            "grant ShipmentAudit class declaration",
            "grant Shipment__c.Tracking_Id__c object_field declaration",
            "grant Shipment_Reminder flow declaration",
            "grant Shipment__c tab unresolved",
        ]
    );
}

#[test]
fn a_grant_names_its_permission_set_and_the_line_of_the_name() {
    let flow = rows_of(
        &in_app("permissionsets/Shipping.permissionset-meta.xml"),
        "reference",
    )
    .find(|row| text(row, "referenced_type") == "flow")
    .expect("the flow is granted");

    assert_eq!(text(flow, "referrer"), "Shipping");
    assert_eq!(
        (flow["line"].as_u64(), flow["column"].as_u64()),
        (Some(39), Some(15))
    );
}

// Custom metadata records.

#[test]
fn a_record_references_its_type_and_what_its_string_values_name() {
    let record = parsed(&in_app(
        "customMetadata/Shipping_Plugin.Tracking.md-meta.xml",
    ));

    assert_eq!(
        record,
        [
            "cmdt_type Shipping_Plugin__mdt object declaration",
            "cmdt_field Shipping_Plugin__mdt.Cache_Partition__c object_field declaration",
            // A cache partition that happens to be called like a class: the
            // row carries the field so that a reviewer can judge.
            "cmdt Circle class declaration <- Cache_Partition__c",
            "cmdt_field Shipping_Plugin__mdt.Enabled__c object_field declaration",
            "cmdt_field Shipping_Plugin__mdt.Flow_Name__c object_field declaration",
            "cmdt Shipment_Reminder flow declaration <- Flow_Name__c",
            "cmdt_field Shipping_Plugin__mdt.Handler__c object_field declaration",
            // Written `shipmenttracker` in the record.
            "cmdt ShipmentTracker class declaration <- Handler__c",
        ]
    );
}

#[test]
fn a_record_named_like_a_class_references_it_by_its_developer_name() {
    let record = parsed(&in_app(
        "customMetadata/Shipping_Plugin.ShipmentAudit.md-meta.xml",
    ));

    assert_eq!(
        record,
        [
            "cmdt ShipmentAudit class declaration <- DeveloperName",
            "cmdt_type Shipping_Plugin__mdt object declaration",
            // It sets these fields, whatever it sets them to.
            "cmdt_field Shipping_Plugin__mdt.Cache_Partition__c object_field declaration",
            "cmdt_field Shipping_Plugin__mdt.Enabled__c object_field declaration",
            "cmdt_field Shipping_Plugin__mdt.Flow_Name__c object_field declaration",
            "cmdt_field Shipping_Plugin__mdt.Handler__c object_field declaration",
        ]
    );
}

#[test]
fn a_value_of_a_metadata_relationship_field_names_what_the_fields_type_says() {
    let record = parsed(&in_app(
        "customMetadata/Shipping_Plugin.Routing.md-meta.xml",
    ));

    assert_eq!(
        record,
        [
            "cmdt_type Shipping_Plugin__mdt object declaration",
            "cmdt_field Shipping_Plugin__mdt.By__c object_field declaration",
            // `Kg__c`: a field of the object that `On__c` names.
            "cmdt Shipment__c.Kg__c object_field declaration <- By__c",
            "cmdt_field Shipping_Plugin__mdt.Enabled__c object_field declaration",
            "cmdt_field Shipping_Plugin__mdt.Next__c object_field declaration",
            // `Tracking`: a record of the type the field points at.
            "cmdt Shipping_Plugin.Tracking metadata_record declaration <- Next__c",
            "cmdt_field Shipping_Plugin__mdt.On__c object_field declaration",
            "cmdt Shipment__c object declaration <- On__c",
        ]
    );
}

/// `Do__c` is a text field in one package directory and a relationship to
/// another record in the other. Which it is in the org cannot be told, so
/// its value is read both ways: as the class `Pack`, and as a record.
#[test]
fn a_value_of_a_field_declared_as_text_and_as_a_relationship_is_read_both_ways() {
    let record = parsed(&in_app("customMetadata/Shipping_Plugin.Mixed.md-meta.xml"));

    assert_eq!(
        record,
        [
            "cmdt_type Shipping_Plugin__mdt object declaration",
            "cmdt_field Shipping_Plugin__mdt.Do__c object_field declaration",
            "cmdt Pack class declaration <- Do__c",
            "cmdt Shipping_Plugin.Pack metadata_record unresolved <- Do__c",
        ]
    );
}

#[test]
fn a_relationship_value_that_cannot_be_placed_and_a_field_nothing_declares_stay_as_rows() {
    let record = parsed(&in_app(
        "customMetadata/Shipping_Plugin.Unrouted.md-meta.xml",
    ));

    assert_eq!(
        record,
        [
            "cmdt_type Shipping_Plugin__mdt object declaration",
            "cmdt_field Shipping_Plugin__mdt.By__c object_field declaration",
            // The record does not say which object `Kg__c` is a field of.
            "cmdt Kg__c object_field unresolved <- By__c",
            "cmdt_field Shipping_Plugin__mdt.Gone__c object_field unresolved",
            "cmdt_field Shipping_Plugin__mdt.On__c object_field declaration",
        ]
    );
}

#[test]
fn a_relationship_value_that_names_a_standard_object_or_field_is_unresolved_not_dropped() {
    let record = parsed(&in_app(
        "customMetadata/Shipping_Plugin.Standard.md-meta.xml",
    ));

    assert_eq!(
        record,
        [
            "cmdt_type Shipping_Plugin__mdt object declaration",
            "cmdt_field Shipping_Plugin__mdt.By__c object_field declaration",
            "cmdt Account.Industry object_field unresolved <- By__c",
            "cmdt_field Shipping_Plugin__mdt.On__c object_field declaration",
            "cmdt Account object unresolved <- On__c",
        ]
    );
}

// Files that are not modelled, and files that do not parse.

#[test]
fn a_metadata_file_of_a_type_that_is_not_modelled_is_a_row_not_a_silence() {
    let suffix_of = |path: &str| -> Vec<&'static str> {
        rows_of(&in_app(path), "unmodelled_file")
            .map(|row| text(row, "suffix"))
            .collect()
    };

    assert_eq!(
        suffix_of("layouts/Shipment__c-Shipment Layout.layout-meta.xml"),
        ["layout"]
    );
    assert_eq!(suffix_of("tabs/Shipment__c.tab-meta.xml"), ["tab"]);
    // A Flow is modelled: it is a declaration, which grants and records
    // resolve to.
    assert_eq!(
        suffix_of("flows/Shipment_Reminder.flow-meta.xml"),
        [] as [&str; 0]
    );
    assert_eq!(
        declared(&in_app("flows/Shipment_Reminder.flow-meta.xml")),
        ["flow Shipment_Reminder 1-7"]
    );
}

#[test]
fn modelled_metadata_and_the_companion_file_of_an_apex_class_are_not_reported_as_unmodelled() {
    let unmodelled: Vec<&str> = ROWS
        .iter()
        .filter(|row| text(row, "row") == "unmodelled_file")
        .map(|row| text(row, "file"))
        .collect();

    // `classes/ShipmentTracker.cls-meta.xml` and every object, field,
    // permission set and record are in the fixture; only these are reported.
    assert_eq!(
        unmodelled,
        [
            in_app("globalValueSets/Modes.globalValueSet-meta.xml"),
            in_app("layouts/Shipment__c-Shipment Layout.layout-meta.xml"),
            in_app("tabs/Shipment__c.tab-meta.xml"),
        ]
    );
}

#[test]
fn a_metadata_file_that_is_not_well_formed_is_reported_and_still_declared() {
    let file = field("Shipment__c", "Broken__c");
    let error = rows_of(&file, "parse_error").next().expect("reported");

    assert_eq!(
        (error.get("errors"), error["line"].as_u64()),
        (None, Some(4))
    );
    assert_eq!(declared(&file), ["object_field Shipment__c.Broken__c 1-5"]);
}

// Apex that names metadata.

fn apex(referrer: &str) -> Vec<String> {
    let file = in_app("classes/ShipmentTracker.cls");
    let rows = rows_of(&file, "reference")
        .filter(|row| text(row, "tier") == "parsed" && text(row, "referrer") == referrer);
    let line = |row: &Value| {
        let detail = row["detail"].as_str().map(|detail| format!(" {detail}"));
        format!(
            "{} {} {}{}",
            text(row, "kind"),
            text(row, "referenced"),
            text(row, "resolution"),
            detail.unwrap_or_default()
        )
    };
    rows.map(line).collect()
}

#[test]
fn a_query_resolves_to_the_object_and_fields_that_metadata_declares() {
    let query = apex("ShipmentTracker.late(id)");

    assert_eq!(
        query,
        [
            "type Shipment__c declaration",
            // A standard field: nothing in the repository declares it.
            "soql_field Shipment__c.Id unresolved",
            "soql_field Shipment__c.Weight__c declaration",
            // `Carrier__r.Region__c`: the lookup, then the field it reaches.
            "soql_field Carrier__c.Region__c declaration",
            "soql_field Shipment__c.Carrier__c declaration",
            // `Payer__r.Region__c`: the lookup is declared twice, to two objects.
            "soql_field Region__c declaration ambiguous_receiver",
            "soql_field Shipment__c.Payer__c declaration",
            "soql_field Shipment__c.Owner.Alias unresolved",
            // `FROM Parcels__r`: the master-detail field that makes the relationship.
            "soql_field Parcel__c.Shipment__c declaration",
            "soql_field Parcel__c.Weight__c declaration",
            "soql_field Parcel__c.Shipment__c declaration",
            "soql_object Shipment__c declaration",
            "soql_field Shipment__c.Carrier__c declaration",
            "soql_field Shipment__c.Delivered__c declaration",
        ]
    );
}

#[test]
fn a_query_through_a_lookup_declared_twice_keeps_every_end_declared_or_not() {
    let query = apex("ShipmentTracker.payers()");

    assert_eq!(
        query,
        [
            "type Shipment__c declaration",
            // `Payer__r.No__c`
            "soql_field Account.No__c declaration",
            "soql_field Contact.No__c unresolved",
            "soql_field Shipment__c.Payer__c declaration",
            // `Payer__r.Name`
            "soql_field Account.Name unresolved",
            "soql_field Contact.Name unresolved",
            "soql_field Shipment__c.Payer__c declaration",
            "soql_object Shipment__c declaration",
            "soql_field Shipment__c.Kg__c declaration",
        ]
    );
}

#[test]
fn dml_and_a_record_initialiser_resolve_to_declared_objects_and_fields() {
    let dml = apex("ShipmentTracker.deliver(list<shipment__c>)");

    assert_eq!(
        dml,
        [
            "type Shipment__c declaration",
            "dml_object Shipment__c declaration",
            "dml_field Shipment__c.Tracking_Id__c declaration",
            "dml_object Parcel__c declaration",
            "construct Parcel__c declaration",
            "sobject_field Parcel__c.Weight__c declaration",
            // The object is declared; this field of it is not.
            "sobject_field Parcel__c.Colour__c unresolved",
        ]
    );
}

#[test]
fn field_tokens_and_a_label_resolve_to_their_declarations() {
    let tokens = apex("ShipmentTracker.title()");

    assert_eq!(
        tokens,
        [
            "static_access Shipment__c.Weight__c declaration",
            "static_access Shipment__c.Delivered__c declaration",
            "static_access Label.Shipment_Title declaration",
        ]
    );
}

#[test]
fn a_custom_setting_or_metadata_type_read_through_its_static_methods_is_referenced() {
    let members = apex("ShipmentTracker.isConfigured()");

    // The variable's type, `getOrgDefaults()` and `getAll()`. The methods
    // themselves, and `settings.get(..)`, are the platform's.
    assert_eq!(
        members,
        [
            "type Shipping_Settings__c declaration",
            "type Shipping_Settings__c declaration",
            "type Shipping_Plugin__mdt declaration",
        ]
    );
}

#[test]
fn a_trigger_resolves_to_the_object_it_is_on() {
    let trigger: Vec<String> = parsed(&in_app("triggers/ShipmentTrigger.trigger"))
        .into_iter()
        .filter(|row| row.starts_with("trigger_object"))
        .collect();

    assert_eq!(trigger, ["trigger_object Shipment__c object declaration"]);
}

#[test]
fn an_object_and_a_label_that_no_metadata_declares_stay_unresolved() {
    let undeclared: Vec<String> = ROWS
        .iter()
        .filter(|row| text(row, "tier") == "parsed")
        .filter(|row| ["Invoice__c", "Label.Tariff_Title"].contains(&text(row, "referenced")))
        .map(|row| text(row, "resolution").to_string())
        .collect();

    assert!(!undeclared.is_empty());
    assert!(
        undeclared
            .iter()
            .all(|resolution| resolution == "unresolved"),
        "{undeclared:?}"
    );
}

// The contract.

fn refs_schema() -> Schema {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schema/refs.schema.json");
    Schema::parse(&std::fs::read_to_string(path).expect("the schema exists"))
}

fn grant() -> Value {
    json!({
        "row": "reference", "tier": "parsed", "kind": "grant",
        "referenced": "Shipment__c", "referenced_type": "object", "resolution": "declaration",
        "referrer": "Shipping", "package": "force-app",
        "file": "force-app/permissionsets/Shipping.permissionset-meta.xml",
        "line": 46, "column": 17
    })
}

#[test]
fn the_schema_accepts_a_reference_read_from_metadata() {
    let schema = refs_schema();
    let mut ambiguous = grant();
    ambiguous["kind"] = json!("formula");
    ambiguous["detail"] = json!("ambiguous_receiver");
    ambiguous["candidates"] = json!(["Account.Region__c", "Contact.Region__c"]);
    // `$Action.Account.New`: a formula global whose kind is not known.
    let mut unknown_global = grant();
    unknown_global["kind"] = json!("formula");
    unknown_global["referenced"] = json!("$Action.Account.New");
    unknown_global["resolution"] = json!("unresolved");
    unknown_global
        .as_object_mut()
        .unwrap()
        .remove("referenced_type");
    let mut from_a_record = grant();
    from_a_record["kind"] = json!("cmdt");
    from_a_record["source_field"] = json!("Handler__c");
    let not_read = json!({
        "row": "unsupported_element", "package": "force-app",
        "file": "force-app/profiles/Admin.profile-meta.xml",
        "element": "layoutAssignments", "reason": "not_interpreted", "line": 8, "column": 5
    });

    for row in [grant(), ambiguous, unknown_global, from_a_record, not_read] {
        assert_eq!(schema.errors(&row), [] as [String; 0], "{row}");
    }
}

#[test]
fn the_schema_refuses_metadata_rows_that_break_the_contract() {
    let schema = refs_schema();
    let with = |key: &str, value: Value| {
        let mut row = grant();
        row[key] = value;
        row
    };
    let without = |key: &str| {
        let mut row = grant();
        row.as_object_mut().unwrap().remove(key);
        row
    };
    let unmodelled = json!({
        "row": "unmodelled_file", "package": "force-app", "file": "force-app/tabs/Shipment__c.tab-meta.xml"
    });
    // With candidates, so that the detail is all that is wrong with it.
    let mut apex_detail = with("detail", json!("unknown_type"));
    apex_detail["candidates"] = json!(["A.B", "C.D"]);

    let broken = [
        (
            "a kind of thing that nothing is",
            with("referenced_type", json!("widget")),
        ),
        ("a detail that only Apex has", apex_detail),
        (
            "candidates without a detail",
            with("candidates", json!(["A.B", "C.D"])),
        ),
        (
            "a detail without candidates",
            with("detail", json!("ambiguous_receiver")),
        ),
        ("no referrer", with("referrer", Value::Null)),
        (
            "a source field that is empty",
            with("source_field", json!("")),
        ),
        (
            "an Apex kind resolved without the kind of thing it names",
            {
                let mut call = with("kind", json!("call"));
                call.as_object_mut().unwrap().remove("referenced_type");
                call
            },
        ),
        ("an unmodelled file without its suffix", unmodelled),
        (
            "a grant that does not say what kind of thing it names",
            without("referenced_type"),
        ),
        (
            "a record's value without the field it came from",
            with("kind", json!("cmdt")),
        ),
        (
            "a field it came from on a row that is not a record's value",
            with("source_field", json!("Handler__c")),
        ),
        (
            "an element that was not read, without the reason",
            json!({
                "row": "unsupported_element", "package": "force-app",
                "file": "force-app/profiles/Admin.profile-meta.xml",
                "element": "layoutAssignments", "line": 8, "column": 5
            }),
        ),
        (
            "an element that was not read, with an empty path",
            json!({
                "row": "unsupported_element", "package": "force-app",
                "file": "force-app/profiles/Admin.profile-meta.xml",
                "element": "", "reason": "no_name", "line": 8, "column": 5
            }),
        ),
        (
            "an element that was not read, for a reason that is not one of the two",
            json!({
                "row": "unsupported_element", "package": "force-app",
                "file": "force-app/flows/Review.flow-meta.xml",
                "element": "futureSettings", "reason": "syntax", "line": 8, "column": 5
            }),
        ),
    ];

    for (what, row) in broken {
        assert!(
            !schema.errors(&row).is_empty(),
            "{what} should be refused: {row}"
        );
    }
}

// Malformed input. These files are written to a project of their own: the
// fixture project is for what is well-formed.

/// The rows for a project that has these files under one package directory.
fn extract_project(name: &str, files: &[(&str, &str)]) -> Vec<Value> {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    for (path, content) in files {
        let path = root.join("app").join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
    std::fs::write(
        root.join("sfdx-project.json"),
        r#"{"packageDirectories": [{"path": "app"}]}"#,
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_sf-impact-extract"))
        .arg("--root")
        .arg(&root)
        .args(["--format", "jsonl"])
        .output()
        .expect("sf-impact-extract runs");
    assert!(
        output.status.success(),
        "sf-impact-extract failed: {output:?}"
    );
    let text = String::from_utf8(output.stdout).expect("output is UTF-8");
    // The rows of the tree: all but the first, which says what the file is of.
    let rows = text.lines().skip(1).map(serde_json::from_str);
    rows.collect::<Result<_, _>>().expect("every line is JSON")
}

/// Each row as `row [kind] [what]`, where `what` is the symbol, the name
/// referred to or the element.
fn outline(rows: &[Value]) -> Vec<String> {
    let line = |row: &Value| {
        let parts = ["row", "kind", "symbol", "referenced", "element", "reason"];
        let present: Vec<&str> = parts.iter().filter_map(|key| row[*key].as_str()).collect();
        present.join(" ")
    };
    rows.iter()
        .filter(|row| text(row, "tier") != "textual")
        .map(line)
        .collect()
}

const FIELD: &str = "objects/Broken__c/fields/Parent__c.field-meta.xml";

/// Regression: the file ends inside the element that should name
/// the object.
#[test]
fn a_lookup_cut_off_before_its_target_is_reported_without_an_empty_reference() {
    let rows = extract_project("cut-off", &[(FIELD, "<CustomField>\n<referenceTo>")]);

    assert_eq!(
        outline(&rows),
        [
            "parse_error",
            "declaration Broken__c.Parent__c",
            "unsupported_element referenceTo no_name",
        ]
    );
}

#[test]
fn an_element_that_should_name_something_and_is_empty_is_a_row_not_an_empty_reference() {
    let field = "<CustomField><referenceTo> </referenceTo><summarizedField/>\
                 <valueSet><valueSetName></valueSetName></valueSet></CustomField>";
    let grants = "<PermissionSet><classAccesses><apexClass/><enabled>true</enabled></classAccesses>\
                  <fieldPermissions><readable>true</readable></fieldPermissions></PermissionSet>";
    let labels = "<CustomLabels><labels><fullName></fullName><value>x</value></labels>\
                  <labels><value>y</value></labels></CustomLabels>";
    let record = "<CustomMetadata><values><field></field><value>x</value></values>\
                  <values><value>y</value></values></CustomMetadata>";

    let rows = extract_project(
        "empty-names",
        &[
            (FIELD, field),
            ("permissionsets/Hollow.permissionset-meta.xml", grants),
            ("labels/CustomLabels.labels-meta.xml", labels),
            ("customMetadata/Rules.Empty.md-meta.xml", record),
        ],
    );

    assert_eq!(
        outline(&rows),
        [
            "declaration Rules.Empty",
            "reference cmdt_type Rules__mdt",
            "unsupported_element values no_name",
            "unsupported_element values no_name",
            "unsupported_element labels/fullName no_name",
            "unsupported_element labels no_name",
            "declaration Broken__c.Parent__c",
            "unsupported_element referenceTo no_name",
            "unsupported_element summarizedField no_name",
            "unsupported_element valueSet/valueSetName no_name",
            "declaration Hollow",
            "unsupported_element classAccesses/apexClass no_name",
            "unsupported_element fieldPermissions no_name",
        ]
    );
}

/// Regressions: a lookup filter compares with `$Source` and no
/// field, and with a global this tool does not know.
#[test]
fn a_lookup_filter_value_that_cannot_be_read_is_reported_not_made_an_untyped_reference() {
    let field = "<CustomField><type>Lookup</type><referenceTo>Account</referenceTo><lookupFilter>\
                 <filterItems><field>Account.Name</field><valueField>$Source</valueField></filterItems>\
                 <filterItems><field>$Future.Field</field><valueField>$Source.Kg__c</valueField></filterItems>\
                 </lookupFilter></CustomField>";

    let rows = extract_project(
        "filter-globals",
        &[("objects/Row__c/fields/Link__c.field-meta.xml", field)],
    );

    assert_eq!(
        outline(&rows),
        [
            "declaration Row__c.Link__c",
            "reference lookup Account",
            "reference lookup_filter Account.Name",
            "reference lookup_filter Row__c.Kg__c",
            "unsupported_element lookupFilter/filterItems/valueField not_interpreted",
            "unsupported_element lookupFilter/filterItems/field not_interpreted",
        ]
    );
    let schema = refs_schema();
    for row in &rows {
        assert_eq!(schema.errors(row), [] as [String; 0], "{row}");
    }
}

// One contract for what is not read, whatever the file: a structure that is
// well-formed and not read is an `unsupported_element`; a syntax error is a
// `parse_error`; a file with both has both.

const FLOW: &str = "flows/Review.flow-meta.xml";

#[test]
fn a_flow_structure_that_is_not_known_is_an_element_that_was_not_read_not_a_parse_error() {
    let flow = "<Flow>\n<recordLookups>\n<name>Find</name>\n<object>Account</object>\n\
                <futureFilter><mode>Deferred</mode></futureFilter>\n</recordLookups>\n\
                <futureSettings><mode>Deferred</mode></futureSettings>\n</Flow>\n";

    let rows = extract_project("flow-unknown", &[(FLOW, flow)]);

    assert_eq!(
        outline(&rows),
        [
            "declaration Review",
            "reference flow_object Account",
            "unsupported_element recordLookups/futureFilter not_interpreted",
            "unsupported_element futureSettings not_interpreted",
        ]
    );
    let lines: Vec<(u64, u64)> = rows
        .iter()
        .filter(|row| text(row, "row") == "unsupported_element")
        .map(|row| {
            (
                row["line"].as_u64().unwrap(),
                row["column"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(lines, [(5, 1), (7, 1)]);
}

#[test]
fn a_flow_with_a_syntax_error_and_an_unknown_structure_has_both_rows() {
    let flow = "<Flow>\n<futureSettings><mode>Deferred</mode></futureSettings>\n\
                <recordLookups><object>Account</object></wrong>\n</Flow>\n";

    let rows = extract_project("flow-both", &[(FLOW, flow)]);

    assert_eq!(
        outline(&rows),
        [
            "parse_error",
            "declaration Review",
            "reference flow_object Account",
            "unsupported_element futureSettings not_interpreted",
        ]
    );
    let error = rows
        .iter()
        .find(|row| text(row, "row") == "parse_error")
        .unwrap();
    assert_eq!(
        (error.get("errors"), error["line"].as_u64()),
        (None, Some(3))
    );
}

#[test]
fn a_flow_with_only_a_syntax_error_has_a_parse_error_and_no_unsupported_element() {
    let rows = extract_project(
        "flow-syntax",
        &[(FLOW, "<Flow>\n<status>Draft</wrong>\n</Flow>\n")],
    );

    assert_eq!(outline(&rows), ["parse_error", "declaration Review"]);
}

#[test]
fn every_diagnostic_of_a_flow_is_valid_against_the_schema() {
    let schema = refs_schema();
    let sources = [
        "<Flow><futureSettings><mode>Deferred</mode></futureSettings></Flow>",
        "<Flow><futureSettings><mode>Deferred</mode></futureSettings><status>Draft</wrong></Flow>",
        "<Flow><recordLookups><a><b><c>x</c></b></a></recordLookups>",
        "<NotAFlow><futureSettings><mode>Deferred</mode></futureSettings></NotAFlow>",
    ];

    for (number, source) in sources.iter().enumerate() {
        let rows = extract_project(&format!("flow-valid-{number}"), &[(FLOW, *source)]);

        for row in &rows {
            assert_eq!(
                schema.errors(row),
                [] as [String; 0],
                "{source:?} gave {row}"
            );
        }
        // The control: the file was read, and something was said about it.
        assert!(
            rows.iter()
                .any(|row| ["parse_error", "unsupported_element"].contains(&text(row, "row"))),
            "{source:?}"
        );
    }
}

#[test]
fn a_file_whose_name_leaves_its_component_without_a_name_is_not_modelled() {
    let rows = extract_project(
        "nameless",
        &[
            ("objects/Broken__c/fields/.field-meta.xml", "<CustomField/>"),
            ("customMetadata/Rules..md-meta.xml", "<CustomMetadata/>"),
            ("customMetadata/.Record.md-meta.xml", "<CustomMetadata/>"),
            ("permissionsets/.permissionset-meta.xml", "<PermissionSet/>"),
            ("profiles/Sales..profile-meta.xml", "<Profile/>"),
        ],
    );

    assert_eq!(
        outline(&rows),
        [
            "unmodelled_file",
            "unmodelled_file",
            "unmodelled_file",
            "unmodelled_file",
            "unmodelled_file",
        ]
    );
}

#[test]
fn every_row_for_malformed_metadata_is_valid_against_the_schema() {
    let schema = refs_schema();
    let cut_off = "<CustomField>\n<referenceTo>";
    let sources = [
        "",
        "<",
        "not xml",
        cut_off,
        "<CustomField><formula>IF(A__c",
        "<CustomField><referenceTo></referenceTo><formula></formula><summaryForeignKey/></CustomField>",
        "<CustomField><lookupFilter><filterItems><field/><valueField> </valueField></filterItems></lookupFilter></CustomField>",
        "<CustomField><lookupFilter><filterItems><field>$Source</field><valueField>$Future.Field</valueField></filterItems><filterItems><field>Bare__c</field><valueField>$Api.Session_ID</valueField></filterItems></lookupFilter></CustomField>",
        "<PermissionSet><classAccesses><apexClass></apexClass></classAccesses><tabSettings/><flowAccesses><flow/>",
        "<CustomLabels><labels><fullName/></labels><labels>",
        "<CustomMetadata><values><field/><value>x</value></values><values><field>F__c</field>",
        "<ValidationRule><errorDisplayField/><errorConditionFormula>$Label.</errorConditionFormula></ValidationRule>",
        "<CustomObject><unknown/></CustomObject>",
    ];
    // Each source as each kind of modelled file.
    let paths = [
        "objects/O__c/O__c.object-meta.xml",
        "objects/O__c/fields/F__c.field-meta.xml",
        "objects/O__c/validationRules/R.validationRule-meta.xml",
        "permissionsets/P.permissionset-meta.xml",
        "profiles/P.profile-meta.xml",
        "customMetadata/T.R.md-meta.xml",
        "labels/CustomLabels.labels-meta.xml",
        "customPermissions/C.customPermission-meta.xml",
    ];

    for (number, source) in sources.iter().enumerate() {
        let files: Vec<(&str, &str)> = paths.iter().map(|path| (*path, *source)).collect();
        let rows = extract_project(&format!("malformed-{number}"), &files);

        for row in &rows {
            let errors = schema.errors(row);
            assert!(errors.is_empty(), "{source:?} gave {row}: {errors:?}");
        }
        // The control: each file was read and gave at least one row.
        let files_with_rows: std::collections::BTreeSet<&str> =
            rows.iter().map(|row| text(row, "file")).collect();
        assert_eq!(files_with_rows.len(), paths.len(), "{source:?}");
    }
}
