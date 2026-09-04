//! 147 退出游戏到主菜单 — `LAB_00438a20`.
//!
//! Two bytes of setup: `push 4; mov ecx, 0xdb9fd0; call FUN_0044f610`.
//! `FUN_0044f610` is the game's state switcher — it stores the new state
//! at `+0x38c8`, remembers the old one at `+0x38cc`, and builds the
//! screen for it in `FUN_0044f8d0`. State 4 constructs the object whose
//! setup loads `ui\Menu\LOG0.tga` and `Music\PI01.mp3` and reads the
//! `SOFTWARE\SOFTSTAR\PAL3_S_GAME` registry key: the title screen.
//!
//! The scripts use it at the endings — Q08's 紫萱结局判断达成 and
//! 花楹结局判断达成, M25's 决斗胜 and 决斗败.

use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

/// 147 Switch2Menu.
#[derive(Debug, Clone)]
pub struct SceCommandSwitchToMainMenu;

impl SceCommand for SceCommandSwitchToMainMenu {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        log::info!("Switch2Menu: 请求返回主菜单");
        state.request_title();
        true
    }
}

impl SceCommandSwitchToMainMenu {
    pub fn new() -> Self {
        Self
    }
}
