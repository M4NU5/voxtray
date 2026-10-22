#!/usr/bin/env bash
# Build voxtray-engine (the Kokoro engine) in release mode and put it where
# Tauri's externalBin looks for it: src-tauri/binaries/voxtray-engine-<target>.
# Then bundle with: tauri build --config src-tauri/tauri.engine.conf.json
#
# Usage: scripts/build-engine.sh [target]   (default: this machine's target)
#        scripts/build-engine.sh universal-apple-darwin
set -euo pipefail
cd "$(dirname "$0")/../src-tauri"

target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
ext=""
case "$target" in *windows*) ext=".exe" ;; esac
mkdir -p binaries

if [ "$target" = universal-apple-darwin ]; then
  for t in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo build --release -p voxtray-engine --target "$t"
  done
  lipo -create -output binaries/voxtray-engine-universal-apple-darwin \
    target/aarch64-apple-darwin/release/voxtray-engine \
    target/x86_64-apple-darwin/release/voxtray-engine
else
  cargo build --release -p voxtray-engine --target "$target"
  cp "target/$target/release/voxtray-engine$ext" "binaries/voxtray-engine-$target$ext"
fi
echo "Built src-tauri/binaries/voxtray-engine-$target$ext"
