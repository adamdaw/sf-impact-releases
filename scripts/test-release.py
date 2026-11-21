#!/usr/bin/env python3
"""Exercise the installer with real archives and a local, offline release."""
import contextlib
import hashlib
import http.server
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import tarfile
import tempfile
import threading
import time
import unittest

# Fake curl: logs every invocation to curl-calls, answers the `latest`
# resolution with $TEST_LATEST_URL / $TEST_LATEST_EXIT, serves any other
# download from the fixture, and records those requests in archive-calls.
FAKE_CURL = r'''#!/bin/sh
printf '%s\n' "$*" >> curl-calls
prev=
for arg do
  [ "$prev" = -o ] && output=$arg
  prev=$arg
done
case "$arg" in
  */latest) printf '%s' "$TEST_LATEST_URL"; exit "${TEST_LATEST_EXIT:-0}";;
  *.sha256) printf '%s\n' "$arg" >> archive-calls; cp "$TEST_CHECKSUM" "$output";;
  *) printf '%s\n' "$arg" >> archive-calls; cp "$TEST_ARCHIVE" "$output";;
esac
'''

ROOT = Path(__file__).resolve().parents[1]
COMMANDS = ['sf-impact-extract', 'sf-impact-delta', 'sf-impact-changed', 'sf-impact-tests', 'sf-impact-render']
INSTALLER = Path(os.environ.get('SF_IMPACT_TEST_INSTALLER', ROOT / 'install.sh'))
MACHINE = subprocess.check_output(['uname', '-m'], text=True).strip()
SYSTEM = subprocess.check_output(['uname', '-s'], text=True).strip()
TARGET = {'arm64': 'aarch64', 'aarch64': 'aarch64', 'x86_64': 'x86_64'}[MACHINE]
TARGET += '-apple-darwin' if SYSTEM == 'Darwin' else '-unknown-linux-musl'

class ReleaseFixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.release = self.root / 'releases/download/v0.1.0'
        self.release.mkdir(parents=True)
        self.payload = self.root / 'payload'
        self.payload.mkdir()
        for command in COMMANDS:
            path = self.payload / command
            path.write_text('#!/bin/sh\necho "' + command + ' 0.1.0"\n')
            path.chmod(0o755)
        for name in ['LICENSE', 'THIRD_PARTY_LICENSES', 'README.md']:
            (self.payload / name).write_text(name + '\n')
        self.archive = self.release / f'sf-impact-{TARGET}.tar.gz'
        with tarfile.open(self.archive, 'w:gz') as archive:
            for path in self.payload.iterdir():
                archive.add(path, arcname=path.name)
        self.checksum = self.release / (self.archive.name + '.sha256')
        self.checksum.write_text(hashlib.sha256(self.archive.read_bytes()).hexdigest()
                                 + '  ' + self.archive.name + '\n')
        latest = self.root / 'releases/latest'
        latest.mkdir()
        (latest / 'download').symlink_to(self.release, target_is_directory=True)
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        # Isolated PATH selects real downloaders, hashers and native OS utilities.
        for command in ['sh', 'mkdir', 'mktemp', 'rm', 'tar', 'gzip', 'cp', 'chmod', 'awk', 'cat', 'tr', 'cut', 'uname', 'sleep']:
            (self.bin / command).symlink_to(shutil.which(command))
        (self.bin / 'curl').symlink_to(shutil.which('curl'))
        hasher = 'shasum' if SYSTEM == 'Darwin' else 'sha256sum'
        (self.bin / hasher).symlink_to(shutil.which(hasher))
        self.uname = self.bin / 'uname'
        self.home = self.root / 'home'
        self.home.mkdir()
        self.env = dict(os.environ, PATH=str(self.bin), HOME=str(self.home),
                        TMPDIR=str(self.root), XDG_DATA_HOME=str(self.home / '.local/share'),
                        SF_IMPACT_RELEASE_BASE_URL=(self.root / 'releases').as_uri(),
                        SF_IMPACT_INSECURE_TEST_BASE='1')

    def use_fake_https_curl(self, latest_url='', latest_exit=0):
        self.env.pop('SF_IMPACT_INSECURE_TEST_BASE', None)
        self.env['SF_IMPACT_RELEASE_BASE_URL'] = 'https://example.invalid/releases'
        curl = self.bin / 'curl'
        curl.unlink(missing_ok=True)
        curl.write_text(FAKE_CURL)
        curl.chmod(0o755)
        self.env.update(TEST_ARCHIVE=str(self.archive), TEST_CHECKSUM=str(self.checksum),
                        TEST_LATEST_URL=latest_url, TEST_LATEST_EXIT=str(latest_exit))

    def assert_fails_closed(self, result, message):
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(message, result.stderr)
        self.assertFalse((self.root / 'archive-calls').exists(), 'archive requested')
        self.assertFalse((self.home / '.local').exists())

    def install(self, *args):
        return self.run_shell(['/bin/sh', str(INSTALLER), *args])

    def run_shell(self, command, **kwargs):
        with subprocess.Popen(command, env=self.env, cwd=self.root, text=True,
                              stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, start_new_session=True) as process:
            try:
                stdout, stderr = process.communicate(kwargs.get('input'), timeout=kwargs.get('timeout', 15))
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGKILL)
                process.communicate()
                self.fail('installer exceeded its download time limit')
            return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)

    def fake_platform(self, system, machine):
        self.uname.unlink()
        self.uname.write_text(f'#!/bin/sh\ncase "$1" in -s) echo {system};; -m) echo {machine};; esac\n')
        self.uname.chmod(0o755)

    def use_real_wget(self):
        (self.bin / 'curl').unlink()
        downloader = shutil.which('wget') or shutil.which('wget2')
        self.assertIsNotNone(downloader, 'install wget or wget2 to run real downloader tests')
        (self.bin / 'wget').symlink_to(downloader)

    def use_busybox_wget(self):
        (self.bin / 'curl').unlink()
        busybox = shutil.which('busybox')
        if busybox:
            print('BusyBox wget test: running real BusyBox wget', flush=True)
            body = f'exec {shlex.quote(busybox)} wget "$@"\n'
        else:
            self.assertNotIn(os.environ.get('CI', '').lower(), ('true', '1'),
                             'real BusyBox wget is required in CI')
            print('BusyBox wget test: running curl-backed stand-in', flush=True)
            # Reject GNU-only flags and delegate portable transfers to curl.
            curl = shlex.quote(shutil.which('curl'))
            body = '''[ "$#" -eq 4 ] && [ "$1" = -q ] && [ "$2" = -O ] || {
    echo 'wget: unsupported option' >&2
    exit 1
}
''' + f'exec {curl} -fsSL -o "$3" "$4"\n'
        (self.bin / 'wget').write_text('#!/bin/sh\n' + body)
        (self.bin / 'wget').chmod(0o755)

    @contextlib.contextmanager
    def http_release(self, sidecar='ok'):
        archive_bytes = self.archive.read_bytes()
        checksum_bytes = self.checksum.read_bytes()

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                is_checksum = self.path.endswith('.sha256')
                if is_checksum and sidecar in ('404', '500'):
                    self.send_error(int(sidecar))
                    return
                if is_checksum and sidecar == 'loop':
                    self.send_response(302)
                    self.send_header('Location', self.path)
                    self.end_headers()
                    return
                body = checksum_bytes if is_checksum else archive_bytes
                self.send_response(200)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                if is_checksum and sidecar == 'short':
                    self.wfile.write(body[:4])
                elif is_checksum and sidecar == 'stall':
                    self.wfile.write(body[:4])
                    self.wfile.flush()
                    time.sleep(5)
                else:
                    self.wfile.write(body)
                self.close_connection = True

        server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.env['SF_IMPACT_RELEASE_BASE_URL'] = f'http://127.0.0.1:{server.server_port}'
        try:
            yield
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

    def assert_checksum_download_refused(self, result):
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn('Checksum download failed', result.stderr)
        self.assertFalse((self.home / '.local').exists())

    def assert_installed(self, prefix):
        self.assertEqual(sorted(p.name for p in prefix.iterdir()), sorted(COMMANDS))
        for command in COMMANDS:
            result = subprocess.run([str(prefix / command)], text=True, capture_output=True, check=True)
            self.assertEqual(result.stdout.strip(), command + ' 0.1.0')
        self.assertEqual(sorted(p.name for p in self.home.iterdir()), ['.local'])


class InstallTest(ReleaseFixture):
    def test_non_https_base_requires_test_opt_in(self):
        del self.env['SF_IMPACT_INSECURE_TEST_BASE']
        for base in ((self.root / 'releases').as_uri(), 'http://127.0.0.1:1'):
            self.env['SF_IMPACT_RELEASE_BASE_URL'] = base
            result = self.install('--version', 'v0.1.0')
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('Non-HTTPS downloads', result.stderr)

    def test_https_protocol_flags(self):
        del self.env['SF_IMPACT_INSECURE_TEST_BASE']
        self.env['SF_IMPACT_RELEASE_BASE_URL'] = 'https://example.invalid/releases'
        curl = self.bin / 'curl'
        curl.unlink()
        curl.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> curl-calls\nexit 1\n')
        curl.chmod(0o755)
        self.install('--version', 'v0.1.0')
        calls = (self.root / 'curl-calls').read_text().splitlines()
        self.assertTrue(calls)
        for call in calls:
            self.assertIn('--proto =https --proto-redir =https', call)

    def test_latest_resolved_once(self):
        del self.env['SF_IMPACT_INSECURE_TEST_BASE']
        self.env['SF_IMPACT_RELEASE_BASE_URL'] = 'https://example.invalid/releases'
        curl = self.bin / 'curl'
        curl.unlink()
        curl.write_text('''#!/bin/sh
printf '%s\\n' "$*" >> requests
args="$*"
while [ "$#" -gt 0 ]; do
  if [ "$1" = -o ]; then output=$2; break; fi
  shift
done
case "$args" in
  *'/releases/latest') printf '%s' 'https://example.invalid/releases/tag/v0.1.0';;
  *'/download/v0.1.0/'*.sha256) cp "$TEST_CHECKSUM" "$output";;
  *'/download/v0.1.0/'*) cp "$TEST_ARCHIVE" "$output";;
  *) exit 1;;
esac
''')
        curl.chmod(0o755)
        self.env.update(TEST_ARCHIVE=str(self.archive), TEST_CHECKSUM=str(self.checksum))
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        requests = (self.root / 'requests').read_text().splitlines()
        self.assertEqual(len(requests), 3)
        self.assertEqual(sum('/releases/latest' in request for request in requests), 1)
        self.assertIn('--proto =https --proto-redir =https', requests[0])
        self.assertTrue(all('/download/v0.1.0/' in request for request in requests[1:]))

    def test_latest_tag_must_be_strict_semver(self):
        base = 'https://example.invalid/releases/tag/'
        tags = ['v..', 'v.', 'vabc', 'v1', 'v1.2', 'v1.2.3.4', 'v1.2.3-rc.1', 'v01.02.03',
                'v1.02.3', 'v--help', 'v1.2.3/../x', 'v1 2', '1.2.3', '', 'v', 'v1.2.3\nx',
                'v1.2.3?x', 'v1.2.']
        for tag in tags:
            with self.subTest(tag=tag):
                self.use_fake_https_curl(latest_url=base + tag)
                self.assert_fails_closed(self.install(), 'Invalid latest release')

    def test_latest_effective_url_must_be_under_https_base(self):
        urls = ['https://evil.invalid/releases/tag/v0.1.0',
                'https://example.invalid/releases-evil/tag/v0.1.0',
                'http://example.invalid/releases/tag/v0.1.0',
                '', 'https://example.invalid/releases/latest']
        for url in urls:
            with self.subTest(url=url):
                self.use_fake_https_curl(latest_url=url)
                self.assert_fails_closed(self.install(), 'Invalid latest release URL')

    def test_latest_resolution_failure_fails_closed(self):
        self.use_fake_https_curl(latest_url='https://example.invalid/releases/tag/v0.1.0',
                                 latest_exit=22)
        self.assert_fails_closed(self.install(), 'Latest release resolution failed')

    def test_latest_accepts_multi_digit_semver(self):
        self.use_fake_https_curl(latest_url='https://example.invalid/releases/tag/v10.0.20')
        result = self.install()
        self.assertEqual(result.returncode, 0, result.stderr)
        archive_calls = (self.root / 'archive-calls').read_text().splitlines()
        self.assertEqual(len(archive_calls), 2)
        self.assertTrue(all('/download/v10.0.20/' in call for call in archive_calls))

    def test_explicit_version_must_be_strict_semver(self):
        self.use_fake_https_curl()
        for version in ['v..', 'v1', 'v1.2', 'v1.2.3.4', 'v1.2.3-rc.1', 'v01.02.03',
                        'v--help', '--help', 'v1.2.3/../x', 'v1 2', 'vabc', 'v']:
            with self.subTest(version=version):
                self.assert_fails_closed(self.install('--version', version), 'Invalid version')
                self.assertFalse((self.root / 'curl-calls').exists(), 'curl invoked')

    def test_explicit_version_and_default_prefix(self):
        result = self.install('--version', 'v0.1.0')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_installed(self.home / '.local/bin')
        self.assertIn('sf-impact: downloader: curl\n', result.stderr)
        self.assertIn('PATH', result.stdout)

    def test_closed_stderr_still_installs(self):
        for downloader in ('curl', 'wget'):
            with self.subTest(downloader=downloader):
                if downloader == 'wget':
                    self.use_real_wget()
                with self.http_release():
                    result = self.run_shell(['/bin/sh', '-c', 'exec /bin/sh "$@" 2>&-',
                                             'sh', str(INSTALLER), '--version', 'v0.1.0'])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assert_installed(self.home / '.local/bin')

    def test_latest_and_prefix_with_spaces(self):
        prefix = self.home / '.local/custom bin'
        self.env['PATH'] += ':' + str(prefix)
        result = self.install('--prefix', str(prefix))
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_installed(prefix)
        self.assertNotIn('PATH', result.stdout)

    def test_checksum_mismatch_leaves_existing_commands_untouched(self):
        prefix = self.home / '.local/bin'
        prefix.mkdir(parents=True)
        (prefix / 'sf-impact-extract').write_text('old installation')
        self.archive.write_bytes(self.archive.read_bytes() + b'corruption')
        result = self.install('--version', '0.1.0')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('checksum', result.stderr.lower())
        self.assertEqual([p.name for p in prefix.iterdir()], ['sf-impact-extract'])
        self.assertEqual((prefix / 'sf-impact-extract').read_text(), 'old installation')

    def test_missing_version_fails_without_installing(self):
        result = self.install('--version', 'v9.9.9')
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Archive download failed', result.stderr)
        self.assertFalse((self.home / '.local').exists())

    def test_incomplete_archive_installs_nothing(self):
        with tarfile.open(self.archive, 'w:gz') as archive:
            archive.add(self.payload / 'sf-impact-extract', arcname='sf-impact-extract')
        self.checksum.write_text(hashlib.sha256(self.archive.read_bytes()).hexdigest()
                                 + '  ' + self.archive.name + '\n')
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Cannot unpack release', result.stderr)
        self.assertFalse((self.home / '.local').exists())

    def test_checksum_must_name_the_selected_archive(self):
        self.checksum.write_text(self.checksum.read_text().replace(self.archive.name, 'other.tar.gz'))
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Checksum names a different archive', result.stderr)
        self.assertFalse((self.home / '.local').exists())

    def test_missing_checksum(self):
        self.checksum.unlink()
        self.assert_checksum_download_refused(self.install())

    def test_checksum_http_404(self):
        with self.http_release('404'):
            self.assert_checksum_download_refused(self.install())

    def test_checksum_http_500(self):
        with self.http_release('500'):
            self.assert_checksum_download_refused(self.install())

    def test_wget_mode_messages(self):
        (self.bin / 'curl').unlink()
        curl = shlex.quote(shutil.which('curl'))
        for banner, mode in [('GNU Wget 1.25.0', 'GNU Wget'),
                             ('GNU Wget2 2.2.0', 'Wget2'),
                             ('', 'basic wget (BusyBox or unknown, without GNU options)')]:
            with self.subTest(mode=mode):
                wget = self.bin / 'wget'
                wget.write_text('#!/bin/sh\n' +
                                'case "$*" in *--version*) ' +
                                (f"echo {shlex.quote(banner)}; exit 0" if banner else 'exit 1') +
                                ';; esac\n' +
                                'while [ "$1" != -q ]; do shift; done\n' +
                                f'exec {curl} -fsSL -o "$3" "$4"\n')
                wget.chmod(0o755)
                result = self.install()
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(f'sf-impact: downloader: {mode}\n', result.stderr)

    def test_real_wget_and_shasum(self):
        self.use_real_wget()
        (self.bin / 'sha256sum').unlink(missing_ok=True)
        if not (self.bin / 'shasum').exists():
            (self.bin / 'shasum').symlink_to(shutil.which('shasum'))
        before = set(self.root.iterdir())
        with self.http_release():
            result = self.install('--version', '0.1.0')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_installed(self.home / '.local/bin')
        self.assertEqual(set(self.root.iterdir()), before, 'downloader left temporary files')
        self.assertEqual(list((self.home / '.local').iterdir()), [self.home / '.local/bin'])

    def test_real_wget_short_checksum_body(self):
        self.use_real_wget()
        with self.http_release('short'):
            self.assert_checksum_download_refused(self.install())

    def test_real_wget_redirect_loop(self):
        self.use_real_wget()
        with self.http_release('loop'):
            self.assert_checksum_download_refused(self.install())

    def test_real_wget_wall_clock_deadline(self):
        self.use_real_wget()
        self.env['SF_IMPACT_DOWNLOAD_TIMEOUT'] = '1'
        started = time.monotonic()
        with self.http_release('stall'):
            self.assert_checksum_download_refused(self.install())
        self.assertLess(time.monotonic() - started, 4, 'download ignored the wall-clock deadline')

    def test_complete_piped_installer(self):
        prefix = self.home / '.local/piped bin'
        result = self.run_shell(['/bin/sh', '-s', '--', '--version', 'v0.1.0',
                                '--prefix', str(prefix)],
                                input=INSTALLER.read_text())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_installed(prefix)

    def test_truncated_pipe_never_writes_files(self):
        script = INSTALLER.read_text()
        # Every prefix before the final complete shell token must be inert,
        # including cuts just after `main` and `main ` in the last invocation.
        # Record attempts as well as persistent writes: cleanup could hide an
        # unintended download/installation attempt with default arguments.
        (self.bin / 'curl').unlink()
        (self.bin / 'curl').write_text('#!/bin/sh\n: > download-started\nexit 1\n')
        (self.bin / 'curl').chmod(0o755)
        before = set(self.root.iterdir())
        for end in range(len(script.rstrip())):
            self.run_shell(['/bin/sh', '-s', '--', '--version', 'v0.1.0',
                            '--prefix', str(self.home / '.local/custom')], input=script[:end])
            self.assertEqual(set(self.root.iterdir()), before, f'files written by byte-prefix {end}')
            self.assertEqual(list(self.home.iterdir()), [], f'HOME changed by byte-prefix {end}')


class PlatformTest(ReleaseFixture):
    def test_unsupported_platform(self):
        self.fake_platform('Plan9', 'x86_64')
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Unsupported operating system', result.stderr)
        self.assertFalse((self.home / '.local').exists())

    def install_platform(self, os_name, arch, target):
        self.fake_platform(os_name, arch)
        archive = self.release / f'sf-impact-{target}.tar.gz'
        archive.write_bytes(self.archive.read_bytes())
        (self.release / (archive.name + '.sha256')).write_text(
            hashlib.sha256(archive.read_bytes()).hexdigest() + '  ' + archive.name + '\n')
        result = self.install('--version', 'v0.1.0')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_installed(self.home / '.local/bin')

    def test_linux_arm(self):
        self.install_platform('Linux', 'aarch64', 'aarch64-unknown-linux-musl')

    def test_macos_intel(self):
        self.install_platform('Darwin', 'x86_64', 'x86_64-apple-darwin')

    def test_macos_apple_silicon(self):
        self.install_platform('Darwin', 'arm64', 'aarch64-apple-darwin')

    def test_unsupported_architecture(self):
        self.fake_platform('Linux', 'riscv64')
        result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('Unsupported architecture', result.stderr)
        self.assertFalse((self.home / '.local').exists())

class BusyBoxInstallTest(ReleaseFixture):
    # The Linux CI job supplies BusyBox; macOS runs InstallTest with native tools.
    def test_busybox_wget(self):
        self.use_busybox_wget()
        with self.http_release():
            result = self.install('--version', 'v0.1.0')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_installed(self.home / '.local/bin')
        self.assertIn('sf-impact: downloader: basic wget (BusyBox or unknown, without GNU options)\n',
                      result.stderr)

    def test_busybox_wget_checksum_refusals(self):
        self.use_busybox_wget()
        self.env['SF_IMPACT_DOWNLOAD_TIMEOUT'] = '1'
        for failure in ('404', '500', 'short', 'loop', 'stall'):
            started = time.monotonic()
            with self.subTest(failure=failure), self.http_release(failure):
                self.assert_checksum_download_refused(self.install())
                self.assertLess(time.monotonic() - started, 4, 'fallback ignored the deadline')
        self.archive.write_bytes(self.archive.read_bytes() + b'corruption')
        with self.http_release():
            result = self.install()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('SHA-256 checksum mismatch', result.stderr)
        self.assertFalse((self.home / '.local').exists())


class PackageTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / 'bin'
        self.bin.mkdir()
        for name in COMMANDS:
            (self.bin / name).write_bytes(name.encode())
        self.out = self.root / 'dist'

    def package(self):
        return subprocess.run(['python3', str(ROOT / 'scripts/package-release.py'),
                               TARGET, str(self.bin), str(self.out)],
                              env=dict(os.environ, SOURCE_DATE_EPOCH='1700000000'),
                              text=True, capture_output=True)

    def test_layout_checksums_and_reproducible_metadata(self):
        result = self.package()
        self.assertEqual(result.returncode, 0, result.stderr)
        archive = self.out / f'sf-impact-{TARGET}.tar.gz'
        first = archive.read_bytes()
        with tarfile.open(archive) as tar:
            self.assertEqual(tar.getnames(), [
                'LICENSE', 'README.md', 'THIRD_PARTY_LICENSES', 'sf-impact-changed',
                'sf-impact-delta', 'sf-impact-extract', 'sf-impact-render', 'sf-impact-tests'])
            self.assertIn(b'Copyright (c) 2026 Adam Daw', tar.extractfile('LICENSE').read())
            self.assertIn(b'Copyright (c) 2012 Anthony Heber',
                          tar.extractfile('THIRD_PARTY_LICENSES').read())
            self.assertTrue(tar.extractfile('README.md').read().startswith(b'sf-impact installs'))
            for member in tar:
                self.assertEqual(member.mtime, 1700000000)
                self.assertEqual((member.uid, member.gid, member.uname, member.gname), (0, 0, '', ''))
                self.assertEqual(member.mode, 0o755 if member.name in COMMANDS else 0o644)
        self.assertEqual((self.out / (archive.name + '.sha256')).read_text(),
                         hashlib.sha256(first).hexdigest() + '  ' + archive.name + '\n')
        for path in self.bin.iterdir():
            os.utime(path, (1800000000, 1800000000))
            path.chmod(0o777)
        self.assertEqual(self.package().returncode, 0)
        self.assertEqual(archive.read_bytes(), first)

    def test_missing_command_refuses_to_package(self):
        (self.bin / 'sf-impact-render').unlink()
        result = self.package()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('missing regular file', result.stderr)
        self.assertFalse(self.out.exists())


if __name__ == '__main__':
    unittest.main()
