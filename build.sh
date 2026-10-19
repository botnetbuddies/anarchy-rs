#!/usr/bin/env bash
# Build the static executable and wordlists using the Makefile.
set -euo pipefail

project_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
make -C "$project_root" dist
printf '%s\n' "$project_root/dist/anarchy-rs"
