# Reusing the previous pack, and running in CI

## Reusing the previous pack

A run on a developer's machine, or a CI job that keeps one file between
runs, can hand extract the pack it wrote last time:

```sh
sf-impact-extract --root . --previous refs.pack --out refs.pack
```

Extract then reads every file in scope, as always, but parses and resolves
only the files whose bytes changed since that pack was written, and takes
every other file's rows from it. The pack it writes is the one a run
without `--previous` writes, byte for byte. The pack carries what this
needs in its manifest (see [docs/pack.md](pack.md)): every file in
scope with the git blob id of the bytes that were read, and the types that
resolving references needs.

- `--previous` must name the file that `--out` names, or extract exits 2:
  the same file, by whatever path or link, where it exists. The file is
  read whole, by the name `--out` gives it and only if that is a regular
  file and not a link. The new pack is written to a temporary file beside
  it, synced to disk and renamed over it: there is only ever one pack,
  and no lock file.
- A previous pack that is missing, cut short, corrupt, of another version
  or release, of another scope or config, or of a repository that names
  its objects otherwise, is not reused: the run is a full run, and
  standard error says why. So is a pack written where no commit could be
  read, which has no manifest.
- Where a changed file declares other things than it did (a method added,
  a class renamed or deleted), every file's references may resolve
  otherwise. What the files that did not change declare is still taken
  from the pack, and only the changed files are parsed before resolving
  starts; then every file's rows are made again, each file parsed for its
  syntax as it is resolved, since the pack keeps no syntax tree. The check
  compares what each changed, new or deleted file declares (declarations,
  types and relationships) with what the pack has for it. It is
  conservative: it can only send a run down this longer path when it
  need not, never reuse rows that a full run would write otherwise.
- Reuse goes by each file's blob id, so a pack of another repository with
  the same scope, config and object format is reused for the files whose
  bytes are the same.
- A run that is killed while it writes leaves the previous pack as it
  was. It can leave its temporary file, `.refs.pack.<random>.tmp`, beside
  it, which nothing reads and which can be deleted.
- Two runs writing one pack at once are not supported.

## Running in CI

A hosted runner starts empty, so a CI job keeps the pack between runs in
a cache: restore the most recent pack before extract, hand it to extract
with `--previous`, and save the pack extract writes. In GitHub Actions,
add these steps after the checkout and install steps of
[Example: GitHub Actions](render.md#example-github-actions), in place of its
`Extract the checked-out head` step. The last step below replaces that
step: keep both and the second extract makes a cold run over the restored
pack.

```yaml
- id: refs-key
  run: echo "tool=$(sf-impact-extract --version | cut -d' ' -f2)" >> "$GITHUB_OUTPUT"
- uses: actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0
  with:
    path: ${{ runner.temp }}/refs.pack
    key: sf-impact-refs-${{ steps.refs-key.outputs.tool }}-${{ hashFiles('sf-impact.json') }}-${{ github.sha }}
    restore-keys: |
      sf-impact-refs-${{ steps.refs-key.outputs.tool }}-${{ hashFiles('sf-impact.json') }}-
- name: Extract, reusing the previous pack
  run: sf-impact-extract --root . --previous "$RUNNER_TEMP/refs.pack" --out "$RUNNER_TEMP/refs.pack"
```

- `--previous` and `--out` name the same path. Extract reads the pack
  there and writes the new one over it, so the file the cache restored is
  the file it saves.
- The key is exact for each commit. When no pack of this commit is
  cached, `restore-keys` restores the most recent one whose key starts
  with that prefix: the same release of the tool and the same config.
  At the end of a successful job the cache saves the new pack under the
  exact key, but only when that key was not already a hit: a cache entry
  is never overwritten. A pull request can restore a pack its base branch
  saved, so a job that also runs on the base branch gives each pull
  request's first run a pack to start from, when a cache entry saved on
  the base branch with the same tool and config exists and the pull
  request's run can read it.
- A cache miss costs time, not correctness. With no pack to restore,
  extract says on standard error that there is none and makes a cold run:
  every file is parsed and resolved, none reused. A pack that is restored
  but cannot be reused is treated the same (see
  [Reusing the previous pack](#reusing-the-previous-pack)). The pack
  written is byte-identical either way.
- Don't commit the pack to the repository. Here it is a cache, not a
  source file: it is remade from the tree on every run. See
  [What may persist](persistence.md#what-may-persist).
