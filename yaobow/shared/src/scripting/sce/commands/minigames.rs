//! The mini-games, the shop-style menus, and the pawnshop's script
//! variables.
//!
//! None of these subsystems exist in yaobow yet, but every one of these
//! opcodes appears in shipped scripts, and an opcode the vm has no arm
//! for panics. So each gets an arm that consumes its parameters and,
//! where a script would otherwise branch on an uninitialised variable or
//! lose the thread of the plot, produces the one outcome that keeps the
//! script moving. That is the same trade `SceCommandGetAppr` already
//! makes for the appraisal game: skip the game, hand back a result.
//!
//! Every mini-game entry blocks in the original — `FUN_0044f610(mode)`
//! then `FUN_00434c40` on a "wait until mode ends" task — so skipping
//! them means the script resumes immediately instead. That is visible
//! only as a missing interlude.

use crate::scripting::sce::commands::vars;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

/// A skipped subsystem. `$note` is shown to the player through the same
/// note widget opcode 68 uses, so a scene that silently does nothing is
/// at least explained; Pal3.Unity does the same for its own mini-game
/// stubs. Pass `""` to skip quietly.
macro_rules! skipped {
    ($name:ident, $doc:expr, $note:expr $(, $field:ident : $ty:ty)*) => {
        #[doc = $doc]
        #[derive(Debug, Clone)]
        pub struct $name {
            $(#[allow(dead_code)] $field: $ty,)*
        }

        impl $name {
            pub fn new($($field: $ty),*) -> Self {
                Self { $($field),* }
            }
        }

        impl SceCommand for $name {
            fn update(
                &mut self,
                _scene_manager: ComRc<ISceneManager>,
                _ui: &Ui,
                state: &mut SceState,
                _delta_sec: f32,
            ) -> bool {
                log::warn!(concat!(stringify!($name), ": 子系统未实现，跳过"));
                if !$note.is_empty() {
                    state.show_note($note.to_string());
                }
                let _ = state;
                true
            }
        }
    };
}

/// 157 显示一条提示 — `LAB_00438f10` → `FUN_00410360(文本)`。
///
/// 名字里的「当铺经营游戏」是误会：`FUN_00410360` 是消息栏（加减钱的
/// `FUN_0040fc90` 也拿它报「获得 %d 文」），屏幕上方一行行叠着、淡出，见
/// [`SceState::push_note`]。实机只有 Q12 的 proc 1302 用它，报的是「景天精减少到1」这种话。
#[derive(Debug, Clone)]
pub struct SceCommandMiniGamePawnshopNote {
    text: String,
}

impl SceCommandMiniGamePawnshopNote {
    pub fn new(text: String) -> Self {
        Self { text }
    }
}

impl SceCommand for SceCommandMiniGamePawnshopNote {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        if !self.text.trim().is_empty() {
            state.push_note(self.text.clone());
        }
        true
    }
}
/// 154 引路蜂飞向 — `LAB_00438ba0`（源码 `cmd_Bee`）：层号对 2 取余，取那一层 (`tile_x`, `tile_y`) 格的
/// 中心当终点；引路蜂在主角的渲染位置现身、站住，再按走路的样子（NPC 速度 `WalkSpeed`）直线挪过去
/// （`Role::AMove`），每帧抬高 80，到了就不在场。脚本不等它。
#[derive(Debug, Clone)]
pub struct SceCommandNavigationBeeFlyTo {
    nav_layer: i32,
    tile_x: i32,
    tile_y: i32,
}

impl SceCommand for SceCommandNavigationBeeFlyTo {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        use crate::openpal3::directors::SceneManagerExtensions;
        use radiance::comdef::IEntityExt;
        let Some(scene) = scene_manager.scn_scene() else {
            return true;
        };
        let scene = scene.inner::<crate::openpal3::scene::ScnScene>();
        let Some(from) = scene_manager
            .get_resolved_role(state, -1)
            .map(|e| e.transform().borrow().position())
        else {
            return true;
        };
        let layer = self.nav_layer.rem_euclid(2) as usize;
        let (to, _) = scene.tile_point(layer, self.tile_x, self.tile_y);
        let speed = state.global_state().role_constants().walk_speed;
        scene.start_guide_bee(from, to, speed);
        true
    }
}

impl SceCommandNavigationBeeFlyTo {
    pub fn new(nav_layer: i32, tile_x: i32, tile_y: i32) -> Self {
        Self {
            nav_layer,
            tile_x,
            tile_y,
        }
    }
}

/// 101 当铺等级 · 120 Manage 类型 · 121 Period 类型 · 122 Accrual 类型 ·
/// 123 UnHock 类型 — `LAB_00436cb0` / `00436d30` / `00436db0` /
/// `00436e10` / `00436e70`。
///
/// 五个都是从当铺的存档状态里取一个数写进脚本变量：等级在状态块的 `+8`
/// （移植端由资本算出来，见 `states::hockshop::level_for_capital`），四个
/// 经营选择在 `+0x27844` / `+0x27838` / `+0x2783c` / `+0x27840`。
/// 详见 `switch/re/notes/hockshop.md`。
macro_rules! pawnshop_query {
    ($name:ident, $what:expr, $read:expr) => {
        #[doc = concat!("当铺状态查询：", $what, "。")]
        #[derive(Debug, Clone)]
        pub struct $name {
            var: i16,
        }

        impl $name {
            pub fn new(var: i16) -> Self {
                Self { var }
            }
        }

        impl SceCommand for $name {
            fn update(
                &mut self,
                _scene_manager: ComRc<ISceneManager>,
                _ui: &Ui,
                state: &mut SceState,
                _delta_sec: f32,
            ) -> bool {
                let read: fn(&crate::openpal3::states::persistent_state::PersistentState) -> i32 =
                    $read;
                let value = read(&state.global_state().persistent_state());
                vars::write(state, self.var, value);
                true
            }
        }
    };
}

pawnshop_query!(SceCommandGetPawnshopLevel, "当铺等级", |p| p
    .hockshop_level());
pawnshop_query!(SceCommandGetPawnshopManage, "Manage 类型", |p| p
    .hockshop_choice(0));
pawnshop_query!(SceCommandGetPawnshopPeriod, "Period 类型", |p| p
    .hockshop_choice(1));
pawnshop_query!(SceCommandGetPawnshopAccrual, "Accrual 类型", |p| p
    .hockshop_choice(2));
pawnshop_query!(SceCommandGetPawnshopUnHock, "UnHock 类型", |p| p
    .hockshop_choice(3));

/// 142 设置 VFX 特效位置 — `LAB_00438930`, three floats straight into
/// `DAT_00d8e478/47c/480` (0x43894b–0x438961). GameBox coordinates.
#[derive(Debug, Clone)]
pub struct SceCommandEffectPosition {
    x: f32,
    y: f32,
    z: f32,
}

impl SceCommand for SceCommandEffectPosition {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        state
            .global_state_mut()
            .set_effect_position(radiance::math::Vec3::new(self.x, self.y, self.z));
        true
    }
}

impl SceCommandEffectPosition {
    pub fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }
}

/// The Y the original adds when aiming a VFX at an actor — `fadd dword
/// ptr [0x56aa5c]` at 0x438a0e, and that constant is 2.0.
const EFFECT_ACTOR_Y_OFFSET: f32 = 2.0;

/// 144 在角色位置播放特效 — `LAB_004389d0`.
///
/// Despite the name it plays nothing: it copies the actor's position
/// (`+0xa0/a4/a8`) into the VFX position globals and raises Y by
/// [`EFFECT_ACTOR_Y_OFFSET`]. A **snapshot** — Pal3.Unity instead
/// parents the effect to the actor so it follows, and adds no offset.
/// The binary is what is reproduced here.
#[derive(Debug, Clone)]
pub struct SceCommandEffectAtRole {
    role_id: i32,
}

impl SceCommand for SceCommandEffectAtRole {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        use crate::openpal3::directors::SceneManagerExtensions;
        use radiance::comdef::IEntityExt;

        if let Some(entity) = scene_manager.get_resolved_role(state, self.role_id) {
            let mut position = entity.transform().borrow().position();
            position.y += EFFECT_ACTOR_Y_OFFSET;
            state.global_state_mut().set_effect_position(position);
        }
        true
    }
}

impl SceCommandEffectAtRole {
    pub fn new(role_id: i32) -> Self {
        Self { role_id }
    }
}
