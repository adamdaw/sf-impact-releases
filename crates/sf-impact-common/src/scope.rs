//! Scope: the `packageDirectories` of `sfdx-project.json` plus the
//! `unpackagedMetadata` paths they name. Nothing else is read.

use crate::rows::{ScopeError, ScopeErrorReason, ScopeSource};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, io, path::Path};

pub const PROJECT_FILE: &str = "sfdx-project.json";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Project {
    #[serde(default)]
    package_directories: Vec<PackageDirectory>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageDirectory {
    path: String,
    unpackaged_metadata: Option<PathEntry>,
}

#[derive(Deserialize)]
struct PathEntry {
    path: String,
}

/// The directories to read, and the declared ones that could not be.
#[derive(Debug, PartialEq, Eq)]
pub struct Scope {
    /// Relative to the root, `/`-separated, sorted, without duplicates.
    pub dirs: Vec<String>,
    /// Provenance of each normalised directory. A package declaration wins
    /// if the same directory is also named as unpackaged metadata.
    pub sources: BTreeMap<String, ScopeSource>,
    pub errors: Vec<ScopeError>,
}

/// A file in scope. `package` is the scope directory it was found under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeFile {
    pub path: String,
    pub package: String,
}

/// The directories that the content of `sfdx-project.json` declares, as
/// written, each with the key that names it.
pub fn declared(project: &[u8]) -> Result<Vec<(String, ScopeSource)>, serde_json::Error> {
    let project: Project = serde_json::from_slice(project)?;
    let mut declared = Vec::new();
    for dir in project.package_directories {
        declared.push((dir.path, ScopeSource::PackageDirectories));
        if let Some(unpackaged) = dir.unpackaged_metadata {
            declared.push((unpackaged.path, ScopeSource::UnpackagedMetadata));
        }
    }
    Ok(declared)
}

/// The scope directory that `path` is under, or is: the nearest, where they
/// nest. A path can be a scope directory itself where version control has
/// the directory as one entry, a submodule or a symbolic link. `dirs` are
/// normalised and `path` is relative to the root; a path that leaves the
/// root is under none.
pub fn package_of<'a>(dirs: &'a [String], path: &str) -> Option<&'a str> {
    let holds = |dir: &str| match dir {
        "" => !path.starts_with("../"),
        _ => path
            .strip_prefix(dir)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/')),
    };
    let above = dirs.iter().map(String::as_str).filter(|dir| holds(dir));
    above.max_by_key(|dir| dir.len())
}

impl Scope {
    /// The scope that the content of `<root>/sfdx-project.json` declares,
    /// with the directories of it that are there to be read.
    pub fn of(root: &Path, project: &[u8]) -> Result<Scope, String> {
        let path = root.join(PROJECT_FILE);
        let declared = declared(project);
        // serde's message can quote a key of the file as it is.
        let shown = |e: serde_json::Error| crate::diagnostic::shown(&e.to_string());
        let declared = declared.map_err(|e| format!("{}: {}", path.display(), shown(e)))?;

        let mut scope = Scope {
            dirs: Vec::new(),
            sources: BTreeMap::new(),
            errors: Vec::new(),
        };
        for (path, declared_in) in declared {
            match check(root, &path) {
                Ok(dir) => {
                    scope
                        .sources
                        .entry(dir.clone())
                        .and_modify(|source| {
                            if declared_in == ScopeSource::PackageDirectories {
                                *source = declared_in;
                            }
                        })
                        .or_insert(declared_in);
                    scope.dirs.push(dir);
                }
                Err(reason) => scope.errors.push(ScopeError {
                    path,
                    declared_in,
                    reason,
                }),
            }
        }
        scope.dirs.sort();
        scope.dirs.dedup();
        Ok(scope)
    }

    /// Every regular file under the scope directories, sorted by path.
    /// Symbolic links are not followed: they could lead outside the scope.
    pub fn files(&self, root: &Path) -> io::Result<Vec<ScopeFile>> {
        let mut files = Vec::new();
        for package in &self.dirs {
            let mut pending = vec![package.clone()];
            while let Some(dir) = pending.pop() {
                for entry in fs::read_dir(root.join(&dir))? {
                    let entry = entry?;
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let path = if dir.is_empty() {
                        name.clone()
                    } else {
                        format!("{dir}/{name}")
                    };
                    let kind = entry.file_type()?;
                    if kind.is_file() {
                        files.push(ScopeFile {
                            path,
                            package: package.clone(),
                        });
                    } else if kind.is_dir() && name != ".git" && !self.dirs.contains(&path) {
                        // A nested scope directory is walked as its own package.
                        pending.push(path);
                    }
                }
            }
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(files)
    }
}

fn check(root: &Path, declared: &str) -> Result<String, ScopeErrorReason> {
    let dir = normalise(declared).ok_or(ScopeErrorReason::OutsideRoot)?;
    // Every component must be a real directory. A symbolic link anywhere
    // along the path could lead out of the root, so none is followed.
    let mut path = root.to_path_buf();
    for component in dir.split('/').filter(|component| !component.is_empty()) {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(ScopeErrorReason::NotADirectory),
            Err(_) => return Err(ScopeErrorReason::Missing),
        }
    }
    Ok(dir)
}

/// A declared path as a `/`-separated path relative to the root, or `None`
/// if it would leave the root. Some projects write Windows separators
/// (`plugins\\Name`); those must resolve on Linux and macOS too.
pub fn normalise(declared: &str) -> Option<String> {
    let path = declared.replace('\\', "/");
    let is_absolute = path.starts_with('/') || path.split('/').next()?.ends_with(':');
    let parts: Vec<&str> = path
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    if is_absolute || parts.contains(&"..") {
        return None;
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A project directory under the system temporary directory, removed on drop.
    struct Project(PathBuf);

    impl Project {
        /// `files` are paths relative to the project root; each gets the same content.
        fn new(name: &str, project_json: &str, files: &[&str]) -> Project {
            let root =
                std::env::temp_dir().join(format!("sf-impact-scope-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join(PROJECT_FILE), project_json).unwrap();
            for file in files {
                let path = root.join(file);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(path, "x").unwrap();
            }
            Project(root)
        }

        /// The scope that its project file declares.
        fn scope(&self) -> Scope {
            let project = fs::read(self.0.join(PROJECT_FILE)).unwrap();
            Scope::of(&self.0, &project).unwrap()
        }

        fn files(&self) -> Vec<(String, String)> {
            let scope = self.scope();
            let files = scope.files(&self.0).unwrap();
            files.into_iter().map(|f| (f.path, f.package)).collect()
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn pair(path: &str, package: &str) -> (String, String) {
        (path.to_string(), package.to_string())
    }

    #[test]
    fn only_files_under_the_declared_directories_are_in_scope() {
        let project = Project::new(
            "only-declared",
            r#"{"packageDirectories": [{"path": "app", "unpackagedMetadata": {"path": "extra"}}]}"#,
            &[
                "app/classes/A.cls",
                "extra/B.cls",
                "elsewhere/C.cls",
                "D.cls",
            ],
        );

        assert_eq!(
            project.files(),
            [
                pair("app/classes/A.cls", "app"),
                pair("extra/B.cls", "extra")
            ]
        );
    }

    #[test]
    fn a_file_in_a_nested_scope_directory_is_listed_once_under_the_nearest() {
        let project = Project::new(
            "nested",
            r#"{"packageDirectories": [{"path": "app"}, {"path": "app/plugin"}, {"path": "./app/"}]}"#,
            &["app/A.cls", "app/plugin/B.cls"],
        );

        assert_eq!(
            project.files(),
            [
                pair("app/A.cls", "app"),
                pair("app/plugin/B.cls", "app/plugin")
            ]
        );
    }

    #[test]
    fn directories_that_cannot_be_read_are_reported_with_the_reason() {
        let project = Project::new(
            "unreadable",
            r#"{"packageDirectories": [{"path": "gone"}, {"path": "a-file"}, {"path": "..\\up"}]}"#,
            &["a-file"],
        );

        let scope = project.scope();

        let errors: Vec<_> = scope
            .errors
            .iter()
            .map(|e| (e.path.as_str(), e.reason))
            .collect();
        assert_eq!(
            errors,
            [
                ("gone", ScopeErrorReason::Missing),
                ("a-file", ScopeErrorReason::NotADirectory),
                ("..\\up", ScopeErrorReason::OutsideRoot),
            ]
        );
        assert!(scope.dirs.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symbolic_links_are_not_followed_out_of_scope() {
        let project = Project::new(
            "symlink",
            r#"{"packageDirectories": [{"path": "app"}]}"#,
            &["app/A.cls", "outside/secret.cls"],
        );
        std::os::unix::fs::symlink(project.0.join("outside"), project.0.join("app/link")).unwrap();

        assert_eq!(project.files(), [pair("app/A.cls", "app")]);
    }

    /// Regression: the declared path is `link/classes`, `link`
    /// points outside the root, and `classes` is a real directory there.
    #[cfg(unix)]
    #[test]
    fn a_symbolic_link_part_way_along_a_declared_path_does_not_lead_out_of_the_root() {
        let outside = Project::new("escape-target", "{}", &["classes/Secret.cls"]);
        let project = Project::new(
            "escape",
            r#"{"packageDirectories": [{"path": "link/classes"}, {"path": "real/link/deeper"}]}"#,
            &["real/placeholder"],
        );
        std::os::unix::fs::symlink(&outside.0, project.0.join("link")).unwrap();
        std::os::unix::fs::symlink(&outside.0, project.0.join("real/link")).unwrap();

        let scope = project.scope();

        let errors: Vec<_> = scope
            .errors
            .iter()
            .map(|e| (e.path.as_str(), e.reason))
            .collect();
        assert_eq!(
            errors,
            [
                ("link/classes", ScopeErrorReason::NotADirectory),
                ("real/link/deeper", ScopeErrorReason::NotADirectory),
            ]
        );
        assert_eq!(scope.files(&project.0).unwrap(), []);
    }

    #[test]
    fn the_declared_directories_are_read_from_the_content_of_the_project_file() {
        let project = br#"{"packageDirectories": [
            {"path": "app", "unpackagedMetadata": {"path": "extra"}},
            {"path": "plugins/billing"}
        ]}"#;

        assert_eq!(
            declared(project).unwrap(),
            [
                ("app".to_string(), ScopeSource::PackageDirectories),
                ("extra".to_string(), ScopeSource::UnpackagedMetadata),
                (
                    "plugins/billing".to_string(),
                    ScopeSource::PackageDirectories
                ),
            ]
        );
        assert!(declared(b"not json").is_err());
    }

    #[test]
    fn a_path_is_in_the_nearest_scope_directory_above_it() {
        let dirs = ["app".to_string(), "app/plugin".to_string()];

        assert_eq!(package_of(&dirs, "app/classes/A.cls"), Some("app"));
        assert_eq!(package_of(&dirs, "app/plugin/B.cls"), Some("app/plugin"));
        assert_eq!(package_of(&dirs, "application/C.cls"), None);
        // The directory itself, where git has it as one entry: a submodule
        // or a symbolic link.
        assert_eq!(package_of(&dirs, "app"), Some("app"));
        assert_eq!(package_of(&dirs, "app/plugin"), Some("app/plugin"));
        assert_eq!(package_of(&dirs, "docs/notes.md"), None);
    }

    #[test]
    fn the_root_as_a_scope_directory_holds_every_path_below_it() {
        let dirs = [String::new(), "app".to_string()];

        assert_eq!(package_of(&dirs, "notes.md"), Some(""));
        assert_eq!(package_of(&dirs, "app/A.cls"), Some("app"));
        assert_eq!(package_of(&dirs, "../elsewhere.md"), None);
    }

    #[test]
    fn windows_separators_become_forward_slashes() {
        assert_eq!(
            normalise("plugins\\Billing").as_deref(),
            Some("plugins/Billing")
        );
    }

    #[test]
    fn redundant_segments_are_dropped() {
        assert_eq!(
            normalise("./force-app//main/").as_deref(),
            Some("force-app/main")
        );
    }

    #[test]
    fn the_root_itself_is_the_empty_path() {
        assert_eq!(normalise(".").as_deref(), Some(""));
    }

    #[test]
    fn paths_that_leave_the_root_are_refused() {
        assert_eq!(normalise("../elsewhere"), None);
        assert_eq!(normalise("force-app\\..\\..\\elsewhere"), None);
        assert_eq!(normalise("/etc"), None);
        assert_eq!(normalise("C:\\src\\force-app"), None);
    }
}
