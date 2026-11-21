//! A well-formed shape the reader does not model is a diagnostic
//! row, never a silent zero. None of these shapes is modelled.
use super::*;

/// The paths of the structures that were not read. Each is well-formed,
/// so it is never a parse error.
fn not_read(rows: &[Value]) -> Vec<&str> {
    assert!(!rows.iter().any(|r| r["row"] == "parse_error"), "{rows:#?}");
    rows.iter()
        .filter(|r| r["row"] == "unsupported_element")
        .map(|r| {
            assert_eq!(r["reason"], "not_interpreted", "{r}");
            r["element"].as_str().unwrap()
        })
        .collect()
}

fn parse_errors(rows: &[Value]) -> usize {
    rows.iter().filter(|r| r["row"] == "parse_error").count()
}

#[test]
fn field_assignment_under_an_unknown_wrapper_is_not_read() {
    let rows = flow(
        "<Flow><futureMap><assignToFieldReference>Industry</assignToFieldReference></futureMap></Flow>",
    );
    assert_eq!(not_read(&rows), ["futureMap"]);
    assert!(refs(&rows).is_empty(), "{rows:#?}");
}

#[test]
fn unknown_style_property_structure_is_not_read() {
    let rows = flow(
        r#"<Flow><screens><name>Screen</name><fields><name>Body</name><fieldType>DisplayText</fieldType>
<styleProperties>
    <verticalAlignment><stringValue>top</stringValue></verticalAlignment>
    <futureBorder><stringValue>thin</stringValue></futureBorder>
</styleProperties>
</fields></screens></Flow>"#,
    );
    assert_eq!(
        not_read(&rows),
        ["screens/fields/styleProperties/futureBorder"]
    );
}

#[test]
fn unknown_leaf_directly_in_style_properties_is_not_read() {
    let rows = flow(
        "<Flow><screens><fields><name>Body</name><styleProperties><futurePadding>4</futurePadding></styleProperties></fields></screens></Flow>",
    );
    assert_eq!(
        not_read(&rows),
        ["screens/fields/styleProperties/futurePadding"]
    );
}

#[test]
fn unknown_leaf_under_a_known_element_is_not_read() {
    let rows = flow(
        "<Flow><label>Example</label><sourceTemplate>ns__Template_Flow</sourceTemplate><start><segment>Loyal_Customers</segment><locationX>50</locationX></start></Flow>",
    );
    assert_eq!(not_read(&rows), ["sourceTemplate", "start/segment"]);
    assert!(refs(&rows).is_empty(), "{rows:#?}");
}

#[test]
fn unknown_leaf_with_a_merge_token_is_read_not_diagnosed() {
    let rows = flow("<Flow><futureHelp>{!$Label.Greeting}</futureHelp></Flow>");
    has(&rows, "flow_label", "Label.Greeting");
    assert!(not_read(&rows).is_empty(), "{rows:#?}");
}

#[test]
fn type_only_reexport_of_a_module_is_a_parse_error() {
    for source in [
        "export type { U } from 'c/other';",
        "export type * from 'c/other';",
        "export type * as Other from 'c/other';",
    ] {
        let rows = js(source);
        assert_eq!(parse_errors(&rows), 1, "{source}: {rows:#?}");
        assert!(refs(&rows).is_empty(), "{source}: {rows:#?}");
    }
}

#[test]
fn type_only_import_of_a_module_stays_a_parse_error() {
    for source in [
        "import type { T } from 'c/types';",
        "import type * as Types from 'c/types';",
    ] {
        let rows = js(source);
        assert_eq!(parse_errors(&rows), 1, "{source}: {rows:#?}");
        assert!(refs(&rows).is_empty(), "{source}: {rows:#?}");
    }
}

#[test]
fn type_declarations_naming_no_module_are_not_diagnosed() {
    for source in [
        "export type U = { name: string };",
        "type U = string; export type { U };",
        "import type from 'c/childPanel';",
    ] {
        let rows = js(source);
        assert_eq!(parse_errors(&rows), 0, "{source}: {rows:#?}");
    }
    has(
        &js("import type from 'c/childPanel';"),
        "lwc_component",
        "childPanel",
    );
}

// Each leaf below can name a dependency (a form, an
// external service, an Apex action's variable, a member of a component or
// subflow), so it is not a control leaf that names nothing.

#[test]
fn automation_event_name_is_not_read() {
    let rows = flow(
        "<Flow><start><eventName>MyExternalService</eventName><eventType>externalEvent</eventType><triggerType>AutomationEvent</triggerType></start></Flow>",
    );
    assert_eq!(not_read(&rows), ["start/eventName"]);
    assert!(refs(&rows).is_empty(), "{rows:#?}");
}

#[test]
fn screen_trigger_event_name_is_not_read() {
    let rows = flow(
        "<Flow><screens><triggers><eventName>flow__screenfieldclick</eventName><eventSource>Button</eventSource></triggers></screens></Flow>",
    );
    assert_eq!(not_read(&rows), ["screens/triggers/eventName"]);
}

#[test]
fn generic_type_variable_of_an_action_is_not_read() {
    let rows = flow(
        "<Flow><actionCalls><dataTypeMappings><typeName>T__records</typeName><typeValue>Account</typeValue></dataTypeMappings></actionCalls></Flow>",
    );
    has(&rows, "flow_object", "Account");
    assert_eq!(not_read(&rows), ["actionCalls/dataTypeMappings/typeName"]);
}

#[test]
fn object_type_value_outside_a_type_mapping_is_not_read() {
    let rows = flow("<Flow><actionCalls><typeValue>Account</typeValue></actionCalls></Flow>");
    assert_eq!(not_read(&rows), ["actionCalls/typeValue"]);
}

#[test]
fn undocumented_stage_criteria_logic_is_not_read() {
    let rows = flow(
        "<Flow><steppedStages><exitCriteriaLogic>and</exitCriteriaLogic><steps><entryCriteriaLogic>and</entryCriteriaLogic></steps></steppedStages></Flow>",
    );
    assert_eq!(
        not_read(&rows),
        [
            "steppedStages/exitCriteriaLogic",
            "steppedStages/steps/entryCriteriaLogic"
        ]
    );
}

#[test]
fn parameter_and_assignment_names_are_not_read() {
    // Each name is a member of what its element configures: an invocable
    // variable of an Apex action, an LWC property, a subflow variable.
    for (element, parameter) in [
        ("actionCalls", "inputParameters"),
        ("actionCalls", "outputParameters"),
        ("subflows", "inputAssignments"),
        ("subflows", "outputAssignments"),
        ("stageSteps", "entryActionInputParameters"),
        ("stageSteps", "exitActionInputParameters"),
    ] {
        let rows = flow(&format!(
            "<Flow><{element}><name>Step</name><{parameter}><name>recordId</name></{parameter}></{element}></Flow>"
        ));
        assert_eq!(not_read(&rows), [format!("{element}/{parameter}/name")]);
    }
}

#[test]
fn own_names_of_elements_and_metadata_keys_name_nothing() {
    let rows = flow(
        "<Flow><variables><name>count</name><dataType>Number</dataType></variables><processMetadataValues><name>BuilderType</name><value><stringValue>LightningFlowBuilder</stringValue></value></processMetadataValues><customProperties><name>ScreenProgressIndicator</name></customProperties></Flow>",
    );
    assert!(not_read(&rows).is_empty(), "{rows:#?}");
}

#[test]
fn literal_step_assignee_is_not_read() {
    // A Group or Queue assignee is named by a literal string.
    let rows = flow(
        "<Flow><orchestratedStages><stageSteps><assignees><assignee><stringValue>Support_Queue</stringValue></assignee><assigneeType>Queue</assigneeType></assignees></stageSteps></orchestratedStages></Flow>",
    );
    assert_eq!(
        not_read(&rows),
        ["orchestratedStages/stageSteps/assignees/assignee/stringValue"]
    );
}

#[test]
fn literal_values_elsewhere_name_nothing() {
    let rows = flow(
        "<Flow><constants><name>limit</name><value><stringValue>Support_Queue</stringValue></value></constants><decisions><rules><conditions><rightValue><stringValue>Closed</stringValue></rightValue></conditions></rules></decisions></Flow>",
    );
    assert!(not_read(&rows).is_empty(), "{rows:#?}");
}

// A parameter, assignment or type-variable name is a
// member of its element's target. It is quiet when that target is a
// reference row (whatever its resolution) or is proven platform by the rule
// that omits the target, and reported when there is no target to stand for it.

/// The elements reported as not read, whatever the reason.
fn reported(rows: &[Value]) -> Vec<&str> {
    rows.iter()
        .filter(|r| r["row"] == "unsupported_element")
        .map(|r| r["element"].as_str().unwrap())
        .collect()
}

#[test]
fn parameter_names_of_an_apex_action_whose_class_is_a_row_are_quiet() {
    let rows = flow(
        "<Flow><actionCalls><name>Send</name><actionName>InvoiceSender</actionName><actionType>apex</actionType><inputParameters><name>invoiceId</name><value><elementReference>recordId</elementReference></value></inputParameters><outputParameters><name>sent</name><assignToReference>wasSent</assignToReference></outputParameters></actionCalls></Flow>",
    );
    has(&rows, "flow_action", "InvoiceSender");
    assert!(reported(&rows).is_empty(), "{rows:#?}");
}

#[test]
fn names_under_an_unresolved_target_that_is_a_row_are_quiet() {
    for element in [
        "<actionCalls><actionName>emailSimple</actionName><actionType>emailSimple</actionType><inputParameters><name>emailBody</name></inputParameters></actionCalls>",
        "<subflows><flowName>Missing_Flow</flowName><inputAssignments><name>recordId</name></inputAssignments><outputAssignments><name>result</name><assignToReference>r</assignToReference></outputAssignments></subflows>",
        "<screens><fields><name>Picker</name><extensionName>c:missingPicker</extensionName><fieldType>ComponentInstance</fieldType><inputParameters><name>label</name></inputParameters></fields></screens>",
        "<orchestratedStages><stageSteps><actionName>Step_Flow</actionName><actionType>stepBackground</actionType><entryActionName>Entry_Flow</entryActionName><entryActionType>EvaluationFlow</entryActionType><inputParameters><name>ActionInput__RecordId</name></inputParameters><entryActionInputParameters><name>recordId</name></entryActionInputParameters></stageSteps></orchestratedStages>",
    ] {
        let rows = flow(&format!("<Flow>{element}</Flow>"));
        assert!(!refs(&rows).is_empty(), "{element}: {rows:#?}");
        assert!(reported(&rows).is_empty(), "{element}: {rows:#?}");
    }
}

#[test]
fn names_of_proven_platform_elements_are_quiet() {
    for element in [
        "<screens><fields><name>Address</name><extensionName>flowruntime:address</extensionName><fieldType>ComponentInstance</fieldType><inputParameters><name>street</name></inputParameters></fields></screens>",
        "<screens><fields><name>Upload</name><extensionName>forceContent:fileUpload</extensionName><fieldType>ComponentInstance</fieldType><inputParameters><name>recordId</name></inputParameters></fields></screens>",
        "<screens><fields><name>Column</name><fieldType>Region</fieldType><inputParameters><name>width</name><value><stringValue>6</stringValue></value></inputParameters></fields></screens>",
        "<waits><waitEvents><name>Later</name><eventType>AlarmEvent</eventType><inputParameters><name>AlarmTime</name></inputParameters></waitEvents></waits>",
    ] {
        let rows = flow(&format!("<Flow>{element}</Flow>"));
        assert!(reported(&rows).is_empty(), "{element}: {rows:#?}");
    }
}

#[test]
fn names_with_no_or_an_empty_target_are_reported() {
    for (element, path) in [
        (
            "<actionCalls><actionName></actionName><actionType>apex</actionType><inputParameters><name>invoiceId</name></inputParameters></actionCalls>",
            "actionCalls/inputParameters/name",
        ),
        (
            "<subflows><inputAssignments><name>recordId</name></inputAssignments></subflows>",
            "subflows/inputAssignments/name",
        ),
        (
            // A region's only platform input is its width.
            "<screens><fields><fieldType>Region</fieldType><inputParameters><name>height</name></inputParameters></fields></screens>",
            "screens/fields/inputParameters/name",
        ),
        (
            // The step's action does not stand for its exit action's inputs.
            "<orchestratedStages><stageSteps><actionName>Step_Flow</actionName><actionType>stepBackground</actionType><exitActionInputParameters><name>recordId</name></exitActionInputParameters></stageSteps></orchestratedStages>",
            "orchestratedStages/stageSteps/exitActionInputParameters/name",
        ),
        (
            // A start element's inputs have no target.
            "<start><inputParameters><name>segment</name></inputParameters></start>",
            "start/inputParameters/name",
        ),
    ] {
        let rows = flow(&format!("<Flow>{element}</Flow>"));
        assert!(reported(&rows).contains(&path), "{element}: {rows:#?}");
    }
}

#[test]
fn type_variable_under_a_target_that_is_a_row_is_quiet() {
    for element in [
        // The mapping comes before the target in Flow Builder's order.
        "<screens><fields><name>Table</name><dataTypeMappings><typeName>T</typeName><typeValue>Account</typeValue></dataTypeMappings><extensionName>c:datatable</extensionName><fieldType>ComponentInstance</fieldType></fields></screens>",
        "<actionCalls><actionName>SortRecords</actionName><actionType>apex</actionType><dataTypeMappings><typeName>T__records</typeName><typeValue>Account</typeValue></dataTypeMappings></actionCalls>",
    ] {
        let rows = flow(&format!("<Flow>{element}</Flow>"));
        has(&rows, "flow_object", "Account");
        assert!(reported(&rows).is_empty(), "{element}: {rows:#?}");
    }
}

#[test]
fn only_a_region_input_named_width_is_platform() {
    // A Region's one platform member is its `width` input.
    for (member, path) in [
        (
            "<outputParameters><name>width</name><assignToReference>columnWidth</assignToReference></outputParameters>",
            "screens/fields/outputParameters/name",
        ),
        (
            "<dataTypeMappings><typeName>width</typeName><typeValue>Account</typeValue></dataTypeMappings>",
            "screens/fields/dataTypeMappings/typeName",
        ),
    ] {
        let rows = flow(&format!(
            "<Flow><screens><fields><name>Column</name><fieldType>Region</fieldType>{member}</fields></screens></Flow>"
        ));
        assert_eq!(reported(&rows), [path], "{member}: {rows:#?}");
    }
    let rows = flow(
        "<Flow><screens><fields><name>Column</name><fieldType>Region</fieldType><inputParameters><name>width</name><value><stringValue>6</stringValue></value></inputParameters></fields></screens></Flow>",
    );
    assert!(reported(&rows).is_empty(), "{rows:#?}");
}
