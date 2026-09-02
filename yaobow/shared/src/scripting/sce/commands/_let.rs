//! 13 变量赋值 · 14 变量增值.

use crate::scripting::sce::commands::vars;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

#[derive(Debug, Clone)]
pub struct SceCommandLet {
    var: i16,
    value: i32,
}

impl SceCommand for SceCommandLet {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        vars::write(state, self.var, self.value);
        true
    }
}

impl SceCommandLet {
    pub fn new(var: i16, value: i32) -> Self {
        Self { var, value }
    }
}

/// 14 `Var += Value` — `LAB_00436380`, which does exactly
/// `[var + 4] += [value + 4]` (0x43639b) with no clamping.
#[derive(Debug, Clone)]
pub struct SceCommandLetAdd {
    var: i16,
    value: i32,
}

impl SceCommand for SceCommandLetAdd {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let sum = vars::read(state, self.var).wrapping_add(self.value);
        vars::write(state, self.var, sum);
        true
    }
}

impl SceCommandLetAdd {
    pub fn new(var: i16, value: i32) -> Self {
        Self { var, value }
    }
}
