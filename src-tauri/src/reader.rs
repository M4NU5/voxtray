//! Capturing the current selection and reading it aloud.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings as EnigoSettings};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::chunker;
use crate::player::Player;
use crate::reading::{Outcome, Reading, Slot};
use crate::settings::Settings;
use crate::tts::{self, ChunkEngine};

/// Shared application state, managed by Tauri and reachable from hotkey handlers.
pub struct AppState {
    pub player: Arc<Player>,
    pub settings: Arc<Mutex<Settings>>,
    /// The chunked reading in progress, if any.
    pub reading: Mutex<Slot>,
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

pub fn notify(app: &AppHandle, title: &str, body: &str) {
    log::warn!("{title}: {}", crate::logging::loggable(body));
    let _ = app.notification().builder().title(title).body(body).show();
}

/// Start reading `text` aloud, replacing anything already playing.
pub fn speak_text(app: &AppHandle, settings: &Settings, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        notify(app, "Voxtray", "No text was selected.");
        return;
    }

    // Stop whatever is currently playing (any engine) before starting anew.
    let generation = stop_all(app);

    let state: tauri::State<AppState> = app.state();
    log::info!(
        "opening {:?} for {} chars",
        settings.provider,
        text.chars().count()
    );
    let mut engine = match ChunkEngine::open(app, settings) {
        Some(Ok(engine)) => engine,
        Some(Err(e)) => {
            crate::pill::failed();
            return notify(app, "Could not read text", &e);
        }
        // The system voice speaks the whole text itself.
        None => {
            match tts::local::speak(settings, text) {
                Ok(child) => state.player.set_local_child(child),
                Err(e) => notify(app, "Could not read text", &e),
            }
            return;
        }
    };

    let interrupt = engine.interrupter();
    let handle = app.clone();
    let start = move || {
        Reading::start(
            chunker::chunk(text),
            move |chunk| engine.synthesize(chunk),
            interrupt,
            move |reading, outcome| finished(&handle, reading, outcome),
        )
    };
    // Opening the engine can take a while (the first Piper or Kokoro
    // download). If Stop or another read happened meanwhile, drop this one.
    // Play is sent under the slot lock, like Stop in stop_all, so the audio
    // thread gets them in the same order as the slot changes.
    let installed = state.reading.lock().ok().and_then(|mut slot| {
        let reading = slot.install(generation, start)?;
        let played = state.player.play_timeline(reading.shared.clone());
        Some((reading, played))
    });
    let Some((reading, played)) = installed else {
        log::info!("start cancelled while opening the engine");
        return;
    };
    if let Err(e) = played {
        reading.stop();
        crate::pill::failed();
        notify(app, "Could not read text", &e);
    }
    crate::pill::refresh();
    crate::sync_skip_hotkeys(app);
}

/// Runs when a reading ends on its own, was stopped, or failed.
fn finished(app: &AppHandle, reading: &Reading, outcome: Outcome) {
    let state: tauri::State<AppState> = app.state();
    if let Ok(mut slot) = state.reading.lock() {
        slot.clear(reading.id);
    }
    crate::sync_skip_hotkeys(app);
    if let Outcome::Failed(e) = outcome {
        crate::pill::failed();
        notify(app, "Could not read text", &e);
    }
}

/// The reading still playing, paused or loading, if any.
pub fn active_reading(app: &AppHandle) -> Option<Arc<Reading>> {
    let state: tauri::State<AppState> = app.state();
    let current = state.reading.lock().ok()?.current();
    current.filter(|r| r.is_active())
}

/// The read hotkey: pause or resume the current reading, or if nothing is
/// being read, read the selection. A new selection needs Stop first.
pub fn trigger_read(app: &AppHandle) {
    if let Some(reading) = active_reading(app) {
        let state = reading.toggle_pause();
        log::info!("reading {}: {state:?}", reading.id);
        return;
    }
    log::info!("read hotkey: capturing selection");
    read_selection(app);
}

/// Capture the selection and read it. Runs the slow work on a worker thread.
pub fn read_selection(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let state: tauri::State<AppState> = app.state();
        let settings = {
            match state.settings.lock() {
                Ok(g) => g.clone(),
                Err(_) => Settings::default(),
            }
        };

        let captured = capture_selection(settings.restore_clipboard);
        if let Ok(text) = &captured {
            log::info!("captured {} chars", text.chars().count());
        }
        match captured {
            Ok(text) => speak_text(&app, &settings, &text),
            Err(e) => notify(&app, "Could not capture selection", &e),
        }
    });
}

/// Pause or resume the current reading. Does nothing when idle.
pub fn toggle_pause(app: &AppHandle) {
    if let Some(reading) = active_reading(app) {
        let state = reading.toggle_pause();
        log::info!("reading {}: {state:?}", reading.id);
    }
}

/// Skip the current reading 5 s forward or back.
pub fn skip(app: &AppHandle, forward: bool) {
    if let Some(reading) = active_reading(app) {
        let (pos, buffered) = reading.skip(forward);
        log::info!(
            "reading {}: skip to {:.1}s of {:.1}s synthesised",
            reading.id,
            pos.as_secs_f32(),
            buffered.as_secs_f32()
        );
    }
}

/// Stop everything. Returns the reading generation a new start should use.
fn stop_all(app: &AppHandle) -> u64 {
    let state: tauri::State<AppState> = app.state();
    let mut slot = state.reading.lock().unwrap_or_else(|e| e.into_inner());
    let generation = slot.stop();
    state.player.stop();
    drop(slot);
    generation
}

/// Stop any current playback (all engines) and cancel pending synthesis.
pub fn trigger_stop(app: &AppHandle) {
    stop_all(app);
    crate::sync_skip_hotkeys(app);
}
