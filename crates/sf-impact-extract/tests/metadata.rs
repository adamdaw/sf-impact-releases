//! Black-box regressions for Flow and LWC extraction, including diagnostics.
#[path = "metadata/catalog.rs"]
mod catalog;
#[path = "../../sf-impact-common/tests/support/schema.rs"]
mod schema;
use serde_json::Value;
use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);
fn extract(files: &[(&str, &str)]) -> Vec<Value> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/metadata-tests")
        .join(format!(
            "{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("sfdx-project.json"),
        r#"{"packageDirectories":[{"path":"pkg"},{"path":"other"}]}"#,
    )
    .unwrap();
    fs::create_dir_all(root.join("other")).unwrap();
    fs::write(root.join("sf-impact.json"), r#"{"namespaces":["ns"]}"#).unwrap();
    for (path, source) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    let run = |jobs| {
        Command::new(env!("CARGO_BIN_EXE_sf-impact-extract"))
            .arg("--root")
            .arg(&root)
            .args(["--format", "jsonl"])
            .args(["--jobs", jobs])
            .output()
            .unwrap()
    };
    let output = run("1");
    assert!(output.status.success(), "{output:?}");
    assert_eq!(output.stdout, run("4").stdout, "parallel output differs");
    fs::remove_dir_all(root).unwrap();
    let rows: Vec<Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let schema = schema::Schema::parse(include_str!("../../../schema/refs.schema.json"));
    for row in &rows {
        assert!(schema.errors(row).is_empty(), "invalid row: {row}");
    }
    rows
}
fn refs(rows: &[Value]) -> Vec<&Value> {
    rows.iter().filter(|r| r["tier"] == "parsed").collect()
}
fn has(rows: &[Value], kind: &str, name: &str) {
    assert!(
        refs(rows)
            .iter()
            .any(|r| r["kind"] == kind && r["referenced"] == name),
        "missing {kind} {name}: {rows:#?}"
    );
}
fn flow(source: &str) -> Vec<Value> {
    extract(&[("pkg/flows/Example.flow-meta.xml", source)])
}
fn js(source: &str) -> Vec<Value> {
    extract(&[("pkg/lwc/example/example.js", source)])
}

#[test]
fn flow_identity_ignores_status_and_declares_even_malformed_files() {
    let rows = extract(&[
        (
            "pkg/flows/Example.flow-meta.xml",
            "<Flow><status>Obsolete</status></Flow>",
        ),
        (
            "pkg/flows/Broken.flow-meta.xml",
            "<Flow><object>Account</Flow>",
        ),
    ]);
    for name in ["Example", "Broken"] {
        assert_eq!(
            rows.iter()
                .filter(|r| r["row"] == "declaration" && r["type"] == "flow" && r["symbol"] == name)
                .count(),
            1
        );
    }
    assert!(
        rows.iter()
            .any(|r| r["row"] == "parse_error" && r["file"] == "pkg/flows/Broken.flow-meta.xml")
    );
}

#[test]
fn every_record_operation_includes_filters_assignments_and_queried_fields() {
    for operation in [
        "recordLookups",
        "recordCreates",
        "recordUpdates",
        "recordDeletes",
    ] {
        let rows = flow(&format!(
            "<Flow><{operation}><object>Invoice__c</object><filters><field>Paid__c</field></filters><inputAssignments><field>Total__c</field></inputAssignments><outputAssignments><field>OwnerId</field></outputAssignments><queriedFields>Id</queriedFields></{operation}></Flow>"
        ));
        has(&rows, "flow_object", "Invoice__c");
        for field in ["Paid__c", "Total__c", "OwnerId", "Id"] {
            has(&rows, "flow_field", &format!("Invoice__c.{field}"));
        }
    }
}

#[test]
fn trigger_object_fields_and_record_expressions_are_bound_including_prior() {
    let rows = flow(
        r#"<Flow><start><object>Invoice__c</object><filters><field>Paid__c</field></filters><triggerFields>Total__c</triggerFields></start>
<formulas><expression>AND({!$Record.Total__c} &gt; 0, $Record__Prior.Paid__c)</expression></formulas>
<assignments><assignmentItems><assignToReference>$Record.OwnerId</assignToReference><value><elementReference>$Record.Account__r.Name</elementReference></value></assignmentItems></assignments></Flow>"#,
    );
    for field in ["Paid__c", "Total__c", "OwnerId", "Account__r.Name"] {
        has(&rows, "flow_field", &format!("Invoice__c.{field}"));
    }
    assert!(refs(&rows).iter().all(|r| r["referrer"] == "Example"));
}

#[test]
fn record_variables_and_lookup_outputs_supply_operation_objects() {
    let rows = flow(
        "<Flow><variables><name>invoices</name><objectType>Invoice__c</objectType></variables><recordUpdates><inputReference>invoices</inputReference><inputAssignments><field>Total__c</field></inputAssignments></recordUpdates><recordLookups><name>getAccount</name><object>Account</object><outputReference>account</outputReference></recordLookups><recordDeletes><inputReference>account</inputReference></recordDeletes><assignments><assignmentItems><assignToReference>getAccount.Name</assignToReference></assignmentItems></assignments></Flow>",
    );
    has(&rows, "flow_object", "Invoice__c");
    has(&rows, "flow_field", "Invoice__c.Total__c");
    has(&rows, "flow_field", "Account.Name");
    assert_eq!(
        refs(&rows)
            .iter()
            .filter(|r| r["referenced"] == "Account")
            .count(),
        2
    );
}

#[test]
fn missing_record_context_is_an_explicit_unresolved_field() {
    let rows = flow(
        "<Flow><recordUpdates><inputReference>unknown</inputReference><inputAssignments><field>Total__c</field></inputAssignments></recordUpdates><decisions><rules><conditions><leftValueReference>$Record.Paid__c</leftValueReference></conditions></rules></decisions></Flow>",
    );
    for name in ["Total__c", "$Record.Paid__c"] {
        let reference = refs(&rows)
            .into_iter()
            .find(|r| r["referenced"] == name)
            .expect("unresolved field is retained");
        assert_eq!(reference["detail"], "unknown_type");
        assert_eq!(reference["resolution"], "unresolved");
    }
}

#[test]
fn apex_actions_plugins_subflows_and_screen_components_resolve_by_type() {
    let rows = extract(&[
        (
            "pkg/flows/Example.flow-meta.xml",
            "<Flow><actionCalls><actionType>apex</actionType><actionName>service</actionName></actionCalls><apexPluginCalls><apexClass>Service</apexClass></apexPluginCalls><subflows><flowName>Child</flowName></subflows><actionCalls><actionType>flow</actionType><actionName>Child</actionName></actionCalls><screens><fields><extensionName>c:childPanel</extensionName></fields></screens></Flow>",
        ),
        (
            "pkg/classes/Service.cls",
            "class Service { @InvocableMethod public static void run() {} }",
        ),
        ("pkg/flows/Child.flow-meta.xml", "<Flow/>"),
        (
            "pkg/lwc/childPanel/childPanel.html",
            "<template></template>",
        ),
    ]);
    for (kind, name) in [
        ("flow_action", "Service"),
        ("flow_subflow", "Child"),
        ("flow_component", "childPanel"),
    ] {
        has(&rows, kind, name);
        assert!(
            refs(&rows)
                .iter()
                .filter(|r| r["kind"] == kind)
                .all(|r| r["resolution"] == "declaration")
        );
    }
}

#[test]
fn structured_types_choices_and_labels_are_references() {
    let rows = flow(
        "<Flow><variables><name>invoice</name><apexClass>Payload</apexClass><objectType>Invoice__c</objectType></variables><dynamicChoiceSets><object>Account</object><displayField>Name</displayField><valueField>Id</valueField><picklistObject>Contact</picklistObject><picklistField>LeadSource</picklistField></dynamicChoiceSets><screens><fields><objectFieldReference>invoice.Total__c</objectFieldReference><fieldText>{!$Label.Greeting}</fieldText></fields></screens></Flow>",
    );
    for (kind, name) in [
        ("flow_action", "Payload"),
        ("flow_object", "Invoice__c"),
        ("flow_field", "Account.Name"),
        ("flow_field", "Account.Id"),
        ("flow_object", "Contact"),
        ("flow_field", "Contact.LeadSource"),
        ("flow_field", "Invoice__c.Total__c"),
        ("flow_label", "Label.Greeting"),
    ] {
        has(&rows, kind, name);
    }
}

#[test]
fn xml_entities_namespaces_cdata_and_positions_are_preserved() {
    let rows = flow(
        "<?xml version=\"1.0\"?>\n<m:Flow xmlns:m=\"http://soap.sforce.com/2006/04/metadata\">\n  <m:start><m:object>Invoice__c</m:object></m:start>\n<m:formulas><m:expression><![CDATA[$Record.Total__c > 0]]></m:expression></m:formulas></m:Flow>",
    );
    has(&rows, "flow_field", "Invoice__c.Total__c");
    let object = refs(&rows)
        .into_iter()
        .find(|r| r["referenced"] == "Invoice__c")
        .unwrap();
    assert_eq!(object["line"], 3);
    assert_eq!(object["column"], 22);
}

#[test]
fn malformed_xml_is_diagnosed_without_losing_textual_matches() {
    for source in [
        "<Flow><object>",
        "<Flow><x></Flow>",
        "<Flow/ ><",
        "<Flow>&bogus;</Flow>",
        "<Flow/><Flow/>",
        "garbage",
        "<Flow a='x' a='y'/>",
    ] {
        let rows = extract(&[
            ("pkg/flows/Example.flow-meta.xml", source),
            ("pkg/classes/Example.cls", "class Example {}"),
        ]);
        assert!(
            rows.iter().any(|r| r["row"] == "parse_error"),
            "{source}: {rows:?}"
        );
    }
    let rows = extract(&[
        ("pkg/flows/Example.flow-meta.xml", "<Flow>Service<broken>"),
        ("pkg/classes/Service.cls", "class Service {}"),
    ]);
    assert!(rows.iter().any(|r| r["tier"] == "textual"
        && r["file"] == "pkg/flows/Example.flow-meta.xml"
        && r["referenced"] == "Service"));
}

#[test]
fn one_bundle_declaration_covers_every_member_including_nested_files() {
    let rows = extract(&[
        ("pkg/lwc/example/example.js", "import 'c/childPanel';"),
        (
            "pkg/lwc/example/example.html",
            "<template><c-child-panel></c-child-panel></template>",
        ),
        ("pkg/lwc/example/utils/helper.js", "import 'c/childPanel';"),
        ("pkg/lwc/childPanel/childPanel.html", "<template/>"),
    ]);
    let declarations: Vec<_> = rows
        .iter()
        .filter(|r| r["row"] == "declaration" && r["symbol"] == "example")
        .collect();
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0]["type"], "lwc");
    assert_eq!(declarations[0]["file"], "pkg/lwc/example/example.js");
    assert_eq!(refs(&rows).len(), 3);
    assert!(
        refs(&rows)
            .iter()
            .all(|r| r["referrer"] == "example" && r["resolution"] == "declaration")
    );
}

#[test]
fn lwc_imports_every_salesforce_module_kind_and_namespaces() {
    let rows = js(
        "import field from '@salesforce/schema/Invoice__c.Total__c';\nimport obj from '@salesforce/schema/Invoice__c';\nimport label from '@salesforce/label/c.Greeting';\nimport allowed from '@salesforce/customPermission/Can_Edit';\nimport channel from '@salesforce/messageChannel/Updated__c';\nimport ext from '@salesforce/schema/ns__Thing__c.ns__Field__c';\nimport label2 from '@salesforce/label/ns.Greeting';\nimport permission2 from '@salesforce/customPermission/ns__Can_Edit';",
    );
    for (kind, name) in [
        ("lwc_schema", "Invoice__c.Total__c"),
        ("lwc_schema", "Invoice__c"),
        ("lwc_label", "Label.Greeting"),
        ("lwc_custom_permission", "CustomPermission.Can_Edit"),
        ("lwc_message_channel", "MessageChannel.Updated__c"),
    ] {
        has(&rows, kind, name);
    }
    for r in refs(&rows) {
        if r["referenced"].as_str().unwrap().contains("ns") {
            assert_eq!(r["resolution"], "external", "{r}");
        }
    }
}

#[test]
fn apex_imports_bind_to_methods_without_guessing_overloads() {
    let rows = extract(&[
        (
            "pkg/lwc/example/example.js",
            "import one from '@salesforce/apex/Service.one'; import many from '@salesforce/apex/Service.many'; import missing from '@salesforce/apex/Service.missing';",
        ),
        (
            "pkg/classes/Service.cls",
            "class Service { public static void one(String a) {} public static void many() {} public static void many(Integer a) {} }",
        ),
    ]);
    has(&rows, "lwc_apex", "Service.one(string)");
    let ambiguous = refs(&rows)
        .into_iter()
        .find(|r| r["referenced"] == "Service.many")
        .unwrap();
    assert_eq!(ambiguous["detail"], "ambiguous_overload");
    assert_eq!(
        ambiguous["candidates"],
        serde_json::json!(["Service.many()", "Service.many(integer)"])
    );
    let missing = refs(&rows)
        .into_iter()
        .find(|r| r["referenced"] == "Service.missing")
        .unwrap();
    assert_eq!(missing["detail"], "no_member");
}

#[test]
fn divergent_apex_imports_keep_inherited_candidates_and_identical_duplicates_collapse() {
    let rows = extract(&[
        (
            "pkg/lwc/example/example.js",
            "import run from '@salesforce/apex/Service.run';",
        ),
        (
            "pkg/classes/Service.cls",
            "class Service extends Parent { public static void run(String v) {} }",
        ),
        ("other/Service.cls", "class Service extends Parent {}"),
        (
            "pkg/classes/Parent.cls",
            "class Parent { public static void run(String v) {} }",
        ),
    ]);
    let reference = refs(&rows)
        .into_iter()
        .find(|r| r["kind"] == "lwc_apex")
        .unwrap();
    assert_eq!(reference["detail"], "ambiguous_receiver");
    assert_eq!(
        reference["candidates"],
        serde_json::json!(["Parent.run(string)", "Service.run(string)"])
    );
    let rows = extract(&[
        (
            "pkg/lwc/example/example.js",
            "import run from '@salesforce/apex/Service.run';",
        ),
        ("pkg/Service.cls", "class Service { static void run() {} }"),
        (
            "other/Service.cls",
            "class Service { static void run() {} }",
        ),
    ]);
    has(&rows, "lwc_apex", "Service.run()");
    assert_eq!(
        refs(&rows)
            .iter()
            .find(|r| r["kind"] == "lwc_apex")
            .unwrap()["detail"],
        Value::Null
    );
}

#[test]
fn scanner_handles_multiline_comments_strings_regex_templates_and_reexports() {
    let rows = js(r#"// import bad from '@salesforce/apex/Fake.comment';
const text = "import bad from '@salesforce/apex/Fake.string'";
const re = /import fake from 'c\/fakePanel'/g;
const template = `import fake from 'c/fakePanel' ${"nested"}`;
/* block */ import /* split */ { default as fetch,
 named } from /* split */ '@salesforce/apex/Service.fetch';
import 'c/sideEffect';
export { default as child } from 'c/childPanel';
export * from 'c/allChildren';
import thing from '@salesforce/schema/Account.\u004eame';
const dynamic = import('c/lazyPanel');
const expression = `result ${import('c/templatePanel')}`;
const object = { import: 'c/notAnImport' }; object.import('c/notAnImport');
"#);
    for name in [
        "Service.fetch",
        "sideEffect",
        "childPanel",
        "allChildren",
        "Account.Name",
        "lazyPanel",
        "templatePanel",
    ] {
        assert!(
            refs(&rows).iter().any(|r| r["referenced"] == name),
            "{name}: {rows:#?}"
        );
    }
    assert_eq!(refs(&rows).len(), 7);
    assert!(!rows.iter().any(|r| r["row"] == "parse_error"));
}

#[test]
fn scanner_reports_unparseable_imports_and_lexical_errors_and_keeps_recovered_imports() {
    for source in [
        "import x from",
        "import x from 'c/broken",
        "/* never closes",
        "const x = `never closes",
        "import('c/' + name)",
        "import(name)",
        "import x 'c/missingFrom';",
        "import '@salesforce/apex/Bad';",
        "const x = /unterminated",
        "const x = '\\xZZ';",
    ] {
        let rows = js(source);
        assert!(
            rows.iter().any(|r| r["row"] == "parse_error"),
            "{source}: {rows:?}"
        );
    }
    let rows = js("import 'c/childPanel';\nimport broken from");
    has(&rows, "lwc_component", "childPanel");
    assert!(
        rows.iter()
            .any(|r| r["row"] == "parse_error" && r["line"] == 2)
    );
}

#[test]
fn templates_ignore_comments_attributes_and_closing_tags_but_support_lwc_syntax() {
    let rows = extract(&[(
        "pkg/lwc/example/example.html",
        r#"<template><input disabled><c-child-panel if:true={show} data-note="<c-fake-panel>"></c-child-panel><!-- <c-fake-panel> --><c-other-panel/><c-child_panel></c-child_panel></template>"#,
    )]);
    for name in ["childPanel", "otherPanel", "child_panel"] {
        has(&rows, "lwc_component", name);
    }
    assert_eq!(refs(&rows).len(), 3);
    assert!(!rows.iter().any(|r| r["row"] == "parse_error"));
    let rows = extract(&[("pkg/lwc/example/example.html", "<template><c-child-panel")]);
    assert!(rows.iter().any(|r| r["row"] == "parse_error"));
}

#[test]
fn meta_xml_targets_objects_and_apex_datasources_are_references() {
    let rows = extract(&[(
        "pkg/lwc/example/example.js-meta.xml",
        "<LightningComponentBundle><targets><target>lightning__RecordPage</target><target>lightning__FlowScreen</target></targets><targetConfigs><targetConfig targets=\"lightning__RecordPage\"><objects><object>Invoice__c</object></objects><property name=\"value\" datasource=\"apex://Options\"/></targetConfig></targetConfigs></LightningComponentBundle>",
    )]);
    has(&rows, "lwc_target", "lightning__RecordPage");
    has(&rows, "lwc_target", "lightning__FlowScreen");
    has(&rows, "lwc_object", "Invoice__c");
    has(&rows, "lwc_apex", "Options");
    assert_eq!(rows.iter().filter(|r| r["row"] == "declaration").count(), 1);
}

#[test]
fn collisions_keep_every_bundle_and_flow_without_cross_type_binding() {
    let rows = extract(&[
        (
            "pkg/flows/SameName.flow-meta.xml",
            "<Flow><subflows><flowName>SameName</flowName></subflows><screens><fields><extensionName>c:SameName</extensionName></fields></screens></Flow>",
        ),
        ("other/flows/SameName.flow-meta.xml", "<Flow/>"),
        ("pkg/lwc/SameName/SameName.js", "import 'c/SameName';"),
        ("other/lwc/SameName/SameName.html", "<template/>"),
        ("pkg/SameName.cls", "class SameName {}"),
    ]);
    let declarations: Vec<_> = rows.iter().filter(|r| r["row"] == "declaration").collect();
    assert_eq!(declarations.len(), 5);
    for r in declarations {
        assert_eq!(r["collision"], r["type"] != "class");
    }
    assert!(refs(&rows).iter().all(|r| r["resolution"] == "declaration"));
    let rows = extract(&[
        ("pkg/Example.cls", "class Example {}"),
        (
            "pkg/flows/Caller.flow-meta.xml",
            "<Flow><subflows><flowName>Example</flowName></subflows></Flow>",
        ),
    ]);
    assert_eq!(refs(&rows)[0]["resolution"], "unresolved");
}

#[test]
fn regex_after_control_flow_and_postfix_division_do_not_create_imports() {
    let rows = js(r#"if (ready) /import fake from 'c\/fakePanel'/.test(text);
while (ready) /import fake from 'c\/fakePanel'/.exec(text);
if (ready) {} /import fake from 'c\/fakePanel'/.exec(text);
const ratio = count++ / 2;
const other = count-- / 2;
import 'c/realPanel';"#);
    assert_eq!(refs(&rows).len(), 1, "{rows:#?}");
    has(&rows, "lwc_component", "realPanel");
    assert!(!rows.iter().any(|r| r["row"] == "parse_error"), "{rows:#?}");
}

#[test]
fn unicode_in_strings_and_escaped_template_text_cannot_panic() {
    let rows = js("const value = '\\é'; const template = `\\é ${1}`; import 'c/realPanel';");
    has(&rows, "lwc_component", "realPanel");
    assert!(!rows.iter().any(|r| r["row"] == "parse_error"), "{rows:#?}");
}

#[test]
fn comments_and_literal_strings_are_not_record_expressions() {
    let rows = flow(
        "<Flow><start><object>Invoice__c</object></start><constants><value><stringValue>$Record.NotAnExpression__c</stringValue></value></constants><screens><fields><fieldText>{!$Record.Total__c}</fieldText></fields></screens></Flow>",
    );
    has(&rows, "flow_field", "Invoice__c.Total__c");
    assert!(
        !refs(&rows)
            .iter()
            .any(|r| r["referenced"] == "Invoice__c.NotAnExpression__c")
    );
}

#[test]
fn unparseable_templates_and_meta_xml_have_diagnostics() {
    for (name, source) in [
        ("example.html", "<template><c-child-panel></template>"),
        ("example.html", "<template><!-- unfinished"),
        ("example.js-meta.xml", "<LightningComponentBundle><targets>"),
    ] {
        let path = format!("pkg/lwc/example/{name}");
        let rows = extract(&[(&path, source)]);
        assert!(
            rows.iter().any(|r| r["row"] == "parse_error"),
            "{source}: {rows:?}"
        );
    }
}

#[test]
fn flow_data_type_mappings_and_process_builder_fields_are_structured_references() {
    let rows = flow(
        "<Flow><dataTypeMappings><typeName>T__record</typeName><typeValue>Invoice__c</typeValue></dataTypeMappings><processMetadataValues><name>ObjectType</name><value><stringValue>Account</stringValue></value></processMetadataValues><recordUpdates><object>Contact</object><inputAssignments><field>Email</field></inputAssignments></recordUpdates></Flow>",
    );
    has(&rows, "flow_object", "Invoice__c");
    has(&rows, "flow_object", "Account");
    has(&rows, "flow_field", "Contact.Email");
}

#[test]
fn action_name_segments_are_api_references_while_version_segments_are_not() {
    let rows = flow(
        "<Flow><actionCalls><actionType>apex</actionType><actionName>Service</actionName><nameSegment>Service</nameSegment><versionSegment>2</versionSegment></actionCalls><actionCalls><actionType>flow</actionType><nameSegment>Child</nameSegment></actionCalls></Flow>",
    );
    assert_eq!(
        refs(&rows)
            .iter()
            .filter(|r| r["kind"] == "flow_action" && r["referenced"] == "Service")
            .count(),
        2
    );
    has(&rows, "flow_subflow", "Child");
    assert_eq!(refs(&rows).len(), 3);
}

#[test]
fn schema_and_flow_fields_do_not_bind_to_same_named_apex_fields() {
    let rows = extract(&[
        (
            "pkg/Invoice__c.cls",
            "class Invoice__c { public String Total__c; }",
        ),
        (
            "pkg/lwc/example/example.js",
            "import total from '@salesforce/schema/Invoice__c.Total__c';",
        ),
        (
            "pkg/flows/Example.flow-meta.xml",
            "<Flow><start><object>Invoice__c</object><filters><field>Total__c</field></filters></start></Flow>",
        ),
    ]);
    has(&rows, "flow_field", "Invoice__c.Total__c");
    has(&rows, "lwc_schema", "Invoice__c.Total__c");
    for reference in refs(&rows)
        .into_iter()
        .filter(|r| r["referenced"] == "Invoice__c.Total__c")
    {
        assert_eq!(reference["resolution"], "unresolved", "{reference}");
    }
}

#[test]
fn orchestration_and_screen_actions_retain_their_flow_dependencies() {
    let rows = flow(
        "<Flow><orchestratedStages><exitActionType>evaluationFlow</exitActionType><exitActionName>Evaluate</exitActionName><stageSteps><actionType>stepBackground</actionType><actionName>Background</actionName></stageSteps></orchestratedStages><steppedStages><steps><actionType>createWorkItem</actionType><actionName>Interactive</actionName></steps></steppedStages><screens><actions><actionType>flow</actionType><actionName>ScreenAction</actionName></actions></screens><actionCalls><actionType>futureActionType</actionType><actionName>FutureAction</actionName></actionCalls></Flow>",
    );
    for name in ["Evaluate", "Background", "Interactive", "ScreenAction"] {
        has(&rows, "flow_subflow", name);
    }
    has(&rows, "flow_action", "FutureAction");
}

#[test]
fn record_operation_with_unknown_input_object_reports_the_gap() {
    let rows = flow(
        "<Flow><recordDeletes><inputReference>unknownRecord</inputReference></recordDeletes></Flow>",
    );
    has(&rows, "flow_object", "SObject");
    assert_eq!(refs(&rows)[0]["detail"], "unknown_type");
}

#[test]
fn contextual_from_identifier_is_a_valid_default_import_binding() {
    let rows = js("import from from 'c/childPanel';");
    has(&rows, "lwc_component", "childPanel");
    assert!(!rows.iter().any(|r| r["row"] == "parse_error"));
}

#[test]
fn javascript_surrogate_escapes_in_strings_do_not_hide_later_imports() {
    let rows = js(r"const emoji = '\uD83D\uDE80'; import 'c/childPanel';");
    has(&rows, "lwc_component", "childPanel");
    assert!(!rows.iter().any(|r| r["row"] == "parse_error"));
}

#[test]
fn metadata_declaration_has_no_phantom_line_after_final_newline() {
    let rows = flow("<Flow/>\n");
    let declaration = rows.iter().find(|r| r["row"] == "declaration").unwrap();
    assert_eq!(declaration["end"], 1);
}

#[path = "metadata/reference_shapes.rs"]
mod reference_shapes;

#[path = "metadata/loud.rs"]
mod loud;

#[path = "metadata/hostile.rs"]
mod hostile;
