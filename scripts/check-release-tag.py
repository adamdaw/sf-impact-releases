#!/usr/bin/env python3
"""Refuse a tag that cannot identify the versioned, reviewed source on main."""
import re
import subprocess
import sys
import tomllib


def git(*args):
    return subprocess.check_output(['git', *args], text=True, stderr=subprocess.PIPE).strip()


def main():
    if len(sys.argv) != 2 or not re.fullmatch(r'v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)', sys.argv[1]):
        sys.exit('release tag: expected v<MAJOR>.<MINOR>.<PATCH> (no suffixes or leading zeroes)')
    tag = sys.argv[1]
    commit = git('rev-parse', '--verify', f'refs/tags/{tag}^{{commit}}')
    if commit != git('rev-parse', 'HEAD'):
        sys.exit('release tag: checkout is not the tagged commit')
    manifest = tomllib.loads(git('show', f'{commit}:Cargo.toml'))
    version = manifest['workspace']['package']['version']
    if tag != f'v{version}':
        sys.exit(f'release tag: {tag} does not match workspace version {version}')
    try:
        git('rev-parse', '--verify', 'refs/remotes/origin/main')
    except subprocess.CalledProcessError:
        sys.exit('release tag: cannot verify origin/main; fetch its full history')
    if subprocess.run(['git', 'merge-base', '--is-ancestor', commit,
                       'refs/remotes/origin/main'], check=False).returncode:
        sys.exit('release tag: tagged commit is not reachable from origin/main')
    print(f'Verified {tag}: {commit} is on origin/main and matches Cargo version {version}')


if __name__ == '__main__':
    try:
        main()
    except subprocess.CalledProcessError as error:
        sys.exit(f'release tag: git verification failed: {error.stderr.strip()}')
