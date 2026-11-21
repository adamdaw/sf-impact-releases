# Security Policy

## Supported versions

Only the latest release of sf-impact is supported. Fixes are made on the
default branch and released as a new version; older releases are not patched.

## Reporting a vulnerability

**Do not open a public issue for a security vulnerability.**

Report it privately with
[GitHub private vulnerability reporting](https://github.com/adamdaw/sf-impact-releases/security/advisories/new)
on `adamdaw/sf-impact-releases`.

These are targets, not guarantees: we aim to acknowledge a report within five
business days and to give an initial assessment within two weeks. This is
maintained by a small team, so a fix may take longer; we will tell you where it
stands.

### What to include

- The affected version (`--version` output) and platform.
- A description of the problem and its impact.
- Steps to reproduce, ideally a minimal input repository or file.
- Any suggested fix or mitigation.

Please do not include proprietary Salesforce metadata or secrets. A minimal
synthetic reproduction is preferred.

## Scope

In scope:

- The five binaries: `sf-impact-extract`, `sf-impact-delta`, `sf-impact-changed`,
  `sf-impact-tests` and `sf-impact-render`.
- `install.sh`.
- The release workflow and the release artifacts (archives, checksums and
  attestations).
- Malformed repository metadata or source (Apex, XML metadata, Flows, LWC) that
  causes a crash, a hang or unbounded CPU or memory use.

## Out of scope

- Reports that depend on an attacker controlling a restored cache or previous
  refs pack. A previous pack is trusted input, as the README states: restore
  packs only from trusted workflows and branches.
- Findings in the contents of a repository being analysed, rather than in
  sf-impact itself.
- Vulnerabilities in third-party dependencies that do not affect sf-impact;
  report those upstream.

## Disclosure

We follow coordinated disclosure. We will agree a disclosure date with the
reporter, normally after a fixed release is available, and credit the reporter
in the advisory unless they prefer not to be named.
