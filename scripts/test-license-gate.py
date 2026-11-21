#!/usr/bin/env python3
"""The real generator must refuse an existing dependency outside the allow-list."""
from pathlib import Path
import re
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
notices = set(re.findall(r'^- (\S+) (\S+)$',
                         (root / 'THIRD_PARTY_LICENSES').read_text(), re.MULTILINE))
for target in ('x86_64-unknown-linux-musl', 'aarch64-unknown-linux-musl',
               'x86_64-apple-darwin', 'aarch64-apple-darwin'):
    tree = subprocess.check_output([
        'cargo', 'tree', '--locked', '--offline', '--color', 'never',
        '--target', target, '--prefix', 'none', '--format', '{p}',
        '--edges', 'normal,build,dev'], cwd=root, text=True)
    packages = set(re.findall(r'^(\S+) v(\S+)', tree, re.MULTILINE))
    assert packages, f'empty dependency tree for {target}'
    assert packages <= notices, f'{target}: missing notices for {packages - notices}'
print('PASS: notices cover normal, build and test dependency trees for all four targets')

with tempfile.TemporaryDirectory() as directory:
    directory = Path(directory)
    config = directory / 'deny-all.toml'
    config.write_text('accepted = []\n')
    output = directory / 'THIRD_PARTY_LICENSES'
    result = subprocess.run(['sh', 'scripts/licenses.sh', str(output), str(config)],
                            cwd=root, text=True, capture_output=True)
    assert result.returncode != 0, 'generator accepted dependencies with no allowed licenses'
    assert 'failed to satisfy license requirements' in result.stderr, result.stderr
    assert not output.exists(), 'failed license gate wrote release notices'
    print('PASS: real locked dependencies rejected with an empty allow-list; no notices emitted')
