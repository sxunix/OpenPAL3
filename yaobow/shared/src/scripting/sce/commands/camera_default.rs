use crate::openpal3::states::camera::Focus;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

/// 37 `CameraDefault(type)` — `cmd_CameraDefault` (script/scriptcmd.cpp):
/// lock on the controlled role, `Default(type)`, `SetFree(TRUE)`. The
/// director's follow step places the camera from the next frame on. [移植]
#[derive(Debug, Clone)]
pub struct SceCommandCameraDefault {
    kind: i32,
}

impl SceCommand for SceCommandCameraDefault {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let camera = state.global_state_mut().camera_mut();
        camera.set_focus(Focus::Player);
        camera.set_default(self.kind);
        camera.set_follow_player(true);
        true
    }
}

impl SceCommandCameraDefault {
    pub fn new(kind: i32) -> Self {
        Self { kind }
    }
}
