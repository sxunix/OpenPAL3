//! 87 播放门打开的动画 — `LAB_00437a80`.

use crate::openpal3::comdef::ICvdModel;
use crate::openpal3::directors::SceneManagerExtensions;
use crate::openpal3::scene::{CvdModelComponent, CvdPlayMode, ScnScene};
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::{IEntity, IEntityExt, ISceneManager};

/// `gbVisNode::SetTimeScale(obj+0x5c, 2.0)` at 0x42bea0 — the door plays
/// at double speed.
const DOOR_SPEED: f32 = 2.0;

/// Every `CvdModelComponent` at or under `entity`. A `.cvd` is a tree of
/// nodes and each one carries its own keyframe tracks, so a door's leaves
/// are separate components from its frame.
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

/// 87 开门.
///
/// `FUN_0042bea0` is `SceneObj::Script_OpenDoor` (`gamebase/sceneobj.cpp:4257`):
///
/// ```cpp
/// ASSERT(m_info.type == 14);           // 门
/// if (m_mdl.m_mdltype == 3) {          // 模型是 .cvd
///     m_mdl.m_geom->SetTimeScale(PLAY_TIME_SCALE);   // 2.0f
///     m_mdl.m_geom->Reset();
///     m_subobj.state = 2;
///     PlaySnd();
/// }
/// ```
///
/// and the opcode then blocks on `IsWaiting()`, i.e. until `m_subobj.state`
/// is back to 0. Pal3.Unity fires and forgets at normal speed; this follows
/// the binary.
///
/// 「animation type 3」就是**模型是 `.cvd`**——`MyMdl::Load`
/// （`gamebase/sceneobj.cpp:3054`）按扩展名定 `m_mdltype`：`.cvd` 是 3、
/// `.pol` 是 2、名字以 `+` 开头的特效是 4。所以「只对有 CVD 关键帧的物件生效」
/// 和原版是同一条判断，不是放宽。
///
/// 还差一处：原版同时 `PlaySnd()` 放物件自己的音效，这里没放。
#[derive(Clone)]
pub struct SceCommandOpenDoor {
    object_id: i32,
    playing: Vec<ComRc<ICvdModel>>,
    started: bool,
}

impl std::fmt::Debug for SceCommandOpenDoor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandOpenDoor")
            .field("object_id", &self.object_id)
            .field("nodes", &self.playing.len())
            .finish()
    }
}

impl SceCommand for SceCommandOpenDoor {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        if !self.started {
            self.started = true;
            let Some(scene) = scene_manager.scn_scene() else {
                return true;
            };
            let Some(entity) = scene.inner::<ScnScene>().get_root_object(self.object_id) else {
                log::warn!("OpenDoor: 找不到场景物件 {}", self.object_id);
                return true;
            };
            cvd_components(&entity, &mut self.playing);
            if self.playing.is_empty() {
                log::warn!("OpenDoor: 物件 {} 没有 CVD 关键帧", self.object_id);
                return true;
            }
            for c in &self.playing {
                c.inner::<CvdModelComponent>()
                    .play(CvdPlayMode::Once, DOOR_SPEED);
            }
        }

        if state.fast_forward() {
            return true;
        }

        self.playing
            .iter()
            .all(|c| c.inner::<CvdModelComponent>().finished())
    }
}

impl SceCommandOpenDoor {
    pub fn new(object_id: i32) -> Self {
        Self {
            object_id,
            playing: vec![],
            started: false,
        }
    }
}
