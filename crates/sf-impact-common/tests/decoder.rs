//! Strict row decoding against outcomes captured from a pre-publication release reader.
#[allow(dead_code)]
#[path = "support/golden.rs"]
mod golden;
use serde::{Serialize, de::DeserializeOwned};

fn decode<T: DeserializeOwned + Serialize>(line: &str) -> (i32, String, String) {
    match serde_json::from_str::<T>(line) {
        Ok(row) => (
            0,
            format!("{}\n", serde_json::to_string(&row).unwrap()),
            String::new(),
        ),
        Err(error) => (1, String::new(), format!("{error}\n")),
    }
}
#[test]
fn decoder_matches_golden_outcomes_except_duplicate_keys() {
    for case in golden::load().cases {
        let (status, stdout, stderr) = if case.changed {
            decode::<sf_impact_common::changed::Row>(&case.line)
        } else {
            decode::<sf_impact_common::rows::Row>(&case.line)
        };
        case.check(&case.decoder, status, stdout.as_bytes(), stderr.as_bytes());
    }
}
#[test]
fn goldens_cover_changed_optional_fields_and_nested_duplicates() {
    let goldens = golden::load();
    for field in ["binary", "previous_file", "gaps", "candidates"] {
        assert!(
            goldens
                .cases
                .iter()
                .any(|case| case.changed && case.id.ends_with(&format!("/{field}:false"))),
            "missing {field} mutation"
        );
    }
    assert!(
        goldens
            .cases
            .iter()
            .any(|case| case.id.starts_with("extra/binary/"))
    );
    assert!(
        goldens
            .cases
            .iter()
            .any(|case| case.id.starts_with("extra/moved-unchanged/"))
    );
    assert_eq!(
        goldens
            .cases
            .iter()
            .filter(|case| case.id.starts_with("nested-hunk/") && case.duplicate.is_some())
            .count(),
        2
    );
}
