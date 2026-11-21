# sf-impact-extract

Reads the package directories of a Salesforce DX project and writes its
refs: every declaration and every reference it finds. By default they are a
refs pack (schema 2, [docs/pack.md](pack.md)), the compact binary file
that `sf-impact-changed` and `sf-impact-tests` read. `--format jsonl` writes
the same rows as `refs.jsonl`, one JSON object per line, to read or keep;
`--dump <pack>` writes a pack's rows the same way.

```sh
sf-impact-extract --root path/to/project --out refs.pack
sf-impact-extract --root path/to/project --format jsonl | jq 'select(.resolution == "unresolved")'
sf-impact-extract --dump refs.pack | head -1
```

`--jobs <n>` sets the number of worker threads (by default one per CPU, at
most 8). `--trust-repo` is for a repository that another user owns: see
[Which tree it read](#which-tree-it-read). It writes nothing into the
project.

[schema/refs.schema.json](../schema/refs.schema.json) defines the rows and is the
contract for anything that reads the file. In short:

- `provenance`, the first row: what wrote the file, and of which tree,
  scope and config. See [Which tree it read](#which-tree-it-read).
- `declaration`: a class, interface, enum, trigger, method, constructor,
  field or enum value, with its file and line range. Overloads are separate
  declarations (`MathUtil.round(decimal)`, `MathUtil.round(decimal,integer)`).
  From metadata: an object, a field of one, a validation rule, a label, a
  custom permission, a permission set, a profile, a custom metadata record,
  a message channel, a Flow or an LWC bundle.
  A name declared more than once is marked `collision` and every copy is kept.
- `reference`, tier `parsed`: read from syntax and resolved. Method and
  constructor calls, types, casts, `extends` and `implements`, static member
  access, and the objects and fields named in SOQL, SOSL, DML and record
  initialisers. Each is a `declaration` in scope, `external` (a configured
  package namespace) or `unresolved`. Where a reference cannot be pinned to
  one declaration the row says why and lists the candidates: several
  overloads fit, or the name is declared twice and the declarations
  disagree. References read from metadata files are parsed rows too, of
  kinds of their own: see [Declarative metadata](#declarative-metadata).
  Flow references include record objects and fields, invocable actions,
  subflows and screen components. LWC references include Salesforce module
  imports, other components, template children, and metadata targets and
  objects.
- `reference`, tier `textual`: every whole-word, case-insensitive occurrence
  of a declared name in any file in scope, comments and strings included.
  This is what catches a class named in a string or a query built at run
  time.
- `parse_error`, `skipped_file`, `unmodelled_file`, `unsupported_element`
  and `scope_error`: an Apex file the grammar could not parse cleanly or a
  metadata file that is not well-formed XML (it is still read, as far as it
  goes), a file the textual tier could not search (binary content), a
  metadata file of a type that is not modelled, an element of a modelled
  metadata file that was not read, and a directory named in
  `sfdx-project.json` that could not be read.

Output is sorted and byte-identical from run to run.

## Which tree it read

The first row of `refs.jsonl`, and line 1 of a refs pack, is its `provenance`: the tool and its
version, the schema version, the tree that was read, where the project is
in its repository (`root`: `.` at the top, never a path of the machine),
the scope that `sfdx-project.json` declares and a hash of the config that
took effect. One repository can hold several projects that declare the
same directories; `root` tells the refs of one from those of another.
`sf-impact-changed` and `sf-impact-tests` hold the file to it: see
[What may persist](persistence.md#what-may-persist).

The tree is one of three things:

- **A commit**, with its id and the id of its tree, if all that was read is
  as the commit that is checked out has it: `sfdx-project.json`,
  `sf-impact.json` and every file under the scope directories, by content,
  and the commit has no file there that was not read.
- **`dirty`**, in a repository, where that is not so: a file in scope is
  changed, new or gone. Which file is said on standard error.
- **`unknown`**, where no commit could be read: no repository, no commit
  in it, or no git.

What counts is what is read, not what git calls clean:

- **A file outside the scope does not count.** A changed README, a
  `refs.jsonl` that an earlier run left at the root or a build directory
  leaves the tree its commit.
- **A file in scope counts though git ignores it.** It is read, and the
  commit does not have it, so the tree is `dirty`.
- **A file's mode is not content.**

Git is asked before a file is read, and not after. It is asked as
`sf-impact-delta` asks it: with plumbing that reads, without any `GIT_*` variable
of the caller's and without the user's and the system's configuration.
Which commit is checked out (`rev-parse`) and what it holds (`ls-tree`).
Nothing is written, the index is neither read nor refreshed, and nothing
is fetched. `--trust-repo` has a repository that another user owns read,
as for `sf-impact-delta`; without it git refuses and the tree is `unknown`.

What is read is held to the commit by the bytes that were read. Each file
is read once, and the id that git gives its content is worked out from
those bytes in the tool (SHA-1, or SHA-256 where the repository names its
objects so); so it is for `sfdx-project.json` and `sf-impact.json`, which
are read first and not again, also where a package directory is the root
and they are files in scope. Those ids and the set of files are compared
with the commit's. A file that
another process changes or restores while the tool runs is the commit's
only if what was read of it is: rows of other content never go out under
the name of the commit.

Limits:

- A tree that git converts on checkout is `dirty`: line ends
  (`core.autocrlf`, `eol` attributes) or a filter such as Git LFS make the
  bytes on disk other than the commit's, and it is the bytes on disk that
  are read.
- A submodule that is checked out in scope makes the tree `dirty`: its
  files are read, and they are not the commit's.
- The id of a tree is given only for a commit. Git gives the id of the
  tree of a working tree only by writing it into the repository, and
  nothing is written there.

## What it reads

The `packageDirectories` of `sfdx-project.json` and the `unpackagedMetadata`
paths they name; nothing else. Windows-style separators in those paths are
accepted. Symbolic links are not followed, neither in a declared path nor
under it. A file is text if and only if it is valid UTF-8, or UTF-16 (LE or BE) with a BOM that decodes without error, and the decoded content contains no NUL (U+0000). An optional UTF-8 BOM is preserved; a UTF-16 BOM is removed.

An optional `sf-impact.json` at the project root:

```json
{
  "textualMinLength": 6,
  "textualAllowList": ["Money"],
  "namespaces": ["npsp"]
}
```

Declared names shorter than `textualMinLength` are left out of the textual
tier unless `textualAllowList` names them. `namespaces` lists the namespaces
of installed packages; references into them are `external`.

## Declarative metadata

Metadata is read in Salesforce DX source format, where one file holds one
component and the file's path names it. So a component is declared even if
its file does not parse.

| File | Declares | References |
|---|---|---|
| `objects/X/X.object-meta.xml` | object `X` (custom, custom metadata type, custom setting, or a standard object the repository customises) | |
| `objects/X/fields/F.field-meta.xml` | field `X.F` | `lookup`: the object a lookup or master-detail points at. `lookup_filter`: the fields a lookup filter compares. `formula`: the names in a formula and in a default value. `summary`: the fields a roll-up summary sums, filters on and joins on. `controlling_field`: the field that controls a dependent picklist or a metadata relationship. `value_set`: a picklist's global value set |
| `objects/X/validationRules/R.validationRule-meta.xml` | validation rule `X.R` | `formula`: the names in its condition. `display_field`: the field its error is shown on |
| `*.permissionset-meta.xml`, `*.profile-meta.xml` | the permission set or profile | `grant`: classes, objects, fields, Flows, tabs (the platform's `standard-` tabs too), custom permissions, custom metadata types and custom settings |
| `customMetadata/T.R.md-meta.xml` | record `T.R` | `cmdt_type`: its type `T__mdt`. `cmdt_field`: each field of the type that it sets. `cmdt`: what its values and its own name name |
| `CustomLabels.labels-meta.xml` | each label, as `Label.Name` | |
| `*.customPermission-meta.xml` | the custom permission | |
| `*.messageChannel-meta.xml` | message channel `MessageChannel.Name__c` | |

Rows read from metadata say what kind of thing they name
(`referenced_type`), because one name can be several things: a class and a
permission set may both be called `Billing`.

- **Nothing in a modelled file is passed over in silence** unless it is
  known to hold no name. Every element is one of three things: read for
  the name in it; listed in `declarative/parts.rs` as holding none (a flag,
  a number, a label, free text, a literal value); or reported with an
  `unsupported_element` row that gives its path and line. Reporting is the
  default, so an element this tool has never heard of is a row. What is
  reported today: page, application, record type and layout entries and a
  permission set's `license` in permission sets and profiles; an object's
  search layouts, compact layout and the page an action override names; a
  lookup filter value that is no field this tool can place. An element
  that should name something and is empty is reported too
  (`reason: no_name`); a reference is never to an empty name. The same row
  reports a Flow structure that the Flow reader does not know.
- **What only the platform defines has no row.** `userPermissions` in a
  permission set or profile name permissions of the platform's
  (`ViewSetup`); a repository's own are `customPermissions`, which are
  read. `userLicense` names a user licence. Neither can be declared by a
  repository, so both are listed as holding no name. A permission set's
  `license` is not: it can be a permission set licence, which a repository
  can define.

- **Formulas** are tokenized, not parsed. A name is a field of the
  formula's object or a relationship path from it, or a global: `$Label`,
  `$Permission`, `$Setup`, `$CustomMetadata`, `$ObjectType`, and `$User`,
  `$Profile`, `$UserRole`, `$Organization` and `$RecordType`, which are
  records of objects. The constants of `$Api` and `$System` are the
  platform's and are left out. Any other global is a row as written,
  `unresolved`. Names in strings and comments are not references.
- **A relationship path** is one row for each field along it:
  `Carrier__r.Region__c` on `Shipment__c` is the lookup
  `Shipment__c.Carrier__c` and then `Carrier__c.Region__c`. From the first
  relationship that cannot be followed (no file declares the lookup, or it
  does not say what it points at) the rest of the path is one `unresolved`
  row, as written. A child relationship (`Parcels__r`) is the lookup field
  that makes it. The same holds for field paths in Apex queries.
- **Custom metadata records** bind classes in frameworks that load them
  by name. A record references each field of its type that it sets
  (`cmdt_field`). What a value names depends on the field. A value of a
  metadata relationship field names what the field's declared type says:
  an object, a field of the object that the controlling field's value
  names, or a record of another type; that is a `cmdt` row whether or not
  anything declares the name. Any other value, and the record's own name,
  gets a `cmdt` row if, as a whole and ignoring case, it equals a declared
  symbol: a class, a Flow, an LWC bundle, an object. Such a value can match
  by coincidence (a cache partition called like a class), so every `cmdt`
  row carries the field it came from (`source_field`). A field that is a
  relationship in one package directory and plain text in another has its
  value read both ways.
- **Profiles** in source format usually hold only part of a profile. Each
  file is a declaration of the profile, marked `collision` where there are
  several, and grants what it holds.
- **A lookup declared more than once** to different objects is followed to
  each of them. Where more than one of them declares the field that comes
  next, the row is `ambiguous_receiver` with every candidate, as for Apex.
  An object that does not declare it is not dropped: it keeps the rest of
  the path as an `unresolved` row of its own, since the repository may
  simply not hold that object's metadata.
- **Apex** that names an object, a field or a label resolves to these
  declarations: types, casts, `new Object__c(..)`, SOQL, SOSL, DML, record
  initialisers, triggers, field tokens and `Label.Name`. A member of a
  declared object (`Settings__c.getInstance()`, `Type__mdt.getAll()`,
  `Object__c.SObjectType`) is a reference to the object, and what follows
  it is the platform's.
- **Flow and LWC** object, field, label, custom permission, custom metadata
  type/record and message channel references resolve through the same catalog.
  This includes record-variable fields, Flow globals, schema imports and LWC
  metadata object targets. Relationship paths keep every possible end, with
  the same collision rules as Apex and formulas. Resolved permission and record
  names use their declaration symbols (`Can_Approve`, `Plugin.Billing`);
  undeclared names retain their original spelling. Flow reference elements
  with empty names produce `unsupported_element` rows with `reason: no_name`.
- **Every other `-meta.xml` file** is a component of a type that is not
  modelled and has an `unmodelled_file` row with its suffix (`layout`,
  `tab`). The companion files of Apex classes and triggers hold nothing to
  model and have none. Flows and the files of LWC bundles are modelled.

## Names declared more than once

A class name can be declared twice in a repository, in two package
directories for example. Each declaration has its own `declaration` row,
marked `collision`.

- If the declarations are the same in substance (same supertypes, same
  method and constructor signatures and return types, same field types),
  they resolve as one. This is the usual case: a copied file.
- If they differ, the type is *divergent*, and so is every type that
  inherits from it. An org cannot hold two classes of one name, so which
  declaration a reference means cannot be known, and nothing is chosen.
  Anything resolved through a divergent type (a member, an inherited
  member, a return or field type, a nested type, the elements of a
  collection) is every candidate reachable from every declaration, with no
  override hiding another. Where that is more than one, the row says
  `ambiguous_receiver` and lists them all. This over-reports on purpose.
