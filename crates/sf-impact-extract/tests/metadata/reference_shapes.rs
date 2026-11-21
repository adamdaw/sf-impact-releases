//! Regression cases: corpus shapes and observable reference/diagnostic contracts.
use super::*;

/// Neither a syntax error nor a structure that was not read.
fn no_diagnostic(rows: &[Value]) {
    let is_diagnostic = |r: &Value| r["row"] == "parse_error" || r["row"] == "unsupported_element";
    assert!(!rows.iter().any(is_diagnostic), "{rows:#?}");
}

/// The structures that were not read, by their path below the root. A
/// structure that is well-formed and unknown is one of these, each with the
/// reason `not_interpreted`, and is not a parse error.
fn not_read(rows: &[Value]) -> Vec<&str> {
    assert!(!rows.iter().any(|r| r["row"] == "parse_error"), "{rows:#?}");
    let unsupported = rows.iter().filter(|r| r["row"] == "unsupported_element");
    unsupported
        .map(|r| {
            assert_eq!(r["reason"], "not_interpreted", "{r}");
            r["element"].as_str().unwrap()
        })
        .collect()
}

#[test]
fn orchestration_assignees_use_start_and_record_variable_bindings() {
    // Handle_User_Deactivation and Handle_User_Deactivation_auto, LightningFlowComponents.
    for (binding, reference, expected) in [
        (
            "<start><object>User</object></start>",
            "$Record.Manager.Username",
            "User.Manager.Username",
        ),
        (
            "<variables><name>curUser</name><objectType>User</objectType></variables>",
            "curUser.Username",
            "User.Username",
        ),
    ] {
        let rows = flow(&format!(
            "<Flow>{binding}<orchestratedStages><stageSteps><assignees><assignee><elementReference>{reference}</elementReference></assignee><assigneeType>User</assigneeType></assignees></stageSteps></orchestratedStages></Flow>"
        ));
        has(&rows, "flow_field", expected);
    }
}

#[test]
fn recommendation_map_items_reference_source_and_target_fields() {
    // Get_Account_Recommendation, ExecuteNBAStrategy in LightningFlowComponents.
    let rows = flow(
        r#"<Flow>
<recordLookups><name>Get_Accounts</name><object>Account</object></recordLookups>
<collectionProcessors>
    <name>Map_Recomendations</name>
    <assignNextValueToReference>currentItem_Map_Recomendations_0</assignNextValueToReference>
    <collectionProcessorType>RecommendationMapCollectionProcessor</collectionProcessorType>
    <collectionReference>Get_Accounts</collectionReference>
    <mapItems><assignToFieldReference>ActionReference</assignToFieldReference><operator>Assign</operator><value><stringValue>TestFloW1</stringValue></value></mapItems>
    <mapItems><assignToFieldReference>Description</assignToFieldReference><operator>Assign</operator><value><elementReference>currentItem_Map_Recomendations_0.Description_Rich__c</elementReference></value></mapItems>
    <mapItems><assignToFieldReference>Name</assignToFieldReference><operator>Assign</operator><value><elementReference>currentItem_Map_Recomendations_0.Name</elementReference></value></mapItems>
    <mapItems><assignToFieldReference>RejectionLabel</assignToFieldReference><operator>Assign</operator><value><stringValue>Reject</stringValue></value></mapItems>
    <mapItems><assignToFieldReference>AcceptanceLabel</assignToFieldReference><operator>Assign</operator><value><elementReference>currentItem_Map_Recomendations_0.Name</elementReference></value></mapItems>
    <mapItems><assignToFieldReference>ExternalId</assignToFieldReference><operator>Assign</operator><value><elementReference>currentItem_Map_Recomendations_0.Id</elementReference></value></mapItems>
    <outputSObjectType>Recommendation</outputSObjectType>
</collectionProcessors>
</Flow>"#,
    );
    has(&rows, "flow_field", "Account.Description_Rich__c");
    has(&rows, "flow_field", "Account.Name");
    has(&rows, "flow_field", "Account.Id");
    for target in [
        "Recommendation.ActionReference",
        "Recommendation.Description",
        "Recommendation.Name",
        "Recommendation.RejectionLabel",
        "Recommendation.AcceptanceLabel",
        "Recommendation.ExternalId",
    ] {
        has(&rows, "flow_field", target);
    }
    assert!(refs(&rows).iter().all(|r| r["detail"].is_null()));
    no_diagnostic(&rows);
}

#[test]
fn orchestration_exit_conditions_reference_record_fields() {
    let rows = flow(
        "<Flow><start><object>Invoice__c</object></start><orchestratedStages><exitConditions><leftValueReference>$Record.Closed__c</leftValueReference><rightValue><booleanValue>true</booleanValue></rightValue></exitConditions></orchestratedStages></Flow>",
    );
    has(&rows, "flow_field", "Invoice__c.Closed__c");
}

#[test]
fn scheduled_path_time_source_is_a_field_on_the_start_object() {
    let rows = flow(
        "<Flow><start><object>Invoice__c</object><scheduledPaths><offsetNumber>3</offsetNumber><offsetUnit>Days</offsetUnit><recordField>Due_Date__c</recordField><timeSource>RecordField</timeSource></scheduledPaths></start></Flow>",
    );
    has(&rows, "flow_field", "Invoice__c.Due_Date__c");
}

#[test]
fn reference_leaves_are_read_even_under_future_containers() {
    for leaf in [
        "elementReference",
        "leftValueReference",
        "assignToReference",
        "inputReference",
        "collectionReference",
        "assignRecordIdToReference",
    ] {
        let rows = flow(&format!(
            "<Flow><start><object>Invoice__c</object></start><future><nested><{leaf}>$Record.Total__c</{leaf}></nested></future></Flow>"
        ));
        has(&rows, "flow_field", "Invoice__c.Total__c");
        no_diagnostic(&rows);
    }
}

#[test]
fn merge_tokens_are_read_in_any_text_leaf() {
    for leaf in [
        "choiceText",
        "helpText",
        "errorMessage",
        "nextOrFinishButtonLabel",
        "formula",
        "futureText",
    ] {
        let rows = flow(&format!(
            "<Flow><start><object>Invoice__c</object></start><future><{leaf}>prefix {{!$Record.Total__c}} / {{!$Label.Greeting}} suffix</{leaf}></future></Flow>"
        ));
        has(&rows, "flow_field", "Invoice__c.Total__c");
        has(&rows, "flow_label", "Label.Greeting");
        no_diagnostic(&rows);
    }
}

#[test]
fn uninterpreted_structure_is_diagnosed_without_guessing_object_meaning() {
    let rows = flow("<Flow><future><object>NotProvenAnObject</object></future></Flow>");
    assert_eq!(not_read(&rows), ["future"]);
    assert!(refs(&rows).is_empty());
}

#[test]
fn partly_interpreted_future_structure_still_reports_its_unknown_leaves() {
    let rows = flow(
        "<Flow><future><object>NotProvenAnObject</object><help>{!$Label.Greeting}</help></future></Flow>",
    );
    has(&rows, "flow_label", "Label.Greeting");
    assert_eq!(not_read(&rows), ["future"]);
}

#[test]
fn screen_object_field_reference_uses_the_variable_object() {
    // Real screen shape: Knowledge_* Flows use curKnowledge.Title, not Knowledge__kav.Title.
    let rows = flow(
        "<Flow><variables><name>curKnowledge</name><objectType>Knowledge__kav</objectType></variables><screens><fields><fieldType>ObjectProvided</fieldType><objectFieldReference>curKnowledge.Title</objectFieldReference></fields></screens></Flow>",
    );
    has(&rows, "flow_field", "Knowledge__kav.Title");
    assert!(
        !refs(&rows)
            .iter()
            .any(|r| r["referenced"] == "curKnowledge.Title")
    );
}

#[test]
fn setup_globals_name_both_the_setting_object_and_field() {
    let rows = flow(
        "<Flow><textTemplates><text>{!$Setup.Billing_Settings__c.Rate__c}</text></textTemplates></Flow>",
    );
    has(&rows, "flow_object", "Billing_Settings__c");
    has(&rows, "flow_field", "Billing_Settings__c.Rate__c");
}

#[test]
fn permission_globals_name_the_custom_permission() {
    let rows = flow(
        "<Flow><formulas><expression>{!$Permission.Can_Approve}</expression></formulas></Flow>",
    );
    has(
        &rows,
        "flow_custom_permission",
        "CustomPermission.Can_Approve",
    );
}

#[test]
fn custom_metadata_globals_separate_type_record_and_field() {
    let rows = flow(
        "<Flow><formulas><expression>{!$CustomMetadata.Tier__mdt.Gold.Limit__c}</expression></formulas></Flow>",
    );
    has(&rows, "flow_object", "Tier__mdt");
    has(&rows, "flow_field", "Tier__mdt.Limit__c");
    has(&rows, "flow_custom_metadata", "Tier__mdt.Gold");
}

#[test]
fn platform_owned_flow_globals_are_omitted() {
    for global in [
        "$Flow.CurrentDateTime",
        "$Api.Partner_Server_URL_260",
        "$Organization.Id",
        "$System.OriginDateTime",
        "$fLoW.CurrentDateTime",
    ] {
        let rows = flow(&format!(
            "<Flow><interviewLabel>Interview {{!{global}}}</interviewLabel><assignments><assignmentItems><value><elementReference>{global}</elementReference></value></assignmentItems></assignments><textTemplates><text>{{!$Future.Value}}</text></textTemplates></Flow>"
        ));
        let references = refs(&rows);
        assert_eq!(references.len(), 1, "{global}: {references:#?}");
        assert_eq!(references[0]["referenced"], "$Future.Value");
        assert_eq!(references[0]["detail"], "unknown_type");
        no_diagnostic(&rows);
    }
}

#[test]
fn user_global_fields_belong_to_user() {
    let rows = flow(
        "<Flow><formulas><expression>{!$User.Region__c}</expression></formulas><assignments><assignmentItems><value><elementReference>$uSeR.Id</elementReference></value></assignmentItems></assignments></Flow>",
    );
    has(&rows, "flow_field", "User.Region__c");
    has(&rows, "flow_field", "User.Id");
    assert_eq!(refs(&rows).len(), 2);
    assert!(refs(&rows).iter().all(|r| r["detail"].is_null()));
    no_diagnostic(&rows);
}

#[test]
fn untyped_dotted_references_remain_explicit_unknown_type_rows() {
    let rows = flow(
        "<Flow><formulas><expression>{!Fetch.record.Custom_Field__c} &amp; {!payload.account.Rating}</expression></formulas></Flow>",
    );
    for name in ["Fetch.record.Custom_Field__c", "payload.account.Rating"] {
        let row = refs(&rows)
            .into_iter()
            .find(|r| r["referenced"] == name)
            .unwrap();
        assert_eq!(row["kind"], "flow_field");
        assert_eq!(row["resolution"], "unresolved");
        assert_eq!(row["detail"], "unknown_type");
    }
}

#[test]
fn variable_and_loop_alias_lookup_is_case_insensitive() {
    let rows = flow(
        "<Flow><variables><name>Accounts</name><objectType>Account</objectType></variables><loops><name>Each</name><collectionReference>ACCOUNTS</collectionReference><assignNextValueToReference>Current</assignNextValueToReference></loops><textTemplates><text>{!current.Rating}</text></textTemplates><recordUpdates><inputReference>accounts</inputReference><inputAssignments><field>Industry</field></inputAssignments></recordUpdates></Flow>",
    );
    has(&rows, "flow_field", "Account.Rating");
    has(&rows, "flow_field", "Account.Industry");
}

#[test]
fn case_colliding_variables_with_different_objects_are_unknown() {
    let rows = flow(
        "<Flow><variables><name>Acct</name><objectType>Account</objectType></variables><variables><name>acct</name><objectType>Contact</objectType></variables><formulas><expression>{!ACCT.Name}</expression></formulas></Flow>",
    );
    let row = refs(&rows)
        .into_iter()
        .find(|r| r["referenced"] == "ACCT.Name")
        .unwrap();
    assert_eq!(row["detail"], "unknown_type");
}

#[test]
fn namespaced_screen_extensions_are_external_only_when_configured() {
    for (name, resolution) in [
        ("ns:managedPanel", "external"),
        ("lbpm:thing", "unresolved"),
    ] {
        let rows = flow(&format!(
            "<Flow><screens><fields><extensionName>{name}</extensionName></fields></screens></Flow>"
        ));
        let row = refs(&rows)
            .into_iter()
            .find(|r| r["referenced"] == name)
            .unwrap();
        assert_eq!(row["resolution"], resolution);
    }
}

#[test]
fn explicitly_platform_screen_extensions_are_omitted() {
    for name in ["flowruntime:address", "forceContent:fileUpload"] {
        let rows = flow(&format!(
            "<Flow><screens><fields><extensionName>{name}</extensionName></fields></screens></Flow>"
        ));
        assert!(refs(&rows).is_empty());
        no_diagnostic(&rows);
    }
}

#[test]
fn namespaced_actions_and_subflows_are_external_only_when_configured() {
    for (ns, resolution) in [("ns", "external"), ("other", "unresolved")] {
        let rows = flow(&format!(
            "<Flow><actionCalls><actionType>apex</actionType><actionName>{ns}__RemoteAction</actionName></actionCalls><subflows><flowName>{ns}__Remote_Flow</flowName></subflows></Flow>"
        ));
        assert_eq!(refs(&rows).len(), 2);
        assert!(refs(&rows).iter().all(|r| r["resolution"] == resolution));
    }
}

#[test]
fn custom_errors_reference_the_start_field_and_merge_tokens() {
    let rows = flow(
        "<Flow><start><object>Invoice__c</object></start><customErrors><customErrorMessages><fieldSelection>Total__c</fieldSelection><errorMessage>Bad {!$Record.Reason__c}</errorMessage></customErrorMessages></customErrors></Flow>",
    );
    has(&rows, "flow_field", "Invoice__c.Total__c");
    has(&rows, "flow_field", "Invoice__c.Reason__c");
}

#[test]
fn transforms_reference_the_target_object_and_output_field() {
    let rows = flow(
        "<Flow><transforms><objectType>Shipment__c</objectType><transformValues><transformValueActions><outputFieldApiName>Carrier__c</outputFieldApiName><value><elementReference>$Record.Carrier__c</elementReference></value></transformValueActions></transformValues></transforms></Flow>",
    );
    has(&rows, "flow_object", "Shipment__c");
    has(&rows, "flow_field", "Shipment__c.Carrier__c");
}

#[test]
fn wait_events_reference_platform_event_objects() {
    let rows = flow(
        "<Flow><waits><waitEvents><eventType>Order_Shipped__e</eventType></waitEvents></waits></Flow>",
    );
    has(&rows, "flow_object", "Order_Shipped__e");
}

#[test]
fn alarm_wait_event_is_not_an_object_reference() {
    let rows = flow(
        "<Flow><waits><waitEvents><eventType>AlarmEvent</eventType></waitEvents></waits></Flow>",
    );
    assert!(refs(&rows).is_empty());
}

#[test]
fn overridden_flow_is_a_flow_dependency() {
    let rows = flow("<Flow><overriddenFlow>ns__Template_Flow</overriddenFlow></Flow>");
    has(&rows, "flow_subflow", "ns__Template_Flow");
    assert_eq!(refs(&rows)[0]["resolution"], "external");
}

#[test]
fn collection_processor_aliases_bind_formula_fields() {
    let rows = flow(
        "<Flow><variables><name>lines</name><objectType>Line__c</objectType></variables><collectionProcessors><collectionReference>lines</collectionReference><assignNextValueToReference>currentItem_Flt</assignNextValueToReference><formula>{!currentItem_Flt.Amount__c} &gt; 0</formula></collectionProcessors></Flow>",
    );
    has(&rows, "flow_field", "Line__c.Amount__c");
}

#[test]
fn configuration_editor_kebab_name_resolves_to_its_bundle() {
    let rows = extract(&[
        (
            "pkg/lwc/quickChoiceFSC/quickChoiceFSC.js-meta.xml",
            "<LightningComponentBundle><targetConfigs><targetConfig configurationEditor='c-quick-choice-cpe'/></targetConfigs></LightningComponentBundle>",
        ),
        ("pkg/lwc/quickChoiceCpe/quickChoiceCpe.js", ""),
    ]);
    has(&rows, "lwc_component", "quickChoiceCpe");
    assert_eq!(refs(&rows)[0]["resolution"], "declaration");
}

#[test]
fn namespaced_template_tags_preserve_namespace_and_camel_case() {
    for (tag, name, resolution) in [
        ("ns-managed-widget", "ns:managedWidget", "external"),
        ("lbpm-widget", "lbpm:widget", "unresolved"),
    ] {
        let rows = extract(&[(
            "pkg/lwc/example/example.html",
            &format!("<template><{tag}></{tag}></template>"),
        )]);
        has(&rows, "lwc_component", name);
        assert_eq!(refs(&rows)[0]["resolution"], resolution);
        no_diagnostic(&rows);
    }
}

#[test]
fn apex_continuation_import_resolves_to_the_method_signature() {
    let rows = extract(&[
        (
            "pkg/classes/ProbeAction.cls",
            "class ProbeAction { @AuraEnabled(continuation=true) static Object startLong() { return null; } }",
        ),
        (
            "pkg/lwc/example/example.js",
            "import start from '@salesforce/apexContinuation/ProbeAction.startLong';",
        ),
    ]);
    has(&rows, "lwc_apex", "ProbeAction.startLong()");
}

#[test]
fn schema_array_property_type_references_the_object() {
    let rows = extract(&[(
        "pkg/lwc/example/example.js-meta.xml",
        "<LightningComponentBundle><targetConfigs><targetConfig><property name='rows' type='@salesforce/schema/Contact[]'/></targetConfig></targetConfigs></LightningComponentBundle>",
    )]);
    has(&rows, "lwc_object", "Contact");
}

#[test]
fn css_imports_reference_shared_component_styles_only() {
    let rows = extract(&[
        (
            "pkg/lwc/example/example.css",
            "/* @import 'c/commentedOut'; */\n@import 'c/sharedStyles';\n.x { content: \"@import 'c/inString';\"; }",
        ),
        ("pkg/lwc/sharedStyles/sharedStyles.css", ".x {}"),
    ]);
    has(&rows, "lwc_component", "sharedStyles");
    assert_eq!(refs(&rows).len(), 1);
    assert_eq!(refs(&rows)[0]["resolution"], "declaration");
    no_diagnostic(&rows);
}

#[test]
fn typescript_component_imports_are_scanned_like_javascript() {
    let rows = extract(&[(
        "pkg/lwc/example/example.ts",
        "import NAME from '@salesforce/schema/Account.Name';\nexport default class Example { value: string; }",
    )]);
    has(&rows, "lwc_schema", "Account.Name");
    assert!(
        rows.iter()
            .any(|r| r["row"] == "declaration" && r["type"] == "lwc")
    );
}

#[test]
fn css_import_allows_comments_before_the_module_string() {
    let rows = extract(&[(
        "pkg/lwc/example/example.css",
        "@import /* shared stylesheet */ 'c/sharedStyles';",
    )]);
    has(&rows, "lwc_component", "sharedStyles");
}

#[test]
fn malformed_css_import_is_diagnosed_without_a_truncated_reference() {
    for source in ["@import 'c/é", "@import 'c/sharedStyles", "/* never closed"] {
        let rows = extract(&[("pkg/lwc/example/example.css", source)]);
        assert!(refs(&rows).is_empty());
        assert!(rows.iter().any(|r| r["row"] == "parse_error"));
    }
}

#[test]
fn screen_layout_style_properties_do_not_report_unknown_syntax() {
    // The same styleProperties shape is written in NebulaLogger's
    // Logger_Recipes_Flow_Screen_Demos and LogEntryHandler_Screen_Tests_for_Flow,
    // and coral-cloud's PersonalizedSchedule (also inside nested screen columns).
    let rows = flow(
        r#"<Flow><screens><name>Screen</name>
<fields><name>Section</name><fieldType>RegionContainer</fieldType>
    <fields><name>Column</name><fieldType>Region</fieldType>
        <fields><name>Body</name><fieldText>&lt;p&gt;Hello&lt;/p&gt;</fieldText><fieldType>DisplayText</fieldType>
            <styleProperties>
                <verticalAlignment><stringValue>top</stringValue></verticalAlignment>
                <width><stringValue>12</stringValue></width>
            </styleProperties>
        </fields>
        <inputParameters><name>width</name><value><stringValue>12</stringValue></value></inputParameters>
        <isRequired>false</isRequired>
    </fields>
    <isRequired>false</isRequired><regionContainerType>SectionWithoutHeader</regionContainerType>
</fields>
</screens></Flow>"#,
    );
    // The column's `width` is the input of a platform layout component.
    no_diagnostic(&rows);
    assert!(refs(&rows).is_empty());
}

#[test]
fn ordinary_control_structures_do_not_report_unknown_syntax() {
    // A known Flow element may carry only a label/control value and no dependency.
    // Parameter and assignment elements are not here: their `name` is a member
    // of what they configure (see `parameter_and_assignment_names_are_not_read`).
    for container in [
        "Flow",
        "start",
        "recordLookups",
        "recordCreates",
        "recordUpdates",
        "recordDeletes",
        "variables",
        "constants",
        "formulas",
        "textTemplates",
        "dynamicChoiceSets",
        "choices",
        "filters",
        "assignments",
        "assignmentItems",
        "value",
        "rightValue",
        "defaultValue",
        "conditions",
        "decisions",
        "rules",
        "actions",
        "actionCalls",
        "apexPluginCalls",
        "subflows",
        "screens",
        "fields",
        "visibilityRule",
        "validationRule",
        "dataTypeMappings",
        "loops",
        "collectionProcessors",
        "sortOptions",
        "processMetadataValues",
        "scheduledPaths",
        "waits",
        "waitEvents",
        "entryConditions",
        "exitCriteria",
        "entryCriteria",
        "steppedStages",
        "steps",
        "orchestratedStages",
        "stageSteps",
        "assignees",
        "assignee",
        "mapItems",
        "exitConditions",
        "customErrors",
        "customErrorMessages",
        "transforms",
        "transformValues",
        "transformValueActions",
        "connector",
        "defaultConnector",
        "nextValueConnector",
        "faultConnector",
        "noMoreValuesConnector",
        "schedule",
        "isDisabled",
        "stages",
        "customProperties",
        "triggers",
        "recordRollbacks",
    ] {
        let body = format!("<{container}><name>Next_Step</name></{container}>");
        let rows = flow(&if container == "Flow" {
            body
        } else {
            format!("<Flow>{body}</Flow>")
        });
        no_diagnostic(&rows);
    }
}

#[test]
fn each_field_leaf_independently_names_its_object_field() {
    for leaf in [
        "field",
        "queriedFields",
        "triggerFields",
        "displayField",
        "valueField",
        "sortField",
        "recordField",
        "outputFieldApiName",
    ] {
        let rows = flow(&format!(
            "<Flow><start><object>Invoice__c</object><{leaf}>Total__c</{leaf}></start></Flow>"
        ));
        has(&rows, "flow_field", "Invoice__c.Total__c");
    }
}

#[test]
fn each_object_leaf_independently_names_an_object() {
    for leaf in [
        "object",
        "objectType",
        "outputSObjectType",
        "picklistObject",
    ] {
        let rows = flow(&format!(
            "<Flow><dynamicChoiceSets><{leaf}>Account</{leaf}></dynamicChoiceSets></Flow>"
        ));
        has(&rows, "flow_object", "Account");
    }
}

#[test]
fn each_bare_formula_leaf_independently_reads_field_references() {
    for leaf in [
        "expression",
        "formulaExpression",
        "filterFormula",
        "formula",
    ] {
        let rows = flow(&format!(
            "<Flow><start><object>Invoice__c</object></start><formulas><{leaf}>$Record.Total__c &gt; 0</{leaf}></formulas></Flow>"
        ));
        has(&rows, "flow_field", "Invoice__c.Total__c");
    }
}

#[test]
fn every_flow_action_type_uses_flow_resolution() {
    for ty in [
        "flow",
        "evaluationFlow",
        "stepBackground",
        "stepInteractive",
        "createWorkItem",
    ] {
        let rows = extract(&[
            (
                "pkg/flows/Caller.flow-meta.xml",
                &format!(
                    "<Flow><actionCalls><actionName>Child</actionName><actionType>{ty}</actionType></actionCalls></Flow>"
                ),
            ),
            ("pkg/flows/Child.flow-meta.xml", "<Flow/>"),
        ]);
        has(&rows, "flow_subflow", "Child");
        assert_eq!(refs(&rows)[0]["resolution"], "declaration");
    }
}

#[test]
fn entry_and_exit_action_names_use_their_own_action_types() {
    for (leaf, ty) in [
        ("entryActionName", "entryActionType"),
        ("exitActionName", "exitActionType"),
    ] {
        let rows = flow(&format!(
            "<Flow><orchestratedStages><{leaf}>Child</{leaf}><{ty}>flow</{ty}></orchestratedStages></Flow>"
        ));
        has(&rows, "flow_subflow", "Child");
    }
}

#[test]
fn every_record_operation_with_unknown_input_reports_unknown_object() {
    for operation in [
        "recordLookups",
        "recordCreates",
        "recordUpdates",
        "recordDeletes",
    ] {
        let rows = flow(&format!(
            "<Flow><{operation}><inputReference>unknown</inputReference></{operation}></Flow>"
        ));
        let row = refs(&rows)
            .into_iter()
            .find(|r| r["referenced"] == "SObject")
            .unwrap();
        assert_eq!(row["detail"], "unknown_type");
    }
}

#[test]
fn plugin_reference_cannot_be_satisfied_by_an_action_call() {
    let rows =
        flow("<Flow><apexPluginCalls><apexClass>Plugin</apexClass></apexPluginCalls></Flow>");
    has(&rows, "flow_action", "Plugin");
}

#[test]
fn each_html_void_element_leaves_the_template_stack_balanced() {
    for tag in [
        "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param",
        "source", "track", "wbr",
    ] {
        let rows = extract(&[(
            "pkg/lwc/example/example.html",
            &format!("<template><{tag}></template>"),
        )]);
        no_diagnostic(&rows);
    }
}

#[test]
fn lightning_template_tags_are_explicitly_platform_components() {
    let rows = extract(&[(
        "pkg/lwc/example/example.html",
        "<template><lightning-card></lightning-card></template>",
    )]);
    assert!(refs(&rows).is_empty());
    no_diagnostic(&rows);
}

#[test]
fn prior_record_binding_is_independent_of_current_record_binding() {
    let rows = flow(
        "<Flow><start><object>Invoice__c</object></start><formulas><expression>$Record__Prior.Previous_Total__c</expression></formulas></Flow>",
    );
    has(&rows, "flow_field", "Invoice__c.Previous_Total__c");
}

#[test]
fn loop_name_is_an_alias_when_no_explicit_output_is_supplied() {
    let rows = flow(
        "<Flow><variables><name>accounts</name><objectType>Account</objectType></variables><loops><name>Each</name><collectionReference>accounts</collectionReference></loops><assignments><assignmentItems><assignToReference>Each.Rating</assignToReference></assignmentItems></assignments></Flow>",
    );
    has(&rows, "flow_field", "Account.Rating");
}

#[test]
fn component_action_resolves_the_local_component_namespace() {
    let rows = extract(&[
        (
            "pkg/flows/Caller.flow-meta.xml",
            "<Flow><actionCalls><actionName>c:childPanel</actionName><actionType>component</actionType></actionCalls></Flow>",
        ),
        ("pkg/lwc/childPanel/childPanel.js", ""),
    ]);
    has(&rows, "flow_component", "childPanel");
    assert_eq!(refs(&rows)[0]["resolution"], "declaration");
}

#[test]
fn targets_attribute_independently_lists_each_exposure() {
    let rows = extract(&[(
        "pkg/lwc/example/example.js-meta.xml",
        "<LightningComponentBundle><targetConfigs><targetConfig targets='lightning__RecordPage, lightning__FlowScreen'/></targetConfigs></LightningComponentBundle>",
    )]);
    has(&rows, "lwc_target", "lightning__RecordPage");
    has(&rows, "lwc_target", "lightning__FlowScreen");
    assert_eq!(refs(&rows).len(), 2);
}

#[test]
fn apex_property_type_is_independent_of_datasource() {
    let rows = extract(&[
        (
            "pkg/lwc/example/example.js-meta.xml",
            "<LightningComponentBundle><targetConfigs><targetConfig><property name='payload' type='apex://Payload'/></targetConfig></targetConfigs></LightningComponentBundle>",
        ),
        ("pkg/classes/Payload.cls", "class Payload {}"),
    ]);
    has(&rows, "lwc_apex", "Payload");
    assert_eq!(refs(&rows)[0]["resolution"], "declaration");
}

#[test]
fn unknown_structure_with_known_value_wrapper_is_still_diagnosed() {
    let rows = flow(
        "<Flow><future><value><stringValue>UninterpretedApiName</stringValue></value></future></Flow>",
    );
    assert_eq!(not_read(&rows), ["future"]);
}

#[test]
fn unknown_ancestor_cannot_supply_a_field_object() {
    let rows = flow(
        "<Flow><future><object>NotProvenAnObject</object><filters><field>Total__c</field></filters></future></Flow>",
    );
    has(&rows, "flow_field", "Total__c");
    assert_eq!(refs(&rows)[0]["detail"], "unknown_type");
    assert_eq!(not_read(&rows), ["future"]);
}

#[test]
fn local_screen_component_position_points_past_the_namespace_prefix() {
    let rows = flow(
        "<Flow><screens><fields><extensionName>c:childPanel</extensionName></fields></screens></Flow>",
    );
    assert_eq!(refs(&rows)[0]["column"], 41);
}

#[test]
fn regex_prefix_keywords_cannot_expose_fake_imports() {
    for prefix in [
        "return",
        "throw",
        "case",
        "typeof",
        "void",
        "delete",
        "yield",
        "await",
        "else",
        "do",
        "instanceof",
        "in",
        "of",
    ] {
        let rows = js(&format!("{prefix} /'/; import 'c/realModule';"));
        has(&rows, "lwc_component", "realModule");
        assert_eq!(refs(&rows).len(), 1);
        no_diagnostic(&rows);
    }
}

#[test]
fn scanner_recovers_regex_after_each_control_condition() {
    // This scanner recovers imports even from incomplete control statements;
    // it checks lexical/import syntax, not the entire JS grammar.
    for keyword in ["if", "while", "for", "with", "switch", "catch"] {
        let rows = js(&format!(
            "{keyword} (condition) /'/; import 'c/realModule';"
        ));
        has(&rows, "lwc_component", "realModule");
        no_diagnostic(&rows);
    }
}

#[test]
fn regex_after_each_statement_block_is_not_a_quoted_string() {
    for keyword in ["else", "try", "finally", "do"] {
        let rows = js(&format!("{keyword} {{}} /'/; import 'c/realModule';"));
        has(&rows, "lwc_component", "realModule");
        no_diagnostic(&rows);
    }
}

#[test]
fn namespace_import_reads_its_module() {
    let rows = js("import * as helpers from 'c/sharedHelpers';");
    has(&rows, "lwc_component", "sharedHelpers");
    no_diagnostic(&rows);
}

#[test]
fn namespace_reexport_reads_its_module() {
    let rows = js("export * as helpers from 'c/sharedHelpers';");
    has(&rows, "lwc_component", "sharedHelpers");
    no_diagnostic(&rows);
}

#[test]
fn namespaced_message_channel_is_external() {
    let rows = js("import channel from '@salesforce/messageChannel/ns__Updated__c';");
    has(
        &rows,
        "lwc_message_channel",
        "MessageChannel.ns__Updated__c",
    );
    assert_eq!(refs(&rows)[0]["resolution"], "external");
}

#[test]
fn decimal_constants_are_not_untyped_field_references() {
    let rows = flow("<Flow><formulas><expression>1.25 + 0.5</expression></formulas></Flow>");
    assert!(refs(&rows).is_empty(), "{rows:#?}");
}

#[test]
fn quoted_formula_text_does_not_become_an_untyped_field() {
    let rows = flow(
        "<Flow><formulas><expression>&quot;example.com&quot; &amp; &quot;{!$Label.Greeting}&quot;</expression></formulas></Flow>",
    );
    has(&rows, "flow_label", "Label.Greeting");
    assert_eq!(refs(&rows).len(), 1, "{rows:#?}");
}
