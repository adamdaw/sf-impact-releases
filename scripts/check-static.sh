#!/bin/sh
set -eu
for binary in sf-impact-extract sf-impact-delta sf-impact-changed sf-impact-tests sf-impact-render; do
    path=${1:?usage: check-static.sh binary-directory}/$binary
    file "$path"
    headers=$(readelf -lW "$path")
    dynamic=$(readelf -dW "$path")
    if printf '%s\n%s\n' "$headers" "$dynamic" | grep -Eq 'INTERP|\(NEEDED\)'; then
        echo "$binary has a runtime dynamic linker or shared dependency" >&2
        exit 1
    fi
    "$path" --version
done
