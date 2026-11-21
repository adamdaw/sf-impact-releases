use super::*;

fn no_name_cases(cases: &[(&str, &str, &str)]) {
    let mut failures = Vec::new();
    for (parent, leaf, siblings) in cases {
        for empty in [
            format!("<{leaf}/>"),
            format!("<{leaf}></{leaf}>"),
            format!("<{leaf}> \n </{leaf}>"),
        ] {
            let source = format!("<Flow>\n<{parent}>{empty}{siblings}</{parent}>\n</Flow>");
            let rows = flow(&source);
            let diagnostics: Vec<_> = rows
                .iter()
                .filter(|r| r["row"] == "unsupported_element")
                .collect();
            let expected = format!("{parent}/{leaf}");
            if diagnostics.len() != 1
                || diagnostics[0]["element"] != expected
                || diagnostics[0]["reason"] != "no_name"
                || diagnostics[0]["line"] != 2
                || diagnostics[0]["column"] != parent.len() + 3
                || !refs(&rows).is_empty()
                || rows.iter().any(|r| r["row"] == "parse_error")
            {
                failures.push(format!("{source}: {rows:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn empty_structural_flow_names_are_no_name_diagnostics() {
    no_name_cases(&[
        ("start", "object", ""),
        ("variables", "objectType", ""),
        ("transforms", "outputSObjectType", ""),
        ("dynamicChoiceSets", "picklistObject", ""),
        ("variables", "apexClass", ""),
        ("subflows", "flowName", ""),
        ("actionCalls", "overriddenFlow", ""),
        ("actionCalls", "actionName", "<actionType>apex</actionType>"),
        (
            "actionCalls",
            "nameSegment",
            "<actionType>flow</actionType>",
        ),
        (
            "steps",
            "entryActionName",
            "<entryActionType>component</entryActionType>",
        ),
        ("steps", "exitActionName", ""),
        ("fields", "extensionName", ""),
        ("fields", "objectFieldReference", ""),
        ("filters", "field", ""),
        ("recordLookups", "queriedFields", ""),
        ("start", "triggerFields", ""),
        ("dynamicChoiceSets", "displayField", ""),
        ("dynamicChoiceSets", "valueField", ""),
        ("dynamicChoiceSets", "picklistField", ""),
        ("recordLookups", "sortField", ""),
        ("fields", "recordField", ""),
        ("fields", "fieldSelection", ""),
        ("transformValueActions", "outputFieldApiName", ""),
        ("mapItems", "assignToFieldReference", ""),
        ("recordUpdates", "inputReference", ""),
        ("dataTypeMappings", "typeValue", ""),
        ("waitEvents", "eventType", ""),
    ]);
}

#[test]
fn empty_flow_expression_and_reference_leaves_are_no_name_diagnostics() {
    no_name_cases(&[
        ("value", "elementReference", ""),
        ("conditions", "leftValueReference", ""),
        ("assignmentItems", "assignToReference", ""),
        ("transforms", "inputReference", ""),
        ("loops", "collectionReference", ""),
        ("recordCreates", "assignRecordIdToReference", ""),
        ("formulas", "expression", ""),
        ("validationRule", "formulaExpression", ""),
        ("start", "filterFormula", ""),
        ("conditions", "formula", ""),
    ]);
}

#[test]
fn empty_process_metadata_object_name_is_reported_but_literal_text_is_not() {
    let rows = flow(
        "<Flow><processMetadataValues><name>ObjectType</name><value><stringValue/></value></processMetadataValues><value><stringValue/></value><description/><label/></Flow>",
    );
    let diagnostics: Vec<_> = rows
        .iter()
        .filter(|r| r["row"] == "unsupported_element")
        .collect();
    assert_eq!(diagnostics.len(), 1, "{rows:?}");
    assert_eq!(
        diagnostics[0]["element"],
        "processMetadataValues/value/stringValue"
    );
    assert_eq!(diagnostics[0]["reason"], "no_name");
    assert!(refs(&rows).is_empty());
}

#[test]
fn a_truncated_flow_name_has_both_diagnostics_and_no_empty_reference() {
    let rows = flow("<Flow>\n<start><object>");
    assert!(rows.iter().any(|r| r["row"] == "parse_error"));
    let missing = rows
        .iter()
        .find(|r| r["row"] == "unsupported_element")
        .expect("missing name is visible");
    assert_eq!(missing["element"], "start/object");
    assert_eq!(missing["reason"], "no_name");
    assert_eq!(missing["line"], 2);
    assert_eq!(missing["column"], 8);
    assert!(refs(&rows).is_empty());
}

#[test]
fn a_message_channel_keeps_unknown_structure_and_malformed_xml_visible() {
    let rows = extract(&[
        (
            "pkg/messageChannels/Changed.messageChannel-meta.xml",
            "<LightningMessageChannel><masterLabel>Changed</masterLabel><isExposed>true</isExposed><lightningMessageFields><fieldName>recordId</fieldName></lightningMessageFields><future><object>Thing__c</object></future></LightningMessageChannel>",
        ),
        (
            "other/messageChannels/Changed.messageChannel-meta.xml",
            "<LightningMessageChannel>",
        ),
        (
            "pkg/lwc/example/example.js",
            "import '@salesforce/messageChannel/Changed__c'; import '@salesforce/messageChannel/Missing__c'; import '@salesforce/messageChannel/ns__Changed__c';",
        ),
    ]);
    let declarations: Vec<_> = rows
        .iter()
        .filter(|r| r["row"] == "declaration" && r["type"] == "message_channel")
        .collect();
    assert_eq!(declarations.len(), 2);
    assert!(
        declarations
            .iter()
            .all(|r| r["symbol"] == "MessageChannel.Changed__c" && r["collision"] == true)
    );
    let channel = refs(&rows)
        .into_iter()
        .find(|r| r["referenced"] == "MessageChannel.Changed__c")
        .unwrap();
    assert_eq!(channel["resolution"], "declaration");
    assert!(channel.get("detail").is_none());
    for (name, resolution) in [
        ("MessageChannel.Missing__c", "unresolved"),
        ("MessageChannel.ns__Changed__c", "external"),
    ] {
        assert!(
            refs(&rows)
                .iter()
                .any(|r| r["referenced"] == name && r["resolution"] == resolution)
        );
    }
    let unsupported: Vec<_> = rows
        .iter()
        .filter(|r| r["row"] == "unsupported_element")
        .collect();
    assert_eq!(unsupported.len(), 1, "{unsupported:?}");
    assert_eq!(unsupported[0]["element"], "future");
    assert_eq!(unsupported[0]["reason"], "not_interpreted");
    assert!(rows.iter().any(|r| r["row"] == "parse_error"
        && r["file"] == "other/messageChannels/Changed.messageChannel-meta.xml"));
}
