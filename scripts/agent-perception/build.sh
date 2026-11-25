#!/usr/bin/env bash
# Build the read-only desktop-window probe with the host's system tooling.
# Generated Wayland bindings live only in a temporary directory; the only
# durable output is the probe binary (default: scripts/agent-perception/probe).
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SRC_DIR="$ROOT/scripts/agent-perception"
OUT=${1:-"$SRC_DIR/probe"}
SCANNER=${WAYLAND_SCANNER:-wayland-scanner}
CC_BIN=${CC:-cc}
PKG_CONFIG=${PKG_CONFIG:-pkg-config}

for tool in "$SCANNER" "$CC_BIN" "$PKG_CONFIG"; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "missing tool: $(basename "$tool")" >&2
    exit 2
  fi
done
if ! "$PKG_CONFIG" --exists wayland-client; then
  echo "missing pkg-config package: wayland-client" >&2
  exit 2
fi

mkdir -p "${TMPDIR:-/tmp/opencode}"
tmp=$(mktemp -d "${TMPDIR:-/tmp/opencode}/perception-build.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

for xml in "$SRC_DIR"/protocols/*.xml; do
  name=$(basename "$xml" .xml)
  "$SCANNER" client-header "$xml" "$tmp/$name.h"
  "$SCANNER" private-code "$xml" "$tmp/$name.c"
done

cflags=$("$PKG_CONFIG" --cflags wayland-client)
libs=$("$PKG_CONFIG" --libs wayland-client)
# shellcheck disable=SC2086
"$CC_BIN" -std=c11 -O2 -Wall -Wextra -I"$tmp" $cflags \
  "$SRC_DIR/probe.c" "$tmp"/*.c -o "$OUT" $libs

echo "PROBE_BUILT"
