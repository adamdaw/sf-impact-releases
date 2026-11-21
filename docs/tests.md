# sf-impact-tests

Reads the refs pack and either `changed.jsonl` or explicit symbol queries, then
writes one `impact.json` document. It reads no source tree and runs no git.
The document is compact JSON on one line, ended by a newline; `jq . impact.json`
shows it indented.

```sh
sf-impact-tests --refs refs.pack --changed changed.jsonl --out impact.json
sf-impact-tests --refs refs.pack --symbol 'Invoice.total()' Invoice__c --depth 1
jq -r '.tests[].symbol' impact.json | sort -u
```

`--depth` defaults to 2. Parsed edges select `@IsTest` classes through that
many incoming hops, including callers of test helpers. Textual edges select
only directly: the test's own text must name a seed. A match on a member name
(method, field/property or nested member) also needs its owning class or any
enclosing type named in that same test file. The type reached through inheritance
or dispatch counts too. Evidence is a case-insensitive whole-word type-name row
or a parsed reference into that type; another textual member word is insufficient.
Otherwise the test goes to `possible` with `member_name_only` and its witness.
Textual matches on type names themselves are unchanged. Any longer path containing
textual evidence goes to **`possible`**, with its reason and witness. LWC seeds
never select Apex tests through text; those paths are possible even at the first
hop. Parsed LWC imports keep their normal direction and treatment.

A changed class seeds its members. A reached member does not open references to
its containing type, and a reached class does not reopen its members. Instead,
changed implementation and override methods seed the same signature on supertypes
and interfaces, with explicit dispatch evidence; sibling implementations are
excluded. Method changes expose their owner's CMDT bindings and textual mentions
in metadata records; invocable methods expose Flow actions naming their class.
These binding paths obey the same parsed/textual rule. Triggers connect to parsed
DML on their objects, including triggers reached from a changed handler.
A method seed with no parsed referrer after dispatch and bindings follows parsed
references to its owning type and explicit constructors, marked `platform_entry`.
This covers platform-called methods such as Queueable/Schedulable `execute` and
Batchable `start`, `execute` and `finish`, and other uncalled method seeds. Each
reference still costs one hop; other members and textual owner references are
not opened. This exception applies only to seeds, not reached methods. A Flow
action is a dynamic binding for class seeds as well as method seeds; if no test
route covers it, confidence is partial with `dynamic_binding`.

`tests` unions `changed`, `convention`, `direct`, `caller`, `trigger_dml` and `platform_entry`
reasons. A changed test class or method selects that test itself. Convention uses
the changed member's owning class. Helpers remain eligible and traversal continues
through them. `untested` lists changed production classes without selected tests,
and changed/reached triggers with no selected DML tests.

A nested `@IsTest` class is not valid Apex: “Classes defined as `@IsTest` must
be top-level classes”
([IsTest Annotation](https://developer.salesforce.com/docs/platform/apex-dev/references/apex-dev/apex-classes-annotation-istest.html)).
Source that has one is read all the same, and a test in it is selected as its
top-level class, sharing that class's one row, so that `tests` never has a
dotted name such as `Outer.Inner`, which no test run can be given. This is
defensive handling of input the platform rejects; it does not make that input
run. The inner name is kept: in the row's `path`, in its `seed` where the path
is empty, and for a convention match in `convention`, which lists the inner
test classes whose names matched, whichever witness the row has. The same holds
for `possible`. A top-level class that is not itself `@IsTest` is named too, so
that the evidence is kept and not dropped, but its name is not a valid
`RunSpecifiedTests` target: a deployment
[runs only test classes](https://developer.salesforce.com/docs/platform/api-metadata/guide/meta-deploy-run-specific-tests.html).

`possible` is a **disjoint list of tests that are not selected**, with
`member_name_only`, `textual_path` or `lwc_textual` reasons. Selection through another seed, parsed
path or convention puts a test in `tests` instead. The renderer shows possible-test
counts and reasons in a collapsed section. Each selected or possible test has the union of its
reasons and one shortest witness across all seeds; ties are deterministic.

[schema/impact.schema.json](../schema/impact.schema.json) documents every field.
Each seed reports only **direct** referrers grouped by source type, with separate
`parsed` and `textual` counts, including zero. These counts are independent of
`--depth` and count distinct input reference rows, not callers. Direct sites are
not copied per overload. Original rows live once in the document's `references`
map; referrers and witness hops use integer IDs. For example:

```sh
jq '. as $d | .tests[] | {symbol, reasons, sites: [.path[] | $d.references[.reference|tostring]]}' impact.json
jq '.possible[] | {symbol, reasons, path}' impact.json
```

Paths mark `member`, `dispatch`, `owner_binding`, `trigger_dml` and `platform_entry` expansions,
including inheritance and trigger-object proof IDs. The CLI serializes typed
output directly; it does not build a JSON value tree for every reached row.

`seeds` retains changed symbols and touched names, including removed symbols and
CMDT types. A non-removed changed symbol missing from refs is an error, with
partial confidence and exit status 1. Unknown symbol queries also write error
rows and exit 1. Queries compare case-insensitively against full symbols,
methods without signatures and short names, seeding and flagging every match.
Overloads and collision locations stay distinct; candidates are unioned.

An explicitly supplied `--config sf-impact.json` can override these defaults:

```json
{
  "testSuffixes": ["Test", "Tests", "_Test", "_Tests"],
  "excludeUnpackagedTests": false
}
```

The suffix list replaces the defaults. Unpackaged tests are included by default;
exclusion uses declaration `scope` provenance, for both selected and possible
tests.

Confidence is partial only for gaps that could involve the change: changed-file
gaps, missing changed declarations, scope errors, unresolved/ambiguous parsed
names matching seeds or symbols reached by parsed paths in a compatible namespace,
selected/reached Apex parse or skipped-file gaps, and
uncovered trigger/dynamic bindings. Reasons carry file and line (null if absent
from the input). Weak-only routes do not make unrelated names relevant. Unrelated
standard objects/fields and unchanged metadata already searched textually are
counted under `diagnostics`; they do not lower confidence. So is an
`unsupported_element` in a Flow that did not change; in a Flow that changed it
is a changed-file gap. Selection still proceeds on partial runs.

`collisions` lists the duplicate API identities that the query reached: a
group is listed when the walk reached any of its declarations, the seeds
included. The groups it did not reach are left out and counted, as groups,
under `diagnostics.unreached_collision`, which is written only when it is not
zero. Every `declaration` row still carries its own `collision` flag.

Before anything is walked, the first row of each input is read. With
`--changed`, the refs have to be of the head that `changed.jsonl` was made
for, by the same checks as `sf-impact-changed` makes: its commit or its tree, its
root, its scope, its config hash and the schema versions. Each first row is first held to its schema; for
`changed.jsonl` that takes in the two rows it was made of (`inputs`), which
have to be valid and to be what the row says of itself, and its
`confidence`, which is `partial` exactly where it gives a reason. A
mismatch writes nothing and exits 1 with one line that names the field,
both values and what to run. Refs of a `dirty` or `unknown` tree are
refused unless `--accept-unverified-refs` is given; the document then has
the confidence reason `unverified_refs`, and has it too where
`changed.jsonl` says that it was made of such refs. With `--symbol` there is
no head to hold the refs to: refs of any tree are read, if they start with
their `provenance` row.

Depth zero keeps direct referrers/counts, changed-test and convention selection.
One input may be `-` for stdin. Output defaults to stdout; `--out -` and
`--out stdout` also select stdout. Invalid input reports a line number; invalid
arguments exit 2. Only the explicit input and config files are read.

Tests and fixture pipelines live in `crates/sf-impact-tests/tests`. Regenerate the
schema-validated goldens with
`UPDATE_GOLDEN=1 cargo test --locked -p sf-impact-tests --test pipeline`.
That writes them compact, as the tool does; the committed goldens are kept
indented to be read, and the test compares them as JSON values, so indent them
again before committing (jq's default output is the committed form).
The binding pipeline uses only the relevant declarations and metadata from the
existing fixture project, and asserts its seed set, binding and empty test lists.

Corpus pins and hand-selected grep words are in `scripts/impact-symbols.json`,
counts in `scripts/impact-counts.json`, and every reviewed difference in
`scripts/impact-accepted.tsv`. Reproduce with:

```sh
scripts/reconcile-impact.py --bin-dir "$PWD/target/release" \
  --clone NebulaLogger=/path/to/NebulaLogger \
  --clone apex-rollup=/path/to/apex-rollup \
  --clone apex-recipes=/path/to/apex-recipes --out /tmp/impact-reconciliation
```

`scripts/measure-impact.py` replaces the large committed range output. It runs
all nine pinned pipelines and a pinned 1/5/10/20/40-class NPSP sample,
compares the default convention picker, and records names, reasons, counts,
output sizes, stage timings and peak RSS under the chosen output directory.
[The committed summary](../scripts/impact-ranges.md) contains the measured table.

```sh
scripts/measure-impact.py --bin-dir "$PWD/target/release" \
  --clone NebulaLogger=/path/to/NebulaLogger \
  --clone apex-rollup=/path/to/apex-rollup --clone NPSP=/path/to/NPSP \
  --out /tmp/impact-ranges
```

Known gaps: extraction misses instance-field reads. An
empty `changed.jsonl` is no longer taken for an empty diff: a file without
its `provenance` row is refused, and an empty diff has that row alone. The
tool does not choose deployment test levels or generate manifests.
