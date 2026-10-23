//! Text-to-speech providers.
//!
//! Piper, Kokoro and the cloud providers synthesise one chunk at a time
//! through a [`ChunkEngine`] opened once per reading. The system voice speaks
//! the whole text itself (see [`local`]).

pub mod elevenlabs;
pub mod kokoro;
pub mod local;
pub mod openai;
pub mod piper;
mod warm;

use std::process::Command;

use crate::settings::{Provider, Settings};
use crate::timeline::Pcm;
use tauri::AppHandle;

/// A provider ready to synthesise the chunks of one reading.
pub enum ChunkEngine {
    Piper(piper::Engine),
    Kokoro(kokoro::Engine),
    Openai(openai::Client),
    Elevenlabs(elevenlabs::Client),
}

impl ChunkEngine {
    /// Open the engine for `settings.provider`, or `None` if it isn't chunked.
    pub fn open(app: &AppHandle, settings: &Settings) -> Option<Result<ChunkEngine, String>> {
        Some(match settings.provider {
            Provider::Piper => piper::Engine::open(app, settings).map(ChunkEngine::Piper),
            Provider::Kokoro => kokoro::Engine::open(app, settings).map(ChunkEngine::Kokoro),
            Provider::Openai => openai::Client::new(settings).map(ChunkEngine::Openai),
            Provider::Elevenlabs => elevenlabs::Client::new(settings).map(ChunkEngine::Elevenlabs),
            Provider::Local => return None,
        })
    }

    pub fn synthesize(&mut self, text: &str) -> Result<Pcm, String> {
        match self {
            ChunkEngine::Piper(e) => e.synthesize(text),
            ChunkEngine::Kokoro(e) => e.synthesize(text),
            ChunkEngine::Openai(c) => c.synthesize(text),
            ChunkEngine::Elevenlabs(c) => c.synthesize(text),
        }
    }

    /// Something that interrupts a synthesis in progress, if the engine can.
    /// Cloud requests can't be aborted, and Kokoro's are left to finish so
    /// the engine stays warm. Their result is discarded instead.
    pub fn interrupter(&self) -> Option<Box<dyn FnOnce() + Send>> {
        match self {
            ChunkEngine::Piper(e) => Some(Box::new(e.killer())),
            _ => None,
        }
    }
}

/// Keep a local engine from opening a console window on Windows.
fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Clamp a speed to a provider's range, rounded to the slider's precision so
/// the request carries 1.2 rather than f32 noise like 1.2000000476837158.
fn speed_param(speed: f32, min: f32, max: f32) -> f64 {
    (f64::from(speed.clamp(min, max)) * 100.0).round() / 100.0
}

/// Shorten an error body for display, on a character boundary.
fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        None => s.to_string(),
        Some((i, _)) => format!("{}…", &s[..i]),
    }
}

/// A short probe used by the "Test voice" button.
pub const SAMPLE_TEXT: &str =
    "This is a test of Voxtray. Selected text will be read to you like this.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_is_clamped_and_rounded() {
        assert_eq!(speed_param(1.2, 0.7, 1.2), 1.2);
        assert_eq!(speed_param(1.05, 0.25, 4.0), 1.05);
        assert_eq!(speed_param(0.1, 0.25, 4.0), 0.25);
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        assert_eq!(truncate("héllo", 2), "hé…");
        assert_eq!(truncate("hi", 5), "hi");
    }
}
