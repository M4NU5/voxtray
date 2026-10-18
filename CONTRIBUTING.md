# Contributing

Thanks for helping. Bug reports, voice requests and pull requests are all welcome.

- **Bugs:** open an issue with your OS, the engine you used, and what happened versus what you expected.
- **Setup:** see [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) for prerequisites, `npm run dev` and builds.
- **Pull requests:** keep them focused on one change, describe how you tested it, and run `cargo fmt` and
  `cargo clippy` in `src-tauri/` before opening one.
- **New engines or voices:** add them behind the existing engine interface in `src-tauri/src/tts/`, and update
  [PRIVACY.md](PRIVACY.md) and [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md) if they download or send anything.

By contributing you agree that your work is released under the [MIT License](LICENSE).
