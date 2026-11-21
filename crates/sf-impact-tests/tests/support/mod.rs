//! The provenance rows that inputs written by hand start with: refs.jsonl
//! of one head, and the changed.jsonl that was made of it.
#![allow(dead_code)]
use serde_json::{Value, json};
use std::io::{ErrorKind, Write};
use std::process::{Child, Output};

const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const BASE: &str = "b000000000000000000000000000000000000000";
pub const HEAD: &str = "a111111111111111111111111111111111111111";
pub const HEAD_TREE: &str = "a222222222222222222222222222222222222222";
pub const CONFIG: &str = "0123456789abcdef";

fn scope() -> Value {
    json!({"packageDirectories": ["app"], "unpackagedMetadata": []})
}
fn without_row(row: &Value) -> Value {
    let mut row = row.clone();
    row.as_object_mut().unwrap().remove("row");
    row
}
pub fn refs_header() -> Value {
    json!({
        "row": "provenance", "tool": "sf-impact-extract", "version": VERSION, "schema": 2,
        "tree": {"state": "commit", "commit": HEAD, "tree": HEAD_TREE},
        "root": "packages/billing", "scope": scope(), "config": CONFIG,
    })
}
pub fn changed_header() -> Value {
    let delta = json!({
        "tool": "sf-impact-delta", "version": VERSION, "schema": 1,
        "base": BASE, "head": HEAD, "head_tree": HEAD_TREE,
        "root": "packages/billing", "scope": scope(), "config": CONFIG,
    });
    json!({
        "row": "provenance", "tool": "sf-impact-changed", "version": VERSION, "schema": 1,
        "base": BASE, "head": HEAD, "head_tree": HEAD_TREE, "root": "packages/billing",
        "scope": scope(), "config": CONFIG, "confidence": "full",
        "inputs": {"refs": without_row(&refs_header()), "delta": delta},
    })
}

/// Writes `input` to the child's stdin and waits for it. A child may
/// refuse its arguments and exit before it reads a byte, so a write that
/// fails with a broken pipe is not an error of the test; any other is.
/// Stdin is closed before the wait, so the child sees the end of its input.
pub fn feed_and_wait(mut child: Child, input: &[u8]) -> Output {
    let mut stdin = child.stdin.take().unwrap();
    match stdin.write_all(input) {
        Err(e) if e.kind() == ErrorKind::BrokenPipe => {}
        written => written.unwrap(),
    }
    drop(stdin);
    child.wait_with_output().unwrap()
}
