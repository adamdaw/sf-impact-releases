# Contributing

Install Rust toolchain 1.100.0 with the `rustfmt` and `clippy` components (the
version CI uses) and run from the repository root:

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
python3 scripts/test-release.py
python3 scripts/test-release-tag.py
python3 scripts/test-license-gate.py
python3 scripts/test-license-notices.py
scripts/licenses.sh
git diff --exit-code THIRD_PARTY_LICENSES
```

`scripts/licenses.sh` requires cargo-about 0.9.3. Install the pinned,
checksum-verified build the same way CI does, then put it on `PATH`
(Linux x86_64 only; elsewhere install 0.9.3 yourself):

```sh
scripts/install-cargo-about.sh "$HOME/.local/cargo-about"
export PATH="$HOME/.local/cargo-about:$PATH"
```

The CI workflow also runs actionlint, ShellCheck and a static release packaging
dry-run; see `.github/workflows/ci.yml` for the complete pipeline. Fixtures, goldens and the reconciliation runs are described in
[docs/development.md](docs/development.md). Use concise,
imperative commit subjects. Open a pull request describing the change and its
tests.

Maintainers apply accepted changes through an export process, so your pull
request may be re-applied rather than merged directly. Do not include proprietary Salesforce metadata or secrets in issues
or pull requests.
