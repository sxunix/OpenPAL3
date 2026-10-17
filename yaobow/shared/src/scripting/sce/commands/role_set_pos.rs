use crate::{
    openpal3::{directors::SceneManagerExtensions, scene::RoleController},
    scripting::sce::{SceCommand, SceState},
};

use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::IEntityExt;
use radiance::comdef::ISceneManager;

#[derive(Debug, Clone)]
pub struct SceCommandRoleSetPos {
    role_id: i32,
    nav_x: f32,
    nav_z: f32,
}

impl SceCommand for SceCommandRoleSetPos {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        // 找不到人（栈顶不是冒险场景，或者场景里没这个人）就跳过。下面同一帧里
        // 再取一次一定取得到。
        if scene_manager
            .get_resolved_role(state, self.role_id)
            .is_none()
        {
            log::warn!("RoleSetPos: 找不到角色 {}，跳过", self.role_id);
            return true;
        }
        let position = {
            let role = scene_manager
                .get_resolved_role(state, self.role_id)
                .unwrap();
            let role_controller = RoleController::get_role_controller(role).unwrap();
            let nav_layer = role_controller.inner::<RoleController>().nav_layer();
            let scn = scene_manager.scn_scene().unwrap();
            let s = scn.inner::<crate::openpal3::scene::ScnScene>();
            s.nav_coord_to_scene_coord(nav_layer, self.nav_x, self.nav_z)
        };

        let role = scene_manager
            .get_resolved_role(state, self.role_id)
            .unwrap();
        role.transform().borrow_mut().set_position(&position);

        state
            .global_state_mut()
            .persistent_state_mut()
            .set_position(position);

        // `Role::SetPosition(tilex, tilez)`: moving the controlled role puts
        // a free camera straight onto it (`FlushSpring`).
        let controlled = state.global_state().role_controlled();
        let camera = state.global_state_mut().camera_mut();
        if (self.role_id == -1 || self.role_id == controlled) && camera.follow_player() {
            camera.request_flush();
        }

        true
    }
}

impl SceCommandRoleSetPos {
    pub fn new(role_id: i32, nav_x: i32, nav_z: i32) -> Self {
        Self {
            role_id,
            nav_x: nav_x as f32,
            nav_z: nav_z as f32,
        }
    }
}
