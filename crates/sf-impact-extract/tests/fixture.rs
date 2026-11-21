//! Runs `sf-impact-extract` on the synthetic project in `fixtures/project` and
//! checks what it writes: against the golden file, against the schema, and
//! row by row for each behaviour the fixture was written to show.
//!
//! The project is read as the one commit of a repository of its own, in a
//! temporary directory. The first row of the output names the commit that
//! was read, and where the fixture lies, in this repository, that is
//! another commit with every change to anything.
//!
//! To accept a deliberate change to the output:
//! `UPDATE_GOLDEN=1 cargo test -p sf-impact-extract --test fixture`

#[path = "fixture/catalog.rs"]
mod catalog;
#[path = "../../sf-impact-common/tests/support/schema.rs"]
mod schema;

use schema::Schema;
use serde_json::{Value, json};
use sf_impact_common::rows::{
    DeclType, Declaration, Detail, RefKind, Reference, Resolution, Row, ScopeErrorReason,
    SkipReason, Tier, UnsupportedReason,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{env, fs};

fn repository() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The fixture project as the one commit of a repository, in a temporary
/// directory that is removed on drop. The commit is made without the
/// configuration of whoever runs the tests, with a fixed identity and a
/// fixed date, so that it is the same commit on every machine.
struct Committed(PathBuf);

impl Committed {
    fn new() -> Committed {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let name = format!(
            "sf-impact-extract-fixture-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let committed = Committed(env::temp_dir().join(name));
        let _ = fs::remove_dir_all(&committed.0);
        copy(&repository().join("fixtures/project"), &committed.0);
        for args in [
            &[
                "init",
                "--quiet",
                "--initial-branch=main",
                "--object-format=sha1",
            ][..],
            &["add", "--all"],
            &["commit", "--quiet", "--message", "the fixture project"],
        ] {
            let mut git = Command::new("git");
            git.current_dir(&committed.0)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1");
            for who in ["AUTHOR", "COMMITTER"] {
                git.env(format!("GIT_{who}_NAME"), "A Test")
                    .env(format!("GIT_{who}_EMAIL"), "test@example.com")
                    .env(format!("GIT_{who}_DATE"), "2026-01-01T00:00:00Z");
            }
            let output = git.output().expect("git runs");
            assert!(output.status.success(), "git {args:?} failed: {output:?}");
        }
        committed
    }
}

impl Drop for Committed {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn extract(extra_args: &[&str]) -> String {
    let project = Committed::new();
    let output = Command::new(env!("CARGO_BIN_EXE_sf-impact-extract"))
        .arg("--root")
        .arg(&project.0)
        .args(["--format", "jsonl"])
        .args(extra_args)
        .output()
        .expect("sf-impact-extract runs");
    assert!(
        output.status.success(),
        "sf-impact-extract failed: {output:?}"
    );
    String::from_utf8(output.stdout).expect("output is UTF-8")
}

static OUTPUT: LazyLock<String> = LazyLock::new(|| extract(&[]));

/// The rows of the tree: all but the first, which says what the file is of.
static ROWS: LazyLock<Vec<Row>> = LazyLock::new(|| {
    let rows = OUTPUT.lines().skip(1).map(serde_json::from_str);
    rows.collect::<Result<_, _>>().expect("every line is a row")
});

fn declarations() -> impl Iterator<Item = &'static Declaration> {
    ROWS.iter().filter_map(|row| match row {
        Row::Declaration(declaration) => Some(declaration),
        _ => None,
    })
}

fn declaration(symbol: &str) -> &'static Declaration {
    let mut found = declarations().filter(|d| d.symbol == symbol);
    let declaration = found
        .next()
        .unwrap_or_else(|| panic!("{symbol} is declared"));
    assert!(found.next().is_none(), "{symbol} is declared once");
    declaration
}

fn references(tier: Tier) -> impl Iterator<Item = &'static Reference> {
    ROWS.iter().filter_map(move |row| match row {
        Row::Reference(reference) if reference.tier == tier => Some(reference),
        _ => None,
    })
}

/// The parsed references made from inside the declaration `referrer`.
fn parsed_from(referrer: &'static str) -> impl Iterator<Item = &'static Reference> {
    references(Tier::Parsed).filter(move |r| r.referrer.as_deref() == Some(referrer))
}

/// The one parsed reference of `kind` from `referrer` whose target starts with `referenced`.
fn parsed(referrer: &'static str, kind: RefKind, referenced: &str) -> &'static Reference {
    let mut found =
        parsed_from(referrer).filter(|r| r.kind == kind && r.referenced.starts_with(referenced));
    let reference = found
        .next()
        .unwrap_or_else(|| panic!("{referrer} has a {kind:?} reference to {referenced}"));
    assert!(found.next().is_none(), "{referrer} has one such reference");
    reference
}

fn textual_in(file: &'static str) -> impl Iterator<Item = &'static Reference> {
    references(Tier::Textual).filter(move |r| r.file.ends_with(file))
}

// The contract: golden file, schema, determinism.

/// The checked-in golden file, rewritten first when `UPDATE_GOLDEN` is set.
static GOLDEN: LazyLock<String> = LazyLock::new(|| {
    let golden = repository().join("fixtures/refs.jsonl");
    if env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(&golden, OUTPUT.as_bytes()).expect("the golden file is writable");
    }
    fs::read_to_string(&golden).expect("fixtures/refs.jsonl exists")
});

#[test]
fn the_fixture_extracts_to_the_golden_file() {
    let first_difference = OUTPUT
        .lines()
        .zip(GOLDEN.lines())
        .position(|(actual, expected)| actual != expected);
    if let Some(line) = first_difference {
        panic!(
            "output differs from fixtures/refs.jsonl at line {}:\n  actual:   {}\n  expected: {}\n\
             If the change is intended, rerun with UPDATE_GOLDEN=1 and review the diff.",
            line + 1,
            OUTPUT.lines().nth(line).unwrap_or_default(),
            GOLDEN.lines().nth(line).unwrap_or_default(),
        );
    }
    assert_eq!(*OUTPUT, *GOLDEN, "one output is a prefix of the other");
}

fn refs_schema() -> Schema {
    let path = repository().join("schema/refs.schema.json");
    Schema::parse(&fs::read_to_string(path).expect("the schema exists"))
}

#[test]
fn every_golden_row_is_valid_against_the_schema() {
    let schema = refs_schema();

    let mut kinds_seen = std::collections::BTreeSet::new();
    for (number, line) in GOLDEN.lines().enumerate() {
        let row: Value = serde_json::from_str(line).expect("a golden line is JSON");
        let errors = schema.errors(&row);
        assert!(errors.is_empty(), "line {}: {errors:?}", number + 1);
        kinds_seen.insert((row["row"].to_string(), row["tier"].to_string()));
    }

    // The golden file exercises every alternative of the schema. Parsed
    // references read from Apex and from metadata are two alternatives.
    assert_eq!(kinds_seen.len(), 9, "{kinds_seen:?}");
    for kind in ["call", "grant"] {
        assert!(GOLDEN.contains(&format!(r#""tier":"parsed","kind":"{kind}""#)));
    }
}

#[test]
fn the_schema_refuses_rows_that_break_the_contract() {
    let schema = refs_schema();
    let valid = json!({
        "row": "reference", "tier": "parsed", "kind": "call",
        "referenced": "MathUtil.round(decimal)", "referenced_type": "method",
        "resolution": "declaration",
        "referrer": null, "package": "force-app", "file": "A.cls", "line": 3, "column": 5
    });
    assert_eq!(schema.errors(&valid), Vec::<String>::new());

    let broken = |change: fn(&mut Value)| {
        let mut row = valid.clone();
        change(&mut row);
        !schema.errors(&row).is_empty()
    };
    assert!(broken(|row| row["kind"] = json!("telepathy")));
    assert!(broken(|row| row["line"] = json!(0)));
    assert!(broken(|row| row["line"] = json!("3")));
    assert!(broken(|row| row["confidence"] = json!("high")));
    assert!(broken(|row| row["candidates"] = json!(["only one"])));
    // Candidates come with an ambiguity and an ambiguity with candidates,
    // each named once.
    assert!(broken(|row| row["candidates"] = json!(["A.m()", "B.m()"])));
    assert!(broken(|row| row["detail"] = json!("ambiguous_overload")));
    assert!(broken(|row| {
        row["detail"] = json!("ambiguous_receiver");
        row["candidates"] = json!(["A.m()", "A.m()"]);
    }));
    assert!(broken(|row| drop(
        row.as_object_mut().unwrap().remove("resolution")
    )));
    assert!(broken(|row| drop(
        row.as_object_mut().unwrap().remove("referrer")
    )));
    // A declaration says what kind of thing it is, in the one vocabulary.
    assert!(broken(|row| drop(
        row.as_object_mut().unwrap().remove("referenced_type")
    )));
    assert!(broken(|row| row["referenced_type"] = json!("widget")));
    assert!(broken(|row| row["referenced_type"] = Value::Null));
    // A textual row claims no resolution.
    assert!(broken(|row| {
        row["tier"] = json!("textual");
        row["kind"] = json!("text");
    }));
    assert!(broken(|row| row["row"] = json!("declaration")));
}

#[test]
fn every_row_survives_a_round_trip_through_the_shared_row_types() {
    for (line, row) in OUTPUT.lines().skip(1).zip(ROWS.iter()) {
        assert_eq!(serde_json::to_string(row).unwrap(), line);
    }
}

#[test]
fn the_first_row_says_that_the_fixture_was_read_as_its_commit() {
    let first: Value = serde_json::from_str(OUTPUT.lines().next().unwrap()).unwrap();

    assert_eq!(first["row"], "provenance");
    assert_eq!(first["tree"]["state"], "commit", "{first}");
    assert_eq!(
        first["scope"],
        json!({
            "packageDirectories": ["force-app", "not-there", "plugins/Billing"],
            "unpackagedMetadata": ["unpackaged"],
        })
    );
    let others = OUTPUT.lines().skip(1);
    assert!(
        !others
            .into_iter()
            .any(|line| line.contains(r#""row":"provenance""#))
    );
}

#[test]
fn output_is_byte_identical_whatever_the_worker_count() {
    for jobs in ["1", "2", "8"] {
        assert_eq!(extract(&["--jobs", jobs]), *OUTPUT, "--jobs {jobs}");
    }
}

#[test]
fn rows_are_sorted_by_file_then_kind_then_position() {
    let key = |row: &Row| match row {
        Row::ScopeError(_) => (String::new(), 0, 0, 0),
        Row::ParseError(e) => (e.file.clone(), 1, e.line, 0),
        Row::SkippedFile(s) => (s.file.clone(), 2, 0, 0),
        Row::UnmodelledFile(u) => (u.file.clone(), 3, 0, 0),
        Row::Declaration(d) => (d.file.clone(), 4, d.start, 0),
        Row::Reference(r) => (r.file.clone(), 5, r.line, r.column),
        Row::UnsupportedElement(u) => (u.file.clone(), 6, u.line, u.column),
    };

    assert!(ROWS.windows(2).all(|pair| key(&pair[0]) <= key(&pair[1])));
}

// Scope.

#[test]
fn unpackaged_metadata_and_windows_style_paths_are_in_scope() {
    assert_eq!(declaration("ExtraTests").package, "unpackaged");
    assert_eq!(declaration("BillingPlugin").package, "plugins/Billing");
    assert_eq!(
        declaration("BillingPlugin").file,
        "plugins/Billing/classes/BillingPlugin.cls"
    );
}

#[test]
fn a_missing_package_directory_is_a_row_not_a_crash() {
    let errors: Vec<_> = ROWS
        .iter()
        .filter_map(|row| match row {
            Row::ScopeError(error) => Some((error.path.as_str(), error.reason)),
            _ => None,
        })
        .collect();

    assert_eq!(errors, [("not-there", ScopeErrorReason::Missing)]);
}

#[test]
fn files_outside_the_package_directories_are_never_read() {
    // The control: the stray class exists and names a class that is in scope.
    let stray = repository().join("fixtures/project/outside/Stray.cls");
    assert!(fs::read_to_string(stray).unwrap().contains("MathUtil"));

    assert!(!OUTPUT.contains("Stray"));
    assert!(!OUTPUT.contains("outside/"));
}

// Declarations.

#[test]
fn overloads_are_distinct_declarations() {
    let clamps: Vec<_> = declarations()
        .filter(|d| d.name == "clamp")
        .map(|d| (d.symbol.as_str(), d.kind))
        .collect();

    assert_eq!(
        clamps,
        [
            ("MathUtil.clamp(integer,integer,integer)", DeclType::Method),
            ("MathUtil.clamp(decimal,decimal,decimal)", DeclType::Method),
        ]
    );
}

#[test]
fn test_classes_and_methods_are_marked() {
    assert!(declaration("ShapeServiceTest").is_test);
    assert!(declaration("ShapeServiceTest.totalsAreas()").is_test);
    assert!(declaration("ShapeServiceTest.describesShapes()").is_test);
    assert!(declaration("ExtraTests").is_test);
    assert!(!declaration("ShapeService").is_test);
    // Not inherited: a helper class inside a test class is not itself marked.
    assert!(!declaration("ShapeServiceTest.FakeShape").is_test);
}

#[test]
fn annotations_that_expose_a_method_outside_apex_are_carried() {
    let exposed = declaration("ShapeService.describeAll(list<shape>)");

    assert_eq!(exposed.annotations, ["AuraEnabled"]);
}

#[test]
fn a_name_declared_in_two_package_directories_is_a_collision_that_keeps_both() {
    let invoices: Vec<_> = declarations()
        .filter(|d| d.symbol == "Invoice")
        .map(|d| (d.package.as_str(), d.collision))
        .collect();

    assert_eq!(invoices, [("force-app", true), ("plugins/Billing", true)]);
    assert!(!declaration("Invoice.total(boolean)").collision);
    assert!(!declaration("BillingPlugin").collision);
    // Their members collide too, each with its own row.
    for member in ["Invoice.Invoice(string)", "Invoice.seal()"] {
        let rows: Vec<_> = declarations().filter(|d| d.symbol == member).collect();
        assert_eq!(rows.len(), 2, "{member}");
        assert!(rows.iter().all(|d| d.collision), "{member}");
    }
}

#[test]
fn a_reference_to_a_colliding_name_resolves_without_choosing_between_them() {
    let reference = parsed("BillingPlugin.run(invoice)", RefKind::Type, "Invoice");

    assert_eq!(reference.resolution, Some(Resolution::Declaration));
    assert_eq!(
        declarations()
            .filter(|d| d.symbol == reference.referenced)
            .count(),
        2
    );
}

#[test]
fn a_constructor_declared_twice_is_one_candidate_not_an_ambiguity() {
    let construct = parsed("BillingPlugin.sealArea()", RefKind::Construct, "Invoice");

    assert_eq!(construct.referenced, "Invoice.Invoice(string)");
    assert_eq!(construct.detail, None);
    assert!(construct.candidates.is_empty());
}

#[test]
fn colliding_declarations_that_disagree_are_both_carried_downstream() {
    // One `Invoice.seal()` returns a Circle, the other a Square.
    let seal = parsed("BillingPlugin.sealArea()", RefKind::Call, "Invoice");
    let area = parsed("BillingPlugin.sealArea()", RefKind::Call, "area");

    assert_eq!(seal.referenced, "Invoice.seal()");
    assert_eq!(seal.detail, None);
    assert_eq!(area.resolution, Some(Resolution::Declaration));
    assert_eq!(area.detail, Some(Detail::AmbiguousReceiver));
    assert_eq!(area.candidates, ["Circle.area()", "Square.area()"]);
}

#[test]
fn collection_returns_that_differ_between_declarations_both_reach_the_next_call() {
    // `new Invoice('INV-2').seals().get(0).area()`: a list of circles in
    // one `Invoice`, of squares in the other.
    let seals = parsed("BillingPlugin.firstSealArea()", RefKind::Call, "Invoice");
    let area = parsed("BillingPlugin.firstSealArea()", RefKind::Call, "area");

    assert_eq!(seals.referenced, "Invoice.seals()");
    assert_eq!(area.detail, Some(Detail::AmbiguousReceiver));
    assert_eq!(area.candidates, ["Circle.area()", "Square.area()"]);
}

// Parsed references.

#[test]
fn a_call_binds_to_the_only_overload_with_its_argument_count() {
    let call = parsed("Circle.area()", RefKind::Call, "MathUtil.round");

    assert_eq!(call.referenced, "MathUtil.round(decimal,integer)");
    assert_eq!(call.detail, None);
}

#[test]
fn a_call_binds_to_the_overload_its_argument_types_select() {
    // clamp(total, 0, 1000) with a Decimal total: not clamp(Integer, ..).
    let by_type = parsed(
        "ShapeService.totalArea(list<shape>)",
        RefKind::Call,
        "MathUtil.clamp",
    );
    // describe(shape.area()): a Decimal is an Object, not a Shape.
    let by_exclusion = parsed(
        "ShapeService.describeAll(list<shape>)",
        RefKind::Call,
        "MathUtil.describe(o",
    );

    assert_eq!(
        by_type.referenced,
        "MathUtil.clamp(decimal,decimal,decimal)"
    );
    assert_eq!(by_exclusion.referenced, "MathUtil.describe(object)");
}

#[test]
fn a_call_that_fits_several_overloads_names_them_all_instead_of_guessing() {
    let call = parsed(
        "ShapeService.clampAny(string)",
        RefKind::Call,
        "MathUtil.clamp",
    );

    assert_eq!(call.referenced, "MathUtil.clamp");
    assert_eq!(call.resolution, Some(Resolution::Declaration));
    assert_eq!(call.detail, Some(Detail::AmbiguousOverload));
    assert_eq!(
        call.candidates,
        [
            "MathUtil.clamp(decimal,decimal,decimal)",
            "MathUtil.clamp(integer,integer,integer)"
        ]
    );
}

#[test]
fn a_call_on_a_value_of_unknown_type_is_reported_whatever_its_name() {
    // `quote.revise().discount()`: nothing declared is called `discount`.
    let call = parsed("ShapeService.price(shape)", RefKind::Call, "discount");

    assert_eq!(call.referenced, "discount");
    assert_eq!(call.resolution, Some(Resolution::Unresolved));
    assert_eq!(call.detail, Some(Detail::UnknownType));
    assert!(!declarations().any(|d| d.name == "discount"));
}

#[test]
fn a_call_on_an_element_of_a_collection_binds_through_the_element_type() {
    // `REGISTRY.get(name).area()` on a `Map<String, Shape>`.
    let call = parsed("ShapeService.lookupArea(string)", RefKind::Call, "Shape");

    assert_eq!(call.referenced, "Shape.area()");
}

#[test]
fn a_collection_spelled_with_its_namespace_returns_its_elements() {
    // `shapes.get(0).area()` on a `System.List<Shape>`.
    let call = parsed("ShapeService.firstArea(list<shape>)", RefKind::Call, "");

    assert_eq!(call.referenced, "Shape.area()");
}

#[test]
fn an_array_is_a_list_when_an_overload_is_chosen() {
    // `MathUtil.describe(new Shape[]{ .. })`: not `describe(Shape)`.
    let call = parsed(
        "ShapeServiceTest.describesShapes()",
        RefKind::Call,
        "MathUtil",
    );

    assert_eq!(call.referenced, "MathUtil.describe(object)");
}

#[test]
fn references_match_declarations_whatever_their_case() {
    // Written `shapeservice.describeall(..)` and `MATHUTIL.DESCRIBE(..)`.
    let lower = parsed(
        "ShapeServiceTest.describesShapes()",
        RefKind::Call,
        "ShapeService",
    );
    let upper = parsed(
        "ShapeService.describeAll(list<shape>)",
        RefKind::Call,
        "MathUtil.describe(o",
    );

    assert_eq!(lower.referenced, "ShapeService.describeAll(list<shape>)");
    assert_eq!(upper.referenced, "MathUtil.describe(object)");
    // The textual tier keeps the case the source has.
    let written: Vec<_> = textual_in("ShapeServiceTest.cls")
        .map(|r| r.referenced.as_str())
        .collect();
    assert!(written.contains(&"shapeservice"), "{written:?}");
    assert!(written.contains(&"describeall"), "{written:?}");
}

#[test]
fn a_call_through_an_interface_binds_to_the_interface_method() {
    let call = parsed("MathUtil.describe(shape)", RefKind::Call, "Shape.");

    assert_eq!(call.referenced, "Shape.describe(string)");
}

#[test]
fn implementers_and_subclasses_point_at_their_supertypes() {
    let supertypes = |kind| {
        let mut found: Vec<_> = references(Tier::Parsed)
            .filter(|r| r.kind == kind)
            .map(|r| (r.referrer.as_deref().unwrap(), r.referenced.as_str()))
            .collect();
        found.sort();
        found
    };

    assert_eq!(
        supertypes(RefKind::Implements),
        [
            ("BaseShape", "Shape"),
            ("ShapeServiceTest.FakeShape", "Shape")
        ]
    );
    assert_eq!(
        supertypes(RefKind::Extends),
        [("Circle", "BaseShape"), ("Square", "BaseShape")]
    );
}

#[test]
fn an_inherited_method_called_through_super_binds_to_the_superclass() {
    let call = parsed("Square.describe(string)", RefKind::Call, "BaseShape");

    assert_eq!(call.referenced, "BaseShape.describe(string)");
}

#[test]
fn constructor_calls_bind_to_a_constructor_or_to_the_class_that_declares_none() {
    let declared = parsed(
        "ShapeService.make(string,decimal)",
        RefKind::Construct,
        "Circle",
    );
    let delegated = parsed("Circle.Circle()", RefKind::Construct, "Circle");
    let implicit = parsed("AccountTrigger", RefKind::Construct, "AccountRepository");

    assert_eq!(declared.referenced, "Circle.Circle(decimal)");
    assert_eq!(delegated.referenced, "Circle.Circle(decimal)");
    assert_eq!(implicit.referenced, "AccountRepository");
}

#[test]
fn static_members_and_casts_are_references() {
    let constant = parsed("Circle.area()", RefKind::StaticAccess, "MathUtil");
    let enum_value = parsed("ShapeService.mode()", RefKind::StaticAccess, "MathUtil");
    let cast = parsed("ShapeService.make(string,decimal)", RefKind::Cast, "Shape");

    assert_eq!(constant.referenced, "MathUtil.PI_ISH");
    assert_eq!(enum_value.referenced, "MathUtil.Mode.UP");
    assert_eq!(cast.resolution, Some(Resolution::Declaration));
}

#[test]
fn a_static_field_used_without_its_class_is_a_reference_however_short() {
    let rate = parsed("Tariff.withTax(decimal)", RefKind::StaticAccess, "");
    let registry = parsed("ShapeService.lookupArea(string)", RefKind::StaticAccess, "");

    assert_eq!(rate.referenced, "Tariff.rate");
    assert_eq!(registry.referenced, "ShapeService.REGISTRY");
    // The control: the textual tier cannot see a name this short.
    assert!(!references(Tier::Textual).any(|r| r.referenced.eq_ignore_ascii_case("rate")));
}

#[test]
fn an_enum_value_in_a_switch_label_is_a_reference() {
    let value = parsed("MathUtil.sign(mode)", RefKind::StaticAccess, "");

    assert_eq!(value.referenced, "MathUtil.Mode.UP");
}

#[test]
fn objects_fields_and_labels_behind_a_platform_qualifier_are_references() {
    let named: Vec<_> = parsed_from("Tariff.canSeeTotals()")
        .chain(parsed_from("Tariff.title()"))
        .map(|r| (r.kind, r.referenced.as_str(), r.resolution))
        .collect();

    let unresolved = Some(Resolution::Unresolved);
    assert_eq!(
        named,
        [
            // Schema.SObjectType.Invoice__c.fields.Total__c.isAccessible()
            (RefKind::StaticAccess, "Invoice__c.Total__c", unresolved),
            // Schema.Invoice__c.Paid__c.getDescribe().isAccessible()
            (RefKind::StaticAccess, "Invoice__c.Paid__c", unresolved),
            // System.Label.Tariff_Title
            (RefKind::StaticAccess, "Label.Tariff_Title", unresolved),
        ]
    );
}

#[test]
fn a_call_on_a_value_of_an_installed_package_type_is_external() {
    let call = parsed(
        "ShapeService.price(shape)",
        RefKind::Call,
        "npsp.Pricing.Quote",
    );

    assert_eq!(call.referenced, "npsp.Pricing.Quote.revise");
    assert_eq!(call.resolution, Some(Resolution::External));
}

#[test]
fn installed_package_namespaces_are_external() {
    let call = parsed("ShapeService.price(shape)", RefKind::Call, "npsp.Pricing.q");
    let object = parsed(
        "AccountRepository.save(list<invoice__c>)",
        RefKind::DmlObject,
        "npsp__",
    );

    assert_eq!(call.referenced, "npsp.Pricing.quote");
    assert_eq!(call.resolution, Some(Resolution::External));
    assert_eq!(object.resolution, Some(Resolution::External));
}

#[test]
fn platform_types_and_methods_are_not_rows() {
    let platform = ["String", "Decimal", "List", "Math.max", "Assert.areEqual"];

    for reference in references(Tier::Parsed) {
        assert!(
            !platform.contains(&reference.referenced.as_str()),
            "{reference:?}"
        );
    }
}

#[test]
fn soql_names_its_object_fields_and_relationships() {
    let mut named: Vec<_> = parsed_from("AccountRepository.withContacts(set<id>,string)")
        .filter(|r| matches!(r.kind, RefKind::SoqlObject | RefKind::SoqlField))
        .map(|r| (r.kind, r.referenced.as_str()))
        .collect();
    named.sort_by_key(|(_, referenced)| *referenced);
    named.dedup();

    assert_eq!(
        named,
        [
            (RefKind::SoqlObject, "Account"),
            // `a.BillingCity`, without the alias.
            (RefKind::SoqlField, "Account.BillingCity"),
            (RefKind::SoqlField, "Account.Contacts"),
            (RefKind::SoqlField, "Account.Contacts.LastName"),
            (RefKind::SoqlField, "Account.Id"),
            (RefKind::SoqlField, "Account.Name"),
            (RefKind::SoqlField, "Account.Owner.Email"),
        ]
    );
}

#[test]
fn an_apex_expression_bound_into_a_query_is_still_walked() {
    let call = parsed(
        "AccountRepository.withContacts(set<id>,string)",
        RefKind::Call,
        "AccountRepository",
    );

    assert_eq!(call.referenced, "AccountRepository.normalise(string)");
}

#[test]
fn dml_names_the_object_of_its_operand_and_the_upsert_key() {
    let save = "AccountRepository.save(list<invoice__c>)";
    let mut objects: Vec<_> = parsed_from(save)
        .filter(|r| r.kind == RefKind::DmlObject)
        .map(|r| r.referenced.as_str())
        .collect();
    objects.sort();

    // A typed list, a `new` expression and a query, in turn.
    assert_eq!(objects, ["Invoice__c", "Invoice__c", "npsp__Gift__c"]);
    assert_eq!(
        parsed(save, RefKind::DmlField, "").referenced,
        "Invoice__c.External_Id__c"
    );
    // A variable's declared type.
    assert_eq!(
        parsed(
            "AccountRepository.rename(account,string)",
            RefKind::DmlObject,
            ""
        )
        .referenced,
        "Account"
    );
}

#[test]
fn the_fields_of_a_record_initialiser_are_references() {
    let field = parsed(
        "AccountRepository.save(list<invoice__c>)",
        RefKind::SobjectField,
        "",
    );

    assert_eq!(field.referenced, "npsp__Gift__c.npsp__Amount__c");
    assert_eq!(field.resolution, Some(Resolution::External));
}

#[test]
fn dml_through_the_database_class_names_its_object() {
    // Database.delete(invoices, false)
    let object = parsed(
        "AccountRepository.archive(list<invoice__c>)",
        RefKind::DmlObject,
        "",
    );

    assert_eq!(object.referenced, "Invoice__c");
}

#[test]
fn a_trigger_references_its_object() {
    let object = parsed("AccountTrigger", RefKind::TriggerObject, "");

    assert_eq!(object.referenced, "Account");
    assert_eq!(declaration("AccountTrigger").kind, DeclType::Trigger);
}

// The textual tier.

#[test]
fn a_name_inside_a_string_is_seen_by_the_textual_tier_only() {
    // 'SELECT Id FROM ApexClass WHERE Name = \'ShapeServiceTest\''
    let dynamic_soql = "AccountRepository.testClassId()";
    // Type.forName('Square')
    let dynamic_class = "ShapeService.make(string,decimal)";

    let textual = |referrer: &str, name: &str| {
        references(Tier::Textual)
            .filter(|r| r.referrer.as_deref() == Some(referrer) && r.referenced == name)
            .count()
    };
    assert_eq!(textual(dynamic_soql, "ShapeServiceTest"), 1);
    assert_eq!(parsed_from(dynamic_soql).count(), 0);
    // In a comment, in the string, and in `new Square(size)`: the last is
    // the only one syntax can see.
    assert_eq!(textual(dynamic_class, "Square"), 3);
    assert_eq!(
        parsed_from(dynamic_class)
            .filter(|r| r.referenced.starts_with("Square"))
            .count(),
        1
    );
}

#[test]
fn every_file_in_scope_is_searched_not_only_apex() {
    let found = |file| {
        textual_in(file)
            .map(|r| (r.referenced.as_str(), r.referrer.as_deref()))
            .collect::<Vec<_>>()
    };

    // The permission set is a declaration: what is in its file is in it,
    // and its own name, which its label repeats, is searched for too.
    assert_eq!(
        found("Shapes.permissionset-meta.xml"),
        [("ShapeService", Some("Shapes")), ("Shapes", Some("Shapes"))]
    );
    assert_eq!(
        found("shapeList.js"),
        [
            ("describeAll", Some("shapeList")),
            ("ShapeService", Some("shapeList")),
            ("describeAll", Some("shapeList")),
            ("ShapeList", Some("shapeList")),
            ("describeAll", Some("shapeList")),
            // A parameter that happens to be called like the permission
            // set `Shapes`: the textual tier is a grep, and says so.
            ("shapes", Some("shapeList"))
        ]
    );
}

#[test]
fn a_utf16_file_is_searched_like_any_other_text() {
    let found: Vec<_> = textual_in("Billing.permissionset-meta.xml")
        .map(|r| (r.referenced.as_str(), r.line))
        .collect();

    // The second is the permission set's own name, in its label.
    assert_eq!(found, [("BillingPlugin", 4), ("Billing", 7)]);
}

#[test]
fn a_file_that_cannot_be_searched_is_a_row_not_a_silence() {
    let skipped: Vec<_> = ROWS
        .iter()
        .filter_map(|row| match row {
            Row::SkippedFile(skipped) => Some((skipped.file.as_str(), skipped.reason)),
            _ => None,
        })
        .collect();

    assert_eq!(
        skipped,
        [(
            "force-app/main/default/staticresources/seal.png",
            SkipReason::Binary
        )]
    );
    // The control: the file does hold a declared name, and it was not matched.
    let image =
        repository().join("fixtures/project/force-app/main/default/staticresources/seal.png");
    let bytes = fs::read(image).unwrap();
    assert!(bytes.windows(13).any(|window| window == b"BillingPlugin"));
    assert_eq!(textual_in("seal.png").count(), 0);
}

#[test]
fn short_names_are_matched_only_when_the_allow_list_names_them() {
    let matched = |name: &str| {
        references(Tier::Textual)
            .filter(|r| r.referenced.eq_ignore_ascii_case(name))
            .count()
    };

    // `Money` is in the fixture's allow-list; `Shape` is as short and is not.
    assert!(matched("Money") > 0);
    assert_eq!(matched("Shape"), 0);
    assert!(declarations().any(|d| d.name == "Shape"));
}

// Parse failures.

#[test]
fn a_file_with_a_syntax_error_is_reported_and_still_read() {
    let errors: Vec<_> = ROWS
        .iter()
        .filter_map(|row| match row {
            Row::ParseError(error) if error.file.ends_with(".cls") => {
                Some((error.file.as_str(), error.line))
            }
            _ => None,
        })
        .collect();

    assert_eq!(errors, [("force-app/main/default/classes/Broken.cls", 6)]);
    // Both tiers still see the part the grammar recovered.
    assert!(textual_in("Broken.cls").any(|r| r.referenced == "MathUtil"));
    assert_eq!(
        parsed("Broken.fine()", RefKind::Call, "MathUtil").referenced,
        "MathUtil.round(decimal)"
    );
}

#[test]
fn flow_fixture_covers_records_actions_subflows_and_lwc_screens() {
    assert_eq!(declaration("InvoiceAfterSave").kind, DeclType::Flow);
    assert_eq!(
        parsed("InvoiceAfterSave", RefKind::FlowAction, "").referenced,
        "FlowInvoiceAction"
    );
    assert_eq!(
        parsed("InvoiceAfterSave", RefKind::FlowSubflow, "").referenced,
        "InvoiceChild"
    );
    assert_eq!(
        parsed("InvoiceScreen", RefKind::FlowComponent, "").referenced,
        "invoicePanel"
    );
    for field in [
        "Invoice__c.Paid__c",
        "Invoice__c.Total__c",
        "Invoice__c.Id",
        "Invoice__c.OwnerId",
    ] {
        assert!(
            parsed_from("InvoiceAfterSave")
                .any(|r| r.kind == RefKind::FlowField && r.referenced == field)
        );
    }
}
#[test]
fn lwc_fixture_covers_apex_schema_labels_and_a_template_child() {
    assert_eq!(declaration("invoicePanel").kind, DeclType::Lwc);
    assert_eq!(
        parsed("invoicePanel", RefKind::LwcApex, "").referenced,
        "FlowInvoiceAction.caption(id)"
    );
    assert_eq!(
        parsed("invoicePanel", RefKind::LwcSchema, "").referenced,
        "Invoice__c.Total__c"
    );
    assert_eq!(
        parsed("invoicePanel", RefKind::LwcLabel, "").referenced,
        "Label.Invoice_Title"
    );
    assert_eq!(
        parsed("invoicePanel", RefKind::LwcComponent, "").referenced,
        "invoiceChild"
    );
}
#[test]
fn golden_covers_every_flow_and_lwc_reference_kind() {
    let schema: Value = serde_json::from_str(include_str!("../../../schema/refs.schema.json"))
        .expect("schema is JSON");
    let rows: Vec<Value> = GOLDEN
        .lines()
        .map(|line| serde_json::from_str(line).expect("golden row is JSON"))
        .collect();
    let missing: Vec<_> = schema["$defs"]["flow_lwc_reference_kind"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|kind| {
            !rows
                .iter()
                .any(|r| r["tier"] == "parsed" && r["kind"] == **kind)
        })
        .collect();
    assert!(
        missing.is_empty(),
        "missing golden reference kinds: {missing:?}"
    );
}

#[test]
fn flow_fixture_preserves_an_untyped_reference() {
    let reference = parsed(
        "InvoiceGlobals",
        RefKind::FlowField,
        "Fetch.record.Total__c",
    );
    assert_eq!(reference.resolution, Some(Resolution::Unresolved));
    assert_eq!(reference.detail, Some(Detail::UnknownType));
}

#[test]
fn flow_fixture_diagnoses_unknown_structure_and_keeps_known_references() {
    // The unknown structure is well-formed XML: it is an element that was
    // not read, with its path, and not a parse error.
    let diagnostics: Vec<_> = ROWS
        .iter()
        .filter_map(|row| match row {
            Row::UnsupportedElement(e) if e.file.ends_with("InvoiceGlobals.flow-meta.xml") => {
                Some((e.element.as_str(), e.reason, e.line))
            }
            Row::ParseError(e) if e.file.ends_with("InvoiceGlobals.flow-meta.xml") => {
                Some(("parse_error", UnsupportedReason::NotInterpreted, e.line))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        diagnostics,
        [(
            "futureInvoiceSettings",
            UnsupportedReason::NotInterpreted,
            7
        )]
    );
    assert_eq!(
        parsed("InvoiceGlobals", RefKind::FlowLabel, "").referenced,
        "Label.Invoice_Title"
    );
}

#[test]
fn malformed_flow_fixture_keeps_its_declaration_diagnostic_and_textual_tier() {
    assert_eq!(declaration("BrokenInvoice").kind, DeclType::Flow);
    assert!(ROWS.iter().any(
        |r| matches!(r, Row::ParseError(e) if e.file.ends_with("BrokenInvoice.flow-meta.xml"))
    ));
    assert!(textual_in("BrokenInvoice.flow-meta.xml").any(|r| r.referenced == "FlowInvoiceAction"));
}

#[test]
fn declarations_record_scope_provenance_without_guessing_from_the_path() {
    let extra = serde_json::to_value(declaration("ExtraTests")).unwrap();
    let packaged = serde_json::to_value(declaration("BillingPlugin")).unwrap();
    assert_eq!(extra["scope"], "unpackagedMetadata");
    assert_eq!(packaged["scope"], "packageDirectories");
}

#[test]
fn the_rows_are_written_whole_to_out_and_a_reader_that_stops_early_is_not_an_error() {
    let project = Committed::new();
    let out_file = project.0.with_extension("refs.jsonl");
    let lines = sf_impact_extract::extract(&project.0, 1, false).unwrap();
    let expected = lines.join("\n") + "\n";
    let extract = || {
        let mut command = Command::new(env!("CARGO_BIN_EXE_sf-impact-extract"));
        command.arg("--root").arg(&project.0);
        command.args(["--format", "jsonl"]);
        command
    };

    let to_stdout = extract().output().unwrap();
    let to_file = extract().arg("--out").arg(&out_file).output().unwrap();
    let written = fs::read_to_string(&out_file);
    let _ = fs::remove_file(&out_file);
    let mut child = extract()
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let stopped_early = child.wait_with_output().unwrap();

    assert!(to_stdout.status.success(), "{to_stdout:?}");
    assert!(
        to_stdout.stdout == expected.as_bytes(),
        "stdout differs from the library's rows"
    );
    assert!(to_file.status.success(), "{to_file:?}");
    assert!(to_file.stdout.is_empty(), "{to_file:?}");
    assert!(expected.len() > 65536, "larger than a pipe's buffer");
    assert!(
        written.unwrap() == expected,
        "--out differs from the library's rows"
    );
    assert_eq!(stopped_early.status.code(), Some(0), "{stopped_early:?}");
    assert_eq!(
        stopped_early.stderr, to_file.stderr,
        "stderr is as when nothing stops"
    );
}
