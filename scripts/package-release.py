#!/usr/bin/env python3
"""Package prebuilt commands with deterministic metadata, then hash the archive."""
import argparse
import gzip
import hashlib
import os
from pathlib import Path
import subprocess
import tarfile

ROOT = Path(__file__).resolve().parents[1]
COMMANDS = ('sf-impact-extract', 'sf-impact-delta', 'sf-impact-changed', 'sf-impact-tests', 'sf-impact-render')
TARGETS = ('x86_64-unknown-linux-musl', 'aarch64-unknown-linux-musl',
           'x86_64-apple-darwin', 'aarch64-apple-darwin')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('target', choices=TARGETS)
    parser.add_argument('bin_dir', type=Path)
    parser.add_argument('out_dir', type=Path)
    args = parser.parse_args()
    files = [(name, args.bin_dir / name, 0o755) for name in COMMANDS]
    files += [(name, ROOT / source, 0o644) for name, source in (
        ('LICENSE', 'LICENSE'), ('THIRD_PARTY_LICENSES', 'THIRD_PARTY_LICENSES'),
        ('README.md', 'docs/release-README.md'))]
    for _, path, _ in files:
        if not path.is_file() or path.is_symlink():
            parser.error(f'missing regular file: {path}')
    epoch = os.environ.get('SOURCE_DATE_EPOCH')
    if epoch is None:
        epoch = subprocess.check_output(['git', 'log', '-1', '--format=%ct'], cwd=ROOT, text=True).strip()
    epoch = int(epoch)
    args.out_dir.mkdir(parents=True, exist_ok=True)
    archive = args.out_dir / f'sf-impact-{args.target}.tar.gz'
    with archive.open('wb') as raw:
        with gzip.GzipFile(filename='', fileobj=raw, mode='wb', mtime=epoch) as compressed:
            with tarfile.open(fileobj=compressed, mode='w', format=tarfile.USTAR_FORMAT) as tar:
                for name, path, mode in sorted(files):
                    info = tarfile.TarInfo(name)
                    info.size = path.stat().st_size
                    info.mode = mode
                    info.mtime = epoch
                    with path.open('rb') as content:
                        tar.addfile(info, content)
    checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
    (args.out_dir / (archive.name + '.sha256')).write_text(f'{checksum}  {archive.name}\n')
    print(f'{checksum}  {archive.name}')
    with tarfile.open(archive) as tar:
        for member in tar:
            print(f'{member.mode:04o} {member.size:9d} {member.name}')


if __name__ == '__main__':
    main()
