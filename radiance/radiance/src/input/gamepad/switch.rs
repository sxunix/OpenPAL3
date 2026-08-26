//! Only `padConfigureInput`, `padInitializeWithMask` and `padUpdate` are real
//! exports in libnx; `padInitializeAny`, `padGetButtons` and `padGetStickPos`
//! are `NX_INLINE`/`NX_CONSTEXPR` header helpers with no symbol behind them
//! (the first `.nro` linked only because this module was dead code at the
//! time). The struct is therefore mirrored field-for-field from
//! `switch/runtime/pad.h` and the accessors reimplemented here.

use crate::input::{Axis, AxisState, Key, KeyState};

/// libnx `HidNpadButton` bits. Only the ones this mapping uses are declared.
const NPAD_A: u64 = 1 << 0;
const NPAD_B: u64 = 1 << 1;
const NPAD_X: u64 = 1 << 2;
const NPAD_Y: u64 = 1 << 3;
const NPAD_LEFT: u64 = 1 << 12;
const NPAD_UP: u64 = 1 << 13;
const NPAD_RIGHT: u64 = 1 << 14;
const NPAD_DOWN: u64 = 1 << 15;

/// `PAD_ANY_ID_MASK` from pad.h — accept every controller input source.
const PAD_ANY_ID_MASK: u64 = 0x1_0001_00FF;

/// `HidNpadStyleSet` — accept every controller layout so the same build works
/// docked with Joy-Cons detached and handheld.
const STYLE_SET_ALL: u32 = 0xFFFF_FFFF;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct HidAnalogStickState {
    x: i32,
    y: i32,
}

/// Field-for-field mirror of libnx's `PadState` (pad.h). 56 bytes, align 8.
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

unsafe extern "C" {
    fn padConfigureInput(max_players: u32, style_set: u32);
    fn padInitializeWithMask(pad: *mut PadState, mask: u64);
    fn padUpdate(pad: *mut PadState);
}

/// Sticks report roughly +/-32768 on each axis.
const STICK_RANGE: f32 = 32767.0;
/// Radial deadzone; the engine treats the stick as a direction vector, so a
/// small centre wobble would otherwise creep the camera.
const STICK_DEADZONE: f32 = 0.15;

pub struct SwitchGamepadInput {
    pad: Box<PadState>,
}

impl SwitchGamepadInput {
    pub fn new() -> Self {
        // padConfigureInput takes care of hidInitializeNpad and the
        // supported-style/id registration internally.
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
            padConfigureInput(1, STYLE_SET_ALL);
            padInitializeWithMask(pad.as_mut(), PAD_ANY_ID_MASK);
        }
        Self { pad }
    }

    pub fn process_message(&mut self, states: &mut [KeyState], axis_states: &mut [AxisState]) {
        unsafe {
            padUpdate(self.pad.as_mut());
        }
        let buttons = self.pad.buttons_cur;

        // Nintendo's A/B and X/Y are mirrored relative to the
        // south/east/west/north layout the engine speaks, so map by
        // physical position rather than by letter.
        set_key_state(states, Key::GamePadEast, buttons & NPAD_A != 0);
        set_key_state(states, Key::GamePadSouth, buttons & NPAD_B != 0);
        set_key_state(states, Key::GamePadNorth, buttons & NPAD_X != 0);
        set_key_state(states, Key::GamePadWest, buttons & NPAD_Y != 0);

        set_key_state(states, Key::GamePadDPadUp, buttons & NPAD_UP != 0);
        set_key_state(states, Key::GamePadDPadDown, buttons & NPAD_DOWN != 0);
        set_key_state(states, Key::GamePadDPadLeft, buttons & NPAD_LEFT != 0);
        set_key_state(states, Key::GamePadDPadRight, buttons & NPAD_RIGHT != 0);

        let left = self.pad.sticks[0];
        let right = self.pad.sticks[1];

        axis_states[Axis::LeftStickX as usize].set_value(normalize(left.x));
        // libnx reports +Y as up; the engine wants +Y down, as on Vita.
        axis_states[Axis::LeftStickY as usize].set_value(-normalize(left.y));
        axis_states[Axis::RightStickX as usize].set_value(normalize(right.x));
        axis_states[Axis::RightStickY as usize].set_value(-normalize(right.y));
    }
}

fn normalize(raw: i32) -> f32 {
    let v = (raw as f32 / STICK_RANGE).clamp(-1.0, 1.0);
    if v.abs() < STICK_DEADZONE { 0.0 } else { v }
}

fn set_key_state(states: &mut [KeyState], key: Key, down: bool) {
    if !states[key as usize].is_down() && down {
        states[key as usize].set_pressed(true);
    } else if states[key as usize].is_down() && !down {
        states[key as usize].set_released(true);
    }

    states[key as usize].set_down(down);
}
