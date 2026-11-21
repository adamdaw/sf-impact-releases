sf-impact installs the `sf-impact-*` commands.

This archive contains sf-impact-extract, sf-impact-delta, sf-impact-changed, sf-impact-tests and
sf-impact-render from one release. Linux binaries use static musl; macOS binaries
use only system libraries. No language runtime or package manager is needed.
Git is needed to read repository history.

Use install.sh from the same release tag to verify the archive's SHA-256
checksum and install into ~/.local/bin, or verify the .sha256 file before
copying the five commands to a directory on PATH yourself.

Usage and schemas: https://github.com/adamdaw/sf-impact-releases
Run each command with --help or --version.

MIT: see LICENSE. Dependency notices: see THIRD_PARTY_LICENSES.

## Verify provenance

```sh
gh attestation verify <archive> --repo adamdaw/sf-impact-releases \
  --signer-workflow adamdaw/sf-impact-releases/.github/workflows/release.yml
```

Check the signer and workflow identity in the output. SHA256SUMS is attested and is the
authoritative checksum list; the individual `*.sha256` files are not attested.
