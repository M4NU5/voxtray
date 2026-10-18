//! Text-to-speech providers and dispatch.

pub mod elevenlabs;
pub mod local;
pub mod openai;
pub mod piper;

use crate::settings::{Provider, Settings};
use tauri::AppHandle;

/// The outcome of a synthesis request.
pub enum Speech {
    /// Encoded audio (MP3) to be played by the [`crate::player::Player`].
    Audio(Vec<u8>),
    /// A spawned local system-voice process that is already speaking.
    Local(std::process::Child),
}

/// Synthesise `text` using the engine selected in `settings`.
pub fn speak(app: &AppHandle, settings: &Settings, text: &str) -> Result<Speech, String> {
    match settings.provider {
        Provider::Openai => openai::synthesize(settings, text).map(Speech::Audio),
        Provider::Elevenlabs => elevenlabs::synthesize(settings, text).map(Speech::Audio),
        Provider::Piper => piper::synthesize(app, settings, text).map(Speech::Audio),
        Provider::Local => local::speak(settings, text).map(Speech::Local),
        // Kokoro is handled entirely in the webview (see reader::speak_text).
        Provider::Kokoro => Err("Kokoro runs in the app window.".into()),
    }
}

/// A short probe used by the "Test voice" button.
pub const SAMPLE_TEXT: &str =
    "This is a test of Voxtray. Selected text will be read to you like this.";
