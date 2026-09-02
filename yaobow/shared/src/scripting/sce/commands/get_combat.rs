//! Opcode 132 `GetCombat(var)` — `var = DAT_005833d0` (0x438420 area),
//! the byte `FUN_004055d0` case 3 sets to `(result == 2)` when the last
//! battle ended: 1 for a win, a round-out or a successful escape, 0 for a
//! loss. Before any battle the byte is 0 in the original; the previous
//! stub returned 1 and scripts written against it read "won", so with no
//! battle on record 1 is kept.

use super::combat::last_result;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

#[derive(Debug, Clone)]
pub struct SceCommandGetCombat {
    var: i16,
}

impl SceCommand for SceCommandGetCombat {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let won = last_result(state).unwrap_or(true);
        state
            .context_mut()
            .current_proc_context_mut()
            .set_local(self.var, won as i32);
        true
    }
}

impl SceCommandGetCombat {
    pub fn new(var: i16) -> Self {
        Self { var }
    }
}
