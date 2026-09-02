use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

#[derive(Debug, Clone)]
pub struct SceCommandIfInTeam {
    role_id: i32,
}

impl SceCommand for SceCommandIfInTeam {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let in_team = state
            .global_state()
            .persistent_state()
            .is_in_team(self.role_id as u32);
        state.global_state_mut().fop_state_mut().push_value(in_team);
        true
    }
}

impl SceCommandIfInTeam {
    pub fn new(role_id: i32) -> Self {
        Self { role_id }
    }
}
