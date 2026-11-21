# sf-impact — design

This document describes what sf-impact is, the principles it follows, the stages of its pipeline and how its results are checked.

## 1. What it is

A small set of command-line tools that answer three questions about a
Salesforce DX repository:

1. **Who references this?** Given a symbol (an Apex class or method, an object,
   a field, a Flow, a custom metadata type), list everything in the repo that
   refers to it, with a count and a reason for each row.
2. **What does this diff touch?** Given a base and a head commit, list the
   symbols a change adds, modifies or removes.
3. **Which tests should run?** Given that diff, select the Apex test classes
   that exercise what changed, with a reason for every selection.

It covers the mix that real Salesforce projects contain: Apex, triggers, Lightning
Web Components, objects and fields, validation rules, Flows, permission sets,
profiles and custom metadata records.

It does nothing else. There is no persistent index, no server, no database, no
embeddings, no editor plugin and no AI integration layer. Every run reads the
tree it is pointed at and finishes. A file that a run wrote may be kept and
used again, but only by a run that it is verifiably for: see
[What may persist](persistence.md#what-may-persist).

## 2. Principles

- **Small tools, one job each.** Separate binaries, connected by files or pipes
  carrying JSON Lines. Any stage can be replaced, inspected with `jq`, or fed by
  something else. One exception today: `sf-impact-changed` links
  `sf-impact-extract` as a library to read the base version of each changed
  file. A replacement extractor's `refs.jsonl` is therefore compared against base
  declarations that this extractor read. Replacing the extractor means replacing
  `sf-impact-changed` too, until that dependency is removed.
- **Schemas are the contract.** `schema/*.schema.json` defines every file that
  passes between tools. Shared code is a convenience, not the contract.
- **Stateless.** Extraction is of the tree on disk. A worktree that is hundreds
  of commits behind gives correct answers for that tree, with no index to go
  stale. A file kept from an earlier run says what it is of, and is refused
  by a run that it is not for.
- **Fast on small machines.** Static binaries, parallel file walking, no runtime
  to install. The target is a free-tier CI runner, not a workstation.
- **Never a silent zero.** Every reference is labelled with how it was found.
  A symbol with no parsed references but some textual matches is reported as
  such. Unresolved references and unmodelled files lower the reported
  `confidence`; they are never dropped.
- **Deterministic.** The same tree gives byte-identical output. Results are
  sorted.
- **Touch nothing.** The tools never modify the files of the analysed repository. Output goes only where the caller says, with `--out`.

## 3. Pipeline

Five stages, one job each, with a file between any two: a refs pack (`docs/pack.md`) out of `extract`, and JSON Lines (or JSON, out of `tests`) everywhere else.

| Tool | Input | Output |
|---|---|---|
| `extract` | a tree (`--root`) | the refs: `declaration` rows (symbol, type, package, file, start, end) and `reference` rows (referrer, referenced API name, kind, tier, file, line), as a refs pack (schema 2, `docs/pack.md`), or as `refs.jsonl` with `--format jsonl`. |
| `delta` | `--base`, `--head`, a git repository (`--root`) | `delta.jsonl`: the two commits, every file that is not the same in both (what became of it, its path, blob and mode at each), the lines of it that differ, and the text of both versions. |
| `changed` | `delta.jsonl`, the refs pack | `changed.jsonl`: changed symbols, including `removed` ones (parsed from the base version of each changed file, not a full base extraction). |
| `sf-impact-tests` | the refs pack, plus `changed.jsonl` or `--symbol`; `--depth` | `impact.json`: one compact JSON document with referrers grouped by type, both tiers and counts, test selection with reasons and paths, `untested`, `confidence`, `unresolved`, and `collisions` that the query reached (the others counted in `diagnostics.unreached_collision`). |
| `sf-impact-render` | `impact.json` (schema and provenance verified), `--format md\|gh-step` | Bounded Markdown for a pull-request comment, or `$GITHUB_OUTPUT` values with explicit empty selection. |

### What differs between two commits (`delta`)

- `delta` is the stage that reads a repository, and it asks git only for
  objects: which commit a name is, where two commits part, what a tree holds
  and what a blob is. It does not run `git diff`, whose answer depends on
  configuration, attributes and the environment. The output is a function of
  the two commits. `extract` asks git one thing more, to say which commit
  it read: see [What may persist](persistence.md#what-may-persist).
- It reads the objects the repository has and fetches none. An object that
  a partial clone was not given is an error that names it. Nothing is
  written but the output.
- A repository that another user owns is read only when told to
  (`--trust-repo`).
- Lines are compared in the tool, as git compares them by default. Nothing
  is ignored.
- No renames are detected. A path only the base has is a file deleted and a
  path only the head has is a file added. The same blob under another path
  is a file that moved.
- A file is text if and only if it is valid UTF-8, or UTF-16 (LE or BE) with a BOM that decodes without error, and the decoded content contains no NUL (U+0000). An optional UTF-8 BOM is preserved; a UTF-16 BOM is removed.
  A regular file that is not text is binary; its lines are not compared.
  Text lines are compared after decoding.
- In CI the base is the merge base, not the tip of the target branch.

### Mapping it to symbols (`changed`)

- `changed` reads `delta.jsonl` and `refs.jsonl` and nothing else. It runs
  no git. It maps nothing until it has verified that the refs are of the
  head of the delta.
- Apex: each hunk maps onto declaration line ranges at `head`. A hunk inside a
  method marks the method; a hunk outside any method marks the class.
- Metadata: SFDX source format stores one symbol per file, so a changed file
  marks its symbol. Permission sets, profiles and custom metadata records also
  report the API names on their changed lines ("grants touched").
- A file that moved and changed is a deleted file and an added file in the
  delta. If the two declare the same symbol in the same package, they are
  one file, and only what differs between them is changed. If not, what
  they declare is removed and added.
- A changed file the model does not cover sets `confidence: partial`.

### What may persist

Nothing has to. Every run can extract the tree it is given and finish, and
that is the default: no index is committed, and none is needed.

- **A `refs.jsonl` may be kept if what it says of itself matches.** It can
  be a CI cache (`actions/cache`) keyed by the head commit, or a local
  file. It is never committed to the repository. Kept files are an
  optimisation, never an input the tools ask for.
- **Each file says what it is of.** `refs.jsonl`, `delta.jsonl` and
  `changed.jsonl` start with a `provenance` row: the tool and its version,
  the schema version, where the project is in its repository, the scope
  and a hash of the config that took effect. Refs name the tree they were
  extracted from, a delta its two commits and the tree of its head, and
  `changed.jsonl` both of those and the first rows of the two files it
  read. The row holds no time, no machine and no path, so output stays
  byte-identical.
- **Refs name a commit only on proof.** Before it reads a file, `extract`
  asks git which commit is checked out and what it holds. It then holds
  what it reads to that commit by the bytes it read: the project file, the
  config and every file in scope, each by the id git gives such content,
  worked out in the tool. If they are the same the tree is that commit; if
  not it is `dirty`; with no repository it is `unknown`. Files outside the
  scope do not count, and a file in scope that git ignores does. Git is
  asked with plumbing alone, under the same environment as `delta`, never
  after a file is read, and nothing is written.
- **The tools verify, and refuse what does not match.** `changed` and
  `sf-impact-tests` read the first rows before anything else, and hold each to
  its schema: a row that is not valid is refused, and no value of it is
  carried into the next file. The refs must be of the head: the same
  commit, or another commit with the same tree, of the same project of
  the repository, with the same scope and config, and of a schema version
  the reader knows. A mismatch is one line that names
  the field, both values and what to run, and a non-zero exit. A file
  without the row is refused as an older format. Nothing stale is used in
  silence.
- **Unverifiable refs need a word.** Refs of a `dirty` or `unknown` tree
  cannot be held to a commit. They are refused unless the caller passes
  `--accept-unverified-refs`, and then the output says so: the first row of
  `changed.jsonl` is `partial` with the gap `unverified_refs`, and
  `impact.json` carries that as a confidence reason.
- **The tools never write into the repository.** Where a kept file lives
  is the caller's choice, with `--out`.

Refs name the commit they were extracted from, so a file kept in a cache
matches the commit it was keyed by.

A minimal GitHub Actions cache, keyed by the head commit, the tool's
version and a hash of the config. The key only finds the file; the check
is what makes it safe to use:

```yaml
- uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4
  with:
    fetch-depth: 0
- id: refs-key
  run: |
    echo "head=$(git rev-parse HEAD)" >> "$GITHUB_OUTPUT"
    echo "tool=$(sf-impact-extract --version | cut -d' ' -f2)" >> "$GITHUB_OUTPUT"
- id: refs
  uses: actions/cache@0057852bfaa89a56745cba8c7296529d2fc39830 # v4
  with:
    path: ${{ runner.temp }}/refs.jsonl
    key: refs-${{ steps.refs-key.outputs.head }}-${{ steps.refs-key.outputs.tool }}-${{ hashFiles('sf-impact.json') }}
- if: steps.refs.outputs.cache-hit != 'true'
  run: sf-impact-extract --root . --out "$RUNNER_TEMP/refs.jsonl"
- run: |
    sf-impact-delta --merge-base origin/main --head HEAD |
      sf-impact-changed --delta - --refs "$RUNNER_TEMP/refs.jsonl" --out "$RUNNER_TEMP/changed.jsonl"
```

### Runtime

Rust. Apex parsing uses the `tree-sitter-sfapex` grammar; XML uses `quick-xml`;
line comparison uses `imara-diff`; file walking is parallel. Releases ship one static musl binary per tool for Linux
(x86_64, aarch64), and native binaries using only system libraries for macOS
(x86_64, aarch64). Installing in CI should take a few seconds.

Measured with grammar 3.0.2 and tree-sitter 0.27: a static
musl binary is about 2.5 MB on both architectures; 245 Apex files (2.9 MB)
parse in about 0.25 s with under 20 MB of memory, and 387 files from two
public corpora parsed with no errors. File reading dominates, so the worker
count is capped (eight workers were barely faster than one, and 32 were slower than one).
Files that fail to parse are reported with the line of the first error and
still feed the `textual` tier; they set `confidence: partial`. SOQL built at run time is a
string, so only the `textual` tier sees it. The grammar is MIT-licensed and its
published crate omits the license file, so release archives include a notice.

## 4. Reference model

Two tiers, always both, every row labelled.

### `parsed` (from syntax)

- **Apex:** method and constructor calls, type references in declarations and
  casts, `extends` / `implements`, static member access, SOQL and DML object and
  field names, annotation targets (`@InvocableMethod`, `@AuraEnabled`,
  `@RestResource`), `@IsTest`. Overloads are kept apart. Names are compared
  case-insensitively, as Apex does.
- **Objects, fields, validation rules:** names used in formulas and rules.
- **Flows:** record operations (object, field), Apex actions, subflows.
- **LWC:** `@salesforce/apex/Class.method`, `@salesforce/schema/Object.Field`,
  `js-meta.xml` targets.
- **Permission sets and profiles:** `<field>`, `<apexClass>`, `<object>`,
  `<flow>`, `<tab>` entries, as kind `grant`.
- **Custom metadata records** (`*.md-meta.xml`): every string value that
  matches a declared API name, and the record's own `DeveloperName` when it
  matches one, as kind `cmdt`. This catches frameworks that bind Apex classes
  through metadata records (trigger-action frameworks and logger plugins, for
  example), where the class has no Apex callers at all and is loaded with
  `Type.forName`. Some frameworks name the class in a field value; others use
  the record's name itself. A value can also match a class name by
  coincidence (a cache partition named like a class, say), so `cmdt` rows
  carry the field they came from and the reviewer can judge. A changed record
  also marks its type, so Apex that queries the type shows up through the SOQL
  rule.
- **Resolution:** to a declaration where one exists; `external` for a known
  installed-package namespace (this does not lower confidence); `unresolved`
  otherwise.

### `textual` (from names)

A case-insensitive whole-word match of every declared API name across every
file in scope, Apex string literals included. This is the grep a careful
reviewer would run by hand, done once for every symbol. Names shorter than six
characters are skipped unless they appear in an allow-list in the config file.

A file is text if and only if it is valid UTF-8, or UTF-16 (LE or BE) with a BOM that decodes without error, and the decoded content contains no NUL (U+0000). An optional UTF-8 BOM is preserved; a UTF-16 BOM is removed.
A file that is not text produces a `skipped_file` row and a gap, never a silent zero.

## 5. Scope and identity

- **Scope** is the `packageDirectories` list in `sfdx-project.json`, plus any
  `unpackagedMetadata` paths they name. Nothing else is read. Paths are
  normalised before use: some projects write Windows separators
  (`plugins\\Name`), which must resolve on Linux and macOS too.
- **Identity** is `(package, type, ApiName)`, but it is not guaranteed unique:
  the same API name can appear twice in one package directory as well as
  across packages. Every duplicate is reported as a `collision`, with all
  candidates kept; none is resolved silently.
- `unpackagedMetadata` is in scope for test selection by default. Package
  projects often keep most of their tests there. A config flag excludes it.
- **Config** is an optional JSON file at the repository root: test-class
  suffix (`Test`, `Tests`, `_Test`, `_Tests`), the textual minimum length and
  allow-list. A repository that needs none of these ships no file.

## 6. Test selection

Select on strong evidence and make weak evidence visible. The default depth is
2 incoming reference hops:

- Parsed edges select tests through the configured depth. `@IsTest` helper
  classes do not end the walk: their callers can be tests too.
- Textual edges select only at the first hop: the test's own text names a seed.
  A member-name match (method, field/property or nested member) also requires
  the member's owning class or any enclosing type named in that same test file.
  A type reached through inheritance/dispatch also counts. A whole-word,
  case-insensitive type-name row or a parsed reference into that type establishes
  this evidence; another textual member word does not. Without it the test goes
  to `possible` with `member_name_only` and its witness. Textual type-name matches
  themselves are unchanged, as are parsed edges.
  A longer path containing any textual edge is evidence for `possible`, never
  selection. This also applies when the textual edge is first and a later edge
  is parsed.
- An LWC seed never selects Apex tests through text. Such paths go to `possible`.
  Parsed references, including `@salesforce/apex` imports, retain their ordinary
  direction and treatment.
- A changed class seeds its members. A member reached later does not expose
  referrers of its containing type, and a reached class does not open its members.
- A changed implementation/override method also seeds the identical signature
  on its supertypes and interfaces, never sibling implementations. Dispatch is
  marked with its inheritance evidence.
- Method changes expose CMDT bindings to their owner, including textual mentions
  in metadata records; invocable methods expose Flow actions naming the class.
  These are marked bindings, not general containment expansion. Parsed binding
  paths select; textual binding paths obey the first-hop rule.
- A changed or reached trigger selects tests through parsed DML on its object.
  A trigger without selected DML tests is `untested` with a confidence reason.
- A method seed with no parsed referrer after dispatch and bindings follows
  parsed references to its owning type and explicit constructors, marked
  `platform_entry`. This covers Queueable/Schedulable `execute`, Batchable
  `start`/`execute`/`finish`, and any other uncalled method seed. It does not open
  other members or textual owner references, and never applies to reached
  methods. Each reference still costs one hop.
- Flow actions are dynamic bindings for class seeds as well as method seeds;
  an uncovered action gives `partial` confidence with `dynamic_binding`.

Selection unions `convention`, `changed`, `direct`, `caller`, `trigger_dml` and
`platform_entry`. `convention` appends `Test`, `Tests`, `_Test` or `_Tests` to the changed owning
class (configurable), and requires an existing `@IsTest` class. `changed` selects
the seed's own test class, including a changed test method. Unpackaged tests are
included by default; explicit exclusion applies to both selected and possible
classes. A changed production class with no selected test is `untested`.
A nested `@IsTest` class is not valid Apex. Where source has one all the same,
a test in it is selected, or possible, as its top-level class, so that no
dotted name is emitted; the witness, or `convention` for a convention match,
keeps the inner name. This handles input the platform rejects and does not
make it run: a top-level class that is not `@IsTest` is still named, to keep
the evidence, though no test run takes it.

`possible` is a first-class, disjoint list of test classes with `member_name_only`,
`textual_path` or `lwc_textual` reasons and a shortest witness. A class selected
by another seed or strong path is in `tests`; possible evidence never promotes it. Rendering must
show `possible` prominently rather than hide uncertainty. Each selected or
possible class has one shortest witness across seeds and a union of reasons.
Direct referrers and per-tier counts are independent of traversal depth. Original
reference rows are stored once and linked by ID, including dispatch/trigger proof.

`confidence` is `partial` only for gaps that could involve the change: a changed
file outside the model or with parse errors; a missing non-removed changed
symbol declaration; scope errors; an unresolved or ambiguous parsed reference
matching a seed or a symbol reached by parsed paths in the appropriate namespace;
selected/reached Apex parse gaps; unknown scope when exclusion is requested; or
uncovered trigger/dynamic bindings. Each reason gives its file and line (null
when the input supplies none). Weak-only paths do not make unrelated names
relevant. Unresolved standard objects/fields unrelated to the change and gaps in
unchanged metadata already searched textually are diagnostic counts, not reasons
for partial confidence.

A partial run still selects tests. The tools never choose deployment test levels
or build deployment manifests. Instance-field reads missing from extraction are a
follow-up. An empty `changed.jsonl` is no longer taken for an empty diff: a file
without its provenance row is refused, and an empty diff has that row alone.

## 7. Layers of coverage

1. **Apex end to end.** `extract` for `.cls` and `.trigger`, the textual tier,
   `changed`, `impact`, `render`. On its own this selects tests from references
   and changes, not from names alone.
2. **Declarative metadata.** Objects, fields, validation rules, Flows, LWC,
   permission sets, profiles, custom metadata records.
3. **CI.** A GitHub Actions example, `refs.jsonl` cached by head commit (see
   [What may persist](persistence.md#what-may-persist)), the `gh-step` renderer and the
   pull-request comment.

## 8. How correctness is checked

1. **Fixture project** in this repository: a synthetic SFDX project with at
   least one of every reference kind, and golden `refs.jsonl` / `impact.json`
   files. Public CI runs this. It must cover the kinds no public corpus does:
   `<flow>` grants in permission sets and profiles, custom metadata records
   that name a Flow, two different classes sharing an API name, overloads,
   case-mismatched references and validation-rule formulas.
2. **Open-source corpora:** pinned commits of public SFDX projects, with
   expected counts for chosen symbols checked against a hand-verified grep.
   Each count either matches or the difference is explained. The commits are
   pinned in `scripts/impact-symbols.json`. The corpora:

   | Corpus | Why |
   |---|---|
   | `jongpie/NebulaLogger` (MIT) | Eight package directories, metadata-bound plugins, Flow Apex actions, LWC imports, large grant lists |
   | `jamessimone/apex-rollup` (MIT) | Record-triggered Flows calling invocable Apex, profiles, a collision, binding by record name, Windows-style paths |
   | `trailheadapps/apex-recipes` (CC0) | One package, small enough to verify by hand, a wide Apex surface |
   | `UnofficialSF/LightningFlowComponents` (BSD-3) | Stress corpus: 280 Flows, subflows, many real collisions |

   `SalesforceFoundation/PMM` and `NPSP` cover installed-package
   namespaces (`external`) and scale. Every candidate is a library or sample
   app; no public repository looks like a typical customer org, so each user's
   own repository remains the final test.
3. **Dependency API calibration** (planned): check against Salesforce's
   `MetadataComponentDependency` for an org deployed from the same commit, and
   report the miss rate by type. That API is Beta and capped, so it serves as an
   oracle, never as an input.

## 9. Install and use

`sf-impact` installs five commands: `sf-impact-extract`, `sf-impact-delta`, `sf-impact-changed`,
`sf-impact-tests` and `sf-impact-render`. No package manager or language runtime is required.
Linux x86_64/aarch64 builds are static musl; macOS x86_64/aarch64 builds use only
system libraries and target macOS 13 or later. Git is needed for repository history.

The POSIX `install.sh` downloads one release archive and its SHA-256 checksum,
verifies before unpacking, and installs into `~/.local/bin` without sudo or profile
changes. It prints a PATH hint only when needed. `--prefix` chooses the directory;
`--version v0.1.0` pins a release (the default is latest). CI downloads the script
from the same tag it passes to `--version`; see the [Actions example](render.md#example-github-actions).
The first tag has not been published yet.

A version tag (`v<MAJOR>.<MINOR>.<PATCH>`) builds all five commands on four native
GitHub-hosted runners, then creates the release with archives, individual
checksums and `SHA256SUMS`.
Each archive includes LICENSE, THIRD_PARTY_LICENSES and a short README.
Pinned cargo-about generates notices from Cargo.lock and rejects licenses outside
an explicit allow-list; the missing upstream sfapex MIT notice is vendored.
Publishing requires a tag matching the Cargo version on `origin/main` and a full
CI run on that commit. Only the publishing job has write permission. Packaging
PRs exercise all targets without publishing. [Release maintenance](releases.md) records the build choices,
reproducibility limits and verification commands. Release provenance (GitHub attestations) is described in [release maintenance](releases.md).

No setup is needed in the analysed project:

```sh
sf-impact-extract --root . --out refs.jsonl
sf-impact-tests --refs refs.jsonl --symbol 'Class.method'
sf-impact-delta --merge-base origin/main --head HEAD --out delta.jsonl
sf-impact-changed --delta delta.jsonl --refs refs.jsonl --out changed.jsonl
sf-impact-tests --refs refs.jsonl --changed changed.jsonl --out impact.json
sf-impact-render --impact impact.json --format md
```

## 10. Non-goals

Aura components; the effect of permission changes on actual users (a
repository has no record of users); comparing open pull requests with each
other; a server or MCP endpoint; any persistent index or registry; editing
any file in the analysed repository; deploy
manifests; deploy test levels.

## 11. Open questions

- Whether `partial` confidence should ever trigger a stricter test rule.
- Whether the Dependency API lasts long enough to justify the calibration job.
