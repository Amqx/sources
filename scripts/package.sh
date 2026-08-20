#!/usr/bin/env bash
#
# Build and package Aidoku sources out of the shared Cargo workspace.
#
# `aidoku package` copies the *first* .wasm file it finds in the build
# directory. That is unambiguous when every source has its own target dir, but
# in a workspace all sources build into one shared target dir, so the CLI would
# happily package the wrong binary. The CLI checks the current directory before
# walking up to the workspace root, so we stage each source's own wasm under
# <source>/target/wasm32-unknown-unknown/release and let it find that instead.
#
# Usage:
#   scripts/package.sh                       # every source
#   scripts/package.sh sources/en.tcbscans   # only the ones listed

set -euo pipefail

cd "$(dirname "$0")/.."
root=$PWD
build_dir="${CARGO_TARGET_DIR:-$root/target}/wasm32-unknown-unknown/release"

if [ $# -gt 0 ]; then
	dirs=("$@")
else
	dirs=(sources/*)
fi

pkg_name() {
	sed -n 's/^name = "\(.*\)"/\1/p' "$1/Cargo.toml" | head -1
}

staged=""
cleanup() {
	if [ -n "$staged" ]; then
		rm -rf "$staged"
		staged=""
	fi
}
trap cleanup EXIT

for dir in "${dirs[@]}"; do
	dir=${dir%/}
	name=$(pkg_name "$dir")

	# One package at a time: building several with `-p a -p b` would unify their
	# feature sets and link features into a source that never asked for them.
	# The workspace target dir still shares every dependency build between them.
	cargo build --release --target wasm32-unknown-unknown -p "$name"

	# Nothing writes to <source>/target any more — cargo sends every member's
	# output to the workspace target dir — so anything here is either ours from
	# an interrupted run or a leftover from the pre-workspace layout. Either way
	# it has to go, otherwise the CLI may find a stale wasm before ours.
	if [ -e "$dir/target" ]; then
		echo "removing stale $dir/target"
		rm -rf "$dir/target"
	fi

	staged="$dir/target"
	mkdir -p "$staged/wasm32-unknown-unknown/release"
	cp "$build_dir/$name.wasm" "$staged/wasm32-unknown-unknown/release/$name.wasm"

	(cd "$dir" && aidoku package)

	cleanup
	echo "packaged $dir"
done
