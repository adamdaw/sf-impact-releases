#!/usr/bin/env python3
"""Corpus reconciliation for sf-impact-delta and sf-impact-changed: holds delta.jsonl
and changed.jsonl for pinned ranges of public repositories against a
reading of the two commits that shares no code with the tools, and fails on
any difference that is not explained.

Local only (the corpora are not vendored, so public CI does not run this).

    scripts/reconcile-changed.py --extract <sf-impact-extract> --delta <sf-impact-delta> \\
        --changed <sf-impact-changed> \\
        --clone NebulaLogger=<clone> --clone apex-rollup=<clone> [<range id> ...]

The ranges are in reconcile-changed.tsv beside this script: an id, the
corpus, and the two commits by their full ids. The clones need the history
of those commits. `git clone --filter=blob:none` is enough: sf-impact-delta
fetches nothing, so the files of each range are fetched here before it is
run, by the `git archive` of the head and a `git diff` of the two commits.
Nothing else is written into a clone: the head tree is unpacked into a
temporary directory for sf-impact-extract. That directory is in no repository, so
the refs say that their tree is `unknown`, and sf-impact-changed is told to accept
them as they are (`--accept-unverified-refs`): what it would verify, that
the refs are of the head, is what this script holds the outputs to by its
own means. Needs python3, git and tar.

The tools run no `git diff`. This script does, as a second opinion: with
renames off, since the tools detect none, and with every file as text.

What is checked, for every range, by means that do not use the tools:

  files      every file `git diff --raw --no-renames` lists has one `file`
             row in each output with what became of it. The one pairing
             the tools make is made again here: the same blob at a path
             that only the base has and at a path that only the head has
             is the file moved.
  hunks      for every file whose lines are to be compared: the base
             version, with the lines of each hunk of the delta replaced by
             its lines at head, is the head version, byte for byte, from
             the two blobs as git has them. So no line that differs is
             outside a hunk. Which files those are is read here, from the
             modes, the blob ids and the bytes: a file with content at
             both commits, changed in place, neither version binary. A
             delta that has no hunks for one is held to that too, and one
             that has hunks for any other file is a difference. `git diff`
             is a comparison of its own and may cut the same change
             otherwise, pairing another of two equal lines; how often it
             does is printed, and is no difference.
  by git     for every such file that is Apex: the symbols that the hunks
             of `git diff` mark, by the outline and the rule below, are
             the symbols the tools mark by lines, whether or not the
             delta has hunks for the file. Where git cuts a change
             otherwise, it still has to come to the same symbols.
  scope      the package of each file, from sfdx-project.json at each
             commit read with the json module, and the scope_changed gap
             exactly where the two commits declare other directories
  evidence   a file row says `binary` exactly where a version of the file
             has a NUL byte among its first 8000 bytes, read from the blob
             here; and the hunks a symbol row lists are hunks of its file,
             each once
  marked     for every hunk of every Apex file changed in place, or deleted
             at one path and added at another as the same class: the
             declarations its lines are lines of, at head and at base, from
             an outline of each version made here by counting braces, are
             exactly the symbols whose rows carry the hunk. Every member a
             hunk crosses has to be there, and nothing else. A symbol
             of such a file that is modified has lines that say so.
  controls   four outputs made wrong here, each in a way both schemas
             allow: a delta without the hunks of a changed file with the
             method that did not change marked instead, a hunk too long, a
             changed method not marked, a text said to be binary. Each has
             to be reported by the checks that are for it, and what the
             tools wrote by none. A check is only as good as its last
             control; the script fails if one is not reported.
  literals   five small repositories made here, in which a changed line is
             shared by a member and something else, each with the symbols
             it has to mark written out: `marked` applies the rule of the
             tool with another parser, and these do not apply it at all
  added,     for every Apex file: what the outline of the base version
  removed    declares and that of the head version does not is `removed`,
             and the reverse `added`, each exactly
  records    every custom metadata record whose content changed has a
             `touched` row of kind cmdt_type for the type its name gives,
             at each version it has
  companions every changed -meta.xml companion of an Apex file marks the
             symbol its name gives, or says no_declaration; and one whose
             text changed lists every hunk of it
  bundles    every changed file under lwc/<name>/ marks <name>
  components every other changed metadata file of a modelled type marks
             the symbol its path gives; of any other type, has the gap
             unmodelled_file
  silence    no file in scope is without a symbol row, a gap, or a reason
             to have neither: only its mode changed, or it only moved

That a file only moved is established from git, never from what the tool
did not say: the same blob at another path that gives it the same name in
the same package. That a deleted file went on in an added file is
established here too: they are the same thing in the same package, by the
class the outline finds at the top of each, by the Apex file a companion
is beside, or by the symbol a metadata file's path gives it, and they are
the only two that are.

A difference is printed with the check, the file and what differs. One that
has been read and is accepted is a line of reconcile-changed-accepted.tsv:
range, check, file, item and the explanation. The script exits 1 if any
difference is not in that file, or if a line of that file no longer matches
anything, and 0 otherwise.

The outline is deliberately simple. It knows types, methods, constructors,
fields, properties and enum values by the shape of their first line and the
braces after it, and nothing of what is inside a method.
"""
import argparse
import contextlib
import copy
import json
import os
import re
import statistics
import subprocess
import sys
import tempfile
import time
from collections import Counter, defaultdict
from pathlib import Path

HERE = Path(__file__).resolve().parent

# The outline of an Apex file ------------------------------------------------

TOKEN = re.compile(r"//[^\n]*|/\*.*?\*/|'(?:\\.|[^'\\\n])*'|@\w+|\w+|\S", re.S)
TYPE_WORDS = {"class", "interface", "enum"}
NOT_A_NAME = {"get", "set"}


def tokens(text):
    """Words and punctuation, each with its line and where it starts and
    ends in the text; no comments, strings as `''`."""
    found = []
    for match in TOKEN.finditer(text):
        word = match.group()
        if word.startswith("//") or word.startswith("/*"):
            continue
        line = text.count("\n", 0, match.start()) + 1
        found.append(("''" if word.startswith("'") else word, line, match.start(), match.end()))
    return found


def matching(toks, at, opening, closing):
    """The index of the token that closes the one at `at`."""
    depth = 0
    for index in range(at, len(toks)):
        if toks[index][0] == opening:
            depth += 1
        elif toks[index][0] == closing:
            depth -= 1
            if depth == 0:
                return index
    return len(toks) - 1


def without_annotations(header):
    """A header's tokens without `@Name` and `@Name(...)`."""
    kept, at = [], 0
    while at < len(header):
        if header[at][0].startswith("@"):
            at += 1
            if at < len(header) and header[at][0] == "(":
                at = matching(header, at, "(", ")") + 1
            continue
        kept.append(header[at])
        at += 1
    return kept


def split_commas(toks):
    """Runs of tokens between the commas that are in no bracket."""
    runs, run, depth = [], [], 0
    for tok in toks:
        if tok[0] in "(<[{":
            depth += 1
        elif tok[0] in ")>]}":
            depth -= 1
        if tok[0] == "," and depth == 0:
            runs.append(run)
            run = []
        else:
            run.append(tok)
    runs.append(run)
    return runs


def parameter_types(toks):
    """`final Map<Id, String> names, Integer n` -> `Map<Id,String>,Integer`."""
    types = []
    for run in split_commas(toks):
        words = [tok[0] for tok in without_annotations(run) if tok[0] != "final"]
        if words:
            types.append("".join(words[:-1]))
    return ",".join(types)


def outline(text):
    """What an Apex file declares: (symbol, kind, first line, last line,
    alone), where `alone` says that nothing but white space is before it
    on its first line and after it on its last, comments included."""
    toks = tokens(text)
    found = []

    def declare(symbol, kind, first, last):
        """From the token at `first` to the token at `last`."""
        before = text[text.rfind("\n", 0, toks[first][2]) + 1 : toks[first][2]]
        after = text[toks[last][3] :].split("\n", 1)[0]
        alone = not before.strip() and not after.strip()
        found.append((symbol, kind, toks[first][1], toks[last][1], alone))

    def body(at, owner, kind_of_owner):
        """The members from `at` to the brace that closes their type."""
        while at < len(toks):
            word, line = toks[at][0], toks[at][1]
            if word == "}":
                return at
            if word == ";":
                at += 1
                continue
            if kind_of_owner == "enum":
                # Constants, with commas between them.
                if word != ",":
                    declare(f"{owner}.{word}", "enum_value", at, at)
                at += 1
                continue
            # The header: up to the first `{`, `;` or `=` outside brackets.
            stop, depth = at, 0
            while stop < len(toks):
                here = toks[stop][0]
                if here == "(":
                    depth += 1
                elif here == ")":
                    depth -= 1
                elif depth == 0 and here in "{;=":
                    break
                stop += 1
            header = without_annotations(toks[at:stop])
            words = [tok[0] for tok in header]
            lowered = [w.lower() for w in words]
            ends_with = toks[stop][0] if stop < len(toks) else ";"
            kind = next((w for w in lowered if w in TYPE_WORDS), None)
            is_type = kind is not None and "(" not in words[: lowered.index(kind)]
            if lowered[:1] == ["trigger"]:
                close = matching(toks, stop, "{", "}")
                declare(words[1], "trigger", at, close)
                at = close + 1
            elif is_type and ends_with == "{":
                name = words[lowered.index(kind) + 1]
                symbol = f"{owner}.{name}" if owner else name
                close = body(stop + 1, symbol, kind)
                declare(symbol, kind, at, close)
                at = close + 1
            elif "(" in words:
                opening = words.index("(")
                closing = matching(header, opening, "(", ")")
                name = words[opening - 1]
                types = parameter_types(header[opening + 1 : closing])
                close = matching(toks, stop, "{", "}") if ends_with == "{" else stop
                declare(f"{owner}.{name}({types})", "callable", at, close)
                at = close + 1
            elif ends_with == "{":
                # A property, or an initializer block, which declares nothing.
                close = matching(toks, stop, "{", "}")
                names = [w for w in words if w.lower() != "static"]
                if len(names) >= 2:
                    declare(f"{owner}.{words[-1]}", "field", at, close)
                at = close + 1
            else:
                # Fields: `Type a, b = 1, c;` to the `;` outside brackets.
                end, depth = stop, 0
                while end < len(toks):
                    here = toks[end][0]
                    if here in "({[":
                        depth += 1
                    elif here in ")}]":
                        depth -= 1
                    elif here == ";" and depth == 0:
                        break
                    end += 1
                last = min(end, len(toks) - 1)
                for run in split_commas(without_annotations(toks[at:end])):
                    before = []
                    for tok in run:
                        if tok[0] == "=":
                            break
                        before.append(tok[0])
                    if before and re.fullmatch(r"\w+", before[-1]) and before[-1] not in NOT_A_NAME:
                        declare(f"{owner}.{before[-1]}", "field", at, last)
                at = end + 1
        return len(toks) - 1

    body(0, "", "file")
    return found


# The contract, written again ---------------------------------------------------


def outermost(decls):
    def inside(d):
        return any(o[2] <= d[2] and d[3] <= o[3] and o[3] - o[2] > d[3] - d[2] for o in decls)

    return [d for d in decls if not inside(d)]


def marked_by_line(decls, line):
    """The declarations a changed line is a line of: every one around a
    line that a declaration starts or ends on, unless that line is proven
    to be of one declaration alone; the innermost otherwise."""
    around = [d for d in decls if d[2] <= line <= d[3]]
    if not around:
        return outermost(decls)
    on_edge = [d for d in around if line in (d[2], d[3])]
    of_one_alone = len({(d[2], d[3]) for d in on_edge}) <= 1 and all(d[4] for d in on_edge)
    if not of_one_alone:
        return around
    smallest = min(d[3] - d[2] for d in around)
    return [d for d in around if d[3] - d[2] == smallest]


def marked_by_gap(decls, after):
    """The declarations around the gap between `after` and the next line."""
    around = [d for d in decls if d[2] <= after and after + 1 <= d[3]]
    if not around:
        return outermost(decls)
    smallest = min(d[3] - d[2] for d in around)
    return [d for d in around if d[3] - d[2] == smallest]


def fold(symbol):
    return symbol.lower()


# One range ---------------------------------------------------------------------


NO_BLOB = "0" * 40


class Range:
    def __init__(self, ident, clone, base, head, accepted):
        self.ident, self.clone, self.base, self.head = ident, clone, base, head
        self.accepted = accepted  # (check, file, item) -> explanation
        self.used = set()
        self.differences = []
        self.counts = Counter()

    def git(self, *args, check=True):
        run = subprocess.run(["git", "-C", self.clone, "-c", "core.quotePath=false", *args], capture_output=True)
        if check and run.returncode:
            raise SystemExit(f"git {' '.join(args)} failed: {run.stderr.decode()}")
        return run.stdout

    def show(self, commit, path):
        return self.git("show", f"{commit}:{path}").decode(errors="replace")

    def differ(self, check, file, item):
        self.counts[check + " differences"] += 1
        key = (check, file, item)
        if key in self.accepted:
            self.used.add(key)
        else:
            self.differences.append(key)

    def expect(self, check, file, expected, actual, what):
        """Two sets that have to be equal; each element that is in one only."""
        self.counts[check] += 1
        for item in sorted(expected - actual):
            self.differ(check, file, f"missing {what}: {item}")
        for item in sorted(actual - expected):
            self.differ(check, file, f"unexpected {what}: {item}")

    # What git says.

    def files(self):
        """What became of each file, by its path at head, or at base for a
        deleted one: (status, path at base of a file that moved). Also, in
        `self.blobs`, the two blobs of each, and in `self.same_blob` the
        paths whose content is the same at both."""
        raw = self.git("diff", "--raw", "-z", "--no-abbrev", "--no-renames", self.base, self.head)
        fields = raw.decode().split("\0")
        found, gone, come, at = {}, {}, {}, 0
        self.same_blob, self.blobs, self.modes = set(), {}, {}
        while at < len(fields) and fields[at]:
            old_mode, new_mode, old, new, status = fields[at].lstrip(":").split(" ")
            path, at = fields[at + 1], at + 2
            if status == "A":
                come[path] = (new, new_mode[:3])
            elif status == "D":
                gone[path] = (old, old_mode[:3])
            else:
                found[path] = ("type_changed" if status == "T" else "modified", None)
                self.blobs[path] = (old, new)
                self.modes[path] = (old_mode, new_mode)
                if old == new:
                    self.same_blob.add(path)
        # The same blob as the same kind of thing under another path is the
        # file moved: those of one name first, the rest in order.
        name = lambda path: path.rsplit("/", 1)[-1]
        moved, left = {}, sorted(come)
        for by_name in (True, False):
            for old_path in sorted(gone):
                fits = [new for new in left if come[new] == gone[old_path] and (not by_name or name(new) == name(old_path))]
                if old_path not in moved and fits:
                    moved[old_path] = fits[0]
                    left.remove(fits[0])
        for old_path, (blob, _) in gone.items():
            if old_path in moved:
                found[moved[old_path]] = ("moved", old_path)
                self.blobs[moved[old_path]] = (blob, blob)
                self.same_blob.add(moved[old_path])
            else:
                found[old_path] = ("deleted", None)
                self.blobs[old_path] = (blob, NO_BLOB)
        for new_path in left:
            found[new_path] = ("added", None)
            self.blobs[new_path] = (NO_BLOB, come[new_path][0])
        return found

    def binary(self, ids):
        """The ids of the blobs that have a NUL among their first 8000 bytes."""
        asked = sorted(set(ids))
        run = subprocess.run(
            ["git", "-C", self.clone, "cat-file", "--batch"], input="".join(f"{i}\n" for i in asked).encode(), capture_output=True
        )
        out, at, found = run.stdout, 0, set()
        for ident in asked:
            end = out.index(b"\n", at)
            header = out[at:end].decode().split(" ")
            at = end + 1
            if header[-1] == "missing":
                continue
            size = int(header[2])
            if header[1] == "blob" and b"\0" in out[at : at + min(size, 8000)]:
                found.add(ident)
            at += size + 1
        return found

    def scope(self, commit):
        run = subprocess.run(
            ["git", "-C", self.clone, "show", f"{commit}:sfdx-project.json"], capture_output=True
        )
        if run.returncode:
            return set()
        dirs = set()
        for entry in json.loads(run.stdout).get("packageDirectories", []):
            dirs.add(entry["path"])
            if "unpackagedMetadata" in entry:
                dirs.add(entry["unpackagedMetadata"]["path"])
        return {"/".join(p for p in d.replace("\\", "/").split("/") if p not in ("", ".")) for d in dirs}

    def git_hunks(self, *what):
        """What `git diff` says differs, every file as text: per path at
        head, the path at base and [(base start, lines, head start,
        lines)]. `what` is two commits, or two blobs."""
        patch = self.git("diff", "-U0", "--no-renames", "--text", "--no-ext-diff", "--no-textconv", "--no-color", *what)
        found, old, new = {}, None, None
        for line in patch.decode(errors="replace").split("\n"):
            if line.startswith("diff --git "):
                old = new = None
            elif line.startswith("--- a/"):
                old = line[6:].rstrip("\t")
            elif line.startswith("+++ b/"):
                new = line[6:].rstrip("\t")
                if old:
                    found[new] = (old, [])
            elif line.startswith("@@ ") and old and new:
                b, bl, h, hl = re.match(r"@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@", line).groups()
                found[new][1].append((int(b), int(bl or 1), int(h), int(hl or 1)))
        return found

    def applies(self, old, new, hunks):
        """Whether the blob `old`, with the lines of each hunk replaced by
        its lines in the blob `new`, is `new`."""
        def lines(blob):
            parts = self.git("cat-file", "blob", blob).split(b"\n")
            return [part + b"\n" for part in parts[:-1]] + ([parts[-1]] if parts[-1] else [])

        (base, head), at_base, at_head = (lines(old), lines(new)), 0, 0
        for base_start, base_lines, head_start, head_lines in sorted(hunks):
            to_base = base_start - 1 if base_lines else base_start
            to_head = head_start - 1 if head_lines else head_start
            if base[at_base:to_base] != head[at_head:to_head]:
                return False
            at_base, at_head = to_base + base_lines, to_head + head_lines
        return base[at_base:] == head[at_head:]

    def key(self, path, commit, package):
        """What a file is in its package, by what it declares at the top or
        by its path; None for a file that is nothing by either."""
        name = path.rsplit("/", 1)[-1]
        companion = re.fullmatch(r"(.+)\.(cls|trigger)-meta\.xml", name)
        if companion:
            return (package, "companion", fold(companion.group(1)))
        if name.lower().endswith((".cls", ".trigger")):
            top = sorted(fold(d[0]) for d in outermost(outline(self.show(commit, path))))
            return (package, "apex", tuple(top)) if top else None
        if name.endswith("-meta.xml") and not re.search(r"(?:^|/)lwc/[^/]+/.+", path):
            symbol, suffix = component_symbol(path)
            return (package, suffix, fold(symbol)) if symbol else None
        return None

    def joined(self, files, in_scope):
        """The added file that each deleted file went on in: the two are the
        same thing in the same package, and no other deleted or added file
        is. By the path of the added file."""
        halves = defaultdict(lambda: ([], []))
        for path, (status, _) in files.items():
            at_base, at_head = in_scope[path]
            if status == "deleted" and at_base is not None:
                halves[self.key(path, self.base, at_base)][0].append(path)
            if status == "added" and at_head is not None:
                halves[self.key(path, self.head, at_head)][1].append(path)
        pairs = [(deleted[0], added[0]) for key, (deleted, added) in halves.items() if key and len(deleted) == len(added) == 1]
        return {added: deleted for deleted, added in pairs}

    def reconcile(self, rows, delta=None):
        """Holds the rows of changed.jsonl, and those of delta.jsonl if they
        are given, against the two commits."""
        file_rows = {r["file"]: r for r in rows if r["row"] == "file"}
        symbols = [r for r in rows if r["row"] == "symbol"]
        touched = [r for r in rows if r["row"] == "touched"]
        files = self.files()
        base_dirs, head_dirs = self.scope(self.base), self.scope(self.head)

        def package(dirs, path):
            holding = [d for d in dirs if path == d or path.startswith(d + "/") or d == ""]
            return max(holding, key=len) if holding else None

        # files
        expected = {f"{path} {status} {previous}" for path, (status, previous) in files.items()}
        actual = set()
        for path, row in file_rows.items():
            status = "modified" if row["status"] == "mode_changed" else row["status"]
            actual.add(f"{path} {status} {row.get('previous_file')}")
        self.expect("files", "changed.jsonl", expected, actual, "file")
        # The files whose lines are to be compared, from the modes, the
        # blobs and the bytes as git has them, whatever the tools say: a
        # file changed in place, a file with content at both commits, with
        # another blob, and with no NUL among the first 8000 bytes of
        # either. Each has hunks to answer for, said or not.
        binary = self.binary(i for pair in self.blobs.values() for i in pair)
        is_file = lambda path: all(mode.startswith("100") for mode in self.modes[path])
        texts = {
            path
            for path, (status, _) in files.items()
            if status == "modified" and is_file(path) and path not in self.same_blob and not set(self.blobs[path]) & binary
        }
        by_git = self.git_hunks(self.base, self.head)
        hunks = {path: by_git.get(path, (path, [])) for path in texts}
        if delta is not None:
            of_delta = [r for r in delta if r["row"] == "file"]
            shown = lambda r: (r["head"] or r["base"])["file"]
            actual = {f"{shown(r)} {r['status']} {r['base']['file'] if r['status'] == 'moved' else None}" for r in of_delta}
            self.expect("files", "delta.jsonl", {e.replace(" type_changed ", " modified ") for e in expected}, actual, "file")
            # hunks: those of the delta make the head of the base.
            tuples = lambda r: [(h["base_start"], h["base_lines"], h["head_start"], h["head_lines"]) for h in r.get("hunks", [])]
            of_tool = {shown(r): tuples(r) for r in of_delta}
            for path in sorted(set(of_tool) - texts):
                if of_tool[path]:
                    self.differ("hunks", path, "the delta has hunks for a file that has no two texts to compare")
            for path in sorted(texts):
                of_file = of_tool.get(path, [])
                self.counts["hunks"] += len(of_file)
                if not self.applies(*self.blobs[path], of_file):
                    self.differ("hunks", path, "the hunks of the delta do not make the head version of the base version")
                of_git = hunks[path][1]
                self.counts["hunks as git has them"] += len(set(of_file) & set(of_git))
                self.counts["hunks git cuts otherwise"] += len(set(of_git) - set(of_file))
                self.counts["files git cuts otherwise"] += set(of_git) != set(of_file)
            hunks = {path: (path, of_tool.get(path, [])) for path in texts}

        # scope
        in_scope = {}
        for path, (status, previous) in files.items():
            at_head = None if status == "deleted" else package(head_dirs, path)
            at_base = package(base_dirs, previous or path) if status != "added" else None
            in_scope[path] = (at_base, at_head)
            want = at_head if at_head is not None else at_base
            got = file_rows.get(path, {}).get("package")
            self.expect("scope", path, {str(want)}, {str(got)}, "package")
        moved = base_dirs != head_dirs
        gap = "scope_changed" in file_rows.get("sfdx-project.json", {}).get("gaps", [])
        self.expect("scope", "sfdx-project.json", {str(moved)}, {str(gap)}, "scope_changed")

        by_file = defaultdict(list)
        for row in symbols:
            by_file[row["file"]].append(row)

        # A deleted file that went on in an added file is one file from
        # here on: at the path of the added one, with the deleted one as
        # where it was. Its hunks are those its rows list; git compares
        # the two blobs for a second opinion of them.
        units = dict(files)
        for added, deleted in self.joined(files, in_scope).items():
            units[added] = ("joined", deleted)
            del units[deleted]
            in_scope[added] = (in_scope[deleted][0], in_scope[added][1])
            self.blobs[added] = (self.blobs[deleted][0], self.blobs[added][1])
            listed = {(h["base_start"], h["base_lines"], h["head_start"], h["head_lines"]) for r in by_file[added] for h in r.get("hunks", [])}
            hunks[added] = (deleted, sorted(listed))
            for row in by_file[added]:
                if row["change"] == "modified":
                    self.expect("files", added, {deleted}, {str(row.get("previous_file"))}, f"previous file of {row['symbol']}")

        # evidence
        for path, (status, _) in files.items():
            old, new = self.blobs[path]
            expected = old != new and status != "type_changed" and bool({old, new} & binary)
            if status != "type_changed":
                self.expect("evidence", path, {str(expected)}, {str(bool(file_rows.get(path, {}).get("binary")))}, "binary")
        for path, (status, previous) in units.items():
            of_file = set(hunks.get(path, (None, []))[1])
            for row in by_file[path]:
                listed = [(h["base_start"], h["base_lines"], h["head_start"], h["head_lines"]) for h in row.get("hunks", [])]
                if len(listed) != len(set(listed)) or not set(listed) <= of_file:
                    self.differ("evidence", path, f"hunks of {row['symbol']} are not hunks of the file, each once")
            old, new = self.blobs[path]
            if status == "joined" and not {old, new} & binary:
                self.counts["hunks"] += len(of_file)
                if not self.applies(old, new, of_file):
                    self.differ("hunks", path, "the hunks of its rows do not make it of the file it was")
                by_git[path] = (previous, [hunk for _, of in self.git_hunks(old, new).values() for hunk in of])
                texts.add(path)

        for path, (status, previous) in sorted(units.items()):
            at_base, at_head = in_scope[path]
            if at_base is None and at_head is None:
                continue
            row = file_rows.get(path, {})
            gaps = set(row.get("gaps", []))
            # From git alone: the same content under another path, in the
            # same package. Whether the path still gives it the same name
            # is for each check to say.
            moved_as_it_is = status == "moved" and at_base == at_head
            mine = by_file[path] + (by_file[previous] if previous else [])
            name = path.rsplit("/", 1)[-1]
            is_apex = name.lower().endswith((".cls", ".trigger"))
            was_apex = (previous or path).lower().endswith((".cls", ".trigger"))
            bundle = re.search(r"(?:^|/)lwc/([^/]+)/.+", path)
            companion = re.fullmatch(r"(.+)\.(cls|trigger)-meta\.xml", name)

            if bundle and at_head is not None:
                self.expect("bundles", path, {bundle.group(1)}, {r["symbol"] for r in by_file[path]}, "bundle")
            elif companion:
                # Each version marks the owner its name gives it; one that
                # only moved, with the same owner, marks nothing.
                owners = {fold(companion.group(1))}
                if previous:
                    was = fold(re.sub(r"\.(cls|trigger)-meta\.xml$", "", previous.rsplit("/", 1)[-1]))
                    owners = set() if moved_as_it_is and was in owners else owners | {was}
                if "no_declaration" not in gaps:
                    self.expect("companions", path, owners, {fold(r["symbol"]) for r in mine}, "owner")
                # A companion whose text changed, and whose owner is the
                # same, has lines to show for it: every hunk of the file,
                # on each row of it.
                same_owner = status == "modified" or (previous and len(owners) == 1)
                old, new = self.blobs[path]
                if same_owner and old != new and not {old, new} & binary:
                    of_file = sorted(hunks.get(path, (None, []))[1])
                    for row in by_file[path]:
                        listed = sorted(
                            (h["base_start"], h["base_lines"], h["head_start"], h["head_lines"]) for h in row.get("hunks", [])
                        )
                        self.expect(
                            "companions", path, {str(of_file)}, {str(listed)}, f"hunks of {row['symbol']}"
                        )
            elif is_apex or was_apex:
                compared = (hunks[path], by_git.get(path, (path, []))) if path in texts else (None, None)
                self.apex(path, previous, status, at_base, at_head, mine, gaps, *compared)
            elif name.endswith(".md-meta.xml"):
                self.record(path, previous, status, at_base, at_head, touched, moved_as_it_is)
                self.component(path, previous, mine, gaps, moved_as_it_is)
            elif name.endswith("-meta.xml") and not bundle:
                self.component(path, previous, mine, gaps, moved_as_it_is)

            # silence
            only_mode = status == "modified" and path in self.same_blob
            if not (mine or gaps or moved_as_it_is or only_mode):
                self.differ("silence", path, "no symbol row and no gap")
            self.counts["silence"] += 1

    def apex(self, path, previous, status, at_base, at_head, rows, gaps, hunks, by_git):
        """An Apex file. `hunks` and `by_git` are the lines that differ, as
        the tools and as git have them, for a file whose two versions are
        texts to compare by the reading made here; None for any other."""
        base_path = previous or path
        was = outline(self.show(self.base, base_path)) if status != "added" and at_base is not None else []
        now = outline(self.show(self.head, path)) if status != "deleted" and at_head is not None else []
        if not base_path.lower().endswith((".cls", ".trigger")):
            was = []
        if not path.lower().endswith((".cls", ".trigger")):
            now = []
        if "parse_error" in gaps:
            return  # The tools say the file did not parse; an outline of it proves nothing.
        moved_package = at_base != at_head
        before = {fold(d[0]) for d in was}
        after = {fold(d[0]) for d in now}
        if moved_package:
            gone, come = before, after
        else:
            gone, come = before - after, after - before
        own = [r for r in rows if r["type"] != "lwc" and not r["file"].endswith("-meta.xml")]
        self.expect("removed", path, gone, {fold(r["symbol"]) for r in own if r["change"] == "removed"}, "symbol")
        self.expect("added", path, come, {fold(r["symbol"]) for r in own if r["change"] == "added"}, "symbol")
        if hunks is None or moved_package:
            return
        carried = defaultdict(set)
        for row in own:
            for hunk in row.get("hunks", []):
                key = (hunk["base_start"], hunk["base_lines"], hunk["head_start"], hunk["head_lines"])
                carried[key].add(fold(row["symbol"]))
            # Two texts were compared: what is modified is so by its lines.
            if row["change"] == "modified" and not row.get("hunks"):
                self.differ("marked", path, f"modified, and no line says so: {row['symbol']}")
        def marks(hunk):
            base_start, base_lines, head_start, head_lines = hunk
            expected = set()
            if head_lines:
                for line in range(head_start, head_start + head_lines):
                    expected |= {fold(d[0]) for d in marked_by_line(now, line)}
            else:
                expected |= {fold(d[0]) for d in marked_by_gap(now, head_start)}
            for line in range(base_start, base_start + base_lines):
                expected |= {fold(d[0]) for d in marked_by_line(was, line)} & after
            return expected

        for hunk in hunks[1]:
            where = f"{path} @@ -{hunk[0]},{hunk[1]} +{hunk[2]},{hunk[3]} @@"
            self.expect("marked", where, marks(hunk), carried.get(hunk, set()), "symbol")
        # A second opinion of the lines: what git's hunks come to.
        of_git = set().union(*[marks(hunk) for hunk in by_git[1]])
        of_tool = set().union(*carried.values())
        self.expect("by git", path, of_git, of_tool, "symbol")

    def record(self, path, previous, status, at_base, at_head, touched, moved_as_it_is):
        def type_of(file):
            return file.rsplit("/", 1)[-1].split(".")[0] + "__mdt"

        def name(file):
            return file.rsplit("/", 1)[-1]

        expected = set()
        if status != "added" and at_base is not None:
            expected.add(f"base {type_of(previous or path)}")
        if status != "deleted" and at_head is not None:
            expected.add(f"head {type_of(path)}")
        # The same record, as it was, in another directory: nothing of it
        # changed. Any other rename is a change to a record.
        if moved_as_it_is and name(previous) == name(path):
            expected = set()
        seeds = {
            f"{r['side']} {r['referenced']}"
            for r in touched
            if r["kind"] == "cmdt_type" and r["file"] in (path, previous)
        }
        self.expect("records", path, {fold(e) for e in expected}, {fold(s) for s in seeds}, "type seed")

    def component(self, path, previous, rows, gaps, moved_as_it_is):
        symbol, suffix = component_symbol(path)
        if suffix == "labels":
            return
        if symbol is None:
            self.expect("components", path, {"unmodelled_file"}, gaps & {"unmodelled_file"}, "gap")
            return
        expected = {fold(symbol)}
        if previous and previous.endswith("-meta.xml"):
            was = component_symbol(previous)[0]
            if moved_as_it_is and was == symbol:
                expected = set()  # The same component, as it was, elsewhere.
            elif was:
                expected.add(fold(was))
        self.expect("components", path, expected, {fold(r["symbol"]) for r in rows}, "symbol")


def component_symbol(path):
    """The symbol a metadata file's path gives it, and its suffix; no
    symbol for a file of a type that is not modelled."""
    name = path.rsplit("/", 1)[-1]
    stem, suffix = name[: -len("-meta.xml")].rsplit(".", 1)
    parent = path.split("/")[-3] if path.count("/") >= 2 else ""
    symbol = {
        "object": stem,
        "field": f"{parent}.{stem}",
        "validationRule": f"{parent}.{stem}",
        "permissionset": stem,
        "profile": stem,
        "md": stem,
        "flow": stem,
        "customPermission": stem,
        "messageChannel": f"MessageChannel.{stem}__c",
    }.get(suffix)
    return symbol, suffix


# Lines that a member shares with something else, and what they have to mark:
# written out, not worked out. (name, base text, head text, symbols)
LITERALS = [
    (
        "two methods meet on a line; the second one's return type changes",
        "public class Alpha {\n Integer first() { return 1; } Integer second() {\n   return 2;\n }\n}\n",
        "public class Alpha {\n Integer first() { return 1; } Decimal second() {\n   return 2;\n }\n}\n",
        {"Alpha", "Alpha.first()", "Alpha.second()"},
    ),
    (
        "a method on the line of the class header; the sharing of the class changes",
        "public with sharing class Alpha { Integer small() { return 1; }\n Integer second() {\n  return 2;\n }\n}\n",
        "public without sharing class Alpha { Integer small() { return 1; }\n Integer second() {\n  return 2;\n }\n}\n",
        {"Alpha", "Alpha.small()"},
    ),
    (
        "the class header goes on to the line of a method; what the class extends changes",
        "public class Alpha\n extends Beta { Integer small() { return 1; }\n Integer second() { return 2; }\n}\n",
        "public class Alpha\n extends Other { Integer small() { return 1; }\n Integer second() { return 2; }\n}\n",
        {"Alpha", "Alpha.small()"},
    ),
    (
        "an initializer ends on the line of a method; the initializer changes",
        "public class Alpha {\n static {\n System.debug(1); } Integer small() { return 1; }\n Integer second() { return 2; }\n}\n",
        "public class Alpha {\n static {\n System.debug(2); } Integer small() { return 1; }\n Integer second() { return 2; }\n}\n",
        {"Alpha", "Alpha.small()"},
    ),
    (
        "a constant on a line of its own; its value changes",
        "public class Alpha {\n static final Integer LIMIT = 1;\n Integer second() { return 2; }\n}\n",
        "public class Alpha {\n static final Integer LIMIT = 2;\n Integer second() { return 2; }\n}\n",
        {"Alpha.LIMIT"},
    ),
]


@contextlib.contextmanager
def made(before, after, extract, delta, changed):
    """A repository made for the occasion, in which the class Alpha is
    `before` at one commit and `after` at the next, and what the tools say
    of the two: the repository, the rows of delta.jsonl and the rows of
    changed.jsonl. The repository is gone when the block is left."""
    identity = {"GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"}
    for who in ("AUTHOR", "COMMITTER"):
        identity[f"GIT_{who}_NAME"] = "reconcile-changed"
        identity[f"GIT_{who}_EMAIL"] = "reconcile-changed@example.invalid"
    with tempfile.TemporaryDirectory() as scratch:
        repo = Path(scratch) / "repo"
        (repo / "app/classes").mkdir(parents=True)

        def git(*args):
            subprocess.run(["git", "-C", str(repo), *args], check=True, capture_output=True, env={**os.environ, **identity})

        git("init", "--quiet", "--initial-branch=main")
        (repo / "sfdx-project.json").write_text('{"packageDirectories": [{"path": "app"}]}\n')
        for parent in ("Beta", "Other"):
            (repo / f"app/classes/{parent}.cls").write_text(f"public virtual class {parent} {{\n}}\n")
        for text, message in ((before, "base"), (after, "head")):
            (repo / "app/classes/Alpha.cls").write_text(text)
            git("add", "--all")
            git("commit", "--quiet", "--message", message)
        refs = Path(scratch) / "refs.jsonl"
        subprocess.run([extract, "--root", str(repo), "--out", str(refs)], check=True, capture_output=True)
        command = [delta, "--root", str(repo), "--base", "HEAD~1", "--head", "HEAD"]
        between = subprocess.run(command, check=True, capture_output=True)
        command = [changed, "--delta", "-", "--refs", str(refs)]
        run = subprocess.run(command, check=True, capture_output=True, input=between.stdout)
        rows = lambda written: [json.loads(line) for line in written.splitlines()]
        yield str(repo), rows(between.stdout), rows(run.stdout)


def literals(extract, delta, changed):
    """Runs the tools on each of LITERALS in a repository made for it. The
    differences, each as (name, what differs)."""
    differences = []
    for name, before, after, expected in LITERALS:
        with made(before, after, extract, delta, changed) as (_, _, rows):
            marked = {row["symbol"] for row in rows if row["row"] == "symbol"}
        for symbol in sorted(expected - marked):
            differences.append((name, f"missing symbol: {symbol}"))
        for symbol in sorted(marked - expected):
            differences.append((name, f"unexpected symbol: {symbol}"))
    return differences


# Controls: outputs that are wrong, which the checks have to report --------
#
# A check that reports nothing has to be known to report something. Each of
# these takes what the tools wrote of one small change, the body of the
# first of two methods, and makes it wrong in a way that both schemas still
# allow: (name, what it does to the rows of delta.jsonl and of changed.jsonl,
# the checks that have to report it).


def the_symbol(rows):
    return next(row for row in rows if row["row"] == "symbol")


def the_file(rows):
    return next(row for row in rows if row["row"] == "file")


def another_method_and_no_lines(between, rows):
    # The delta says no line differs, and changed.jsonl marks the method
    # that did not change, as a whole.
    the_file(between).pop("hunks")
    symbol = the_symbol(rows)
    symbol.pop("hunks")
    symbol.update(symbol="Alpha.second()", start=6, end=8, reason="whole_file")


def a_hunk_too_long(between, rows):
    the_file(between)["hunks"][0]["head_lines"] += 1


def no_symbol(between, rows):
    rows.remove(the_symbol(rows))


def binary_that_is_not(between, rows):
    the_file(rows)["binary"] = True


CONTROL = (
    "public class Alpha {\n Integer first() {\n  return 1;\n }\n\n Integer second() {\n  return 2;\n }\n}\n",
    "public class Alpha {\n Integer first() {\n  return 10;\n }\n\n Integer second() {\n  return 2;\n }\n}\n",
)
WRONG = [
    ("no hunks in the delta, and the method that did not change marked whole", another_method_and_no_lines, {"hunks", "marked", "by git"}),
    ("a hunk of the delta one line longer than it is", a_hunk_too_long, {"hunks"}),
    ("the changed method not marked", no_symbol, {"marked", "by git", "silence"}),
    ("a file of text said to be binary", binary_that_is_not, {"evidence"}),
]


def controls(extract, delta, changed):
    """Holds each of WRONG against the checks. What is not as it has to
    be, each as (name, what): a wrong output that a check it is for did not
    report, or the right output reported."""
    failures = []
    with made(*CONTROL, extract, delta, changed) as (repo, between, rows):

        def reported(between, rows):
            one = Range("control", repo, "HEAD~1", "HEAD", {})
            one.reconcile(rows, between)
            return {check for check, _, _ in one.differences}

        if reported(between, rows):
            failures.append(("what the tools wrote", f"reported by {sorted(reported(between, rows))}"))
        for name, make_wrong, checks in WRONG:
            wrong = copy.deepcopy((between, rows))
            make_wrong(*wrong)
            silent = checks - reported(*wrong)
            if silent:
                failures.append((name, f"not reported by {', '.join(sorted(silent))}"))
    return failures


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--extract", required=True, help="the sf-impact-extract binary")
    parser.add_argument("--delta", required=True, help="the sf-impact-delta binary")
    parser.add_argument("--changed", required=True, help="the sf-impact-changed binary")
    parser.add_argument("--clone", action="append", default=[], metavar="CORPUS=PATH")
    parser.add_argument("--keep", metavar="DIR", help="keep each range's refs.jsonl, delta.jsonl and changed.jsonl here")
    parser.add_argument("ranges", nargs="*", help="range ids; all of them by default")
    args = parser.parse_args()
    clones = dict(clone.split("=", 1) for clone in args.clone)

    def table(name):
        lines = (HERE / name).read_text().splitlines()[1:]
        return [line.split("\t") for line in lines if line.strip()]

    accepted = defaultdict(dict)
    for ident, check, file, item, explanation in table("reconcile-changed-accepted.tsv"):
        accepted[ident][(check, file, item)] = explanation

    failed = False
    if not args.ranges:
        found = literals(args.extract, args.delta, args.changed)
        print(f"literals: {len(LITERALS)} shared lines with their symbols written out, {len(found)} differences")
        for name, difference in found:
            print(f"    DIFFERENT literals  {name}  {difference}")
            failed = True
        found = controls(args.extract, args.delta, args.changed)
        print(f"controls: {len(WRONG)} wrong outputs held against the checks, {len(found)} not as they have to be")
        for name, difference in found:
            print(f"    DIFFERENT controls  {name}  {difference}")
            failed = True
    for ident, corpus, base, head in table("reconcile-changed.tsv"):
        if args.ranges and ident not in args.ranges:
            continue
        if corpus not in clones:
            raise SystemExit(f"{ident}: no --clone {corpus}=<path>")
        clone = clones[corpus]
        with tempfile.TemporaryDirectory() as scratch:
            tree = Path(scratch) / "tree"
            tree.mkdir()
            archive = subprocess.Popen(["git", "-C", clone, "archive", head], stdout=subprocess.PIPE)
            subprocess.run(["tar", "-x", "-C", str(tree)], stdin=archive.stdout, check=True)
            if archive.wait():
                raise SystemExit(f"{ident}: git archive {head} failed")
            refs = Path(scratch) / "refs.jsonl"
            subprocess.run([args.extract, "--root", str(tree), "--out", str(refs)], check=True, capture_output=True)
            # In a clone without all its blobs, git fetches here the files
            # that differ. sf-impact-delta would not, and says what it lacks.
            subprocess.run(["git", "-C", clone, "diff", "--no-renames", base, head], check=True, capture_output=True)
            # Each stage five times, for the time it takes.
            def timed(command, given=None):
                seconds = []
                for _ in range(5):
                    started = time.monotonic()
                    run = subprocess.run(command, capture_output=True, check=True, input=given)
                    seconds.append(time.monotonic() - started)
                return run.stdout, statistics.median(seconds)

            between, delta_seconds = timed([args.delta, "--root", clone, "--base", base, "--head", head])
            # The refs are of an archive of the head, which names no commit.
            written, changed_seconds = timed([args.changed, "--delta", "-", "--refs", str(refs), "--accept-unverified-refs"], between)
            if args.keep:
                keep = Path(args.keep)
                keep.mkdir(parents=True, exist_ok=True)
                (keep / f"{ident}.delta.jsonl").write_bytes(between)
                (keep / f"{ident}.changed.jsonl").write_bytes(written)
                (keep / f"{ident}.refs.jsonl").write_bytes(refs.read_bytes())
        rows = [json.loads(line) for line in written.splitlines()]
        one = Range(ident, clone, base, head, accepted[ident])
        one.reconcile(rows, [json.loads(line) for line in between.splitlines()])

        kinds = Counter(r["row"] for r in rows)
        # Of the files: the first row is partial here too, for the refs.
        partial = sum(1 for r in rows if r["row"] == "file" and r["confidence"] == "partial")
        print(
            f"{ident} {corpus} {base[:10]}..{head[:10]}: {kinds['file']} files, {kinds['symbol']} symbol rows, "
            f"{kinds['touched']} touched rows, {partial} partial; "
            f"sf-impact-delta {delta_seconds:.3f} s, sf-impact-changed {changed_seconds:.3f} s (medians of 5)"
        )
        if one.counts["hunks git cuts otherwise"]:
            print(
                f"    git has {one.counts['hunks as git has them']} of the {one.counts['hunks']} hunks as the delta has them, "
                f"and cuts {one.counts['files git cuts otherwise']} files otherwise"
            )
        checks = ["files", "hunks", "by git", "scope", "evidence", "marked", "added", "removed", "records", "companions", "bundles", "components", "silence"]
        print("    checked: " + ", ".join(f"{one.counts[c]} {c}" for c in checks if one.counts[c]))
        for key in sorted(one.used):
            print(f"    accepted  {key[0]}  {key[1]}  {key[2]}\n              {one.accepted[key]}")
        for key in sorted(set(one.accepted) - one.used):
            print(f"    STALE     {key[0]}  {key[1]}  {key[2]}: accepted, and no longer a difference")
            failed = True
        for check, file, item in one.differences:
            print(f"    DIFFERENT {check}  {file}  {item}")
            failed = True
    print("differences to explain" if failed else "every difference is explained")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
