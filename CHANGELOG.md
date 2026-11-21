# Changelog

## 0.1.0 — Unreleased

- **Behaviour change (declarative metadata).** Invalid UTF-8 anywhere in
  declarative metadata XML, in an attribute, a comment, a CDATA section or a
  name as well as in text, is now reported as a parse error; it was accepted
  silently before. This follows the quick-xml upgrade that fixes
  RUSTSEC-2026-0194.
- **Contract change (refs schema 2).** Refs flow between the stages as the
  refs pack (`docs/pack.md`): `sf-impact-extract` writes it by default and
  keeps `--format jsonl` for the same rows as refs.jsonl, and
  `sf-impact-changed` and `sf-impact-tests` read only the pack. A pack that is
  truncated, corrupt, of another version, or whose line 1 is not
  sf-impact-extract's is refused; refs.jsonl is refused by its schema version
  or as no pack. The refs provenance row says `"schema":2` wherever it
  appears, in refs and as embedded in changed.jsonl and impact.json. A pack
  holds each file once, of one package, so refs in which one file's rows name
  two packages, which extract never writes, can no longer be given.
- **Contract change (refs pack version 2).** `sf-impact-extract` writes the
  pack's manifest whenever the tree is in a git repository, and
  `--previous <pack>` (the same file as `--out`) reuses the rows of the
  files that have not changed, writing the pack that a full run writes.
  Where what a changed file declares has changed, it still reuses what the
  other files declare, and makes every file's rows again. The
  manifest keeps each class, interface and enum as extract declared it, in
  source order, where version 1 kept type facts sorted by owner; packs of
  version 1 are refused as of another version. The rows, and `--dump`, are
  unchanged.
- **Contract change (impact.json).** `collisions` lists only the collision
  groups that the query reached (a declaration in the walk, the seeds
  included); the others are counted, as groups, in the new
  `diagnostics.unreached_collision`, written only when not zero.
  `sf-impact-render` reads the new key and renders as before.

- Decode refs and changed rows directly into closed wire types, without the
  `serde_json::Value` clone/serialize/diff round-trip. Duplicate keys in refs and changed data
  rows are now refused; delta rows, provenance rows and whole documents still
  accept the last value. Generated fixtures are unchanged because their struct serializers
  never write duplicate keys.

- `sf-impact-extract` reads Apex, declarative metadata, Flows and LWC into sorted
  declarations and references, including unresolved names and coverage gaps.
- `sf-impact-delta` compares two git commits without changing the repository and
  writes changed files, line hunks and both versions' text.
- `sf-impact-changed` verifies input provenance and maps a diff onto symbols,
  permission grants and metadata bindings.
- `sf-impact-tests` walks incoming references, selects Apex tests with evidence,
  and reports possible tests, untested code and partial confidence.
- `sf-impact-render` verifies impact output and renders bounded PR comments or
  GitHub Actions step outputs, with explicit handling of empty selections.
- Release packaging supplies all five commands for Linux x86_64/aarch64
  (static musl) and macOS x86_64/aarch64, SHA-256 checksums, MIT and dependency
  notices, and a checksum-verifying installer for `~/.local/bin`.
