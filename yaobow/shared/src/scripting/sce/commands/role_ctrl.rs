use crate::openpal3::directors::SceneManagerExtensions;
use crate::openpal3::scene::{RoleController, ScnScene};
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::{IEntityExt, ISceneManager};

#[derive(Debug, Clone)]
pub struct SceCommandRoleCtrl {
    role_id: i32,
}

impl SceCommand for SceCommandRoleCtrl {
    /// 204：`PlayerMgr::SetCtrl(id)`（`gamebase/playermgr.cpp:1923`，`bCopyLast` 缺省为真）：
    /// 新主角接过旧主角的位置、朝向、所在层，旧的不在场、新的在场并站好，再换成新主角受控。
    ///
    /// 先前这里只改了「谁受控」：脚本前面摆好的是旧主角，换过去的新主角还停在建场景时的默认位置
    /// （m08 御剑跌落后 `RoleCtrl 0` 换回景天，景天站在地图边外，迷宫一处也走不到）。
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        set_ctrl(&scene_manager, state, self.role_id);
        true
    }
}

impl SceCommandRoleCtrl {
    pub fn new(role_id: i32) -> Self {
        Self { role_id }
    }
}

/// `PlayerMgr::SetCtrl(new, true)`（`FUN_0040e5f0`）：新主角接过旧主角的位置、朝向、所在层，旧的不在场、
/// 新的在场并站好，再换成新主角受控。换成龙葵时她的模型一律换回 `"105"`（蓝，人形）。
/// [未做] `SceneMgr::m_fHurtTime = 10`、头像框、中毒状态的画面提示。
pub fn set_ctrl(scene_manager: &ComRc<ISceneManager>, state: &mut SceState, new: i32) {
    let old = state.global_state().role_controlled();
    if old != new {
        if let Some(scn) = scene_manager.scn_scene() {
            let scn = scn.inner::<ScnScene>();
            if let (Some(o), Some(n)) = (scn.get_role_entity(old), scn.get_role_entity(new)) {
                let transform = o.transform().borrow().matrix().clone();
                n.transform().borrow_mut().set_matrix(transform);
                let layer = RoleController::get_role_controller(o.clone())
                    .map(|rc| rc.inner::<RoleController>().nav_layer());
                if let Some(rc) = RoleController::get_role_controller(o) {
                    rc.inner::<RoleController>().set_active(false);
                }
                if let Some(rc) = RoleController::get_role_controller(n) {
                    let rc = rc.inner::<RoleController>();
                    if let Some(layer) = layer {
                        rc.set_nav_layer(layer);
                    }
                    rc.set_active(true);
                    rc.idle();
                }
            }
        }
    }
    if new == super::story_flags::LONGKUI_ROLE_ID {
        if let Some(scn) = scene_manager.scn_scene() {
            let scn = scn.inner::<ScnScene>();
            if let Some(rc) = scn
                .get_role_entity(new)
                .and_then(RoleController::get_role_controller)
            {
                let rc = rc.inner::<RoleController>();
                if !rc
                    .model_name()
                    .eq_ignore_ascii_case(super::story_flags::LONGKUI_MODEL_BLUE)
                {
                    rc.set_model_name(super::story_flags::LONGKUI_MODEL_BLUE);
                }
            }
        }
        // 原版是把角色记录里的模型名写成 "105"，存档、换场景都跟着它；移植端的对应物是龙葵形象标志。
        state
            .global_state_mut()
            .persistent_state_mut()
            .set_longkui_mode(0);
    }
    state.global_state_mut().set_role_controlled(new);
}
