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

## The Kokoro engine

Kokoro runs in `voxtray-engine`, a separate program. Its crate is
`src-tauri/engine`, a member of the Cargo workspace. It links the
[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) static libraries, which
the `sherpa-onnx` crate downloads on its first build. Voxtray starts one per
reading and talks to it over stdin and stdout, using the JSON-lines protocol in
`src-tauri/engine/src/protocol.rs`.

It is separate because sherpa-onnx includes espeak-ng, which is
GPL-3.0-or-later. The app only depends on the crate's protocol and client, with
the `kokoro` feature off, so none of that code ends up in Voxtray itself.

`npm run dev` doesn't build it. Build it once:

```bash
cd src-tauri
cargo build -p voxtray-engine
```

It lands in `target/debug` next to the dev app, which is where Voxtray looks
for it. `VOXTRAY_ENGINE_EXE` overrides the location.

Installers bundle it as a Tauri sidecar. `scripts/build-engine.sh` builds it
into `src-tauri/binaries/`, and `--config src-tauri/tauri.engine.conf.json`
adds it to the bundle. That setting lives in its own file because tauri-build
refuses to build when a sidecar is missing, and a plain `cargo build` shouldn't
need the engine.

## Voxtray Dev on Windows, from WSL

`scripts/build-dev-windows.sh` cross-compiles **Voxtray Dev** and the Kokoro
engine with [cargo-xwin](https://github.com/rust-cross/cargo-xwin) (it also
needs `clang` and `lld`) and copies them to `%LOCALAPPDATA%\VoxtrayDev`. Voxtray
Dev uses its own identifier (`src-tauri/tauri.dev.conf.json`), so its settings,
downloads, keychain entries and `voxtray.log` stay apart from an installed
Voxtray. On Windows the log is at `%APPDATA%\dev.harshal.voxtray.dev\voxtray.log`.

## Tests

```bash
cd src-tauri
cargo test
cargo clippy --all-targets
cargo test -p voxtray-engine     # the protocol, and the client against a fake engine
cargo clippy -p voxtray-engine --all-targets
```

Tests that need a real Piper binary and voice are ignored by default:

```bash
VOXTRAY_PIPER_EXE=/path/to/piper VOXTRAY_PIPER_MODEL=/path/to/voice.onnx \
  cargo test -- --ignored piper
```

So are the ones that need the Kokoro model. Unpack
[kokoro-multi-lang-v1_0.tar.bz2](https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models)
somewhere, then:

```bash
export VOXTRAY_KOKORO_MODEL=/path/to/kokoro-multi-lang-v1_0
cargo test -p voxtray-engine -- --ignored
VOXTRAY_ENGINE_EXE=$PWD/target/debug/voxtray-engine cargo test -- --ignored kokoro
```

`VOXTRAY_KOKORO_ARCHIVE=/path/to/kokoro-multi-lang-v1_0.tar.bz2` also runs the
test that unpacks the archive.

`render_passage_with_piper` and `render_passage_with_kokoro` are ignored too.
They read a text file through the whole pipeline, write the audio to a WAV and
check every chunk was ready before real-time playback needed it. Set
`VOXTRAY_RENDER_TEXT` and `VOXTRAY_RENDER_WAV` as well, and run them with
`--release --nocapture`. `VOXTRAY_RENDER_SPEED` sets Kokoro's speed.

## Build installers

```bash
scripts/build-engine.sh
npm run build -- --config src-tauri/tauri.engine.conf.json
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
│   │   └── tts/             # openai / elevenlabs / piper / kokoro / local engines
│   ├── engine/              # voxtray-engine: Kokoro in its own process
│   │   ├── src/protocol.rs  # the stdio protocol (JSON lines)
│   │   ├── src/client.rs    # the app's side of it
│   │   ├── src/main.rs      # the engine binary
│   │   └── src/kokoro.rs    # Kokoro via sherpa-onnx
│   ├── icons/               # generated from icon-source.png
│   ├── capabilities/        # Tauri permission grants
│   ├── tauri.conf.json
│   ├── tauri.dev.conf.json  # Voxtray Dev identity overlay
│   ├── tauri.engine.conf.json  # bundles voxtray-engine (release builds)
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
