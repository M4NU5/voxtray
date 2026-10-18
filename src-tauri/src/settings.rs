//! Persistent user settings and secure API-key storage.
//!
//! Non-secret settings live in a JSON file in the platform config dir.
//! API keys are kept in the OS keychain via the `keyring` crate so they never
//! touch disk in plaintext.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const KEYRING_SERVICE: &str = "dev.harshal.voxtray";

/// Which text-to-speech engine to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Openai,
    Elevenlabs,
    /// Free, offline, neural local voice (downloaded on first use).
    Piper,
    /// Free neural voice that runs in the webview via kokoro-js (beta).
    Kokoro,
    /// Built-in operating-system voice.
    Local,
}

impl Provider {
    /// Keychain account name used to store this provider's key.
    pub fn key_account(self) -> Option<&'static str> {
        match self {
            Provider::Openai => Some("openai_api_key"),
            Provider::Elevenlabs => Some("elevenlabs_api_key"),
            Provider::Piper | Provider::Kokoro | Provider::Local => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Active TTS engine.
    pub provider: Provider,
    /// Global hotkey that reads the current selection, as a Tauri accelerator.
    pub read_hotkey: String,
    /// Global hotkey that stops playback.
    pub stop_hotkey: String,
    /// Restore the user's previous clipboard contents after capturing selection.
    pub restore_clipboard: bool,
    /// Playback speed multiplier (0.5 - 2.0 for cloud providers).
    pub speed: f32,
    /// Launch the app automatically at login.
    pub launch_at_startup: bool,

    // --- OpenAI ---
    pub openai_model: String,
    pub openai_voice: String,

    // --- ElevenLabs ---
    pub elevenlabs_model: String,
    pub elevenlabs_voice_id: String,

    // --- Piper (free local neural voice) ---
    /// Piper voice key, e.g. `en_US-amy-medium`.
    pub piper_voice: String,

    // --- Kokoro (free neural voice in the webview, beta) ---
    /// Kokoro voice id, e.g. `af_heart`.
    pub kokoro_voice: String,
    /// Model precision: fp32 | fp16 | q8 | q4 | q4f16.
    pub kokoro_dtype: String,

    // --- Local system voice ---
    /// Platform-specific voice identifier/name (empty = system default).
    pub local_voice: String,
    /// Words-per-minute-ish rate for the local voice (platform dependent).
    pub local_rate: i32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            provider: Provider::Openai,
            read_hotkey: "CmdOrControl+Alt+R".into(),
            stop_hotkey: "CmdOrControl+Alt+S".into(),
            restore_clipboard: true,
            speed: 1.0,
            launch_at_startup: false,
            openai_model: "gpt-4o-mini-tts".into(),
            openai_voice: "alloy".into(),
            elevenlabs_model: "eleven_multilingual_v2".into(),
            elevenlabs_voice_id: "21m00Tcm4TlvDq8ikWAM".into(), // "Rachel"
            piper_voice: "en_US-amy-medium".into(),
            kokoro_voice: "af_heart".into(),
            kokoro_dtype: "q8".into(),
            local_voice: String::new(),
            local_rate: 0,
        }
    }
}

impl Default for Provider {
    fn default() -> Self {
        Provider::Openai
    }
}

fn config_path() -> Result<PathBuf, String> {
    let mut dir = dirs_config_dir().ok_or("could not resolve a config directory")?;
    dir.push("dev.harshal.voxtray");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create config dir: {e}"))?;
    dir.push("settings.json");
    Ok(dir)
}

/// Minimal config-dir resolution without pulling in the `dirs` crate.
fn dirs_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA").map(PathBuf::from)
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| {
            let mut p = PathBuf::from(h);
            p.push("Library/Application Support");
            p
        })
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(x) = std::env::var_os("XDG_CONFIG_HOME") {
            Some(PathBuf::from(x))
        } else {
            std::env::var_os("HOME").map(|h| {
                let mut p = PathBuf::from(h);
                p.push(".config");
                p
            })
        }
    }
}

/// Load settings from disk, falling back to defaults on any error.
pub fn load() -> Settings {
    match config_path().and_then(|p| std::fs::read_to_string(&p).map_err(|e| e.to_string())) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

/// Persist settings to disk.
pub fn save(settings: &Settings) -> Result<(), String> {
    let path = config_path()?;
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("write settings: {e}"))
}

// --- Secure API key storage ---------------------------------------------------

fn entry(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| format!("keychain: {e}"))
}

/// Store (or clear, if `key` is empty) a provider's API key in the OS keychain.
pub fn set_api_key(provider: Provider, key: &str) -> Result<(), String> {
    let account = match provider.key_account() {
        Some(a) => a,
        None => return Ok(()),
    };
    let entry = entry(account)?;
    if key.trim().is_empty() {
        // Ignore "not found" when clearing.
        let _ = entry.delete_credential();
        Ok(())
    } else {
        entry
            .set_password(key.trim())
            .map_err(|e| format!("save key: {e}"))
    }
}

/// Retrieve a provider's API key from the OS keychain, if present.
pub fn get_api_key(provider: Provider) -> Option<String> {
    let account = provider.key_account()?;
    let entry = entry(account).ok()?;
    entry.get_password().ok()
}

/// Whether a key is stored for the provider (always true for the local voice).
pub fn has_api_key(provider: Provider) -> bool {
    match provider {
        Provider::Piper | Provider::Kokoro | Provider::Local => true,
        _ => get_api_key(provider)
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false),
    }
}
