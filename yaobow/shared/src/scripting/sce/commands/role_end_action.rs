use crate::{
    openpal3::{
        directors::SceneManagerExtensions,
        scene::{RoleAnimationRepeatMode, RoleState},
    },
    scripting::sce::{SceCommand, SceState},
};

use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

#[derive(Debug, Clone)]
pub struct SceCommandRoleEndAction {
    role_id: i32,
}

impl SceCommand for SceCommandRoleEndAction {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        scene_manager
            .resolve_role_mut_do(state, self.role_id, |_, rc| {
                // A looping action (`RoleShowAction` mode `-1`) wraps forever and
                // never reaches `Idle`/`AnimationFinished`, so blocking on it here
                // would stall the script permanently. Mirror `RoleShowAction`'s
                // loop short-circuit: unblock immediately and leave the loop
                // playing (keeps the actor's current pose).
                if rc.repeat_mode() == RoleAnimationRepeatMode::Loop {
                    return true;
                }

                if rc.state() == RoleState::AnimationHolding {
                    rc.continue_anim();
                }

                rc.state() == RoleState::Idle || rc.state() == RoleState::AnimationFinished
            })
            // 找不到这个人就当动作已经结束。主线第一章「在客房与赵文昌对话」
            // （Q01 过程 1003）在运行模式 2 下把 `221 -1` 和 104 鉴定一起发出去
            // 并行跑：鉴定一上来把自己的场景压到栈顶，下一帧这里就找不到主角，
            // 先前 `unwrap()` 直接崩。原版鉴定是 `SwitchMode`，冒险世界停着、
            // 主角的动作也不会再往前走，等它不如放行。
            .unwrap_or(true)
    }
}

impl SceCommandRoleEndAction {
    pub fn new(role_id: i32) -> Self {
        Self { role_id }
    }
}
