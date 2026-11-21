#[path = "../../sf-impact-common/tests/support/hostile.rs"]
mod hostile;
use hostile::{Dir, HOSTILE, assert_clean};
use std::process::Command;

const BINARY: &str = env!("CARGO_BIN_EXE_sf-impact-delta");

#[test]
fn a_hostile_value_is_shown_on_the_one_line_of_a_message() {
    let dir = Dir::new("sf-impact-delta");
    let git = |args: &[&str]| {
        let status = Command::new("git")
            .args(args)
            .current_dir(&dir.0)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "A Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "A Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    };
    git(&["init", "-q"]);
    git(&["commit", "-q", "--allow-empty", "-m", "base"]);
    for value in HOSTILE {
        for args in [
            vec![*value],
            vec!["--base", value, "--head", "HEAD"],
            vec!["--merge-base", "HEAD", "--head", value],
        ] {
            assert_clean(BINARY, &args, &dir.0);
        }
    }
}
