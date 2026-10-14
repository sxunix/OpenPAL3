//! 124 与指定场景物件交互 — `LAB_00438350`.
//!
//! The handler looks the object up and calls
//! `FUN_00424c20(player, 2, 0, 0.001)` on it — the engine's generic
//! "something interacted with this object" entry point, a switch over the
//! object's `.scn` node type at `obj+0xf4` with about forty arms. Each arm
//! checks the event itself; for the **2** this opcode passes the ones that
//! act are 1、2（动画播一遍 / 循环播）、7/8（旋转石梁）、0x11、0x13（吊桥）、
//! 0x16（升降平台）、0x17（开关，看参数 1）、0x1c（剑冢的锤）、0x1f、
//! 0x22（剑冢的剑桥）、0x24（坠落的障碍物）。
//!
//! 1、2 只播动画，在这里做；别的一律交给场景物件的动作（[`crate::openpal3::scene::object_action`]，
//! 收事件 2），由那边按类型照原版取舍——先前这里只挑了几种转过去，剑桥（0x22）叫不动。

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
        _state: &mut SceState,
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
            _ => scene.post_object_event(self.object_id, 2),
        }

        true
    }
}

impl SceCommandInteractWithObject {
    pub fn new(object_id: i32) -> Self {
        Self { object_id }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_animations_play_at_normal_speed() {
        assert_eq!(PLAIN_ANIM_SPEED, 1.0);
    }
}
