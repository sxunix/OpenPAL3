use crate::openpal3::directors::SceneManagerExtensions;
use crate::openpal3::scene::RoleController;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

#[derive(Debug, Clone)]
pub struct SceCommandDlgFace {
    _id: i32,
    face_name: String,
    left_or_right: i32,
}

impl SceCommand for SceCommandDlgFace {
    fn initialize(&mut self, _scene_manager: ComRc<ISceneManager>, _state: &mut SceState) {}

    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let _resolved_role_id = if self._id == -1 {
            state.global_state().role_controlled()
        } else {
            self._id
        };
        // A script can name a role the current scene never built — the
        // portrait is then simply absent rather than fatal.
        let role_entity = _scene_manager.scn_scene().and_then(|scn| {
            scn.inner::<crate::openpal3::scene::ScnScene>()
                .get_role_entity(_resolved_role_id)
        });
        let Some(role_name) = role_entity
            .and_then(RoleController::get_role_controller)
            .map(|c| c.inner::<RoleController>().model_name().to_owned())
        else {
            log::warn!("DlgFace: 场景里没有角色 {_resolved_role_id}");
            return true;
        };
        state.set_dialog_avatar(&role_name, &self.face_name, self.left_or_right);

        true
    }
}

impl SceCommandDlgFace {
    pub fn new(_id: i32, face_name: String, left_or_right: i32) -> Self {
        Self {
            _id,
            face_name,
            left_or_right,
        }
    }
}
