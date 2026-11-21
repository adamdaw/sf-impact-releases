//! Draft 2020-12 validation shared by the crates' integration tests.
use serde_json::Value;

pub struct Schema(jsonschema::Validator);
impl Schema {
    pub fn parse(text: &str) -> Self {
        let document: Value = serde_json::from_str(text).expect("the schema is JSON");
        Self(jsonschema::draft202012::new(&document).expect("valid Draft 2020-12 schema"))
    }
    pub fn errors(&self, instance: &Value) -> Vec<String> {
        self.0
            .iter_errors(instance)
            .map(|error| error.to_string())
            .collect()
    }
}
