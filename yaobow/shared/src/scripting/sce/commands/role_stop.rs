use crate::{
    openpal3::directors::SceneManagerExtensions,
    scripting::sce::{SceCommand, SceState},
};

use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

#[derive(Debug, Clone)]
pub struct SceCommandRoleStop {
    role_id: i32,
}

impl SceCommand for SceCommandRoleStop {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        // 找不到人（栈顶是小游戏 / 战斗的场景，或者场景里没这个人）就什么都不做。
        if scene_manager
            .resolve_role_mut_do(state, self.role_id, |_, r| r.idle())
            .is_none()
        {
            log::warn!("RoleStop: 找不到角色 {}，跳过", self.role_id);
        }

        true
    }
}

impl SceCommandRoleStop {
    pub fn new(role_id: i32) -> Self {
        Self { role_id }
    }
}
