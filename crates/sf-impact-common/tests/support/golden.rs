//! Captured once from a pre-publication release build.
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
pub struct Goldens {
    pub base: String,
    pub refs_header: String,
    pub changed_header: String,
    pub declaration: String,
    pub cases: Vec<Case>,
}
#[derive(Deserialize)]
pub struct Case {
    pub id: String,
    pub changed: bool,
    pub line: String,
    pub duplicate: Option<String>,
    pub decoder: Outcome,
    pub cli: Outcome,
}
#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct Outcome {
    pub status: i32,
    pub stdout_sha256: String,
    pub stderr: String,
}
pub fn load() -> Goldens {
    // decoder-shapes.jsonl is the frozen input; decoder-goldens.json is its captured output.
    let goldens: Goldens = serde_json::from_str(include_str!("decoder-goldens.json")).unwrap();
    assert_eq!(goldens.base, "fa4f18b12c9aea49632e7baecd2edb620f5aa8a4");
    assert_eq!(
        goldens
            .cases
            .iter()
            .filter(|c| c.id.starts_with("astra/") && c.duplicate.is_none())
            .count(),
        952
    );
    goldens
}
impl Case {
    pub fn check(&self, expected: &Outcome, status: i32, stdout: &[u8], stderr: &[u8]) {
        let actual = Outcome {
            status,
            stdout_sha256: Sha256::digest(stdout)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
            stderr: String::from_utf8(stderr.to_vec()).unwrap(),
        };
        if let Some(field) = &self.duplicate {
            // Duplicate keys are the only exception to the golden outcomes.
            assert_eq!(self.decoder.status, 0, "old reader must accept {}", self.id);
            assert_eq!(status, 1, "{}: {actual:?}", self.id);
            assert!(stdout.is_empty(), "{}", self.id);
            assert!(
                actual
                    .stderr
                    .contains(&format!("duplicate field `{field}`")),
                "{}: {actual:?}",
                self.id
            );
        } else {
            assert_eq!(&actual, expected, "{}: {}", self.id, self.line);
        }
    }
}
