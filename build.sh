#!/usr/bin/env bash
# Build the static username-anarchy-rs binary and the shipped bundle, then
# print the full path of the final executable.
#
# Outputs:
#   dist/username-anarchy      static-PIE, stripped, musl-linked binary
#   dist/names/                bundled name dataset directories
#
# Usage: ./build.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="x86_64-unknown-linux-musl"
RELEASE_BIN="$SCRIPT_DIR/target/$TARGET/release/username-anarchy-rs"
DIST_DIR="$SCRIPT_DIR/dist"
DIST_BIN="$DIST_DIR/username-anarchy"

cd "$SCRIPT_DIR"

# --- toolchain ---------------------------------------------------------------
if ! command -v cargo >/dev/null 2>&1; then
	echo "error: cargo not found - install Rust (https://rustup.rs)" >&2
	exit 1
fi

if [ -f "$RELEASE_BIN" ]; then
	stripped_ok=1
else
	stripped_ok=0
fi

if ! rustup target list --installed 2>/dev/null | grep -q "^$TARGET$"; then
	echo "installing Rust target $TARGET ..." >&2
	rustup target add "$TARGET"
fi

# --- build -------------------------------------------------------------------
# .cargo/config.toml pins the musl target, so `--release` builds the right one.
cargo build --release

# --- bundle ------------------------------------------------------------------
mkdir -p "$DIST_DIR"
strip --strip-all -o "$DIST_BIN" "$RELEASE_BIN"

if [ -d "$SCRIPT_DIR/username-anarchy/names" ]; then
	rm -rf "$DIST_DIR/names"
	cp -r "$SCRIPT_DIR/username-anarchy/names" "$DIST_DIR/names"
else
	echo "warning: no source dataset directory found; dist/names/ not refreshed" >&2
fi

# --- report ------------------------------------------------------------------
FULL_PATH="$(cd "$(dirname "$DIST_BIN")" && pwd)/$(basename "$DIST_BIN")"

echo "built: $FULL_PATH"
[ -d "$DIST_DIR/names" ] && echo "data:  $DIST_DIR/names"

# Sanity: must be fully static.
if readelf -l "$DIST_BIN" 2>/dev/null | grep -q INTERP; then
	echo "warning: binary unexpectedly links a dynamic loader" >&2
fi

echo "$FULL_PATH"
