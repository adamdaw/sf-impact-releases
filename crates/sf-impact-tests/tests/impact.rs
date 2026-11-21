#[path = "../../sf-impact-common/tests/support/pack.rs"]
mod pack;
#[path = "../../sf-impact-common/tests/support/schema.rs"]
mod schema;
mod support;
use serde_json::{Value, json};
use sf_impact_tests::{Config, impact};

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
fn names_are_case_insensitive_and_all_matching_overloads_are_seeds() {
    let refs = [
        declaration("Price.round(integer)", "method"),
        declaration("Price.round(decimal)", "method"),
    ];
    let out = run(&refs, &[], &["pRiCe.RoUnD"], 1, json!({}));
    assert_eq!(out["seeds"].as_array().unwrap().len(), 2);
    assert_eq!(out["queries"][0]["matches"].as_array().unwrap().len(), 2);
    assert_eq!(out["queries"][0]["ambiguous"], true);
}
#[test]
fn unknown_names_are_error_rows_with_partial_confidence() {
    let out = run(&[], &[], &["Gone"], 2, json!({}));
    assert_eq!(
        out["errors"],
        json!([{"name":"Gone","reason":"unknown_symbol"}])
    );
    assert_eq!(out["confidence"], "partial");
}
#[test]
fn exact_overloads_stay_distinct_and_candidates_reach_each() {
    let mut ambiguous = reference("Checkout", "Price.round", "parsed", "call");
    ambiguous["detail"] = json!("ambiguous_overload");
    ambiguous["candidates"] = json!(["Price.round(integer)", "Price.round(decimal)"]);
    let refs = [
        declaration("Price.round(integer)", "method"),
        declaration("Price.round(decimal)", "method"),
        declaration("Checkout", "class"),
        ambiguous,
        reference("Checkout", "Price.round(decimal)", "parsed", "call"),
    ];
    let out = run(&refs, &[], &["Price.round(integer)"], 1, json!({}));
    assert_eq!(out["seeds"][0]["counts"], json!({"parsed":1,"textual":0}));
    assert_eq!(hits(&out, "parsed")[0]["ambiguous"], true);
    let decimal = run(&refs, &[], &["Price.round(decimal)"], 1, json!({}));
    assert_eq!(decimal["seeds"][0]["counts"]["parsed"], 2);
}
#[test]
fn divergent_collisions_keep_all_declarations_and_paths() {
    let mut a = declaration("Handler", "class");
    a["collision"] = json!(true);
    let mut b = a.clone();
    b["package"] = json!("plugin");
    b["file"] = json!("plugin/Handler.cls");
    let refs = [
        a,
        b,
        test("HandlerTest"),
        reference("HandlerTest", "Handler", "parsed", "type"),
    ];
    let out = run(&refs, &[], &["Handler"], 1, json!({}));
    assert_eq!(out["seeds"].as_array().unwrap().len(), 2);
    assert_eq!(
        out["collisions"][0]["declarations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(out["tests"][0]["reasons"].as_array().unwrap().len(), 2);
    assert_eq!(hits(&out, "parsed")[0]["ambiguous"], true);
}
/// The same declaration in another package: a collision of the two.
fn twice(symbol: &str, kind: &str) -> [Value; 2] {
    let mut a = declaration(symbol, kind);
    a["collision"] = json!(true);
    let mut b = a.clone();
    b["package"] = json!("plugin");
    b["file"] = json!(format!("plugin/{}.cls", symbol.split('.').next().unwrap()));
    [a, b]
}
#[test]
fn collisions_are_the_groups_the_walk_reached_and_the_others_are_counted() {
    // Reached: Handler, the seed, and Caller, which refers to it. Not
    // reached: Elsewhere, which nothing reached refers to, and two
    // declarations of Elsewhere.run() that only it has.
    let refs: Vec<Value> = [
        twice("Handler", "class"),
        twice("Caller", "class"),
        twice("Elsewhere", "class"),
        twice("Elsewhere.run()", "method"),
    ]
    .into_iter()
    .flatten()
    .chain([
        reference("Caller", "Handler", "parsed", "type"),
        reference("Elsewhere", "Unrelated", "parsed", "type"),
    ])
    .collect();

    let out = run(&refs, &[], &["Handler"], 2, json!({}));

    let kept: Vec<_> = out["collisions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|group| {
            let files = group["declarations"].as_array().unwrap().iter();
            let files: Vec<_> = files.map(|d| d["file"].as_str().unwrap()).collect();
            (group["symbol"].as_str().unwrap(), files)
        })
        .collect();
    assert_eq!(
        kept,
        [
            ("Caller", vec!["app/Caller.cls", "plugin/Caller.cls"]),
            ("Handler", vec!["app/Handler.cls", "plugin/Handler.cls"]),
        ]
    );
    assert_eq!(out["diagnostics"]["unreached_collision"], 2);
}
#[test]
fn no_unreached_collision_count_is_written_when_every_group_is_reached() {
    let out = run(&twice("Handler", "class"), &[], &["Handler"], 2, json!({}));

    assert_eq!(out["collisions"].as_array().unwrap().len(), 1);
    assert!(
        out["diagnostics"].get("unreached_collision").is_none(),
        "{out}"
    );
}
#[test]
fn cmdt_flow_lwc_and_permission_referrers_are_grouped_by_type() {
    let refs = [
        declaration("Handler", "class"),
        declaration("Handler.run()", "method"),
        declaration("Plugin.Binding", "metadata_record"),
        declaration("Dispatch", "flow"),
        declaration("panel", "lwc"),
        declaration("Access", "permission_set"),
        reference("Plugin.Binding", "Handler", "parsed", "cmdt"),
        reference("Dispatch", "Handler", "parsed", "flow_action"),
        reference("panel", "Handler.run()", "parsed", "lwc_apex"),
        reference("Access", "Handler", "parsed", "grant"),
    ];
    let out = run(&refs, &[], &["Handler"], 1, json!({}));
    let groups: Vec<_> = out["seeds"][0]["referrers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| g["type"].as_str().unwrap())
        .collect();
    assert_eq!(groups, ["flow", "lwc", "metadata_record", "permission_set"]);
    assert_eq!(out["seeds"][0]["counts"], json!({"parsed":4,"textual":0}));
}
#[test]
fn textual_only_is_visible_and_selects_tests() {
    let refs = [
        declaration("Handler", "class"),
        test("OddTests"),
        reference("OddTests", "Handler", "textual", "text"),
    ];
    let out = run(&refs, &[], &["Handler"], 2, json!({}));
    assert_eq!(out["seeds"][0]["counts"], json!({"parsed":0,"textual":1}));
    assert_eq!(names(&out), ["OddTests"]);
    assert_eq!(out["tests"][0]["reasons"][0], "direct");
    assert_eq!(
        out["references"][out["tests"][0]["path"][0]["reference"].to_string()]["tier"],
        "textual"
    );
}
#[test]
fn depth_two_reaches_callers_and_cycles_terminate() {
    let refs = [
        declaration("Handler", "class"),
        declaration("Service", "class"),
        test("ServiceTest"),
        reference("Service", "Handler", "parsed", "type"),
        reference("Handler", "Service", "parsed", "type"),
        reference("ServiceTest", "Service", "textual", "text"),
    ];
    let one = run(&refs, &[], &["Handler"], 1, json!({}));
    assert_eq!(names(&one), Vec::<&str>::new());
    let two = run(&refs, &[], &["Handler"], 2, json!({}));
    assert_eq!(names(&two), Vec::<&str>::new());
    assert_eq!(two["possible"][0]["symbol"], "ServiceTest");
    assert_eq!(two["possible"][0]["reasons"][0], "textual_path");
    assert_eq!(two["possible"][0]["path"].as_array().unwrap().len(), 2);
    let deep = run(&refs, &[], &["Handler"], 50, json!({}));
    assert_eq!(deep["seeds"][0]["counts"], json!({"parsed":1,"textual":0}));
}
#[test]
fn convention_direct_and_caller_reasons_are_unioned() {
    let refs = [
        declaration("Handler", "class"),
        declaration("Service", "class"),
        test("HandlerTest"),
        reference("HandlerTest", "Handler", "parsed", "type"),
        reference("Service", "Handler", "parsed", "type"),
        reference("HandlerTest", "Service", "parsed", "type"),
    ];
    let out = run(&refs, &[changed("Handler", "class")], &[], 2, json!({}));
    assert_eq!(names(&out), ["HandlerTest"]);
    let reasons: Vec<_> = out["tests"][0]["reasons"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert_eq!(reasons, ["caller", "convention", "direct"]);
    assert_eq!(out["untested"], json!([]));
}
#[test]
fn suffixes_are_configurable_and_only_test_classes_qualify() {
    let refs = [
        declaration("Handler", "class"),
        test("HandlerSpec"),
        declaration("HandlerTest", "class"),
    ];
    let out = run(
        &refs,
        &[changed("Handler", "class")],
        &[],
        2,
        json!({"testSuffixes":["Spec"]}),
    );
    assert_eq!(names(&out), ["HandlerSpec"]);
}
#[test]
fn unpackaged_tests_are_included_by_default_and_can_be_excluded() {
    let mut extra = test("HandlerTest");
    extra["scope"] = json!("unpackagedMetadata");
    let refs = [declaration("Handler", "class"), extra];
    let included = run(&refs, &[changed("Handler", "class")], &[], 2, json!({}));
    assert_eq!(names(&included), ["HandlerTest"]);
    assert_eq!(included["tests"][0]["scope"], "unpackagedMetadata");
    let excluded = run(
        &refs,
        &[changed("Handler", "class")],
        &[],
        2,
        json!({"excludeUnpackagedTests":true}),
    );
    assert_eq!(names(&excluded), Vec::<&str>::new());
    assert_eq!(excluded["untested"][0]["symbol"], "Handler");
}
#[test]
fn a_changed_member_uses_its_class_for_convention_and_a_changed_class_reaches_method_calls() {
    let refs = [
        declaration("Handler", "class"),
        declaration("Handler.run()", "method"),
        test("HandlerTest"),
        test("OddTests"),
        reference("OddTests", "Handler.run()", "parsed", "call"),
    ];
    let method = run(
        &refs,
        &[changed("Handler.run()", "method")],
        &[],
        2,
        json!({}),
    );
    assert_eq!(names(&method), ["HandlerTest", "OddTests"]);
    let class = run(&refs, &[changed("Handler", "class")], &[], 2, json!({}));
    assert_eq!(names(&class), ["HandlerTest", "OddTests"]);
}
#[test]
fn removed_symbols_and_touched_cmdt_types_are_seeds() {
    let mut removed = changed("Gone.run()", "method");
    removed["change"] = json!("removed");
    let touched = json!({"row":"touched","referrer":"Plugin.Binding","side":"base","kind":"cmdt_type",
        "referenced":"Plugin__mdt","referenced_type":"object","resolution":"declaration","package":"app","file":"app/binding.xml","line":4});
    let refs = [
        declaration("Plugin__mdt", "object"),
        test("QueriesTest"),
        reference("QueriesTest", "Plugin__mdt", "parsed", "soql_object"),
    ];
    let out = run(&refs, &[removed, touched], &[], 2, json!({}));
    let seeds: Vec<_> = out["seeds"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["symbol"].as_str().unwrap())
        .collect();
    assert_eq!(seeds, ["Gone.run()", "Plugin__mdt"]);
    assert_eq!(names(&out), ["QueriesTest"]);
}
#[test]
fn unresolved_matching_names_and_reached_gaps_lower_confidence_with_locations() {
    let mut unresolved = reference("OddTests", "Handler", "parsed", "type");
    unresolved["resolution"] = json!("unresolved");
    let refs = [
        declaration("Handler", "class"),
        test("OddTests"),
        unresolved,
        reference("OddTests", "Handler", "textual", "text"),
        json!({"row":"unsupported_element","package":"app","file":"app/OddTests.cls","line":8,"column":1,"element":"future","reason":"not_interpreted"}),
    ];
    let files = json!({"row":"file","file":"app/other.xml","status":"modified","package":"app","confidence":"partial","gaps":["unmodelled_file","parse_error"]});
    let out = run(
        &refs,
        &[changed("Handler", "class"), files],
        &[],
        2,
        json!({}),
    );
    assert_eq!(out["confidence"], "partial");
    assert_eq!(out["unresolved"].as_array().unwrap().len(), 1);
    assert_eq!(out["gaps"].as_array().unwrap().len(), 4);
    assert!(
        out["gaps"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["line"] == 8)
    );
    assert_eq!(out["seeds"][0]["counts"], json!({"parsed":0,"textual":1}));
}
#[test]
fn a_full_run_with_no_test_reports_untested_and_ignores_unrelated_gaps() {
    let refs = [
        declaration("Handler", "class"),
        json!({"row":"unmodelled_file","package":"app","file":"app/unrelated.xml","suffix":"layout"}),
    ];
    let out = run(&refs, &[changed("Handler", "class")], &[], 2, json!({}));
    assert_eq!(out["confidence"], "full");
    assert_eq!(out["untested"][0]["symbol"], "Handler");
}
#[test]
fn input_order_does_not_change_output_and_malformed_inputs_are_refused() {
    let refs = [declaration("Handler", "class"), test("HandlerTest")];
    let mut reversed = refs.clone();
    reversed.reverse();
    let out = run(&refs, &[], &["Handler"], 2, json!({}));
    assert_eq!(out, run(&reversed, &[], &["Handler"], 2, json!({})));
    let refused = |refs: Vec<u8>, changes: Option<Vec<u8>>, names: &[String]| {
        let config = Config::default();
        impact(
            refs.as_slice(),
            changes.as_deref(),
            names,
            2,
            &config,
            false,
        )
        .unwrap_err()
    };
    let cut = pack::truncated(&pack::pack(&support::refs_header(), &refs));
    let refused_refs = refused(cut, None, &["Handler".into()]);
    assert!(refused_refs.starts_with("refs pack: "), "{refused_refs}");
    let no_row = [lines(support::changed_header(), &[]), b"{}\n".to_vec()].concat();
    let no_refs = pack::pack(&support::refs_header(), &[]);
    let refused_changed = refused(no_refs, Some(no_row), &[]);
    assert!(
        refused_changed.starts_with("changed: line 2: "),
        "{refused_changed}"
    );
}

#[test]
fn touched_names_without_declarations_are_never_dropped() {
    let touched = json!({"row":"touched","referrer":"Access","side":"base","kind":"grant","referenced":"Gone",
        "referenced_type":"class","resolution":"unresolved","package":"app","file":"app/access.xml","line":7});
    let mut r = reference("Caller", "Gone", "parsed", "type");
    r["resolution"] = json!("unresolved");
    let out = run(
        &[declaration("Caller", "class"), r],
        &[touched],
        &[],
        2,
        json!({}),
    );
    assert_eq!(out["seeds"][0]["symbol"], "Gone");
    assert_eq!(out["unresolved"].as_array().unwrap().len(), 1);
}
#[test]
fn removed_collision_keeps_its_base_identity() {
    let mut removed = changed("Handler", "class");
    removed["change"] = json!("removed");
    removed["file"] = json!("app/old/Handler.cls");
    removed["collision"] = json!(true);
    let out = run(
        &[declaration("Handler", "class")],
        &[removed],
        &[],
        1,
        json!({}),
    );
    assert_eq!(out["seeds"][0]["file"], "app/old/Handler.cls");
    assert_eq!(out["seeds"].as_array().unwrap().len(), 1);
}
#[test]
fn convention_for_a_removed_method_survives_without_its_owner_at_head() {
    let mut removed = changed("Gone.run()", "method");
    removed["change"] = json!("removed");
    let out = run(&[test("GoneTest")], &[removed], &[], 2, json!({}));
    assert_eq!(names(&out), ["GoneTest"]);
}
#[test]
fn unrelated_unmodelled_files_and_outgoing_unresolved_names_are_counted() {
    let mut unresolved = reference("Service", "Missing.run", "parsed", "call");
    unresolved["resolution"] = json!("unresolved");
    let mut text = reference("Layout", "Handler", "textual", "text");
    text["referrer"] = Value::Null;
    text["file"] = json!("app/X.layout-meta.xml");
    let refs = [
        declaration("Handler", "class"),
        declaration("Service", "class"),
        reference("Service", "Handler", "parsed", "type"),
        unresolved,
        text,
        json!({"row":"unmodelled_file","package":"app","file":"app/X.layout-meta.xml","suffix":"layout"}),
    ];
    let out = run(&refs, &[], &["Handler"], 1, json!({}));
    assert_eq!(out["confidence"], "full");
    assert_eq!(out["unresolved"], json!([]));
    assert_eq!(out["diagnostics"]["unresolved"], 1);
    assert_eq!(out["diagnostics"]["unmodelled_file"], 1);
}
#[test]
fn untested_is_a_class_result_after_unioning_all_its_changed_members() {
    let refs = [
        declaration("Handler", "class"),
        declaration("Handler.one()", "method"),
        declaration("Handler.two()", "method"),
        test("OddTests"),
        reference("OddTests", "Handler.one()", "parsed", "call"),
    ];
    let out = run(
        &refs,
        &[
            changed("Handler.one()", "method"),
            changed("Handler.two()", "method"),
        ],
        &[],
        1,
        json!({}),
    );
    assert_eq!(names(&out), ["OddTests"]);
    assert_eq!(out["untested"], json!([]));
}

#[test]
fn an_exact_method_seed_does_not_turn_its_owners_type_references_into_direct_calls() {
    let refs = [
        declaration("Price", "class"),
        declaration("Price.round(integer)", "method"),
        declaration("Price.round(decimal)", "method"),
        test("IntegerTests"),
        test("DecimalTests"),
        reference("IntegerTests", "Price.round(integer)", "parsed", "call"),
        reference("DecimalTests", "Price.round(decimal)", "parsed", "call"),
        reference("DecimalTests", "Price", "parsed", "type"),
    ];
    let out = run(&refs, &[], &["Price.round(integer)"], 1, json!({}));
    assert_eq!(names(&out), ["IntegerTests"]);
    assert_eq!(out["seeds"][0]["counts"]["parsed"], 1);
}
#[test]
fn an_apex_type_reference_does_not_reach_a_permission_set_with_the_same_name() {
    let refs = [
        declaration("Access", "class"),
        declaration("Access", "permission_set"),
        test("ConsumerTest"),
        reference("ConsumerTest", "Access", "parsed", "type"),
    ];
    let out = run(
        &refs,
        &[changed("Access", "permission_set")],
        &[],
        1,
        json!({}),
    );
    assert_eq!(names(&out), Vec::<&str>::new());
    assert_eq!(out["seeds"][0]["counts"]["parsed"], 0);
}

#[test]
fn repeated_sites_share_a_referrer_path_without_losing_counts_or_tiers() {
    let first = reference("OddTests", "Handler", "parsed", "type");
    let mut second = first.clone();
    second["line"] = json!(8);
    let refs = [
        declaration("Handler", "class"),
        test("OddTests"),
        first,
        second,
        reference("OddTests", "Handler", "textual", "text"),
    ];
    let out = run(&refs, &[], &["Handler"], 1, json!({}));
    assert_eq!(out["seeds"][0]["counts"], json!({"parsed":2,"textual":1}));
    assert_eq!(hits(&out, "parsed").len(), 1);
    assert_eq!(
        hits(&out, "parsed")[0]["references"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(out["tests"][0]["reasons"].as_array().unwrap().len(), 1);
}

#[test]
fn construction_of_a_custom_metadata_record_reaches_its_object_type() {
    let refs = [
        declaration("Plugin__mdt", "object"),
        test("PluginTest"),
        reference("PluginTest", "Plugin__mdt", "parsed", "construct"),
    ];
    let out = run(&refs, &[], &["Plugin__mdt"], 1, json!({}));
    assert_eq!(out["seeds"][0]["counts"]["parsed"], 1);
    assert_eq!(names(&out), ["PluginTest"]);
}

#[test]
fn an_invocable_method_change_reaches_the_flow_that_names_its_class() {
    let mut method = declaration("Handler.run()", "method");
    method["annotations"] = json!(["InvocableMethod"]);
    let refs = [
        declaration("Handler", "class"),
        method,
        declaration("Dispatch", "flow"),
        reference("Dispatch", "Handler", "parsed", "flow_action"),
    ];
    let out = run(
        &refs,
        &[changed("Handler.run()", "method")],
        &[],
        1,
        json!({}),
    );
    assert_eq!(out["seeds"][0]["counts"]["parsed"], 1);
    assert_eq!(out["seeds"][0]["referrers"][0]["type"], "flow");
}

#[test]
fn apex_type_namespace_prefers_a_declared_class_to_an_object_of_the_same_name() {
    let refs = [
        declaration("Account", "class"),
        declaration("Account", "object"),
        test("ConsumerTest"),
        reference("ConsumerTest", "Account", "parsed", "type"),
    ];
    let out = run(&refs, &[changed("Account", "object")], &[], 1, json!({}));
    assert_eq!(out["seeds"][0]["counts"]["parsed"], 0);
}

// A nested @IsTest class is not valid Apex, and RunSpecifiedTests has no name
// for one. Rows that have one all the same select its top-level class; the
// witness, the seed or `convention` keeps the inner name.
#[test]
fn an_inner_test_class_is_selected_as_its_top_level_class_and_its_witness_names_it() {
    let refs = [
        declaration("Price", "class"),
        declaration("Price.round()", "method"),
        test("RoundingChecks"),
        test("RoundingChecks.Halves"),
        declaration("RoundingChecks.Halves.up()", "method"),
        reference(
            "RoundingChecks.Halves.up()",
            "Price.round()",
            "parsed",
            "call",
        ),
    ];
    let out = run(&refs, &[], &["Price.round()"], 1, json!({}));
    assert_eq!(names(&out), ["RoundingChecks"]);
    assert_eq!(out["tests"][0]["reasons"], json!(["direct"]));
    assert_eq!(
        out["tests"][0]["path"][0]["referrer"]["symbol"],
        "RoundingChecks.Halves.up()"
    );
}
#[test]
fn inner_test_classes_and_their_top_level_class_are_one_row_with_unioned_reasons() {
    let refs = [
        declaration("Price", "class"),
        declaration("Price.round()", "method"),
        declaration("Cart", "class"),
        declaration("Cart.total()", "method"),
        reference("Cart.total()", "Price.round()", "parsed", "call"),
        test("RoundingChecks"),
        declaration("RoundingChecks.exact()", "method"),
        reference("RoundingChecks.exact()", "Price.round()", "parsed", "call"),
        test("RoundingChecks.Halves"),
        declaration("RoundingChecks.Halves.up()", "method"),
        reference(
            "RoundingChecks.Halves.up()",
            "Cart.total()",
            "parsed",
            "call",
        ),
        test("RoundingChecks.Thirds"),
        declaration("RoundingChecks.Thirds.down()", "method"),
        reference(
            "RoundingChecks.Thirds.down()",
            "Cart.total()",
            "parsed",
            "call",
        ),
    ];
    let out = run(&refs, &[], &["Price.round()"], 2, json!({}));
    assert_eq!(names(&out), ["RoundingChecks"]);
    assert_eq!(out["tests"][0]["reasons"], json!(["caller", "direct"]));
}
#[test]
fn a_changed_inner_test_class_selects_its_top_level_class_with_the_inner_seed() {
    let refs = [
        test("RoundingChecks"),
        test("RoundingChecks.Halves"),
        declaration("RoundingChecks.Halves.up()", "method"),
    ];
    let out = run(
        &refs,
        &[changed("RoundingChecks.Halves.up()", "method")],
        &[],
        2,
        json!({}),
    );
    assert_eq!(names(&out), ["RoundingChecks"]);
    assert_eq!(out["tests"][0]["reasons"], json!(["changed"]));
    assert_eq!(
        out["tests"][0]["seed"]["symbol"],
        "RoundingChecks.Halves.up()"
    );
}
#[test]
fn a_convention_match_on_an_inner_test_class_names_its_top_level_class_and_the_match() {
    let refs = |matched: &str| {
        [
            declaration("Ledger", "class"),
            declaration("Ledger.Entry", "class"),
            test(matched),
        ]
    };
    let changes = [changed("Ledger.Entry", "class")];
    let out = run(&refs("Ledger.EntryTest"), &changes, &[], 2, json!({}));
    assert_eq!(names(&out), ["Ledger"]);
    assert_eq!(out["tests"][0]["reasons"], json!(["convention"]));
    assert_eq!(out["tests"][0]["seed"]["symbol"], "Ledger.Entry");
    assert_eq!(
        out["tests"][0]["convention"],
        json!([{"symbol":"Ledger.EntryTest","file":"app/Ledger.cls"}])
    );
    // Another test that matches is other evidence for the same row.
    let other = run(&refs("Ledger.EntryTests"), &changes, &[], 2, json!({}));
    assert_eq!(names(&other), ["Ledger"]);
    assert_eq!(
        other["tests"][0]["convention"],
        json!([{"symbol":"Ledger.EntryTests","file":"app/Ledger.cls"}])
    );
}
#[test]
fn the_matched_inner_test_is_kept_when_the_convention_witness_beats_a_reference() {
    let refs = [
        declaration("Ledger", "class"),
        declaration("Ledger.Entry", "class"),
        test("Ledger.EntryTest"),
        declaration("Ledger.EntryTest.posts()", "method"),
        reference("Ledger.EntryTest.posts()", "Ledger.Entry", "parsed", "type"),
    ];
    let out = run(
        &refs,
        &[changed("Ledger.Entry", "class")],
        &[],
        2,
        json!({}),
    );
    assert_eq!(names(&out), ["Ledger"]);
    assert_eq!(out["tests"][0]["reasons"], json!(["convention", "direct"]));
    // The one witness is the shortest, which has no path to name the inner class.
    assert_eq!(out["tests"][0]["path"], json!([]));
    assert_eq!(out["tests"][0]["seed"]["symbol"], "Ledger.Entry");
    assert_eq!(
        out["tests"][0]["convention"],
        json!([{"symbol":"Ledger.EntryTest","file":"app/Ledger.cls"}])
    );
}
#[test]
fn a_convention_match_on_the_selected_class_itself_needs_no_other_evidence() {
    let refs = [declaration("Handler", "class"), test("HandlerTest")];
    let out = run(&refs, &[changed("Handler", "class")], &[], 2, json!({}));
    assert_eq!(names(&out), ["HandlerTest"]);
    assert_eq!(out["tests"][0]["reasons"], json!(["convention"]));
    assert_eq!(out["tests"][0].get("convention"), None);
}
#[test]
fn possible_evidence_in_an_inner_test_class_belongs_to_its_top_level_class() {
    let mut refs = vec![
        declaration("Price", "class"),
        declaration("Price.round()", "method"),
        test("RoundingChecks"),
        test("RoundingChecks.Halves"),
        reference("RoundingChecks.Halves", "round", "textual", "text"),
    ];
    let out = run(&refs, &[], &["Price.round()"], 1, json!({}));
    assert_eq!(names(&out), Vec::<&str>::new());
    assert_eq!(out["possible"][0]["symbol"], "RoundingChecks");
    assert_eq!(out["possible"][0]["reasons"], json!(["member_name_only"]));
    assert_eq!(
        out["possible"][0]["path"][0]["referrer"]["symbol"],
        "RoundingChecks.Halves"
    );

    // Selected by its own call, the class is not also possible for its inner one.
    refs.push(declaration("RoundingChecks.exact()", "method"));
    refs.push(reference(
        "RoundingChecks.exact()",
        "Price.round()",
        "parsed",
        "call",
    ));
    let out = run(&refs, &[], &["Price.round()"], 1, json!({}));
    assert_eq!(names(&out), ["RoundingChecks"]);
    assert_eq!(out["possible"], json!([]));
}

#[test]
fn impact_gives_the_same_document_each_time_it_is_called_in_one_process() {
    let refs = pack::of_jsonl(include_str!("../../../fixtures/refs.jsonl"));
    let symbols = [sf_impact_common::types::normal_symbol(
        "FlowInvoiceAction.run(List<Id>)",
    )];
    let call = || {
        impact(
            refs.as_slice(),
            None,
            &symbols,
            2,
            &Config::default(),
            false,
        )
        .unwrap()
    };

    let documents = [call(), call(), call()];

    assert!(!documents[0]["seeds"].as_array().unwrap().is_empty());
    assert_eq!(documents[0], documents[1]);
    assert_eq!(documents[1], documents[2]);
}
