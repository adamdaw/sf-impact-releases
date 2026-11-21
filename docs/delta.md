# sf-impact-delta

Compares two commits of the git repository that holds the project and writes
`delta.jsonl`: the two commits, every file that is not the same in both, the
lines of it that differ, and the text of both versions.

```sh
sf-impact-delta --base "$(git merge-base origin/main HEAD)" --head HEAD --out delta.jsonl
sf-impact-delta --merge-base origin/main --head HEAD | jq 'select(.row == "file") | .status'
```

`--base` is used as it is given. Give the merge base of the branch and its
target, not the tip of the target: from the tip, everything the target
gained since the branch left it reads as a change the branch undid.
`--merge-base <rev>` works it out. `--root` is the project directory, the
current one by default. `--trust-repo` is for a repository that another
user owns: see In CI below.

[schema/delta.schema.json](../schema/delta.schema.json) defines the rows:

- `provenance`, the first row: what wrote the file, the two commits, the
  tree of the head, where the project is in the repository (`root`), and
  the scope and the config hash of the head. Refs that go with the delta
  are of that head and of that project.
- `commit`, one for the base and one for the head: the commit's id, and the
  blobs that are `sfdx-project.json` and `sf-impact.json` in it.
- `file`, one for every file that differs, in the project or not: `added`,
  `deleted`, `modified` or `moved`, with its `file`, blob id, mode and whether it
  is binary at each commit, and its `hunks` where both versions are text.
- `blob`: the text of each version that is text, once for each blob.

This is the tool that reads the objects of a repository, and what it asks
of git is small. (`sf-impact-extract` asks git only which commit it read.)

- **Plumbing only.** Which commit a name is (`rev-parse`), where two commits
  part (`merge-base`), what a tree holds (`ls-tree`) and what a blob is
  (`cat-file`). These read objects. `git diff` is not run, so nothing that
  shapes its answer has a say: no configuration, no attributes, no diff
  driver. An older git is asked one thing more, as the limits say.
- **Lines are compared here**, with the `imara-diff` crate: Myers' algorithm
  with the indent heuristic, which is what git does by default. A line is
  everything up to and including its line feed, and nothing is ignored: a
  line that ends in CRLF is not the line that ends in LF.
- **No renames are detected.** A path only the base has is a file deleted,
  and a path only the head has is a file added, however alike the two are.
  The one exception needs no judgement: the very same blob under another
  path is the file `moved`. A file that moved and changed is one deleted
  and one added, and `sf-impact-changed` puts the two together by what they
  declare.
- **Binary** is a regular file that is not text. A file is text if and only if it is valid UTF-8, or UTF-16 (LE or BE) with a BOM that decodes without error, and the decoded content contains no NUL (U+0000). An optional UTF-8 BOM is preserved; a UTF-16 BOM is removed.
  Its lines are not compared. Text lines are compared after decoding.
- **A symbolic link and a submodule** are entries with a mode of their own
  (`120000`, `160000`). Neither is followed or read.
- **For two commits, the output is a function of their objects.** Git is
  run without any `GIT_*` variable of the caller's and without the user's
  and the system's configuration, and a commit is read as it is: no
  replacement object and no graft stands in for it or for its parents.
  Which commit a name is, and with `--merge-base` where two commits part,
  is read from the repository: its refs, and the history it has, of which
  a shallow clone has less. The `commit` rows say which two commits they
  were.
- **Nothing is fetched.** A clone that was not given all its objects (a
  partial clone) is promised the rest by a remote, and git fetches one when
  it is asked to read it. That writes to the repository, and runs whatever
  the repository says its remote is reached with. Git is told not to
  (`--no-lazy-fetch`), so an object that is not there is an error that
  names it, no remote is contacted and no such program is run.
- **Nothing is written** but the output: no temporary file, no temporary
  repository, no object, no configuration.

## In CI

- **History.** `--merge-base` needs the history of the branch and of its
  target back to where they part. A checkout of one commit does not have
  it: fetch with full depth.
- **A partial clone** (`--filter=blob:none`) has to have what is read
  before the tool is run: the trees of the two commits and both versions
  of every file that differs. `git diff <base> <head>` fetches the files
  that differ, and whatever is still not there the error names:

  ```sh
  base=$(git merge-base origin/main HEAD)
  git diff "$base" HEAD > /dev/null     # fetches; sf-impact-delta does not
  sf-impact-delta --base "$base" --head HEAD --out delta.jsonl
  ```

- **A workspace that another user owns.** In a container job the checkout
  is often made by one user and read by another, and git refuses to read a
  repository that the user who runs it does not own. The exception that a
  checkout step writes to the user's configuration (`safe.directory`) is
  not read here, since no configuration of the user is. `--trust-repo`
  has the repository read all the same:

  ```sh
  sf-impact-delta --trust-repo --merge-base origin/main --head HEAD --out delta.jsonl
  ```

  Git is given the exception on the command line of each call, for the
  top of the one working tree that the root is in, never for every
  repository, and nothing is written to any configuration. It is off
  unless given, and without it the refusal says that it exists. What is
  trusted is the repository's own configuration. The commands that are
  run take no program to run from it, and nothing is fetched with the flag
  or without. It needs a git that takes `safe.directory` from the
  command line; one that does not goes on refusing.

Limits:

- **Git older than 2.45 cannot be told not to fetch.** With such a git,
  every repository is first asked what it has, in a way that fetches
  nothing (`rev-list --missing=print`). It is read only if every tree and
  file of both commits is there, and with `--merge-base` every commit of
  both histories. A repository that has all its objects passes, at the
  cost of a walk of each commit's tree; a partial clone has to have been
  given all of that. Whether a repository is promised objects is not
  asked: the answer is git's to give, and it is not what is relied on.
  With 2.45 or later only what is read has to be there.
- **A path that is not UTF-8 is not read.** A commit that has a file with
  such a path, changed or not, is refused: written as text, two such paths
  could become one.
- Git is run with the repository's own configuration only.
- A repository that finds its objects only through the caller's
  environment (`GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`)
  is not read: the run fails on the first object it lacks. Objects borrowed
  through the repository's own `objects/info/alternates` are found.
- A file that is not text has no `blob` row. `sf-impact-changed` cannot read such a version and says so.
- The text of every changed file is in the output, both versions. A change
  to a large text file makes a large `delta.jsonl`.
