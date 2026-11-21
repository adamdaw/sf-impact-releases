use super::*;

const FIXTURE: &str = include_str!("../../../../fixtures/refs.jsonl");

fn fixture() -> Pack {
    Pack::from_jsonl(FIXTURE).expect("the fixture is valid refs")
}

fn jsonl(pack: &Pack) -> String {
    pack.to_jsonl()
        .iter()
        .map(|line| format!("{line}\n"))
        .collect()
}

/// A manifest for the fixture's rows: every file that has rows, one file
/// that has none, and types whose names no row has.
fn manifest(rows: &[Row]) -> Manifest {
    let mut files: Vec<FileEntry> = Vec::new();
    for row in rows {
        if let Some((package, path)) = located(row)
            && files.last().is_none_or(|last| last.path != path)
        {
            files.push(FileEntry {
                package: package.to_string(),
                path: path.to_string(),
                blob: vec![0xab; 20],
            });
        }
    }
    let rowless = FileEntry {
        package: files[0].package.clone(),
        path: format!("{}~rowless.txt", files[0].path),
        blob: vec![0xcd; 20],
    };
    files.insert(1, rowless);
    Manifest {
        types: vec![
            TypeDecl {
                file: 0,
                symbol: "Pack".into(),
                superclass: None,
                interfaces: vec!["ManifestOnlyInterface".into()],
                methods: Vec::new(),
                constructors: Vec::new(),
                fields: Vec::new(),
            },
            TypeDecl {
                file: 2,
                symbol: "ShipmentAudit".into(),
                superclass: Some("Base".into()),
                interfaces: Vec::new(),
                methods: vec![Callable {
                    symbol: "ShipmentAudit.run()".into(),
                    name: "run".into(),
                    params: Vec::new(),
                    returns: Some("ManifestOnlyType".into()),
                }],
                constructors: vec![Callable {
                    symbol: "ShipmentAudit.ShipmentAudit(Integer)".into(),
                    name: "ShipmentAudit".into(),
                    params: vec!["Integer".into()],
                    returns: None,
                }],
                fields: vec![Field {
                    symbol: "ShipmentAudit.LEVEL".into(),
                    key: "level".into(),
                    ty: "Integer".into(),
                    is_static: true,
                    is_enum_value: false,
                }],
            },
            TypeDecl {
                file: 2,
                symbol: "ShipmentAudit.Inner".into(),
                superclass: None,
                interfaces: Vec::new(),
                methods: Vec::new(),
                constructors: Vec::new(),
                fields: Vec::new(),
            },
        ],
        relationships: vec![RelationshipFact {
            file: 2,
            field: "Parcel__c.Shipment__c".into(),
            to: "Shipment__c".into(),
            child_name: Some("Parcels__r".into()),
            controlling: None,
        }],
        files,
    }
}

/// The sections of a valid pack, changed by `edit` and framed again with
/// a valid checksum: what a writer that broke the rules would write.
fn reframed(pack: &[u8], edit: impl FnOnce(&mut Vec<(u8, Vec<u8>)>)) -> Vec<u8> {
    let (provenance, mut sections) = unframe(pack).unwrap();
    edit(&mut sections);
    frame(provenance, &sections).unwrap()
}

fn payload(sections: &mut [(u8, Vec<u8>)], kind: u8) -> &mut Vec<u8> {
    &mut sections.iter_mut().find(|(k, _)| *k == kind).unwrap().1
}

fn strings_of(pack: &[u8]) -> Vec<String> {
    let (_, sections) = unframe(pack).unwrap();
    let mut input = Input::new(&sections[0].1);
    let count = input.int().unwrap();
    (0..count)
        .map(|_| {
            let length = input.int().unwrap();
            input.string_of(length).unwrap()
        })
        .collect()
}

fn refused(bytes: &[u8], why: &str) {
    match Pack::decode(bytes) {
        Ok(_) => panic!("decoded a pack that should be refused for: {why}"),
        Err(e) => assert!(e.contains(why), "refused, but for `{e}` and not `{why}`"),
    }
}

#[test]
fn a_row_only_pack_round_trips_through_its_jsonl_projection_byte_for_byte() {
    let pack = fixture().encode().unwrap();

    let decoded = Pack::decode(&pack).unwrap();
    let projection = jsonl(&decoded);
    let again = Pack::from_jsonl(&projection).unwrap().encode().unwrap();

    assert_eq!(projection, FIXTURE);
    assert!(again == pack, "pack -> JSONL -> pack changed the bytes");
}

#[test]
fn a_pack_with_a_manifest_round_trips_and_projects_as_its_rows_alone() {
    let rows = fixture();
    let with = Pack {
        manifest: Some(manifest(&rows.rows)),
        ..rows.clone()
    };

    let bytes = with.encode().unwrap();
    let decoded = Pack::decode(&bytes).unwrap();

    assert_eq!(decoded, with);
    assert!(
        decoded.encode().unwrap() == bytes,
        "encode -> decode -> encode"
    );
    assert_eq!(jsonl(&decoded), jsonl(&rows));
    let (row_only, all) = (strings_of(&rows.encode().unwrap()), strings_of(&bytes));
    assert_eq!(
        all[..row_only.len()],
        row_only[..],
        "row strings come first"
    );
    assert!(all[row_only.len()..].contains(&"ManifestOnlyType".to_string()));
}

#[test]
fn equal_content_gives_equal_bytes() {
    let one = fixture().encode().unwrap();
    let two = Pack::decode(&one).unwrap().encode().unwrap();

    assert!(one == two);
}

#[test]
fn the_first_line_is_the_plain_provenance_row_with_schema_2() {
    let pack = fixture().encode().unwrap();

    let first = pack.split(|byte| *byte == b'\n').next().unwrap();

    assert_eq!(first, FIXTURE.lines().next().unwrap().as_bytes());
}

#[test]
fn a_pack_of_another_schema_or_version_is_refused() {
    let pack = fixture().encode().unwrap();
    let line_1 = FIXTURE.lines().next().unwrap();
    let schema_1 = [
        line_1
            .replacen(r#""schema":2,"#, r#""schema":1,"#, 1)
            .as_bytes(),
        &pack[pack.iter().position(|b| *b == b'\n').unwrap()..],
    ]
    .concat();
    refused(&schema_1, "schema");

    let mut version = pack.clone();
    let header = version.iter().position(|b| *b == b'\n').unwrap() + 1 + MAGIC.len();
    version[header] = 9;
    refused(&version, "version 9");
}

#[test]
fn a_truncated_pack_is_refused_wherever_it_is_cut() {
    let pack = fixture().encode().unwrap();
    let body = pack.iter().position(|b| *b == b'\n').unwrap();

    for cut in [
        0,
        body,
        body + 1,
        body + 3,
        pack.len() / 2,
        pack.len() - 33,
        pack.len() - 1,
    ] {
        assert!(
            Pack::decode(&pack[..cut]).is_err(),
            "cut at {cut} of {}",
            pack.len()
        );
    }
}

#[test]
fn a_corrupt_pack_is_refused() {
    let pack = fixture().encode().unwrap();

    let mut flipped = pack.clone();
    let middle = flipped.len() / 2;
    flipped[middle] ^= 0x40;
    refused(&flipped, "checksum");

    let mut extra = pack.clone();
    extra.insert(pack.len() - 32, 0);
    refused(&extra, "checksum");
}

#[test]
fn a_string_that_nothing_uses_is_refused() {
    let pack = fixture().encode().unwrap();

    let unused = reframed(&pack, |sections| {
        let strings = payload(sections, STRINGS);
        let mut input = Input::new(strings);
        let count = input.int().unwrap();
        let mut rewritten = Vec::new();
        int(&mut rewritten, count + 1);
        rewritten.extend_from_slice(&strings[input.at..]);
        string(&mut rewritten, "Unused");
        *strings = rewritten;
    });

    refused(&unused, "a string that nothing uses");
}

#[test]
fn strings_out_of_first_use_order_are_refused() {
    let pack = fixture();
    let table = strings_of(&pack.encode().unwrap());
    // A writer that numbers the third string first: the same content, but
    // the rows use id 1 before id 0.
    let mut wrong = Strings::default();
    wrong.id(&table[2]);

    let misnumbered = pack.encode_with(wrong).unwrap();

    refused(&misnumbered, "first-use order");
}

#[test]
fn a_second_manifest_section_is_refused() {
    let rows = fixture();
    let with = Pack {
        manifest: Some(manifest(&rows.rows)),
        ..rows
    };
    let pack = with.encode().unwrap();

    let twice = reframed(&pack, |sections| {
        let manifest = sections.last().unwrap().clone();
        sections.push(manifest);
    });

    refused(&twice, "a second manifest section");
}

#[test]
fn manifest_entries_out_of_order_are_refused_by_the_writer_and_the_reader() {
    let rows = fixture();
    let mut swapped = manifest(&rows.rows);
    swapped.files.swap(0, 1);
    let with = |manifest| Pack {
        manifest: Some(manifest),
        ..rows.clone()
    };
    assert!(with(swapped).encode().is_err());

    // The same entries written out of order, every string already used by
    // the rows, so that order is all that is wrong.
    let valid = with(manifest(&rows.rows)).encode().unwrap();
    let table = strings_of(&valid);
    let id = |s: &str| table.iter().position(|t| t == s).unwrap() as u64;
    let files = manifest(&rows.rows).files;
    let out_of_order = reframed(&valid, |sections| {
        let section = payload(sections, MANIFEST);
        section.clear();
        int(section, 2);
        for entry in [&files[2], &files[0]] {
            int(section, id(&entry.package));
            int(section, id(&entry.path));
            section.push(20);
            section.extend_from_slice(&entry.blob);
        }
        int(section, 0);
        int(section, 0);
        let (strings, _) = sections.split_first_mut().unwrap();
        // Only the row strings: the manifest's own strings are gone.
        let rows_only = strings_of(&rows.encode().unwrap());
        strings.1.clear();
        int(&mut strings.1, rows_only.len() as u64);
        for s in &rows_only {
            string(&mut strings.1, s);
        }
    });

    refused(&out_of_order, "out of order");
}

#[test]
fn types_out_of_file_order_are_refused_and_in_a_file_keep_their_order() {
    let rows = fixture();
    let mut manifest = manifest(&rows.rows);
    // The types of one file are in the order the file declares them,
    // which is content: the other order is a pack of its own.
    manifest.types.swap(1, 2);
    let swapped = Pack {
        manifest: Some(manifest.clone()),
        ..rows.clone()
    };
    let decoded = Pack::decode(&swapped.encode().unwrap()).unwrap();
    assert_eq!(decoded.manifest, Some(manifest.clone()));
    manifest.types.swap(0, 2);

    let refused = Pack {
        manifest: Some(manifest),
        ..rows
    }
    .encode();

    assert!(refused.unwrap_err().contains("out of order"));
}

#[test]
fn both_decoders_refuse_malformed_textual_rows_alike() {
    // A single textual row gives its referenced name a string-table entry
    // that no earlier row can use. The first group then contains one row.
    let mut one = fixture();
    one.rows = one
        .rows
        .into_iter()
        .filter(|row| matches!(row, Row::Reference(r) if r.tier == Tier::Textual))
        .take(1)
        .collect();
    let pack = one.encode().unwrap();
    let rewrite = |field: usize, value: u64| {
        reframed(&pack, |sections| {
            let textual = payload(sections, TEXTUAL);
            let mut input = Input::new(textual);
            let mut offsets = Vec::new();
            for _ in 0..5 {
                let start = input.at;
                input.int().unwrap();
                offsets.push((start, input.at));
            }
            let (start, end) = offsets[field];
            let mut out = textual[..start].to_vec();
            int(&mut out, value);
            out.extend_from_slice(&textual[end..]);
            *textual = out;
        })
    };
    let empty_referenced = reframed(&pack, |sections| {
        let referenced = {
            let mut input = Input::new(payload(sections, TEXTUAL));
            input.int().unwrap(); // referrer
            input.int().unwrap(); // count
            input.int().unwrap()
        };
        let table = payload(sections, STRINGS);
        let mut input = Input::new(table);
        let count = input.int().unwrap();
        let mut out = table[..input.at].to_vec();
        for id in 0..count {
            let length = input.int().unwrap();
            let value = input.string_of(length).unwrap();
            string(&mut out, if id == referenced { "" } else { &value });
        }
        *table = out;
    });
    let cases = [
        ("referrer out of range", rewrite(0, 1 << 40)),
        ("referenced out of range", rewrite(2, 1 << 40)),
        ("line out of range", rewrite(3, 1 << 34)),
        ("column out of range", rewrite(4, 1 << 32)),
        ("column zero", rewrite(4, 0)),
        ("empty referenced", empty_referenced),
    ];
    for (what, bytes) in cases {
        let full = Pack::decode(&bytes).err();
        assert!(full.is_some(), "{what}: full decode accepted it");
        assert_eq!(full, Pack::decode_without_textual(&bytes).err(), "{what}");
    }
}

#[test]
fn a_textual_group_split_in_two_is_refused() {
    let pack = fixture().encode().unwrap();

    // The first group of two or more rows, written as two groups with the
    // same referrer: the same rows, but not canonical.
    let split = reframed(&pack, |sections| {
        let textual = payload(sections, TEXTUAL);
        let mut input = Input::new(textual);
        loop {
            let group = input.at;
            let referrer = input.int().unwrap();
            let count = input.int().unwrap();
            let first = input.at;
            for _ in 0..count * 3 {
                input.int().unwrap();
            }
            if count < 2 {
                continue;
            }
            let mut one = Input::new(&textual[first..]);
            for _ in 0..3 {
                one.int().unwrap();
            }
            let (one, rest) = textual[first..input.at].split_at(one.at);
            let mut rewritten = textual[..group].to_vec();
            int(&mut rewritten, referrer);
            int(&mut rewritten, 1);
            rewritten.extend_from_slice(one);
            int(&mut rewritten, referrer);
            int(&mut rewritten, count - 1);
            rewritten.extend_from_slice(rest);
            rewritten.extend_from_slice(&textual[input.at..]);
            *textual = rewritten;
            break;
        }
    });

    refused(&split, "same referrer");
}

#[test]
fn a_row_that_refs_jsonl_would_refuse_is_refused() {
    let mut pack = fixture();
    let Some(Row::Declaration(declaration)) = pack
        .rows
        .iter_mut()
        .find(|row| matches!(row, Row::Declaration(_)))
    else {
        panic!("the fixture has a declaration");
    };
    declaration.start = 0;

    assert!(pack.encode().is_err());
}

/// Every `enum` or `const` string under `value`.
fn named(value: &serde_json::Value, into: &mut std::collections::BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                match (key.as_str(), value) {
                    ("const", serde_json::Value::String(name)) => {
                        into.insert(name.clone());
                    }
                    ("enum", serde_json::Value::Array(names)) => {
                        into.extend(names.iter().filter_map(|n| n.as_str()).map(str::to_owned));
                    }
                    _ => named(value, into),
                }
            }
        }
        serde_json::Value::Array(values) => values.iter().for_each(|v| named(v, into)),
        _ => {}
    }
}

#[test]
fn every_reference_kind_in_the_schema_has_one_tag() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../../../schema/refs.schema.json")).unwrap();
    let defs = &schema["$defs"];
    let mut declared = std::collections::BTreeSet::new();
    for def in [
        "parsed_reference",
        "metadata_reference",
        "textual_reference",
    ] {
        named(&defs[def]["properties"]["kind"], &mut declared);
    }
    named(&defs["flow_lwc_reference_kind"], &mut declared);

    let tagged: Vec<String> = KINDS
        .iter()
        .map(|kind| {
            serde_json::to_value(kind)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();

    assert_eq!(tagged.len(), declared.len(), "one tag each, no repeats");
    assert_eq!(
        tagged
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>(),
        declared
    );
}

/// A pack of these sections, each a kind, the size it says and its frame
/// as given: what a writer other than this module could write.
fn assemble(sections: &[(u8, u64, Vec<u8>)]) -> Vec<u8> {
    let mut out = br#"{"row":"provenance","tool":"sf-impact-extract","schema":2}"#.to_vec();
    out.push(b'\n');
    out.extend_from_slice(MAGIC);
    int(&mut out, VERSION);
    for (kind, size, frame) in sections {
        out.push(*kind);
        int(&mut out, *size);
        int(&mut out, frame.len() as u64);
        out.extend_from_slice(frame);
    }
    let checksum = Sha256::digest(&out);
    out.extend_from_slice(&checksum);
    out
}

/// Each payload compressed as one frame, with its true size.
fn compressed(payloads: &[(u8, &[u8])]) -> Vec<(u8, u64, Vec<u8>)> {
    let frame = |payload: &[u8]| zstd::bulk::compress(payload, 3).unwrap();
    let each = payloads
        .iter()
        .map(|(kind, payload)| (*kind, payload.len() as u64, frame(payload)));
    each.collect()
}

#[test]
fn a_small_pack_that_says_it_expands_past_its_budget_is_refused_unread() {
    // Reproducer: 2,253 bytes, a valid checksum, and a string table
    // of one 64 MiB string that only the manifest uses.
    let bomb = include_bytes!("valid-bomb.pack");

    refused(bomb, "budget");
}

#[test]
fn sections_each_within_budget_are_refused_when_together_they_are_not() {
    // 16 KiB that does not compress, then zeros: each section expands about
    // 25 times, under a section's 32, and two of them over a pack's 16.
    let mut seed = 1u32;
    let mut payload: Vec<u8> = (0..16 * 1024)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 24) as u8
        })
        .collect();
    payload.resize(25 * 16 * 1024, 0);
    let mut sections = compressed(&[(STRINGS, &payload), (FILES, &payload)]);
    sections.extend(compressed(&[(RECORDS, b""), (PARSED, b""), (TEXTUAL, b"")]));
    // Each section alone is within its own budget.
    assert!(SECTION_RATIO * sections[1].2.len() as u64 > sections[1].1);

    refused(&assemble(&sections), "the pack's decompression budget");
}

#[test]
fn section_kinds_are_checked_before_anything_is_decompressed() {
    let not_zstd = vec![0xff; 16];
    let sections = [(STRINGS, 16, not_zstd.clone()), (STRINGS, 16, not_zstd)];

    refused(&assemble(&sections), "its sections are");
}

#[test]
fn a_section_of_two_frames_is_refused() {
    let pack = fixture().encode().unwrap();
    let (_, payloads) = unframe(&pack).unwrap();
    let payloads: Vec<(u8, &[u8])> = payloads.iter().map(|(k, p)| (*k, p.as_slice())).collect();
    let mut sections = compressed(&payloads);
    let empty = zstd::bulk::compress(b"", 3).unwrap();
    sections[4].2.extend_from_slice(&empty);

    refused(&assemble(&sections), "exactly one zstd frame");
}

#[test]
fn a_manifest_string_that_the_schema_requires_is_never_empty() {
    let rows = fixture();
    let blank: [fn(&mut Manifest); 7] = [
        |m| m.files[0].path.clear(),
        |m| m.types[0].symbol.clear(),
        |m| m.relationships[0].field.clear(),
        |m| m.relationships[0].to.clear(),
        |m| m.types[1].methods[0].name.clear(),
        |m| m.types[1].constructors[0].symbol.clear(),
        |m| m.types[1].fields[0].key.clear(),
    ];
    for (at, blank) in blank.iter().enumerate() {
        let mut manifest = manifest(&rows.rows);
        blank(&mut manifest);
        let pack = Pack {
            manifest: Some(manifest),
            ..rows.clone()
        };

        let refused = pack.encode().expect_err("an empty string is refused");

        assert!(refused.contains("empty"), "{at}: {refused}");
    }
}

#[test]
fn a_pack_whose_manifest_has_an_empty_path_or_type_symbol_is_refused() {
    // Reproducers: rowless packs whose manifests a writer outside
    // this module wrote.
    let rowless = |strings: &[u8], manifest: &[u8]| {
        assemble(&compressed(&[
            (STRINGS, strings),
            (FILES, b"\0\0"),
            (RECORDS, b""),
            (PARSED, b""),
            (TEXTUAL, b""),
            (MANIFEST, manifest),
        ]))
    };
    let blob = [0u8; 20];
    let path = rowless(
        b"\x02\x01p\x00",
        &[b"\x01\x00\x01\x14".as_slice(), &blob, b"\0\0"].concat(),
    );
    let owner = rowless(
        b"\x03\x01p\x01f\x00",
        &[
            b"\x01\x00\x01\x14".as_slice(),
            &blob,
            b"\x01\x00\x02\x00\x00\x00\x00\x00\x00",
        ]
        .concat(),
    );

    refused(&path, "empty path");
    refused(&owner, "empty symbol");
}

#[test]
fn a_frame_whose_window_is_at_most_4_mib_is_read() {
    // Fixture: a valid pack whose one frame advertises 4 MiB.
    assert!(Pack::decode(include_bytes!("window-4MiB.pack")).is_ok());
}

#[test]
fn a_frame_whose_window_is_over_4_mib_is_refused_before_it_is_decompressed() {
    // Fixtures: the same pack, its frame advertising 4.5 and 8 MiB.
    refused(include_bytes!("window-4.5MiB.pack"), "window");
    refused(include_bytes!("window-8MiB.pack"), "window");
}

#[test]
fn a_small_pack_whose_rows_repeat_a_long_string_is_refused_as_it_is_decoded() {
    // Reproducer: 49 KB, 4,096 scope errors that all use one 64 KiB
    // path, which as rows would take 256 MiB.
    refused(include_bytes!("rows-4096.pack"), "decoding budget");
}

#[test]
fn a_small_pack_whose_long_list_would_take_more_than_its_strings_is_refused() {
    // Reproducer: 42 KB, a manifest whose one parameter list has
    // 680,000 entries of a short string. Its strings are within the budget;
    // the list that holds them, 24 bytes an entry, is not.
    refused(include_bytes!("parameters-680000.pack"), "decoding budget");
}

#[test]
fn a_count_beyond_the_budget_is_refused_before_anything_is_reserved() {
    // The fixture's string table, claiming 2^40 entries: the table's room
    // alone would be 24 TiB, so it is charged and refused, not reserved.
    let pack = fixture().encode().unwrap();

    let huge = reframed(&pack, |sections| {
        let strings = payload(sections, STRINGS);
        let mut input = Input::new(strings);
        input.int().unwrap();
        let mut rewritten = Vec::new();
        int(&mut rewritten, 1 << 40);
        rewritten.extend_from_slice(&strings[input.at..]);
        *strings = rewritten;
    });

    refused(&huge, "decoding budget");
}

#[test]
fn a_long_string_in_a_diagnostic_is_cut_to_a_short_excerpt() {
    // An unused 40,000-byte string: the refusal names it, but by a prefix
    // and its length, not by copying it.
    let pack = fixture().encode().unwrap();
    let long = "x".repeat(40_000);

    let unused = reframed(&pack, |sections| {
        let strings = payload(sections, STRINGS);
        let mut input = Input::new(strings);
        let count = input.int().unwrap();
        let mut rewritten = Vec::new();
        int(&mut rewritten, count + 1);
        rewritten.extend_from_slice(&strings[input.at..]);
        string(&mut rewritten, &long);
        *strings = rewritten;
    });

    let why = Pack::decode(&unused).unwrap_err();
    assert!(why.contains("a string that nothing uses"), "{why}");
    assert!(why.contains("(40000 bytes)"), "{why}");
    assert!(why.len() < 300, "a {}-byte diagnostic", why.len());
}

#[test]
fn a_string_in_a_diagnostic_cannot_start_a_line_of_its_own() {
    let pack = fixture().encode().unwrap();
    let hostile = "x\n::error::injected\u{1b}[2K";

    let unused = reframed(&pack, |sections| {
        let strings = payload(sections, STRINGS);
        let mut input = Input::new(strings);
        let count = input.int().unwrap();
        let mut rewritten = Vec::new();
        int(&mut rewritten, count + 1);
        rewritten.extend_from_slice(&strings[input.at..]);
        string(&mut rewritten, hostile);
        *strings = rewritten;
    });

    let why = Pack::decode(&unused).unwrap_err();
    assert!(why.contains("`x\\n::error::injected\\u{1b}[2K`"), "{why}");
    assert!(!why.chars().any(char::is_control), "{why:?}");
}

#[test]
fn many_section_headers_are_refused_before_they_are_kept() {
    // Five sections, then 10,000 empty headers of an unknown kind. The
    // refusal lists no more kinds than it needs to refuse.
    let mut sections = compressed(&[
        (STRINGS, &[0]),
        (FILES, &[0, 0]),
        (RECORDS, &[]),
        (PARSED, &[]),
        (TEXTUAL, &[]),
    ]);
    sections.extend((0..10_000).map(|_| (9, 0, Vec::new())));

    let why = Pack::decode(&assemble(&sections)).unwrap_err();
    assert!(why.contains("its sections are"), "{why}");
    assert!(why.len() < 300, "a {}-byte diagnostic", why.len());
}

#[test]
fn a_line_1_over_its_limit_is_refused_before_it_is_parsed() {
    // A valid provenance row, padded with JSON whitespace to one byte over
    // the 64 KiB limit: valid JSON, so only the limit refuses it.
    let pack = fixture().encode().unwrap();
    let lf = pack.iter().position(|b| *b == b'\n').unwrap();
    let mut padded = pack[..lf].to_vec();
    padded.resize(64 * 1024 + 1, b' ');
    let mut long = [&padded[..], &pack[lf..pack.len() - CHECKSUM]].concat();
    let checksum = Sha256::digest(&long);
    long.extend_from_slice(&checksum);

    refused(&long, "line 1 is over");
}

/// Line 1 of a pack that `unframe` accepts.
const PROVENANCE: &str = r#"{"row":"provenance","tool":"sf-impact-extract","schema":2}"#;

#[test]
fn a_section_that_compresses_past_its_budget_is_stored_and_read_back() {
    // One tag byte for each of a million rows compresses far past 32 times;
    // 2 MiB that does not compress keeps the pack as a whole within its own.
    let mut seed = 1u32;
    let incompressible = (0..2 << 20)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 24) as u8
        })
        .collect();
    let sections = vec![
        (STRINGS, vec![0; 1 << 20]),
        (FILES, b"\0\0".to_vec()),
        (RECORDS, incompressible),
        (PARSED, Vec::new()),
        (TEXTUAL, Vec::new()),
    ];
    let compressed = zstd::bulk::compress(&sections[0].1, LEVEL).unwrap();
    assert!(SECTION_RATIO * (compressed.len() as u64) < 1 << 20);

    let pack = frame(PROVENANCE, &sections).unwrap();

    assert_eq!(unframe(&pack).unwrap(), (PROVENANCE, sections));
}

#[test]
fn sections_that_together_compress_past_the_packs_budget_are_stored_until_they_do_not() {
    // As in the reader's test: each expands about 25 times, two over 16.
    let mut seed = 1u32;
    let mut payload: Vec<u8> = (0..16 * 1024)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 24) as u8
        })
        .collect();
    payload.resize(25 * 16 * 1024, 0);
    let sections = vec![
        (STRINGS, payload.clone()),
        (FILES, payload),
        (RECORDS, Vec::new()),
        (PARSED, Vec::new()),
        (TEXTUAL, Vec::new()),
    ];

    let pack = frame(PROVENANCE, &sections).unwrap();

    assert_eq!(unframe(&pack).unwrap(), (PROVENANCE, sections));
}

#[test]
fn a_stored_frame_is_one_zstd_frame_of_its_payload() {
    for length in [0, 1, 128 * 1024, 128 * 1024 + 1, 300_000] {
        let payload: Vec<u8> = (0..length).map(|at| at as u8).collect();

        let stored = stored(&payload);

        assert_eq!(
            zstd::zstd_safe::find_frame_compressed_size(&stored),
            Ok(stored.len())
        );
        assert_eq!(window(&stored), Some(128 * 1024));
        assert_eq!(zstd::bulk::decompress(&stored, length).unwrap(), payload);
    }
}
