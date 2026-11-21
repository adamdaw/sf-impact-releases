#[path = "../../sf-impact-common/tests/support/pack.rs"]
mod pack;
mod support;
use serde_json::{Value, json};

#[test]
fn impact_keeps_verified_input_provenance_for_its_consumer() {
    let refs = support::refs_header().to_string() + "\n";
    let changes = support::changed_header().to_string() + "\n";
    let document = sf_impact_tests::impact(
        pack::of_jsonl(&refs).as_slice(),
        Some(changes.as_bytes()),
        &[],
        2,
        &sf_impact_tests::Config::default(),
        false,
    )
    .unwrap();
    let mut refs: Value = serde_json::from_str(refs.trim()).unwrap();
    let mut changes: Value = serde_json::from_str(changes.trim()).unwrap();
    refs.as_object_mut().unwrap().remove("row");
    changes.as_object_mut().unwrap().remove("row");
    assert_eq!(
        document["provenance"],
        json!({"tool":"sf-impact-tests","version":env!("CARGO_PKG_VERSION"),"schema":1,"inputs":{"refs":refs,"changed":changes}})
    );
}

#[test]
fn symbol_query_keeps_refs_and_explicitly_has_no_changed_input() {
    let refs = support::refs_header().to_string() + "\n";
    let document = sf_impact_tests::impact(
        pack::of_jsonl(&refs).as_slice(),
        None,
        &["Unknown".into()],
        2,
        &sf_impact_tests::Config::default(),
        false,
    )
    .unwrap();
    assert_eq!(
        document["provenance"]["inputs"]["refs"]["tree"]["commit"],
        support::HEAD
    );
    assert!(
        document["provenance"]["inputs"]
            .as_object()
            .unwrap()
            .contains_key("changed")
    );
    assert!(document["provenance"]["inputs"]["changed"].is_null());
}
