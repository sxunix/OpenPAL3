use crate::openpal3::directors::SceneManagerExtensions;
use crate::openpal3::scene::RoleController;
use crate::openpal3::states::game_db::PLAYER_ACTOR_IDS;
use crate::openpal3::states::role_move::{MIN_MOVE_DISTANCE, MoveMode};
use crate::scripting::sce::{SceCommand, SceState};

use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::IEntityExt;
use radiance::comdef::ISceneManager;
use radiance::math::Vec3;

/// 按第四个参数摆好走 / 跑 / 站的动作。opcode 20 和 214 共用。
pub(crate) fn apply_move_action(
    scene_manager: &ComRc<ISceneManager>,
    state: &mut SceState,
    role_id: i32,
    mode: MoveMode,
) {
    scene_manager.resolve_role_mut_do(state, role_id, |_e, r| match mode {
        MoveMode::Run => r.run(),
        MoveMode::Walk => r.walk(),
        // 原版这一支播的是 `z1`——站立的那条，人保持站姿被挪过去。
        MoveMode::Still => r.idle(),
    });
}

/// 这一次移动走多快（`FUN_00412710` 的 `switch (+0x504)`）。
///
/// 「是不是主角」原版看的是角色类型 `+0xad == -1`；移植端按角色号在
/// `PLAYER_ACTOR_IDS` 里判，效果相同。角色自带的速度（`+0x1fc`）还没有
/// 对应的通路，这里一律当 0。
pub(crate) fn move_speed(state: &SceState, role_id: i32, mode: MoveMode) -> f32 {
    let is_player = PLAYER_ACTOR_IDS.contains(&(role_id as u32));
    state
        .global_state()
        .role_constants()
        .speed(mode, is_player, 0.)
}

#[derive(Debug, Clone)]
pub struct SceCommandRoleMoveTo {
    role_id: i32,
    nav_x: f32,
    nav_z: f32,
    /// 第四个参数，见 `openpal3::states::role_move::MoveMode`。
    /// 原先这里叫 `unknown` 且没用上——于是所有角色移动都是跑的。
    mode: MoveMode,
}

impl SceCommand for SceCommandRoleMoveTo {
    fn initialize(&mut self, scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        apply_move_action(&scene_manager, state, self.role_id, self.mode);
    }

    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        let speed = move_speed(state, self.role_id, self.mode);

        let Some(role) = scene_manager.get_resolved_role(state, self.role_id) else {
            log::warn!("RoleMoveTo: 找不到角色 {}，跳过", self.role_id);
            return true;
        };

        let role_controller = RoleController::get_role_controller(role).unwrap();
        let to = {
            let nav_layer = role_controller.inner::<RoleController>().nav_layer();
            let scn = scene_manager.scn_scene().unwrap();
            let s = scn.inner::<crate::openpal3::scene::ScnScene>();
            s.nav_coord_to_scene_coord(nav_layer, self.nav_x, self.nav_z)
        };

        let role = scene_manager
            .get_resolved_role(state, self.role_id)
            .unwrap();
        let position = role.transform().borrow().position();
        let step = speed * delta_sec;
        let remain = Vec3::sub(&to, &position);
        // 原版 `FUN_00412b60` 一进来就量距离，不到 12 就当已经到了。
        let completed =
            state.fast_forward() || remain.norm() < step || remain.norm() < MIN_MOVE_DISTANCE;
        let new_position = if completed {
            to
        } else {
            Vec3::add(
                &position,
                &Vec3::scalar_mul(step, &Vec3::normalized(&remain)),
            )
        };

        // 站着挪那一支是背对目标走（和 opcode 20 一致）。
        let look_at = if self.mode == MoveMode::Still {
            Vec3::add(&position, &Vec3::sub(&position, &to))
        } else {
            Vec3::new(to.x, position.y, to.z)
        };
        role.transform()
            .borrow_mut()
            .look_at(&look_at)
            .set_position(&new_position);

        if completed {
            role_controller.inner::<RoleController>().idle();
        }

        completed
    }
}

impl SceCommandRoleMoveTo {
    pub fn new(role_id: i32, nav_x: i32, nav_z: i32, mode: i32) -> Self {
        Self {
            role_id,
            nav_x: nav_x as f32,
            nav_z: nav_z as f32,
            mode: MoveMode::from_param(mode),
        }
    }
}
