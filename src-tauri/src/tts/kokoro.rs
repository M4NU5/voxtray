//! Kokoro: free, offline, neural voice. It runs in `voxtray-engine`, a
//! separate binary that ships next to Voxtray (see the `voxtray_engine`
//! crate). The model is fetched on first use (see [`crate::download`]).
//!
//! The engine stays loaded for [`KEEP_WARM`] after a reading, so the next
//! one starts without waiting about 2 s for the model. Stop doesn't kill it:
//! the chunk in progress finishes, is thrown away, and the engine goes back
//! to waiting.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use tauri::AppHandle;
use voxtray_engine::{kokoro_lexicon, Client};

use super::warm::KeepWarm;
use crate::download;
use crate::settings::Settings;
use crate::timeline::Pcm;

/// How long an unused engine stays loaded.
const KEEP_WARM: Duration = Duration::from_secs(10 * 60);

/// The engine left by the last reading, while it is warm.
static IDLE: KeepWarm<Loaded> = KeepWarm::new("Kokoro engine", KEEP_WARM);

/// An engine process and the lexicon it loaded with the model.
struct Loaded {
    client: Client,
    lexicon: &'static str,
}

/// A running Kokoro engine that synthesises one chunk at a time. Dropping it
/// keeps the engine warm for the next reading.
pub struct Engine {
    /// Only taken when the engine is dropped.
    loaded: Option<Loaded>,
    voice: String,
    speed: f32,
    /// A request failed, so the engine may be in a bad state. It isn't kept.
    failed: bool,
}

impl Engine {
    /// The warm engine if there is one. Otherwise download the model if
    /// needed and start an engine, returning before the model has loaded;
    /// the first chunk waits for it.
    pub fn open(app: &AppHandle, settings: &Settings) -> Result<Engine, String> {
        if let Some(engine) = Engine::reuse(&settings.kokoro_voice, settings.speed) {
            return Ok(engine);
        }
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
        let loaded = Loaded {
            client: Client::spawn(cmd, "Kokoro")?,
            lexicon: kokoro_lexicon(voice),
        };
        Ok(Engine::new(loaded, voice, speed))
    }

    /// The engine the last reading left warm, if it is still running and
    /// loaded the lexicon `voice` needs. Otherwise it is stopped.
    fn reuse(voice: &str, speed: f32) -> Option<Engine> {
        let loaded = IDLE.take()?;
        if loaded.lexicon != kokoro_lexicon(voice) || !loaded.client.is_running() {
            return None;
        }
        log::info!("Kokoro engine still warm");
        Some(Engine::new(loaded, voice, speed))
    }

    fn new(loaded: Loaded, voice: &str, speed: f32) -> Engine {
        Engine {
            loaded: Some(loaded),
            voice: voice.to_string(),
            speed: super::speed_param(speed, 0.5, 2.0) as f32,
            failed: false,
        }
    }

    pub fn synthesize(&mut self, text: &str) -> Result<Pcm, String> {
        let loaded = self.loaded.as_mut().expect("only taken on drop");
        let wav = loaded.client.synthesize(text, &self.voice, self.speed);
        self.failed |= wav.is_err();
        Pcm::decode(wav?)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(loaded) = self.loaded.take() {
            if !self.failed && loaded.client.is_running() {
                IDLE.put(loaded);
            }
        }
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
    fn a_dropped_engine_is_reused_while_warm() {
        let mut engine = engine();
        engine.synthesize("Hi.").unwrap();
        drop(engine);
        let mut again = Engine::reuse("af_bella", 1.2).expect("still warm");
        assert_eq!((again.voice.as_str(), again.speed), ("af_bella", 1.2));
        again.synthesize("Hello again.").unwrap();
        drop(again);
        // A British voice needs the other lexicon, so it gets a new engine.
        assert!(Engine::reuse("bf_emma", 1.0).is_none());
    }

    #[test]
    #[ignore = "needs voxtray-engine and the Kokoro model"]
    fn stopping_mid_chunk_keeps_the_engine_warm() {
        use crate::reading::{Outcome, Reading};
        use crate::tts::ChunkEngine;

        let mut engine = ChunkEngine::Kokoro(engine());
        let interrupt = engine.interrupter();
        let long = "This sentence is long enough to keep the engine busy for a while, \
                    so the stop lands while it is still being synthesised.";
        let (tx, rx) = std::sync::mpsc::channel();
        let reading = Reading::start(
            vec![long.to_string(); 3],
            move |chunk| engine.synthesize(chunk),
            interrupt,
            move |_: &Reading, outcome| tx.send(outcome).unwrap(),
        );
        std::thread::sleep(std::time::Duration::from_millis(300));
        reading.stop();
        let outcome = rx.recv_timeout(std::time::Duration::from_secs(60));
        assert_eq!(outcome, Ok(Outcome::Stopped));
        let mut again = Engine::reuse("af_heart", 1.0).expect("still warm after stop");
        again.synthesize("Still here.").unwrap();
    }
}
