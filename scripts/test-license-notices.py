#!/usr/bin/env python3
"""Promote a real placeholder-notice crate from dev to normal dependency."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


class NoticeTest(unittest.TestCase):
    def generate(self, dependency_kind):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        fixture = Path(temporary.name)
        for directory in ('scripts', 'licenses'):
            shutil.copytree(ROOT / directory, fixture / directory)
        for name in ('about.toml', 'about.hbs', 'LICENSE'):
            shutil.copyfile(ROOT / name, fixture / name)
        (fixture / 'src').mkdir()
        (fixture / 'src/lib.rs').write_text('')
        (fixture / 'Cargo.toml').write_text('''[package]
name = "notice-fixture"
version = "0.1.0"
edition = "2021"
license = "MIT"
publish = false
[dependencies]
tree-sitter-sfapex = "=3.0.2"
''' + ('' if dependency_kind == 'normal' else '[dev-dependencies]\n')
            + 'num-cmp = "=0.1.0"\n')
        subprocess.run(['cargo', 'generate-lockfile', '--offline'], cwd=fixture,
                       capture_output=True, check=True)
        output = fixture / 'THIRD_PARTY_LICENSES'
        result = subprocess.run(['sh', 'scripts/licenses.sh', str(output)],
                                cwd=fixture, text=True, capture_output=True)
        return result, output

    def test_normal_dependency_cannot_use_placeholder(self):
        result, output = self.generate('normal')
        self.assertNotEqual(result.returncode, 0, 'normal num-cmp accepted a placeholder notice')
        self.assertIn('placeholder license text', result.stderr)
        self.assertIn('num-cmp 0.1.0', result.stderr)
        self.assertFalse(output.exists(), 'failed notice gate wrote release notices')

    def test_dev_placeholder_and_reviewed_vendored_notice(self):
        result, output = self.generate('dev')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('Copyright (c) 2012 Anthony Heber', output.read_text())
        self.assertIn('- num-cmp 0.1.0', output.read_text())


if __name__ == '__main__':
    unittest.main()
