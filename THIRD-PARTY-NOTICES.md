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
| [bzip2](https://github.com/trifectatechfoundation/bzip2-rs) and libbz2-rs-sys (unpacking the Kokoro model) | MIT or Apache-2.0 / bzip2-1.0.6 |

The full dependency list with versions is in `src-tauri/Cargo.lock` and `package-lock.json`.

## The Kokoro engine, installed with the app

Voxtray installs a second program next to itself, `voxtray-engine`, which runs the Kokoro voice. They are separate
programs that talk over standard input and output. The engine's own code, in `src-tauri/engine`, is MIT. It
statically links espeak-ng, though, so **`voxtray-engine` as shipped falls under the
[GNU GPL, version 3 or later](https://www.gnu.org/licenses/gpl-3.0.html)**. Voxtray itself contains none of the code
below and stays MIT.

Its corresponding source is this repository plus [sherpa-onnx v1.13.8](https://github.com/k2-fsa/sherpa-onnx/tree/v1.13.8),
whose `cmake/` files pin each upstream version listed here.

| Component | Source | Licence |
| --- | --- | --- |
| [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx) 1.13.8, and its Rust crate | k2-fsa/sherpa-onnx v1.13.8 | Apache-2.0 |
| espeak-ng, including ucd-tools | [csukuangfj/espeak-ng @ ed530aa](https://github.com/csukuangfj/espeak-ng/tree/ed530aa113046142eb5115cf2fc9157854d0ffe1) | GPL-3.0-or-later |
| piper-phonemize | [csukuangfj/piper-phonemize @ f3ff95a](https://github.com/csukuangfj/piper-phonemize/tree/f3ff95afc03640bc1399e113e83361192a2fafb4) | MIT |
| ONNX Runtime 1.28.2 | [microsoft/onnxruntime](https://github.com/microsoft/onnxruntime), prebuilt by csukuangfj/onnxruntime-libs | MIT |
| OpenFst | [csukuangfj/openfst v1.8.5-2026-07-09](https://github.com/csukuangfj/openfst/tree/v1.8.5-2026-07-09) | Apache-2.0 |
| kaldi-decoder 0.3.0, kaldifst 1.8.0, kaldi-native-fbank 1.22.3 | [k2-fsa/kaldi-decoder](https://github.com/k2-fsa/kaldi-decoder), [k2-fsa/kaldifst](https://github.com/k2-fsa/kaldifst), [csukuangfj/kaldi-native-fbank](https://github.com/csukuangfj/kaldi-native-fbank) | Apache-2.0 |
| kissfft | [mborgerding/kissfft @ febd4ca](https://github.com/mborgerding/kissfft/tree/febd4caeed32e33ad8b2e0bb5ea77542c40f18ec) | BSD-3-Clause |
| simple-sentencepiece 0.7 | [pkufool/simple-sentencepiece](https://github.com/pkufool/simple-sentencepiece/tree/v0.7) | Apache-2.0 |

## Downloaded only if you choose that engine

These are not shipped with Voxtray. They are fetched from their official sources the first time you select them.

| Component | Source | Licence |
| --- | --- | --- |
| Piper text-to-speech engine | [rhasspy/piper](https://github.com/rhasspy/piper) | MIT |
| Piper voices (Amy, HFC male, Lessac, Alan, Jenny) | [rhasspy/piper-voices](https://huggingface.co/rhasspy/piper-voices) | Per voice: see each voice's `MODEL_CARD` |
| Kokoro-82M v1.0 model, packaged for sherpa-onnx (`kokoro-multi-lang-v1_0`) | [k2-fsa/sherpa-onnx tts-models](https://github.com/k2-fsa/sherpa-onnx/releases/tag/tts-models); model by [hexgrad/Kokoro-82M](https://huggingface.co/hexgrad/Kokoro-82M) | Apache-2.0 |
| espeak-ng pronunciation data, in the same download | espeak-ng | GPL-3.0-or-later |

Some Piper voices are trained on datasets with their own terms. Check a voice's model card before using its
audio commercially.
