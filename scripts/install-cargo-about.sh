#!/bin/sh
# Pinned upstream Linux x86_64 tool, used only by the license CI job.
set -eu
prefix=${1:?usage: install-cargo-about.sh directory}
mkdir -p "$prefix"
work=$(mktemp -d)
trap 'rm -rf "$work"' 0
trap 'exit 1' HUP INT TERM
curl -fsSL --proto '=https' --proto-redir '=https' --retry 3 \
    https://github.com/EmbarkStudios/cargo-about/releases/download/0.9.3/cargo-about-0.9.3-x86_64-unknown-linux-musl.tar.gz \
    -o "$work/cargo-about.tar.gz"
(cd "$work" && echo '3ad1e94ea1e63d69d1961f4fb15ccbbe9575d906111f96d2cb4dce6df48f71b4  cargo-about.tar.gz' | sha256sum -c -)
tar -xzf "$work/cargo-about.tar.gz" -C "$prefix" --strip-components=1 \
    cargo-about-0.9.3-x86_64-unknown-linux-musl/cargo-about
