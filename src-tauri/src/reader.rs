//! Capturing the current selection and reading it aloud.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings as EnigoSettings};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::player::Player;
use crate::settings::{Provider, Settings};
use crate::tts::{self, Speech};

/// Shared application state, managed by Tauri and reachable from hotkey handlers.
pub struct AppState {
    pub player: Arc<Player>,
    pub settings: Arc<Mutex<Settings>>,
}

/// Simulate the platform copy shortcut so the focused app copies its selection.
///
/// Because the read hotkey (e.g. Ctrl+Alt+R) is usually still physically held
/// when this runs, we first send key-up for every modifier to neutralise the
/// OS modifier state — otherwise a stray Alt/Shift turns our synthetic Ctrl+C
/// into Ctrl+Alt+C and nothing gets copied.
fn simulate_copy() -> Result<(), String> {
    let mut enigo = Enigo::new(&EnigoSettings::default()).map_err(|e| e.to_string())?;

    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;

    // Release any modifiers the user is still holding from the hotkey.
    for k in [Key::Alt, Key::Shift, Key::Control, Key::Meta] {
        let _ = enigo.key(k, Direction::Release);
    }
    std::thread::sleep(Duration::from_millis(60));

    enigo
        .key(modifier, Direction::Press)
        .map_err(|e| e.to_string())?;
    enigo
        .key(Key::Unicode('c'), Direction::Click)
        .map_err(|e| e.to_string())?;
    enigo
        .key(modifier, Direction::Release)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Capture the currently-selected text by copying it to the clipboard.
///
/// The clipboard is cleared first so an empty result reliably means
/// "nothing was selected", and the previous contents are restored afterwards
/// when `restore` is set.
fn capture_selection(restore: bool) -> Result<String, String> {
    let mut clipboard = Clipboard::new().map_err(|e| format!("clipboard: {e}"))?;
    let previous = clipboard.get_text().ok();

    // Sentinel so we can detect whether the copy produced anything.
    let _ = clipboard.set_text(String::new());
    // Small delay so the emptied clipboard settles before we send copy.
    std::thread::sleep(Duration::from_millis(40));

    simulate_copy()?;

    // Poll the clipboard: apps service the copy command asynchronously, and
    // slower ones (browsers, Electron apps) can take several hundred ms.
    let mut text = String::new();
    let deadline = Instant::now() + Duration::from_millis(1200);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(40));
        if let Ok(t) = clipboard.get_text() {
            if !t.is_empty() {
                text = t;
                break;
            }
        }
    }

    if restore {
        if let Some(prev) = previous {
            let _ = clipboard.set_text(prev);
        }
    }

    Ok(text)
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show();
}

/// Speak the given text using the provided settings, routing to the player.
pub fn speak_text(app: &AppHandle, player: &Player, settings: &Settings, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        notify(app, "Voxtray", "No text was selected.");
        return;
    }

    // Stop whatever is currently playing (any engine) before starting anew.
    player.stop();
    let _ = app.emit("kokoro-stop", ());

    // Kokoro runs entirely in the webview; hand off the text to the UI.
    if settings.provider == Provider::Kokoro {
        let _ = app.emit(
            "kokoro-speak",
            serde_json::json!({
                "text": text,
                "voice": settings.kokoro_voice,
                "speed": settings.speed,
                "dtype": settings.kokoro_dtype,
            }),
        );
        return;
    }

    match tts::speak(app, settings, text) {
        Ok(Speech::Audio(bytes)) => player.play_bytes(bytes),
        Ok(Speech::Local(child)) => player.set_local_child(child),
        Err(e) => notify(app, "Could not read text", &e),
    }
}

/// Capture the selection and read it. Runs the slow work on a worker thread.
pub fn trigger_read(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let state: tauri::State<AppState> = app.state();
        let settings = {
            match state.settings.lock() {
                Ok(g) => g.clone(),
                Err(_) => Settings::default(),
            }
        };
        let player = state.player.clone();

        match capture_selection(settings.restore_clipboard) {
            Ok(text) => speak_text(&app, &player, &settings, &text),
            Err(e) => notify(&app, "Could not capture selection", &e),
        }
    });
}

/// Stop any current playback (all engines).
pub fn trigger_stop(app: &AppHandle) {
    let state: tauri::State<AppState> = app.state();
    state.player.stop();
    let _ = app.emit("kokoro-stop", ());
}
