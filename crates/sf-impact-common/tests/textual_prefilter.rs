use sf_impact_common::rows::{Reference, Row};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

struct Counted;
thread_local! {
    static COUNT: Cell<Option<usize>> = const { Cell::new(None) };
}
fn allocated() {
    let _ = COUNT.try_with(|count| {
        if let Some(n) = count.get() {
            count.set(Some(n + 1));
        }
    });
}
unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocated();
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocated();
        unsafe { System.realloc(ptr, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counted = Counted;

#[test]
fn the_prefilter_allocates_nothing_and_only_skips_valid_textual_rows() {
    const TEXT: &str = r#"{"row":"reference","tier":"textual","kind":"text","referenced":"Invoice","referrer":null,"package":"app","file":"app/A.cls","line":1,"column":2}"#;
    let cases = [
        TEXT.to_owned(),
        TEXT.replace("null", "\"Invoice\""),
        TEXT.replace("Invoice", "État"),
        TEXT.replace("textual", "parsed"),
        TEXT.replace("\"line\":1", "\"line\":0"),
        TEXT.replace("\"line\":1", "\"line\":4294967296"),
        TEXT.replace("\"line\":1", "\"line\":01"),
        TEXT.replace("\"line\":1", "\"line\":1.0"),
        TEXT.replace("Invoice", ""),
        TEXT.replace("app/A.cls", ""),
        TEXT.replace("Invoice", "\\u0041"),
        TEXT.replace("\"line\":1", "\"line\":1,\"line\":2"),
        format!("{TEXT} trailing"),
        r#"{"row":"declaration","unknown":{"tier":"textual"}}"#.into(),
    ];
    COUNT.with(|n| n.set(Some(0)));
    let results: [bool; 14] = std::array::from_fn(|i| Reference::is_textual_row(&cases[i]));
    let allocations = COUNT.with(|n| n.replace(None)).unwrap();
    assert_eq!(allocations, 0);
    assert_eq!(&results[..3], &[true, true, true]);
    assert!(results[3..].iter().all(|skip| !skip));
    for (line, skip) in cases.iter().zip(results) {
        if skip {
            assert!(
                matches!(serde_json::from_str::<Row>(line), Ok(Row::Reference(r)) if r.tier == sf_impact_common::rows::Tier::Textual)
            );
        }
    }
}
