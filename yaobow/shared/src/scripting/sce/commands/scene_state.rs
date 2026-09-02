//! Scene-object state the story reads and writes: maze switches, the
//! do-not-load list, and the divine tree's flower.
//!
//! The original keeps this in a save-state store (`DAT_017b1550`), looked
//! up by `FUN_00539540(city, scene, objectId)` with the switch byte at
//! the record's `+3`. It is per (city, block, object), so a switch thrown
//! in one maze is still thrown when the player comes back.

use crate::openpal3::directors::SceneManagerExtensions;
use crate::openpal3::scene::ScnScene;
use crate::scripting::sce::commands::vars;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

/// The current scene's cpk name, which is the store's first key part.
fn current_city(scene_manager: &ComRc<ISceneManager>) -> Option<String> {
    Some(
        scene_manager
            .scn_scene()?
            .inner::<ScnScene>()
            .name()
            .to_string(),
    )
}

/// 126 取迷宫开关状态赋值给变量 — `LAB_00438390`.
///
/// The original has two paths: when `sceneName` names the block the
/// player is standing in it reads the live object's byte at `obj+0xaf`
/// (0x4383e4), otherwise it goes to the save-state store (0x438400) and
/// reads the record's `+3`. Both are 0 when nothing is found (0x438428).
///
/// Here only the store exists — nothing in yaobow flips a live object's
/// switch yet, so the store is the sole writer and the two paths would
/// give the same answer. Pal3.Unity makes the same simplification, except
/// that it clamps the result to 0/1; the original does not, so neither
/// does this.
#[derive(Debug, Clone)]
pub struct SceCommandGetSwitch {
    scene: String,
    object_id: i32,
    var: i16,
}

impl SceCommand for SceCommandGetSwitch {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let city = current_city(&scene_manager).unwrap_or_default();
        let switch = state.global_state().persistent_state().scene_object_state(
            &city,
            &self.scene,
            self.object_id,
        );
        vars::write(state, self.var, switch);
        true
    }
}

impl SceCommandGetSwitch {
    pub fn new(scene: String, object_id: i32, var: i16) -> Self {
        Self {
            scene,
            object_id,
            var,
        }
    }
}

/// 158 某个物件出场时不从存档里拿状态 — `LAB_00438f30` →
/// `FUN_00539e70`, a `push_back` onto the store's vector. Additive, and
/// never cleared here.
#[derive(Debug, Clone)]
pub struct SceCommandObjectNotFromSave {
    object_id: i32,
}

impl SceCommand for SceCommandObjectNotFromSave {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        state
            .global_state_mut()
            .persistent_state_mut()
            .do_not_load_object_from_save(self.object_id);
        true
    }
}

impl SceCommandObjectNotFromSave {
    pub fn new(object_id: i32) -> Self {
        Self { object_id }
    }
}

/// The divine tree's flower: scene object 22 in `m16`'s block `4`
/// (`LAB_00438f50` compares against the literals at 0x5962a0 and
/// 0x590654).
const FLOWER_CITY: &str = "m16";
const FLOWER_SCENE: &str = "4";
const FLOWER_OBJECT: i32 = 0x16;

/// 159 初始化神树之花 — `LAB_00438f50`.
///
/// Toggles that one object's switch state and then latches
/// `DAT_00d8bc38` so it cannot run twice. The guard is a plain global,
/// not part of the save, so it resets when the game restarts — which is
/// why the latch lives on [`GlobalState`] here rather than in the save.
#[derive(Debug, Clone)]
pub struct SceCommandInitFlower {}

impl SceCommand for SceCommandInitFlower {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        if state.global_state().divine_tree_flower_initialized() {
            return true;
        }
        state
            .global_state_mut()
            .set_divine_tree_flower_initialized(true);

        let mut p_state = state.global_state_mut().persistent_state_mut();
        let was = p_state.scene_object_state(FLOWER_CITY, FLOWER_SCENE, FLOWER_OBJECT);
        p_state.set_scene_object_state(FLOWER_CITY, FLOWER_SCENE, FLOWER_OBJECT, (was == 0) as i32);
        true
    }
}

impl SceCommandInitFlower {
    pub fn new() -> Self {
        Self {}
    }
}

#[cfg(test)]
mod tests {
    use crate::openpal3::states::persistent_state::{PersistentState, scene_object_key};

    #[test]
    fn a_switch_defaults_to_zero_and_survives_being_set() {
        let mut state = PersistentState::new("test".to_string());
        assert_eq!(state.scene_object_state("m16", "4", 22), 0);
        state.set_scene_object_state("m16", "4", 22, 1);
        assert_eq!(state.scene_object_state("m16", "4", 22), 1);
        // A different object in the same block is untouched.
        assert_eq!(state.scene_object_state("m16", "4", 23), 0);
    }

    #[test]
    fn the_key_ignores_case_so_scn_and_script_spellings_agree() {
        // Scripts write "m16" while the scene's own name is "M16".
        assert_eq!(
            scene_object_key("M16", "4", 22),
            scene_object_key("m16", "4", 22)
        );
    }

    #[test]
    fn objects_load_from_save_until_the_script_says_otherwise() {
        let mut state = PersistentState::new("test".to_string());
        assert!(state.object_loads_from_save(7));
        state.do_not_load_object_from_save(7);
        assert!(!state.object_loads_from_save(7));
    }
}
