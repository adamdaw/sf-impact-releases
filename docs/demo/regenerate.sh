#!/bin/sh
# Re-record docs/demo/demo.cast with asciinema and render docs/demo/demo.svg.
#
#   docs/demo/regenerate.sh [BIN_DIR]    record, then render (BIN_DIR holds the
#                                        five sf-impact-* commands; default
#                                        target/x86_64-unknown-linux-musl/release)
#   docs/demo/regenerate.sh --render     render demo.svg from the committed
#                                        demo.cast; builds and records nothing
#
# Needs podman (or set CONTAINER=docker) and network access. The recorder and
# the renderer run in containers whose base images are pinned by digest; their
# packages are pinned by version and checksum (requirements.txt for asciinema,
# package-lock.json for svg-term-cli).
set -eu

PYTHON_IMAGE=docker.io/library/python@sha256:9ef1a599f863231c3d91e941389bfdc4a2d6070557243d54df1a4e57015b960e
NODE_IMAGE=docker.io/library/node@sha256:12f54cafa58d0e96554ae05871e29663ff7241836377dc9a7916449b8adb062c
COLS=112
ROWS=30
engine=${CONTAINER:-podman}

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)

if [ "${1:-}" != --render ]; then
    bin=${1:-$root/target/x86_64-unknown-linux-musl/release}
    for tool in extract delta changed tests render; do
        [ -x "$bin/sf-impact-$tool" ] || { echo "regenerate.sh: no $bin/sf-impact-$tool" >&2; exit 1; }
    done
    bin=$(cd "$bin" && pwd)
    "$engine" run --rm \
        -v "$here:/demo:ro,Z" -v "$root/fixtures/project:/fixtures/project:ro,Z" \
        -v "$bin:/opt/sf-impact:ro,Z" -v "$here:/out:Z" \
        -e TERM=xterm-256color -e PYTHONPATH=/tmp/py -e PATH=/opt/sf-impact:/tmp/py/bin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
        -w /tmp -t "$PYTHON_IMAGE" sh -c '
            apk add --no-cache bash git jq >/dev/null
            pip install --quiet --no-cache-dir --require-hashes --target /tmp/py -r /demo/requirements.txt
            python3 -m asciinema rec --quiet --overwrite --cols '"$COLS"' --rows '"$ROWS"' \
                --idle-time-limit 2 --command "bash /demo/demo.sh" /out/demo.cast'
    # The header holds the wall-clock time of the recording. Drop it, so that
    # recording again does not change the header.
    sed -i '1s/"timestamp": [0-9]*, //' "$here/demo.cast"
fi

"$engine" run --rm -v "$here:/demo:Z" -e HOME=/tmp -w /tmp "$NODE_IMAGE" sh -c '
    mkdir /tmp/render && cp /demo/package.json /demo/package-lock.json /tmp/render/ && cd /tmp/render
    npm ci --ignore-scripts --no-audit --no-fund --loglevel=error
    ./node_modules/.bin/svg-term --in /demo/demo.cast --out /demo/demo.svg \
        --width '"$COLS"' --height '"$ROWS"' --padding 12 --no-cursor
    # svg-term gives its shared elements all-digit ids; prefix them with f.
    sed -i -E "s/(id=\"|href=\"#)([0-9]+)\"/\\1f\\2\"/g" /demo/demo.svg'
