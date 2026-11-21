#!/usr/bin/env python3
"""Validate copyright coverage before appending the reviewed upstream notice."""
import hashlib
import json
import re
import subprocess
from pathlib import Path
import sys
import tomllib

# Normalize harvested CRLF text; the vendored notice below stays verbatim.
generated = Path(sys.argv[1])
generated.write_bytes(generated.read_bytes().replace(b'\r\n', b'\n'))
lock = tomllib.loads(Path('Cargo.lock').read_text())
versions = [p['version'] for p in lock['package'] if p['name'] == 'tree-sitter-sfapex']
if versions != ['3.0.2']:
    sys.exit('Review the vendored tree-sitter-sfapex notice for this new version')
notice = Path('licenses/tree-sitter-sfapex-LICENSE').read_bytes()
if hashlib.sha256(notice).hexdigest() != 'c3d045abbc0a8dec1b00737c0668f6dbb028d248521b74e79778c726671925ed':
    sys.exit('Vendored tree-sitter-sfapex license differs from the reviewed upstream text')
# cargo-about may fall back to SPDX template text when a crate omits a file.
# A template's unfilled copyright is not a distributable upstream notice.
report = json.loads(Path(sys.argv[2]).read_text())
metadata = json.loads(subprocess.check_output([
    'cargo', 'metadata', '--no-deps', '--format-version', '1', '--locked', '--offline']))
workspace = {(p['name'], p['version']) for p in metadata['packages']
             if p['id'] in metadata['workspace_members'] and p['license'] == 'MIT'}
project_notice = Path('LICENSE').read_text()
placeholder = re.compile(r'<year>|<copyright holders?>|\[year\]|\[fullname\]', re.IGNORECASE)
if placeholder.search(project_notice) or 'Permission is hereby granted' not in project_notice:
    sys.exit('Review the workspace MIT license text')
# The workspace MIT text is shipped as LICENSE; sfapex's exact upstream notice
# was version/hash checked above and is appended below. No license-policy bypass.
provided_mit_notices = workspace | {('tree-sitter-sfapex', '3.0.2')}
tree = subprocess.check_output([
    'cargo', 'tree', '--workspace', '--locked', '--offline', '--color', 'never',
    '--target', 'all', '--edges', 'normal,build', '--prefix', 'none', '--format', '{p}'], text=True)
normal = set(re.findall(r'^(\S+) v(\S+)', tree, re.MULTILINE))
for license in report['licenses']:
    unfilled = placeholder.search(license['text'])
    # Apache's standard appendix is legitimate in an upstream file, but a
    # synthesized template with no source file cannot supply a copyright notice.
    if license['source_path'] is None:
        unfilled = unfilled or re.search(r'\[yyyy\]|\[name of copyright owner\]', license['text'])
    if not unfilled:
        continue
    covered = {(entry['crate']['name'], entry['crate']['version']) for entry in license['used_by']}
    missing = (normal & covered) - (provided_mit_notices if license['id'] == 'MIT' else set())
    if missing:
        sys.exit('placeholder license text for normal/build dependencies: '
                 + ', '.join(f'{name} {version}' for name, version in sorted(missing)))

with Path(sys.argv[1]).open('ab') as out:
    out.write(b'\n===============================================================================\n'
              b'tree-sitter-sfapex 3.0.2: upstream MIT copyright notice\n'
              b'The published crate omits this file; included verbatim from:\n'
              b'https://github.com/aheber/tree-sitter-sfapex/blob/'
              b'9c07659fb5697b5db8d57a9bd4478e5577baf42c/LICENSE\n\n')
    out.write(notice)
    out.write(b'\n')
