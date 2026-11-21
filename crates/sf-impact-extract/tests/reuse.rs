//! Extract's reuse of its previous pack (`--previous`): a run that reuses a
//! pack writes the pack that a run without one writes, byte for byte, its
//! manifest included. Each test makes a small repository in a temporary
//! directory, runs the `sf-impact-extract` binary over a sequence of
//! commits with one pack kept between runs, and holds every run to a fresh
//! run of the same commit. Whether a run took its fast path is checked
//! too, by the note extract writes on standard error, so that a test of
//! the fast path cannot pass by missing it. Where the stages downstream
//! read a pack that a run left cut short, they are run too, by their
//! libraries, which are this crate's dev-dependencies: what their binaries
//! do with the error is one line on standard error and exit status 1.

#[path = "../../sf-impact-common/tests/support/pack.rs"]
mod pack;

use sf_impact_common::pack::Pack;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::{env, fs};

const PROJECT: &str = r#"{"packageDirectories": [{"path": "app"}]}"#;
const RATES: &str = "app/classes/Rates.cls";
const SHIPPING: &str = "app/classes/Shipping.cls";

/// What the tests start from: classes that call each other, a test, a
/// file in scope with no rows, a lookup field, and a field whose type no
/// row names.
fn files() -> Vec<(String, String)> {
    vec![
        ("sfdx-project.json".into(), PROJECT.into()),
        (
            RATES.into(),
            "public class Rates {\n    public static Map<String, List<Decimal>> table;\n\
             \x20   public static Integer quote(Integer weight) {\n        return weight * 2;\n    }\n}\n"
                .into(),
        ),
        (
            SHIPPING.into(),
            "public class Shipping {\n    public Integer cost() {\n        return Rates.quote(3);\n\
             \x20   }\n}\n"
                .into(),
        ),
        (
            "app/classes/ShippingTest.cls".into(),
            "@IsTest\nclass ShippingTest {\n    @IsTest\n    static void costs() {\n\
             \x20       System.assertEquals(6, new Shipping().cost());\n    }\n}\n"
                .into(),
        ),
        ("app/classes/notes.txt".into(), "Nothing is declared in here.\n".into()),
        (
            "app/objects/Parcel__c/fields/Shipment__c.field-meta.xml".into(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <CustomField xmlns=\"http://soap.sforce.com/2006/04/metadata\">\n\
             \x20   <fullName>Shipment__c</fullName>\n    <referenceTo>Shipment__c</referenceTo>\n\
             \x20   <relationshipName>Parcels</relationshipName>\n    <type>MasterDetail</type>\n\
             </CustomField>\n"
                .into(),
        ),
    ]
}

/// A repository in a temporary directory, removed on drop, and the pack
/// that is kept between its runs, outside it.
struct Repo {
    dir: PathBuf,
}

/// What one run wrote, and what it said.
struct Run {
    pack: Vec<u8>,
    stderr: String,
}

impl Run {
    /// Whether the run took its fast path: reused the previous pack's rows
    /// for the files that did not change.
    fn reused(&self) -> bool {
        self.stderr.contains("the previous pack was reused:")
    }

    /// Whether the run took its partial path: reused what the files that
    /// did not change declare, and made the rows of every file again.
    fn partial(&self) -> bool {
        self.stderr
            .contains("the previous pack was reused in part:")
    }

    /// Whether the run reused nothing of the previous pack.
    fn cold(&self) -> bool {
        !self.reused() && !self.partial()
    }
}

fn git(dir: &Path) -> Command {
    let mut git = Command::new("git");
    git.current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "A Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_AUTHOR_DATE", "2026-01-01T00:00:00Z")
        .env("GIT_COMMITTER_NAME", "A Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_DATE", "2026-01-01T00:00:00Z");
    git
}

fn succeeds(command: &mut Command) -> Output {
    let output = command.output().expect("the command starts");
    assert!(output.status.success(), "{command:?} failed: {output:?}");
    output
}

fn extract(args: &[&str]) -> Output {
    let binary = env!("CARGO_BIN_EXE_sf-impact-extract");
    Command::new(binary)
        .args(args)
        .output()
        .expect("extract starts")
}

impl Repo {
    fn new(name: &str) -> Repo {
        Repo::of(name, &files(), &[])
    }

    /// `files` committed in a new repository, made with `init` added to
    /// `git init`.
    fn of(name: &str, files: &[(String, String)], init: &[&str]) -> Repo {
        let dir = env::temp_dir().join(format!(
            "sf-impact-extract-reuse-{}-{name}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        let repo = Repo { dir };
        fs::create_dir_all(repo.root()).unwrap();
        fs::create_dir_all(repo.dir.join("kept")).unwrap();
        for (path, content) in files {
            repo.write(path, content);
        }
        let mut command = git(&repo.root());
        succeeds(
            command
                .args(["init", "--quiet", "--initial-branch=main"])
                .args(init),
        );
        repo.commit();
        repo
    }

    fn root(&self) -> PathBuf {
        self.dir.join("repo")
    }

    /// Where the pack is kept between runs: outside the repository.
    fn kept(&self) -> PathBuf {
        self.dir.join("kept/refs.pack")
    }

    fn write(&self, path: &str, content: &str) {
        let path = self.root().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.root().join(path)).unwrap()
    }

    fn edit(&self, path: &str, from: &str, to: &str) {
        let content = self.read(path);
        assert!(content.contains(from), "{path} has no `{from}`");
        self.write(path, &content.replacen(from, to, 1));
    }

    fn remove(&self, path: &str) {
        fs::remove_file(self.root().join(path)).unwrap();
    }

    fn commit(&self) {
        succeeds(git(&self.root()).args(["add", "--all"]));
        succeeds(git(&self.root()).args(["commit", "--quiet", "--allow-empty", "-m", "step"]));
    }

    /// A run that reuses the kept pack and replaces it.
    fn reuse(&self) -> Run {
        let (root, kept) = (self.root(), self.kept());
        let (root, kept) = (root.to_str().unwrap(), kept.to_str().unwrap());
        let output = extract(&["--root", root, "--previous", kept, "--out", kept]);
        assert!(output.status.success(), "{output:?}");
        // What it writes is the pack alone, as a fresh run's is.
        assert!(output.stdout.is_empty(), "{output:?}");
        Run {
            pack: fs::read(self.kept()).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }

    /// A run without a previous pack, to another file.
    fn fresh(&self) -> Vec<u8> {
        let out = self.dir.join("fresh.pack");
        let output = extract(&[
            "--root",
            self.root().to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]);
        assert!(output.status.success(), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let pack = fs::read(&out).unwrap();
        fs::remove_file(&out).unwrap();
        pack
    }

    /// A run that reuses the kept pack, held to a fresh run of the same
    /// commit, byte for byte.
    fn reuse_as_fresh(&self, step: &str) -> Run {
        let run = self.reuse();
        same(&run.pack, &self.fresh(), step);
        run
    }

    /// Asserts that sf-impact-changed and sf-impact-tests refuse `refs`,
    /// in one line that names the refs, after showing that they
    /// accept the fresh pack of the same commit, the one before it being
    /// the delta's base.
    fn refused_downstream(&self, refs: &[u8], what: &str) {
        let base = sf_impact_delta::Base::Commit("HEAD~1".into());
        let delta = sf_impact_delta::delta(&self.root(), false, &base, "HEAD");
        let delta = delta.expect("delta").join("\n") + "\n";
        let changed = |refs: &[u8]| sf_impact_changed::changed(delta.as_bytes(), refs, false);
        let config = sf_impact_tests::Config::default();
        let tests = |refs: &[u8], changed: &[u8]| {
            sf_impact_tests::impact(refs, Some(changed), &[], 2, &config, false)
        };

        // The control: the fresh pack is read.
        let fresh = self.fresh();
        let lines = changed(&fresh).unwrap_or_else(|e| panic!("{what}: {e}"));
        let lines = lines.join("\n") + "\n";
        tests(&fresh, lines.as_bytes()).unwrap_or_else(|e| panic!("{what}: {e}"));

        for (stage, refused) in [
            ("changed", changed(refs).err()),
            ("tests", tests(refs, lines.as_bytes()).err()),
        ] {
            let message = refused.unwrap_or_else(|| panic!("{what}, {stage}: accepted"));
            assert_eq!(message.lines().count(), 1, "{what}, {stage}: {message}");
            // A pack cut inside its first line is not known to be a pack,
            // and is refused as refs that are not refs.jsonl either.
            assert!(message.contains("refs"), "{what}, {stage}: {message}");
            assert!(
                message.contains("re-run sf-impact-extract") || message.contains("refs pack"),
                "{what}, {stage}: {message}"
            );
        }
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// Two packs are the same bytes. Where they are not, the message says
/// first where their content differs.
fn same(reused: &[u8], fresh: &[u8], step: &str) {
    if reused == fresh {
        return;
    }
    let (reused, fresh) = (Pack::decode(reused), Pack::decode(fresh));
    let (reused, fresh) = (reused.expect("a pack"), fresh.expect("a pack"));
    let (a, b) = (reused.to_jsonl(), fresh.to_jsonl());
    if let Some(at) = (0..a.len().max(b.len())).find(|at| a.get(*at) != b.get(*at)) {
        panic!(
            "{step}: line {} differs:\n reused: {:?}\n  fresh: {:?}",
            at + 1,
            a.get(at),
            b.get(at)
        );
    }
    assert_eq!(
        reused.manifest, fresh.manifest,
        "{step}: the manifests differ"
    );
    panic!("{step}: the packs differ in their bytes alone");
}

/// The rows of one file, as refs.jsonl lines.
fn rows_of(pack: &[u8], file: &str) -> Vec<String> {
    let lines = Pack::decode(pack).unwrap().to_jsonl();
    let file = format!(r#""file":"{file}""#);
    lines
        .into_iter()
        .filter(|line| line.contains(&file))
        .collect()
}

#[test]
fn a_pack_has_a_manifest_of_every_file_in_scope_and_the_blob_that_was_read() {
    let repo = Repo::new("manifest");

    let pack = Pack::decode(&repo.fresh()).unwrap();

    let manifest = pack
        .manifest
        .expect("a pack of a repository has a manifest");
    let paths: Vec<&str> = manifest.files.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "app/classes/Rates.cls",
            "app/classes/Shipping.cls",
            "app/classes/ShippingTest.cls",
            "app/classes/notes.txt",
            "app/objects/Parcel__c/fields/Shipment__c.field-meta.xml",
        ]
    );
    for entry in &manifest.files {
        let spec = format!("HEAD:{}", entry.path);
        let id = succeeds(git(&repo.root()).args(["rev-parse", &spec])).stdout;
        let hex: String = entry.blob.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, String::from_utf8(id).unwrap().trim(), "{}", entry.path);
    }
    // Every type, in the order its file declares it, with what resolving a
    // reference to it needs.
    let symbols: Vec<&str> = manifest.types.iter().map(|t| t.symbol.as_str()).collect();
    assert_eq!(symbols, ["Rates", "Shipping", "ShippingTest"]);
    assert_eq!(manifest.types[0].methods[0].params, ["Integer"]);
    assert_eq!(manifest.relationships.len(), 1);
}

#[test]
fn every_step_of_the_invalidation_chain_writes_the_fresh_pack() {
    let repo = Repo::new("chain");
    assert!(
        !repo.reuse_as_fresh("prime").reused(),
        "nothing to reuse yet"
    );

    // A comment on the declaration line.
    repo.edit(
        SHIPPING,
        "public class Shipping",
        "/* priced */ public class Shipping",
    );
    repo.commit();
    assert!(repo.reuse_as_fresh("comment").reused());

    // A method added: what the file declares has changed.
    repo.edit(
        SHIPPING,
        "    public Integer cost()",
        "    public static Integer added() { return 42; }\n    public Integer cost()",
    );
    repo.commit();
    let run = repo.reuse_as_fresh("add method");
    assert!(run.partial(), "{}", run.stderr);

    // The class renamed, and its file with it.
    let renamed = "app/classes/ShippingRenamed.cls";
    let content = repo
        .read(SHIPPING)
        .replace("class Shipping", "class ShippingRenamed");
    repo.remove(SHIPPING);
    repo.write(renamed, &content);
    repo.commit();
    let run = repo.reuse_as_fresh("rename class");
    assert!(run.partial(), "{}", run.stderr);

    // A comment again, after what is declared has changed.
    repo.edit(renamed, "/* priced */", "/* priced, renamed */");
    repo.commit();
    assert!(repo.reuse_as_fresh("comment after rename").reused());
}

/// Whether a string is in a pack: as the whole of a value of one of its
/// rows or of its manifest, which is what its string table holds.
fn has_string(pack: &[u8], string: &str) -> bool {
    let pack = Pack::decode(pack).unwrap();
    let quoted = format!("{string:?}");
    let manifest = format!("{:?}", pack.manifest);
    pack.to_jsonl().iter().any(|line| line.contains(&quoted)) || manifest.contains(&quoted)
}

/// The bytes of a pack's string table section.
fn strings(pack: &[u8]) -> &[u8] {
    let (_, at) = pack::sections(pack)
        .into_iter()
        .find(|(kind, _)| *kind == 1)
        .expect("a pack has a string table");
    &pack[at]
}

/// One edit that leaves what every file declares as it was.
struct StringsOnly<'a> {
    step: &'a str,
    /// A row the edit adds, by parts of it: none before it, one after.
    row: &'a [&'a str],
    /// The string the edit adds to the table, if it adds one.
    new: Option<&'a str>,
    /// Whether the edit changes the string table, by what it holds or by
    /// its order, as a premise. Not asserted where it is not one.
    renumbers: bool,
    edit: &'a dyn Fn(),
}

#[test]
fn an_edit_that_changes_only_strings_writes_the_fresh_pack() {
    let mut files = files();
    // A bundle declared in its script, whose template, before the script
    // in path order, has no rows yet.
    let template = "app/lwc/parcelview/parcelview.html";
    files.push((
        template.into(),
        "<template>Nothing to see</template>\n".into(),
    ));
    files.push((
        "app/lwc/parcelview/parcelview.js".into(),
        "export default class extends LightningElement {}\n".into(),
    ));
    let repo = Repo::of("strings", &files, &[]);
    let mut before = repo.reuse().pack;

    let steps = [
        StringsOnly {
            step: "a textual word",
            row: &[r#""referenced":"SHIPPING""#],
            new: Some("SHIPPING"),
            renumbers: true,
            edit: &|| {
                repo.edit(
                    SHIPPING,
                    "    public Integer cost()",
                    "    // SHIPPING\n    public Integer cost()",
                )
            },
        },
        // A referrer is a declared symbol or the name of a bundle, which a
        // declaration row has already. A new one moves its first use: here
        // from the script's declaration to the template, which is before it.
        StringsOnly {
            step: "a referrer name",
            row: &[r#""referrer":"parcelview""#],
            new: None,
            renumbers: true,
            edit: &|| repo.edit(template, "Nothing to see", "Shipping"),
        },
        StringsOnly {
            step: "a referrer of another declaration",
            row: &[
                r#""referenced":"Shipping""#,
                r#""referrer":"Shipping.cost()""#,
            ],
            new: None,
            renumbers: false,
            edit: &|| {
                repo.edit(
                    SHIPPING,
                    "return Rates.quote(3);",
                    "return Rates.quote(3); // Shipping",
                )
            },
        },
        // A diagnostic message: an element of the field that is not read,
        // which declares nothing and relates the field to nothing, named
        // by its path, which no row or manifest of the pack has yet.
        StringsOnly {
            step: "a diagnostic message",
            row: &[
                r#""row":"unsupported_element""#,
                r#""element":"shippingNotes""#,
                r#""reason":"not_interpreted""#,
            ],
            new: Some("shippingNotes"),
            renumbers: true,
            edit: &|| {
                repo.edit(
                    "app/objects/Parcel__c/fields/Shipment__c.field-meta.xml",
                    "    <type>MasterDetail</type>\n",
                    "    <type>MasterDetail</type>\n    <shippingNotes>Fragile</shippingNotes>\n",
                )
            },
        },
        // A file whose type is not modelled, and its suffix.
        StringsOnly {
            step: "an unmodelled file",
            row: &[r#""suffix":"layout""#],
            new: Some("layout"),
            renumbers: true,
            edit: &|| {
                repo.write(
                    "app/layouts/Parcel__c-Parcel Layout.layout-meta.xml",
                    "<Layout/>\n",
                )
            },
        },
    ];
    for StringsOnly {
        step,
        row,
        new,
        renumbers,
        edit,
    } in steps
    {
        let has_row = |pack: &[u8]| {
            let lines = Pack::decode(pack).unwrap().to_jsonl();
            lines
                .iter()
                .any(|line| row.iter().all(|part| line.contains(part)))
        };
        assert!(!has_row(&before), "{step}: the premise");
        if let Some(new) = new {
            assert!(!has_string(&before, new), "{step}: the premise");
        }
        edit();
        repo.commit();

        let run = repo.reuse_as_fresh(step);

        assert!(run.reused(), "{step}: {}", run.stderr);
        assert!(has_row(&run.pack), "{step}");
        if renumbers {
            assert_ne!(strings(&before), strings(&run.pack), "{step}");
        }
        before = run.pack;
    }
}

#[test]
fn a_file_with_no_rows_and_a_string_only_the_manifest_has_survive_a_reuse() {
    let repo = Repo::new("manifest-only");
    let primed = repo.reuse().pack;
    // The premises: a file in scope with no rows, and a type name that no
    // row has.
    let rows = Pack::decode(&primed).unwrap().to_jsonl().join("\n");
    assert!(rows_of(&primed, "app/classes/notes.txt").is_empty());
    let manifest = Pack::decode(&primed).unwrap().manifest.unwrap();
    let field = &manifest.types[0].fields[0];
    assert_eq!(field.ty, "Map<String,List<Decimal>>");
    assert!(!rows.contains(&field.ty));

    repo.edit(
        SHIPPING,
        "public class Shipping",
        "/* edited */ public class Shipping",
    );
    repo.commit();
    let run = repo.reuse_as_fresh("edit");

    assert!(run.reused(), "{}", run.stderr);
    let manifest = Pack::decode(&run.pack).unwrap().manifest.unwrap();
    assert!(
        manifest
            .files
            .iter()
            .any(|e| e.path == "app/classes/notes.txt")
    );
    assert_eq!(manifest.types[0].fields[0].ty, "Map<String,List<Decimal>>");
}

#[test]
fn a_signature_changed_in_one_file_changes_how_another_resolves() {
    let repo = Repo::new("signature");
    let before = repo.reuse().pack;

    repo.edit(RATES, "quote(Integer weight)", "quote(String weight)");
    repo.commit();
    let run = repo.reuse_as_fresh("signature");
    assert!(run.partial(), "{}", run.stderr);

    // The premise: Shipping's call resolves otherwise, and Shipping did
    // not change.
    assert_ne!(rows_of(&before, SHIPPING), rows_of(&run.pack, SHIPPING));
}

#[test]
fn a_file_is_matched_by_its_exact_path_never_by_a_part_of_one() {
    let mut files = files();
    // Each path is part of the other's: `.cls` of `.cls-meta.xml`, and
    // `app/` of `subapp/`.
    files[0].1 = r#"{"packageDirectories": [{"path": "app"}, {"path": "subapp"}]}"#.into();
    files.push((
        "app/classes/Shipping.cls-meta.xml".into(),
        "<ApexClass><!-- Shipping costs --></ApexClass>\n".into(),
    ));
    files.push((
        "subapp/classes/Shipping.cls".into(),
        "public class Shipping {\n    // Rates\n}\n".into(),
    ));
    let repo = Repo::of("exact-path", &files, &[]);
    repo.reuse();

    repo.edit(
        SHIPPING,
        "public class Shipping",
        "/* a */ public class Shipping",
    );
    repo.commit();
    assert!(repo.reuse_as_fresh("the class").reused());

    repo.edit("app/classes/Shipping.cls-meta.xml", "costs", "costs, again");
    repo.commit();
    assert!(repo.reuse_as_fresh("its meta file").reused());

    repo.edit("subapp/classes/Shipping.cls", "// Rates", "// Rates, again");
    repo.commit();
    assert!(repo.reuse_as_fresh("the class of another package").reused());
}

#[test]
fn a_deleted_file_and_an_added_one_write_the_fresh_pack() {
    let repo = Repo::new("added-deleted");
    repo.reuse();

    // Neither declares anything.
    repo.remove("app/classes/notes.txt");
    repo.write("app/classes/more.txt", "See Shipping and Rates.\n");
    repo.commit();
    assert!(repo.reuse_as_fresh("text files").reused());

    // Both declare something.
    repo.remove("app/classes/ShippingTest.cls");
    repo.write(
        "app/classes/Insurance.cls",
        "public class Insurance {\n    Integer x = Rates.quote(1);\n}\n",
    );
    repo.commit();
    let run = repo.reuse_as_fresh("classes");
    assert!(run.partial(), "{}", run.stderr);

    // A class deleted, and nothing else changed.
    repo.remove("app/classes/Insurance.cls");
    repo.commit();
    let run = repo.reuse_as_fresh("a class deleted");
    assert!(run.partial(), "{}", run.stderr);
    assert!(
        run.stderr
            .contains("Insurance.cls, which declared things, is gone")
    );
}

#[test]
fn more_than_sixteen_changed_files_write_the_fresh_pack() {
    let mut files = files();
    let classes: Vec<(String, String)> = (0..20)
        .map(|n| {
            let path = format!("app/classes/Parcel{n:02}.cls");
            let content =
                format!("public class Parcel{n:02} {{\n    Integer w = Rates.quote({n});\n}}\n");
            (path, content)
        })
        .collect();
    files.extend(classes.iter().cloned());
    let repo = Repo::of("many", &files, &[]);
    repo.reuse();

    for (path, _) in &classes {
        repo.edit(path, "public class", "/* weighed */ public class");
    }
    repo.commit();
    let run = repo.reuse_as_fresh("twenty files");

    assert!(run.reused(), "{}", run.stderr);
}

#[test]
fn a_bundle_declared_in_another_of_its_files_writes_the_fresh_pack() {
    let mut files = files();
    files.push((
        "app/lwc/quote/quote.html".into(),
        "<template>Rates</template>\n".into(),
    ));
    let repo = Repo::of("bundle", &files, &[]);
    repo.reuse();

    // The bundle is now declared in its script, and its template, which
    // did not change, declares nothing.
    repo.write("app/lwc/quote/quote.js", "export default class Quote {}\n");
    repo.commit();
    let run = repo.reuse_as_fresh("script added");
    assert!(run.partial(), "{}", run.stderr);
}

#[test]
fn a_change_of_config_or_of_scope_is_a_cold_run() {
    let repo = Repo::new("config-scope");
    repo.reuse();

    repo.write("sf-impact.json", r#"{"textualMinLength": 4}"#);
    repo.commit();
    let run = repo.reuse_as_fresh("config");
    assert!(run.cold(), "{}", run.stderr);

    repo.write(
        "sfdx-project.json",
        r#"{"packageDirectories": [{"path": "app"}, {"path": "more"}]}"#,
    );
    repo.write("more/classes/More.cls", "public class More {\n}\n");
    repo.commit();
    let run = repo.reuse_as_fresh("scope");
    assert!(run.cold(), "{}", run.stderr);
}

#[test]
fn a_previous_pack_that_is_missing_damaged_or_foreign_is_a_cold_run() {
    let repo = Repo::new("damaged");
    let primed = repo.reuse().pack;
    repo.edit(
        SHIPPING,
        "public class Shipping",
        "/* edited */ public class Shipping",
    );
    repo.commit();
    let fresh = repo.fresh();

    let header = primed.iter().position(|b| *b == b'\n').unwrap() + 1;
    let mut previous: Vec<(String, Option<Vec<u8>>)> = vec![
        ("missing".into(), None),
        ("empty".into(), Some(Vec::new())),
        ("corrupt".into(), Some(pack::corrupted(&primed, 2, false))),
        (
            "of another version".into(),
            Some(pack::of_version(&primed, 9)),
        ),
        (
            "of another release".into(),
            Some(pack::resealed(
                String::from_utf8_lossy(&primed)
                    .replacen(r#""version":""#, r#""version":"0.0.0-other"#, 1)
                    .into_bytes(),
            )),
        ),
    ];
    // Cut short as an interrupted write leaves it.
    for at in [
        1,
        header / 2,
        header,
        header + 5,
        primed.len() / 2,
        primed.len() - 1,
    ] {
        previous.push((format!("cut at {at}"), Some(primed[..at].to_vec())));
    }
    for (what, bytes) in previous {
        match &bytes {
            Some(bytes) => fs::write(repo.kept(), bytes).unwrap(),
            None => fs::remove_file(repo.kept()).unwrap(),
        }
        if what.starts_with("cut") {
            // What reads the pack downstream refuses it loudly.
            assert!(Pack::decode(bytes.as_ref().unwrap()).is_err(), "{what}");
            repo.refused_downstream(bytes.as_ref().unwrap(), &what);
        }

        let run = repo.reuse();

        assert!(run.cold(), "{what}: {}", run.stderr);
        same(&run.pack, &fresh, &what);
    }
}

#[test]
fn a_pack_of_a_repository_that_names_objects_otherwise_is_a_cold_run() {
    let sha1 = Repo::new("sha1");
    let sha256 = Repo::of("sha256", &files(), &["--object-format=sha256"]);
    fs::write(sha1.kept(), sha256.reuse().pack).unwrap();

    let run = sha1.reuse_as_fresh("sha-256 pack, sha-1 repository");

    assert!(run.cold(), "{}", run.stderr);
}

#[test]
fn a_pack_of_another_repository_is_reused_by_its_blobs() {
    let other = Repo::new("other-repository");
    let repo = Repo::new("this-repository");
    repo.edit(
        SHIPPING,
        "public class Shipping",
        "/* here */ public class Shipping",
    );
    repo.commit();
    fs::write(repo.kept(), other.reuse().pack).unwrap();

    let run = repo.reuse_as_fresh("a pack of another repository");

    // Its other files are the same blobs, so their rows are reused.
    assert!(run.reused(), "{}", run.stderr);
}

#[test]
fn a_run_killed_while_it_writes_the_pack_leaves_the_previous_pack() {
    let repo = Repo::new("killed");
    let previous = repo.reuse().pack;
    repo.edit(
        SHIPPING,
        "public class Shipping",
        "/* edited */ public class Shipping",
    );
    repo.commit();
    let fresh = repo.fresh();
    // A file may grow to 512 bytes and no further: the write past that
    // kills the run, as SIGXFSZ does by default.
    assert!(fresh.len() > 512, "the pack is larger than the limit");
    let (root, kept) = (repo.root(), repo.kept());
    let output = Command::new("sh")
        .args(["-c", r#"ulimit -f 1 && exec "$0" "$@""#])
        .arg(env!("CARGO_BIN_EXE_sf-impact-extract"))
        .arg("--root")
        .arg(&root)
        .arg("--previous")
        .arg(&kept)
        .arg("--out")
        .arg(&kept)
        .output()
        .expect("extract starts");

    assert!(!output.status.success(), "{output:?}");
    // The pack is written beside the kept one and renamed over it, so a
    // killed run leaves the previous pack whole.
    assert_eq!(fs::read(&kept).unwrap(), previous);

    let run = repo.reuse();

    assert!(run.reused(), "{}", run.stderr);
    same(&run.pack, &fresh, "after a killed run");
}

#[test]
fn the_pack_is_replaced_and_no_other_file_is_left() {
    let repo = Repo::new("in-place");
    repo.reuse();
    repo.edit(
        SHIPPING,
        "public class Shipping",
        "/* edited */ public class Shipping",
    );
    repo.commit();

    assert!(repo.reuse().reused());

    let kept: BTreeSet<_> = fs::read_dir(repo.dir.join("kept"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(kept, BTreeSet::from(["refs.pack".into()]));
    let status = succeeds(git(&repo.root()).args(["status", "--porcelain", "--ignored"]));
    assert!(status.stdout.is_empty(), "the repository is as committed");
}

#[test]
fn previous_must_name_the_file_that_is_written() {
    let repo = Repo::new("previous-out");
    repo.reuse();
    let (root, kept) = (repo.root(), repo.kept());
    let (root, kept) = (root.to_str().unwrap(), kept.to_str().unwrap());
    let other = repo.dir.join("other.pack");
    let other = other.to_str().unwrap();
    let same_file = repo.dir.join("kept/../kept/refs.pack");

    for args in [
        &["--root", root, "--previous", kept, "--out", other][..],
        &["--root", root, "--previous", kept],
        &["--root", root, "--previous", kept, "--out", "-"],
        &[
            "--root",
            root,
            "--previous",
            kept,
            "--out",
            kept,
            "--format",
            "jsonl",
        ],
    ] {
        let output = extract(args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(!Path::new(other).exists(), "{args:?}");
    }
    // Another file that is there is refused, and left as it is.
    fs::write(other, "another file").unwrap();
    let output = extract(&["--root", root, "--previous", kept, "--out", other]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert_eq!(fs::read(other).unwrap(), b"another file");

    // The control: the same file, named other ways.
    let link = repo.dir.join("link.pack");
    std::os::unix::fs::symlink(repo.kept(), &link).unwrap();
    for alias in [same_file, link] {
        let output = extract(&[
            "--root",
            root,
            "--previous",
            alias.to_str().unwrap(),
            "--out",
            kept,
        ]);
        assert!(output.status.success(), "{alias:?}: {output:?}");
    }
}

#[test]
fn a_first_run_names_as_previous_the_pack_it_is_about_to_write() {
    let repo = Repo::new("first-run");
    let fresh = repo.fresh();
    let root = repo.root();
    let root = root.to_str().unwrap();
    let kept = repo.kept();
    let link = repo.dir.join("link");
    std::os::unix::fs::symlink(repo.dir.join("kept"), &link).unwrap();
    let aliases = [
        repo.kept(),
        repo.dir.join("kept/../kept/refs.pack"),
        link.join("refs.pack"),
    ];

    for alias in aliases {
        // The premise: neither name is of a file yet.
        assert!(!kept.exists(), "{alias:?}");
        let output = extract(&[
            "--root",
            root,
            "--previous",
            alias.to_str().unwrap(),
            "--out",
            kept.to_str().unwrap(),
        ]);

        assert!(output.status.success(), "{alias:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{alias:?}: {output:?}");
        let run = Run {
            pack: fs::read(&kept).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        };
        assert!(run.cold(), "{alias:?}: {}", run.stderr);
        same(&run.pack, &fresh, &format!("{alias:?}"));
        fs::remove_file(&kept).unwrap();
    }

    // The control: another name that is of no file is refused, and
    // nothing is written to either.
    let other = repo.dir.join("kept/other.pack");
    let output = extract(&[
        "--root",
        root,
        "--previous",
        other.to_str().unwrap(),
        "--out",
        kept.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(!kept.exists() && !other.exists());
}

#[test]
fn an_out_that_is_a_link_is_refused_and_what_it_names_is_left_as_it_was() {
    let repo = Repo::new("out-link");
    let victim = repo.dir.join("victim");
    fs::write(&victim, "a file outside\n").unwrap();
    std::os::unix::fs::symlink(&victim, repo.kept()).unwrap();
    let (root, kept) = (repo.root(), repo.kept());
    let (root, kept) = (root.to_str().unwrap(), kept.to_str().unwrap());

    for args in [
        &["--root", root, "--out", kept][..],
        &["--root", root, "--out", kept, "--format", "jsonl"],
        &["--root", root, "--previous", kept, "--out", kept],
    ] {
        let output = extract(args);

        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(
            stderr.contains("is not a regular file"),
            "{args:?}: {stderr}"
        );
        let victim = fs::read_to_string(&victim).unwrap();
        assert_eq!(victim, "a file outside\n", "{args:?}");
        assert!(fs::symlink_metadata(kept).unwrap().is_symlink(), "{args:?}");
    }
    // Nor is the previous pack read through the link: what it names is
    // no pack, and is not decoded to say so.
    let output = extract(&["--root", root, "--previous", kept, "--out", kept]);
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("the previous pack was not reused: is not a regular file"),
        "{stderr}"
    );
    let left: BTreeSet<_> = fs::read_dir(repo.dir.join("kept"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left, BTreeSet::from(["refs.pack".into()]));

    // The control: once the link is gone, the same run writes the pack.
    fs::remove_file(kept).unwrap();
    assert!(repo.reuse().cold());
}

#[test]
fn a_project_or_config_file_that_is_a_link_or_too_large_is_not_read() {
    let repo = Repo::new("root-file-link");
    let outside = repo.dir.join("outside.json");
    let project = repo.read("sfdx-project.json");
    let root = repo.root();
    let root = root.to_str().unwrap();
    // The control: the project as committed is read.
    assert!(extract(&["--root", root, "--out", "-"]).status.success());

    fs::write(&outside, &project).unwrap();
    repo.remove("sfdx-project.json");
    std::os::unix::fs::symlink(&outside, root.to_string() + "/sfdx-project.json").unwrap();
    let output = extract(&["--root", root, "--out", "-"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("sfdx-project.json: is not a regular file"),
        "{stderr}"
    );

    repo.remove("sfdx-project.json");
    repo.write("sfdx-project.json", &project);
    fs::write(&outside, "{}").unwrap();
    std::os::unix::fs::symlink(&outside, repo.root().join("sf-impact.json")).unwrap();
    let output = extract(&["--root", root, "--out", "-"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("sf-impact.json: is not a regular file"),
        "{stderr}"
    );

    // A regular file, but past any config's size: not read at all.
    repo.remove("sf-impact.json");
    let large = fs::File::create(repo.root().join("sf-impact.json")).unwrap();
    large.set_len((16 << 20) + 1).unwrap();
    let output = extract(&["--root", root, "--out", "-"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("sf-impact.json: is over 16777216 bytes"),
        "{stderr}"
    );
}
