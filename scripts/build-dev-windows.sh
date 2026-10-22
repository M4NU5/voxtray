#!/usr/bin/env bash
# Build "Voxtray Dev" for Windows from WSL (or Linux) and, under WSL, copy it
# to %LOCALAPPDATA%\VoxtrayDev. Voxtray Dev has its own settings, downloads,
# log and keychain entries, so it runs alongside an installed Voxtray.
#
# Needs: cargo-xwin, clang and lld. Extra arguments go to `tauri build`.
set -euo pipefail
cd "$(dirname "$0")/.."

[ -d node_modules ] || npm ci
npx tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --no-bundle \
  --config src-tauri/tauri.dev.conf.json "$@"

exe=src-tauri/target/x86_64-pc-windows-msvc/release/voxtray.exe
if command -v cmd.exe >/dev/null 2>&1; then
  dest="$(wslpath "$(cmd.exe /c 'echo %LOCALAPPDATA%' 2>/dev/null | tr -d '\r')")/VoxtrayDev"
  # A running copy locks the file.
  taskkill.exe /IM "Voxtray Dev.exe" /F >/dev/null 2>&1 || true
  mkdir -p "$dest"
  cp "$exe" "$dest/Voxtray Dev.exe"
  echo "Built $dest/Voxtray Dev.exe"
else
  echo "Built $exe"
fi
