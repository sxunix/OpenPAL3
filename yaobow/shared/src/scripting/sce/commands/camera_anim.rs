//! The camera moves that take time: 32 推镜头, 33 环绕, 35 摇镜.
//!
//! All three hand a task object (`DAT_00d8e4c0`) a start value, a target,
//! a duration and a curve, then block the script on it (`FUN_00434c40`).
//! They differ only in what they animate and which end of the polar model
//! is held still while it runs:
//!
//! | op | setup | type | animates | held |
//! |---|---|---|---|---|
//! | 32 | `FUN_0041cee0` | 1 | distance (`+0x1c` ← target, `+0x20` ← `+0x1d4`) | look-at |
//! | 33 | `FUN_0041cf20` | 2 | yaw + pitch (`+0x24`/`+0x2c`, start from `+0x1c8`/`+0x1c4`) | look-at |
//! | 35 | `FUN_0041cf70` | 4 | yaw + pitch, same fields | camera |
//!
//! 33 is 镜头环绕 — the look-at point stays and the camera swings around
//! it. 35 is 镜头摇动 — the camera stays and its aim swings. yaobow had
//! the second behaviour attached to opcode **33** and nothing on 35.
//!
//! The angles are absolute, not deltas, and arrive in degrees.

use std::f32::consts::PI;

use crate::openpal3::states::camera::Pal3Camera;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::{ISceneExt, ISceneManager};

/// Interpolation curve (`+0x18`): Pal3.Unity documents 0 as Linear and 1
/// as Sine. The sine form is the one the world-rotation tasks use
/// (`(1 - cos(pi t)) / 2`, from `FUN_00438110`); the camera task's own
/// per-frame update was not read, so the shape is `[推断]`.
fn ease(curve: i32, t: f32) -> f32 {
    if curve == 0 {
        t
    } else {
        (1. - (PI * t).cos()) / 2.
    }
}

/// Push the current polar state onto the scene's camera.
fn apply(scene_manager: &ComRc<ISceneManager>, camera: &Pal3Camera) {
    let Some(scene) = scene_manager.scene() else {
        return;
    };
    let (position, look_at) = (camera.position(), camera.look_at());
    scene
        .camera_mut()
        .transform_mut()
        .set_position(&position)
        .look_at(&look_at);
}

/// Progress of a timed camera move, or `None` once it has finished.
fn progress(
    elapsed: &mut f32,
    duration: f32,
    curve: i32,
    delta_sec: f32,
    snap: bool,
) -> (f32, bool) {
    if snap || duration <= 0. {
        return (1., true);
    }
    *elapsed += delta_sec;
    let t = (*elapsed / duration).min(1.);
    (ease(curve, t), t >= 1.)
}

/// 32 镜头推动作 — `LAB_004353a0` → `FUN_0041cee0`. Moves the camera in
/// or out along its current aim; the look-at point and both angles stay.
#[derive(Debug, Clone)]
pub struct SceCommandCameraPush {
    distance: f32,
    duration: f32,
    curve: i32,
    start: Option<f32>,
    elapsed: f32,
}

impl SceCommand for SceCommandCameraPush {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        let snap = state.fast_forward();
        let camera = state.global_state_mut().camera_mut();
        let start = *self.start.get_or_insert(camera.distance());
        let (t, done) = progress(
            &mut self.elapsed,
            self.duration,
            self.curve,
            delta_sec,
            snap,
        );

        camera.set_distance(start + (self.distance - start) * t);
        camera.recompute_position();
        let snapshot = camera.clone();
        apply(&scene_manager, &snapshot);
        done
    }
}

impl SceCommandCameraPush {
    pub fn new(distance: f32, duration: f32, curve: i32) -> Self {
        Self {
            distance,
            duration,
            curve,
            start: None,
            elapsed: 0.,
        }
    }
}

/// Shared body of 33 and 35: interpolate to absolute `yaw`/`pitch`, then
/// recompute whichever end the animation type holds still.
#[derive(Debug, Clone)]
struct AngleSwing {
    yaw: f32,
    pitch: f32,
    duration: f32,
    curve: i32,
    start: Option<(f32, f32)>,
    elapsed: f32,
    /// `true` for 33 (look-at held, camera swings), `false` for 35.
    hold_look_at: bool,
}

impl AngleSwing {
    fn new(yaw: f32, pitch: f32, duration: f32, curve: i32, hold_look_at: bool) -> Self {
        Self {
            yaw: yaw.to_radians(),
            pitch: pitch.to_radians(),
            duration,
            curve,
            start: None,
            elapsed: 0.,
            hold_look_at,
        }
    }

    fn step(
        &mut self,
        scene_manager: &ComRc<ISceneManager>,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        let snap = state.fast_forward();
        let camera = state.global_state_mut().camera_mut();
        let (yaw0, pitch0) = *self.start.get_or_insert((camera.yaw(), camera.pitch()));
        let (t, done) = progress(
            &mut self.elapsed,
            self.duration,
            self.curve,
            delta_sec,
            snap,
        );

        camera.set_yaw(yaw0 + (self.yaw - yaw0) * t);
        camera.set_pitch(pitch0 + (self.pitch - pitch0) * t);
        if self.hold_look_at {
            camera.recompute_position();
        } else {
            camera.recompute_look_at();
        }
        let snapshot = camera.clone();
        apply(scene_manager, &snapshot);
        done
    }
}

/// 33 镜头环绕 LookAt 位置旋转 — `FUN_004353f0` → `FUN_0041cf20`, type 2.
#[derive(Debug, Clone)]
pub struct SceCommandCameraOrbit(AngleSwing);

impl SceCommand for SceCommandCameraOrbit {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        self.0.step(&scene_manager, state, delta_sec)
    }
}

impl SceCommandCameraOrbit {
    pub fn new(yaw: f32, pitch: f32, duration: f32, curve: i32) -> Self {
        Self(AngleSwing::new(yaw, pitch, duration, curve, true))
    }
}

/// 35 镜头摇动（from 点固定）— `FUN_00435470` → `FUN_0041cf70`, type 4.
#[derive(Debug, Clone)]
pub struct SceCommandCameraSwing(AngleSwing);

impl SceCommand for SceCommandCameraSwing {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        self.0.step(&scene_manager, state, delta_sec)
    }
}

impl SceCommandCameraSwing {
    pub fn new(yaw: f32, pitch: f32, duration: f32, curve: i32) -> Self {
        Self(AngleSwing::new(yaw, pitch, duration, curve, false))
    }
}

/// 155 相机设置 Yaw — `LAB_00438d00` → `FUN_0041d590`.
///
/// Instant, absolute, and in **radians**: the handler passes the
/// parameter straight through with none of the `* 0.017453292` the other
/// angle commands apply (0x438d09). `FUN_0041c710(1)` then holds the
/// camera and swings its aim, as 35 does.
#[derive(Debug, Clone)]
pub struct SceCommandCameraYaw {
    yaw: f32,
}

impl SceCommand for SceCommandCameraYaw {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let camera = state.global_state_mut().camera_mut();
        camera.set_yaw(self.yaw);
        camera.recompute_look_at();
        let snapshot = camera.clone();
        apply(&scene_manager, &snapshot);
        true
    }
}

impl SceCommandCameraYaw {
    pub fn new(yaw: f32) -> Self {
        Self { yaw }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_linear_curve_is_the_identity_and_the_sine_one_eases_both_ends() {
        assert_eq!(ease(0, 0.25), 0.25);
        assert_eq!(ease(0, 1.0), 1.0);

        assert!(ease(1, 0.0).abs() < 1e-6);
        assert!((ease(1, 1.0) - 1.0).abs() < 1e-6);
        assert!((ease(1, 0.5) - 0.5).abs() < 1e-6);
        // Slower than linear at the start, faster in the middle.
        assert!(ease(1, 0.25) < 0.25);
        assert!(ease(1, 0.75) > 0.75);
    }

    #[test]
    fn a_zero_duration_move_lands_immediately() {
        let mut elapsed = 0.;
        assert_eq!(progress(&mut elapsed, 0., 0, 0.016, false), (1., true));
    }

    #[test]
    fn fast_forward_snaps_to_the_end() {
        let mut elapsed = 0.;
        assert_eq!(progress(&mut elapsed, 5., 1, 0.016, true), (1., true));
    }

    #[test]
    fn a_timed_move_reports_done_only_at_the_end() {
        let mut elapsed = 0.;
        let (t, done) = progress(&mut elapsed, 1., 0, 0.5, false);
        assert!((t - 0.5).abs() < 1e-6);
        assert!(!done);
        let (t, done) = progress(&mut elapsed, 1., 0, 0.6, false);
        assert_eq!(t, 1.);
        assert!(done);
    }
}
