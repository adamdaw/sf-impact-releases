# Install

The first release, `v0.1.0`, is not published yet. Until it is, build the
commands from source (see [Build from source](#build-from-source)). The rest of
this page describes the installer that works once a release exists.

## Install a release

```sh
curl -fsSL --proto '=https' --proto-redir '=https' https://raw.githubusercontent.com/adamdaw/sf-impact-releases/v0.1.0/install.sh -o install.sh
sh install.sh --version v0.1.0
```

The script installs all five commands into `~/.local/bin`, checks the archive's
SHA-256 checksum before unpacking, and refuses a mismatch.

- `--prefix /path/to/bin` chooses another directory.
- `--version` defaults to `latest`. `latest` is resolved once to a release tag,
  which must be strictly `MAJOR.MINOR.PATCH` (digits only, no leading zeros);
  both files are then downloaded from that tag. `--version v0.1.0` selects one
  release (the leading `v` is optional) and is checked in the same way.
- It needs curl, which enforces HTTPS for the downloads and the redirects: the
  script refuses to download over anything else. It also needs `sha256sum` or
  `shasum -a 256`.
- It needs no sudo, npm or package manager and makes no profile edits. Add the
  install directory to `PATH` if the script says it is missing.
- Git is needed when reading repository history.

For CI, pin both the script URL and `--version` to the same release tag; the
[Actions example](render.md#example-github-actions) does this.

## What a release contains

Archives are available for x86_64 and aarch64 Linux (static musl), and for Intel
and Apple Silicon macOS (macOS 13 or later, system libraries only). No language
runtime is needed. The archive name is the same for every tag
(`sf-impact-<target>.tar.gz`).

Each archive includes all five commands, LICENSE, THIRD_PARTY_LICENSES and a
short README. A release provides a `.sha256` file beside every archive and an
aggregate `SHA256SUMS`.

## Verify where an archive came from

Checksums detect corruption, not a compromised publisher; they are not
signatures. To check where an archive came from, verify its GitHub attestation:

```sh
gh attestation verify <archive> --repo adamdaw/sf-impact-releases \
  --signer-workflow adamdaw/sf-impact-releases/.github/workflows/release.yml
```

Check the signer and workflow identity in the output. `SHA256SUMS` is attested
and is the authoritative checksum list; the per-archive `.sha256` files are
convenience copies and are not attested. See
[release maintenance](releases.md) for builds, license policy and checks.

## Build from source

Until a release is published, or to build for another platform, build with
Rust 1.100.0, the toolchain that releases are built with:

```sh
cargo build --release --locked
```

The five commands, `sf-impact-extract`, `sf-impact-delta`,
`sf-impact-changed`, `sf-impact-tests` and `sf-impact-render`, are written to
`target/release/`. Copy them to a directory on `PATH`. This build links the
system C library; the static musl build that releases use is described in
[release maintenance](releases.md#local-checks).
