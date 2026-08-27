//! imgui platform layer for Switch: feeds gamepad state into imgui's nav
//! system each frame. Without this the UI renders but cannot be operated —
//! there is no mouse or keyboard, and dear imgui only navigates by gamepad
//! when the backend actually submits gamepad key events.
//!
//! Owns its own libnx `PadState` (a per-caller snapshot of HID shared memory;
//! reading it here does not disturb the engine's `SwitchGamepadInput`, which
//! keeps a separate one). The struct mirror and the export-vs-inline situation
//! are the same as in `input/gamepad/switch.rs`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::{cell::RefCell, rc::Rc};

use imgui::{BackendFlags, ConfigFlags, Context, Key, MouseButton};

use crate::application::Platform;

/// Raw `buttons_cur` of the most recent frame, for the on-screen input
/// diagnostic overlay (single-threaded writer; relaxed is fine).
pub static LAST_PAD_BUTTONS: AtomicU64 = AtomicU64::new(0);

const NPAD_A: u64 = 1 << 0;
const NPAD_B: u64 = 1 << 1;
const NPAD_X: u64 = 1 << 2;
const NPAD_Y: u64 = 1 << 3;
const NPAD_L: u64 = 1 << 6;
const NPAD_R: u64 = 1 << 7;
const NPAD_LEFT: u64 = 1 << 12;
const NPAD_UP: u64 = 1 << 13;
const NPAD_RIGHT: u64 = 1 << 14;
const NPAD_DOWN: u64 = 1 << 15;

const PAD_ANY_ID_MASK: u64 = 0x1_0001_00FF;
const STICK_RANGE: f32 = 32767.0;
const STICK_DEADZONE: f32 = 0.15;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct HidAnalogStickState {
    x: i32,
    y: i32,
}

/// Field-for-field mirror of libnx's `PadState` (pad.h).
#[repr(C)]
struct PadState {
    id_mask: u8,
    active_id_mask: u8,
    read_handheld: bool,
    active_handheld: bool,
    style_set: u32,
    attributes: u32,
    buttons_cur: u64,
    buttons_old: u64,
    sticks: [HidAnalogStickState; 2],
    gc_triggers: [u32; 2],
}

/// Mirror of libnx `HidTouchState` (hid.h) — 40 bytes.
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct HidTouchState {
    delta_time: u64,
    attributes: u32,
    finger_id: u32,
    x: u32,
    y: u32,
    diameter_x: u32,
    diameter_y: u32,
    rotation_angle: u32,
    reserved: u32,
}

/// Mirror of libnx `HidTouchScreenState` (hid.h) — 656 bytes.
#[repr(C)]
struct HidTouchScreenState {
    sampling_number: u64,
    count: i32,
    reserved: u32,
    touches: [HidTouchState; 16],
}

unsafe extern "C" {
    fn padInitializeWithMask(pad: *mut PadState, mask: u64);
    fn padUpdate(pad: *mut PadState);
    fn hidInitializeTouchScreen();
    fn hidGetTouchScreenStates(states: *mut HidTouchScreenState, count: usize) -> usize;
}

/// Touch panel coordinate space (fixed, independent of dock state).
const TOUCH_W: f32 = 1280.0;
const TOUCH_H: f32 = 720.0;

pub struct ImguiPlatform {
    context: Rc<RefCell<Context>>,
    pad: Box<PadState>,
    touch_down: bool,
}

impl ImguiPlatform {
    pub fn new(context: Rc<RefCell<Context>>, _platform: &mut Platform) -> Rc<RefCell<Self>> {
        {
            let mut ctx = context.borrow_mut();
            Self::set_display_size(&mut ctx);
            let io = ctx.io_mut();
            io.config_flags |= ConfigFlags::NAV_ENABLE_GAMEPAD;
            io.backend_flags |= BackendFlags::HAS_GAMEPAD;
        }

        // padConfigureInput is process-global and already issued by the
        // engine's SwitchGamepadInput; issuing it twice is harmless, but this
        // snapshot only needs its own initialize.
        let mut pad = Box::new(PadState {
            id_mask: 0,
            active_id_mask: 0,
            read_handheld: false,
            active_handheld: false,
            style_set: 0,
            attributes: 0,
            buttons_cur: 0,
            buttons_old: 0,
            sticks: [HidAnalogStickState::default(); 2],
            gc_triggers: [0; 2],
        });
        unsafe {
            padInitializeWithMask(pad.as_mut(), PAD_ANY_ID_MASK);
            hidInitializeTouchScreen();
        }

        Rc::new(RefCell::new(Self {
            context,
            pad,
            touch_down: false,
        }))
    }

    /// Called by `ImguiContext::draw_ui` before `Context::frame()` — the
    /// window dear imgui expects input events in.
    pub fn new_frame(&mut self) {
        unsafe { padUpdate(self.pad.as_mut()) };
        let cur = self.pad.buttons_cur;
        LAST_PAD_BUTTONS.store(cur, Ordering::Relaxed);
        // Bring-up diagnostic: log raw pad transitions so input delivery can
        // be verified from the log file alone (no screen access needed).
        if cur != self.pad.buttons_old {
            log::info!("pad buttons: {:#018x}", cur);
        }
        let old = self.pad.buttons_old;
        let sticks = self.pad.sticks;

        let mut ctx = self.context.borrow_mut();
        let io = ctx.io_mut();

        // Edge-triggered digital events. Nintendo's A (physical east) is the
        // platform confirm — map it to imgui's activate (FaceDown), B to
        // cancel (FaceRight), matching Switch conventions rather than labels.
        for (bit, key) in [
            (NPAD_A, Key::GamepadFaceDown),
            (NPAD_B, Key::GamepadFaceRight),
            (NPAD_X, Key::GamepadFaceUp),
            (NPAD_Y, Key::GamepadFaceLeft),
            (NPAD_UP, Key::GamepadDpadUp),
            (NPAD_DOWN, Key::GamepadDpadDown),
            (NPAD_LEFT, Key::GamepadDpadLeft),
            (NPAD_RIGHT, Key::GamepadDpadRight),
            (NPAD_L, Key::GamepadL1),
            (NPAD_R, Key::GamepadR1),
        ] {
            let down = cur & bit != 0;
            if down != (old & bit != 0) {
                io.add_key_event(key, down);
            }
        }

        // Left stick as analog nav (scrolling lists). imgui wants one event
        // per direction with the magnitude in [0, 1].
        let lx = (sticks[0].x as f32 / STICK_RANGE).clamp(-1.0, 1.0);
        let ly = (sticks[0].y as f32 / STICK_RANGE).clamp(-1.0, 1.0);
        let dz = |v: f32| if v.abs() < STICK_DEADZONE { 0.0 } else { v.abs() };
        io.add_key_analog_event(Key::GamepadLStickLeft, lx < -STICK_DEADZONE, dz(lx.min(0.0)));
        io.add_key_analog_event(Key::GamepadLStickRight, lx > STICK_DEADZONE, dz(lx.max(0.0)));
        // libnx +Y is up; imgui's LStickUp expects "stick pushed up".
        io.add_key_analog_event(Key::GamepadLStickUp, ly > STICK_DEADZONE, dz(ly.max(0.0)));
        io.add_key_analog_event(Key::GamepadLStickDown, ly < -STICK_DEADZONE, dz(ly.min(0.0)));

        // Touch screen -> imgui mouse. The panel reports in its fixed
        // 1280x720 space; scale to the imgui display. First finger only —
        // imgui is a pointer UI, multitouch has no meaning to it. (Ryujinx
        // forwards host mouse clicks as touch, so this also gives the
        // emulator a pointer.)
        let mut ts = HidTouchScreenState {
            sampling_number: 0,
            count: 0,
            reserved: 0,
            touches: [HidTouchState::default(); 16],
        };
        let got = unsafe { hidGetTouchScreenStates(&mut ts, 1) };
        let down = got > 0 && ts.count > 0;
        if down {
            let t = &ts.touches[0];
            let [dw, dh] = io.display_size;
            io.add_mouse_pos_event([
                t.x as f32 * (dw / TOUCH_W),
                t.y as f32 * (dh / TOUCH_H),
            ]);
        }
        if down != self.touch_down {
            io.add_mouse_button_event(MouseButton::Left, down);
            self.touch_down = down;
        }
    }

    pub fn prepare_render(&self, _ui: &imgui::Ui) {}

    fn set_display_size(context: &mut Context) {
        // Docked resolution; the switchgl backend does not resize the surface.
        context.io_mut().display_size = [1920., 1080.];
    }
}
