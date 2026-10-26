//! The reading pill's window: a layered, topmost popup above the taskbar. It
//! takes clicks but never focus, so the app being read keeps the keyboard.
//! It runs on its own thread. While it shows, a 16 ms timer reads the
//! reading's state and redraws. The morphing follows Inlay's recording pill
//! (`Windows/src/overlay.rs` on Inlay's `windows-glacier-ui` branch).

use std::f32::consts::{PI, TAU};
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    TrackMouseEvent, TME_LEAVE, TRACKMOUSEEVENT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::canvas::{alpha, mix, Canvas, Rect};
use super::{
    bar_height, button_at, smooth, Button, Stage, BARS, BARS_LEFT, BAR_PITCH, BAR_WIDTH,
    BUTTON_RADIUS, BUTTON_Y, CLOCK_LEFT, CLOCK_WIDTH, HEIGHT, ORB, ROW, SEPARATOR_X, WIDTH,
};
use crate::reader::AppState;

const PILL: u32 = 0x05080B;
const ORB_FILL: u32 = 0x0E161D;
const BORDER: u32 = 0x2E4556;
const ACCENT: u32 = 0xA7D6F5;
const INK: u32 = 0xEDF5FA;
const MUTED: u32 = 0xA4B8C7;
const FAINT: u32 = 0x6F8798;
const HOVER: u32 = 0x1B2833;
const TIP: u32 = 0x222C35;
const ORANGE: u32 = 0xFF9F0A;

const FRAME_TIMER: usize = 1;
const WM_REFRESH: u32 = WM_APP + 1;
/// Not in the windows-sys features this crate uses.
const WM_MOUSELEAVE: u32 = 0x02A3;
/// Seconds for one shape to morph into another.
const MORPH: f32 = 0.35;
/// The orb opening into the pill takes longer, so it visibly opens.
const MORPH_OPEN: f32 = 0.5;
/// Seconds to grow in on appearing and shrink out on leaving.
const FADE: f32 = 0.2;
/// Hover this long before a tooltip shows.
const TIP_DELAY: f32 = 0.4;
/// How long a failure stays up.
const FAILED_FOR: Duration = Duration::from_secs(4);

// The window is a fixed canvas, in logical pixels. The pill sits at its
// bottom centre with room around it for the shadow and above it for a
// tooltip. Fully transparent pixels let clicks through.
const MARGIN: f32 = 16.0;
const TIP_SPACE: f32 = 34.0;
const CANVAS_WIDTH: f32 = 240.0;
const CANVAS_HEIGHT: f32 = MARGIN + TIP_SPACE + HEIGHT + MARGIN;
/// Inlay's pill sits this far above the work area's bottom edge. So does this one.
const ABOVE_TASKBAR: f32 = 24.0;

/// What the pill needs to know each frame.
pub struct Snapshot {
    pub stage: Stage,
    /// Loudness of the audio playing, 0 to 1.
    pub level: f32,
    pub position: Duration,
}

/// What a click asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Press(Button),
    /// The orb was clicked while waiting for the first audio.
    Cancel,
    /// The failure mark was clicked.
    ShowDetails,
}

/// How the pill reads the app and acts on it. Tests pass fakes.
pub struct Hooks {
    pub snapshot: Box<dyn Fn() -> Snapshot + Send + Sync>,
    pub act: Box<dyn Fn(Action) + Send + Sync>,
    pub tooltip: Box<dyn Fn(Button, bool) -> (String, String) + Send + Sync>,
}

static HOOKS: OnceLock<Hooks> = OnceLock::new();
static WINDOW: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static FAILED_AT: Mutex<Option<Instant>> = Mutex::new(None);
/// The pill while it shows. Only the pill's thread touches it.
static PILL_STATE: Mutex<Option<Pill>> = Mutex::new(None);

/// Starts the pill for the app.
pub fn start(app: AppHandle) -> Result<(), String> {
    let (a, b, c) = (app.clone(), app.clone(), app);
    install(Hooks {
        snapshot: Box::new(move || snapshot(&a)),
        act: Box::new(move |action| act(&b, action)),
        tooltip: Box::new(move |button, paused| {
            let (label, key) = super::tooltip(button, paused, &settings(&c));
            (label.to_string(), key)
        }),
    })
}

/// Shows the pill if a reading has started. Any thread.
pub fn refresh() {
    let window = WINDOW.load(Ordering::SeqCst);
    if !window.is_null() {
        unsafe { PostMessageW(window, WM_REFRESH, 0, 0) };
    }
}

/// Shows the failure mark for a few seconds. Any thread.
pub fn failed() {
    *lock(&FAILED_AT) = Some(Instant::now());
    refresh();
}

/// Creates the window on its own thread, which pumps its messages. Once per process.
pub fn install(hooks: Hooks) -> Result<(), String> {
    HOOKS
        .set(hooks)
        .map_err(|_| "the reading pill is already running")?;
    let (ready, created) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("reading-pill".into())
        .spawn(move || unsafe {
            let window = create_window();
            let _ = ready.send(!window.is_null());
            if window.is_null() {
                return;
            }
            WINDOW.store(window, Ordering::SeqCst);
            let mut message: MSG = std::mem::zeroed();
            while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        })
        .map_err(|e| e.to_string())?;
    match created.recv() {
        Ok(true) => Ok(()),
        _ => Err("could not create the reading pill window".into()),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn settings(app: &AppHandle) -> crate::settings::Settings {
    let state: tauri::State<AppState> = app.state();
    let settings = lock(&state.settings).clone();
    settings
}

fn snapshot(app: &AppHandle) -> Snapshot {
    let hidden = Snapshot {
        stage: Stage::Hidden,
        level: 0.0,
        position: Duration::ZERO,
    };
    if !settings(app).show_pill {
        return hidden;
    }
    let state: tauri::State<AppState> = app.state();
    let current = lock(&state.reading).current();
    if let Some(reading) = current {
        // Locked directly: `Shared::with` would wake the synthesis worker.
        let (stage, position) = {
            let t = lock(&reading.shared.timeline);
            (Stage::of(t.state(), t.buffered()), t.position())
        };
        if stage != Stage::Hidden {
            *lock(&FAILED_AT) = None;
            return Snapshot {
                stage,
                level: reading.shared.level(),
                position,
            };
        }
    }
    let failed = lock(&FAILED_AT).is_some_and(|at| at.elapsed() < FAILED_FOR);
    if failed {
        Snapshot {
            stage: Stage::Failed,
            ..hidden
        }
    } else {
        hidden
    }
}

fn act(app: &AppHandle, action: Action) {
    let app = app.clone();
    // Off the pill's thread: stopping can wait on the engine.
    std::thread::spawn(move || match action {
        Action::Press(Button::Back) => crate::reader::skip(&app, false),
        Action::Press(Button::Forward) => crate::reader::skip(&app, true),
        Action::Press(Button::PlayPause) => crate::reader::toggle_pause(&app),
        Action::Press(Button::Stop) | Action::Cancel => crate::reader::trigger_stop(&app),
        Action::ShowDetails => {
            *lock(&FAILED_AT) = None;
            crate::show_main(&app);
        }
    });
}

/// A shape: its size in logical pixels and colours as 0xAARRGGBB.
#[derive(Clone, Copy)]
struct Look {
    width: f32,
    height: f32,
    fill: u32,
    glow: u32,
}

fn look(stage: Stage) -> Look {
    if stage.is_full() {
        Look {
            width: WIDTH,
            height: HEIGHT,
            fill: 0xFF00_0000 | PILL,
            glow: 0x30A7_D6F5,
        }
    } else {
        Look {
            width: ORB,
            height: ORB,
            fill: 0xFF00_0000 | ORB_FILL,
            glow: if stage == Stage::Failed {
                0x30FF_9F0A
            } else {
                0x26A7_D6F5
            },
        }
    }
}

/// The pill while it shows.
struct Pill {
    stage: Stage,
    previous: Stage,
    /// When the current morph began, and how long it takes.
    since: f32,
    duration: f32,
    from: Look,
    shown_at: f32,
    leaving_at: Option<f32>,
    level: f32,
    clock: String,
    hover: Option<Button>,
    hover_since: f32,
    pressed: Option<Button>,
    tracking: bool,
    /// Bottom centre of the pill on screen, in device pixels.
    anchor: (i32, i32),
    scale: f32,
    /// How far the pill has moved up to clear Inlay's, in device pixels.
    lift: f32,
    /// Where the full pill's top left was last drawn, in window pixels,
    /// and its pixels per logical pixel. For hit testing.
    origin: (f32, f32, f32),
}

impl Pill {
    fn new(stage: Stage, now: f32) -> Pill {
        let (anchor, scale) = anchor();
        Pill {
            stage,
            previous: stage,
            since: now - MORPH,
            duration: MORPH,
            from: look(stage),
            shown_at: now,
            leaving_at: None,
            level: 0.0,
            clock: "0:00".into(),
            hover: None,
            hover_since: now,
            pressed: None,
            tracking: false,
            anchor,
            scale,
            lift: 0.0,
            origin: (0.0, 0.0, scale),
        }
    }

    /// Moves to `stage`, morphing when the shape changes.
    fn go(&mut self, stage: Stage, now: f32) {
        if stage == self.stage {
            return;
        }
        if stage.is_full() == self.stage.is_full() && stage != Stage::Hidden {
            // Same shape: only the content changes.
            self.stage = stage;
            return;
        }
        let current = frame(self, now);
        self.from = Look {
            width: current.width,
            height: current.height,
            ..current.look
        };
        self.duration = if stage.is_full() { MORPH_OPEN } else { MORPH };
        self.previous = self.stage;
        self.stage = stage;
        self.since = now;
    }

    /// Ready for clicks: the full pill, done morphing, not leaving.
    fn interactive(&self, now: f32) -> bool {
        self.stage.is_full() && self.leaving_at.is_none() && now - self.since >= self.duration
    }
}

/// One frame of the animation.
struct Frame {
    width: f32,
    height: f32,
    scale: f32,
    look: Look,
    /// How visible the outgoing and incoming content are, 0 to 1.
    old: f32,
    new: f32,
    alpha: f32,
}

/// Overshoots a little before settling, so shapes grow into place.
fn ease_out_back(t: f32) -> f32 {
    let (c1, c3) = (1.2, 2.2);
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

fn smooth_step(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn frame(pill: &Pill, now: f32) -> Frame {
    let target = look(pill.stage);
    let t = ((now - pill.since) / pill.duration).clamp(0.0, 1.0);
    let eased = smooth_step(t);
    let grow = ease_out_back(t);
    let width = pill.from.width + (target.width - pill.from.width) * grow;
    let height = pill.from.height + (target.height - pill.from.height) * grow;
    // Dip mid-morph, then grow into the new shape.
    let mut scale = 1.0 - 0.1 * (PI * t).sin();
    let appear = ((now - pill.shown_at) / FADE).clamp(0.0, 1.0);
    scale *= 0.6 + 0.4 * ease_out_back(appear);
    let mut alpha = appear;
    if let Some(leaving) = pill.leaving_at {
        let gone = ((now - leaving) / FADE).clamp(0.0, 1.0);
        scale *= 1.0 - 0.4 * gone * gone;
        alpha *= 1.0 - gone;
    }
    Frame {
        width,
        height,
        scale,
        look: Look {
            width,
            height,
            fill: mix(pill.from.fill, target.fill, eased),
            glow: mix(pill.from.glow, target.glow, eased),
        },
        old: if t >= 1.0 {
            0.0
        } else {
            1.0 - (t / 0.45).min(1.0)
        },
        new: ((t - 0.35) / 0.55).clamp(0.0, 1.0),
        alpha,
    }
}

/// Seconds since the process started, for animation.
fn clock() -> f32 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f32()
}

/// Shows, morphs, redraws or hides the pill. Pill thread only.
fn update() {
    let Some(hooks) = HOOKS.get() else { return };
    let window = WINDOW.load(Ordering::SeqCst);
    if window.is_null() {
        return;
    }
    let snapshot = (hooks.snapshot)();
    let now = clock();
    let mut guard = lock(&PILL_STATE);
    let mut appearing = false;
    match (guard.as_mut(), snapshot.stage) {
        (None, Stage::Hidden) => return,
        (None, stage) => {
            *guard = Some(Pill::new(stage, now));
            appearing = true;
        }
        (Some(pill), Stage::Hidden) => match pill.leaving_at {
            None => {
                // Shrink back into the orb, then fade out.
                let was_full = pill.stage.is_full();
                pill.go(Stage::Hidden, now);
                pill.leaving_at = Some(now + if was_full { MORPH * 0.6 } else { 0.0 });
                pill.hover = None;
            }
            Some(leaving) if now - leaving >= FADE => {
                *guard = None;
                unsafe {
                    KillTimer(window, FRAME_TIMER);
                    ShowWindow(window, SW_HIDE);
                }
                return;
            }
            Some(_) => {}
        },
        (Some(pill), stage) => {
            // A new reading while shrinking away: grow back in.
            if pill.leaving_at.take().is_some() {
                pill.shown_at = now;
            }
            pill.go(stage, now);
        }
    }
    let Some(pill) = guard.as_mut() else { return };
    if snapshot.stage != Stage::Hidden {
        pill.level = smooth(pill.level, snapshot.level);
        pill.clock = super::clock(snapshot.position);
    }
    pill.lift += (inlay_lift(pill) - pill.lift) * 0.25;
    let tip = tooltip(pill, hooks, now);
    let frame = frame(pill, now);
    present(window, pill, &frame, now, tip.as_ref());
    if appearing {
        unsafe {
            SetWindowPos(
                window,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
            ShowWindow(window, SW_SHOWNOACTIVATE);
            SetTimer(window, FRAME_TIMER, 16, None);
        }
    }
}

/// A tooltip: what the button does, its key, the button's centre in logical
/// pixels, and how far it has faded in.
struct Tip {
    label: String,
    key: String,
    x: f32,
    opacity: f32,
}

fn tooltip(pill: &Pill, hooks: &Hooks, now: f32) -> Option<Tip> {
    let button = pill.hover.filter(|_| pill.interactive(now))?;
    let shown = now - pill.hover_since - TIP_DELAY;
    if shown < 0.0 {
        return None;
    }
    let (label, key) = (hooks.tooltip)(button, pill.stage == Stage::Paused);
    Some(Tip {
        label,
        key,
        x: button.x(),
        opacity: (shown / 0.12).min(1.0),
    })
}

/// The window size in device pixels at `scale`.
fn canvas_size(scale: f32) -> (i32, i32) {
    (
        (CANVAS_WIDTH * scale).ceil() as i32,
        (CANVAS_HEIGHT * scale).ceil() as i32,
    )
}

/// Draws one frame into the top-down premultiplied `pixels` of a canvas
/// sized by [`canvas_size`]. Returns the full pill's top left and pixels per
/// logical pixel, for hit testing.
fn draw(
    pixels: *mut u8,
    pill: &Pill,
    frame: &Frame,
    now: f32,
    tip: Option<&Tip>,
) -> (f32, f32, f32) {
    let s = pill.scale;
    let (width, height) = canvas_size(s);
    let canvas = Canvas::over_pixels(pixels, width, height);
    let k = s * frame.scale;
    let (w, h) = (frame.width * k, frame.height * k);
    let bottom = height as f32 - MARGIN * s;
    let centre = (width as f32 / 2.0, bottom - frame.height * s / 2.0);
    let shape = Rect {
        x: centre.0 - w / 2.0,
        y: centre.1 - h / 2.0,
        w,
        h,
    };
    let radius = ORB / 2.0 * k;

    // A soft shadow, then a faint glow.
    for step in 1..=7 {
        let grow = step as f32 * 1.7 * s;
        let shadow = Rect {
            y: shape.y + 3.0 * s,
            ..shape
        }
        .inflate(grow);
        canvas.round(shadow, radius + grow, 0x0C00_0000);
    }
    canvas.outline(
        shape.inflate(3.0 * s),
        radius + 3.0 * s,
        frame.look.glow,
        2.0 * s,
    );
    canvas.round(shape, radius, frame.look.fill);
    canvas.outline(shape, radius, 0xFF00_0000 | BORDER, s);

    // Content lays out on the full pill's grid, from the shape's top.
    let origin = (centre.0 - WIDTH / 2.0 * k, shape.y, k);
    canvas.clip_round(shape, radius);
    let view = View {
        canvas: &canvas,
        origin,
        centre: (centre.0, centre.1),
        pill,
        now,
    };
    if frame.old > 0.0 {
        view.content(pill.previous, frame.old);
    }
    view.content(pill.stage, frame.new);
    canvas.reset_clip();

    if let Some(tip) = tip {
        view.tip(tip, width as f32, s);
    }
    origin
}

/// Draws a stage's content.
struct View<'a> {
    canvas: &'a Canvas,
    /// The full pill's top left and pixels per logical pixel.
    origin: (f32, f32, f32),
    /// The shape's centre, for the orb's content.
    centre: (f32, f32),
    pill: &'a Pill,
    now: f32,
}

impl View<'_> {
    /// A point on the full pill's grid.
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        let (left, top, k) = self.origin;
        (left + x * k, top + y * k)
    }

    fn content(&self, stage: Stage, opacity: f32) {
        if opacity <= 0.0 {
            return;
        }
        let k = self.origin.2;
        let canvas = self.canvas;
        match stage {
            Stage::Hidden => {}
            Stage::Starting => {
                let breathe = 0.5 + 0.5 * (TAU * self.now / 0.7).cos();
                canvas.arc(
                    self.centre,
                    (6.0 + 1.2 * breathe) * k,
                    (0.0, 360.0),
                    alpha(ACCENT, (0.6 + 0.4 * breathe) * opacity),
                    2.0 * k,
                );
            }
            Stage::Failed => {
                let (x, y) = self.centre;
                let colour = alpha(ORANGE, opacity);
                canvas.lines(&[(x, y - 6.0 * k), (x, y + 1.5 * k)], colour, 2.4 * k);
                canvas.circle((x, y + 5.5 * k), 1.4 * k, colour);
            }
            Stage::Playing | Stage::Paused | Stage::CatchingUp => self.full(stage, opacity),
        }
    }

    fn full(&self, stage: Stage, opacity: f32) {
        let k = self.origin.2;
        let canvas = self.canvas;
        let bars = if stage == Stage::Playing {
            ACCENT
        } else {
            FAINT
        };
        for i in 0..BARS {
            let tall = bar_height(stage, self.pill.level, self.now, i) * k;
            let (x, y) = self.at(BARS_LEFT + i as f32 * BAR_PITCH, ROW / 2.0);
            let bar = Rect {
                x,
                y: y - tall / 2.0,
                w: BAR_WIDTH * k,
                h: tall,
            };
            canvas.round(bar, BAR_WIDTH / 2.0 * k, alpha(bars, opacity));
        }

        // The clock blinks while paused.
        let blink = if stage == Stage::Paused && self.now.rem_euclid(1.4) >= 0.7 {
            0.35
        } else {
            1.0
        };
        let (x, y) = self.at(CLOCK_LEFT, 0.0);
        let area = Rect {
            x,
            y,
            w: (CLOCK_WIDTH + 6.0) * k,
            h: ROW * k,
        };
        canvas.text(
            &self.pill.clock,
            area,
            13.0 * k,
            false,
            false,
            alpha(FAINT, opacity * blink),
        );

        for button in Button::ALL {
            let centre = self.at(button.x(), BUTTON_Y);
            let hovered = self.pill.hover == Some(button);
            if hovered {
                canvas.circle(centre, BUTTON_RADIUS * k, alpha(HOVER, opacity));
            }
            let rgb = if hovered || button == Button::PlayPause {
                INK
            } else {
                MUTED
            };
            let pressed = hovered && self.pill.pressed == Some(button);
            let size = if pressed { 16.0 } else { 18.0 };
            self.icon(
                button,
                stage == Stage::Paused,
                centre,
                size * k,
                alpha(rgb, opacity),
            );
        }
        let (x, top) = self.at(SEPARATOR_X, BUTTON_Y - 8.0);
        canvas.round(
            Rect {
                x: x - 0.5 * k,
                y: top,
                w: k,
                h: 16.0 * k,
            },
            0.5 * k,
            alpha(BORDER, opacity),
        );
    }

    /// A button's icon, drawn on a 24-unit grid `size` pixels square.
    fn icon(&self, button: Button, paused: bool, (cx, cy): (f32, f32), size: f32, colour: u32) {
        let u = size / 24.0;
        let p = |x: f32, y: f32| (cx + (x - 12.0) * u, cy + (y - 12.0) * u);
        let canvas = self.canvas;
        let rect = |x: f32, y: f32, w: f32, h: f32| {
            let (left, top) = p(x, y);
            Rect {
                x: left,
                y: top,
                w: w * u,
                h: h * u,
            }
        };
        match button {
            Button::PlayPause if paused => {
                let points = [p(8.0, 5.5), p(19.0, 12.0), p(8.0, 18.5)];
                canvas.polygon(&points, colour);
                canvas.lines(
                    &[points[0], points[1], points[2], points[0]],
                    colour,
                    1.6 * u,
                );
            }
            Button::PlayPause => {
                canvas.round(rect(6.5, 5.0, 4.0, 14.0), 1.2 * u, colour);
                canvas.round(rect(13.5, 5.0, 4.0, 14.0), 1.2 * u, colour);
            }
            Button::Stop => canvas.round(rect(6.5, 6.5, 11.0, 11.0), 2.2 * u, colour),
            Button::Back | Button::Forward => {
                // A circular arrow round a 5. Back turns anticlockwise.
                let (end, sweep) = if button == Button::Back {
                    (8.0, 270.0)
                } else {
                    (16.0, -270.0)
                };
                let side = if button == Button::Back { 2.5 } else { -2.5 };
                let width = 1.8 * u;
                canvas.lines(&[p(end, 6.5), p(12.0, 6.5)], colour, width);
                canvas.arc(p(12.0, 13.0), 6.5 * u, (270.0, sweep), colour, width);
                canvas.lines(
                    &[p(end + side, 4.0), p(end, 6.5), p(end + side, 9.0)],
                    colour,
                    width,
                );
                canvas.text("5", rect(6.0, 8.0, 12.0, 10.5), 8.5 * u, true, true, colour);
            }
        }
    }

    /// The tooltip, above the pill and centred on its button.
    fn tip(&self, tip: &Tip, canvas_width: f32, s: f32) {
        let canvas = self.canvas;
        let size = 12.0 * s;
        let label = canvas.text_width(&tip.label, size, false);
        let key = if tip.key.is_empty() {
            0.0
        } else {
            canvas.text_width(&tip.key, size, false)
        };
        let gap = if key > 0.0 { 6.0 * s } else { 0.0 };
        let pad = 9.0 * s;
        let w = pad + label + gap + key + pad;
        let h = 24.0 * s;
        let (x, _) = self.at(tip.x, 0.0);
        // Not `clamp`: a tooltip wider than the window would make it panic.
        let left = (x - w / 2.0).min(canvas_width - 4.0 * s - w).max(4.0 * s);
        let area = Rect {
            x: left,
            y: self.origin.1 - 8.0 * s - h,
            w,
            h,
        };
        canvas.round(area, 6.0 * s, alpha(TIP, tip.opacity));
        canvas.outline(area, 6.0 * s, alpha(BORDER, tip.opacity), s);
        let text = |x: f32, w: f32, text: &str, rgb: u32| {
            let area = Rect {
                x,
                w: w + 2.0 * s,
                ..area
            };
            canvas.text(text, area, size, false, false, alpha(rgb, tip.opacity));
        };
        text(left + pad, label, &tip.label, INK);
        if key > 0.0 {
            text(left + pad + label + gap, key, &tip.key, FAINT);
        }
    }
}

/// Bottom centre of the work area on the monitor being read from, and its
/// pixels per logical pixel.
fn anchor() -> ((i32, i32), f32) {
    let mut info: MONITORINFO = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    let (mut dpi_x, mut dpi_y) = (96, 96);
    unsafe {
        let monitor = MonitorFromWindow(GetForegroundWindow(), MONITOR_DEFAULTTOPRIMARY);
        GetMonitorInfoW(monitor, &mut info);
        GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
    }
    let scale = dpi_x.max(96) as f32 / 96.0;
    let work = info.rcWork;
    let bottom = work.bottom - (ABOVE_TASKBAR * scale).round() as i32;
    (((work.left + work.right) / 2, bottom), scale)
}

/// How far up to move so this pill sits above Inlay's, if Inlay's shows
/// in the same place.
fn inlay_lift(pill: &Pill) -> f32 {
    let class: Vec<u16> = "InlayPill\0".encode_utf16().collect();
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    unsafe {
        let inlay = FindWindowW(class.as_ptr(), null());
        if inlay.is_null() || IsWindowVisible(inlay) == 0 || GetWindowRect(inlay, &mut rect) == 0 {
            return 0.0;
        }
    }
    let s = pill.scale;
    let (x, bottom) = pill.anchor;
    let half = (WIDTH / 2.0 * s) as i32;
    let overlaps =
        rect.left < x + half && rect.right > x - half && rect.bottom > bottom - (HEIGHT * s) as i32;
    if !overlaps {
        return 0.0;
    }
    // Inlay's window has a 16 px margin round its pill. Leave a gap of 10.
    let inlay_top = rect.top as f32 + MARGIN * s;
    (bottom as f32 - (inlay_top - 10.0 * s)).max(0.0)
}

/// Draws a frame and puts it on screen.
fn present(window: HWND, pill: &mut Pill, frame: &Frame, now: f32, tip: Option<&Tip>) {
    let (width, height) = canvas_size(pill.scale);
    unsafe {
        let screen = GetDC(null_mut());
        let memory = CreateCompatibleDC(screen);
        let mut header: BITMAPINFO = std::mem::zeroed();
        header.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(memory, &header, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(memory);
            ReleaseDC(null_mut(), screen);
            return;
        }
        let pixels = bits.cast::<u8>();
        std::ptr::write_bytes(pixels, 0, (width * height * 4) as usize);
        let previous = SelectObject(memory, bitmap);
        pill.origin = draw(pixels, pill, frame, now, tip);

        let (centre, bottom) = pill.anchor;
        let position = POINT {
            x: centre - width / 2,
            y: bottom + (MARGIN * pill.scale).round() as i32 - height - pill.lift.round() as i32,
        };
        let size = SIZE {
            cx: width,
            cy: height,
        };
        let source = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: (frame.alpha * 255.0).round() as u8,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        UpdateLayeredWindow(
            window, screen, &position, &size, memory, &source, 0, &blend, ULW_ALPHA,
        );
        SelectObject(memory, previous);
        DeleteObject(bitmap);
        DeleteDC(memory);
        ReleaseDC(null_mut(), screen);
    }
}

unsafe fn create_window() -> HWND {
    super::canvas::start();
    let class_name: Vec<u16> = "VoxtrayPill\0".encode_utf16().collect();
    let title: Vec<u16> = "Voxtray reading\0".encode_utf16().collect();
    unsafe {
        let instance = GetModuleHandleW(null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            ..std::mem::zeroed()
        };
        RegisterClassW(&class);
        CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            class_name.as_ptr(),
            title.as_ptr(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    }
}

/// The client point in `lparam`, in window pixels.
fn point(lparam: LPARAM) -> (f32, f32) {
    let x = (lparam & 0xFFFF) as i16 as f32;
    let y = ((lparam >> 16) & 0xFFFF) as i16 as f32;
    (x, y)
}

/// The button under a window point, if the pill takes clicks now.
fn hit(pill: &Pill, (x, y): (f32, f32), now: f32) -> Option<Button> {
    if !pill.interactive(now) {
        return None;
    }
    let (left, top, k) = pill.origin;
    button_at((x - left) / k, (y - top) / k)
}

/// Whether a window point is on the orb.
fn on_orb(pill: &Pill, (x, y): (f32, f32)) -> bool {
    let (left, top, k) = pill.origin;
    let (cx, cy) = (left + WIDTH / 2.0 * k, top + ORB / 2.0 * k);
    (x - cx).hypot(y - cy) <= ORB / 2.0 * k
}

fn mouse(window: HWND, message: u32, lparam: LPARAM) {
    let now = clock();
    let mut guard = lock(&PILL_STATE);
    let Some(pill) = guard.as_mut() else { return };
    let at = point(lparam);
    let mut action = None;
    match message {
        WM_MOUSEMOVE => {
            let over = hit(pill, at, now);
            if over != pill.hover {
                pill.hover = over;
                pill.hover_since = now;
            }
            if !pill.tracking {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: window,
                    dwHoverTime: 0,
                };
                pill.tracking = unsafe { TrackMouseEvent(&mut track) } != 0;
            }
        }
        WM_MOUSELEAVE => {
            pill.tracking = false;
            pill.hover = None;
            pill.pressed = None;
        }
        WM_LBUTTONDOWN => pill.pressed = hit(pill, at, now),
        WM_LBUTTONUP => {
            let over = hit(pill, at, now);
            if let Some(button) = pill.pressed.take().filter(|&b| Some(b) == over) {
                action = Some(Action::Press(button));
                // The tooltip names the old state until the mouse moves again.
                pill.hover_since = now;
            } else if pill.leaving_at.is_none() && on_orb(pill, at) {
                action = match pill.stage {
                    Stage::Starting => Some(Action::Cancel),
                    Stage::Failed => Some(Action::ShowDetails),
                    _ => None,
                };
            }
        }
        _ => {}
    }
    drop(guard);
    if let (Some(action), Some(hooks)) = (action, HOOKS.get()) {
        (hooks.act)(action);
    }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_TIMER | WM_REFRESH => {
            update();
            0
        }
        WM_MOUSEMOVE | WM_MOUSELEAVE | WM_LBUTTONDOWN | WM_LBUTTONUP => {
            mouse(window, message, lparam);
            0
        }
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        _ => unsafe { DefWindowProcW(window, message, wparam, lparam) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::Graphics::GdiPlus::{
        GdipCreateBitmapFromScan0, GdipDisposeImage, GdipSaveImageToFile,
    };

    /// A settled pill at `stage`, drawn at twice 96 DPI.
    fn settled(stage: Stage, now: f32) -> Pill {
        let mut pill = Pill::new(stage, now);
        pill.scale = 2.0;
        pill.shown_at = now - 1.0;
        pill.level = 0.8;
        pill.clock = "0:12".into();
        pill
    }

    fn render(pill: &mut Pill, now: f32, tip: Option<&Tip>) -> (Vec<u8>, i32, i32) {
        let (width, height) = canvas_size(pill.scale);
        let mut pixels = vec![0u8; (width * height * 4) as usize];
        let frame = frame(pill, now);
        pill.origin = draw(pixels.as_mut_ptr(), pill, &frame, now, tip);
        (pixels, width, height)
    }

    fn alpha_at(pixels: &[u8], width: i32, (x, y): (f32, f32)) -> u8 {
        pixels[((y as i32 * width + x as i32) * 4 + 3) as usize]
    }

    #[test]
    fn the_full_pill_is_opaque_and_the_tip_space_lets_clicks_through() {
        let now = 10.0;
        let mut pill = settled(Stage::Playing, now);
        let (pixels, width, _) = render(&mut pill, now, None);
        let (left, top, k) = pill.origin;
        for button in Button::ALL {
            let at = (left + button.x() * k, top + BUTTON_Y * k);
            assert_eq!(alpha_at(&pixels, width, at), 255, "{button:?}");
        }
        // Above the pill, where a tooltip would go, nothing is drawn.
        let above = (width as f32 / 2.0, top - 20.0 * k);
        assert_eq!(alpha_at(&pixels, width, above), 0);
    }

    #[test]
    fn clicks_find_buttons_where_they_are_drawn() {
        let now = 10.0;
        let mut pill = settled(Stage::Paused, now);
        render(&mut pill, now, None);
        let (left, top, k) = pill.origin;
        for button in Button::ALL {
            let at = (left + button.x() * k, top + BUTTON_Y * k);
            assert_eq!(hit(&pill, at, now), Some(button));
        }
        assert_eq!(hit(&pill, (left + 30.0 * k, top + 10.0 * k), now), None);
        // Mid-morph, nothing takes clicks yet.
        pill.go(Stage::Starting, now);
        assert_eq!(
            hit(
                &pill,
                (left + Button::Stop.x() * k, top + BUTTON_Y * k),
                now
            ),
            None
        );
    }

    #[test]
    fn a_tooltip_wider_than_the_window_still_draws() {
        let now = 10.0;
        let mut pill = settled(Stage::Paused, now);
        pill.hover = Some(Button::PlayPause);
        let tip = Tip {
            label: "Resume".into(),
            key: "Ctrl+Alt+Shift+Super+PrintScreen+Ctrl+Alt+Shift+Super".into(),
            x: Button::PlayPause.x(),
            opacity: 1.0,
        };
        render(&mut pill, now, Some(&tip));
    }

    #[test]
    fn leaving_shrinks_into_the_orb_and_fades() {
        let now = 10.0;
        let mut pill = settled(Stage::Playing, now);
        pill.go(Stage::Hidden, now);
        pill.leaving_at = Some(now + MORPH * 0.6);
        let middle = frame(&pill, now + MORPH * 0.6);
        assert!(middle.width < WIDTH && middle.alpha == 1.0);
        let gone = frame(&pill, now + MORPH * 0.6 + FADE);
        assert!(gone.alpha < 0.001, "{}", gone.alpha);
        assert!((gone.width - ORB).abs() < 0.01, "{}", gone.width);
    }

    /// Writes PNGs of each state to `VOXTRAY_PILL_SHOTS`, for the PR.
    #[test]
    #[ignore = "writes images; set VOXTRAY_PILL_SHOTS"]
    fn render_each_state() {
        let dir = std::env::var("VOXTRAY_PILL_SHOTS").expect("VOXTRAY_PILL_SHOTS");
        let now = 10.0;
        let tip = Tip {
            label: "Back 5 s".into(),
            key: "Ctrl+Alt+←".into(),
            x: Button::Back.x(),
            opacity: 1.0,
        };
        let mut morphing = settled(Stage::Starting, now);
        morphing.go(Stage::Playing, now);
        let mut leaving = settled(Stage::Playing, now);
        leaving.go(Stage::Hidden, now);
        leaving.leaving_at = Some(now + MORPH * 0.6);
        let mut hovered = settled(Stage::Playing, now);
        hovered.hover = Some(Button::Back);
        let shots: Vec<(&str, Pill, f32, Option<&Tip>)> = vec![
            ("1-starting", settled(Stage::Starting, now), now, None),
            ("2-opening", morphing, now + MORPH_OPEN * 0.45, None),
            ("3-playing", settled(Stage::Playing, now), now, None),
            ("4-paused", settled(Stage::Paused, now), now, None),
            ("5-catching-up", settled(Stage::CatchingUp, now), now, None),
            ("6-hover", hovered, now, Some(&tip)),
            ("7-failed", settled(Stage::Failed, now), now, None),
            ("8-leaving", leaving, now + MORPH * 0.4, None),
        ];
        // PNG's encoder in GDI+.
        let png = windows_sys::core::GUID::from_u128(0x557cf406_1a04_11d3_9a73_0000f81ef32e);
        for (name, mut pill, at, tip) in shots {
            let (mut pixels, width, height) = render(&mut pill, at, tip);
            let path: Vec<u16> = format!("{dir}\\{name}.png\0").encode_utf16().collect();
            unsafe {
                let mut bitmap = null_mut();
                GdipCreateBitmapFromScan0(
                    width,
                    height,
                    width * 4,
                    0xE200B,
                    pixels.as_mut_ptr(),
                    &mut bitmap,
                );
                let status = GdipSaveImageToFile(bitmap.cast(), path.as_ptr(), &png, null());
                GdipDisposeImage(bitmap.cast());
                assert_eq!(status, 0, "{name}");
            }
        }
    }
}

/// Tests that put the pill on the real desktop and click it with the mouse.
/// They move the pointer, so run them only when nobody is using the machine.
#[cfg(test)]
mod desktop_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        MOUSEINPUT,
    };

    fn click() {
        let input = |flags| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let inputs = [input(MOUSEEVENTF_LEFTDOWN), input(MOUSEEVENTF_LEFTUP)];
        unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
    }

    #[test]
    #[ignore = "shows the pill on the desktop and clicks it with the real mouse"]
    fn clicking_the_pill_presses_buttons_without_taking_focus() {
        let showing = Arc::new(AtomicBool::new(true));
        let pressed = Arc::new(Mutex::new(Vec::new()));
        let (s, p) = (showing.clone(), pressed.clone());
        install(Hooks {
            snapshot: Box::new(move || Snapshot {
                stage: if s.load(Ordering::SeqCst) {
                    Stage::Playing
                } else {
                    Stage::Hidden
                },
                level: 0.6,
                position: Duration::from_secs(12),
            }),
            act: Box::new(move |action| lock(&p).push(action)),
            tooltip: Box::new(|_, _| ("Pause".into(), "Right Ctrl".into())),
        })
        .unwrap();
        let foreground = unsafe { GetForegroundWindow() };
        refresh();
        std::thread::sleep(Duration::from_millis(800));
        let window = WINDOW.load(Ordering::SeqCst);
        assert_ne!(unsafe { IsWindowVisible(window) }, 0, "the pill shows");

        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe { GetWindowRect(window, &mut rect) };
        let (left, top, k) = lock(&PILL_STATE).as_ref().unwrap().origin;
        let mut cursor = POINT { x: 0, y: 0 };
        unsafe { GetCursorPos(&mut cursor) };
        for button in [Button::PlayPause, Button::Forward] {
            let x = rect.left + (left + button.x() * k) as i32;
            let y = rect.top + (top + BUTTON_Y * k) as i32;
            unsafe { SetCursorPos(x, y) };
            std::thread::sleep(Duration::from_millis(150));
            click();
            std::thread::sleep(Duration::from_millis(300));
        }
        // Hover the back button long enough for its tooltip, for a screenshot.
        let x = rect.left + (left + Button::Back.x() * k) as i32;
        unsafe { SetCursorPos(x, rect.top + (top + BUTTON_Y * k) as i32) };
        std::thread::sleep(Duration::from_millis(900));
        if let Ok(dir) = std::env::var("VOXTRAY_PILL_SHOTS") {
            screenshot(rect, &format!("{dir}\\desktop-hover.png"));
        }
        unsafe { SetCursorPos(cursor.x, cursor.y) };

        assert_eq!(
            *lock(&pressed),
            [
                Action::Press(Button::PlayPause),
                Action::Press(Button::Forward)
            ]
        );
        assert_eq!(
            unsafe { GetForegroundWindow() },
            foreground,
            "focus stays put"
        );

        showing.store(false, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(unsafe { IsWindowVisible(window) }, 0, "the pill hides");
    }

    /// Saves the screen inside `rect` as a PNG.
    fn screenshot(rect: RECT, path: &str) {
        use windows_sys::Win32::Graphics::GdiPlus::{
            GdipCreateBitmapFromHBITMAP, GdipDisposeImage, GdipSaveImageToFile,
        };
        let (w, h) = (rect.right - rect.left, rect.bottom - rect.top);
        let png = windows_sys::core::GUID::from_u128(0x557cf406_1a04_11d3_9a73_0000f81ef32e);
        let path: Vec<u16> = format!("{path}\0").encode_utf16().collect();
        unsafe {
            let screen = GetDC(null_mut());
            let memory = CreateCompatibleDC(screen);
            let bitmap = CreateCompatibleBitmap(screen, w, h);
            let previous = SelectObject(memory, bitmap);
            BitBlt(memory, 0, 0, w, h, screen, rect.left, rect.top, SRCCOPY);
            SelectObject(memory, previous);
            let mut image = null_mut();
            GdipCreateBitmapFromHBITMAP(bitmap, null_mut(), &mut image);
            GdipSaveImageToFile(image.cast(), path.as_ptr(), &png, null());
            GdipDisposeImage(image.cast());
            DeleteObject(bitmap);
            DeleteDC(memory);
            ReleaseDC(null_mut(), screen);
        }
    }
}
