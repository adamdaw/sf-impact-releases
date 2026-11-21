//! Input built to exhaust the readers. Each is read within a bound, and a
//! file that reaches one says so with a `parse_error` row.
use super::*;

fn rows_of<'a>(rows: &'a [Value], file: &str) -> Vec<&'a Value> {
    rows.iter().filter(|r| r["file"] == file).collect()
}

#[test]
fn apex_nested_past_the_limit_is_a_parse_error_and_its_syntax_is_not_read() {
    // 100,000 parentheses, 200 KB, overflowed the walk's stack.
    let deep = format!(
        "public class Deep {{\n  Integer s = {}1{};\n}}\n",
        "(".repeat(100_000),
        ")".repeat(100_000)
    );
    let rows = extract(&[
        ("pkg/classes/Deep.cls", &deep),
        (
            "pkg/classes/Other.cls",
            "public class Other {\n  Deep d;\n}\n",
        ),
    ]);

    let deep = rows_of(&rows, "pkg/classes/Deep.cls");
    assert_eq!(deep.len(), 1, "{deep:#?}");
    assert_eq!(
        (&deep[0]["row"], &deep[0]["line"]),
        (&"parse_error".into(), &2.into())
    );
    // Nothing declares `Deep`, so what names it is unresolved.
    let other = rows_of(&rows, "pkg/classes/Other.cls");
    assert!(
        other
            .iter()
            .any(|r| r["referenced"] == "Deep" && r["resolution"] == "unresolved"),
        "{other:#?}"
    );
}

#[test]
fn apex_nested_within_the_limit_is_read() {
    // A string of 1,000 concatenated calls nests once per operand.
    let calls = vec!["Other.name()"; 1_000].join(" + ");
    let wide = format!("public class Wide {{\n  String s = {calls};\n}}\n");
    let rows = extract(&[
        ("pkg/classes/Wide.cls", &wide),
        (
            "pkg/classes/Other.cls",
            "public class Other {\n  public static String name() { return ''; }\n}\n",
        ),
    ]);

    let wide = rows_of(&rows, "pkg/classes/Wide.cls");
    assert!(!wide.iter().any(|r| r["row"] == "parse_error"));
    let calls = wide
        .iter()
        .filter(|r| r["tier"] == "parsed" && r["referenced"] == "Other.name()");
    assert_eq!(calls.count(), 1_000);
}

#[test]
fn lwc_templates_nested_past_the_limit_are_a_parse_error_after_what_was_read() {
    // 1,000,000 nested template literals, 3 MB, overflowed the scanner's stack.
    let rows = js(&format!(
        "import 'c/childPanel';\nconst x = {};",
        "`${".repeat(1_000_000)
    ));

    has(&rows, "lwc_component", "childPanel");
    assert!(
        rows.iter()
            .any(|r| r["row"] == "parse_error" && r["line"] == 2),
        "{rows:#?}"
    );
}

#[test]
fn a_metadata_file_nested_past_the_limit_is_a_parse_error() {
    let deep = format!(
        "<?xml version=\"1.0\"?>\n<CustomObject>\n{}{}</CustomObject>\n",
        "<a>".repeat(100_000),
        "</a>".repeat(100_000)
    );
    let rows = extract(&[("pkg/objects/Acc__c/Acc__c.object-meta.xml", &deep)]);

    assert!(
        rows.iter()
            .any(|r| r["row"] == "parse_error" && r["line"] == 3),
        "{rows:#?}"
    );
}
