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

macro_rules! skipped {
    ($name:ident, $doc:expr $(, $field:ident : $ty:ty)*) => {
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
                _state: &mut SceState,
                _delta_sec: f32,
            ) -> bool {
                log::warn!(concat!(stringify!($name), ": 子系统未实现，跳过"));
                true
            }
        }
    };
}

skipped!(
    SceCommandMiniGamePawnshop,
    "103 进入当铺经营游戏 — `LAB_00436c60`.",
    flag: i32
);
skipped!(
    SceCommandMiniGameSwatAFly,
    "105 进入打苍蝇游戏 — `LAB_004370e0`.（无参数）"
);
skipped!(
    SceCommandMiniGameCaveExperience,
    "112 进入山洞初体验游戏 — `LAB_00437c20`.（无参数）"
);
skipped!(
    SceCommandMiniGamePawnshopNote,
    "157 当铺经营游戏显示信息 — `LAB_00438f10`.",
    text: String
);
skipped!(
    SceCommandDealerMenu,
    "100 调用买卖系统 — `LAB_00436c00`，参数是商店的数据文件名。\
     买卖界面本身还没有；跳过等于该商人不卖东西。",
    shop: String
);
skipped!(
    SceCommandSmeltingMenu,
    "146 打开冶炼菜单 — `LAB_00436a10`.",
    kind: i32
);
skipped!(
    SceCommandNavigationBeeFlyTo,
    "154 控制引路蜂飞向指定位置 — `LAB_00438ba0`，参数是 nav 层与 \
     tile 坐标。引路蜂是纯提示性的装饰，跳过不影响可玩性。",
    nav_layer: i32,
    tile_x: i32,
    tile_y: i32
);

/// 107 进入滑雪游戏 — `LAB_00436f40`, mode 11.
///
/// The one entry here with a consequence: the original stashes the
/// parameter in `DAT_00dba254` and, on the way out of the ski mode
/// (`FUN_0052f350`), runs it as a script — `FUN_00432170(DAT_00dba254,
/// 0)`. Skipping the game while dropping that call would strand the
/// plot, so the script is invoked directly instead.
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
            "SceCommandMiniGameSki: 滑雪游戏未实现，直接跑结束脚本 {}",
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
