use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

/// 四条 fade 命令共用的时长。真值和出处在
/// [`crate::openpal3::states::fade::FADE_SECONDS`]。
pub use crate::openpal3::states::fade::FADE_SECONDS as SCE_COMMAND_FADE_IN_TIMEOUT;

#[derive(Debug, Clone)]
pub struct SceCommandFadeIn {
    spent: f32,
}

impl SceCommand for SceCommandFadeIn {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        state.set_auto_fade_in(false);
        let opacity = 1. - self.spent / SCE_COMMAND_FADE_IN_TIMEOUT;

        state.set_curtain(opacity);
        self.spent += delta_sec;

        if state.fast_forward() || self.spent >= SCE_COMMAND_FADE_IN_TIMEOUT {
            state.set_curtain(0.);
            true
        } else {
            false
        }
    }
}

impl SceCommandFadeIn {
    pub fn new() -> Self {
        Self { spent: 0. }
    }
}
