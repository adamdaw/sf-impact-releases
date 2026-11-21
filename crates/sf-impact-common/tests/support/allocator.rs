//! Each command allocates with mimalloc. Asked by `MIMALLOC_VERBOSE`,
//! mimalloc says on stderr that it started; no other allocator does.

use std::process::Command;

pub fn assert_mimalloc(binary: &str) {
    let output = Command::new(binary)
        .arg("--version")
        .env("MIMALLOC_VERBOSE", "1")
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{binary}: {stderr}");
    assert!(
        stderr.contains("mimalloc: process init"),
        "{binary} did not start mimalloc; stderr: {stderr}"
    );
}
