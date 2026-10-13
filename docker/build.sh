#!/usr/bin/env bash
# Build the OpenDeck .deb in a container, so the host needs no Rust, Deno or
# webkit development packages.
#
#   JOBS=3 MEM=6g docker/build.sh
#
# The container gets a hard memory cap and no swap: if the link step is too
# greedy it dies inside the container instead of taking the desktop with it.
set -euo pipefail

REPO=$(cd "$(dirname "$0")/.." && pwd)
CACHE="${XDG_CACHE_HOME:-$HOME/.cache}/opendeck-build"
JOBS="${JOBS:-3}"
MEM="${MEM:-6g}"

mkdir -p "$CACHE/cargo" "$CACHE/deno" "$CACHE/home"

docker build -t opendeck-build "$REPO/docker"

docker run --rm \
	--user "$(id -u):$(id -g)" \
	--memory="$MEM" --memory-swap="$MEM" \
	-e CARGO_HOME=/cache/cargo \
	-e DENO_DIR=/cache/deno \
	-e HOME=/cache/home \
	-e CARGO_BUILD_JOBS="$JOBS" \
	-v "$REPO":/src \
	-v "$CACHE":/cache \
	-w /src \
	opendeck-build \
	bash -lc 'set -e; deno install; deno task tauri build --bundles deb'

echo
echo "artefacts:"
find "$REPO/src-tauri/target/release/bundle/deb" -name '*.deb' -printf '  %p (%s bytes)\n' 2>/dev/null || echo "  none found"
