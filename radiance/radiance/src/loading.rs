//! Loading-screen progress while a scene is built synchronously.
//!
//! The original (`SceneMgr::LoadScene` → `UILoadingFrm`) draws the loading
//! screen once and then builds the scene in the same call, so its bar never
//! moves. Here the code building the scene reports progress, and while a
//! load is running ([`begin`]..[`end`]) every report (at most one per 0.1 s)
//! is handed to the rendering backend's pump ([`set_pump`]), which draws and
//! presents a loading frame on its own. Backends that register no pump
//! (desktop Vulkan, where scenes load in a blink) make all of this a no-op.
//!
//! Progress comes in stages: [`stage`]`(from, to)` starts one and shows
//! `from`; [`tick`] calls from inside long loops ease the bar towards `to`
//! with the time spent in the stage, so it keeps moving through steps that
//! can't report a fraction of their own (one big model file).

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use image::RgbaImage;

/// Loading-screen art, drawn at native size in the 800×600 UI space.
pub struct LoadingArt {
    /// Bar background (`ui/Loading/loading_1.tga`).
    pub bar: RgbaImage,
    /// Bar fill (`loading_2.tga`).
    pub fill: RgbaImage,
    /// Ornament riding the bar's head, swapped every 0.3 s
    /// (`loading_3.tga` / `loading_5.tga`).
    pub ornament: [RgbaImage; 2],
}

#[derive(Clone, Copy)]
struct Stage {
    from: f32,
    to: f32,
    since: Instant,
}

type Pump = Box<dyn FnMut(f32, f32)>;

thread_local! {
    static ART: RefCell<Option<Rc<LoadingArt>>> = const { RefCell::new(None) };
    static PUMP: RefCell<Option<Pump>> = const { RefCell::new(None) };
    static STARTED: Cell<Option<Instant>> = const { Cell::new(None) };
    static STAGE: Cell<Option<Stage>> = const { Cell::new(None) };
    static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
    static SHOWN: Cell<f32> = const { Cell::new(0.) };
}

/// Each loading frame is time taken from the load itself; ten a second
/// keeps the bar visibly moving.
const INTERVAL: Duration = Duration::from_millis(100);
/// Time constant of the ease towards a stage's end (seconds).
const EASE_SEC: f32 = 0.8;

pub fn set_art(art: LoadingArt) {
    ART.with(|a| *a.borrow_mut() = Some(Rc::new(art)));
}

pub fn art() -> Option<Rc<LoadingArt>> {
    ART.with(|a| a.borrow().clone())
}

/// Register the backend's frame pump: it gets the bar fraction (0..=1) and
/// the seconds since the load began (for the ornament swap).
pub fn set_pump(pump: Pump) {
    PUMP.with(|p| *p.borrow_mut() = Some(pump));
}

/// A load starts: reports from here to [`end`] draw frames.
pub fn begin() {
    STARTED.with(|s| s.set(Some(Instant::now())));
    STAGE.with(|s| s.set(None));
    LAST.with(|l| l.set(None));
    SHOWN.with(|s| s.set(0.));
}

/// The load is done: shows the full bar once and stops drawing.
pub fn end() {
    stage(1., 1.);
    present(true);
    STARTED.with(|s| s.set(None));
}

/// Enter a stage spanning `from..to` of the bar.
pub fn stage(from: f32, to: f32) {
    STAGE.with(|s| {
        s.set(Some(Stage {
            from,
            to,
            since: Instant::now(),
        }))
    });
    present(false);
}

/// Somewhere inside a long step: redraw if it's time.
pub fn tick() {
    present(false);
}

fn present(force: bool) {
    let Some(started) = STARTED.with(|s| s.get()) else {
        return;
    };
    let now = Instant::now();
    if !force && LAST.with(|l| l.get()).is_some_and(|t| now - t < INTERVAL) {
        return;
    }
    let fraction = STAGE.with(|s| s.get()).map_or(0., |s| {
        let k = 1. - (-(now - s.since).as_secs_f32() / EASE_SEC).exp();
        s.from + (s.to - s.from) * k
    });
    // Never step backwards.
    let fraction = SHOWN.with(|s| {
        let f = fraction.clamp(0., 1.).max(s.get());
        s.set(f);
        f
    });
    LAST.with(|l| l.set(Some(now)));
    let Some(mut pump) = PUMP.with(|p| p.borrow_mut().take()) else {
        return;
    };
    pump(fraction, (now - started).as_secs_f32());
    PUMP.with(|p| *p.borrow_mut() = Some(pump));
}
