#[path = "../../sf-impact-common/tests/support/allocator.rs"]
mod allocator;

#[test]
fn allocates_with_mimalloc() {
    allocator::assert_mimalloc(env!("CARGO_BIN_EXE_sf-impact-changed"));
}
