//! The optional config file at the repository root. A repository that needs
//! none of it ships no file. Keys used by other tools are ignored here.

use serde::Deserialize;
use std::path::Path;

pub const CONFIG_FILE: &str = "sf-impact.json";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Config {
    /// Declared names shorter than this are left out of the textual tier.
    pub textual_min_length: usize,
    /// Short names to match textually anyway.
    pub textual_allow_list: Vec<String>,
    /// Namespaces of installed packages; references into them are `external`.
    pub namespaces: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            textual_min_length: 6,
            textual_allow_list: Vec::new(),
            namespaces: Vec::new(),
        }
    }
}

impl Config {
    /// The config that the content of `<root>/sf-impact.json` is; the
    /// default config where there is no file.
    pub fn of(root: &Path, bytes: Option<&[u8]>) -> Result<Config, String> {
        let Some(bytes) = bytes else {
            return Ok(Config::default());
        };
        let path = root.join(CONFIG_FILE);
        // serde's message can quote a key of the file as it is.
        let shown = |e: serde_json::Error| crate::diagnostic::shown(&e.to_string());
        serde_json::from_slice(bytes).map_err(|e| format!("{}: {}", path.display(), shown(e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_keys_take_their_defaults() {
        let config: Config = serde_json::from_str(r#"{"namespaces": ["npsp"]}"#).unwrap();

        assert_eq!(config.textual_min_length, 6);
        assert_eq!(config.namespaces, ["npsp"]);
    }

    #[test]
    fn keys_meant_for_other_tools_are_ignored() {
        let config: Config = serde_json::from_str(r#"{"testSuffix": "_Test"}"#).unwrap();

        assert_eq!(config, Config::default());
    }
}
