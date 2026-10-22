//! Voxtray — library entry point wired up by `main.rs`.

mod chunker;
mod commands;
mod download;
mod player;
mod reader;
mod settings;
mod timeline;
mod tts;

use std::sync::{Arc, Mutex};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_autostart::{ManagerExt, MacosLauncher};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use player::Player;
use reader::AppState;
use settings::Settings;

/// (Re)register the read/stop global shortcuts from the given settings.
pub fn register_hotkeys(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();

    let read = settings.read_hotkey.trim().to_string();
    let stop = settings.stop_hotkey.trim().to_string();

    if !read.is_empty() {
        let label = read.clone();
        gs.on_shortcut(read.as_str(), move |app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                reader::trigger_read(app);
            }
        })
        .map_err(|e| format!("read hotkey '{label}': {e}"))?;
    }

    if !stop.is_empty() && stop != read {
        let label = stop.clone();
        gs.on_shortcut(stop.as_str(), move |app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                reader::trigger_stop(app);
            }
        })
        .map_err(|e| format!("stop hotkey '{label}': {e}"))?;
    }

    Ok(())
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let read_i = MenuItem::with_id(app, "read", "Read selection", true, None::<&str>)?;
    let stop_i = MenuItem::with_id(app, "stop", "Stop", true, None::<&str>)?;
    let settings_i = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&read_i, &stop_i, &sep, &settings_i, &quit_i])?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("bundled window icon");

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip("Voxtray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "read" => reader::trigger_read(app),
            "stop" => reader::trigger_stop(app),
            "settings" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .setup(|app| {
            let handle = app.handle().clone();

            // Load persisted settings and set up shared state.
            let loaded = settings::load();
            let state = AppState {
                player: Arc::new(Player::new()),
                settings: Arc::new(Mutex::new(loaded.clone())),
            };
            app.manage(state);

            // Keep the OS autostart entry in sync with the saved preference.
            let autostart = handle.autolaunch();
            if loaded.launch_at_startup {
                let _ = autostart.enable();
            } else {
                let _ = autostart.disable();
            }

            // Register global shortcuts.
            if let Err(e) = register_hotkeys(&handle, &loaded) {
                log::error!("failed to register hotkeys: {e}");
            }

            // System tray.
            build_tray(&handle)?;

            Ok(())
        })
        // Closing the window hides it to the tray instead of quitting.
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::get_provider_status,
            commands::save_settings,
            commands::set_api_key,
            commands::has_api_key,
            commands::list_local_voices,
            commands::list_piper_voices,
            commands::piper_status,
            commands::ensure_piper,
            commands::read_now,
            commands::stop_playback,
            commands::test_voice,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Voxtray");
}
