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
fn class_level_flow_action_is_an_uncovered_dynamic_binding() {
    let refs = [
        declaration("FlowAct", "class"),
        declaration("Order_Flow", "flow"),
        reference("Order_Flow", "FlowAct", "parsed", "flow_action"),
    ];
    let out = run(&refs, &[changed("FlowAct", "class")], &[], 2, json!({}));
    assert_eq!(out["confidence"], "partial");
    assert_eq!(out["tests"], json!([]));
    assert_eq!(out["possible"], json!([]));
    assert_eq!(out["untested"][0]["symbol"], "FlowAct");
    assert_eq!(hits(&out, "parsed")[0]["referrer"]["symbol"], "Order_Flow");
    assert_eq!(
        out["gaps"],
        json!([{
            "reason":"dynamic_binding", "file":"app/Order_Flow.cls", "line":5, "source":"refs"
        }])
    );
}

#[test]
fn platform_and_other_uncalled_methods_follow_parsed_owner_entries_to_depth() {
    for signature in [
        "execute(queueablecontext)",
        "start(database.batchablecontext)",
        "execute(database.batchablecontext,list<sobject>)",
        "finish(database.batchablecontext)",
        "execute(schedulablecontext)",
        "callback()",
    ] {
        let method = format!("AsyncJob.{signature}");
        let refs = [
            declaration("AsyncJob", "class"),
            declaration(&method, "method"),
            declaration("AsyncJob.AsyncJob()", "constructor"),
            declaration("AsyncJob.other()", "method"),
            declaration("Starter.kick()", "method"),
            test("DirectCheck"),
            test("ViaStarterCheck"),
            test("OtherMemberCheck"),
            test("TextOwnerCheck"),
            reference("DirectCheck", "AsyncJob", "parsed", "type"),
            reference(
                "Starter.kick()",
                "AsyncJob.AsyncJob()",
                "parsed",
                "construct",
            ),
            reference("ViaStarterCheck", "Starter.kick()", "parsed", "call"),
            reference("OtherMemberCheck", "AsyncJob.other()", "parsed", "call"),
            reference("TextOwnerCheck", "AsyncJob", "textual", "text"),
        ];
        let out = run(&refs, &[changed(&method, "method")], &[], 2, json!({}));
        assert_eq!(names(&out), ["DirectCheck", "ViaStarterCheck"], "{method}");
        assert_eq!(out["possible"], json!([]));
        assert_eq!(out["seeds"][0]["counts"], json!({"parsed":2,"textual":0}));
        for test in out["tests"].as_array().unwrap() {
            assert_eq!(test["reasons"], json!(["platform_entry"]));
            assert_eq!(test["path"][0]["via"], "platform_entry");
        }
        let one = run(&refs, &[], &[&method], 1, json!({}));
        assert_eq!(names(&one), ["DirectCheck"]);
        let zero = run(&refs, &[], &[&method], 0, json!({}));
        assert_eq!(zero["tests"], json!([]));
        assert_eq!(zero["seeds"][0]["counts"], out["seeds"][0]["counts"]);
    }
}

#[test]
fn owner_fallback_requires_no_parsed_referrer_after_dispatch_and_bindings() {
    let mut refs = vec![
        declaration("Worker", "class"),
        declaration("Worker.run()", "method"),
        test("OwnerCheck"),
        reference("OwnerCheck", "Worker", "parsed", "construct"),
    ];
    assert_eq!(
        names(&run(&refs, &[], &["Worker.run()"], 1, json!({}))),
        ["OwnerCheck"]
    );
    for extra in [
        vec![
            test("DirectCheck"),
            reference("DirectCheck", "Worker.run()", "parsed", "call"),
        ],
        vec![
            declaration("Parent", "interface"),
            declaration("Parent.run()", "method"),
            reference("Worker", "Parent", "parsed", "implements"),
            test("DispatchCheck"),
            reference("DispatchCheck", "Parent.run()", "parsed", "call"),
        ],
        vec![
            declaration("Binding.Worker", "metadata_record"),
            reference("Binding.Worker", "Worker", "parsed", "cmdt"),
        ],
    ] {
        let mut bound = refs.clone();
        bound.extend(extra);
        let out = run(&bound, &[], &["Worker.run()"], 2, json!({}));
        assert!(!names(&out).contains(&"OwnerCheck"));
        assert!(hits(&out, "parsed").iter().all(|h| {
            h["routes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|r| r["via"] != "platform_entry")
        }));
    }
    // The exception is for method seeds, never members reached later.
    refs.extend([
        declaration("Seed", "class"),
        reference("Worker.run()", "Seed", "parsed", "type"),
    ]);
    assert!(names(&run(&refs, &[], &["Seed"], 3, json!({}))).is_empty());
}

#[test]
fn npsp_bdi_start_reaches_all_pinned_test_constructors() {
    // NPSP f5072b44781e7f31cb396c6919cfa7d0ead0ec6a, original extractor rows:
    // batch/start/constructor declarations, test owners and one construct per test.
    // This pin has nine distinct test classes.
    let refs: Vec<Value> = include_str!("data/npsp-bdi-start.refs.jsonl")
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let out = run(
        &refs,
        &[],
        &["BDI_DataImport_BATCH.start(database.batchablecontext)"],
        1,
        json!({}),
    );
    assert_eq!(
        names(&out),
        [
            "BDI_AdditionalObjectService_TEST",
            "BDI_DataImport_TEST",
            "BDI_DataImport_TEST2",
            "BDI_Donations_TEST",
            "BDI_Donations_TEST3",
            "BDI_DryRun_TEST",
            "BDI_GAUAllocationsUtil_TEST",
            "BDI_PostProcess_TEST",
            "BDI_RecurringDonations_TEST",
        ]
    );
    assert_eq!(out["confidence"], "full");
    assert_eq!(out["seeds"][0]["counts"], json!({"parsed":9,"textual":0}));
    for t in out["tests"].as_array().unwrap() {
        assert_eq!(t["reasons"], json!(["platform_entry"]));
        let hop = &t["path"][0];
        assert_eq!(hop["via"], "platform_entry");
        assert_eq!(
            out["references"][hop["reference"].to_string()]["kind"],
            "construct"
        );
    }
}

#[test]
fn uncorroborated_member_words_are_possible_for_method_and_class_seeds() {
    for (member, kind) in [
        ("record", "field"),
        ("execute()", "method"),
        ("getInstance()", "method"),
    ] {
        let symbol = format!("OwnerClass.{member}");
        let refs = [
            declaration("OwnerClass", "class"),
            declaration(&symbol, kind),
            test("WordOnly"),
            reference("WordOnly", member.trim_end_matches("()"), "textual", "text"),
        ];
        for seed in [symbol.as_str(), "OwnerClass"] {
            let out = run(&refs, &[], &[seed], 2, json!({}));
            assert_eq!(names(&out), Vec::<&str>::new(), "{seed}");
            assert_eq!(out["possible"][0]["symbol"], "WordOnly");
            assert_eq!(out["possible"][0]["reasons"], json!(["member_name_only"]));
            assert_eq!(out["possible"][0]["path"].as_array().unwrap().len(), 1);
            assert_eq!(out["seeds"][0]["counts"], json!({"parsed":0,"textual":1}));
        }
    }
}

#[test]
fn a_missing_owner_declaration_does_not_turn_member_text_into_type_text() {
    let refs = [
        declaration("MissingOwner.execute()", "method"),
        test("WordOnly"),
        reference("WordOnly", "execute", "textual", "text"),
    ];
    let out = run(&refs, &[], &["MissingOwner.execute()"], 1, json!({}));
    assert_eq!(out["tests"], json!([]));
    assert_eq!(out["possible"][0]["reasons"], json!(["member_name_only"]));
}

#[test]
fn member_text_needs_same_file_owner_evidence_and_type_names_remain_unchanged() {
    let refs = [
        declaration("OwnerClass", "class"),
        declaration("OwnerClass.execute()", "method"),
        test("BothNames"),
        test("WrongWord"),
        test("ParsedCheck"),
        test("ClassNameOnly"),
        reference("BothNames", "execute", "textual", "text"),
        reference("BothNames", "oWnErClAsS", "textual", "text"),
        reference("WrongWord", "execute", "textual", "text"),
        reference("WrongWord", "OwnerClassSuffix", "textual", "text"),
        reference("ParsedCheck", "OwnerClass.execute()", "parsed", "call"),
        reference("ClassNameOnly", "OwnerClass", "textual", "text"),
    ];
    let out = run(&refs, &[], &["OwnerClass.execute()"], 2, json!({}));
    assert_eq!(names(&out), ["BothNames", "ParsedCheck"]);
    assert_eq!(out["possible"][0]["symbol"], "WrongWord");
    assert_eq!(out["possible"][0]["reasons"], json!(["member_name_only"]));
    let class = run(&refs, &[], &["OwnerClass"], 2, json!({}));
    assert_eq!(names(&class), ["BothNames", "ClassNameOnly", "ParsedCheck"]);
}

#[test]
fn nested_members_accept_any_enclosing_type_but_not_a_different_file() {
    let refs = [
        declaration("OuterType", "class"),
        declaration("OuterType.InnerType", "class"),
        declaration("OuterType.InnerType.record", "field"),
        test("OuterCheck"),
        test("InnerCheck"),
        test("OnlyWord"),
        reference("OuterCheck", "record", "textual", "text"),
        reference("OuterCheck", "OuterType", "textual", "text"),
        reference("InnerCheck", "record", "textual", "text"),
        reference("InnerCheck", "InnerType", "textual", "text"),
        reference("OnlyWord", "record", "textual", "text"),
    ];
    let out = run(&refs, &[], &["OuterType.InnerType.record"], 1, json!({}));
    assert_eq!(names(&out), ["InnerCheck", "OuterCheck"]);
    assert_eq!(out["possible"][0]["symbol"], "OnlyWord");
}

#[test]
fn dispatched_member_text_accepts_seed_and_dispatch_types_or_parsed_owner_references() {
    let refs = [
        declaration("ChildType", "class"),
        declaration("ChildType.execute()", "method"),
        declaration("MiddleType", "class"),
        declaration("ParentType", "interface"),
        declaration("ParentType.execute()", "method"),
        reference("ChildType", "MiddleType", "parsed", "extends"),
        reference("MiddleType", "ParentType", "parsed", "implements"),
        test("ChildCheck"),
        test("MiddleCheck"),
        test("ParentCheck"),
        test("NoOwner"),
        reference("ChildCheck", "execute", "textual", "text"),
        reference("ChildCheck", "ChildType", "textual", "text"),
        reference("MiddleCheck", "execute", "textual", "text"),
        reference("MiddleCheck", "MiddleType", "parsed", "type"),
        reference("ParentCheck", "execute", "textual", "text"),
        reference("ParentCheck", "ParentType", "textual", "text"),
        reference("NoOwner", "execute", "textual", "text"),
    ];
    let out = run(&refs, &[], &["ChildType.execute()"], 1, json!({}));
    assert_eq!(names(&out), ["ChildCheck", "MiddleCheck", "ParentCheck"]);
    assert_eq!(out["possible"][0]["symbol"], "NoOwner");
    assert_eq!(out["possible"][0]["reasons"], json!(["member_name_only"]));
}

/// One textual word, two declarations with that name: both seeds see the same
/// textual row, and the row stays ambiguous for each of them.
#[test]
fn an_ambiguous_textual_word_reaches_every_target_and_stays_ambiguous() {
    let refs = [
        declaration("Alpha", "class"),
        declaration("Alpha.execute()", "method"),
        declaration("Beta", "class"),
        declaration("Beta.execute()", "method"),
        test("BothChecks"),
        reference("BothChecks", "execute", "textual", "text"),
        reference("BothChecks", "Alpha", "textual", "text"),
        reference("BothChecks", "Beta", "textual", "text"),
    ];
    let alpha = run(&refs, &[], &["Alpha.execute()"], 1, json!({}));
    let beta = run(&refs, &[], &["Beta.execute()"], 1, json!({}));
    for (out, target) in [(&alpha, "Alpha.execute()"), (&beta, "Beta.execute()")] {
        assert_eq!(names(out), ["BothChecks"], "{target}");
        let hit = hits(out, "textual");
        assert_eq!(hit.len(), 1, "{target}");
        assert_eq!(hit[0]["ambiguous"], true, "{target}");
        assert_eq!(hit[0]["routes"][0]["target"]["symbol"], target);
        let step = &out["tests"][0]["path"][0];
        assert_eq!(step["ambiguous"], true, "{target}");
        assert_eq!(step["target"]["symbol"], target);
        let row = &out["references"][step["reference"].to_string()];
        assert_eq!(row["referenced"], "execute");
    }
    assert_eq!(
        alpha["tests"][0]["path"][0]["reference"],
        beta["tests"][0]["path"][0]["reference"]
    );
}

/// A test file that declares the owning type proves a bare member word, even
/// when the same word also names a member of a type declared elsewhere.
#[test]
fn a_test_file_declaring_the_type_proves_its_member_word() {
    let mut owner = declaration("Widget", "class");
    owner["file"] = json!("app/Suite.cls");
    let mut member = declaration("Widget.execute()", "method");
    member["file"] = json!("app/Suite.cls");
    let refs = [
        owner,
        member,
        declaration("Gadget", "class"),
        declaration("Gadget.execute()", "method"),
        test("Suite"),
        test("Loose"),
        reference("Suite", "execute", "textual", "text"),
        reference("Loose", "execute", "textual", "text"),
    ];
    let widget = run(&refs, &[], &["Widget.execute()"], 1, json!({}));
    assert_eq!(names(&widget), ["Suite"]);
    assert_eq!(widget["tests"][0]["reasons"], json!(["direct"]));
    assert_eq!(widget["tests"][0]["path"][0]["ambiguous"], true);
    assert_eq!(widget["possible"][0]["symbol"], "Loose");
    assert_eq!(
        widget["possible"][0]["reasons"],
        json!(["member_name_only"])
    );
    let gadget = run(&refs, &[], &["Gadget.execute()"], 1, json!({}));
    assert_eq!(names(&gadget), Vec::<&str>::new());
    let possible: Vec<_> = gadget["possible"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| (t["symbol"].as_str().unwrap(), t["reasons"].clone()))
        .collect();
    assert_eq!(
        possible,
        [
            ("Loose", json!(["member_name_only"])),
            ("Suite", json!(["member_name_only"]))
        ]
    );
}
