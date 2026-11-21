#!/usr/bin/env bash
# Corpus reconciliation: compares what sf-impact-extract reports for chosen symbols
# in pinned public repositories with an independent grep, and prints every
# line on which the two disagree so that each can be explained.
#
# Local only (the corpora are not vendored, so public CI does not run this).
#
#   scripts/reconcile.sh <sf-impact-extract> <apex-recipes clone> <NebulaLogger clone> \
#     <apex-rollup clone> [<LightningFlowComponents clone>]
#
# The fifth clone is optional: with it, the Flow and LWC symbols chosen in
# LightningFlowComponents are reconciled too.
#
# The clones must be at the pinned commits below. Nothing is written into
# them: output goes to a temporary directory. Needs bash, git, jq, grep, awk
# realpath and, for timing, GNU time at /usr/bin/time.
#
# For every symbol the report gives
#   grep      whole-word, case-insensitive occurrences of its name in the
#             scope directories (`grep -rIowi`), found without sf-impact-extract
#   textual   rows of the textual tier for the same declared name; equals grep
#   parsed    rows of the parsed tier that resolve to the symbol (for a class:
#             to it, its members and its nested types), from Apex and from
#             metadata alike, and how many of each kind
# and then each line where the textual rows differ from grep (none expected)
# and each line where the number of parsed rows differs from the number of
# grep occurrences, with a count of those lines by type of file. Those
# lines are what a reviewer explains.
#
# For declarative metadata it also checks the whole of each repository, by
# means that do not use sf-impact-extract:
#   files     metadata files of each modelled type, counted with find,
#             against the declarations of that type, Flows and the
#             directories of LWC bundles among them; every other -meta.xml
#             file against the unmodelled_file rows
#   elements  every element directly under the root of every modelled
#             metadata file, found with awk by its indentation and not by
#             its name, against the rows on its lines: a reference or an
#             unsupported_element row. The script holds no list of which
#             elements matter. For permission sets and profiles, where each
#             element with elements in it is one grant, every such element
#             must have a row, whatever it is called. The one exception is
#             named below, PLATFORM_ONLY: `userPermissions` name permissions
#             that the platform defines and no repository can, so the
#             extractor makes no row for them and none is asked for; they
#             are counted in a column of their own. A repository's own
#             permissions are `customPermissions`, which must have rows like
#             any other. For every type of file it prints the names of
#             the elements that have no row, for a reader to judge: those
#             are what the extractor says nothing about. The awk relies on
#             the four-space indentation that the Salesforce CLI writes,
#             which these repositories have.
#
# Exit status: 1 if the textual tier differs from grep for any symbol, a
# count of files differs from its rows, or an element of a permission set
# or profile that holds other elements has no row. Differences between the parsed
# tier and grep are expected and are reported, not judged: this script
# cannot tell a correct binding from a wrong one. The Flow and LWC symbols
# are held to more: any line on which they differ that has no pinned
# explanation in reconcile-flow-lwc.tsv fails the script.
# The undeclared Aura-only playSound control deliberately has no textual pattern.
# This is evidence about the chosen names, not proof that nothing is missing.
set -euo pipefail
export LC_ALL=C
# Even git status must not refresh an index in the read-only corpus clones.
export GIT_OPTIONAL_LOCKS=0

if [ "$#" -ne 4 ] && [ "$#" -ne 5 ]; then
  sed -n '2,13p' "$0" >&2
  exit 2
fi
extract=$1
recipes=$2
nebula=$3
rollup=$4
lightning=${5-}

RECIPES_COMMIT=eda6167c5126a34daf66d9e1371ff6957ae223bd
NEBULA_COMMIT=b792ac8d8b4eb31b14a164a24951c3cb4125a616
ROLLUP_COMMIT=5f16190f7ff4337db8b7acdd1769cbc23b88cc15
LIGHTNING_COMMIT=5de6df2c7ce5d23d50e45887c48920b1bfa4f511

# kind:symbol. A method symbol stands for all of its overloads. Any other
# kind than class and method is the `type` of a declaration read from
# metadata, and the symbol is matched exactly.
RECIPES_SYMBOLS="
class:MetadataTriggerHandler
class:CustomMetadataUtilties
method:StubExample.setGreeting
method:FilesRecipes.createFileAttachedToRecord
method:CanTheUser.flsAccessible
"
NEBULA_SYMBOLS="
class:FlowRecordLogEntry
class:LoggerPlugin
class:LoggerEmailSender
method:LoggerEmailSender.sendErrorEmail
method:LoggerStackTrace.ignoreOrigin
method:LoggerSObjectMetadata.getSchema
"
# Declarative metadata. SlackLoggerPlugin has no Apex caller outside its own
# file and its test: a custom metadata record binds it.
NEBULA_METADATA="
class:SlackLoggerPlugin
object_field:LogEntry__c.StackTrace__c
object_field:Log__c.LoggedBy__c
object:LoggerPlugin__mdt
object:LogEntryEvent__e
custom_permission:CanModifyLoggerSettings
permission_set:LoggerAdmin
"
# The Admin profile is declared in four package directories.
ROLLUP_METADATA="
profile:Admin
class:RollupCustomObjectLogger
object:Rollup__mdt
object_field:Rollup__mdt.RollupOperation__c
object_field:RollupOrderBy__mdt.Rollup__c
object:RollupLog__c
permission_set:See_Rollup_App
"

# Flows and LWC bundles, and classes that Flows call. Every line on which
# these differ needs a pinned explanation: see `explain_difference`.
NEBULA_FLOW_LWC="
class:FlowRecordLogEntry
lwc:loggerLWCEmbedDemo
lwc:loggerLWCGetLoggerImportDemo
lwc:loggerLWCCreateLoggerImportDemo
flow:Logger_Recipes_Flow_Screen_Demos
"
ROLLUP_FLOW_LWC="
class:RollupFlowBulkProcessor
class:RollupFlowBulkSaver
class:RollupFlowRecalculator
lwc:rollupJobPoller
lwc:rollupOrderBy
"
LIGHTNING_FLOW_LWC="
flow:Load_Related_Data_Case
flow:Summer_20_Issue_Sub_Flow
lwc:clipboardButton
lwc:playSound
lwc:dualFlowsSelector
lwc:formfactorFlowHelperLwc
"

tab=$(printf '\t')
# The one element of a permission set or profile that holds elements and is
# expected to have no row: it names a permission of the platform's.
PLATFORM_ONLY=userPermissions
textual_mismatches=0
structural_mismatches=0
work=$(mktemp -d)
explanations=$(dirname "$(realpath "$0")")/reconcile-flow-lwc.tsv
trap 'rm -rf "$work"' EXIT

# The scope directories, read from sfdx-project.json without sf-impact-extract.
scope_dirs() {
  jq -r '.packageDirectories[] | .path, (.unpackagedMetadata.path // empty)' \
    "$1/sfdx-project.json" | sed -e 's|\\|/|g' -e 's|^\./||' -e 's|/$||' | sort -u
}

# The files in the scope of repo $1 whose names end in $2, each once.
scope_files() {
  (
    cd "$1"
    scope_dirs . | while read -r dir; do
      [ -d "$dir" ] && find "$dir" -type f -name "*$2" || true
    done
  ) | sort -u
}

# The elements directly under the root of the files named on standard
# input, as "file TAB first line TAB last line TAB name TAB group|text",
# where a group is an element that holds other elements. Found by
# indentation alone: no element is known to this script by its name.
root_elements() {
  (cd "$1" && xargs -r -d '\n' awk -v OFS="$tab" '
    function close_element() { print FILENAME, first, FNR, name, (nested ? "group" : "text"); name = "" }
    FNR == 1 { name = "" }
    name == "" && /^    <[A-Za-z]/ {
      name = $0; sub(/^    </, "", name); sub(/[ >\/].*/, "", name)
      first = FNR; nested = 0
      if ($0 ~ ("</" name ">") || $0 ~ /\/>[ \r]*$/) close_element()
      next
    }
    name != "" && /^        <[A-Za-z]/ { nested = 1 }
    name != "" && $0 ~ ("</" name ">") { close_element() }')
}

# The lines of the rows that say something about what is inside a metadata
# file, as "file TAB line TAB kind": parsed references and
# unsupported_element rows.
element_rows() {
  jq -r 'select(.row == "unsupported_element" or (.row == "reference" and .tier == "parsed"))
    | [.file, .line, (.kind // .row)] | @tsv' "$1"
}

# Joins the elements in $1 with the rows in $2: each element with the
# number of rows on its lines, as "file TAB name TAB group|text TAB rows".
elements_with_rows() {
  awk -F "$tab" -v OFS="$tab" '
    NR == FNR { rows[$1] = rows[$1] " " $2; next }
    {
      n = split(rows[$1], lines, " "); count = 0
      for (i = 1; i <= n; i++) if (lines[i] + 0 >= $2 + 0 && lines[i] + 0 <= $3 + 0) count++
      print $1, $4, $5, count
    }' "$2" "$1"
}

# One row of a table that compares a count made by hand with a count of
# rows. With a fourth argument, the two counts are only shown, and the row
# is `same` if that argument says so.
compare() {
  local verdict=same
  if [ "${4-$([ "$2" = "$3" ] && echo same)}" != same ]; then
    verdict=DIFFERENT
    structural_mismatches=$((structural_mismatches + 1))
  fi
  echo "| $1 | $2 | $3 | $verdict |"
}

# Declarative metadata across the whole of repo $1, against the rows in $2.
metadata_check() {
  local repo=$1 refs=$2 pair suffix type
  echo "| metadata | by hand | rows | |"
  echo "|---|---:|---:|---|"
  for pair in object:object field:object_field validationRule:validation_rule \
    permissionset:permission_set profile:profile md:metadata_record \
    customPermission:custom_permission messageChannel:message_channel flow:flow; do
    suffix=${pair%%:*} type=${pair#*:}
    compare "\`*.$suffix-meta.xml\` files / \`$type\` declarations" \
      "$(scope_files "$repo" ".$suffix-meta.xml" | wc -l)" \
      "$(jq -c --arg type "$type" 'select(.row == "declaration" and .type == $type)' "$refs" | wc -l)"
  done
  compare "\`<labels>\` elements / \`label\` declarations" \
    "$(scope_files "$repo" ".labels-meta.xml" | (cd "$repo" && xargs -r -d '\n' grep -ho '<labels>') | wc -l)" \
    "$(jq -c 'select(.row == "declaration" and .type == "label")' "$refs" | wc -l)"
  # A bundle is a directory under `lwc` with a file in it.
  compare "directories under \`lwc/\` / \`lwc\` declarations" \
    "$(scope_files "$repo" "" | sed -n 's|^\(\(.*/\)\{0,1\}lwc/[^/]*\)/.*|\1|p' | sort -u | wc -l)" \
    "$(jq -c 'select(.row == "declaration" and .type == "lwc")' "$refs" | wc -l)"
  # The metadata file of a bundle belongs to the bundle; elsewhere a
  # `js-meta.xml` file is a component like any other.
  compare "other \`-meta.xml\` files / \`unmodelled_file\` rows" \
    "$(scope_files "$repo" "-meta.xml" |
      grep -Ev '\.(object|field|validationRule|permissionset|profile|md|customPermission|messageChannel|labels|flow|cls|trigger)-meta\.xml$' |
      grep -Ecv '(^|/)lwc/[^/]+/.*\.js-meta\.xml$')" \
    "$(jq -c 'select(.row == "unmodelled_file")' "$refs" | wc -l)"
  compare "\`*.md-meta.xml\` files / \`cmdt_type\` rows" \
    "$(scope_files "$repo" ".md-meta.xml" | wc -l)" \
    "$(jq -c 'select(.kind == "cmdt_type")' "$refs" | wc -l)"
  echo

  # Every element under the root of every modelled file, with its rows.
  local rows=$work/element-rows elements=$work/elements file
  element_rows "$refs" >"$rows"
  for suffix in object field validationRule permissionset profile md customPermission messageChannel labels; do
    scope_files "$repo" ".$suffix-meta.xml" | root_elements "$repo" >"$work/roots.$suffix"
    elements_with_rows "$work/roots.$suffix" "$rows" >"$elements.$suffix"
  done

  echo "| permission set or profile | \`userPermissions\` | other elements that hold elements |" \
    "of those, with a row | \`grant\` rows | \`unsupported_element\` rows | |"
  echo "|---|---:|---:|---:|---:|---:|---|"
  # Not a pipeline: `compare` counts mismatches in this shell.
  while read -r file; do
    local platform groups with_row
    platform=$(awk -F "$tab" -v file="$file" -v platform="$PLATFORM_ONLY" \
      '$1 == file && $2 == platform' "$elements.permissionset" "$elements.profile" | wc -l)
    groups=$(awk -F "$tab" -v file="$file" -v platform="$PLATFORM_ONLY" \
      '$1 == file && $3 == "group" && $2 != platform' \
      "$elements.permissionset" "$elements.profile" | wc -l)
    with_row=$(awk -F "$tab" -v file="$file" -v platform="$PLATFORM_ONLY" \
      '$1 == file && $3 == "group" && $2 != platform && $4 > 0' \
      "$elements.permissionset" "$elements.profile" | wc -l)
    # A file in which no element was found was not read by this script:
    # that is not agreement.
    compare "\`$file\` | $platform | $groups | $with_row" \
      "$(awk -F "$tab" -v file="$file" '$1 == file && $3 == "grant"' "$rows" | wc -l)" \
      "$(awk -F "$tab" -v file="$file" '$1 == file && $3 == "unsupported_element"' "$rows" | wc -l)" \
      "$([ "$groups" = "$with_row" ] && grep -qF "$file$tab" "$elements.permissionset" "$elements.profile" && echo same)"
  done < <(scope_files "$repo" ".permissionset-meta.xml"; scope_files "$repo" ".profile-meta.xml")
  echo

  echo "| type of file | \`unsupported_element\` rows, by element | elements under the root with no row, by name |"
  echo "|---|---|---|"
  for suffix in object field validationRule permissionset profile md customPermission messageChannel labels; do
    echo "| \`$suffix\` | $(jq -r --arg end ".$suffix-meta.xml" '
        select(.row == "unsupported_element" and (.file | endswith($end))) | "\(.element) (\(.reason))"' "$refs" |
        sort | uniq -c | sort -k1,1nr -k2 | awk '{printf "%s%s %s %s", sep, $2, $3, $1; sep = ", "} END {if (!NR) printf "none"}') |" \
      "$(awk -F "$tab" '$4 == 0 {print $2}' "$elements.$suffix" | sort | uniq -c | sort -k1,1nr -k2 |
        awk '{printf "%s%s %s", sep, $2, $1; sep = ", "} END {if (!NR) printf "none"}') |"
  done
  echo
}

# file:line of each whole-word occurrence of $2 under the scope of repo $1.
# -b gives each occurrence a distinct key, so nested scope directories
# cannot count one occurrence twice.
grep_occurrences() {
  (
    cd "$1"
    scope_dirs . | while read -r dir; do
      [ -d "$dir" ] && grep -rIonbwi -- "$2" "$dir" || true
    done
  ) | sort -u | cut -d: -f1,2 | sort
}

textual_rows() {
  jq -r --arg name "$2" '
    select(.row == "reference" and .tier == "textual"
           and (.referenced | ascii_downcase) == ($name | ascii_downcase))
    | "\(.file):\(.line)"' "$1" | sort
}

# Parsed rows that resolve to the symbol, as "file:line TAB kind". $2 is the
# kind of symbol, $3 the symbol.
parsed_rows() {
  jq -r --arg kind "$2" --arg symbol "$3" '
    def hits(prefixes): . as $s | any(prefixes[]; . as $p | $s | startswith($p));
    (if $kind == "class" then [$symbol + ".", $symbol + "("]
     elif $kind == "method" then [$symbol + "("]
     else [] end) as $prefixes
    | select(.row == "reference" and .tier == "parsed" and .resolution == "declaration")
    | select(if $kind == "flow" then .kind == "flow_subflow"
             elif $kind == "lwc" then (.kind == "flow_component" or .kind == "lwc_component")
             else true end)
    | select(($kind != "method" and .referenced == $symbol) or (.referenced | hits($prefixes))
             or any(.candidates[]?; . == $symbol or hits($prefixes)))
    | "\(.file):\(.line)\t\(.kind)"' "$1" | sort
}

# Lines on which two sorted file:line lists hold different counts, as
# "<count in $1> TAB <count in $2> TAB file:line". Paths may contain spaces.
differing_lines() {
  local counted='s/^ *\([0-9][0-9]*\) \(.*\)$/\2\t\1/'
  join -t "$tab" -a1 -a2 -e0 -o 0,1.2,2.2 \
    <(uniq -c "$1" | sed "$counted") <(uniq -c "$2" | sed "$counted") |
    awk -F "$tab" -v OFS="$tab" '$2 != $3 {print $2, $3, $1}'
}

# The differing lines of $1 counted by type of file, as "type grep/parsed".
by_file_type() {
  awk -F "$tab" '{
    file = $3; sub(/:[0-9]+$/, "", file); sub(/.*\//, "", file)
    type = file
    if (file ~ /-meta\.xml$/) { match(file, /[^.]+-meta\.xml$/); type = substr(file, RSTART) }
    else { sub(/.*\./, "", type) }
    grep[type] += $1; parsed[type] += $2
  }
  END { for (type in grep) printf "%s %d/%d\n", type, grep[type], parsed[type] }' "$1" |
    sort | awk '{printf "%s%s", sep, $0; sep = ", "}'
}

source_line() {
  local file=${2%:*} line=${2##*:}
  sed -n "${line}p" "$1/$file" | sed -e 's/^[[:space:]]*//' | cut -c1-150
}

# The Flow and LWC symbols have a reviewed explanation for every differing
# line at these pins. Any new difference fails instead of inheriting a
# category. `$pinned` is set by `reconcile` for those symbols.
explain_difference() {
  [ "$pinned" -eq 1 ] || return 0
  local reason
  reason=$(awk -F '\t' -v corpus="$1" -v symbol="$2" -v tier="$3" \
    -v g="$4" -v p="$5" -v location="$6" \
    '$1 == corpus && $2 == symbol && $3 == tier && $4 == g && $5 == p && $6 == location {print $7}' "$explanations")
  if [ -z "$reason" ]; then
    echo "  UNEXPLAINED difference: $1 $2 $3 $6"
    touch "$work/unexplained"
  else
    echo "  Explanation: $reason"
  fi
}

# $4 are the symbols whose differences are reported, $5 those whose every
# difference must also have a pinned explanation. A symbol in both is held
# to the stricter rule, once.
reconcile() {
  local name=$1 repo=$2 commit=$3 symbols=$4 pinned_symbols=$5
  local head
  head=$(git -C "$repo" rev-parse HEAD)
  if [ "$head" != "$commit" ]; then
    echo "$name: HEAD is $head, expected $commit" >&2
    exit 1
  fi
  if [ -n "$(git -C "$repo" status --porcelain)" ]; then
    echo "$name: the clone has local changes" >&2
    exit 1
  fi

  local refs=$work/$name.jsonl
  /usr/bin/time -v -o "$work/$name.time" "$extract" --root "$repo" --format jsonl --out "$refs"

  echo "## $name @ $commit"
  echo
  echo "- rows: $(wc -l <"$refs") ($(jq -r '.row + " " + (.tier // "")' "$refs" | sort | uniq -c |
    awk '{printf "%s%s %s", sep, $1, $2 ($3 ? "/" $3 : ""); sep = ", "}'))"
  echo "- wall time: $(awk -F': ' '/Elapsed/ {print $2}' "$work/$name.time")," \
    "peak RSS: $(awk -F': ' '/Maximum resident/ {print $2}' "$work/$name.time") kB"
  echo "- parse errors: $(jq -c 'select(.row == "parse_error")' "$refs" | wc -l)," \
    "scope errors: $(jq -c 'select(.row == "scope_error")' "$refs" | wc -l)"
  echo
  metadata_check "$repo" "$refs"
  echo "| symbol | declarations | grep | textual | parsed |"
  echo "|---|---:|---:|---:|---:|"

  local details=$work/$name.details
  : >"$details"
  # A symbol listed in both is left out of the first list: it is done once,
  # with the pinned ones.
  local pinned plain_symbols=""
  for entry in $symbols; do
    case " $(echo $pinned_symbols) " in
    *" $entry "*) ;;
    *) plain_symbols="$plain_symbols $entry" ;;
    esac
  done
  for entry in $plain_symbols $pinned_symbols; do
    pinned=0
    case " $(echo $pinned_symbols) " in *" $entry "*) pinned=1 ;; esac
    local kind=${entry%%:*} symbol=${entry#*:}
    local word=${symbol##*.}
    local id=$work/$name.$symbol
    grep_occurrences "$repo" "$word" >"$id.grep"
    textual_rows "$refs" "$word" >"$id.textual"
    parsed_rows "$refs" "$kind" "$symbol" >"$id.kinds"
    cut -f1 "$id.kinds" >"$id.parsed"
    local declared
    declared=$(jq -c --arg kind "$kind" --arg symbol "$symbol" '
      select(.row == "declaration")
      | select(if $kind == "class" then .symbol == $symbol
               elif $kind == "method" then (.symbol | startswith($symbol + "("))
               else .symbol == $symbol and .type == $kind end)' "$refs" | wc -l)
    echo "| \`$symbol\` | $declared | $(wc -l <"$id.grep") | $(wc -l <"$id.textual") | $(wc -l <"$id.parsed") |"

    {
      echo "### \`$symbol\` (grep for \`$word\`)"
      echo
      echo "textual vs grep:"
      if cmp -s "$id.grep" "$id.textual"; then
        echo "identical, line for line."
      else
        if [ "$declared" -eq 0 ] && [ ! -s "$id.textual" ]; then
          echo "No declaration: the textual tier indexes declared names only; all grep occurrences below are expected to be absent. The parsed column counts declaration resolutions only."
        else
          textual_mismatches=$((textual_mismatches + 1))
        fi
        differing_lines "$id.grep" "$id.textual" | while IFS=$tab read -r g t where; do
          echo "- grep $g, textual $t: \`$where\` \`$(source_line "$repo" "$where")\`"
          explain_difference "$name" "$symbol" textual "$g" "$t" "$where"
        done
      fi
      echo
      echo "parsed rows by kind: $(cut -f2 "$id.kinds" | sort | uniq -c |
        awk '{printf "%s%s %s", sep, $2, $1; sep = ", "} END {if (!NR) printf "none"}')"
      echo
      differing_lines "$id.grep" "$id.parsed" >"$id.differing"
      echo "parsed vs grep (lines where the counts differ), by type of file as grep/parsed:" \
        "$(by_file_type "$id.differing")"
      echo
      while IFS=$tab read -r g p where; do
        echo "- grep $g, parsed $p: \`$where\` \`$(source_line "$repo" "$where")\`"
        explain_difference "$name" "$symbol" parsed "$g" "$p" "$where"
      done <"$id.differing"
      echo
    } >>"$details"
  done
  echo
  cat "$details"
  if [ "$name" = LightningFlowComponents ]; then
    local unresolved
    unresolved=$(jq -c 'select(.tier == "parsed" and .kind == "flow_component" and .referenced == "playSound" and .resolution == "unresolved")' "$refs" | wc -l)
    echo "Aura-only playSound: $unresolved unresolved Flow component references (expected 4)."
    [ "$unresolved" -eq 4 ] || return 1
  fi
}

reconcile apex-recipes "$recipes" "$RECIPES_COMMIT" "$RECIPES_SYMBOLS" ""
reconcile NebulaLogger "$nebula" "$NEBULA_COMMIT" "$NEBULA_SYMBOLS $NEBULA_METADATA" "$NEBULA_FLOW_LWC"
reconcile apex-rollup "$rollup" "$ROLLUP_COMMIT" "$ROLLUP_METADATA" "$ROLLUP_FLOW_LWC"
if [ -n "$lightning" ]; then
  reconcile LightningFlowComponents "$lightning" "$LIGHTNING_COMMIT" "" "$LIGHTNING_FLOW_LWC"
fi

if [ "$textual_mismatches" -ne 0 ] || [ "$structural_mismatches" -ne 0 ]; then
  echo "textual tier differs from grep for $textual_mismatches symbol(s);" \
    "$structural_mismatches count(s) of metadata files or of elements with a row differ" >&2
  exit 1
fi

if [ -f "$work/unexplained" ]; then
  echo "unexplained corpus differences; review each line before updating reconcile-flow-lwc.tsv" >&2
  exit 1
fi
