#!/usr/bin/env python3
"""Release tags must describe the checked-out, versioned commit on origin/main."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().with_name('check-release-tag.py')


class ReleaseTagTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.env = dict(os.environ, GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_SYSTEM=os.devnull,
                        GIT_AUTHOR_NAME='Fixture', GIT_AUTHOR_EMAIL='fixture@example.invalid',
                        GIT_COMMITTER_NAME='Fixture', GIT_COMMITTER_EMAIL='fixture@example.invalid')
        self.git('init', '-b', 'main')
        (self.root / 'Cargo.toml').write_text('[workspace.package]\nversion = "0.1.0"\n')
        self.git('add', 'Cargo.toml')
        self.git('commit', '-m', 'fixture')
        self.git('update-ref', 'refs/remotes/origin/main', 'HEAD')

    def git(self, *args):
        return subprocess.check_output(['git', *args], cwd=self.root, env=self.env,
                                       text=True, stderr=subprocess.STDOUT).strip()

    def check(self, tag='v0.1.0'):
        return subprocess.run(['python3', str(SCRIPT), tag], cwd=self.root, env=self.env,
                              text=True, capture_output=True)

    def assert_refused(self, result, message):
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(message, result.stderr)

    def test_versioned_tag_on_main(self):
        self.git('tag', '-a', 'v0.1.0', '-m', 'fixture tag')
        result = self.check()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_malformed_tag(self):
        for tag in ('vnext', 'v0.1', 'v0.1.0-rc1', 'v0.1.0+build', 'v00.1.0', 'v0.1.0\n'):
            with self.subTest(tag=tag):
                self.assert_refused(self.check(tag), 'expected v<MAJOR>.<MINOR>.<PATCH>')

    def test_tag_version_mismatch(self):
        self.git('tag', 'v0.2.0')
        self.assert_refused(self.check('v0.2.0'), 'does not match workspace version')

    def test_commit_outside_main(self):
        self.git('checkout', '-b', 'unmerged')
        self.git('commit', '--allow-empty', '-m', 'not on main')
        self.git('tag', 'v0.1.0')
        self.assert_refused(self.check(), 'not reachable from origin/main')

    def test_old_commit_on_main_is_allowed(self):
        self.git('tag', 'v0.1.0')
        self.git('commit', '--allow-empty', '-m', 'later main commit')
        self.git('update-ref', 'refs/remotes/origin/main', 'HEAD')
        self.git('checkout', 'v0.1.0')
        result = self.check()
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_checkout_must_be_the_tagged_commit(self):
        self.git('tag', 'v0.1.0')
        self.git('commit', '--allow-empty', '-m', 'different checkout')
        self.assert_refused(self.check(), 'checkout is not the tagged commit')

    def test_missing_main_ref_is_refused(self):
        self.git('tag', 'v0.1.0')
        self.git('update-ref', '-d', 'refs/remotes/origin/main')
        self.assert_refused(self.check(), 'cannot verify origin/main')


if __name__ == '__main__':
    unittest.main()
