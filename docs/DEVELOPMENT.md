# Development guide

## Prerequisites

- [Node.js](https://nodejs.org) 18+
- [Rust](https://rustup.rs) (stable)
- Tauri platform dependencies — follow
  <https://tauri.app/start/prerequisites/> for your OS. In short:
  - **Windows**: Microsoft C++ Build Tools + WebView2 (preinstalled on Win 11).
  - **macOS**: Xcode Command Line Tools (`xcode-select --install`).
  - **Linux (Debian/Ubuntu)**:
    ```bash
    sudo apt update
    sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
      libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev \
      libasound2-dev espeak-ng
    ```
    `libxdo-dev` is needed by the keystroke simulation crate, `libasound2-dev`
    by audio playback, and `espeak-ng` powers the System voice.

## Run

```bash
npm install
npm run dev
```

This launches the app with hot-reloading of the Rust core and the static UI.

## Voxtray Dev on Windows, from WSL

`scripts/build-dev-windows.sh` cross-compiles **Voxtray Dev** with
[cargo-xwin](https://github.com/rust-cross/cargo-xwin) (it also needs `clang`
and `lld`) and copies it to `%LOCALAPPDATA%\VoxtrayDev\Voxtray Dev.exe`. Voxtray
Dev uses its own identifier (`src-tauri/tauri.dev.conf.json`), so its settings,
downloads, keychain entries and `voxtray.log` stay apart from an installed
Voxtray. On Windows the log is at `%APPDATA%\dev.harshal.voxtray.dev\voxtray.log`.

## Tests

```bash
cd src-tauri
cargo test
cargo clippy --all-targets
```

Tests that need a real Piper binary and voice are ignored by default:

```bash
VOXTRAY_PIPER_EXE=/path/to/piper VOXTRAY_PIPER_MODEL=/path/to/voice.onnx \
  cargo test -- --ignored piper
```

`render_passage_with_piper` (also ignored) reads a text file through the whole
pipeline, writes the audio to a WAV and checks every chunk was ready before
real-time playback needed it. Set `VOXTRAY_RENDER_TEXT` and `VOXTRAY_RENDER_WAV`
as well, and run it with `--release --nocapture`.

## Build installers

```bash
npm run build
```

Artifacts land in `src-tauri/target/release/bundle/` (`.msi`/`.exe` on Windows,
`.dmg`/`.app` on macOS, `.AppImage`/`.deb` on Linux).

## Project layout

```
voxtray/
├── src/                     # Settings UI (plain HTML/CSS/JS)
│   ├── index.html
│   ├── main.js              # invokes Tauri commands
│   └── styles.css
├── src-tauri/
│   ├── src/
│   │   ├── main.rs          # binary entry point
│   │   ├── lib.rs           # app setup, tray, hotkeys, plugins
│   │   ├── settings.rs      # config file + OS keychain
│   │   ├── reader.rs        # capture selection → speak; hotkey actions
│   │   ├── chunker.rs       # text → sentence-sized chunks
│   │   ├── reading.rs       # one reading: synthesis ahead, pause, skip, stop
│   │   ├── timeline.rs      # decoded PCM + play cursor (rodio source)
│   │   ├── player.rs        # rodio audio + local process control
│   │   ├── logging.rs       # voxtray.log (timings, never text)
│   │   ├── commands.rs      # #[tauri::command] handlers
│   │   └── tts/             # openai / elevenlabs / piper / local engines
│   ├── icons/               # generated from icon-source.png
│   ├── capabilities/        # Tauri permission grants
│   ├── tauri.conf.json
│   ├── tauri.dev.conf.json  # Voxtray Dev identity overlay
│   └── Cargo.toml
└── .github/workflows/release.yml
```

## How selection capture works

There is no reliable cross-platform "get the selected text" API. Instead the app:

1. Saves the current clipboard, then clears it (as a sentinel).
2. Synthesises the platform copy shortcut (`Ctrl/Cmd+C`) with the `enigo` crate,
   so the focused app copies its own selection.
3. Reads the clipboard back (`arboard`); empty means nothing was selected.
4. Optionally restores the original clipboard.

This works in virtually any app or website without per-app integrations.

## Regenerating icons

Edit `src-tauri/icons/generate_icon.py` (requires Python + Pillow), then:

```bash
python src-tauri/icons/generate_icon.py     # writes icon-source.png
npm run icon                                 # regenerates the platform set
```

## Releasing

Tag a version and push it; the GitHub Actions workflow builds and drafts a
release with installers for all three platforms:

```bash
git tag v0.1.0
git push origin v0.1.0
```

See `.github/workflows/release.yml`.

## Notes / known limitations

- Builds are **unsigned**. Users will see OS warnings; document code-signing
  before a public 1.0 if desired (Apple Developer ID + notarization, Windows
  Authenticode).
- Wayland restricts global shortcuts and input synthesis on some compositors.
- The local (System voice) engine quality depends on installed OS voices.
