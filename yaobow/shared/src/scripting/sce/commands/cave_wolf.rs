//! 112 山洞初体验（山洞之狼）— `LAB_00437c20` → `FUN_00406ff0(0)`。
//!
//! 规则在 `openpal3::states::cave_wolf`；这里只负责**开局**：跑 `init.sce`
//! 的过程 99 把场景和三个人摆好，再把「这一局开始了」记到 `SceState::ext`
//! 上。真正的计时和抓人在 `AdventureDirector` 里走——原版也是这样，它是主
//! 循环的一个游戏状态（`DAT_00bfda6c = 7`），不是脚本命令自己在跑。

use std::rc::Rc;

use crate::openpal3::states::cave_wolf::SETUP_PROC;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

/// `SceState::ext` 上的开局标记，导演读了就把这一局跑起来。
pub const EXT_START: &str = "pal3.cave_wolf.start";

/// 有没有人请求开一局，取走标记。
pub fn take_start_request(state: &mut SceState) -> bool {
    state.ext_mut().remove(EXT_START).is_some()
}

#[derive(Debug, Clone)]
pub struct SceCommandMiniGameCaveExperience;

impl SceCommandMiniGameCaveExperience {
    pub fn new() -> Self {
        Self
    }
}

impl SceCommand for SceCommandMiniGameCaveExperience {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        // 过程 99 在 init.sce 里，不在当前场景的脚本里。
        let init = Rc::new(state.asset_mgr().load_init_sce());
        if !state.call_proc_in(init, "init", SETUP_PROC) {
            log::warn!("CaveWolf: init.sce 里没有 proc {SETUP_PROC}");
            return true;
        }
        state
            .ext_mut()
            .insert(EXT_START.to_string(), Box::new(true));
        log::info!("CaveWolf: 山洞之狼开始");
        true
    }
}
