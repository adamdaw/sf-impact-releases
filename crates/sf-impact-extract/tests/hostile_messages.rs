#[path = "../../sf-impact-common/tests/support/hostile.rs"]
mod hostile;
use hostile::{Dir, HOSTILE, assert_clean};

const BINARY: &str = env!("CARGO_BIN_EXE_sf-impact-extract");

#[test]
fn a_hostile_value_is_shown_on_the_one_line_of_a_message() {
    let dir = Dir::new("sf-impact-extract");
    for value in HOSTILE {
        for args in [
            vec![*value],
            vec!["--jobs", value],
            vec!["--format", value],
            vec!["--root", value],
            vec!["--dump", value],
        ] {
            assert_clean(BINARY, &args, &dir.0);
        }
    }
}
