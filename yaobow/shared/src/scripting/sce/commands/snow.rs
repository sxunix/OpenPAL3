//! 140 开启/关闭下雪 — `LAB_004387b0`.
//!
//! Three instructions: read the parameter, and write `param != 0` into
//! the byte at `0xbfda40`. The frame loop tests that byte (0x4175) and,
//! when it is set, calls `gbDropGroup::Render` on the `gbSnow` built at
//! startup — see `openpal3::scene::snow` for what that does.
//!
//! Scripts turn it on in 冰风谷 (M22) and 雪岭镇 (Q15) and off again on
//! the way out; there is a second drop group for rain at `0xbfda3c` that
//! no SCE command touches.

use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

/// 140 Snow.
#[derive(Debug, Clone)]
pub struct SceCommandSnow {
    on: i32,
}

impl SceCommand for SceCommandSnow {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        state.set_snow(self.on != 0);
        true
    }
}

impl SceCommandSnow {
    pub fn new(on: i32) -> Self {
        Self { on }
    }
}
