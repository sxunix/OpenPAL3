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

/// Values the pawnshop query opcodes hand back while the game itself is
/// missing. Zero is the "nothing special" answer for each of the four
/// type queries and for the shop's level.
const PAWNSHOP_DEFAULT: i32 = 0;

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

/// 103 进入当铺经营游戏 — `LAB_00436c60`。
///
/// 参数是**过程号**，不是标志位：原版把它存进 `DAT_00dba244`，玩家从当铺
/// 出来时（`FUN_0051f1b0` 的按钮 0xa01b05）`FUN_00432170(DAT_00dba244, 0)`
/// 当脚本跑。实机唯一一处是 Q01 的 proc 1326「当铺投资」，传 1297。
///
/// 经营那一套还没移植（见 `switch/re/notes/hockshop.md`），所以这里只把
/// 那个过程跑起来，让剧情接得上。
#[derive(Debug, Clone)]
pub struct SceCommandMiniGamePawnshop {
    end_proc: i32,
}

impl SceCommandMiniGamePawnshop {
    pub fn new(end_proc: i32) -> Self {
        Self { end_proc }
    }
}

impl SceCommand for SceCommandMiniGamePawnshop {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        log::warn!("当铺经营还没做，直接跑收尾过程 {}", self.end_proc);
        state.show_note("当铺经营尚未实现，已跳过".to_string());
        if self.end_proc > 0 {
            state.call_proc(self.end_proc as u32);
        }
        true
    }
}

/// 157 显示一条提示 — `LAB_00438f10` → `FUN_00410360(文本)`。
///
/// 名字里的「当铺经营游戏」是误会：`FUN_00410360` 就是提示框那一路
/// （加减钱的 `FUN_0040fc90` 也拿它弹「获得 %d 文」），和 opcode 68 同一个
/// 控件。实机只有 Q12 的 proc 1302 用它，弹的是「景天精减少到1」这种话。
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
            state.show_note(self.text.clone());
        }
        true
    }
}
skipped!(
    SceCommandNavigationBeeFlyTo,
    "154 控制引路蜂飞向指定位置 — `LAB_00438ba0`，参数是 nav 层与 \
     tile 坐标。引路蜂是纯提示性的装饰，跳过不影响可玩性。",
    "",
    nav_layer: i32,
    tile_x: i32,
    tile_y: i32
);

/// 107 进入滑雪游戏 — `LAB_00436f40`，模式 11。见 `switch/re/notes/ski.md`。
///
/// 参数是过程号：原版存进 `DAT_00dba254`，滑完回到冒险模式时
/// `FUN_00432170(DAT_00dba254, 0)` 当脚本跑（`FUN_0052f350` 收尾那一支）。
/// 跳过游戏但不跑这个过程剧情就断了，所以这里直接跑它。
///
/// 滑道本身没做。原版滑的过程中最多能捡 **3 件**东西（`+0x50c` 三个槽，
/// 背包里已经有的不捡），跑完 `FUN_00532410` 才塞进背包，分数是
/// `件数 × 100 + (+0x11a0) × 30`。跳过这一段就等于这几件拿不到——不发是
/// 因为发什么取决于玩家怎么滑，凭空发等于替原版编结果。除此之外滑雪不动
/// 钱也不动精。
#[derive(Debug, Clone)]
pub struct SceCommandMiniGameSki {
    end_game_proc: i32,
}

impl SceCommandMiniGameSki {
    pub fn new(end_game_proc: i32) -> Self {
        Self { end_game_proc }
    }
}

impl SceCommand for SceCommandMiniGameSki {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        log::warn!(
            "SceCommandMiniGameSki: 滑道没做（途中的收集品也就拿不到），\
             直接跑收尾过程 {}",
            self.end_game_proc
        );
        if self.end_game_proc > 0 {
            state.call_proc(self.end_game_proc as u32);
        }
        true
    }
}

/// 101 当铺等级 · 120 Manage 类型 · 121 Period 类型 · 122 Accrual 类型 ·
/// 123 UnHock 类型 — `LAB_00436cb0` / `00436d30` / `00436db0` /
/// `00436e10` / `00436e70`.
///
/// All five read one byte of pawnshop state and assign it to a script
/// variable. With no pawnshop there is no state to read, so they write
/// [`PAWNSHOP_DEFAULT`] — leaving the variable untouched would let the
/// script branch on whatever the slot happened to hold.
macro_rules! pawnshop_query {
    ($name:ident, $what:expr) => {
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
                log::warn!(concat!("当铺未实现，", $what, " 取默认值"));
                vars::write(state, self.var, PAWNSHOP_DEFAULT);
                true
            }
        }
    };
}

pawnshop_query!(SceCommandGetPawnshopLevel, "当铺等级");
pawnshop_query!(SceCommandGetPawnshopManage, "Manage 类型");
pawnshop_query!(SceCommandGetPawnshopPeriod, "Period 类型");
pawnshop_query!(SceCommandGetPawnshopAccrual, "Accrual 类型");
pawnshop_query!(SceCommandGetPawnshopUnHock, "UnHock 类型");

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
