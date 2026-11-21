#!/bin/sh
# Install one sf-impact release. No privilege escalation or shell-profile edits.
# Parse the complete installer before any piped input can have side effects.
main() {
    set -eu

    fail() { printf 'sf-impact: %s\n' "$*" >&2; exit 1; }
    # One wall-clock deadline per file, even if a peer keeps sending partial data.
    download_timeout=${SF_IMPACT_DOWNLOAD_TIMEOUT:-60}
    case "$download_timeout" in ''|0*|*[!0-9]*) fail 'Invalid SF_IMPACT_DOWNLOAD_TIMEOUT';; esac
    bounded_download() (
        # Confine any downloader caches and lock files to our temporary directory.
        XDG_DATA_HOME="$work" TMPDIR="$work" "$@" &
        transfer=$!
        (
            sleep "$download_timeout" &
            sleeper=$!
            trap 'kill "$sleeper" 2>/dev/null || :; exit 0' HUP INT TERM
            wait "$sleeper"
            kill -KILL "$transfer" 2>/dev/null || :
        ) </dev/null >/dev/null 2>&1 &
        timer=$!
        trap 'kill "$transfer" "$timer" 2>/dev/null || :; wait "$timer" 2>/dev/null || :' 0
        trap 'exit 1' HUP INT TERM
        result=0
        wait "$transfer" || result=$?
        exit "$result"
    )
    # Strict MAJOR.MINOR.PATCH: three non-empty digit parts, no leading zeros
    # (same rule as scripts/check-release-tag.py). Applies to every version we
    # splice into a download URL, whether typed by the user or resolved remotely.
    semver() (
        case "$1" in ''|*[!0-9.]*|.*|*.|*..*) return 1;; esac
        IFS=.
        # shellcheck disable=SC2086 # deliberate split on dots; input is digits and dots only
        set -- $1
        [ "$#" -eq 3 ] || return 1
        for part do
            case "$part" in 0|[1-9]*) ;; *) return 1;; esac
        done
    )
    non_https='Non-HTTPS downloads require SF_IMPACT_INSECURE_TEST_BASE=1 (tests only)'
    version=latest
    prefix=${HOME:?HOME must be set}/.local/bin
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --version|--prefix)
                if [ "$#" -lt 2 ] || [ -z "$2" ]; then
                    fail "$1 needs a value"
                fi
                case "$1" in --version) version=$2;; --prefix) prefix=$2;; esac
                shift 2 ;;
            --help|-h)
                echo 'Usage: install.sh [--version v0.1.0|latest] [--prefix directory]'
                exit 0 ;;
            *) fail "Unknown argument: $1" ;;
        esac
    done
    # Make relative prefixes unambiguous to POSIX utilities, including leading dashes.
    case "$prefix" in /*) ;; *) prefix=$PWD/$prefix;; esac
    case "$version" in
        latest) release=latest/download ;;
        *)
            version=${version#v}
            semver "$version" || fail 'Invalid version'
            release=download/v$version ;;
    esac
    case "$(uname -s)" in
        Linux) os=unknown-linux-musl ;;
        Darwin) os=apple-darwin ;;
        *) fail 'Unsupported operating system (requires Linux or macOS)' ;;
    esac
    case "$(uname -m)" in
        x86_64|amd64) arch=x86_64 ;;
        aarch64|arm64) arch=aarch64 ;;
        *) fail 'Unsupported architecture (requires x86_64 or aarch64)' ;;
    esac
    work=$(mktemp -d "${TMPDIR:-/tmp}/sf-impact.XXXXXXXX")
    trap 'rm -rf "$work"' 0
    trap 'exit 1' HUP INT TERM
    base=${SF_IMPACT_RELEASE_BASE_URL:-https://github.com/adamdaw/sf-impact-releases/releases}
    insecure_test=${SF_IMPACT_INSECURE_TEST_BASE:-0}
    case "$base" in
        https://*) insecure_test=0 ;;
        http://*|file://*) [ "$insecure_test" = 1 ] || fail "$non_https" ;;
        *) fail "Unsupported release base URL" ;;
    esac
    if command -v curl >/dev/null 2>&1; then
        printf '%s\n' 'sf-impact: downloader: curl' >&2 || :
        download() {
            if [ "$insecure_test" = 1 ]; then
                set -- --proto "=https,http,file" --proto-redir "=https,http,file" "$@"
            else
                set -- --proto "=https" --proto-redir "=https" "$@"
            fi
            bounded_download curl -fsSL "$1" "$2" "$3" "$4" --connect-timeout 10 --max-time 60 \
                --retry 2 --retry-max-time 60 --max-redirs 5 -o "$6" "$5" && [ -s "$6" ]
        }
    elif command -v wget >/dev/null 2>&1; then
        [ "$insecure_test" = 1 ] || fail "$non_https; wget cannot enforce HTTPS-only redirects, so install curl"
        # BusyBox lacks GNU's options. Probe before enabling them; the portable
        # fallback still has the same wall-clock deadline and checksum gate.
        wget_version=$(XDG_DATA_HOME="$work" TMPDIR="$work" wget --no-config --no-hsts \
            --tries=2 --timeout=10 --waitretry=1 --max-redirect=5 --version 2>/dev/null) || wget_version=
        case "$wget_version" in
            *Wget2*) downloader_mode=Wget2 ;;
            '') downloader_mode='basic wget (BusyBox or unknown, without GNU options)' ;;
            *) downloader_mode='GNU Wget' ;;
        esac
        printf 'sf-impact: downloader: %s\n' "$downloader_mode" >&2 || :
        download() {
            url=$1
            destination=$2
            set --
            if [ -n "$wget_version" ]; then
                set -- --no-config --no-hsts --tries=2 --timeout=10 --waitretry=1 --max-redirect=5
            fi
            case "$wget_version" in
                *Wget2*) set -- "$@" --no-hpkp --no-ocsp --no-tls-resume ;;
            esac
            bounded_download wget "$@" -q -O "$destination" "$url" && [ -s "$destination" ]
        }
    else
        fail 'Install curl or wget first'
    fi
    # Under SF_IMPACT_INSECURE_TEST_BASE=1 (tests only) latest is NOT resolved
    # once: each file is fetched from $base/latest/download, so a mirror can
    # change between the two requests. Only the HTTPS path is pinned.
    if [ "$version" = latest ] && [ "$insecure_test" != 1 ]; then
        bounded_download curl -fsSL --proto '=https' --proto-redir '=https' \
            --connect-timeout 10 --max-time 60 --max-redirs 5 \
            -o "$work/latest-body" -w '%{url_effective}' "$base/latest" > "$work/latest-url" \
            || fail 'Latest release resolution failed'
        resolved=$(cat "$work/latest-url")
        case "$resolved" in
            "$base"/tag/v*) version=${resolved#"$base"/tag/v} ;;
            *) fail 'Invalid latest release URL' ;;
        esac
        semver "$version" || fail 'Invalid latest release tag'
        release=download/v$version
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        digest() { sha256sum "$1"; }
    elif command -v shasum >/dev/null 2>&1; then
        digest() { shasum -a 256 "$1"; }
    else
        fail 'Install sha256sum or shasum first'
    fi
    archive=sf-impact-$arch-$os.tar.gz
    # Override for mirrors and offline release tests; defaults to GitHub HTTPS.
    download "$base/$release/$archive" "$work/$archive" || fail 'Archive download failed'
    download "$base/$release/$archive.sha256" "$work/checksum" || fail 'Checksum download failed'
    read -r expected filename < "$work/checksum" || fail 'Invalid checksum file'
    [ "$filename" = "$archive" ] || fail 'Checksum names a different archive'
    [ "${#expected}" -eq 64 ] || fail 'Invalid checksum length'
    case "$expected" in *[!0-9a-f]*) fail 'Invalid SHA-256 checksum';; esac
    actual=$(digest "$work/$archive")
    actual=${actual%% *}
    [ "$actual" = "$expected" ] || fail 'SHA-256 checksum mismatch; nothing installed'
    # Explicit members: no documentation or other archive content goes into the prefix.
    set -- sf-impact-extract sf-impact-delta sf-impact-changed sf-impact-tests sf-impact-render
    tar -xzf "$work/$archive" -C "$work" "$@" || fail 'Cannot unpack release'
    for binary do
        if [ ! -f "$work/$binary" ] || [ -L "$work/$binary" ]; then
            fail "Missing regular binary: $binary"
        fi
    done
    mkdir -p "$prefix"
    for binary do
        # Remove a previous symlink rather than follow it outside the requested prefix.
        rm -f "$prefix/$binary"
        cp "$work/$binary" "$prefix/$binary"
        chmod 755 "$prefix/$binary"
    done
    printf 'Installed sf-impact (%s) into %s\n' "$version" "$prefix"
    case ":${PATH:-}:" in
        *":$prefix:"*) ;;
        *) printf 'Add %s to your PATH to run the sf-impact-* commands.\n' "$prefix" ;;
    esac
}

# Parse the argument forwarding too before a truncated pipe can invoke main.
{
    main "$@"
}
