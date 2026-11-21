# Development

The version 1 schemas use closed objects and reject unknown schema versions.
Adding any field, including an optional field, requires a schema version bump.
Delta versions name their source `file`, as references and changed rows do.
Changed and impact outputs both use `gaps` for confidence limitations. Delta
statuses compare tree entries; changed statuses additionally classify changes
of mode and file type. Their schema definitions are `delta_status` and
`changed_status`; their file row shapes are `delta_file` and `changed_file`.

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --workspace --bins --locked --target x86_64-unknown-linux-musl
```

`fixtures/project` is a synthetic project with one of each case the
extractor handles, and `fixtures/refs.jsonl` is its expected output. After a
deliberate change to the output, regenerate it with
`UPDATE_GOLDEN=1 cargo test -p sf-impact-extract --test fixture` and review the diff.
The test reads the project as the one commit of a repository that it makes
in a temporary directory, with a fixed identity and date, so that the
commit the first row names is the same everywhere. The repositories of the
other tests are made the same way.

`fixtures/changed/base` is the project every pipeline scenario starts
from. Each test in `crates/sf-impact-changed/tests/scenarios.rs` commits it in a
temporary git repository, makes one change, runs the stages and compares
the output with `fixtures/changed/<scenario>.jsonl`.
`crates/sf-impact-changed/tests/mapping.rs` maps deltas written by hand, with no
repository, and `crates/sf-impact-delta/tests/delta.rs` runs `sf-impact-delta` on small
repositories of its own, with goldens in `fixtures/delta`. Regenerate
goldens with `UPDATE_GOLDEN=1 cargo test -p sf-impact-changed --test scenarios`
and `UPDATE_GOLDEN=1 cargo test -p sf-impact-delta --test delta`.

`scripts/reconcile.sh` compares the output for chosen symbols in pinned
public repositories with an independent grep, their metadata files with
counts made by `find`, and every element under the root of every modelled
file, found by `awk` from its indentation, with the rows on its lines. It
needs local clones and is not part of CI:

```sh
scripts/reconcile.sh target/release/sf-impact-extract /path/to/apex-recipes \
  /path/to/NebulaLogger /path/to/apex-rollup [/path/to/LightningFlowComponents]
```

For the Flow and LWC symbols every differing line needs a reviewed
explanation in `scripts/reconcile-flow-lwc.tsv`. The undeclared Aura-only
`playSound` control has no textual pattern; its Flow component references
are still emitted as unresolved.

`scripts/reconcile-changed.py` extracts each head from a `git archive` of
it, which is in no repository, so it passes `--accept-unverified-refs`: the
script is itself the check that the files go together. It holds `sf-impact-delta`
and `sf-impact-changed` to the commit ranges pinned in `scripts/reconcile-changed.tsv`, nine of
NebulaLogger and apex-rollup, by a reading of the two commits that shares no
code with the tools: files and what became of them, scope, and for every
hunk of every Apex file the declarations its lines are lines of, from an
outline of each version that the script makes by counting braces. The hunks
of the delta have to make the head version of the base version, byte for
byte, for every file whose lines are to be compared; which files those are
the script reads from the modes, blobs and bytes as git has them, so a
delta that says nothing of a changed file is held to it too. `git diff` is
asked as a second opinion: it may cut a change otherwise, pairing another
of two equal lines, and the symbols its hunks mark still have to be those
the tools mark. The script also checks what is added and removed, the type
of every changed custom metadata record, companions, bundles and gaps, runs
five small repositories whose expected symbols are written out, and times
the two stages apart. It holds four outputs that it makes wrong itself
against its checks, and fails if one is not reported. It exits 1 on any
difference that is not explained in
`scripts/reconcile-changed-accepted.tsv`, which is empty: there is none.
Local only, like the other:

```sh
scripts/reconcile-changed.py --extract target/release/sf-impact-extract \
  --delta target/release/sf-impact-delta --changed target/release/sf-impact-changed \
  --clone NebulaLogger=/path/to/NebulaLogger --clone apex-rollup=/path/to/apex-rollup
```
