//! The reading pill: a small black pill above the taskbar while Voxtray reads.
//! Its top row is an equaliser and the time read, like Inlay's recording
//! pill. Its bottom row has buttons to skip back, pause, skip forward and stop.
//! Windows only.
//!
//! This file decides what the pill shows and where its buttons are, so it can
//! be tested anywhere. `window` draws it and takes the clicks.

#![cfg_attr(not(windows), allow(dead_code))]

use std::time::Duration;

use crate::settings::Settings;
use crate::timeline::State;

#[cfg(windows)]
mod canvas;
#[cfg(windows)]
mod window;

#[cfg(windows)]
pub use window::{failed, refresh, start};

/// Show the pill if a reading has started. Elsewhere there is no pill.
#[cfg(not(windows))]
pub fn refresh() {}

/// Show that a reading failed. Elsewhere there is no pill.
#[cfg(not(windows))]
pub fn failed() {}

/// What the pill shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Hidden,
    /// Waiting for the first audio: a small orb with a breathing ring.
    Starting,
    Playing,
    Paused,
    /// Synthesis fell behind playback, so the reading waits for audio.
    CatchingUp,
    /// The reading failed: an orange mark on the orb for a few seconds.
    Failed,
}

impl Stage {
    /// The stage for a reading in `state` with `buffered` audio made so far.
    pub fn of(state: State, buffered: Duration) -> Stage {
        match state {
            State::Finished => Stage::Hidden,
            State::Loading if buffered.is_zero() => Stage::Starting,
            State::Loading => Stage::CatchingUp,
            State::Playing => Stage::Playing,
            State::Paused => Stage::Paused,
        }
    }

    /// The full pill with its controls, rather than the small orb.
    pub fn is_full(self) -> bool {
        matches!(self, Stage::Playing | Stage::Paused | Stage::CatchingUp)
    }
}

// Sizes are logical pixels at 96 DPI, measured from the pill's top left.

/// The orb's width and height.
pub const ORB: f32 = 36.0;
pub const WIDTH: f32 = 148.0;
pub const HEIGHT: f32 = 74.0;
/// The equaliser row's height. The buttons sit below it.
pub const ROW: f32 = 36.0;
pub const BARS: usize = 10;
pub const BAR_WIDTH: f32 = 3.0;
pub const BAR_PITCH: f32 = 6.0;
/// The equaliser and clock together, centred in the top row.
const ROW_CONTENT: f32 = BARS as f32 * BAR_PITCH - BAR_WIDTH + 10.0 + CLOCK_WIDTH;
pub const BARS_LEFT: f32 = (WIDTH - ROW_CONTENT) / 2.0;
pub const CLOCK_LEFT: f32 = BARS_LEFT + BARS as f32 * BAR_PITCH - BAR_WIDTH + 10.0;
pub const CLOCK_WIDTH: f32 = 30.0;
/// The centre line of the button row.
pub const BUTTON_Y: f32 = 53.0;
/// The radius of a button's hover circle.
pub const BUTTON_RADIUS: f32 = 14.0;
/// The line between forward and stop.
pub const SEPARATOR_X: f32 = 104.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Back,
    PlayPause,
    Forward,
    Stop,
}

impl Button {
    pub const ALL: [Button; 4] = [
        Button::Back,
        Button::PlayPause,
        Button::Forward,
        Button::Stop,
    ];

    /// The button's centre, from the pill's left edge.
    pub fn x(self) -> f32 {
        match self {
            Button::Back => 24.5,
            Button::PlayPause => 54.5,
            Button::Forward => 84.5,
            Button::Stop => 123.5,
        }
    }
}

/// The button under `(x, y)` on the full pill, if any.
pub fn button_at(x: f32, y: f32) -> Option<Button> {
    if !(ROW..=HEIGHT).contains(&y) {
        return None;
    }
    Button::ALL
        .into_iter()
        .find(|b| (x - b.x()).abs() <= BUTTON_RADIUS + 1.0)
}

/// Bar `i`'s height at `now` seconds. `level` is the smoothed loudness.
pub fn bar_height(stage: Stage, level: f32, now: f32, i: usize) -> f32 {
    let i = i as f32;
    match stage {
        // Inlay's meter: the loudness, with each bar wobbling on its own.
        Stage::Playing => {
            let display = 0.14 + 0.86 * level;
            let wobble = 0.45 + 0.55 * (now * 7.0 + i * 1.7).sin().abs();
            (22.0 * display * wobble).max(4.0)
        }
        // A slow ripple, so waiting doesn't look like a pause.
        Stage::CatchingUp => 4.0 + 6.0 * (now * 5.0 - i * 0.7).sin().max(0.0),
        _ => 4.0,
    }
}

/// Moves the meter towards `target`: quick to rise, slow to fall, as Inlay's does.
pub fn smooth(previous: f32, target: f32) -> f32 {
    if target > previous {
        previous + (target - previous) * 0.6
    } else {
        previous * 0.86 + target * 0.14
    }
}

/// The time read so far, as m:ss.
pub fn clock(position: Duration) -> String {
    let seconds = position.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// What a button does, and the key that does the same, for its tooltip.
/// The key is empty when there isn't one.
pub fn tooltip(button: Button, paused: bool, settings: &Settings) -> (&'static str, String) {
    match button {
        Button::Back => ("Back 5 s", skip_label("CmdOrControl+Alt+Left", settings)),
        Button::Forward => (
            "Forward 5 s",
            skip_label("CmdOrControl+Alt+Right", settings),
        ),
        Button::PlayPause => {
            let key = if crate::tap_key::vk_for(&settings.read_tap_key) != 0 {
                tap_label(&settings.read_tap_key)
            } else {
                key_label(&settings.read_hotkey)
            };
            (if paused { "Resume" } else { "Pause" }, key)
        }
        Button::Stop => ("Stop", key_label(&settings.stop_hotkey)),
    }
}

/// A skip shortcut, or nothing if Read or Stop uses the same keys, since
/// then the skip key isn't registered. Compared as `apply_skip_hotkeys` does,
/// so modifier order doesn't matter.
fn skip_label(accelerator: &str, settings: &Settings) -> String {
    use tauri_plugin_global_shortcut::Shortcut;
    let id = |keys: &str| keys.trim().parse::<Shortcut>().ok().map(|s| s.id());
    let skip = id(accelerator);
    let taken = [&settings.read_hotkey, &settings.stop_hotkey]
        .iter()
        .any(|hotkey| skip.is_some() && id(hotkey) == skip);
    if taken {
        String::new()
    } else {
        key_label(accelerator)
    }
}

/// A shortcut the way Windows writes it: `CmdOrControl+Alt+Left` is `Ctrl+Alt+←`.
fn key_label(accelerator: &str) -> String {
    let accelerator = accelerator.trim();
    if accelerator.is_empty() {
        return String::new();
    }
    accelerator
        .split('+')
        .map(|part| match part {
            "CmdOrControl" | "CommandOrControl" | "CmdOrCtrl" | "CommandOrCtrl" | "Control" => {
                "Ctrl"
            }
            "Left" => "←",
            "Right" => "→",
            "Up" => "↑",
            "Down" => "↓",
            other => other,
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// A tap key's name as Settings shows it: `rightCtrl` is `Right Ctrl`.
fn tap_label(id: &str) -> String {
    let mut label = String::new();
    for (i, c) in id.chars().enumerate() {
        if i == 0 {
            label.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            label.push(' ');
            label.push(c);
        } else {
            label.push(c);
        }
    }
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_follow_the_reading() {
        let none = Duration::ZERO;
        let some = Duration::from_secs(2);
        assert_eq!(Stage::of(State::Loading, none), Stage::Starting);
        assert_eq!(Stage::of(State::Loading, some), Stage::CatchingUp);
        assert_eq!(Stage::of(State::Playing, some), Stage::Playing);
        assert_eq!(Stage::of(State::Paused, some), Stage::Paused);
        assert_eq!(Stage::of(State::Finished, some), Stage::Hidden);
        assert!(!Stage::Starting.is_full());
        assert!(!Stage::Failed.is_full());
        assert!(Stage::CatchingUp.is_full());
    }

    #[test]
    fn the_top_row_is_centred_and_fits() {
        assert_eq!(BARS_LEFT, 25.5);
        assert_eq!(CLOCK_LEFT + CLOCK_WIDTH, WIDTH - BARS_LEFT);
    }

    #[test]
    fn each_button_is_found_at_its_centre_and_nothing_in_the_top_row() {
        for b in Button::ALL {
            assert_eq!(button_at(b.x(), BUTTON_Y), Some(b));
        }
        assert_eq!(button_at(Button::Stop.x(), ROW / 2.0), None);
        // The gap by the separator belongs to no button.
        assert_eq!(button_at(SEPARATOR_X, BUTTON_Y), None);
        assert_eq!(button_at(2.0, BUTTON_Y), None);
    }

    #[test]
    fn buttons_stay_inside_the_pill() {
        for b in Button::ALL {
            assert!(b.x() - BUTTON_RADIUS >= 8.0, "{b:?}");
            assert!(b.x() + BUTTON_RADIUS <= WIDTH - 8.0, "{b:?}");
        }
    }

    #[test]
    fn bars_are_dots_unless_playing_or_waiting() {
        for i in 0..BARS {
            assert_eq!(bar_height(Stage::Paused, 1.0, 3.3, i), 4.0);
            let playing = bar_height(Stage::Playing, 1.0, 3.3, i);
            assert!((4.0..=22.0).contains(&playing), "{playing}");
            // Silence between sentences drops the meter to dots.
            assert_eq!(bar_height(Stage::Playing, 0.0, 3.3, i), 4.0);
            let waiting = bar_height(Stage::CatchingUp, 0.0, 3.3, i);
            assert!((4.0..=10.0).contains(&waiting), "{waiting}");
        }
    }

    #[test]
    fn the_meter_rises_fast_and_falls_slowly() {
        assert!(smooth(0.0, 1.0) >= 0.6);
        assert!(smooth(1.0, 0.0) >= 0.8);
    }

    #[test]
    fn the_clock_reads_minutes_and_seconds() {
        assert_eq!(clock(Duration::from_millis(12_900)), "0:12");
        assert_eq!(clock(Duration::from_secs(605)), "10:05");
    }

    #[test]
    fn tooltips_name_the_keys_in_settings() {
        let mut s = Settings::default();
        assert_eq!(
            tooltip(Button::Back, false, &s),
            ("Back 5 s", "Ctrl+Alt+←".to_string())
        );
        assert_eq!(
            tooltip(Button::Forward, false, &s),
            ("Forward 5 s", "Ctrl+Alt+→".to_string())
        );
        assert_eq!(
            tooltip(Button::PlayPause, false, &s),
            ("Pause", "Ctrl+Alt+R".to_string())
        );
        assert_eq!(
            tooltip(Button::Stop, false, &s),
            ("Stop", "Ctrl+Alt+S".to_string())
        );
        s.read_tap_key = "rightCtrl".into();
        assert_eq!(
            tooltip(Button::PlayPause, true, &s),
            ("Resume", "Right Ctrl".to_string())
        );
        s.read_tap_key = "f13".into();
        assert_eq!(tooltip(Button::PlayPause, false, &s).1, "F13");
        s.stop_hotkey = String::new();
        assert_eq!(tooltip(Button::Stop, false, &s).1, "");
        // Stop on Ctrl+Alt+Left means that key no longer skips back.
        s.stop_hotkey = "CmdOrControl+Alt+Left".into();
        assert_eq!(tooltip(Button::Back, false, &s).1, "");
        s.stop_hotkey = "Alt+Control+Left".into();
        assert_eq!(tooltip(Button::Back, false, &s).1, "");
        assert_eq!(tooltip(Button::Forward, false, &s).1, "Ctrl+Alt+→");
    }
}
