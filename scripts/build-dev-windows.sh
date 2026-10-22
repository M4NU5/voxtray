#!/usr/bin/env bash
# Build "Voxtray Dev" and its Kokoro engine for Windows from WSL (or Linux)
# and, under WSL, copy both to %LOCALAPPDATA%\VoxtrayDev. Voxtray Dev has its
# own settings, downloads, log and keychain entries, so it runs alongside an
# installed Voxtray.
#
# Needs: cargo-xwin, clang and lld. Extra arguments go to `tauri build`.
set -euo pipefail
cd "$(dirname "$0")/.."

[ -d node_modules ] || npm ci
npx tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --no-bundle \
  --config src-tauri/tauri.dev.conf.json "$@"
cargo xwin build --release -p voxtray-engine --target x86_64-pc-windows-msvc \
  --manifest-path src-tauri/Cargo.toml

exe=src-tauri/target/x86_64-pc-windows-msvc/release/voxtray.exe
engine=src-tauri/target/x86_64-pc-windows-msvc/release/voxtray-engine.exe
if command -v cmd.exe >/dev/null 2>&1; then
  dest="$(wslpath "$(cmd.exe /c 'echo %LOCALAPPDATA%' 2>/dev/null | tr -d '\r')")/VoxtrayDev"
  # A running copy locks the file.
  taskkill.exe /IM "Voxtray Dev.exe" /F >/dev/null 2>&1 || true
  mkdir -p "$dest"
  cp "$exe" "$dest/Voxtray Dev.exe"
  # An engine still finishing a chunk can hold its exe for a moment.
  for _ in 1 2 3 4 5; do cp "$engine" "$dest/voxtray-engine.exe" && break; sleep 1; done
  cmp -s "$engine" "$dest/voxtray-engine.exe" || { echo "Could not copy the engine" >&2; exit 1; }
  echo "Built $dest/Voxtray Dev.exe and voxtray-engine.exe"
else
  echo "Built $exe and $engine"
fi
