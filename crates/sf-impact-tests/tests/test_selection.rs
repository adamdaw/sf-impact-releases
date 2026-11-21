#[path = "../../sf-impact-common/tests/support/pack.rs"]
mod pack;
#[path = "../../sf-impact-common/tests/support/schema.rs"]
mod schema;
mod support;
use serde_json::{Value, json};
use sf_impact_tests::impact;

fn declaration(symbol: &str, kind: &str) -> Value {
    let owner = symbol.split('.').next().unwrap();
    json!({"row":"declaration","symbol":symbol,"name":symbol.split('(').next().unwrap().rsplit('.').next().unwrap(),
        "type":kind,"package":"app","file":format!("app/{owner}.cls"),"start":1,"end":100,
        "is_test":false,"annotations":[],"collision":false,"scope":"packageDirectories"})
}
fn test(symbol: &str) -> Value {
    let mut d = declaration(symbol, "class");
    d["is_test"] = json!(true);
    d["annotations"] = json!(["IsTest"]);
    d
}
fn reference(from: &str, to: &str, tier: &str, kind: &str) -> Value {
    let owner = from.split('.').next().unwrap();
    let mut r = json!({"row":"reference","referrer":from,"referenced":to,"tier":tier,"kind":kind,
        "package":"app","file":format!("app/{owner}.cls"),"line":5,"column":3});
    if tier == "parsed" {
        r["resolution"] = json!("declaration");
    }
    if matches!(kind, "cmdt" | "grant") {
        r["referenced_type"] = json!(if to.contains('.') {
            "metadata_record"
        } else {
            "class"
        });
    }
    // What sf-impact-extract writes for a declaration this syntax names.
    if tier == "parsed" && !matches!(kind, "cmdt" | "grant") {
        r["referenced_type"] = json!(match kind {
            "call" | "lwc_apex" => "method",
            "construct" if to.contains('(') => "constructor",
            "soql_object" | "dml_object" | "trigger_object" => "object",
            "soql_field" | "dml_field" | "sobject_field" => "object_field",
            "static_access" => "field",
            _ if to.ends_with("__c") || to.ends_with("__mdt") => "object",
            _ => "class",
        });
    }
    if kind == "cmdt" {
        r["source_field"] = json!("Handler__c");
    }
    r
}
fn changed(symbol: &str, kind: &str) -> Value {
    json!({"row":"symbol","symbol":symbol,"type":kind,"package":"app","file":format!("app/{}.cls",symbol.split('.').next().unwrap()),
        "change":"modified","reason":"whole_file","collision":false})
}
/// The lines of an input: its provenance row, then these rows.
fn lines(first: Value, rows: &[Value]) -> Vec<u8> {
    std::iter::once(&first)
        .chain(rows)
        .map(|r| format!("{r}\n"))
        .collect::<String>()
        .into_bytes()
}
fn run(refs: &[Value], changes: &[Value], names: &[&str], depth: usize, config: Value) -> Value {
    let output = impact(
        pack::pack(&support::refs_header(), refs).as_slice(),
        if names.is_empty() {
            Some(lines(support::changed_header(), changes))
        } else {
            None
        }
        .as_deref(),
        &names.iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        depth,
        &serde_json::from_value(config).unwrap(),
        false,
    )
    .unwrap();
    let schema = schema::Schema::parse(include_str!("../../../schema/impact.schema.json"));
    assert_eq!(schema.errors(&output), Vec::<String>::new());
    output
}
fn names(out: &Value) -> Vec<&str> {
    out["tests"]
        .as_array()
        .expect("test rows")
        .iter()
        .map(|t| t["symbol"].as_str().unwrap())
        .collect()
}
fn hits<'a>(out: &'a Value, tier: &str) -> Vec<&'a Value> {
    out["seeds"][0]["referrers"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|g| g[tier].as_array().unwrap())
        .collect()
}

#[test]
fn walks_through_test_helpers() {
    let refs = [
        declaration("Calc.calc()", "method"),
        test("Driver"),
        declaration("Driver.drive()", "method"),
        test("ViaHelper"),
        reference("Driver.drive()", "Calc.calc()", "parsed", "call"),
        reference("ViaHelper", "Driver.drive()", "parsed", "call"),
    ];
    let out = run(&refs, &[], &["Calc.calc()"], 2, json!({}));
    assert_eq!(names(&out), ["Driver", "ViaHelper"]);
}
#[test]
fn a_changed_test_selects_itself_even_at_depth_zero() {
    let refs = [test("SelfCheck"), declaration("SelfCheck.t()", "method")];
    let out = run(
        &refs,
        &[changed("SelfCheck.t()", "method")],
        &[],
        0,
        json!({}),
    );
    assert_eq!(names(&out), ["SelfCheck"]);
    assert_eq!(out["tests"][0]["reasons"], json!(["changed"]));
}
fn trigger_rows() -> Vec<Value> {
    vec![
        declaration("OrderTrg", "trigger"),
        declaration("Order__c", "object"),
        declaration("Handler.go()", "method"),
        test("DmlCheck"),
        reference("OrderTrg", "Order__c", "parsed", "trigger_object"),
        reference("OrderTrg", "Handler.go()", "parsed", "call"),
        reference("DmlCheck", "Order__c", "parsed", "dml_object"),
    ]
}
#[test]
fn trigger_changes_select_dml_tests() {
    let out = run(
        &trigger_rows(),
        &[changed("OrderTrg", "trigger")],
        &[],
        1,
        json!({}),
    );
    assert_eq!(names(&out), ["DmlCheck"]);
    assert_eq!(out["tests"][0]["reasons"], json!(["trigger_dml"]));
    assert_eq!(out["tests"][0]["path"][0]["via"], "trigger_dml");
}
#[test]
fn handler_reached_through_trigger_selects_dml_tests() {
    let out = run(
        &trigger_rows(),
        &[changed("Handler.go()", "method")],
        &[],
        2,
        json!({}),
    );
    assert_eq!(names(&out), ["DmlCheck"]);
    assert_eq!(out["tests"][0]["path"].as_array().unwrap().len(), 2);
}
#[test]
fn trigger_without_dml_tests_is_untested_and_partial() {
    let mut refs = trigger_rows();
    refs.pop();
    let out = run(&refs, &[changed("OrderTrg", "trigger")], &[], 2, json!({}));
    assert_eq!(out["untested"][0]["symbol"], "OrderTrg");
    assert_eq!(out["confidence"], "partial");
    assert!(
        out["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|g| g["reason"] == "trigger_entry")
    );
}
#[test]
fn dispatch_uses_super_signatures_without_sibling_or_overload_expansion() {
    let refs = [
        declaration("Child", "class"),
        declaration("Child.ship(integer)", "method"),
        declaration("Parent", "interface"),
        declaration("Parent.ship(integer)", "method"),
        declaration("Parent.ship(string)", "method"),
        declaration("Sibling", "class"),
        declaration("Sibling.ship(integer)", "method"),
        test("InterfaceCheck"),
        test("SiblingCheck"),
        test("StringCheck"),
        reference("Child", "Parent", "parsed", "implements"),
        reference("Sibling", "Parent", "parsed", "implements"),
        reference("InterfaceCheck", "Parent.ship(integer)", "parsed", "call"),
        reference("SiblingCheck", "Sibling.ship(integer)", "parsed", "call"),
        reference("StringCheck", "Parent.ship(string)", "parsed", "call"),
    ];
    let out = run(
        &refs,
        &[changed("Child.ship(integer)", "method")],
        &[],
        1,
        json!({}),
    );
    assert_eq!(names(&out), ["InterfaceCheck"]);
    assert_eq!(out["tests"][0]["path"][0]["via"], "dispatch");
    assert!(
        !out["tests"][0]["path"][0]["evidence"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn override_and_abstract_dispatch_continue_through_the_super_caller() {
    for parent in ["Base", "Abstract"] {
        let refs = [
            declaration("Child", "class"),
            declaration("Child.hook()", "method"),
            declaration(parent, "class"),
            declaration(&format!("{parent}.hook()"), "method"),
            declaration(&format!("{parent}.run()"), "method"),
            test("OverrideCheck"),
            reference("Child", parent, "parsed", "extends"),
            reference(
                &format!("{parent}.run()"),
                &format!("{parent}.hook()"),
                "parsed",
                "call",
            ),
            reference(
                "OverrideCheck",
                &format!("{parent}.run()"),
                "parsed",
                "call",
            ),
        ];
        let out = run(
            &refs,
            &[changed("Child.hook()", "method")],
            &[],
            2,
            json!({}),
        );
        assert_eq!(names(&out), ["OverrideCheck"]);
    }
}
#[test]
fn method_changes_report_owner_cmdt_bindings_and_select_parsed_paths() {
    let refs = [
        declaration("Handler", "class"),
        declaration("Handler.call()", "method"),
        declaration("Binding.Order", "metadata_record"),
        test("BindingCheck"),
        reference("Binding.Order", "Handler", "parsed", "cmdt"),
        reference("BindingCheck", "Binding.Order", "parsed", "cmdt"),
    ];
    let out = run(
        &refs,
        &[changed("Handler.call()", "method")],
        &[],
        2,
        json!({}),
    );
    assert_eq!(names(&out), ["BindingCheck"]);
    assert_eq!(out["seeds"][0]["referrers"][0]["type"], "metadata_record");
    assert_eq!(out["tests"][0]["path"][0]["via"], "owner_binding");
}
#[test]
fn textual_owner_bindings_are_visible_but_only_possible_beyond_first_hop() {
    let refs = [
        declaration("Handler", "class"),
        declaration("Handler.call()", "method"),
        declaration("Binding.Order", "metadata_record"),
        test("BindingCheck"),
        reference("Binding.Order", "Handler", "textual", "text"),
        reference("BindingCheck", "Binding.Order", "parsed", "cmdt"),
    ];
    let out = run(
        &refs,
        &[changed("Handler.call()", "method")],
        &[],
        2,
        json!({}),
    );
    assert_eq!(out["seeds"][0]["referrers"][0]["type"], "metadata_record");
    assert!(names(&out).is_empty());
    assert_eq!(out["possible"][0]["symbol"], "BindingCheck");
    assert_eq!(out["confidence"], "partial");
}
#[test]
fn counts_always_describe_direct_referrers() {
    let refs = [
        declaration("Handler", "class"),
        declaration("Service", "class"),
        test("ServiceCheck"),
        reference("Service", "Handler", "textual", "text"),
        reference("ServiceCheck", "Service", "parsed", "type"),
    ];
    let one = run(&refs, &[], &["Handler"], 1, json!({}));
    let two = run(&refs, &[], &["Handler"], 2, json!({}));
    assert_eq!(two["seeds"][0]["counts"], one["seeds"][0]["counts"]);
    assert_eq!(two["seeds"][0]["counts"], json!({"parsed":0,"textual":1}));
    assert_eq!(two["seeds"][0]["referrers"], one["seeds"][0]["referrers"]);
}
#[test]
fn missing_modified_or_added_declarations_are_errors_including_empty_refs() {
    for refs in [vec![], vec![declaration("Different", "class")]] {
        for change in ["modified", "added"] {
            let mut row = changed("Missing.go()", "method");
            row["change"] = json!(change);
            let out = run(&refs, &[row], &[], 2, json!({}));
            assert_eq!(out["confidence"], "partial");
            assert_eq!(out["errors"][0]["reason"], "missing_declaration");
            assert_eq!(out["errors"][0]["file"], "app/Missing.cls");
            assert!(out["seeds"].as_array().unwrap().is_empty());
        }
    }
}
#[test]
fn textual_paths_select_only_the_first_hop_and_other_tests_are_possible() {
    for (first, second) in [
        ("parsed", "textual"),
        ("textual", "parsed"),
        ("textual", "textual"),
    ] {
        let refs = [
            declaration("Handler", "class"),
            declaration("Service", "class"),
            test("DirectCheck"),
            test("DeepCheck"),
            reference("DirectCheck", "Handler", "textual", "text"),
            reference(
                "Service",
                "Handler",
                first,
                if first == "parsed" { "type" } else { "text" },
            ),
            reference(
                "DeepCheck",
                "Service",
                second,
                if second == "parsed" { "type" } else { "text" },
            ),
        ];
        let out = run(&refs, &[], &["Handler"], 2, json!({}));
        assert_eq!(names(&out), ["DirectCheck"]);
        assert_eq!(out["possible"][0]["symbol"], "DeepCheck");
        assert_eq!(out["possible"][0]["reasons"], json!(["textual_path"]));
    }
}
#[test]
fn lwc_textual_paths_never_select_apex_tests() {
    let refs = [
        declaration("logger", "lwc"),
        test("LoggerCheck"),
        reference("LoggerCheck", "logger", "textual", "text"),
    ];
    let out = run(&refs, &[changed("logger", "lwc")], &[], 1, json!({}));
    assert!(names(&out).is_empty());
    assert_eq!(out["possible"][0]["symbol"], "LoggerCheck");
    assert_eq!(out["possible"][0]["reasons"], json!(["lwc_textual"]));
}
#[test]
fn reached_members_do_not_expose_the_containing_class() {
    let refs = [
        declaration("Handler", "class"),
        declaration("Hub", "class"),
        declaration("Hub.go()", "method"),
        test("HubCheck"),
        reference("Hub.go()", "Handler", "parsed", "type"),
        reference("HubCheck", "Hub", "parsed", "type"),
    ];
    let out = run(&refs, &[], &["Handler"], 2, json!({}));
    assert!(names(&out).is_empty());
}
#[test]
fn unrelated_standard_objects_and_searched_metadata_are_diagnostics_not_gaps() {
    let mut unresolved = reference("Check", "User.Id", "parsed", "soql_field");
    unresolved["resolution"] = json!("unresolved");
    let refs = [
        declaration("Handler", "class"),
        test("Check"),
        reference("Check", "Handler", "parsed", "type"),
        unresolved,
        reference("Layout", "Handler", "textual", "text"),
        json!({"row":"unmodelled_file","package":"app","file":"app/Layout.cls","suffix":"layout"}),
    ];
    let out = run(&refs, &[], &["Handler"], 2, json!({}));
    assert_eq!(out["confidence"], "full");
    assert_eq!(out["diagnostics"]["unresolved"], 1);
    assert_eq!(out["diagnostics"]["unmodelled_file"], 1);
}
#[test]
fn relevant_ambiguous_parsed_references_lower_confidence() {
    let mut r = reference("Check", "Handler.go", "parsed", "call");
    r["detail"] = json!("ambiguous_overload");
    r["candidates"] = json!(["Handler.go(integer)", "Handler.go(string)"]);
    let out = run(
        &[
            declaration("Handler.go(integer)", "method"),
            declaration("Handler.go(string)", "method"),
            test("Check"),
            r,
        ],
        &[],
        &["Handler.go(integer)"],
        1,
        json!({}),
    );
    assert_eq!(out["confidence"], "partial");
    assert_eq!(out["gaps"][0]["reason"], "ambiguous");
}
#[test]
fn underscore_plural_is_a_default_test_suffix() {
    let out = run(
        &[declaration("Handler", "class"), test("Handler_Tests")],
        &[],
        &["Handler"],
        0,
        json!({}),
    );
    assert_eq!(names(&out), ["Handler_Tests"]);
}
#[test]
fn output_interns_references_and_keeps_one_shortest_witness_per_test() {
    let refs = [
        declaration("Handler", "class"),
        declaration("Handler.go()", "method"),
        test("Check"),
        reference("Check", "Handler.go()", "parsed", "call"),
        reference("Check", "Handler", "textual", "text"),
    ];
    let out = run(
        &refs,
        &[
            changed("Handler", "class"),
            changed("Handler.go()", "method"),
        ],
        &[],
        2,
        json!({}),
    );
    assert!(out["references"].is_object());
    assert_eq!(out["tests"].as_array().unwrap().len(), 1);
    assert_eq!(out["tests"][0]["path"].as_array().unwrap().len(), 1);
    assert!(hits(&out, "parsed")[0]["references"][0].is_u64());
    assert!(out["possible"].as_array().unwrap().is_empty());
}

#[test]
fn reference_table_schema_validates_every_entry() {
    let schema = schema::Schema::parse(
        r#"{"type":"object","additionalProperties":{"type":"integer","minimum":0}}"#,
    );
    assert!(schema.errors(&json!({"7":3})).is_empty());
    assert!(!schema.errors(&json!({"7":"bad"})).is_empty());
    assert!(!schema.errors(&json!({"7":-1})).is_empty());
}

#[test]
fn direct_sites_are_not_repeated_for_each_member_of_a_class_seed() {
    let out = run(
        &[
            declaration("Handler", "class"),
            declaration("Handler.perform(integer)", "method"),
            declaration("Handler.perform(string)", "method"),
            test("Check"),
            reference("Check", "perform", "textual", "text"),
        ],
        &[],
        &["Handler"],
        2,
        json!({}),
    );
    assert_eq!(hits(&out, "textual").len(), 1);
    assert_eq!(
        hits(&out, "textual")[0]["references"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn weak_routes_do_not_turn_unrelated_names_or_triggers_into_confidence_gaps() {
    let mut unresolved = reference("Other", "Helper.go", "parsed", "call");
    unresolved["resolution"] = json!("unresolved");
    let refs = [
        declaration("logger", "lwc"),
        declaration("Helper", "class"),
        declaration("Helper.go()", "method"),
        declaration("Trg", "trigger"),
        unresolved,
        reference("Helper.go()", "logger", "textual", "text"),
        reference("Trg", "logger", "textual", "text"),
    ];
    let out = run(&refs, &[changed("logger", "lwc")], &[], 2, json!({}));
    assert_eq!(out["confidence"], "full");
    assert_eq!(out["diagnostics"]["unresolved"], 1);
}

#[test]
fn schema_keeps_possible_reasons_out_of_selected_tests_and_vice_versa() {
    let out = run(
        &[
            declaration("Handler", "class"),
            test("Check"),
            reference("Check", "Handler", "parsed", "type"),
        ],
        &[],
        &["Handler"],
        1,
        json!({}),
    );
    let schema = schema::Schema::parse(include_str!("../../../schema/impact.schema.json"));
    for (section, reason) in [("tests", "textual_path"), ("possible", "direct")] {
        let mut invalid = out.clone();
        let mut entry = out["tests"][0].clone();
        entry["reasons"] = json!([reason]);
        invalid[section] = json!([entry]);
        assert!(!schema.errors(&invalid).is_empty(), "{section}: {reason}");
    }
}
