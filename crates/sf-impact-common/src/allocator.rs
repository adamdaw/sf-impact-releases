//! How the commands' allocator, mimalloc, returns freed memory. By default
//! it keeps freed pages for a second before returning them to the system,
//! which raised peak RSS by up to 275 MiB in the tests stage; at 0 it returns
//! them at once, for a few hundredths of a second.

use libmimalloc_sys::{mi_option_set, mi_option_t};

/// `mi_option_purge_delay` in mimalloc's `mimalloc.h`, which
/// `libmimalloc-sys` does not name.
const PURGE_DELAY: mi_option_t = 15;

/// Makes mimalloc return freed memory to the system at once. Call it first in
/// `main`, before any thread starts.
pub fn return_freed_memory_at_once() {
    // SAFETY: sets a valid option; no other thread runs yet to race with it.
    unsafe { mi_option_set(PURGE_DELAY, 0) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libmimalloc_sys::mi_option_get;
    use std::{env, process::Command};

    const NAMED: &str = "allocator::tests::the_index_is_the_option_named_purge_delay";
    /// Marks the run that the test below starts, whatever else the
    /// environment holds.
    const CHILD: &str = "SF_IMPACT_PURGE_DELAY_CHILD";

    #[test]
    fn freed_memory_is_returned_at_once() {
        return_freed_memory_at_once();
        // SAFETY: reads an option; any index below _mi_option_last is valid.
        assert_eq!(unsafe { mi_option_get(PURGE_DELAY) }, 0);
    }

    /// The code names the option by index, the environment by name: a run
    /// given `MIMALLOC_PURGE_DELAY=37` must read 37 at that index.
    #[test]
    fn the_index_is_the_option_named_purge_delay() {
        if env::var_os(CHILD).is_some() {
            // SAFETY: as above.
            assert_eq!(unsafe { mi_option_get(PURGE_DELAY) }, 37);
            return;
        }
        let output = Command::new(env::current_exe().unwrap())
            .args(["--exact", NAMED])
            .env(CHILD, "1")
            .env("MIMALLOC_PURGE_DELAY", "37")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{stdout}");
        assert!(stdout.contains("1 passed"), "{stdout}");
    }
}
