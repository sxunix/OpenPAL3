//! 36 设置镜头参数 — `LAB_00435570`.

use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneExt;
use radiance::comdef::ISceneManager;
use radiance::math::Vec3;

/// 36 `CameraSet(yaw°, pitch°, distance, x, y, z)`.
///
/// `LAB_00435570` converts the two angles to radians (`* 0x56a9f0`) and
/// hands everything to `FUN_0041c9a0(yaw, pitch, distance, x, y, z, 1)`,
/// which stores pitch at `+0x1c4`, distance at `+0x1d4`, yaw at `+0x1c8`,
/// clears roll, writes `(x, y, z)` to `+0x1ec` — the **look-at point** —
/// and then calls `FUN_0041c710(0)`, deriving the camera position as
/// `look_at + distance · dir`.
///
/// So the triple is what the camera looks *at*, and the third scalar is
/// how far away it stands. This used to place the camera at `(x, y, z)`
/// and ignore the distance, which put every scripted shot inside its own
/// subject — a black frame in q01/xa's 竹林, where the target is 664
/// units from where the camera belongs.
///
/// It does not touch the focus target (`+0x17c` / `+0x180`); a script
/// that wants the camera to keep tracking says so separately.
#[derive(Debug, Clone)]
pub struct SceCommandCameraSet {
    yaw: f32,
    pitch: f32,
    distance: f32,
    look_at: Vec3,
}

impl SceCommand for SceCommandCameraSet {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let position = {
            let camera = state.global_state_mut().camera_mut();
            camera.set_yaw(self.yaw);
            camera.set_pitch(self.pitch);
            camera.set_distance(self.distance);
            camera.set_look_at(self.look_at);
            camera.recompute_position();
            camera.position()
        };

        if let Some(scene) = scene_manager.scene() {
            scene
                .camera_mut()
                .transform_mut()
                .set_position(&position)
                .look_at(&self.look_at);
        }

        true
    }
}

impl SceCommandCameraSet {
    pub fn new(yaw: f32, pitch: f32, distance: f32, x: f32, y: f32, z: f32) -> Self {
        Self {
            yaw: yaw.to_radians(),
            pitch: pitch.to_radians(),
            distance,
            look_at: Vec3::new(x, y, z),
        }
    }
}
