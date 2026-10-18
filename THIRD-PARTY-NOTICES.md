# Third-party notices

Voxtray is MIT licensed. It is built on, or downloads at runtime, the following work. Each keeps its own licence.

## Bundled in the app

| Component | Licence |
| --- | --- |
| [Tauri](https://tauri.app) and its plugins (global-shortcut, autostart, notification, opener) | MIT or Apache-2.0 |
| [rodio](https://github.com/RustAudio/rodio) and symphonia (audio playback) | MIT or Apache-2.0 / MPL-2.0 |
| [arboard](https://github.com/1Password/arboard) (clipboard) | MIT or Apache-2.0 |
| [enigo](https://github.com/enigo-rs/enigo) (copy shortcut) | MIT |
| [keyring](https://github.com/hwchen/keyring-rs) (OS keychain) | MIT or Apache-2.0 |
| [reqwest](https://github.com/seanmonstar/reqwest), serde, zip, flate2, tar | MIT or Apache-2.0 |

The full dependency list with versions is in `src-tauri/Cargo.lock` and `package-lock.json`.

## Downloaded only if you choose that engine

These are not shipped with Voxtray. They are fetched from their official sources the first time you select them.

| Component | Source | Licence |
| --- | --- | --- |
| Piper text-to-speech engine | [rhasspy/piper](https://github.com/rhasspy/piper) | MIT |
| Piper voices (Amy, HFC male, Lessac, Alan, Jenny) | [rhasspy/piper-voices](https://huggingface.co/rhasspy/piper-voices) | Per voice: see each voice's `MODEL_CARD` |
| kokoro-js | [npm](https://www.npmjs.com/package/kokoro-js), via cdn.jsdelivr.net | Apache-2.0 |
| Kokoro-82M model | [onnx-community/Kokoro-82M-v1.0-ONNX](https://huggingface.co/onnx-community/Kokoro-82M-v1.0-ONNX) | Apache-2.0 |

Some Piper voices are trained on datasets with their own terms. Check a voice's model card before using its
audio commercially.
