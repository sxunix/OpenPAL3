//! 34 镜头平移 — `LAB_004354f0`.
//!
//! Reads `x, y, z, duration, curve` and hands them to `FUN_0041d010`, which
//! sets up the camera task (`DAT_00d8e4c0`) as type 3: the **camera
//! position** (`+0x1ec..+0x1f4`) goes from where it is to `(x, y, z)` over
//! `duration` with the same curve as 32/33/35, and every frame
//! `FUN_0041c710(0)` puts the look-at point at `position + distance · dir`
//! — angles and distance are kept, the whole camera slides. The handler
//! then blocks the script on the task (`FUN_00434c40`). Pal3.Unity's
//! `CameraMoveCommand` moves the camera position the same way.
//!
//! This used to drop the camera itself at `(x, y, z)` at once and carry on.

use super::camera_anim::{apply, progress};
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;
use radiance::math::Vec3;

#[derive(Debug, Clone)]
pub struct SceCommandCameraMove {
    target: Vec3,
    duration: f32,
    curve: i32,
    start: Option<Vec3>,
    elapsed: f32,
}

impl SceCommand for SceCommandCameraMove {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        let snap = state.fast_forward();
        let camera = state.global_state_mut().camera_mut();
        let start = *self.start.get_or_insert(camera.position());
        let (t, done) = progress(
            &mut self.elapsed,
            self.duration,
            self.curve,
            delta_sec,
            snap,
        );
        let at = Vec3::new(
            start.x + (self.target.x - start.x) * t,
            start.y + (self.target.y - start.y) * t,
            start.z + (self.target.z - start.z) * t,
        );
        camera.set_position(at);
        camera.recompute_look_at();
        let snapshot = camera.clone();
        apply(&scene_manager, &snapshot);
        done
    }
}

impl SceCommandCameraMove {
    pub fn new(x: f32, y: f32, z: f32, duration: f32, curve: i32) -> Self {
        Self {
            target: Vec3::new(x, y, z),
            duration,
            curve,
            start: None,
            elapsed: 0.,
        }
    }
}
