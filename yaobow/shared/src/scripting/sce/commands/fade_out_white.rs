use super::fade_in::SCE_COMMAND_FADE_IN_TIMEOUT;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

#[derive(Debug, Clone)]
pub struct SceCommandFadeOutWhite {
    spent: f32,
}

impl SceCommand for SceCommandFadeOutWhite {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        state.set_auto_fade_in(false);
        // 原版指令一执行就把淡出排进屏幕效果队列，消息栏（`FUN_00410360`）从这一帧起就不收；
        // 效果先 `Update` 再画，所以先加 `dt` 再算透明度，头一帧幕布就不是 0。
        self.spent += delta_sec;
        let opacity = (self.spent / SCE_COMMAND_FADE_IN_TIMEOUT).min(1.);
        state.set_curtain(-opacity);

        if state.fast_forward() || self.spent >= SCE_COMMAND_FADE_IN_TIMEOUT {
            state.set_curtain(-1.);
            true
        } else {
            false
        }
    }
}

impl SceCommandFadeOutWhite {
    pub fn new() -> Self {
        Self { spent: 0. }
    }
}
