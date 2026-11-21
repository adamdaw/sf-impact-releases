# Limits

- `Flow.Interview.Welcome` in Apex is `unresolved`.
- There is no catalogue of standard objects and fields. An object or a
  field that no file in the repository declares is `unresolved`: `Account`,
  `Account.Name`, and the `Id` and `Name` of a custom object too.
- Not every part of a modelled metadata file is read for references. A
  part that is not, and is not known to hold no name, is an
  `unsupported_element` row: it says where the gap is, not what is in it.
- A custom metadata value of a field that is not a metadata relationship
  is matched as a whole. One name in a list, or in a longer text, is seen
  by the textual tier only.
- Metadata is recognised by its source-format file name. A file in
  Metadata API format (`Account.object`) is not recognised as metadata.
- A reference read from Apex does not say what kind of thing it names. Where
  an Apex class and an object share a name (`Account`), Apex means the class.
- Flow API names come from file stems, ignoring status/version metadata. Reference
  leaves and `{!...}` merge tokens are read in every Flow subtree, including
  future elements. Position-dependent names (such as `object` and `field`) need
  recognised structure. An unknown structure with an uninterpreted leaf is an
  `unsupported_element` row with its path, and so is an unknown leaf directly
  under a recognised element (`sourceTemplate`, `start/segment`) or an unknown
  child of `styleProperties`. The leaves known to name nothing are the ones
  Flow Builder writes in the pinned test corpora that the Flow metadata
  reference documents as flags, enumerations, literals, free text or names
  within the Flow, listed in `metadata/flow.rs`; any other leaf is reported.
  The name of a parameter, an assignment or a `dataTypeMappings` type
  variable is a member of its element's target (an Apex action's invocable
  variable, a component property, a subflow variable). Members are not
  modelled. The name is quiet when the target (`actionName`, `extensionName`,
  `flowName`, `apexClass`, a stage step's entry or exit action) is a reference
  row, whatever its resolution, or is omitted as the platform's
  (`flowruntime:` and `forceContent:` components, a `Region`'s `width`,
  `AlarmEvent` inputs). With no target or an empty one it is reported. A
  literal orchestration step assignee, which names a user, group or queue, is
  reported too. A `parse_error` is for XML that is not
  well-formed, and a Flow with both has both. Recovered references and
  textual matches remain.
  Record variables, operation outputs, loop/collection aliases and the start
  object supply field context, with case-insensitive variable lookup. Untyped
  dotted references remain `flow_field` rows with `unknown_type`; this includes
  component/action outputs and globals whose types are not modelled, so that
  kind alone does not prove an SObject field. `$Setup`, `$Permission` and
  `$CustomMetadata` name their setting objects/fields, custom permissions and
  metadata types/records/fields. `$User.Field` names a field on `User`.
  `$Flow.*`, `$Api.*`, `$Organization.*` and `$System.*` are explicitly omitted
  as platform-owned globals. Structured action names remain visible even when
  their action type is not modelled. Configured namespaces resolve as `external`;
  other namespaces remain `unresolved`. Only `flowruntime:` and `forceContent:`
  screen extensions are explicitly omitted as platform components.
- One LWC declaration is anchored to the main JS file, else the main HTML,
  else metadata, else the first bundle file. Every bundle file carries that
  referrer. JS and TS files share the import scanner. CSS quoted `@import 'c/…'`
  rules, custom-property editors, schema property types and Apex continuation
  imports also produce references. Namespaced template tags retain their namespace;
  only `lightning-` tags are explicitly omitted as platform components.
  The JS scanner understands imports/re-exports, comments, strings,
  regexes and template expressions; it is not a JavaScript syntax checker.
  Lexical errors, malformed imports and nonliteral dynamic imports produce
  `parse_error` rows, and so do TypeScript type-only imports and re-exports of
  a module (`import type { T } from 'c/x'`, `export type * from 'c/y'`), which
  are not modelled. Recovered references and textual matches are retained.
  Apex imports without arguments retain every possible overload. LWC targets
  remain explicit unresolved names; this slice does not declare platform targets.
  `@salesforce/resourceUrl`, `@salesforce/contentAssetUrl` and
  `@salesforce/userPermission` are not modelled as parsed dependencies in this
  slice; their source still feeds the textual tier. CSS support covers quoted
  module imports, not `url(...)` imports or escaped module names.
- An Apex reference is left out only when it is proven to be the platform's:
  a platform name (`String`, `System.debug`), or a member of a value whose
  static type is proven to be a platform type. A platform class without type
  arguments returns platform types; a collection returns its elements from
  the methods that are modelled. A platform type with type arguments, or a
  collection method that is not modelled, proves nothing, and what follows
  it is `unresolved` with `detail: unknown_type`. The list of platform names
  in `crates/sf-impact-extract/src/platform.rs` is curated; a platform name
  missing from it shows up as `unresolved`.
- Types are read off declarations, not inferred. A collection returns its
  elements and a declared method its declared type; what a method of an
  installed package or a field of an SObject is, is not known, and a call
  on such a value is `unresolved` with `detail: unknown_type`. `System.List`
  and the like are the same types as `List`.
- Access to an instance member (`invoice.amount`, as opposed to the call
  `invoice.total()` or the static `Invoice.RATE`) is not a parsed reference.
  That includes a field read or written through a record
  (`shipment.Weight__c`), which is how Apex mostly uses fields.
- How deeply a file may nest is bounded, so that one file cannot exhaust
  the stack and end the run. A file past a bound has a `parse_error` row
  and its text is still searched by the textual tier:
  - An Apex file whose syntax tree nests more than 4,000 levels (each
    pair of parentheses is a level, and so is each operand of a chain of
    `+`) is not read for declarations or parsed references at all. The
    row is at the line where the first node past 4,000 levels starts. The
    deepest class in the pinned test corpora nests 265 levels.
  - A metadata file, a Flow or an LWC metadata file whose elements nest
    more than 1,000 deep is read up to the first element past that, as
    XML that is not well-formed is read up to its error. The deepest file
    in the pinned test corpora nests 9.
  - An LWC JS or TS file is not scanned past a template literal nested in
    the `${}` of 1,000 others. The row is at the outermost of them, and
    what was read before it is kept.
