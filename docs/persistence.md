# What may persist

Nothing is required to. Each run can extract the tree it is given, and no
index is committed.

- **A refs pack whose provenance matches may be kept** between runs: as a
  CI cache (`actions/cache`) keyed by the head commit, for example, or as a
  local file. It is never committed to the repository.
- **The tools verify provenance and refuse mismatches.** The refs,
  `delta.jsonl` and `changed.jsonl` each start with a `provenance` row that
  says what wrote the file and of what. `sf-impact-changed` and `sf-impact-tests` read
  it before anything else and refuse refs that are not of the head they
  were given, in one line that names the field, both values and what to
  run. A file without the row is refused as an older format. A stale file
  is never used in silence.
- **Refs that cannot be verified need `--accept-unverified-refs`.** Those
  of a tree that is no commit's, `dirty` or `unknown`, are refused without
  it, and with it the output says that they were accepted.
- **The tools never write into the repository.** Where a kept file lives
  is the caller's choice, with `--out`. The file is written beside that
  path and renamed over it, and a path that names a symbolic link, a
  directory or a device is refused: a link is never written through. A
  path with a symbolic link in its directory part, below the current
  directory (`--out parent/refs.pack` where `parent` is a link), is refused
  too; a link above the current directory or on an absolute path that does
  not go through it (`/tmp` as `/private/tmp`, a home that is a link) is
  the machine's own, and is accepted. The temporary file has a random name
  and is made with `O_EXCL`. What is not covered: the output directory must
  not be writable by anyone who is not trusted with the output, as for most
  tools that write files, since `std` has no `openat` and a user who can
  change that directory can still swap a name between two calls.
  `sfdx-project.json`, `sf-impact.json` and a `--config` file are read
  only if each is a regular file, not a link, of at most 16 MiB.

The row holds no time, no name of a machine and no path of one, so the same
inputs still give the same bytes. What each field is, is in the schemas;
how `sf-impact-extract` knows which tree it read is under
[Which tree it read](extract.md#which-tree-it-read).

Refs name the commit they were extracted from, so a file kept in a cache
matches the commit it was keyed by.

A minimal GitHub Actions cache, keyed by the head commit, the tool's
version and a hash of the config. The key only finds the file; the check
is what makes it safe to use:

```yaml
- uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
  with:
    fetch-depth: 0
- id: refs-key
  run: |
    echo "head=$(git rev-parse HEAD)" >> "$GITHUB_OUTPUT"
    echo "tool=$(sf-impact-extract --version | cut -d' ' -f2)" >> "$GITHUB_OUTPUT"
- id: refs
  uses: actions/cache@0057852bfaa89a56745cba8c7296529d2fc39830 # v4
  with:
    path: ${{ runner.temp }}/refs.pack
    key: refs-${{ steps.refs-key.outputs.head }}-${{ steps.refs-key.outputs.tool }}-${{ hashFiles('sf-impact.json') }}
- if: steps.refs.outputs.cache-hit != 'true'
  run: sf-impact-extract --root . --out "$RUNNER_TEMP/refs.pack"
- run: |
    sf-impact-delta --merge-base origin/main --head HEAD |
      sf-impact-changed --delta - --refs "$RUNNER_TEMP/refs.pack" --out "$RUNNER_TEMP/changed.jsonl"
```
