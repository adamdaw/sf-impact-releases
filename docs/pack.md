# The refs pack, schema 2: byte-level specification

A refs pack is a binary encoding of exactly the rows of `refs.jsonl`
(`schema/refs.schema.json`), in exactly their order, with an optional
manifest (`schema/refs-pack.schema.json`). The library is
`sf_impact_common::pack`; `sf-impact-extract --dump PACK` writes a pack's
rows back out as JSON Lines. It is what flows between the stages:
`sf-impact-extract` writes it by default (`--format jsonl` writes the
projection instead), and `sf-impact-changed` and `sf-impact-tests` read it.
They read line 1 first and hold it to the delta or to changed.jsonl, as they
did refs.jsonl's, then decode the whole pack before they use any row.

Extract encodes the rows of each file on their own, in parallel, with the
file's strings numbered in the order of their first use in it, and numbers
them again for the whole pack as it joins the files in path order. A string's
first use in the pack is its first use in the first file that uses it, so the
pack is the one that encoding all the rows at once gives.

The pack is a function of its content alone. Equal rows (and equal
manifests) always give byte-equal packs from one build of the compressor,
and a reader refuses a pack whose content is not in canonical form (below).

## Layout

```
pack       = provenance LF header section{5,6} checksum
provenance = the provenance row of refs.jsonl, as one line of compact JSON,
             with "schema":2. No LF inside it. `head -1` prints it.
             At most 65,536 bytes (below).
header     = "SFIP" version          ; the four ASCII bytes, then version, an int: 2
section    = kind size length frame
             kind   : 1 byte
             size   : int, the payload's length in bytes
             length : int, the frame's length in bytes
             frame  : one zstd frame whose content is the payload
checksum   = 32 bytes: SHA-256 of every byte before it, line 1 and its LF included
```

The sections come in this order, each exactly once, and the manifest at
most once:

| kind | section | payload |
|---|---|---|
| 1 | strings | the string table |
| 2 | files | the scope error count and the file table |
| 3 | records | scope errors, then declarations and diagnostics |
| 4 | parsed | references of the `parsed` tier |
| 5 | textual | references of the `textual` tier |
| 6 | manifest | optional: reuse facts, for a writer that keeps them |

A reader checks, in this order: line 1 is at most 65,536 bytes, looked
for only that far and refused unread if it is longer (real packs' are
313 bytes, LFC, and 429, NPSP; a writer refuses a longer one too); line 1
is a provenance row of
`sf-impact-extract` with `"schema":2` (another schema is refused by
number); the header is `SFIP` and the version is 2 (another version is
refused by number); the checksum; every section's header, before any
frame is decompressed: a missing, repeated, unknown or misplaced section
is refused, and so is anything between the last section and the checksum,
and a reader keeps no header beyond the seventh, which is refused already;
the decompression budget (below); then each section, whose frame must be
exactly one zstd frame, must ask for a window of at most 4 MiB, must
decompress to exactly `size` bytes, and whose
payload must be read to its last byte.

**Decompression limits.** `size` is the pack's own claim, so a reader
bounds it by what is trusted, the bytes that are there, before it
allocates or decompresses anything. With `frame_length` a section's
`length` and `file_length` the length of the whole pack in bytes:

- each section: `size <= max(65536, 32 × frame_length)`;
- all sections: `sum of sizes <= max(65536, 16 × file_length)`;
- each frame's header asks for a window of at most 4 MiB (4,194,304
  bytes): its window descriptor, or for a single-segment frame its content
  size, read from the header before the frame is decompressed (RFC 8878,
  3.1.1.1). A level-3 writer uses at most 2 MiB.

The real packs this format was measured on (NPSP and LFC refs) expand at
most 9.3 times in a section (LFC; NPSP 7.1) and 4.5 times in all (LFC;
NPSP 3.4), so the ratios leave about three and a half times the headroom.

**Decoding limit.** Rows hold their own copies of the strings they use,
so a short pack whose rows use one long string many times, or one long
list of a short string, would take far more memory than its sections. A
reader counts what it allocates from the pack as it decodes, and charges
each allocation before it makes it:

- the string table: each string twice (the table's copy and the copy that
  finds repeats), 24 bytes an entry for the table, and for the map that
  finds repeats `3 × count + 9` buckets of 33 bytes (a power of two
  buckets at most 7/8 full, an entry and a control byte each);
- the rows: the size of a row (160 bytes in this build) for every row the
  files section declares, all charged before the first row is decoded;
- every list, at the size of its element for the count the pack gives:
  24 bytes a string for annotations, candidates and parameters, and the
  size of a manifest entry, type, callable, field or relationship; 32 bytes a file
  for the files that have rows;
- the bytes of every string it hands to a row or to the manifest, and of
  every blob id;

and refuses the pack once

- `taken > max(16 MiB, 384 × file_length)`.

Real packs take 91 times their length (NPSP) and 135 times (LFC); refs.jsonl
is 79 and 120 times the pack. The ratio is nearly three times the larger.

So for a 4 MiB pack a reader asks its allocator for at most about
1.57 GiB: 1.5 GiB charged (384 × 4 MiB), 64 MiB of decompressed sections
(16 × 4 MiB), the 4 MiB of the pack, line 1 (at most 64 KiB) and its
parse, and a few fixed buffers. A diagnostic never copies a pack's
string whole: past 128 bytes it shows the first 128 and the length.

The allocator adds its own overhead to that. glibc's malloc serves a
request of n bytes from its heap with a chunk of at least 32 bytes,
n + 8 rounded up to 16: at most 32 − n bytes more for n up to 24, and
at most 23 bytes more above that. One of 128 KiB or more it maps, for
under a page more. What decoding charges, it allocates in two kinds:

- the bytes of a string or a blob id, charged with its 24-byte header
  (counted in the row, list element, manifest entry, table entry or map
  bucket that holds it). The worst case is a one-byte string: 25 bytes
  charged, a 32-byte chunk, 31 bytes more, 1.24 times what it is charged;
- the elements of a collection (the rows, the table, the map, a list),
  charged at their own size. Every element is at least 24 bytes, so a
  collection that allocates asks for 24 bytes, 32, or 48 or more, and its
  overhead is at most half its charge (the worst, 16 bytes more on 32).

So the charged bytes cost at most 2.24 times themselves: about 3.4 GiB in
all for a 4 MiB pack of one-byte strings, on glibc. The commands allocate with
mimalloc, on every target, which rounds differently; this bound is derived for
glibc's malloc only.

Measured peak RSS (GNU time's `%M`, a process that runs one
`Pack::decode` of one pack, glibc release build with glibc's malloc, 2026-11-17): NPSP's
2.2 MB pack about 245 MiB, LFC's 1.5 MB pack about 239 MiB; the 42 KB
pack whose one parameter list has 680,000 entries is refused, at about
27 MiB. A pack beyond any limit is refused, whatever its checksum says.

Writers compress each payload with zstd at level 3, one frame per
section. The frame bytes are the compressor's: a reader holds the
payloads to canonical form, not the frames. A writer stays within the
decompression limits: a section whose frame is past its own limit is
stored instead, as one zstd frame of raw blocks with a 128 KiB window,
and while the sections together are past the pack's limit, the section
that compresses best among those not stored is stored too. No real pack
is past either limit, so a real pack's frames are the compressor's.

## Values

- **int**: unsigned LEB128, in its shortest form (a multi-byte int does not
  end in a 0x00 byte), at most 64 bits. Lines, columns, counts and string
  ids fit 32 bits.
- **delta**: a signed difference as an int, zigzag-encoded: `(d << 1) ^ (d >> 63)`.
- **id**: an int, an index into the string table.
- **opt**: an int: 0 for absent (or `null`), otherwise id + 1. Whether an
  absent value is written as `null` or left out is refs.jsonl's rule for
  the field: `referrer` is `null`, `source_field` is left out; in the
  manifest's logical form `child_name` and `controlling` are `null`.
  Likewise an empty `candidates` and an absent `scope` are left out.
- **tag**: one byte, an index into a list below.
- **opt tag**: one byte: 0 for absent, otherwise tag + 1.
- **ids**: a count, then that many ids.

## The string table (kind 1)

A count, then that many strings, each a byte length and that many bytes of
UTF-8. No string appears twice. Every string is used, and strings appear in
the order of their first use in this traversal:

1. the rows, in row order, each row's string fields in the order that
   refs.jsonl writes them:
   - `scope_error`: `path`;
   - `parse_error`, `skipped_file`: `package`, `file`;
   - `unmodelled_file`: `package`, `file`, `suffix`;
   - `declaration`: `symbol`, `name`, `package`, `file`, then each of `annotations`;
   - `reference`: `referenced`, each of `candidates`, `referrer`, `source_field`, `package`, `file`;
   - `unsupported_element`: `package`, `file`, `element`;
2. then the manifest, if there is one: each entry's `package` and `path`,
   in entry order; each type's `symbol`, `superclass` and `interfaces`,
   then each of its methods' and then its constructors' `symbol`, `name`,
   `params` and `returns`, then each of its fields' `symbol`, `key` and
   `ty`; each relationship's `field`, `to`, `child_name` and `controlling`.

A row-only pack's table is therefore a prefix of the table of the same rows
with a manifest.

## Row order and the file table (kind 2)

Rows are in refs.jsonl order: `scope_error` rows first, then the rows of
each file, the files in path order (by bytes), and every row of a file
together. The files payload is:

```
scope_errors : int, the number of scope_error rows
files        : int, then for each file:
  package    : id
  path       : id      ; strictly increasing, by bytes, file to file
  rows       : int     ; at least 1
  tags       : rows bytes, the kind of each of the file's rows, in order
```

Row tags: 0 `parse_error`, 1 `skipped_file`, 2 `unmodelled_file`,
3 `declaration`, 4 `reference` of tier `parsed`, 5 `reference` of tier
`textual`, 6 `unsupported_element`.

The three row sections are file-major in the same file order. A reader
rebuilds the rows of a file by reading its tags in order and taking each
row's fields from the next record of that tag's section. Nothing is
sorted on reading. A row's `package` and `file` are its file's.

## Records (kind 3)

The scope errors, each `path` id, `declared_in` tag, `reason` tag. Then,
for each file and each of its rows tagged 0, 1, 2, 3 or 6, in row order:

| tag | row | fields |
|---|---|---|
| 0 | `parse_error` | `line` int |
| 1 | `skipped_file` | `reason` tag |
| 2 | `unmodelled_file` | `suffix` id |
| 3 | `declaration` | `symbol` id, `name` id, `type` tag, `start` int, `end` delta from `start`, flags byte, `annotations` ids |
| 6 | `unsupported_element` | `element` id, `reason` tag, `line` int, `column` int |

Declaration flags: bit 0 `is_test`, bit 1 `collision`, bits 2–3 `scope`
(0 absent, 1 `packageDirectories`, 2 `unpackagedMetadata`); the other bits
are 0.

## Parsed references (kind 4)

For each file and each of its rows tagged 4, in row order: `kind` tag,
`referenced` id, `referenced_type` opt tag, `resolution` opt tag, `detail`
opt tag, `candidates` ids, `referrer` opt, `source_field` opt, `line` delta
from the line of the file's previous row tagged 4 (from 0 for its first),
`column` int. `tier` is `parsed`.

## Textual references (kind 5)

Textual rows are grouped as an encoding detail. For each file with rows
tagged 5, in file order, groups until the file's rows tagged 5 are used up:

```
referrer : opt
count    : int, at least 1, no more than the file's rows still to come
count times:
  referenced : id
  line       : delta from the file's previous textual row (from 0 for its first)
  column     : int
```

Groups are runs in the order of the file's textual rows, whatever rows of
other tags fall between them: two consecutive groups of one file have
different referrers, and the line delta runs on across groups. `tier` is
`textual` and `kind` is `text`.

## The manifest (kind 6)

Written by `sf-impact-extract` whenever the files it reads have git blob
ids, and read back by `sf-impact-extract --previous`, which reuses the rows
of the files that have not changed ([Reusing the previous pack](ci.md#reusing-the-previous-pack)).
Any other reader decodes it with the same strictness as the rows, and
refuses a malformed one, but no output depends on it and `--dump` does not
print it.

```
entries : int, then for each file in scope (with rows or not):
  package : id
  path    : id     ; strictly increasing by bytes
  blob    : one byte, 20 or 32 (the same for every entry), then that many bytes:
            the git blob id of the bytes that were read
types   : int, then for each class, interface and enum:
  file         : int: the type's entry index minus the previous type's
                 (the index itself for the first)
  symbol       : id
  superclass   : opt
  interfaces   : ids
  methods      : int, then for each: symbol id, name id, params ids, returns opt
  constructors : int, then for each, as for methods (returns is absent)
  fields       : int, then for each: symbol id, key id, ty id,
                 then one byte: bit 0 is_static, bit 1 is_enum_value, the rest 0
relationships : int, then for each:
  file        : int: as for types, from the previous relationship
                (the index itself for the first: the count restarts)
  field, to   : id, id
  child_name, controlling : opt, opt
```

Every file that has rows is an entry, with the same package. Types are in
order of their entry index, and the types of one file, and each type's
interfaces and members, are in the order that the file declares them:
resolving a reference depends on that order, so it is content, and a
reader keeps it. A type's names are as written; a callable's owner is its
type and its key its name case-folded, so neither is stored, and a field
stores its key, its name case-folded, which is all that resolving reads of
it. Relationships are strictly increasing by (entry index, `field`, `to`,
`child_name`, `controlling`), absent before present.

Version 1 of the layout stored type facts sorted by owner and fact, which
lost the order of a type's members and could not hold two equal facts, so
a type could not be rebuilt as it was declared. Version 2 stores the types.

## Tag lists

Each list is the schema's values in this order, numbered from 0.

- `type` (declaration): class, interface, enum, trigger, method,
  constructor, field, enum_value, object, object_field, validation_rule,
  label, custom_permission, message_channel, permission_set, profile,
  metadata_record, flow, lwc.
- `kind` (reference): call, construct, type, cast, extends, implements,
  static_access, soql_object, soql_field, dml_object, dml_field,
  sobject_field, trigger_object, lookup, lookup_filter, formula, summary,
  controlling_field, display_field, value_set, grant, cmdt, cmdt_field,
  cmdt_type, flow_object, flow_field, flow_action, flow_subflow,
  flow_component, flow_label, flow_custom_permission, flow_custom_metadata,
  lwc_apex, lwc_schema, lwc_label, lwc_custom_permission,
  lwc_message_channel, lwc_component, lwc_target, lwc_object, text.
- `referenced_type`: class, method, constructor, field, enum_value,
  trigger, object, object_field, validation_rule, label, custom_permission,
  message_channel, permission_set, profile, metadata_record, flow, lwc,
  tab, global_value_set.
- `resolution`: declaration, external, unresolved.
- `detail`: ambiguous_overload, ambiguous_receiver, unknown_type, no_member.
- `declared_in`: packageDirectories, unpackagedMetadata.
- `reason` (scope error): missing, not_a_directory, outside_root.
- `reason` (skipped file): binary.
- `reason` (unsupported element): not_interpreted, no_name.

## Rows are held to refs.jsonl's rules

A decoded row must be one that the refs.jsonl reader accepts: positive
lines, a declared `referenced_type` where it is required, `candidates` only
with an ambiguous detail, and so on. A writer refuses to write a row that
breaks them, and a reader refuses a pack that holds one.
