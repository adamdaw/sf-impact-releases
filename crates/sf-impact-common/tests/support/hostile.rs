//! What a hostile value does to standard error, for each binary: a path or
//! an argument that holds a line feed, a carriage return, a terminal escape
//! or a CI command (`::error::`) is a value that a repository, a branch
//! name or a caller controls, and it must come out as text on the one line
//! of the message, never as a line of its own or as a control character.
#![allow(dead_code)]
use std::path::Path;
use std::process::Command;

/// Every one of these has `::` in it, so that a message that shows the
/// value shows that too, and a test can tell that the value was shown.
pub const HOSTILE: &[&str] = &[
    "x\n::error::forged\ny",
    "\n::error::at the start",
    "x\r::error::carriage return",
    "x\r\n::error file=a,line=1::crlf",
    "x\u{1b}]8;;http://e.example\u{7}::link\u{1b}[2K\u{1b}[1A",
    "::stop-commands::token",
    "x\n::stop-commands::token\n::add-mask::secret",
    "x\u{2028}::error::line separator",
    "x\u{85}::error::next line",
    "x\u{202e}::error::bidi\u{2066}",
    "tab\t::and\u{7f}delete",
];

/// Runs `binary` with `args` in `dir` and holds its standard error to the
/// rule. The run must fail: a message is what is checked.
pub fn assert_clean(binary: &str, args: &[&str], dir: &Path) {
    let output = Command::new(binary)
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .output()
        .unwrap();
    let context = format!("{binary} {args:?}");
    assert!(!output.status.success(), "{context}");
    let stderr = String::from_utf8(output.stderr).unwrap_or_else(|_| panic!("{context}: UTF-8"));

    let message = stderr.split("\n\n").next().unwrap().trim_end_matches('\n');
    assert!(!message.contains('\n'), "{context}: {stderr:?}");
    assert!(
        message.contains("::"),
        "{context}: the value is not shown at all: {stderr:?}"
    );
    for line in stderr.lines() {
        assert!(!line.starts_with("::"), "{context}: {stderr:?}");
    }
    for c in stderr.chars() {
        let troubles = (c.is_control() && c != '\n' && c != '\t')
            || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}');
        assert!(!troubles, "{context}: {c:?} in {stderr:?}");
    }
}

/// An empty directory of its own, removed when it is dropped.
pub struct Dir(pub std::path::PathBuf);

impl Dir {
    pub fn new(name: &str) -> Dir {
        let dir =
            std::env::temp_dir().join(format!("sf-impact-hostile-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Dir(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
