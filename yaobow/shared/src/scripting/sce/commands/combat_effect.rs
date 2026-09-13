//! 143 播放战斗特效组 / 148 预载特效组 — `LAB_004389a0` / `LAB_00438980`.
//!
//! 两条都只是把参数转给特效管理器（`0xbfda78`）：
//!
//! ```text
//! 148: FUN_004ce460(mgr, groupId)                        // 预载
//! 143: FUN_004ce340(mgr, groupId, x, y, z, 0)            // 播放
//!      x/y/z 取自 [0xd8e478] / [0xd8e47c] / [0xd8e480]，
//!      也就是 142 CEft_Pos 和 144 CEft_Role 刚写进去的那个位置。
//! ```
//!
//! `FUN_004ce340` 的遍历在 `switch/re/notes/flexeff_format.md` 里解过：按
//! 15940 字节定位特效组，组头是条数，`+4` 是特效 id 数组，`+0x44` 是等长的
//! 槽位数组（每槽 8 个 124 字节子块），对每个启用的子块给特效实例发一次
//! `vfunc+0x30`。id 经 `FUN_004ce000` 查那张 1324 × 280 的定义表。
//!
//! # 这里做到哪一步
//!
//! 定义表、特效组表、15 个特效类的注册表都解开了，15 个类也都有了实现：两条指令把特效组
//! 交给导演（`AdventureDirector` 里按类名分派，particle / quad / cylinder / model / grid /
//! explosive / meteor / shock / audio / camera / action / time / quake / bink / callback），
//! 各类的画法在 `openpal3::scene::flexeff_*`。

use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

/// 子块 `+0x48`（72）——同一个特效的多份拷贝靠它错开起播时间。实机里同一
/// 组的 8 份 `fuzi.tga` 就是 0.0 / 0.1 / 0.2 / 0.3 / 0.4 / 0.5 这样排开的。
pub const SUB_DELAY: usize = 72;

/// 子块 `+0x4c`（76）——**时长**。`quad` 建实例时挂的两条时间线都拿它当
/// 时长（`FUN_004d3010` 的第 6 个参数），生成器管这段时间内生成几个 quad、
/// 补间器管这段时间内属性怎么变。
///
/// （先前怀疑它不是时长，是拿它去比 `languang` 的图集长度 32 帧 ÷ 16fps =
/// 2.0 秒——那两件事无关：图集按 fps 自己循环，时长管的是补间和生成。）
pub const SUB_LIFETIME: usize = 76;

/// 子块 `+0x54`（84）——生成器重复生成的间隔（秒），配合 [`SUB_REPEAT`]。
pub const SUB_REPEAT_INTERVAL: usize = 84;

/// 子块 `+0x59`（89）——`+0x58` 起四个标志字节的第 2 个：生成器是否在时长
/// 内反复生成。0x4d2e66 处 `mov al, byte [ebp+0x71]` 读的就是它。
pub const SUB_REPEAT: usize = 89;

/// 子块 `+0x50`（80）——`quad` 类的 `vfunc+0x30`（0x4d2090）把它读进对象的
/// `+0x38`，是这 124 字节里唯一一个已经确知去向的字段。
pub const SUB_QUAD_PARAM: usize = 80;

/// 把一个特效组的内容写进日志。143 和 148 都用它。
fn describe(state: &SceState, group_id: i32, what: &str) {
    let Some(eff) = state.asset_mgr().flexeff() else {
        log::warn!("{what}: flexeff.data 读不到，特效组 {group_id} 跳过");
        return;
    };
    let Some(group) = eff.group(group_id.max(0) as usize) else {
        log::warn!("{what}: 特效组 {group_id} 越界（只有 560 组）");
        return;
    };
    if !group.is_valid() {
        log::debug!("{what}: 特效组 {group_id} 是空的（计数 {}）", group.count());
        return;
    }

    for (k, id) in group.ids().iter().enumerate() {
        let Some(def) = eff.find(*id).and_then(|i| eff.definition(i)) else {
            log::warn!("{what}: 组 {group_id} 第 {k} 条的 id 0x{id:08x} 查不到定义");
            continue;
        };
        let asset = def.asset();
        let asset = if asset.is_empty() {
            def.type_name()
        } else {
            asset
        };
        let delays: Vec<f32> = (0..fileformats::pal3::flexeff::GROUP_SUBS)
            .filter(|s| group.sub_enabled(k, *s))
            .filter_map(|s| group.sub(k, s))
            .map(|b| f32::from_le_bytes(b[SUB_DELAY..SUB_DELAY + 4].try_into().unwrap()))
            .collect();
        log::debug!(
            "{what}: 组 {group_id} 第 {k} 条 {} 「{}」 延迟 {:?}",
            def.class().unwrap_or("?"),
            asset,
            delays
        );
    }
}

/// 143 CEft —— 在 142/144 设定的位置播一个特效组。
#[derive(Debug, Clone)]
pub struct SceCommandPlayCombatEffect {
    group_id: i32,
}

impl SceCommand for SceCommandPlayCombatEffect {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let position = state.global_state().effect_position();
        log::info!(
            "CEft: 播特效组 {} 于 ({:.2}, {:.2}, {:.2})",
            self.group_id,
            position.x,
            position.y,
            position.z
        );
        describe(state, self.group_id, "CEft");
        state.play_effect(self.group_id, position);
        // 原版这条不阻塞脚本——`FUN_004ce340` 建完实例就返回。
        true
    }
}

impl SceCommandPlayCombatEffect {
    pub fn new(group_id: i32) -> Self {
        Self { group_id }
    }
}

/// 148 CEft_Load —— 预载一个特效组。
#[derive(Debug, Clone)]
pub struct SceCommandPreloadCombatEffect {
    group_id: i32,
}

impl SceCommand for SceCommandPreloadCombatEffect {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        log::debug!("CEft_Load: 预载特效组 {}", self.group_id);
        describe(state, self.group_id, "CEft_Load");
        true
    }
}

impl SceCommandPreloadCombatEffect {
    pub fn new(group_id: i32) -> Self {
        Self { group_id }
    }
}
