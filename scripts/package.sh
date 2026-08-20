#!/usr/bin/env bash
#
# Build and package Aidoku sources out of the shared Cargo workspace.
#
# Kept as a thin wrapper so existing docs and CI keep working; the packaging
# itself now lives in scripts/aidoku.py, alongside the rest of the
# workspace-aware CLI commands.
#
# Usage:
#   scripts/package.sh                       # every source
#   scripts/package.sh sources/en.tcbscans   # only the ones listed

set -euo pipefail

exec "$(dirname "$0")/aidoku.py" package "$@"
