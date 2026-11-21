//! A second, independent reading of the refs pack, written from
//! `docs/pack.md` alone. It decodes what `Pack::encode` writes into plain
//! `serde_json::Value`s and compares them with the fixture.

use serde_json::{Map, Value, json};
use sf_impact_common::pack::{
    Callable, Field, FileEntry, Manifest, Pack, RelationshipFact, TypeDecl,
};
use sha2::{Digest, Sha256};

const FIXTURE: &str = include_str!("../../../fixtures/refs.jsonl");

#[rustfmt::skip]
const DECL_TYPE: &[&str] = &[
    "class", "interface", "enum", "trigger", "method", "constructor", "field", "enum_value",
    "object", "object_field", "validation_rule", "label", "custom_permission", "message_channel",
    "permission_set", "profile", "metadata_record", "flow", "lwc",
];
#[rustfmt::skip]
const REF_KIND: &[&str] = &[
    "call", "construct", "type", "cast", "extends", "implements", "static_access", "soql_object",
    "soql_field", "dml_object", "dml_field", "sobject_field", "trigger_object", "lookup",
    "lookup_filter", "formula", "summary", "controlling_field", "display_field", "value_set",
    "grant", "cmdt", "cmdt_field", "cmdt_type", "flow_object", "flow_field", "flow_action",
    "flow_subflow", "flow_component", "flow_label", "flow_custom_permission",
    "flow_custom_metadata", "lwc_apex", "lwc_schema", "lwc_label", "lwc_custom_permission",
    "lwc_message_channel", "lwc_component", "lwc_target", "lwc_object", "text",
];
#[rustfmt::skip]
const REFERENCED_TYPE: &[&str] = &[
    "class", "method", "constructor", "field", "enum_value", "trigger", "object", "object_field",
    "validation_rule", "label", "custom_permission", "message_channel", "permission_set",
    "profile", "metadata_record", "flow", "lwc", "tab", "global_value_set",
];
const RESOLUTION: &[&str] = &["declaration", "external", "unresolved"];
const DETAIL: &[&str] = &[
    "ambiguous_overload",
    "ambiguous_receiver",
    "unknown_type",
    "no_member",
];
const DECLARED_IN: &[&str] = &["packageDirectories", "unpackagedMetadata"];
const SCOPE_REASON: &[&str] = &["missing", "not_a_directory", "outside_root"];
const SKIP_REASON: &[&str] = &["binary"];
const UNSUPPORTED_REASON: &[&str] = &["not_interpreted", "no_name"];

/// A cursor over one payload, with the value types of "## Values".
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    strings: &'a [String],
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], strings: &'a [String]) -> Self {
        Reader {
            bytes,
            at: 0,
            strings,
        }
    }
    fn byte(&mut self) -> u8 {
        let b = self.bytes[self.at];
        self.at += 1;
        b
    }
    fn take(&mut self, n: usize) -> &'a [u8] {
        let s = &self.bytes[self.at..self.at + n];
        self.at += n;
        s
    }
    fn int(&mut self) -> u64 {
        let (mut value, mut shift) = (0u64, 0u32);
        loop {
            let b = self.byte();
            assert!(shift < 64, "int longer than 64 bits");
            value |= u64::from(b & 0x7f) << shift;
            if b & 0x80 == 0 {
                assert!(shift == 0 || b != 0, "int not in shortest form");
                return value;
            }
            shift += 7;
        }
    }
    fn usize(&mut self) -> usize {
        usize::try_from(self.int()).unwrap()
    }
    fn delta(&mut self) -> i64 {
        let n = self.int();
        ((n >> 1) as i64) ^ -((n & 1) as i64)
    }
    fn id(&mut self) -> Value {
        let i = self.usize();
        Value::String(self.strings[i].clone())
    }
    fn ids(&mut self) -> Value {
        let n = self.usize();
        Value::Array((0..n).map(|_| self.id()).collect())
    }
    fn opt(&mut self) -> Value {
        match self.usize() {
            0 => Value::Null,
            i => Value::String(self.strings[i - 1].clone()),
        }
    }
    fn tag(&mut self, list: &[&str]) -> Value {
        json!(list[usize::from(self.byte())])
    }
    fn opt_tag(&mut self, list: &[&str]) -> Option<Value> {
        match self.byte() {
            0 => None,
            t => Some(json!(list[usize::from(t - 1)])),
        }
    }
    fn finished(&self) {
        assert_eq!(
            self.at,
            self.bytes.len(),
            "payload not read to its last byte"
        );
    }
}

struct Decoded {
    line1: String,
    strings: Vec<String>,
    rows: Vec<Value>,
    manifest: Option<Value>,
}

fn decode(pack: &[u8]) -> Decoded {
    let lf = pack.iter().position(|&b| b == b'\n').expect("line 1");
    let line1 = std::str::from_utf8(&pack[..lf]).unwrap().to_string();
    let provenance: Value = serde_json::from_str(&line1).unwrap();
    assert_eq!(provenance["row"], "provenance");
    assert_eq!(provenance["tool"], "sf-impact-extract");
    assert_eq!(provenance["schema"], 2);

    let body_end = pack.len() - 32;
    let mut top = Reader::new(&pack[..body_end], &[]);
    top.at = lf + 1;
    assert_eq!(top.take(4), b"SFIP");
    assert_eq!(top.int(), 2, "version");
    assert_eq!(&Sha256::digest(&pack[..body_end])[..], &pack[body_end..]);

    let mut payloads = Vec::new();
    while top.at < body_end {
        let kind = top.byte();
        let size = top.usize();
        let length = top.usize();
        let payload = zstd::stream::decode_all(top.take(length)).unwrap();
        assert_eq!(payload.len(), size, "section {kind} size");
        assert_eq!(usize::from(kind), payloads.len() + 1, "section order");
        payloads.push(payload);
    }
    assert!(payloads.len() == 5 || payloads.len() == 6, "section count");

    let mut st = Reader::new(&payloads[0], &[]);
    let strings: Vec<String> = (0..st.usize())
        .map(|_| {
            let n = st.usize();
            String::from_utf8(st.take(n).to_vec()).unwrap()
        })
        .collect();
    st.finished();

    let mut files = Reader::new(&payloads[1], &strings);
    let mut records = Reader::new(&payloads[2], &strings);
    let mut parsed = Reader::new(&payloads[3], &strings);
    let mut textual = Reader::new(&payloads[4], &strings);
    let mut rows = Vec::new();

    for _ in 0..files.usize() {
        rows.push(json!({
            "row": "scope_error",
            "path": records.id(),
            "declared_in": records.tag(DECLARED_IN),
            "reason": records.tag(SCOPE_REASON),
        }));
    }
    let mut previous_path: Option<String> = None;
    for _ in 0..files.usize() {
        let package = files.id();
        let file = files.id();
        let path = file.as_str().unwrap().to_string();
        assert!(
            previous_path
                .as_deref()
                .is_none_or(|p| p.as_bytes() < path.as_bytes())
        );
        previous_path = Some(path);
        let n = files.usize();
        assert!(n >= 1, "a file has at least one row");
        let tags = files.take(n).to_vec();
        let mut textual_left = tags.iter().filter(|&&t| t == 5).count();
        let (mut parsed_line, mut textual_line) = (0i64, 0i64);
        let (mut group_left, mut group_referrer) = (0usize, None::<Value>);
        for tag in tags {
            let mut row = Map::new();
            match tag {
                0 => {
                    row.insert("row".into(), json!("parse_error"));
                    row.insert("line".into(), json!(records.int()));
                }
                1 => {
                    row.insert("row".into(), json!("skipped_file"));
                    row.insert("reason".into(), records.tag(SKIP_REASON));
                }
                2 => {
                    row.insert("row".into(), json!("unmodelled_file"));
                    row.insert("suffix".into(), records.id());
                }
                3 => {
                    row.insert("row".into(), json!("declaration"));
                    row.insert("symbol".into(), records.id());
                    row.insert("name".into(), records.id());
                    row.insert("type".into(), records.tag(DECL_TYPE));
                    let start = records.int();
                    let end = start as i64 + records.delta();
                    row.insert("start".into(), json!(start));
                    row.insert("end".into(), json!(end));
                    let flags = records.byte();
                    assert_eq!(flags & !0b1111, 0, "unused flag bits");
                    row.insert("is_test".into(), json!(flags & 1 != 0));
                    row.insert("collision".into(), json!(flags & 2 != 0));
                    match flags >> 2 {
                        0 => {}
                        s => _ = row.insert("scope".into(), json!(DECLARED_IN[usize::from(s - 1)])),
                    }
                    row.insert("annotations".into(), records.ids());
                }
                4 => {
                    row.insert("row".into(), json!("reference"));
                    row.insert("tier".into(), json!("parsed"));
                    row.insert("kind".into(), parsed.tag(REF_KIND));
                    row.insert("referenced".into(), parsed.id());
                    for (key, list) in [
                        ("referenced_type", REFERENCED_TYPE),
                        ("resolution", RESOLUTION),
                        ("detail", DETAIL),
                    ] {
                        if let Some(v) = parsed.opt_tag(list) {
                            row.insert(key.into(), v);
                        }
                    }
                    let candidates = parsed.ids();
                    if !candidates.as_array().unwrap().is_empty() {
                        row.insert("candidates".into(), candidates);
                    }
                    row.insert("referrer".into(), parsed.opt());
                    let source_field = parsed.opt();
                    if !source_field.is_null() {
                        row.insert("source_field".into(), source_field);
                    }
                    parsed_line += parsed.delta();
                    row.insert("line".into(), json!(parsed_line));
                    row.insert("column".into(), json!(parsed.int()));
                }
                5 => {
                    if group_left == 0 {
                        let referrer = textual.opt();
                        assert_ne!(Some(&referrer), group_referrer.as_ref(), "adjacent groups");
                        group_left = textual.usize();
                        assert!((1..=textual_left).contains(&group_left), "group count");
                        group_referrer = Some(referrer);
                    }
                    group_left -= 1;
                    textual_left -= 1;
                    row.insert("row".into(), json!("reference"));
                    row.insert("tier".into(), json!("textual"));
                    row.insert("kind".into(), json!("text"));
                    row.insert("referenced".into(), textual.id());
                    row.insert("referrer".into(), group_referrer.clone().unwrap());
                    textual_line += textual.delta();
                    row.insert("line".into(), json!(textual_line));
                    row.insert("column".into(), json!(textual.int()));
                }
                6 => {
                    row.insert("row".into(), json!("unsupported_element"));
                    row.insert("element".into(), records.id());
                    row.insert("reason".into(), records.tag(UNSUPPORTED_REASON));
                    row.insert("line".into(), json!(records.int()));
                    row.insert("column".into(), json!(records.int()));
                }
                other => panic!("unknown row tag {other}"),
            }
            row.insert("package".into(), package.clone());
            row.insert("file".into(), file.clone());
            rows.push(Value::Object(row));
        }
        assert_eq!(group_left, 0, "a textual group spills past its file");
    }
    for r in [&files, &records, &parsed, &textual] {
        r.finished();
    }

    let manifest = payloads
        .get(5)
        .map(|p| decode_manifest(&mut Reader::new(p, &strings)));
    Decoded {
        line1,
        strings,
        rows,
        manifest,
    }
}

fn decode_manifest(m: &mut Reader) -> Value {
    let mut blob_len = None;
    let entries: Vec<Value> = (0..m.usize())
        .map(|_| {
            let (package, path) = (m.id(), m.id());
            let n = m.byte();
            assert!(n == 20 || n == 32);
            assert_eq!(*blob_len.get_or_insert(n), n, "one blob length");
            json!({"package": package, "path": path, "blob": m.take(usize::from(n))})
        })
        .collect();
    let mut file = 0;
    let types: Vec<Value> = (0..m.usize())
        .map(|_| {
            file += m.usize();
            let (symbol, superclass, interfaces) = (m.id(), m.opt(), m.ids());
            let (methods, constructors) = (callables(m), callables(m));
            let fields: Vec<Value> = (0..m.usize())
                .map(|_| {
                    let (symbol, key, ty) = (m.id(), m.id(), m.id());
                    let flags = m.byte();
                    assert!(flags < 4, "field flags {flags}");
                    json!({"symbol": symbol, "key": key, "ty": ty,
                           "is_static": flags & 1 != 0, "is_enum_value": flags & 2 != 0})
                })
                .collect();
            json!({"file": file, "symbol": symbol, "superclass": superclass,
                   "interfaces": interfaces, "methods": methods,
                   "constructors": constructors, "fields": fields})
        })
        .collect();
    file = 0;
    let relationships: Vec<Value> = (0..m.usize())
        .map(|_| {
            file += m.usize();
            json!({"file": file, "field": m.id(), "to": m.id(),
                   "child_name": m.opt(), "controlling": m.opt()})
        })
        .collect();
    m.finished();
    json!({"files": entries, "types": types, "relationships": relationships})
}

/// A manifest type's methods or constructors.
fn callables(m: &mut Reader) -> Vec<Value> {
    (0..m.usize())
        .map(|_| json!({"symbol": m.id(), "name": m.id(), "params": m.ids(), "returns": m.opt()}))
        .collect()
}

/// The strings of a decoded pack in the order of "## The string table".
fn first_use_order(rows: &[Value], manifest: Option<&Value>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |v: &Value| match v {
        Value::String(s) if !out.contains(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|i| {
            let s = i.as_str().unwrap().to_string();
            if !out.contains(&s) {
                out.push(s);
            }
        }),
        _ => {}
    };
    for row in rows {
        let fields: &[&str] = match row["row"].as_str().unwrap() {
            "scope_error" => &["path"],
            "parse_error" | "skipped_file" => &["package", "file"],
            "unmodelled_file" => &["package", "file", "suffix"],
            "declaration" => &["symbol", "name", "package", "file", "annotations"],
            "reference" => &[
                "referenced",
                "candidates",
                "referrer",
                "source_field",
                "package",
                "file",
            ],
            _ => &["package", "file", "element"],
        };
        fields.iter().for_each(|f| push(&row[*f]));
    }
    if let Some(m) = manifest {
        m["files"].as_array().unwrap().iter().for_each(|e| {
            push(&e["package"]);
            push(&e["path"]);
        });
        for t in m["types"].as_array().unwrap() {
            ["symbol", "superclass", "interfaces"]
                .iter()
                .for_each(|f| push(&t[*f]));
            for c in ["methods", "constructors"]
                .iter()
                .flat_map(|f| t[*f].as_array().unwrap())
            {
                ["symbol", "name", "params", "returns"]
                    .iter()
                    .for_each(|f| push(&c[*f]));
            }
            for f in t["fields"].as_array().unwrap() {
                ["symbol", "key", "ty"].iter().for_each(|k| push(&f[*k]));
            }
        }
        for r in m["relationships"].as_array().unwrap() {
            ["field", "to", "child_name", "controlling"]
                .iter()
                .for_each(|f| push(&r[*f]));
        }
    }
    out
}

fn schema_2_fixture() -> String {
    assert!(FIXTURE.lines().next().unwrap().contains("\"schema\":2,"));
    FIXTURE.to_string()
}

fn fixture_rows(text: &str) -> Vec<Value> {
    text.lines()
        .skip(1)
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

#[test]
fn a_row_only_pack_decodes_to_exactly_the_fixture_rows_in_order() {
    let text = schema_2_fixture();
    let bytes = Pack::from_jsonl(&text).unwrap().encode().unwrap();
    let decoded = decode(&bytes);

    assert_eq!(decoded.line1, text.lines().next().unwrap());
    let expected = fixture_rows(&text);
    assert_eq!(decoded.rows.len(), expected.len());
    for (i, (got, want)) in decoded.rows.iter().zip(&expected).enumerate() {
        assert_eq!(got, want, "row {} (fixture line {})", i, i + 2);
    }
    assert!(decoded.manifest.is_none());
    assert_eq!(decoded.strings, first_use_order(&decoded.rows, None));
}

#[test]
fn a_pack_with_a_manifest_decodes_the_manifest_as_built_and_the_rows_unchanged() {
    let text = schema_2_fixture();
    let rows = fixture_rows(&text);
    let mut files: Vec<(String, String)> = Vec::new();
    for r in rows.iter().filter(|r| r.get("file").is_some()) {
        let key = (
            r["package"].as_str().unwrap().into(),
            r["file"].as_str().unwrap().into(),
        );
        if !files.contains(&key) {
            files.push(key);
        }
    }
    let extra = "force-app/declarative/classes/Packet.cls".to_string();
    assert!(files[0].1 < extra && extra < files[1].1);
    files.insert(1, ("force-app".into(), extra));
    let index = |path: &str| files.iter().position(|f| f.1.ends_with(path)).unwrap();
    let blob = |i: usize| (0..20).map(|b| (i * 7 + b) as u8).collect::<Vec<u8>>();

    let account_no = index("objects/Account/fields/No__c.field-meta.xml");
    let tracker = index("classes/ShipmentTracker.cls");
    let deliver = "ShipmentTracker.deliver(list<shipment__c>)";
    let callable = |symbol: &str, name: &str, params: &[&str], returns: Option<&str>| Callable {
        symbol: symbol.into(),
        name: name.into(),
        params: params.iter().map(|p| p.to_string()).collect(),
        returns: returns.map(Into::into),
    };
    let types = vec![
        TypeDecl {
            file: 0,
            symbol: "Pack".into(),
            superclass: Some("NoRowNamesThisType".into()),
            interfaces: vec!["Schedulable".into(), "Database.Batchable<SObject>".into()],
            methods: Vec::new(),
            constructors: vec![callable("Pack.Pack()", "Pack", &[], None)],
            fields: Vec::new(),
        },
        TypeDecl {
            file: tracker,
            symbol: "ShipmentTracker".into(),
            superclass: None,
            interfaces: Vec::new(),
            methods: vec![
                callable(deliver, "deliver", &["list<shipment__c>"], None),
                callable("ShipmentTracker.count()", "count", &[], Some("Integer")),
            ],
            constructors: Vec::new(),
            fields: vec![
                Field {
                    symbol: "ShipmentTracker.n".into(),
                    key: "n".into(),
                    ty: "Integer".into(),
                    is_static: false,
                    is_enum_value: false,
                },
                Field {
                    symbol: "ShipmentTracker.m".into(),
                    key: "m".into(),
                    ty: "Integer".into(),
                    is_static: true,
                    is_enum_value: false,
                },
            ],
        },
        TypeDecl {
            file: tracker,
            symbol: "ShipmentTracker.State".into(),
            superclass: None,
            interfaces: Vec::new(),
            methods: Vec::new(),
            constructors: Vec::new(),
            fields: vec![Field {
                symbol: "ShipmentTracker.State.DONE".into(),
                key: "done".into(),
                ty: "ShipmentTracker.State".into(),
                is_static: true,
                is_enum_value: true,
            }],
        },
    ];
    let callable_json = |c: &Callable| json!({"symbol": c.symbol, "name": c.name, "params": c.params, "returns": c.returns});
    let relationship = (
        account_no,
        "Account.No__c",
        "Account",
        Some("Children"),
        None::<&str>,
    );

    let expected = json!({
        "files": files.iter().enumerate()
            .map(|(i, (p, f))| json!({"package": p, "path": f, "blob": blob(i)}))
            .collect::<Vec<_>>(),
        "types": types.iter().map(|t| json!({
            "file": t.file, "symbol": t.symbol, "superclass": t.superclass,
            "interfaces": t.interfaces,
            "methods": t.methods.iter().map(callable_json).collect::<Vec<_>>(),
            "constructors": t.constructors.iter().map(callable_json).collect::<Vec<_>>(),
            "fields": t.fields.iter().map(|f| json!({
                "symbol": f.symbol, "key": f.key, "ty": f.ty,
                "is_static": f.is_static, "is_enum_value": f.is_enum_value,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "relationships": [{
            "file": relationship.0, "field": relationship.1, "to": relationship.2,
            "child_name": relationship.3, "controlling": relationship.4,
        }],
    });

    let mut pack = Pack::from_jsonl(&text).unwrap();
    pack.manifest = Some(Manifest {
        files: files
            .iter()
            .enumerate()
            .map(|(i, (p, f))| FileEntry {
                package: p.clone(),
                path: f.clone(),
                blob: blob(i),
            })
            .collect(),
        types,
        relationships: vec![RelationshipFact {
            file: relationship.0,
            field: relationship.1.into(),
            to: relationship.2.into(),
            child_name: relationship.3.map(Into::into),
            controlling: relationship.4.map(Into::into),
        }],
    });
    let decoded = decode(&pack.encode().unwrap());

    assert_eq!(decoded.manifest.as_ref(), Some(&expected));
    assert_eq!(decoded.rows, rows);
    assert!(decoded.strings.iter().any(|s| s == "NoRowNamesThisType"));
    assert_eq!(
        decoded.strings,
        first_use_order(&decoded.rows, decoded.manifest.as_ref())
    );
    let row_only = decode(&Pack::from_jsonl(&text).unwrap().encode().unwrap());
    assert!(decoded.strings.starts_with(&row_only.strings));
}
