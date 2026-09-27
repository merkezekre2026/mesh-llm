#!/usr/bin/env bash
# Stage a built mesh-llm host and its native runtime(s) as the Tauri sidecar
# and resources of the desktop app (desktop/src-tauri/binaries/).
#
# The host is copied unchanged: this script only relocates an already built
# product for Tauri's bundler and never places backend libraries beside the
# host, so the host dependency policy enforced at packaging time still holds.
#
# usage: stage-desktop-sidecar.sh (--binary <mesh-llm> | --placeholder) [--runtimes <dir>] [--target <triple>]
#   --binary      path to a built mesh-llm host (target/debug or target/release)
#   --placeholder stage a stub that only reports it is not a real build, so the
#                 app can be linted and unit-tested without building mesh-llm
#   --runtimes    a native-runtimes directory; each runtime subdirectory is
#                 copied (archives and checksums beside them are skipped)
#   --target      Rust target triple of the binary (default: rustc host triple)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STAGE_DIR="$REPO_ROOT/desktop/src-tauri/binaries"

binary=""
placeholder=0
runtimes=""
target=""

usage() {
    sed -n '9,16p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --binary) binary="${2:?}"; shift 2 ;;
        --placeholder) placeholder=1; shift ;;
        --runtimes) runtimes="${2:?}"; shift 2 ;;
        --target) target="${2:?}"; shift 2 ;;
        -h|--help) usage ;;
        *) echo "unknown argument: $1" >&2; usage ;;
    esac
done

if [[ "$placeholder" -eq 1 ]]; then
    [[ -z "$binary" ]] || usage
    binary="$(mktemp)"
    trap 'rm -f "$binary"' EXIT
    printf '#!/bin/sh\necho "placeholder sidecar: stage a real mesh-llm build" >&2\nexit 1\n' > "$binary"
fi
[[ -n "$binary" ]] || usage
if [[ ! -f "$binary" ]]; then
    echo "mesh-llm binary not found: $binary (build it with 'just build' or 'just release-build')" >&2
    exit 1
fi
if [[ -z "$target" ]]; then
    target="$(rustc -vV | sed -n 's/^host: //p')"
fi

ext=""
case "$target" in
    *windows*) ext=".exe" ;;
esac

mkdir -p "$STAGE_DIR"
sidecar="$STAGE_DIR/mesh-llm-sidecar-$target$ext"
cp "$binary" "$sidecar"
chmod +x "$sidecar"
echo "staged sidecar: ${sidecar#"$REPO_ROOT"/}"

runtime_stage="$STAGE_DIR/native-runtimes"
rm -rf "$runtime_stage"
mkdir -p "$runtime_stage"
# Tauri requires the resource directory to exist even when it is empty.
touch "$runtime_stage/.keep"

if [[ -z "$runtimes" ]]; then
    echo "no --runtimes given: the app will use mesh-llm's own runtime discovery"
    exit 0
fi
if [[ ! -d "$runtimes" ]]; then
    echo "native runtime directory not found: $runtimes" >&2
    exit 1
fi

copied=0
for runtime in "$runtimes"/*/; do
    [[ -d "$runtime" ]] || continue
    cp -R "${runtime%/}" "$runtime_stage/"
    echo "staged runtime: $(basename "$runtime")"
    copied=$((copied + 1))
done
if [[ "$copied" -eq 0 ]]; then
    echo "no runtime directories found in $runtimes" >&2
    exit 1
fi
