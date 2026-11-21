#[path = "../../sf-impact-common/tests/support/hostile.rs"]
mod hostile;
use hostile::{Dir, HOSTILE, assert_clean};

const BINARY: &str = env!("CARGO_BIN_EXE_sf-impact-tests");

#[test]
fn a_hostile_value_is_shown_on_the_one_line_of_a_message() {
    let dir = Dir::new("sf-impact-tests");
    for value in HOSTILE {
        let out = format!("{value}/x");
        for args in [
            vec![*value],
            vec!["--refs", value, "--symbol", "X"],
            vec!["--refs", "-", "--symbol", "X", "--config", value],
            vec!["--refs", "-", "--symbol", "X", "--out", &out],
            vec!["--refs", "-", "--changed", value],
        ] {
            assert_clean(BINARY, &args, &dir.0);
        }
    }
}
