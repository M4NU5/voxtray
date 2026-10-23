//! Tauri commands invoked from the settings UI.

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_autostart::ManagerExt;

use crate::download;
use crate::reader::{self, AppState};
use crate::settings::{self, Provider, Settings};
use crate::tts::{self, SAMPLE_TEXT};

fn parse_provider(s: &str) -> Result<Provider, String> {
    match s {
        "openai" => Ok(Provider::Openai),
        "elevenlabs" => Ok(Provider::Elevenlabs),
        "piper" => Ok(Provider::Piper),
        "kokoro" => Ok(Provider::Kokoro),
        "local" => Ok(Provider::Local),
        other => Err(format!("unknown provider: {other}")),
    }
}

#[derive(Serialize)]
pub struct ProviderStatus {
    pub has_openai: bool,
    pub has_elevenlabs: bool,
}

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Settings {
    state.settings.lock().map(|g| g.clone()).unwrap_or_default()
}

#[tauri::command]
pub fn get_provider_status() -> ProviderStatus {
    ProviderStatus {
        has_openai: settings::has_api_key(Provider::Openai),
        has_elevenlabs: settings::has_api_key(Provider::Elevenlabs),
    }
}

#[tauri::command]
pub fn save_settings(app: AppHandle, state: State<AppState>, new: Settings) -> Result<(), String> {
    // Persist to disk.
    settings::save(&new)?;

    // Apply autostart preference.
    let autostart = app.autolaunch();
    if new.launch_at_startup {
        let _ = autostart.enable();
    } else {
        let _ = autostart.disable();
    }

    // Update in-memory copy.
    if let Ok(mut g) = state.settings.lock() {
        *g = new.clone();
    }

    // Re-register global shortcuts with any new key combos.
    crate::register_hotkeys(&app, &new)?;

    Ok(())
}

#[tauri::command]
pub fn set_api_key(provider: String, key: String) -> Result<(), String> {
    let provider = parse_provider(&provider)?;
    settings::set_api_key(provider, &key)
}

#[tauri::command]
pub fn has_api_key(provider: String) -> Result<bool, String> {
    let provider = parse_provider(&provider)?;
    Ok(settings::has_api_key(provider))
}

#[tauri::command]
pub fn list_local_voices() -> Vec<String> {
    tts::local::list_voices()
}

#[derive(Serialize)]
pub struct PiperVoiceInfo {
    pub key: String,
    pub name: String,
    pub installed: bool,
}

#[tauri::command]
pub fn list_piper_voices() -> Vec<PiperVoiceInfo> {
    download::PIPER_VOICES
        .iter()
        .map(|v| PiperVoiceInfo {
            key: v.key.to_string(),
            name: v.name.to_string(),
            installed: download::voice_installed(v.key),
        })
        .collect()
}

#[derive(Serialize)]
pub struct PiperStatus {
    pub engine: bool,
    pub voice: bool,
}

#[tauri::command]
pub fn piper_status(voice: String) -> PiperStatus {
    PiperStatus {
        engine: download::installed_exe().is_some(),
        voice: download::voice_installed(&voice),
    }
}

/// Download the Piper engine and the given voice (if missing). Runs on a worker
/// thread and reports progress via `hd-progress`; finishes with `hd-done`.
#[tauri::command]
pub fn ensure_piper(app: AppHandle, voice: String) {
    std::thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            download::ensure_engine(&app)?;
            download::ensure_voice(&app, &voice)?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                let _ = app.emit("hd-done", serde_json::json!({ "ok": true }));
            }
            Err(e) => {
                let _ = app.emit("hd-done", serde_json::json!({ "ok": false, "error": e }));
            }
        }
    });
}

#[derive(Serialize)]
pub struct KokoroStatus {
    pub engine: bool,
    pub model: bool,
}

#[tauri::command]
pub fn kokoro_status() -> KokoroStatus {
    KokoroStatus {
        engine: tts::kokoro::engine_exe().is_ok(),
        model: download::kokoro_model_installed(),
    }
}

/// Download the Kokoro model (if missing). Runs on a worker thread and
/// reports progress via `kokoro-progress`; finishes with `kokoro-done`.
#[tauri::command]
pub fn ensure_kokoro(app: AppHandle) {
    std::thread::spawn(move || {
        if download::kokoro_model_installed() {
            let _ = app.emit("kokoro-done", serde_json::json!({ "ok": true }));
        } else {
            // Emits kokoro-done itself, also when a reading started it.
            let _ = download::ensure_kokoro_model(&app);
        }
    });
}

#[tauri::command]
pub fn read_now(app: AppHandle) {
    reader::read_selection(&app);
}

#[tauri::command]
pub fn stop_playback(app: AppHandle) {
    reader::trigger_stop(&app);
}

/// Speak a fixed sample using the provided (possibly unsaved) settings.
#[tauri::command]
pub fn test_voice(app: AppHandle, settings: Settings) -> Result<(), String> {
    // Run off the UI thread; starting an engine can take a moment.
    std::thread::spawn(move || {
        reader::speak_text(&app, &settings, SAMPLE_TEXT);
    });
    Ok(())
}
