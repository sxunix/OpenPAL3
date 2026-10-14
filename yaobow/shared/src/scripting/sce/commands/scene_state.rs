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
/// `FUN_00539e70`, a `push_back` onto the store's vector. 下一次进场景
/// 套存档时（`FUN_00538d60`）名单里的物件跳过，套完清空。
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
/// 花是关着的（`DAT_00d8bc38` 为 0）才动：把 m16 区块 4 的 22 号物件（神树花总开关）的开关状态翻过来——
/// 当前就在那个场景就翻场上那个物件的，否则翻存着的那份；再把花记成开着。花开着就什么都不做。
/// 移植端开关状态都在 [`crate::openpal3::states::persistent_state::PersistentState`] 里，两种情况是同一个地方。不在这里重新摆场上的花
/// （原版这里也不调 `FUN_0042c280`）。
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
        if state.global_state().divine_tree_flower() {
            return true;
        }
        {
            let mut p_state = state.global_state_mut().persistent_state_mut();
            let was = p_state.scene_object_state(FLOWER_CITY, FLOWER_SCENE, FLOWER_OBJECT);
            p_state.set_scene_object_state(
                FLOWER_CITY,
                FLOWER_SCENE,
                FLOWER_OBJECT,
                (was == 0) as i32,
            );
        }
        state.global_state_mut().set_divine_tree_flower(true);
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
    use super::position_is_persisted;
    use crate::openpal3::states::persistent_state::{PersistentState, scene_object_key};

    #[test]
    fn the_nine_listed_scenes_do_not_remember_a_moved_object() {
        for sub in ["A", "B", "C", "D", "A1", "B1", "C1", "D1"] {
            assert!(!position_is_persisted("m15", sub), "m15/{sub}");
        }
        assert!(!position_is_persisted("m08", "3"));
        // Case comes from two different places — the script and the scn.
        assert!(!position_is_persisted("M15", "a"));
        // Anything else does remember.
        assert!(position_is_persisted("m15", "E"));
        assert!(position_is_persisted("m08", "2"));
        assert!(position_is_persisted("m16", "4"));
    }

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

/// The nine scenes where `LAB_004378e0` performs the move but skips
/// setting the object's "position was modified" flag at `+0x244`
/// (0x437997–0x437a67, each an `FUN_004235b0(scene, sub_scene)` test).
const NOT_PERSISTED: [(&str, &str); 9] = [
    ("m15", "A"),
    ("m15", "B"),
    ("m15", "C"),
    ("m15", "D"),
    ("m15", "A1"),
    ("m15", "B1"),
    ("m15", "C1"),
    ("m15", "D1"),
    ("m08", "3"),
];

fn position_is_persisted(scene: &str, sub_scene: &str) -> bool {
    !NOT_PERSISTED
        .iter()
        .any(|(s, sub)| scene.eq_ignore_ascii_case(s) && sub.eq_ignore_ascii_case(sub_scene))
}

/// 90 移动场景物体 — `LAB_004378e0`.
///
/// The three floats are an **offset** from where the object is, not a
/// destination (0x437922–0x437966). It does not block: the move is
/// queued onto the object's own command ring and the script runs on, so
/// here it goes to `ScnScene`'s per-frame slide list.
///
/// Pal3.Unity skips the move entirely in a couple of scenes; the binary
/// always moves and only skips *remembering* the new position, in the
/// nine scenes above.
#[derive(Debug, Clone)]
pub struct SceCommandObjectMove {
    object_id: i32,
    x: f32,
    y: f32,
    z: f32,
    duration: f32,
}

impl SceCommand for SceCommandObjectMove {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let Some(scene_rc) = scene_manager.scn_scene() else {
            return true;
        };
        let scene = scene_rc.inner::<ScnScene>();
        let offset = radiance::math::Vec3::new(self.x, self.y, self.z);
        let duration = if state.fast_forward() {
            0.
        } else {
            self.duration
        };
        let Some(landing) = scene.move_object(self.object_id, offset, duration) else {
            return true;
        };

        let (name, sub_name) = (scene.name().to_string(), scene.sub_name().to_string());
        if position_is_persisted(&name, &sub_name) {
            state
                .global_state_mut()
                .persistent_state_mut()
                .set_scene_object_position(&name, &sub_name, self.object_id, landing);
        }
        true
    }
}

impl SceCommandObjectMove {
    pub fn new(object_id: i32, x: f32, y: f32, z: f32, duration: f32) -> Self {
        Self {
            object_id,
            x,
            y,
            z,
            duration,
        }
    }
}
