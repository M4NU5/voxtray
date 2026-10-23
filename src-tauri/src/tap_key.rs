//! Read by tapping a single key on its own, such as Right Ctrl. Windows only.
//!
//! The pattern follows Inlay's Windows client: a low-level keyboard hook
//! (`WH_KEYBOARD_LL`) on its own thread that only observes, so the key keeps
//! working everywhere else. Global shortcuts can't do this: `RegisterHotKey`
//! needs a non-modifier key and can't tell Right Ctrl from Left Ctrl.
//!
//! A tap counts only when the key goes down with nothing else held, comes up
//! within [`TAP_MAX_MS`], and no other key or mouse button was used meanwhile.
//! So Ctrl+C, Ctrl+click and Ctrl+scroll never read anything.

#![cfg_attr(not(windows), allow(dead_code))]

/// A key held longer than this isn't a tap.
pub const TAP_MAX_MS: u32 = 500;

pub struct Key {
    pub id: &'static str,
    pub vk: u16,
}

/// The keys offered in settings (labelled in `index.html`). Inlay's list, so
/// the two apps agree.
pub const KEYS: &[Key] = &[
    key("rightCtrl", 0xA3),
    key("rightAlt", 0xA5),
    key("rightShift", 0xA1),
    key("scrollLock", 0x91),
    key("pause", 0x13),
    key("f13", 0x7C),
    key("f14", 0x7D),
    key("f15", 0x7E),
    key("f16", 0x7F),
    key("f17", 0x80),
    key("f18", 0x81),
    key("f19", 0x82),
    key("f20", 0x83),
    key("f21", 0x84),
    key("f22", 0x85),
    key("f23", 0x86),
    key("f24", 0x87),
];

const fn key(id: &'static str, vk: u16) -> Key {
    Key { id, vk }
}

/// The virtual-key code for a setting, or 0 (off) for an empty or unknown id.
pub fn vk_for(id: &str) -> u16 {
    KEYS.iter().find(|k| k.id == id).map_or(0, |k| k.vk)
}

/// One event from the hooks. Times are the hook's millisecond clock.
#[derive(Debug, Clone, Copy)]
pub enum Input {
    Down(u16, u32),
    Up(u16, u32),
    /// A mouse button or wheel, never a plain move.
    Mouse,
}

/// Decides when a sequence of inputs is a tap of `key`.
#[derive(Default)]
pub struct Tap {
    /// 0 means off.
    key: u16,
    down_at: Option<u32>,
    /// Something else happened during this press, so it isn't a tap.
    spoiled: bool,
}

impl Tap {
    pub fn set_key(&mut self, vk: u16) {
        *self = Tap {
            key: vk,
            ..Tap::default()
        };
    }

    /// Feed one input. True when it completes a tap. `others_held` reports
    /// whether any other key or mouse button is down; it is only asked when
    /// the key goes down.
    pub fn feed(&mut self, input: Input, others_held: impl FnOnce() -> bool) -> bool {
        if self.key == 0 {
            return false;
        }
        match input {
            Input::Down(vk, at) if vk == self.key => {
                // Auto-repeat sends more downs while held; only the first counts.
                if self.down_at.is_none() {
                    self.down_at = Some(at);
                    self.spoiled = others_held();
                }
                false
            }
            Input::Up(vk, at) if vk == self.key => match self.down_at.take() {
                Some(down) => !self.spoiled && at.wrapping_sub(down) <= TAP_MAX_MS,
                None => false,
            },
            Input::Down(..) | Input::Mouse => {
                self.spoiled |= self.down_at.is_some();
                false
            }
            Input::Up(..) => false,
        }
    }
}

#[cfg(windows)]
pub use hook::{set_key, start};

#[cfg(windows)]
mod hook {
    use super::{Input, Tap};
    use std::ptr::{null, null_mut};
    use std::sync::mpsc::{self, Sender};
    use std::sync::{Mutex, OnceLock};
    use tauri::AppHandle;
    use windows_sys::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
        HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, MSG, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN,
        WM_KEYUP, WM_MOUSEMOVE, WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    static TAP: Mutex<Tap> = Mutex::new(Tap {
        key: 0,
        down_at: None,
        spoiled: false,
    });
    static SINK: OnceLock<Sender<()>> = OnceLock::new();

    /// Use the key with setting id `id`, or none if it's empty or unknown.
    pub fn set_key(id: &str) {
        let vk = super::vk_for(id);
        log::info!("read tap key: {}", if vk == 0 { "off" } else { id });
        lock().set_key(vk);
    }

    /// Read (or pause) on each tap.
    pub fn start(app: AppHandle) -> Result<(), String> {
        install(move || {
            log::info!("read key tapped");
            crate::reader::trigger_read(&app);
        })
    }

    /// Install the hooks on their own thread, which pumps messages for them.
    /// `on_tap` runs on another thread, since the hooks must return quickly.
    /// Once per process.
    pub fn install(on_tap: impl Fn() + Send + 'static) -> Result<(), String> {
        let (tx, taps) = mpsc::channel();
        SINK.set(tx)
            .map_err(|_| "the tap key is already installed")?;
        std::thread::Builder::new()
            .name("tap-key".into())
            .spawn(move || taps.iter().for_each(|()| on_tap()))
            .map_err(|e| e.to_string())?;

        let (ready, installed) = mpsc::channel();
        std::thread::Builder::new()
            .name("tap-key-hook".into())
            .spawn(move || unsafe {
                let module = GetModuleHandleW(null());
                let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), module, 0);
                let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), module, 0);
                let _ = ready.send(!keyboard.is_null() && !mouse.is_null());
                // The hooks live as long as the app, so they're never removed.
                let mut message: MSG = std::mem::zeroed();
                while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            })
            .map_err(|e| e.to_string())?;
        match installed.recv() {
            Ok(true) => Ok(()),
            _ => Err("Windows refused the keyboard hook, so the read key can't work.".into()),
        }
    }

    fn lock() -> std::sync::MutexGuard<'static, Tap> {
        TAP.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn feed(input: Input, held_except: u16) {
        if lock().feed(input, || others_held(held_except)) {
            if let Some(sink) = SINK.get() {
                let _ = sink.send(());
            }
        }
    }

    /// Must return quickly: Windows drops hooks that stall input.
    unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
            // Skip Voxtray's own synthetic Ctrl+C.
            if event.dwExtraInfo != enigo::EVENT_MARKER as usize {
                let vk = sided(event);
                let input = match wparam as u32 {
                    WM_KEYDOWN | WM_SYSKEYDOWN => Some(Input::Down(vk, event.time)),
                    WM_KEYUP | WM_SYSKEYUP => Some(Input::Up(vk, event.time)),
                    _ => None,
                };
                if let Some(input) = input {
                    feed(input, vk);
                }
            }
        }
        unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
    }

    unsafe extern "system" fn mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 && wparam as u32 != WM_MOUSEMOVE {
            feed(Input::Mouse, 0);
        }
        unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
    }

    /// Whether any key or mouse button other than `vk` is down. The generic
    /// Shift, Ctrl and Alt codes are skipped: they mirror the sided ones,
    /// including the tap key itself.
    fn others_held(vk: u16) -> bool {
        (1..=0xFEu16)
            .filter(|&k| k != vk && !matches!(k, VK_SHIFT | VK_CONTROL | VK_MENU))
            .any(|k| unsafe { GetAsyncKeyState(i32::from(k)) as u16 & 0x8000 != 0 })
    }

    /// Low-level hooks normally report left/right modifier codes already; this
    /// also handles drivers that send the generic code with the extended flag.
    fn sided(event: &KBDLLHOOKSTRUCT) -> u16 {
        let extended = event.flags & LLKHF_EXTENDED != 0;
        match event.vkCode as u16 {
            VK_CONTROL if extended => VK_RCONTROL,
            VK_CONTROL => VK_LCONTROL,
            VK_MENU if extended => VK_RMENU,
            VK_MENU => VK_LMENU,
            VK_SHIFT if event.scanCode == 0x36 => VK_RSHIFT,
            VK_SHIFT => VK_LSHIFT,
            vk => vk,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RCTRL: u16 = 0xA3;
    const C: u16 = 0x43;

    fn tap() -> Tap {
        let mut tap = Tap::default();
        tap.set_key(RCTRL);
        tap
    }

    /// Feed inputs with nothing else held; true if any completed a tap.
    fn run(tap: &mut Tap, inputs: &[Input]) -> Vec<bool> {
        inputs.iter().map(|&i| tap.feed(i, || false)).collect()
    }

    #[test]
    fn a_quick_press_and_release_is_a_tap() {
        let mut t = tap();
        let fired = run(&mut t, &[Input::Down(RCTRL, 1000), Input::Up(RCTRL, 1120)]);
        assert_eq!(fired, [false, true]);
    }

    #[test]
    fn auto_repeat_does_not_restart_the_press() {
        let mut t = tap();
        let fired = run(
            &mut t,
            &[
                Input::Down(RCTRL, 1000),
                Input::Down(RCTRL, 1300),
                Input::Up(RCTRL, 1600),
            ],
        );
        assert_eq!(fired, [false, false, false], "held 600 ms in total");
    }

    #[test]
    fn a_shortcut_is_not_a_tap() {
        let mut t = tap();
        let fired = run(
            &mut t,
            &[
                Input::Down(RCTRL, 1000),
                Input::Down(C, 1050),
                Input::Up(C, 1080),
                Input::Up(RCTRL, 1100),
            ],
        );
        assert!(fired.iter().all(|f| !f));
    }

    #[test]
    fn mouse_use_while_held_is_not_a_tap() {
        let mut t = tap();
        let fired = run(
            &mut t,
            &[
                Input::Down(RCTRL, 1000),
                Input::Mouse,
                Input::Up(RCTRL, 1100),
            ],
        );
        assert!(fired.iter().all(|f| !f));
    }

    #[test]
    fn a_press_with_another_key_already_held_is_not_a_tap() {
        let mut t = tap();
        assert!(!t.feed(Input::Down(RCTRL, 1000), || true));
        assert!(!t.feed(Input::Up(RCTRL, 1100), || false));
    }

    #[test]
    fn a_long_hold_is_not_a_tap() {
        let mut t = tap();
        let fired = run(
            &mut t,
            &[
                Input::Down(RCTRL, 1000),
                Input::Up(RCTRL, 1000 + TAP_MAX_MS + 1),
            ],
        );
        assert_eq!(fired, [false, false]);
    }

    #[test]
    fn the_clock_wrapping_mid_tap_is_fine() {
        let mut t = tap();
        let fired = run(
            &mut t,
            &[Input::Down(RCTRL, u32::MAX - 50), Input::Up(RCTRL, 50)],
        );
        assert_eq!(fired, [false, true]);
    }

    #[test]
    fn a_spoiled_press_does_not_spoil_the_next() {
        let mut t = tap();
        run(
            &mut t,
            &[Input::Down(RCTRL, 0), Input::Mouse, Input::Up(RCTRL, 100)],
        );
        let fired = run(&mut t, &[Input::Down(RCTRL, 500), Input::Up(RCTRL, 600)]);
        assert_eq!(fired, [false, true]);
    }

    #[test]
    fn other_keys_and_no_key_never_tap() {
        let mut t = tap();
        assert_eq!(
            run(&mut t, &[Input::Down(C, 0), Input::Up(C, 50)]),
            [false, false]
        );
        let mut off = Tap::default();
        assert_eq!(
            run(&mut off, &[Input::Down(RCTRL, 0), Input::Up(RCTRL, 50)]),
            [false, false]
        );
    }

    #[test]
    fn setting_ids_map_to_keys() {
        assert_eq!(vk_for("rightCtrl"), RCTRL);
        assert_eq!(vk_for("f24"), 0x87);
        assert_eq!(vk_for(""), 0);
        assert_eq!(vk_for("leftCtrl"), 0);
    }
}

/// Needs a Windows desktop session. It injects Right Ctrl and F24 presses,
/// which apps ignore on their own.
#[cfg(all(test, windows))]
mod desktop_tests {
    use std::sync::mpsc;
    use std::time::Duration;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_KEYUP, VK_F24, VK_RCONTROL,
    };

    fn press(vk: u16, up: bool) -> INPUT {
        let mut flags = if vk == VK_RCONTROL {
            KEYEVENTF_EXTENDEDKEY
        } else {
            0
        };
        if up {
            flags |= KEYEVENTF_KEYUP;
        }
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn send(inputs: &[INPUT]) {
        let sent = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        assert_eq!(sent as usize, inputs.len(), "SendInput was blocked");
    }

    #[test]
    #[ignore = "injects key presses into the Windows desktop"]
    fn the_hook_reads_on_a_real_tap_only() {
        let (tx, taps) = mpsc::channel();
        super::hook::install(move || tx.send(()).unwrap()).unwrap();
        super::set_key("rightCtrl");

        send(&[press(VK_RCONTROL, false), press(VK_RCONTROL, true)]);
        assert!(taps.recv_timeout(Duration::from_secs(2)).is_ok(), "tap");

        send(&[
            press(VK_RCONTROL, false),
            press(VK_F24, false),
            press(VK_F24, true),
            press(VK_RCONTROL, true),
        ]);
        assert!(
            taps.recv_timeout(Duration::from_secs(1)).is_err(),
            "shortcut"
        );

        super::set_key("");
        send(&[press(VK_RCONTROL, false), press(VK_RCONTROL, true)]);
        assert!(taps.recv_timeout(Duration::from_secs(1)).is_err(), "off");
    }
}
