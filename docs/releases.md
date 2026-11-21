# Release maintenance

A `v<MAJOR>.<MINOR>.<PATCH>` tag triggers `.github/workflows/release.yml`.
Review the release, merge it to `main`, and confirm the workspace Cargo version
before tagging. Tags must have exactly three numeric components, with no leading
zeroes or suffixes, match that version, and point to a commit reachable from
`origin/main`. The workflow checks these conditions and reruns the full CI
workflow on the tagged commit; publication requires both checks and all four
native builds to pass. Publishing is restricted to `adamdaw/sf-impact-releases`.

The four native builds run on PRs changing release/CI workflows, packaging and
license scripts, the installer, Cargo manifests/lockfile, or packaged notices
and documentation. Ordinary Rust source changes run the full CI workflow
(including a static musl build) on every PR and every push to `main`. Every tag
still builds all four targets, so Rust changes affecting other release targets
must pass those builds before publication. Only the publishing job gets
`contents: write`; all other jobs have `contents: read`, and actions are pinned
by commit SHA.

## Build choices

All targets build natively on GitHub-hosted runners: `ubuntu-24.04` for x86_64
musl, `ubuntu-24.04-arm` for aarch64 musl, `macos-15-intel` for Intel macOS and
`macos-15` for Apple Silicon. Native builds keep the tree-sitter C grammar on
the runner's ordinary compiler, avoiding cross containers and extra toolchains.
The [runner labels](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
are architecture-specific. Rust is pinned to 1.100.0; every build uses
`--workspace --bins --locked`. Linux uses musl-gcc and rejects ELF interpreters
and shared dependencies for each of the five binaries. Every binary is smoke
run with `--version`; macOS additionally checks that only system libraries link.

Release builds set SOURCE_DATE_EPOCH from the source commit and remap checkout
and Cargo paths in Rust and C on every target. Packaging sorts
members, fixes modes, ownership and timestamps, and removes gzip filename/time
variation. Identical binary and notice bytes with the same epoch produce identical
archives. Bit-for-bit rebuilding across runner updates is not promised: runner
images, system C/linker versions, macOS SDKs and Python/zlib are not pinned.
The lockfile fixes Rust dependencies; it does not fix that system toolchain.

The archive name is stable across tags (`sf-impact-<target>.tar.gz`), allowing
GitHub's `latest/download` redirect. Each has a `.tar.gz.sha256` sidecar;
`SHA256SUMS` concatenates the four sidecars. All are attached to the release.
Tags and assets must be treated as immutable after publication.

## Dependency notices

`about.toml` is the explicit allow-list, in preference order:
**MIT, MIT-0, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Unicode-3.0, Zlib**.
A dual-license expression may use an allowed alternative; every required term
of an AND expression must be allowed. No crate-specific license exemptions exist.

`cargo-about 0.9.3` is downloaded by `scripts/install-cargo-about.sh` from its
upstream release and checked against a pinned SHA-256 digest. The
[upstream generator](https://github.com/EmbarkStudios/cargo-about/tree/0.9.3/docs/src/cli/generate)
harvests the locked workspace's dependencies, including build/test dependencies
with no platform filter, offline after `cargo fetch --locked`. Disabled optional
dependencies and impossible target predicates can remain in Cargo.lock without
belonging to the build graph; they are not shipped or included in the notices. This avoids mutable
remote license services. The checked-in THIRD_PARTY_LICENSES is regenerated in
CI; both drift and any unsatisfied license expression fail the job.

The published `tree-sitter-sfapex 3.0.2` crate omits LICENSE. Its upstream text is
vendored in `licenses/tree-sitter-sfapex-LICENSE`, copied verbatim from upstream
and checked against [the source commit's LICENSE](https://github.com/aheber/tree-sitter-sfapex/blob/9c07659fb5697b5db8d57a9bd4478e5577baf42c/LICENSE).
The generator appends that copyright notice and source URL. A changed crate
version or changed vendored bytes fails until reviewed; cargo-about's generic
MIT text alone is not a substitute for the upstream copyright notice.

`scripts/check-license-notices.py` also checks cargo-about's structured report
against the normal/build dependency tree for all targets. A dependency covered
only by unfilled copyright template text fails the gate. The workspace crates
use the packaged project MIT LICENSE; sfapex uses the version/hash-checked notice
above. Placeholder text for dev-only dependencies may remain in the report.

Regenerate after dependency changes, using Python 3.11+:

```sh
scripts/install-cargo-about.sh "$PWD/target/license-tools"
export PATH="$PWD/target/license-tools:$PATH"
scripts/licenses.sh
python3 scripts/test-license-gate.py
python3 scripts/test-license-notices.py
```

The gate test first checks that notices cover the normal, build and test
dependency trees for all four release targets. Its negative check runs the real generator with an empty allow-list against
the real lockfile. It requires a license-resolution error and no generated output,
so tool setup or network failures cannot masquerade as a successful refusal test.
The notice regression tests use the real generator and a locked local fixture:
`num-cmp 0.1.0` is allowed as a dev dependency, but promoting it to a normal
dependency fails because its harvested notice has an unfilled copyright.

## Local checks

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --workspace --bins --locked --target x86_64-unknown-linux-musl
scripts/check-static.sh target/x86_64-unknown-linux-musl/release
actionlint .github/workflows/*.yml
shellcheck install.sh scripts/check-static.sh scripts/licenses.sh scripts/install-cargo-about.sh
python3 scripts/test-release.py
python3 scripts/test-release-tag.py
python3 scripts/test-license-gate.py
python3 scripts/test-license-notices.py
python3 scripts/package-release.py x86_64-unknown-linux-musl target/x86_64-unknown-linux-musl/release target/dist
(cd target/dist && sha256sum --check ./*.sha256)
```

The installer tests use a fake local release directory, real archives and real
hashers. curl reads file URLs; real wget (or wget2) uses a loopback HTTP server
without contacting an external release. Install wget to run the tests locally.
CI runs the installer tests on both Linux and real macOS, using the native
`uname`, tar, mktemp and hasher. Separate platform mapping tests use a uname double.
Tests cover missing/404/500 checksums, mismatches, short responses, redirect loops,
download deadlines, default/latest and explicit versions, custom prefixes, PATH
hints, separate OS/architecture refusals, and truncated piped scripts. The script
body is parsed as a function before its final invocation, so an incomplete body
cannot write into the current directory. Packaging tests check exact membership,
notices, checksums and reproducible archive metadata.

Downloads have bounded retries/redirects and a 60-second wall-clock limit
per file (`SF_IMPACT_DOWNLOAD_TIMEOUT` accepts a positive number of seconds to override
it). Normal installation requires curl with HTTPS-only redirects. The test-only
wget path confines caches to temporary XDG/TMP directories.
`SF_IMPACT_RELEASE_BASE_URL` allows a release mirror. `$base/latest` must
redirect to `$base/tag/v<semver>` and the assets must live under
`$base/download/v<semver>/`. `<semver>` is strictly `MAJOR.MINOR.PATCH`
(digits only, no leading zeros); any other resolved tag, or a redirect outside
`$base`, is refused before anything is downloaded. HTTPS is
required; `SF_IMPACT_INSECURE_TEST_BASE=1` permits HTTP and file URLs only for
local tests, and in that test-only mode `latest` is not resolved once: each
file is fetched from `$base/latest/download`. Otherwise the installer resolves
`latest` once to a release tag before downloading both files; `--version`
selects a specific release (same strict form, leading `v` optional).

Checksums detect corruption, not a compromised publisher. Verify an archive's
GitHub attestation with:

```sh
gh attestation verify <archive> --repo adamdaw/sf-impact-releases \
  --signer-workflow adamdaw/sf-impact-releases/.github/workflows/release.yml
```

Check the signer and workflow identity in the verification output. The release
workflow attests the archives and `SHA256SUMS`; `SHA256SUMS` is the authoritative
checksum list. The per-archive `*.sha256` sidecar files are convenience copies
and are not attested. macOS signing/notarization is not provided. Releases are
not published to crates.io and do not include a Homebrew package.
