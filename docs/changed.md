# sf-impact-changed

Maps `delta.jsonl` onto the symbols of the refs pack and writes
`changed.jsonl`: the files that differ, the symbols the difference adds,
modifies and removes, and the grants and bindings it touches.

```sh
sf-impact-extract --root . --out refs.pack          # with the head checked out
sf-impact-delta --merge-base origin/main --head HEAD |
  sf-impact-changed --delta - --refs refs.pack --out changed.jsonl
jq 'select(.row == "symbol" and .change == "removed")' changed.jsonl
```

It reads its two inputs and nothing else: no repository, no working tree,
no environment, and it runs no git. `--delta` is the `delta.jsonl` of the two
commits and `--refs` the refs pack of the tree of the head; one of them
can be `-`, standard input. The same two files give the same bytes.

Before it maps anything it reads the first row of each file and verifies
that the two go together:

- **Each first row is what its schema says.** Every field that the
  schema requires is there, though it be null; every field is valid and
  the fields agree with each other: ids are ids, a root is a path from the
  top of a repository, the row has no field the schema does not have. A
  row that is not is refused, and nothing of it is written on. What wrote
  the file has to be named and need not be one of these tools.
- **The refs are of the head of the delta**: extracted at the same commit,
  or at another commit with the same tree, as after the head is amended,
  rebased or squashed without a change.
- **The refs are of the same project of the repository**: the `root` is
  the same in both. Two projects of one commit do not stand for each other.
- **The scope and the config hash are the same** in both.
- **Each file is of a schema version the tool knows.**

Anything else is refused: nothing is written, the exit status is 1, and
one line names the field, what each file has for it and what to run:

```
sf-impact-changed: refs.jsonl is not of the head of delta.jsonl: `commit` is 2c1f0fb0878e8e96af6e99fe2901817e554361f5 in refs.jsonl and ea0934234cad5efff2dc2c0eaadeb4ce947da384 in delta.jsonl; re-run sf-impact-extract at ea0934234cad5efff2dc2c0eaadeb4ce947da384
```

Refs of a tree that is `dirty` or `unknown` cannot be verified and are
refused too. `--accept-unverified-refs` maps onto them as they are; the
`provenance` row of the output is then `partial`, with the gap
`unverified_refs`. Refs that are known to be of another head are refused
with the option as without it. A file that does not start with a
`provenance` row is of an older format and is refused.

[schema/changed.schema.json](../schema/changed.schema.json) defines the rows. In
short:

- `provenance`, the first row: what wrote the file, the two commits, the
  scope and the config hash, whether the refs were verified, and the first
  rows of the two files that were read.
- `file`: one row for every file that differs, in scope or not, so that
  nothing that changed is passed over. It says what became of the file
  (`added`, `deleted`, `modified`, `moved` with the path it had,
  `mode_changed`, `type_changed`), the package directory it is in, or `null`
  if it is out of scope, and a `confidence`. That is `partial`, with the
  reasons in `gaps`, where the symbol rows may not be all there is to say:
  the file is a component of a type that is not modelled
  (`unmodelled_file`), declares nothing the model knows (`no_declaration`:
  a static resource, an Aura file, a README in a package directory), did
  not parse cleanly at base or at head, or has no text to read
  (`parse_error`), or has a changed entry that was not read, or is a Flow
  whose head version holds an element that was not read
  (`unsupported_element`). On the row of `sfdx-project.json`,
  `scope_changed` says that the scope directories are not what they were:
  see Scope below.
- `symbol`: a declaration that is `added`, `modified` or `removed`, with the
  changed file and the lines that are the evidence, and a `reason`. `hunk`:
  changed lines are lines of the declaration, and `hunks` lists them.
  `whole_file`: the file as a whole marks it, as an added or deleted file
  does, a binary change, a file of an LWC bundle or the `-meta.xml`
  companion of a class. `removed`: the base version of a file that is still
  there declared it and the head version does not.
- `touched`: a name in an entry of a permission set, a profile or a custom
  metadata record that the difference changes, at `base`, at `head` or at
  both: a grant given, withdrawn or altered, a class bound or unbound.

How a difference is mapped:

- **Head from `refs.jsonl`, base from the delta.** The declarations at head
  and their lines are the `declaration` rows of `refs.jsonl`. For each
  changed file, and only for those, the base version is read from the text
  the delta gives, with the code that `sf-impact-extract` reads the tree with.
  What both declare, by package, kind and name, is the same symbol; the
  rest is `removed` or `added`. A method's name includes its parameter
  types, so a changed signature is one overload removed and another added.
- **A line marks the innermost declaration around it.** A line in a method
  marks the method; a line in no member marks the class, and in an inner
  class the inner class; a line above the class marks the class. A line
  that the head has marks what is around it at head, and a line that only
  the base had marks what it was in at base, if that is still declared: an
  annotation taken from above a method marks the method. Lines taken away
  also mark what is around the gap they leave, so a member removed whole
  marks its class. Nothing is ignored: a blank line between two methods is
  a line of the class, and a change of white space alone is a change.
- **A line that a declaration shares marks everything around it.** The
  line a member starts or ends on can hold more than the member: another
  member, the rest of a class header (`extends Base {`), the end of an
  initializer. A line cannot say which of them changed, so it marks every
  declaration around it, the class too. It marks the member alone only on
  proof: only declarations on the very same lines start or end on it, and
  nothing but white space is before them on their first line or after them
  on their last. The proof is read from the text of each version. Code as
  it is usually formatted passes it, so an edit to a one-line constant
  marks the constant and not its class.
- **A metadata file is its component.** A change to a field, a Flow, a
  permission set or a record marks it. In the labels file each label has
  its own lines. Any file of an LWC bundle marks the bundle, which is
  `added` if its directory came with the file and `removed` if no file of
  it is left.
- **The companion of an Apex file marks what the file declares.**
  `A.cls-meta.xml` holds the API version and the status of the class `A`,
  so `A` is marked when the companion is changed, added or deleted, with
  lines or without. Each version of a companion has the owner its name
  gives it: one that goes from `A.cls-meta.xml` to `B.cls-meta.xml` marks
  `A` and `B`. A companion and its Apex file are not taken to account for
  each other; a companion of an Apex file that is not there is a
  `no_declaration` gap.
- **Entries, not lines.** In a permission set, a profile or a custom
  metadata record, a name is touched if any line of its entry changed, not
  only its own: changing `<readable>` touches the field the entry names.
  Names at base are resolved against what the head declares.
- **A changed record marks its type.** Every change to the content of a
  custom metadata record has a `touched` row of kind `cmdt_type` for its
  type, at base and at head, whatever line changed: that is how Apex that
  queries the type is reached. What the record's own name names is touched
  in the same way.
- **A file that moved.** The delta says `moved` only of the very same
  content under another path. Within one package directory such a file
  changes no symbol and has only its `file` row; moved to another package
  directory, what it declares is removed from the one and added to the
  other; moved to a place where it is another kind of file, an Apex file
  into the directory of a bundle, it is read one version at a time.
- **A file that moved and changed** is one `deleted` and one `added` in the
  delta. Where the two are the same thing in the same package directory,
  because they declare the same symbol at the top, or are the companion of
  an Apex file of the same name, they are read as one file: their lines are
  compared here, and only what differs is marked. A method edited in a
  class that moved is `modified`, and its row says in `previous_file`
  where its base lines are. Where it is not one deleted file and one added
  file that are the same thing but more of either, none is chosen: what
  they declare is removed and added. So is a class that is given another
  name with its file, and a declaration that goes from one changed file to
  another by any other way; the removed row's `collision` says whether the
  name is still declared.
- **Scope** is that of each commit, from the `sfdx-project.json` the delta
  names for it: at head for a file's path at head, at base for its path at
  base. If the two commits do not declare the same directories, files that
  did not change are in scope where they were not, out of it, or in another
  package. They are not in the difference and have no rows, so the row of
  `sfdx-project.json` is `partial` with the gap `scope_changed`. A package
  directory that is itself a submodule or a symbolic link is in scope and
  is not read: `no_declaration`.
- **Collisions** are not resolved. A name declared in several files has a
  row for each declaration whose file changed, marked `collision`. On a
  `removed` row that means the name is still declared somewhere at head.

Limits:

- The refs have to be of the tree of the head commit, and `delta.jsonl`
  of the same head. That is verified from the first row of each, which is
  what the tool that wrote the file says of it: a file that was altered
  after it was written is not found out.
- Lines are compared with nothing ignored. A class that was only
  reformatted has every member marked, and so has one whose line ends
  changed.
- A binary change has no lines: every declaration in the file is marked.
  A file is text if and only if it is valid UTF-8, or UTF-16 (LE or BE) with a BOM that decodes without error, and the decoded content contains no NUL (U+0000). Text lines are compared after decoding.
- A version that the delta has no text for is not read. The file is `partial` (`parse_error`),
  nothing is known to be removed from it, and what the head declares in it
  is `modified`, not `added`.
- A name at base is read against the declarations of the head. A custom
  metadata value at base that named something the head no longer declares
  is a `touched` row only if its field is a metadata relationship.
- An in-scope file that is no metadata at all lowers the confidence, since
  nothing says that it cannot matter. There is no way yet to exclude it.
- A scope that changed is reported, not worked out: the files it brings in
  or leaves out are not listed.
