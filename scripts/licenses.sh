#!/bin/sh
# Generate notices and refuse any license outside about.toml's allow-list.
set -eu
cd "$(dirname "$0")/.."
[ "$(cargo about --version)" = 'cargo-about 0.9.3' ] || {
    echo 'Requires cargo-about 0.9.3; see scripts/install-cargo-about.sh' >&2
    exit 1
}
output=${1:-THIRD_PARTY_LICENSES}
config=${2:-about.toml}
work=$(mktemp -d)
trap 'rm -rf "$work"' 0
trap 'exit 1' HUP INT TERM
# Fetch the locked tree first. Offline harvesting then avoids mutable remote
# license services. The missing sfapex copyright is supplied below, verbatim.
cargo fetch --locked
cargo about generate --workspace --locked --offline --fail \
    --config "$config" about.hbs --output-file "$work/notices"
cargo about generate --workspace --locked --offline --fail \
    --config "$config" --format json --output-file "$work/licenses.json"
python3 scripts/check-license-notices.py "$work/notices" "$work/licenses.json"
cp "$work/notices" "$output"
