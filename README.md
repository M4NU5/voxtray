<div align="center">

<img src="src-tauri/icons/128x128.png" width="96" height="96" alt="Voxtray" />

# Voxtray

**Select text in any app, press a hotkey, hear it read aloud.**

A tiny, cross-platform desktop app that lives in your system tray and reads your
selected text out loud, in any application or website. Use a free neural voice
that runs on your own machine, OpenAI, ElevenLabs, or your computer's built-in voice.

[![Release](https://img.shields.io/github/v/release/harshaldevvv/voxtray?include_prereleases)](https://github.com/harshaldevvv/voxtray/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Build](https://github.com/harshaldevvv/voxtray/actions/workflows/release.yml/badge.svg)](https://github.com/harshaldevvv/voxtray/actions/workflows/release.yml)

</div>

---

## Why

If you read a lot of documentation, articles, or long threads, letting your ears
do some of the work saves your eyes and helps you power through. This app makes
that a one-key habit: **highlight → hotkey → listen**, everywhere on your
computer, with no copy-paste into a website.

## How it works

1. You select text anywhere — a browser, a PDF, your IDE, a chat window.
2. You press the global hotkey (default **Ctrl/Cmd + Alt + R**).
3. The app copies the selection and reads it a sentence or two at a time, so
   long passages start playing almost at once.
4. Press the read hotkey again to pause or resume. While reading,
   **Ctrl/Cmd + Alt + ←/→** skip back or forward 5 seconds. Press
   **Ctrl/Cmd + Alt + S** to stop.

It runs quietly in the tray and can start automatically at login.

## Voice engines

| Engine              | API key | Cost            | Quality          | Offline |
| ------------------- | :-----: | --------------- | ---------------- | :-----: |
| **OpenAI TTS**      |   Yes   | ~$15 / 1M chars | Very natural     |   No    |
| **ElevenLabs**      |   Yes   | Plan-based      | Most human       |   No    |
| **Local HD (Piper)**|   No    | **Free**        | Good & natural   |   Yes   |
| **Kokoro (beta)**   |   No    | **Free**        | Very natural     | After first download |
| **System voice**    |   No    | Free            | Basic/robotic    |   Yes   |

**Local HD** is a free neural voice ([Piper](https://github.com/rhasspy/piper))
that runs entirely on your machine. The engine and your chosen voice (~50 MB)
download once on first use, then work offline with no key and no cost.

Pick whichever you like in Settings and switch anytime. Your API keys are stored
in the operating system's secure keychain — never in a plaintext file.

## Install

Download the installer for your platform from the
[**Releases**](https://github.com/harshaldevvv/voxtray/releases) page:

- **Windows** — `.msi` or `.exe` (NSIS)
- **macOS** — `.dmg` (universal)
- **Linux** — `.AppImage` or `.deb`

Then open the app, choose a voice engine, paste your API key (if needed), and
you're set. See [docs/SETUP.md](docs/SETUP.md) for a step-by-step guide, API-key
instructions, and platform permission notes.

> **macOS note:** the app needs **Accessibility** permission to send the copy
> keystroke. macOS will prompt you on first use — see the setup guide.

## Build from source

You need [Node.js](https://nodejs.org) and the
[Rust toolchain](https://rustup.rs), plus the platform prerequisites listed in
the [Tauri docs](https://tauri.app/start/prerequisites/).

```bash
git clone https://github.com/harshaldevvv/voxtray.git
cd voxtray
npm install
npm run dev      # run in development
npm run build    # produce installers in src-tauri/target/release/bundle
```

More detail in [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md).

## Tech stack

- **[Tauri 2](https://tauri.app)** — tiny, secure desktop shell (Rust core)
- Rust backend: global hotkeys, clipboard capture, secure key storage, audio
- Plain HTML/CSS/JS settings UI (no framework, no build step)

## Privacy

No telemetry, no accounts, no servers of our own. What you read is sent only to the voice provider you choose
(OpenAI or ElevenLabs); System voice, Local HD and Kokoro run on your machine. API keys stay in your OS keychain.
Details in [PRIVACY.md](PRIVACY.md).

## License and disclaimer

Voxtray is free, open source software under the [MIT License](LICENSE). Contributions welcome: see
[CONTRIBUTING.md](CONTRIBUTING.md). Third-party components and their licences are listed in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md). Report security issues privately as described in
[SECURITY.md](SECURITY.md).

Voxtray is provided as is, without warranty of any kind. Cloud voices are billed by their providers to your own
account, and their terms apply to what you send them. OpenAI and ElevenLabs are trademarks of their owners;
Voxtray is independent and not affiliated with them.

Made by [harshal.dev](https://harshal.dev).
