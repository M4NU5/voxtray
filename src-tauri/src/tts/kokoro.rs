//! Kokoro: free, offline, neural voice. It runs in `voxtray-engine`, a
//! separate binary that ships next to Voxtray (see the `voxtray_engine`
//! crate). The model is fetched on first use (see [`crate::download`]).
//!
//! One engine process stays warm for a whole reading, so the model loads
//! once. Stop kills it.

use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::AppHandle;
use voxtray_engine::Client;

use crate::download;
use crate::settings::Settings;
use crate::timeline::Pcm;

/// A running Kokoro engine that synthesises one chunk at a time.
pub struct Engine {
    client: Client,
    voice: String,
    speed: f32,
}

impl Engine {
    /// Download the model if needed, then start the engine. Returns before
    /// the model has loaded; the first chunk waits for it.
    pub fn open(app: &AppHandle, settings: &Settings) -> Result<Engine, String> {
        let exe = engine_exe()?;
        if !download::kokoro_model_installed() {
            crate::reader::notify(
                app,
                "Downloading Kokoro",
                "The Kokoro model (350 MB) downloads once. Reading starts when it's done.",
            );
        }
        let model = download::ensure_kokoro_model(app)?;
        Engine::spawn(&exe, &model, &settings.kokoro_voice, settings.speed)
    }

    pub fn spawn(exe: &Path, model: &Path, voice: &str, speed: f32) -> Result<Engine, String> {
        let mut cmd = Command::new(exe);
        cmd.arg("--model-dir").arg(model).args(["--voice", voice]);
        super::hide_console(&mut cmd);
        Ok(Engine {
            client: Client::spawn(cmd, "Kokoro")?,
            voice: voice.to_string(),
            speed: super::speed_param(speed, 0.5, 2.0) as f32,
        })
    }

    /// Kills the process from another thread, unblocking [`Engine::synthesize`].
    pub fn killer(&self) -> impl FnOnce() + Send + 'static {
        self.client.killer()
    }

    pub fn synthesize(&mut self, text: &str) -> Result<Pcm, String> {
        Pcm::decode(self.client.synthesize(text, &self.voice, self.speed)?)
    }
}

/// The engine binary: `VOXTRAY_ENGINE_EXE` if set (for development),
/// otherwise `voxtray-engine` next to Voxtray, where the installer puts it.
pub fn engine_exe() -> Result<PathBuf, String> {
    if let Some(exe) = std::env::var_os("VOXTRAY_ENGINE_EXE") {
        return Ok(PathBuf::from(exe));
    }
    let exe = std::env::current_exe()
        .ok()
        .and_then(|me| {
            Some(
                me.parent()?
                    .join(format!("voxtray-engine{}", std::env::consts::EXE_SUFFIX)),
            )
        })
        .ok_or("could not locate the Kokoro engine")?;
    if exe.exists() {
        Ok(exe)
    } else {
        Err("The Kokoro engine (voxtray-engine) is missing. Reinstall Voxtray.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The engine from `VOXTRAY_ENGINE_EXE` and the model from
    /// `VOXTRAY_KOKORO_MODEL`. Run with `cargo test -- --ignored`.
    fn engine() -> Engine {
        let exe = engine_exe().expect("set VOXTRAY_ENGINE_EXE");
        let model = std::env::var_os("VOXTRAY_KOKORO_MODEL").expect("set VOXTRAY_KOKORO_MODEL");
        Engine::spawn(&exe, Path::new(&model), "af_heart", 1.0).expect("spawn Kokoro")
    }

    #[test]
    #[ignore = "needs voxtray-engine and the Kokoro model"]
    fn warm_engine_synthesises_chunks_in_order() {
        let mut engine = engine();
        let short = engine.synthesize("Hi.").unwrap();
        let long = engine
            .synthesize("This sentence is a good deal longer than that.")
            .unwrap();
        assert_eq!((short.channels, short.sample_rate), (1, 24_000));
        assert!(long.duration() > short.duration());
        assert!(engine.synthesize("...").is_ok());
    }

    #[test]
    #[ignore = "needs voxtray-engine and the Kokoro model"]
    fn killing_the_engine_unblocks_synthesis_with_an_error() {
        let mut engine = engine();
        engine.killer()();
        assert!(engine.synthesize("Hello there.").is_err());
    }
}
