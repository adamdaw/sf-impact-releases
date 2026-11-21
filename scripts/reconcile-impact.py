#!/usr/bin/env python3
"""Reconcile impact depth-one sites against an independent whole-word grep.

The words in impact-symbols.json were read from the pinned source declarations,
not from refs.jsonl. They include a class's members (names shorter than six are
not textual patterns). Overloads share a word but have distinct parsed counts.
Every differing file/line and its three counts needs an exact reviewed entry in
impact-accepted.tsv. A changed count is a new difference, never auto-accepted.
"""
import argparse
import collections
import json
import os
from pathlib import Path
import re
import subprocess

HERE = Path(__file__).resolve().parent


def scoped_files(root):
    project = json.loads((root / 'sfdx-project.json').read_text())
    paths = set()
    for package in project['packageDirectories']:
        dirs = [package['path']]
        if package.get('unpackagedMetadata'):
            dirs.append(package['unpackagedMetadata']['path'])
        for directory in dirs:
            directory = directory.replace('\\', '/')
            for parent, children, files in os.walk(root / directory, followlinks=False):
                children[:] = [d for d in children if d != '.git' and not (Path(parent) / d).is_symlink()]
                for name in files:
                    path = Path(parent) / name
                    if not path.is_symlink():
                        paths.add(str(path.relative_to(root)))
    return sorted(paths)


def grep(root, files, words):
    # GNU grep emits one row per occurrence, not one per matching line.
    found = collections.Counter()
    for start in range(0, len(files), 100):
        result = subprocess.run(['grep', '-HnIowiE', '|'.join(re.escape(w) for w in words), '--', *files[start:start + 100]], cwd=root, capture_output=True, text=True)
        if result.returncode not in (0, 1):
            raise RuntimeError(result.stderr)
        for line in result.stdout.splitlines():
            path, number, _ = line.rsplit(':', 2)
            found[path, int(number)] += 1
    return found


def sites(seed, tier, references):
    # The same input reference can reach colliding source declarations. Counts
    # count original rows once, not once per ambiguous endpoint.
    rows = {json.dumps(references[str(r)], sort_keys=True) for group in seed['referrers'] for hit in group[tier] for r in hit['references']}
    result = collections.Counter()
    for row in rows:
        ref = json.loads(row)
        result[ref['file'], ref['line']] += 1
    assert sum(result.values()) == seed['counts'][tier], (seed['symbol'], tier, seed['counts'], result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bin-dir', type=Path, required=True)
    parser.add_argument('--clone', action='append', required=True, help='Name=/absolute/path')
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    clones = {name: Path(path).resolve() for name, path in (entry.split('=', 1) for entry in args.clone)}
    manifest = json.loads((HERE / 'impact-symbols.json').read_text())
    accepted = {}
    accepted_file = HERE / 'impact-accepted.tsv'
    if accepted_file.exists():
        for line in accepted_file.read_text().splitlines():
            if line and not line.startswith('#'):
                *key, explanation = line.split('\t')
                assert explanation.strip(), line
                accepted[tuple(key)] = explanation
    args.out.mkdir(parents=True, exist_ok=True)
    differences, counts, missing, used = [], [], [], set()
    for name, corpus in manifest.items():
        root = clones[name]
        head = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
        assert head == corpus['commit'], (name, head, corpus['commit'])
        files = scoped_files(root)
        directory = args.out / name
        directory.mkdir(exist_ok=True)
        refs = directory / 'refs.jsonl'
        subprocess.run([str(args.bin_dir / 'sf-impact-extract'), '--root', str(root), '--out', str(refs)], check=True)
        for query, expected in corpus['queries'].items():
            output = directory / (query + '.json')
            subprocess.run([str(args.bin_dir / 'sf-impact-tests'), '--refs', str(refs), '--symbol', query, '--depth', '1', '--out', str(output)], check=True)
            impact = json.loads(output.read_text())
            assert {s['symbol'] for s in impact['seeds']} == set(expected), (name, query)
            for seed in impact['seeds']:
                manual = grep(root, files, expected[seed['symbol']])
                parsed, textual = sites(seed, 'parsed', impact['references']), sites(seed, 'textual', impact['references'])
                counts.append([name, query, seed['symbol'], sum(manual.values()), sum(parsed.values()), sum(textual.values())])
                for path, line in sorted(manual.keys() | parsed.keys() | textual.keys()):
                    g, p, t = manual[path, line], parsed[path, line], textual[path, line]
                    if g == p == t:
                        continue
                    key = tuple(map(str, [name, query, seed['symbol'], path, line, g, p, t]))
                    explanation = accepted.get(key, '')
                    source = (root / path).read_text(errors='replace').splitlines()[line - 1]
                    differences.append([*key, explanation, source.strip()])
                    if not explanation:
                        missing.append(key)
                    else:
                        used.add(key)
    (args.out / 'counts.json').write_text(json.dumps(counts, indent=2) + '\n')
    (args.out / 'differences.tsv').write_text('\n'.join('\t'.join(row) for row in differences) + '\n')
    for row in counts:
        print('\t'.join(map(str, row)))
    stale = accepted.keys() - used
    print(f'{len(counts)} seed identities; {len(differences)} differing lines; {len(missing)} unexplained; {len(stale)} stale explanations')
    if missing or stale:
        raise SystemExit(1)


if __name__ == '__main__':
    main()
