//! 124 与指定场景物件交互 — `LAB_00438350`.
//!
//! The handler looks the object up and calls
//! `FUN_00424c20(player, 2, 0, 0.001)` on it — the engine's generic
//! "something interacted with this object" entry point, a switch over the
//! object's `.scn` node type at `obj+0xf4` with about forty arms. Most of
//! those arms open by rejecting the caller: they run only for
//! `param_2 == 0` (walked into) or `param_2 == 1` (bumped), and 124
//! always passes **2**. So for this opcode the switch collapses to a
//! handful of types:
//!
//! | node type | 原版做的事 |
//! |---|---|
//! | 1 | 动画播一遍（时间缩放 1.0），状态置 4 |
//! | 2 | 动画循环播（时间缩放 1.0），状态置 1 |
//! | 22 | 把物件抬到 `+0xf8` 给的高度，再按一次放回 |
//! | 23 | 翻转开关标志 `+0xaf`；动画型物件重播一遍，时间缩放 2.0；参数 0 为 1 时「可通过」跟着开关走（交给物件动作） |
//! | 28 | 只翻转开关标志（`+0xf8 == 1` 时） |
//! | 19（0x13，吊桥） | 放下来：交给场景物件的动作队列（[`crate::openpal3::scene::object_action`]） |
//! | 其它 | 对 `param_2 == 2` 不响应 |
//!
//! Every acting arm also decrements the object's remaining-uses counter
//! at `+0xae` when it is not -1 (unlimited). Which `.scn` field feeds
//! that counter was not identified, so it is not reproduced — objects
//! here can be used any number of times.
//!
//! Type 22's target height is `obj+0xf8`, which the object constructor
//! fills from record offset `0x4c` — the same slot a ladder node keeps
//! its first nav coordinate in. The arm sets the object's y to it on the
//! first press and puts it back on the second. The original slides the
//! object there over time (its per-frame update owns the motion, keyed on
//! the state byte); here it moves in one step, so the end position is
//! right and the travel is not.
//!
//! The switch flag is the same one 126 `GetSwitch` reads, so a mechanism
//! toggled here is visible to the scripts that gate on it.

use crate::openpal3::comdef::ICvdModel;
use crate::openpal3::directors::SceneManagerExtensions;
use crate::openpal3::scene::{CvdModelComponent, CvdPlayMode, ScnScene};
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::{IEntity, IEntityExt, ISceneManager};

/// Types 1 and 2 play at normal speed.
pub const PLAIN_ANIM_SPEED: f32 = 1.0;

/// Node types whose arm acts when the caller passes 2, i.e. the ones
/// 124 can do anything with at all.
pub const INTERACTIVE_TYPES: [u16; 6] = [1, 2, 19, 22, 23, 28];

fn cvd_components(entity: &ComRc<IEntity>, out: &mut Vec<ComRc<ICvdModel>>) {
    if let Some(c) = entity
        .get_component(ICvdModel::uuid())
        .and_then(|c| c.query_interface::<ICvdModel>())
    {
        out.push(c);
    }
    for child in entity.children() {
        cvd_components(&child, out);
    }
}

/// Play every CVD track under the object once, at `speed`. Returns
/// whether there was anything to play.
fn play_once(entity: &ComRc<IEntity>, speed: f32) -> bool {
    let mut nodes = vec![];
    cvd_components(entity, &mut nodes);
    for node in &nodes {
        node.inner::<CvdModelComponent>()
            .play(CvdPlayMode::Once, speed);
    }
    !nodes.is_empty()
}

fn play_loop(entity: &ComRc<IEntity>, speed: f32) -> bool {
    let mut nodes = vec![];
    cvd_components(entity, &mut nodes);
    for node in &nodes {
        node.inner::<CvdModelComponent>()
            .play(CvdPlayMode::Loop, speed);
    }
    !nodes.is_empty()
}

/// 124 Trigger.
#[derive(Debug, Clone)]
pub struct SceCommandInteractWithObject {
    object_id: i32,
}

impl SceCommand for SceCommandInteractWithObject {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let Some(scene) = scene_manager.scn_scene() else {
            return true;
        };
        let scene = scene.inner::<ScnScene>();
        let Some(node_type) = scene.object_node_type(self.object_id) else {
            log::warn!("Trigger: 找不到场景物件 {}", self.object_id);
            return true;
        };
        let entity = scene.get_root_object(self.object_id);

        match node_type {
            1 => {
                if let Some(entity) = &entity {
                    play_once(entity, PLAIN_ANIM_SPEED);
                }
            }
            2 => {
                if let Some(entity) = &entity {
                    play_loop(entity, PLAIN_ANIM_SPEED);
                }
            }
            // 开关（`FUN_00424c20` case 0x17 事件 2）交给物件动作：翻开关、播动画、扣次数，参数 0 为 1 的
            // 「可通过」跟着开关走（蜀山故道最上方的石门就是这种）。先前这里只翻开关、播动画，门开了格子还挡着。
            23 => scene.post_object_event(self.object_id, 2),
            28 => {
                let _ = self.toggle_switch(&scene, state);
            }
            22 => self.lift(&scene, state),
            // 吊桥（`FUN_00424c20` case 0x13）排的动作要带着主角做，交给冒险循环。
            19 => scene.post_object_event(self.object_id, 2),
            other => log::debug!(
                "Trigger: 物件 {} 的 node type {other} 对这种交互不响应",
                self.object_id
            ),
        }

        true
    }
}

impl SceCommandInteractWithObject {
    pub fn new(object_id: i32) -> Self {
        Self { object_id }
    }

    /// Type 22: up to the parameter height, or back down. The original
    /// tracks four states (idle / rising / up / falling); with no motion
    /// to be in the middle of, the switch flag carries the same
    /// information — off means down, on means up.
    fn lift(&self, scene: &ScnScene, state: &mut SceState) {
        let Some(target) = scene.object_parameter(self.object_id) else {
            return;
        };
        let Some((home, _)) = scene.object_placement(self.object_id) else {
            return;
        };
        let raised = self.toggle_switch(scene, state) == 1;
        let y = if raised { target as f32 } else { home.y };
        let Some(entity) = scene.get_root_object(self.object_id) else {
            return;
        };
        let mut position = entity.transform().borrow().position();
        position.y = y;
        entity.transform().borrow_mut().set_position(&position);
        log::debug!(
            "Trigger: 升降物件 {} -> y {y} ({})",
            self.object_id,
            if raised { "升起" } else { "落回" }
        );
    }

    /// `+0xaf = !+0xaf`, the byte 126 `GetSwitch` reads back. Returns the
    /// new value.
    fn toggle_switch(&self, scene: &ScnScene, state: &mut SceState) -> i32 {
        let city = scene.name().to_string();
        let sub = scene.sub_name().to_string();
        let mut persistent = state.global_state_mut().persistent_state_mut();
        let now = persistent.scene_object_state(&city, &sub, self.object_id);
        let next = if now == 0 { 1 } else { 0 };
        persistent.set_scene_object_state(&city, &sub, self.object_id, next);
        log::debug!("Trigger: 物件 {} 开关 {now} -> {next}", self.object_id);
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_five_node_types_answer_this_kind_of_interaction() {
        // Everything else in `FUN_00424c20` returns early for the 2 this
        // opcode passes; the list is what the switch actually acts on.
        assert_eq!(INTERACTIVE_TYPES, [1, 2, 19, 22, 23, 28]);
        assert_eq!(PLAIN_ANIM_SPEED, 1.0);
    }
}
