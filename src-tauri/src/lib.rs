//! Voxtray — library entry point wired up by `main.rs`.

mod chunker;
mod commands;
mod download;
mod logging;
mod player;
mod reader;
mod reading;
mod settings;
mod tap_key;
mod timeline;
mod tts;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use player::Player;
use reader::AppState;
use settings::Settings;

/// (Re)register the read/stop global shortcuts from the given settings.
pub fn register_hotkeys(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    #[cfg(windows)]
    tap_key::set_key(&settings.read_tap_key);

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

    // unregister_all dropped the skip keys too; restore them mid-reading.
    sync_skip_hotkeys(app);
    Ok(())
}

/// Skip back / forward 5 s. Registered only while a reading is active, so the
/// keys stay free for other apps the rest of the time.
const SKIP_HOTKEYS: [(&str, bool); 2] = [
    ("CmdOrControl+Alt+Left", false),
    ("CmdOrControl+Alt+Right", true),
];

/// Register the skip keys if a reading is active, otherwise release them.
///
/// The work is posted to the main thread from another thread, so it runs on
/// a later turn of the event loop. That way it never runs inside a shortcut
/// handler (the plugin holds its shortcut lock while handlers run, so
/// (un)registering there deadlocks), and it is serialised with
/// `register_hotkeys`, which also runs on the main thread.
pub fn sync_skip_hotkeys(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        let handle = app.clone();
        let _ = app.run_on_main_thread(move || apply_skip_hotkeys(&handle));
    });
}

fn apply_skip_hotkeys(app: &AppHandle) {
    static WARNED: AtomicBool = AtomicBool::new(false);
    let active = reader::active_reading(app).is_some();
    let state: tauri::State<AppState> = app.state();
    let settings = state.settings.lock().map(|s| s.clone()).unwrap_or_default();
    // Never touch a combo the user picked for read or stop.
    let taken: Vec<u32> = [&settings.read_hotkey, &settings.stop_hotkey]
        .iter()
        .filter_map(|s| s.trim().parse::<Shortcut>().ok())
        .map(|s| s.id())
        .collect();

    let gs = app.global_shortcut();
    for (combo, forward) in SKIP_HOTKEYS {
        let Ok(shortcut) = combo.parse::<Shortcut>() else {
            continue;
        };
        if taken.contains(&shortcut.id()) {
            continue;
        }
        let registered = gs.is_registered(shortcut);
        if active && !registered {
            let result = gs.on_shortcut(shortcut, move |app, _shortcut, event| {
                if event.state() == ShortcutState::Pressed {
                    reader::skip(app, forward);
                }
            });
            match result {
                Ok(()) => log::info!("registered {combo}"),
                Err(e) => {
                    log::warn!("could not register {combo}: {e}");
                    if !WARNED.swap(true, Ordering::Relaxed) {
                        reader::notify(
                            app,
                            "Skip keys unavailable",
                            &format!(
                                "{} is taken by another app (graphics drivers often use \
                                 Ctrl+Alt+arrows to rotate the screen).",
                                combo.replace("CmdOrControl", "Ctrl")
                            ),
                        );
                    }
                }
            }
        } else if !active && registered {
            match gs.unregister(shortcut) {
                Ok(()) => log::info!("released {combo}"),
                Err(e) => log::warn!("could not release {combo}: {e}"),
            }
        }
    }
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
    let pause_i = MenuItem::with_id(app, "pause", "Pause / Resume", true, None::<&str>)?;
    let stop_i = MenuItem::with_id(app, "stop", "Stop", true, None::<&str>)?;
    let settings_i = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[&read_i, &pause_i, &stop_i, &sep, &settings_i, &quit_i],
    )?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("bundled window icon");

    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .tooltip(&app.package_info().name)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "read" => reader::read_selection(app),
            "pause" => reader::toggle_pause(app),
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

            // Voxtray Dev runs under its own identifier (tauri.dev.conf.json).
            settings::set_app_id(&app.config().identifier);
            if let Ok(dir) = settings::config_dir() {
                logging::init(&dir.join("voxtray.log"));
            }
            log::info!(
                "{} {} started as {}",
                app.package_info().name,
                app.package_info().version,
                settings::app_id()
            );

            // Load persisted settings and set up shared state.
            let loaded = settings::load();
            let state = AppState {
                player: Arc::new(Player::new()),
                settings: Arc::new(Mutex::new(loaded.clone())),
                reading: Mutex::new(Default::default()),
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
            match register_hotkeys(&handle, &loaded) {
                Ok(()) => log::info!(
                    "registered read {} and stop {}",
                    loaded.read_hotkey,
                    loaded.stop_hotkey
                ),
                Err(e) => log::error!("failed to register hotkeys: {e}"),
            }

            #[cfg(windows)]
            if let Err(e) = tap_key::start(handle.clone()) {
                log::error!("{e}");
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
            commands::kokoro_status,
            commands::ensure_kokoro,
            commands::read_now,
            commands::stop_playback,
            commands::test_voice,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Voxtray");
}
