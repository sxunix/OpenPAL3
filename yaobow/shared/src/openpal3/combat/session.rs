//! The turn loop: drives a [`Battle`] frame by frame, hands the party's
//! turns to a menu and the enemies' turns to the AI, and keeps what the
//! settlement needs. Nothing here knows about the renderer or the script
//! VM — the SCE `Combat` command owns the session and draws it.
//!
//! What the original does at this layer (`FUN_004055d0` case 2 → the
//! per-frame `FUN_004db7a0` / `FUN_004f7910` pair) is: fill gauges, let
//! whoever is ready act, test the outcome after every action. The menu
//! is the 控制盘: `FUN_005196c0` builds its buttons (see `hud::RingButton`), and which
//! of them can be pressed follows the 0x4dc7a0 rules ([`CombatSession::entry_enabled`]).
//! 合击 has no entry of its own: as in the original, a 仙术 turns into a 合击 by
//! itself when a combo's conditions hold (see [`super::combo`]).

use std::collections::VecDeque;

use super::actions::{ATTR_HP, ItemEffect, SPECIAL_COST_GU, SPECIAL_COST_MONEY, SkillEffect};
use super::ai::{self, Delegation, Kit, Plan, Tool};
use super::combo::{self, ComboDef};
use super::formulas::CombatRng;
use super::hud;
use super::settlement::{self, DefeatedEnemy, Spoils, Survivor};
use super::state::{
    Battle, Due, K_BERSERK, K_CONFUSED, K_FORBID, K_PARALYSIS, K_SEAL, K_SLEEP, Outcome,
    PARTY_SLOTS, SLOTS, Side,
};

/// A skill as the menu offers it.
#[derive(Debug, Clone)]
pub struct SkillChoice {
    pub name: String,
    /// `FUN_004f76c0` 给这一招的出手速度倍率：我方仙术是法术表里的等级 + 1，怪物是 gdb
    /// `skill_levels[i]`（不加 1），特技找不到算 0。0 在任务里当 1 用，> 3 瞬发。
    pub multiplier: i32,
    pub effect: SkillEffect,
}

impl SkillChoice {
    /// 打施术者自己这边吗：`target_range_type` 1/2 是，其余都不是。控制盘选目标时按
    /// `FUN_004df370` 把范围换成标志：1 → 0x10、2 → 2（我方），4 → 3、5 → 5、6 → 9，其余（含 0）
    /// → 0x11（bit0 敌方、单体）。
    pub fn targets_own_side(&self) -> bool {
        matches!(self.effect.target_range, 1 | 2)
    }

    /// 全体（2 我方全体 / 4 敌方全体）。原版出招函数按指令里的目标表逐位结算。
    pub fn hits_everyone(&self) -> bool {
        matches!(self.effect.target_range, 2 | 4)
    }

    /// 解「死亡」（状态枚举 24）的就是复活（回生照、烟水还魂、赎魂光……），倒下的也能选。
    pub fn revives(&self) -> bool {
        self.effect.state_codes[24] == 2
    }
}

/// A stack of items as the combat menu offers it (`FUN_004f95f0`).
#[derive(Debug, Clone)]
pub struct ItemChoice {
    pub name: String,
    /// How many are left to use this battle. 五灵珠 count their charges
    /// instead (`FUN_004f7140` spends a byte out of `DAT_00bfdb70..77`),
    /// which is what `pearl` marks.
    pub count: u32,
    pub pearl: bool,
    pub effect: ItemEffect,
}

impl ItemChoice {
    /// 我方的东西（`target_range_type` 1/2）打自己人，3/4 打对面。
    pub fn targets_own_side(&self) -> bool {
        matches!(self.effect.target_range, 1 | 2)
    }

    /// 全体（2 我方全体 / 4 敌方全体）。
    pub fn hits_everyone(&self) -> bool {
        matches!(self.effect.target_range, 2 | 4)
    }

    /// 解「死亡」（状态枚举 24）的就是复活药。
    pub fn revives(&self) -> bool {
        self.effect.state_codes[24] == 2
    }
}

/// What an enemy is worth when it falls (its database row's
/// `experience`, `money_when_killed`, `normal_loot`).
#[derive(Debug, Clone, Copy, Default)]
pub struct Reward {
    pub experience: i32,
    pub money: i32,
    pub loot: Option<(u32, i32)>,
}

/// One combatant's non-rules data: what to call it, what it can cast,
/// what it pays out.
#[derive(Debug, Clone, Default)]
pub struct Profile {
    pub name: String,
    /// 仙术（術）。
    pub skills: Vec<SkillChoice>,
    /// 特技（技）：`SkillType` 0 那一类，原版战斗位 `+0x2c8` 六格，按 id 排。
    pub specials: Vec<SkillChoice>,
    pub reward: Reward,
}

/// A button press, already mapped from keyboard or pad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Cancel,
}

/// The top-level menu entries, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuEntry {
    Attack,
    Skill,
    Item,
    Defend,
    Escape,
    /// 特技（控制盘的「技」，行动码 0x104）。
    Special,
    /// 托管面板（控制盘的「AI」）。
    Ai,
    /// 阵型面板（控制盘的「陣」）。
    Lineup,
    /// 援护面板（控制盘的「援」）。
    Protect,
}

impl MenuEntry {
    pub const ALL: [MenuEntry; 9] = [
        MenuEntry::Attack,
        MenuEntry::Skill,
        MenuEntry::Item,
        MenuEntry::Defend,
        MenuEntry::Escape,
        MenuEntry::Special,
        MenuEntry::Ai,
        MenuEntry::Lineup,
        MenuEntry::Protect,
    ];

    pub fn label(self) -> &'static str {
        match self {
            MenuEntry::Attack => "攻击",
            MenuEntry::Skill => "仙术",
            MenuEntry::Item => "道具",
            MenuEntry::Defend => "防御",
            MenuEntry::Escape => "逃跑",
            MenuEntry::Special => "特技",
            MenuEntry::Ai => "AI",
            MenuEntry::Lineup => "阵型",
            MenuEntry::Protect => "援护",
        }
    }
}

/// What the target cursor is choosing a target for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Attack,
    /// Index into the actor's skill list.
    Cast(usize),
    /// Index into the actor's 特技 list.
    Special(usize),
    /// Index into [`CombatSession::items`].
    UseItem(usize),
}

/// Where the cursor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Menu {
    Main {
        index: usize,
    },
    Skills {
        index: usize,
    },
    Specials {
        index: usize,
    },
    /// 托管面板（`FUN_0051b210`）。`focus` 0..=5 是六个勾选框（按
    /// [`ai::Switch::ALL`]），[`hud::AI_OK_FOCUS`] 是「确定」；`draft` 是面板上
    /// 现在的勾，按了确定才写进战斗位。
    Ai {
        focus: usize,
        draft: Delegation,
    },
    /// 阵型面板（`FUN_0051b710`）。`cursor` 是光标所在的五灵位（1..=6）；`held`
    /// 是抓在手上的我方战斗位；`moved` 记这次开面板有没有真换过位。
    Lineup {
        cursor: u8,
        held: Option<usize>,
        moved: bool,
    },
    /// 援护面板（`FUN_0051bc70`）。`focus` 0..=2 是雪见、龙葵、紫萱三格，
    /// [`hud::PROTECT_OK_FOCUS`] 是「确定」；`draft` 是面板上勾着的战斗位。
    Protect {
        focus: usize,
        draft: Option<usize>,
    },
    /// `index` is into [`CombatSession::items`].
    Items {
        index: usize,
    },
    /// `index` is into [`CombatSession::targets`].
    Target {
        action: Action,
        index: usize,
    },
}

/// A party member waiting for the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerTurn {
    pub slot: usize,
    pub menu: Menu,
}

/// 界面音效（`scripting::sce::commands::ui_sound` 里同名的那几个）。
const SFX_CLICK: &str = "wa001";
const SFX_NOTE: &str = "wa003";
const SFX_DENIED: &str = "wa004";
/// 情加了（`FUN_004f34e0` 正数那一支）。
/// 谁替这个位挑行动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiMode {
    /// 我方托管 / 乱 / 狂（`FUN_004d86d0`）。
    Delegate,
    /// 怪物中了乱（`FUN_004d8580`）。
    Confused,
    /// 怪物的阶梯（`FUN_004d7120`）。
    Monster,
    /// 快进代打：我方按六个开关全开托管。
    AutoPlay,
}

impl AiMode {
    fn label(self) -> &'static str {
        match self {
            Self::Delegate => "托管",
            Self::Confused => "乱",
            Self::Monster => "怪物",
            Self::AutoPlay => "代打",
        }
    }
}

const SFX_AFFECTION: &str = "wa010";

/// 普攻暴击的特效组（`FUN_004ce340(0x1d6, …)`）。
pub const EFFECT_CRIT: i32 = 0x1d6;

/// `FUN_0040d010` 的类别 0xe：饿鬼能吃的「回复类」道具。
fn is_restorative(e: &ItemEffect) -> bool {
    e.kind == 1
        && (e.state_codes[24] == 2 || e.impacts[..3].iter().any(|(kind, v)| *kind == 2 || *v > 0))
}

/// 合击技开演用的出手速度倍率：大于 3 就是瞬发（`FUN_00514870` 里 `FUN_004db750(k, 0, 11500, 1)`）。
const COMBO_MULTIPLIER: i32 = 4;

/// How many log lines are kept for the overlay.
pub const LOG_LINES: usize = 8;

/// How long a transient notice (逃跑失败 and the like) stays up.
const NOTICE_SECONDS: f32 = 1.2;

/// 已经选定、正在出手段等待生效的行动。原版把这个记在行动任务的记录里
/// （`FUN_00514870` 的 `task->+0x2c`/`+0x28`），出手条满了才真正结算。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pending {
    Attack {
        slot: usize,
        target: usize,
    },
    Cast {
        slot: usize,
        target: usize,
        skill: usize,
        /// 特技（行动码 0x104，`FUN_004f1420`）而不是仙术（0x105，`FUN_004f1070`）：
        /// `skill` 是特技列表的下标。
        special: bool,
    },
    Item {
        slot: usize,
        target: usize,
        item: usize,
    },
    Escape {
        slot: usize,
    },
    /// 合击技（行动码 0x10e）。`combo` 是 [`CombatSession::combos`] 的下标，`target` 是
    /// 原先那个仙术选的目标。
    Combo {
        slot: usize,
        target: usize,
        combo: usize,
    },
}

impl Pending {
    pub fn slot(&self) -> usize {
        match self {
            Pending::Attack { slot, .. }
            | Pending::Cast { slot, .. }
            | Pending::Item { slot, .. }
            | Pending::Combo { slot, .. }
            | Pending::Escape { slot } => *slot,
        }
    }
}

/// 出手条满了、交给舞台演的那一次行动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Staging {
    pub slot: usize,
    /// 已经在出手那一刻结算过了，只等人走回去。
    pub landed: bool,
}

#[derive(Clone)]
pub struct CombatSession {
    pub battle: Battle,
    profiles: Vec<Profile>,
    /// The AI's own generator, separate from the rules' (`0x4f4...`
    /// reads `DAT_00c11760`, the combat one reads the CRT seed).
    ai_rng: CombatRng,
    /// 界面音效，调用方每帧取走播放（原版 `FUN_0053df70(名字, 1)`）。
    pub sfx: Vec<&'static str>,
    /// 普攻没打着、要播回避的战斗位，舞台每帧取走（`FUN_004f0a10` 排 `FUN_004feef0`）。
    pub dodged: Vec<usize>,
    /// 要战斗镜头播的 flexcam 序列（`FUN_00505770`），调用方每帧取走。
    pub camera_requests: Vec<i32>,
    /// 要放的特效组和放在哪个战斗位身上（`FUN_004ce340`），舞台每帧取走。
    pub effects: Vec<(i32, usize)>,
    /// 舞台每帧填：这个战斗位的人离自己的原位超过 13（`FUN_0050a700` 为否：当前位置
    /// `+0x19b8` 到原位 `+0x19c4` 的距离）。仙术 / 特技结算时用（[`Self::cast`]）。
    pub away_from_home: [bool; SLOTS],
    /// 饿鬼整格吃掉的道具（id, 个数），散场时从背包里扣。
    pub eaten_items: Vec<(u32, u32)>,
    pub elapsed: f32,
    pub log: VecDeque<String>,
    pub turn: Option<PlayerTurn>,
    pub notice: Option<(String, f32)>,
    defeated: Vec<DefeatedEnemy>,
    /// Every skill the party cast, `(slot, skill id)`, for the use
    /// counters the settlement levels skills with.
    pub casts: Vec<(usize, u32)>,
    /// 战斗菜单里的道具，开战时从背包抓的一份快照。
    items: Vec<ItemChoice>,
    /// 用掉的道具 id，散场时写回背包（五灵珠扣充能，其余扣数量）。
    pub used_items: Vec<u32>,
    /// 合击技表（gdb 里的顺序），开战时给。
    combos: Vec<ComboDef>,
    /// 选好了、还在出手段的行动，见 [`Pending`]，每个位至多一个，按选定的先后排。
    ///
    /// 原版每个行动是一条单开的虚拟线程（`CCBSystem::AddThread`），出手段各走各的；舞台一次只演一个
    /// （`CCBStage::ProcGrabStageRole` 抢舞台），这里由 [`Self::staging`] 管。主调度
    /// `ProcControl`（`combat/cbsystem.cpp:910`）每帧把 11 个位过一遍，谁到了出招点就挑行动，
    /// **不等别人的行动结算完**。先前这里只容一个、别人在出手段时轮到的人先等着：我方四五个人轮流
    /// 满条，敌人常常一直轮不上（大渡口之后 M06 的虾兵、龟丞相整场没出过手）。
    /// 逃跑不占这一格，见 [`Self::escape`]。
    pub pending_actions: Vec<Pending>,
    /// 按了「逃」、正在等逃跑蓄力满的那一位（原版 `CCBSystem::ProcFleeWait`
    /// 是条**单开的虚拟线程**，不占行动任务那一格：等待期间别人照样出手，
    /// 所以这里也单开一格，逃跑的人不会把全场挂住）。
    pub escape: Option<usize>,
    /// 交给舞台演出的那一次行动（见 [`CombatSession::stage_driven`]）。
    pub staging: Option<Staging>,
    /// 出手条满时先别结算，等舞台演到出手那一刻再调 [`CombatSession::strike`]。
    ///
    /// 原版：出手条满后 `FUN_00514870` 把任务切到 0x514ae0，那里冻结出手者
    /// 自己的行动条（`FUN_004db920(k, 1)`），排走位 / 出手 / 结算一串步骤，
    /// 结算由步骤按时间点触发。**演出期间移植端整个时钟停住**——原版 0x514ae0
    /// 只冻结出手者，别人的条照走；移植端同一时刻只挂得住一个行动，所以简化。
    /// 快进和测试不开这个开关，行为和以前一样。
    pub stage_driven: bool,
    finished: Option<Outcome>,
}

impl CombatSession {
    /// 战斗中新造出来的怪要补一份名片（分身、召唤）。
    pub fn set_profile(&mut self, slot: usize, profile: Profile) {
        if slot < SLOTS {
            self.profiles[slot] = profile;
        }
    }

    /// `profiles` is indexed by battle slot; an unused slot's profile is
    /// ignored.
    pub fn new(battle: Battle, profiles: Vec<Profile>, ai_seed: u32) -> Self {
        let mut profiles = profiles;
        profiles.resize_with(SLOTS, Profile::default);
        Self {
            battle,
            profiles,
            ai_rng: CombatRng::new(ai_seed),
            sfx: Vec::new(),
            dodged: Vec::new(),
            effects: Vec::new(),
            away_from_home: [false; SLOTS],
            eaten_items: Vec::new(),
            camera_requests: Vec::new(),
            elapsed: 0.,
            log: VecDeque::new(),
            turn: None,
            notice: None,
            pending_actions: Vec::new(),
            escape: None,
            staging: None,
            stage_driven: false,
            defeated: Vec::new(),
            casts: Vec::new(),
            items: Vec::new(),
            used_items: Vec::new(),
            combos: Vec::new(),
            finished: None,
        }
    }

    /// Hand the session the party's usable items — what the 道具 menu
    /// lists. Built by the caller from the inventory because the rules
    /// layer doesn't know the item database.
    pub fn set_items(&mut self, items: Vec<ItemChoice>) {
        self.items = items;
    }

    pub fn items(&self) -> &[ItemChoice] {
        &self.items
    }

    /// 合击技表，按数据库里的顺序（配的时候先到先得）。
    pub fn set_combos(&mut self, combos: Vec<ComboDef>) {
        self.combos = combos;
    }

    pub fn combos(&self) -> &[ComboDef] {
        &self.combos
    }

    /// 调试：轮到我方有人选行动时，不看条件直接放 `id` 这条合击技，打第一个还站着的
    /// 敌人。不是我方回合、或者表里没有这条时返回 false。
    pub fn debug_combo(&mut self, id: u32) -> bool {
        let Some(slot) = self.turn.map(|t| t.slot) else {
            return false;
        };
        let Some(combo) = self.combos.iter().position(|c| c.id == id) else {
            return false;
        };
        let Some(target) = self.standing(Side::Enemy).first().copied() else {
            return false;
        };
        self.turn = None;
        self.push_log(format!("合击「{}」（调试）", self.combos[combo].name));
        self.commit(
            Pending::Combo {
                slot,
                target,
                combo,
            },
            COMBO_MULTIPLIER,
        );
        true
    }

    /// 还有没有得用（`FUN_004f6fb0`：拿着就行，五灵珠看充能）。
    pub fn item_available(&self, i: usize) -> bool {
        self.items.get(i).map(|it| it.count > 0).unwrap_or(false)
    }

    pub fn name(&self, slot: usize) -> &str {
        &self.profiles[slot].name
    }

    pub fn skills(&self, slot: usize) -> &[SkillChoice] {
        &self.profiles[slot].skills
    }

    pub fn specials(&self, slot: usize) -> &[SkillChoice] {
        &self.profiles[slot].specials
    }

    /// 登记的这次施放用的是哪条（特技 / 仙术各查各的列表）。
    pub fn cast_choice(&self, slot: usize, skill: usize, special: bool) -> Option<&SkillChoice> {
        if special {
            self.specials(slot).get(skill)
        } else {
            self.skills(slot).get(skill)
        }
    }

    pub fn finished(&self) -> Option<Outcome> {
        self.finished
    }

    /// Slots on `side` that are still standing, in slot order.
    pub fn standing(&self, side: Side) -> Vec<usize> {
        (0..SLOTS)
            .filter(|s| {
                self.battle
                    .get(*s)
                    .map(|c| c.side == side && c.in_play())
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Slots on `side` that are on the field at all, standing or not —
    /// what a 复活 item may be aimed at.
    pub fn placed(&self, side: Side) -> Vec<usize> {
        (0..SLOTS)
            .filter(|s| {
                self.battle
                    .get(*s)
                    .map(|c| c.side == side && !c.retired)
                    .unwrap_or(false)
            })
            .collect()
    }

    /// Who the current target cursor may land on.
    pub fn targets(&self) -> Vec<usize> {
        let Some(PlayerTurn {
            slot,
            menu: Menu::Target { action, .. },
        }) = self.turn
        else {
            return Vec::new();
        };
        // 道具看的是 `target_range_type`，不是效果的正负；能复活的那些还要
        // 把倒下的人算进来（`FUN_004f1a30` 的复活支）。
        if let Action::UseItem(i) = action {
            let Some(item) = self.items.get(i) else {
                return Vec::new();
            };
            let side = if item.targets_own_side() {
                Side::Party
            } else {
                Side::Enemy
            };
            return if item.revives() {
                self.placed(side)
            } else {
                self.standing(side)
            };
        }
        let choice = match action {
            // 普攻（也只有普攻）过 `CanBeSelect` 里那道「挡在前面的人」检查
            // （`cbrolestate.cpp:1933`）：10 号位还在就选不到 6 号位。
            Action::Attack => {
                return self
                    .standing(Side::Enemy)
                    .into_iter()
                    .filter(|t| self.battle.attackable(slot, *t))
                    .collect();
            }
            Action::Cast(i) => self.skills(slot).get(i),
            Action::Special(i) => self.specials(slot).get(i),
            Action::UseItem(_) => unreachable!("道具在上面单独处理"),
        };
        let Some(choice) = choice else {
            return Vec::new();
        };
        let side = if choice.targets_own_side() {
            Side::Party
        } else {
            Side::Enemy
        };
        if choice.revives() {
            self.placed(side)
        } else {
            self.standing(side)
        }
    }

    /// 选目标时亮起来的战斗位：目标选择 0x4fd3d0 用 `FUN_00503ae0` 按指令标志把光标展开
    /// 进 `+0x5c`。标志在控制盘状态 `FUN_004dc760` 里定：仙术 / 特技 / 道具按
    /// `target_range`（2 / 4 全体 → 整边能选的，其余只有光标那个）；普攻 1 | 横排 4（`+0x289`）
    /// / 竖列 8（`+0x28a`）/ 单体 0x10，横排竖列用的就是结算用的 `FUN_004efd70` /
    /// `FUN_004efe90`，所以等于 [`Battle::attack_targets`]。
    pub fn highlighted(&self) -> Vec<usize> {
        let Some(PlayerTurn {
            slot,
            menu: Menu::Target { action, index },
        }) = self.turn
        else {
            return Vec::new();
        };
        let targets = self.targets();
        let Some(&cursor) = targets.get(index) else {
            return Vec::new();
        };
        let everyone = match action {
            Action::Attack => return self.battle.attack_targets(slot, cursor),
            Action::Cast(i) => self.skills(slot).get(i).is_some_and(|c| c.hits_everyone()),
            Action::Special(i) => self
                .specials(slot)
                .get(i)
                .is_some_and(|c| c.hits_everyone()),
            Action::UseItem(i) => self.items().get(i).is_some_and(|c| c.hits_everyone()),
        };
        if everyone { targets } else { vec![cursor] }
    }

    /// Whether `slot` can pay for its `i`-th skill right now.
    ///
    /// 气/神那两项是 `FUN_004f7480`；第三项消耗是 `FUN_004f7200` 的
    /// switch（分派在 0x4f7267）：`-1` 和 0xb..0xf（五灵）无条件放行，10 要
    /// 钱够，0x10 要背包里有蛊（不看量），其余拿定义值和对应属性比。实机数据
    /// 只用到 10 和 0x10 两种。
    pub fn can_afford(&self, slot: usize, i: usize) -> bool {
        self.skills(slot)
            .get(i)
            .is_some_and(|s| self.affordable(slot, s))
    }

    /// 同 [`Self::can_afford`]，查特技列表（特技也走 `FUN_004f7480` / `FUN_004f7200`，
    /// `FUN_004f91b0` 画列表时拿它们定亮不亮）。
    pub fn can_afford_special(&self, slot: usize, i: usize) -> bool {
        self.specials(slot)
            .get(i)
            .is_some_and(|s| self.affordable(slot, s))
    }

    fn affordable(&self, slot: usize, s: &SkillChoice) -> bool {
        let Some(c) = self.battle.get(slot) else {
            return false;
        };
        if c.sp < s.effect.sp_cost || c.mp < s.effect.mp_cost {
            return false;
        }
        match s.effect.special_cost_type {
            SPECIAL_COST_MONEY => self.battle.money_left() >= s.effect.special_cost_value,
            SPECIAL_COST_GU => self.battle.gu_left() > 0,
            _ => true,
        }
    }

    /// Advance the battle by `dt` seconds. Returns the outcome once the
    /// battle is over; `None` while it runs or waits for the player.
    pub fn tick(&mut self, dt: f32) -> Option<Outcome> {
        if let Some(o) = self.finished {
            return Some(o);
        }
        if let Some((_, ttl)) = self.notice.as_mut() {
            *ttl -= dt;
            if *ttl <= 0. {
                self.notice = None;
            }
        }
        // 逃跑等待期间中了「禁」，和挨打一样把这一次逃跑打掉
        // （`CCBSystem::ProcFleeWait`，`cbsystem.cpp:1960`；用道具逃的打不掉，
        // 移植端的道具逃走 [`crate::openpal3::combat::actions`] 那一路不经过这里）。
        if let Some(slot) = self.escape {
            if self.battle.get(slot).is_some_and(|c| c.has_state(K_FORBID)) {
                self.battle.finish_turn(slot);
                self.battle.flee_broken = Some(slot);
            }
        }
        self.drop_broken_escape();
        if self.staging.is_some() {
            return None;
        }
        // 胜负判定 `FUN_004f7910` 每帧都跑（`FUN_004ecb00`），不管控制面板开没开；分出来了
        // `ProcControl`（`cbsystem.cpp:920`）就把面板强收回去。玩家正选目标时最后一个敌人
        // 跑出场，面板不收的话就停在一个没有目标可选的菜单上。
        if self.settle_outcome() {
            return self.finished;
        }
        if self.turn.is_some() {
            return None;
        }

        self.elapsed += dt;
        self.battle.advance(dt);
        // 逃走（`Battle::own_turn` 掷中）：有舞台时舞台演跑、跑到了才 `finish_fleeing`；没舞台（快进、
        // 测试）就当场跑完——不然逃的人一直算在场又不出手，最后一个敌人逃时战斗停不下来。
        if !self.stage_driven {
            for slot in 0..SLOTS {
                if self.battle.get(slot).is_some_and(|c| c.fleeing) {
                    self.battle.finish_fleeing(slot);
                }
            }
        }
        // 原版主调度每帧把 11 个位都过一遍：谁没挑出行动就下一帧再试（不清行动条），
        // 不耽误后面的人。
        let mut waiting: Vec<usize> = Vec::new();
        loop {
            let Some(due) = self.battle.due(&waiting) else {
                return None;
            };

            match due {
                // 出手条满了：把先前选定的行动结算掉。
                Due::Executed(slot) => {
                    let staged = self.stage_driven && self.pending_for(slot).is_some();
                    if staged {
                        self.staging = Some(Staging {
                            slot,
                            landed: false,
                        });
                        return None;
                    }
                    self.resolve_pending(slot);
                    self.drop_broken_escape();
                    self.settle_outcome();
                    return self.finished;
                }
                Due::Ready(slot) => {
                    // 调度器一次只排一个行动任务：有人还在出手段（登记的行动没结算），轮到的人
                    // 这一帧先等着，下一帧再看——出手完成的那个优先。
                    // 原版 `ProcControl` 挑行动不看别人在不在出手段（见 `pending_actions`），
                    // 只有舞台一次演一个（上面 `staging` 时整段不跑）。
                    let actor = self
                        .battle
                        .get(slot)
                        .expect("update returned a placed slot");
                    let side = actor.side;
                    let confused = actor.has_state(K_CONFUSED);
                    let asleep = actor.has_state(K_SLEEP);
                    let paralysed = actor.has_state(K_PARALYSIS);
                    let berserk = actor.has_state(K_BERSERK);
                    let delegated = actor.delegation.on;

                    // 主调度 `LAB_00513840`（0x513925 起）：眠（0x11）、定（0x0e）→ 0x513ab8，
                    // 只有一句 `FUN_004db750(k, 0, 0, 0)`，这一回合跳过。我方：托管开关 `+0x345`、
                    // 乱（0x12）、狂（0x13）→ 0x513a36 走 `FUN_004d86d0`（乱在里面走随机的那两支）。
                    // 怪物走 0x513a48 那条：一律 `FUN_004d7120`，乱在里面转 `FUN_004d8580`。
                    // 哪条挑不出行动都是下一帧再试（0x513ac7，不清行动条）。
                    let acted = if asleep || paralysed {
                        let why = if asleep { "睡着了" } else { "定住了" };
                        self.push_log(format!("{} {why}", self.name(slot)));
                        self.battle.finish_turn(slot);
                        true
                    } else if side == Side::Party && (delegated || confused || berserk) {
                        self.act_by_delegate(slot)
                    } else if side == Side::Enemy && confused {
                        self.act_confused(slot)
                    } else if side == Side::Enemy {
                        self.act_by_ai(slot, false)
                    } else {
                        self.turn = Some(PlayerTurn {
                            slot,
                            menu: Menu::Main { index: 0 },
                        });
                        return None;
                    };
                    if !acted {
                        waiting.push(slot);
                        continue;
                    }
                }
            }
            break;
        }
        self.settle_outcome();
        self.finished
    }

    /// 最早选定、还在出手段的那个行动（测试和快照用）。
    pub fn pending(&self) -> Option<Pending> {
        self.pending_actions.first().copied()
    }

    /// 第 `slot` 位选定、还在出手段的行动。
    pub fn pending_for(&self, slot: usize) -> Option<Pending> {
        self.pending_actions
            .iter()
            .find(|p| p.slot() == slot)
            .copied()
    }

    /// 舞台演到出手那一刻：把交出去的行动结算掉。重复调用无害。
    pub fn strike(&mut self) {
        let Some(st) = self.staging.as_mut() else {
            return;
        };
        if st.landed {
            return;
        }
        st.landed = true;
        let slot = st.slot;
        self.resolve_pending(slot);
        self.settle_outcome();
    }

    /// 普攻除最后一下之外的那几下（动作里 `work` / `work1` 两个标记、龙葵的武器连击，见
    /// `motion::attack_events`）：照样结算一次 `FUN_004f0a10`，行动留着等最后一下 [`Self::strike`]。
    pub fn attack_hit(&mut self) {
        let Some(st) = self.staging else {
            return;
        };
        if st.landed {
            return;
        }
        if let Some(Pending::Attack { slot, target }) = self.pending_for(st.slot) {
            {
                self.attack(slot, target);
                self.settle_outcome();
            }
        }
    }

    /// 这一位此刻在演的行动：登记的那个行动，或者正在等的那次逃跑。
    pub fn acting(&self, slot: usize) -> Option<Pending> {
        if self.escape == Some(slot) {
            return Some(Pending::Escape { slot });
        }
        self.pending_for(slot)
    }

    /// 逃跑等待被打掉：把这一次逃跑作废，人回蓄力段重新选。原版在
    /// `ProcFleeWait`（`cbsystem.cpp:1972`）发现序列器回到第一槽时弹「逃跑失败」。
    fn drop_broken_escape(&mut self) {
        let Some(slot) = self.battle.flee_broken.take() else {
            return;
        };
        if self.escape != Some(slot) {
            return;
        }
        self.escape = None;
        self.push_log(format!("{} 逃跑被打断", self.name(slot)));
        self.notice("逃跑失败");
    }

    /// 调试：场上的敌人全部倒下，下一帧按「敌全倒」判胜、照常结算。
    pub fn debug_win(&mut self) {
        for slot in 0..crate::openpal3::combat::state::SLOTS {
            if let Some(c) = self.battle.get_mut(slot) {
                if c.side == Side::Enemy && c.alive {
                    c.die();
                }
            }
        }
        self.battle.pending_spawns.clear();
        self.battle.pending_stage = None;
    }

    /// 舞台不演了、这一次行动作废：不结算，直接回蓄力段（我方普攻开演时目标全倒下）。
    pub fn abandon(&mut self) {
        let Some(st) = self.staging.take() else {
            return;
        };
        self.pending_actions.retain(|p| p.slot() != st.slot);
        self.battle.finish_turn(st.slot);
    }

    /// 舞台演完（人走回去了）：放开时钟。没结算过的先结算。
    pub fn release(&mut self) {
        self.strike();
        self.staging = None;
    }

    /// 出手段结束，行动生效。原版在这一步之后一律走 `FUN_004f4820` →
    /// `FUN_004db710`（两条清零、回蓄力段）。
    fn resolve_pending(&mut self, slot: usize) {
        if self.escape == Some(slot) {
            self.escape = None;
            let ok = self.battle.try_escape(slot);
            self.push_log(format!(
                "{} 逃跑{}",
                self.name(slot),
                if ok { "成功" } else { "失败" }
            ));
            if !ok {
                self.notice("逃跑失败");
            }
            self.battle.finish_turn(slot);
            return;
        }
        let Some(i) = self.pending_actions.iter().position(|p| p.slot() == slot) else {
            // 没有登记的行动（例如被打断后又自己走完了出手段），当作空过。
            self.battle.finish_turn(slot);
            return;
        };
        let p = self.pending_actions.remove(i);
        match p {
            Pending::Attack { slot, target } => self.attack(slot, target),
            Pending::Cast {
                slot,
                target,
                skill,
                special,
            } => self.cast(slot, target, skill, special),
            Pending::Item { slot, target, item } => self.use_item(slot, target, item),
            Pending::Combo {
                slot,
                target,
                combo,
            } => self.combo(slot, target, combo),
            // 逃跑不走这一格，上面已经接走。
            Pending::Escape { .. } => {}
        }
        self.battle.finish_turn(slot);
    }

    /// 登记一个行动并进入出手段。`multiplier` 是 `FUN_004f76c0` 给的出手速度
    /// 倍率，普通攻击没有技能可查，按 1 走。
    fn commit(&mut self, action: Pending, multiplier: i32) {
        // 合击技：原版在行动任务开头（`FUN_00514870`）先问 `FUN_004eea70`，配上了就把
        // 行动改成 0x10e、复位行动条、交给 0x514220 直接开演，不走出手段。这里用瞬发
        // （倍率 > 3，出手条直接满）让它下一帧就开演 [近似]。
        let (action, multiplier) = match self.combo_for(&action) {
            Some((combo, target)) => {
                let slot = action.slot();
                self.push_log(format!("合击「{}」", self.combos[combo].name));
                (
                    Pending::Combo {
                        slot,
                        target,
                        combo,
                    },
                    COMBO_MULTIPLIER,
                )
            }
            None => (action, multiplier),
        };
        let slot = action.slot();
        match action {
            Pending::Escape { .. } => {
                self.escape = Some(slot);
                self.battle.begin_escape(slot);
            }
            _ => {
                self.pending_actions.retain(|p| p.slot() != slot);
                self.pending_actions.push(action);
                self.battle.begin_execute(slot, multiplier);
            }
        }
    }

    /// Let the AI take the pending player turn — 快进 while a script is
    /// being skipped.
    pub fn auto_play(&mut self) {
        let Some(PlayerTurn { slot, .. }) = self.turn.take() else {
            return;
        };
        let confused = self
            .battle
            .get(slot)
            .map(|c| c.has_state(K_CONFUSED))
            .unwrap_or(false);
        // 挑不出来就还是就绪，下一帧又轮到他、又快进一次。
        let _ = self.act_by_ai(slot, confused);
        self.settle_outcome();
    }

    /// Feed one button press into the menu. Ignored when it is not the
    /// player's turn.
    pub fn input(&mut self, input: Input) {
        let Some(mut turn) = self.turn else {
            return;
        };
        let slot = turn.slot;
        match turn.menu {
            Menu::Main { index } => match input {
                Input::Up | Input::Down | Input::Left | Input::Right => {
                    // 焦点在控制盘的九宫格上走，见 `hud::ring_move`。
                    let step = match input {
                        Input::Up => (0, -1),
                        Input::Down => (0, 1),
                        Input::Left => (-1, 0),
                        _ => (1, 0),
                    };
                    let from = hud::RingButton::of_entry(MenuEntry::ALL[index]);
                    let to = hud::ring_move(from, step, |b| {
                        b.entry().is_some_and(|e| self.entry_enabled(slot, e))
                    });
                    if let Some(i) = to
                        .entry()
                        .and_then(|e| MenuEntry::ALL.iter().position(|x| *x == e))
                    {
                        turn.menu = Menu::Main { index: i };
                    }
                }
                Input::Confirm => match MenuEntry::ALL[index] {
                    MenuEntry::Attack => {
                        turn.menu = Menu::Target {
                            action: Action::Attack,
                            index: 0,
                        };
                    }
                    MenuEntry::Skill => {
                        if self.skills(slot).is_empty() {
                            self.notice("没有可用的仙术");
                        } else {
                            turn.menu = Menu::Skills { index: 0 };
                        }
                    }
                    MenuEntry::Special => {
                        if self.specials(slot).is_empty() {
                            self.notice("没有可用的特技");
                        } else {
                            turn.menu = Menu::Specials { index: 0 };
                        }
                    }
                    MenuEntry::Item => {
                        if self.items.is_empty() {
                            self.notice("没有可用的道具");
                        } else {
                            turn.menu = Menu::Items { index: 0 };
                        }
                    }
                    MenuEntry::Ai => {
                        // 打开面板放 wa003（0x4dd69b），勾选框按战斗位现在的开关摆
                        // （`FUN_004f9790`）。
                        let draft = self
                            .battle
                            .get(slot)
                            .map(|c| c.delegation)
                            .unwrap_or_default();
                        self.sfx.push(SFX_NOTE);
                        turn.menu = Menu::Ai { focus: 0, draft };
                    }
                    MenuEntry::Lineup => {
                        let cursor = self
                            .battle
                            .get(slot)
                            .map(|c| c.element_position)
                            .filter(|p| (1..=6).contains(p))
                            .unwrap_or(1);
                        self.sfx.push(SFX_NOTE);
                        // 打开阵型面板播 6 号序列（0x4ddd6d）：镜头绕着战场慢慢飘。
                        self.camera_requests.push(6);
                        turn.menu = Menu::Lineup {
                            cursor,
                            held: None,
                            moved: false,
                        };
                    }
                    MenuEntry::Protect => {
                        if self.entry_enabled(slot, MenuEntry::Protect) {
                            // 打开（0x4de8ea）：护的对象不在 1..=3 就当没有；放 wa003。
                            let draft = self
                                .battle
                                .get(0)
                                .and_then(|c| c.protecting)
                                .filter(|t| (1..=3).contains(t));
                            self.sfx.push(SFX_NOTE);
                            turn.menu = Menu::Protect { focus: 0, draft };
                        } else {
                            self.sfx.push(SFX_DENIED);
                        }
                    }
                    MenuEntry::Defend => {
                        self.turn = None;
                        self.battle.defend(slot);
                        self.push_log(format!("{} 防御", self.name(slot)));
                        self.battle.finish_turn(slot);
                        self.settle_outcome();
                        return;
                    }
                    MenuEntry::Escape => {
                        // 逃跑也要走一段出手，速度是 速_eff*8+20
                        // （`LAB_00515200` 的 0x515295）。
                        self.turn = None;
                        self.commit(Pending::Escape { slot }, 1);
                        return;
                    }
                },
                Input::Cancel => {}
            },
            Menu::Ai { focus, mut draft } => {
                let back = MenuEntry::ALL
                    .iter()
                    .position(|e| *e == MenuEntry::Ai)
                    .unwrap_or(0);
                match input {
                    // 两列三行，确定在最底下。
                    Input::Up => {
                        let focus = match focus {
                            hud::AI_OK_FOCUS => 4,
                            f if f >= 2 => f - 2,
                            f => f,
                        };
                        turn.menu = Menu::Ai { focus, draft };
                    }
                    Input::Down => {
                        let focus = if focus >= 4 {
                            hud::AI_OK_FOCUS
                        } else {
                            focus + 2
                        };
                        turn.menu = Menu::Ai { focus, draft };
                    }
                    Input::Left | Input::Right => {
                        if focus < hud::AI_OK_FOCUS {
                            turn.menu = Menu::Ai {
                                focus: focus ^ 1,
                                draft,
                            };
                        }
                    }
                    Input::Confirm if focus < hud::AI_OK_FOCUS => {
                        let ok = draft.toggle(ai::Switch::ALL[focus]);
                        self.sfx.push(if ok { SFX_CLICK } else { SFX_DENIED });
                        turn.menu = Menu::Ai { focus, draft };
                    }
                    Input::Confirm => {
                        // 确定（0xdc，0x4dda28）：放 wa001，六个开关写进战斗位；和打开时比有变化就置
                        // `+0x60`，把 0x10a 当这一回合的行动（0x513b3f → 任务 0x5158f0，只做
                        // `FUN_004db710` 两条清零）——这一回合用掉，下回合起才由托管出手。没变化
                        // 回控制盘。
                        self.sfx.push(SFX_CLICK);
                        let changed = self
                            .battle
                            .get(slot)
                            .map(|c| c.delegation != draft)
                            .unwrap_or(false);
                        if let Some(c) = self.battle.get_mut(slot) {
                            c.delegation = draft;
                        }
                        if changed {
                            self.turn = None;
                            self.push_log(format!("{} 改了托管", self.name(slot)));
                            self.battle.finish_turn(slot);
                            self.settle_outcome();
                            return;
                        }
                        turn.menu = Menu::Main { index: back };
                    }
                    // 关掉（0xd6 带 −1 参数那一支，0x4dd761）：什么都不写。
                    Input::Cancel => turn.menu = Menu::Main { index: back },
                }
            }
            Menu::Lineup {
                cursor,
                held,
                moved,
            } => {
                let back = MenuEntry::ALL
                    .iter()
                    .position(|e| *e == MenuEntry::Lineup)
                    .unwrap_or(0);
                match input {
                    Input::Up | Input::Down | Input::Left | Input::Right => {
                        let step = match input {
                            Input::Up => (0, -1),
                            Input::Down => (0, 1),
                            Input::Left => (-1, 0),
                            _ => (1, 0),
                        };
                        turn.menu = Menu::Lineup {
                            cursor: hud::lineup_move(cursor, step),
                            held,
                            moved,
                        };
                    }
                    Input::Confirm => match held {
                        // 抓起（0x4de3d0）：点中的头像得在场、且能动（`FUN_004f0750`）。
                        None => match self.lineup_occupant(cursor) {
                            Some(s) if self.can_rearrange(s) => {
                                self.sfx.push(SFX_CLICK);
                                turn.menu = Menu::Lineup {
                                    cursor,
                                    held: Some(s),
                                    moved,
                                };
                            }
                            _ => self.sfx.push(SFX_DENIED),
                        },
                        // 放下（0x4de4f8）：目标位上站着动不了的人（`FUN_004f0700`）就
                        // 弹回；否则两人对调五灵位（战斗位 `+0x470`）。
                        Some(a) => {
                            let ok = self.rearrange(a, cursor);
                            self.sfx.push(if ok { SFX_CLICK } else { SFX_DENIED });
                            turn.menu = Menu::Lineup {
                                cursor,
                                held: None,
                                moved: moved || ok,
                            };
                        }
                    },
                    Input::Cancel if held.is_some() => {
                        turn.menu = Menu::Lineup {
                            cursor,
                            held: None,
                            moved,
                        };
                    }
                    Input::Cancel => {
                        // 关面板播 7 号序列回到平时的机位（0x4ddef6 / 0x4de84f）。
                        self.camera_requests.push(7);
                        if moved {
                            // 关面板（0x4de699）：原版给我方每人排一个 0x108 行动（换位的动作），
                            // 镜头切 7，停 0.5 秒，再回主调度。移植端只把这一回合算作用掉。
                            // 没换位时 0x4ddf52 比对快照（0x17a8ee0）一样，直接回控制盘，不算用掉。
                            self.turn = None;
                            self.push_log(format!("{} 变换阵型", self.name(slot)));
                            self.battle.finish_turn(slot);
                            self.settle_outcome();
                            return;
                        }
                        turn.menu = Menu::Main { index: back };
                    }
                }
            }
            Menu::Protect { focus, draft } => {
                let back = MenuEntry::ALL
                    .iter()
                    .position(|e| *e == MenuEntry::Protect)
                    .unwrap_or(0);
                match input {
                    Input::Up => {
                        turn.menu = Menu::Protect {
                            focus: focus.saturating_sub(1),
                            draft,
                        };
                    }
                    Input::Down => {
                        turn.menu = Menu::Protect {
                            focus: (focus + 1).min(hud::PROTECT_OK_FOCUS),
                            draft,
                        };
                    }
                    Input::Left | Input::Right => {}
                    // 三格是单选：点一格先全清再勾它（0x4dedc0）；点勾着的那格就取消。
                    // 人不在场或倒下的那格是灰的（0x4deafe）。
                    Input::Confirm if focus < hud::PROTECT_OK_FOCUS => {
                        let t = focus + 1;
                        if self.can_be_protected(t) {
                            let draft = if draft == Some(t) { None } else { Some(t) };
                            turn.menu = Menu::Protect { focus, draft };
                        }
                    }
                    // 确定（0x4dec61）：写 0 号位 `+0x27c`；和打开时一样就回控制盘，变了就
                    // 置 `+0x60` 把 0x10c 当这一回合的行动——那个任务（0x5158f0）只做
                    // `FUN_004db710`，两条清零，也就是这一回合用掉。
                    Input::Confirm => {
                        let before = self.battle.get(0).and_then(|c| c.protecting);
                        if let Some(c) = self.battle.get_mut(0) {
                            c.protecting = draft;
                        }
                        if before != draft {
                            self.turn = None;
                            let line = match draft {
                                Some(t) => format!("{} 援护 {}", self.name(0), self.name(t)),
                                None => format!("{} 不再援护", self.name(0)),
                            };
                            self.push_log(line);
                            self.battle.finish_turn(slot);
                            self.settle_outcome();
                            return;
                        }
                        turn.menu = Menu::Main { index: back };
                    }
                    Input::Cancel => turn.menu = Menu::Main { index: back },
                }
            }
            Menu::Skills { index } => {
                let n = self.skills(slot).len();
                match input {
                    Input::Up | Input::Left => {
                        turn.menu = Menu::Skills {
                            index: (index + n - 1) % n,
                        };
                    }
                    Input::Down | Input::Right => {
                        turn.menu = Menu::Skills {
                            index: (index + 1) % n,
                        };
                    }
                    Input::Confirm => {
                        if self.can_afford(slot, index) {
                            turn.menu = Menu::Target {
                                action: Action::Cast(index),
                                index: 0,
                            };
                        } else {
                            self.notice("精气不足");
                        }
                    }
                    Input::Cancel => {
                        turn.menu = Menu::Main { index: 1 };
                    }
                }
            }
            Menu::Specials { index } => {
                let n = self.specials(slot).len();
                match input {
                    Input::Up | Input::Left => {
                        turn.menu = Menu::Specials {
                            index: (index + n - 1) % n,
                        };
                    }
                    Input::Down | Input::Right => {
                        turn.menu = Menu::Specials {
                            index: (index + 1) % n,
                        };
                    }
                    Input::Confirm => {
                        if self.can_afford_special(slot, index) {
                            turn.menu = Menu::Target {
                                action: Action::Special(index),
                                index: 0,
                            };
                        } else {
                            self.notice("精气不足");
                        }
                    }
                    Input::Cancel => {
                        let back = MenuEntry::ALL
                            .iter()
                            .position(|e| *e == MenuEntry::Special)
                            .unwrap_or(0);
                        turn.menu = Menu::Main { index: back };
                    }
                }
            }
            Menu::Items { index } => {
                let n = self.items.len();
                match input {
                    Input::Up | Input::Left => {
                        turn.menu = Menu::Items {
                            index: (index + n - 1) % n,
                        };
                    }
                    Input::Down | Input::Right => {
                        turn.menu = Menu::Items {
                            index: (index + 1) % n,
                        };
                    }
                    Input::Confirm => {
                        if self.item_available(index) {
                            turn.menu = Menu::Target {
                                action: Action::UseItem(index),
                                index: 0,
                            };
                        } else {
                            self.notice("已经用完了");
                        }
                    }
                    Input::Cancel => {
                        turn.menu = Menu::Main { index: 2 };
                    }
                }
            }
            Menu::Target { action, index } => {
                let targets = self.targets();
                let n = targets.len().max(1);
                match input {
                    Input::Up | Input::Left => {
                        turn.menu = Menu::Target {
                            action,
                            index: (index + n - 1) % n,
                        };
                    }
                    Input::Down | Input::Right => {
                        turn.menu = Menu::Target {
                            action,
                            index: (index + 1) % n,
                        };
                    }
                    Input::Confirm => {
                        let Some(target) = targets.get(index).copied() else {
                            return;
                        };
                        self.turn = None;
                        match action {
                            Action::Attack => self.commit(Pending::Attack { slot, target }, 1),
                            Action::Cast(i) => {
                                let m = self.cast_multiplier(slot, i, false);
                                self.commit(
                                    Pending::Cast {
                                        slot,
                                        target,
                                        skill: i,
                                        special: false,
                                    },
                                    m,
                                )
                            }
                            // 特技不在法术表里，`FUN_004f76c0` 返回 0，任务里当 1。
                            Action::Special(i) => self.commit(
                                Pending::Cast {
                                    slot,
                                    target,
                                    skill: i,
                                    special: true,
                                },
                                1,
                            ),
                            Action::UseItem(i) => self.commit(
                                Pending::Item {
                                    slot,
                                    target,
                                    item: i,
                                },
                                1,
                            ),
                        }
                        return;
                    }
                    Input::Cancel => {
                        turn.menu = match action {
                            Action::Attack => Menu::Main { index: 0 },
                            Action::Cast(i) => Menu::Skills { index: i },
                            Action::Special(i) => Menu::Specials { index: i },
                            Action::UseItem(i) => Menu::Items { index: i },
                        };
                    }
                }
            }
        }
        self.turn = Some(turn);
    }

    /// The payout for what has fallen so far — the original settles the
    /// same way after a win, a round-out and a successful escape.
    pub fn spoils(&self) -> Spoils {
        let party: Vec<Survivor> = (0..SLOTS)
            .filter_map(|slot| {
                let c = self.battle.get(slot)?;
                (c.side == Side::Party).then_some(Survivor {
                    slot,
                    alive: c.alive,
                    exp_bonus_percent: c.exp_bonus as f32,
                    money_bonus_percent: c.money_bonus as f32,
                })
            })
            .collect();
        settlement::settle(&self.defeated, &party)
    }

    fn notice(&mut self, text: &str) {
        self.notice = Some((text.to_string(), NOTICE_SECONDS));
    }

    /// 在招式横幅上写一行字，停 `seconds` 秒（合击技开演时写它的名字，`0x4fd880`）。
    pub fn announce(&mut self, text: &str, seconds: f32) {
        self.notice = Some((text.to_string(), seconds));
    }

    fn push_log(&mut self, line: String) {
        log::debug!("Combat log: {line}");
        if self.log.len() >= LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }

    /// Record the outcome once; true when the battle is over.
    fn settle_outcome(&mut self) -> bool {
        if self.finished.is_some() {
            return true;
        }
        let outcome = self.battle.outcome();
        if outcome != Outcome::Ongoing {
            self.finished = Some(outcome);
            self.turn = None;
        }
        outcome != Outcome::Ongoing
    }

    /// 控制盘上这一格对 `slot` 能不能按——开盘的状态函数 0x4dc7a0 逐个显隐：
    ///
    /// * 術（+0x54）：中了封（0xf）不可按；
    /// * 物、逃（+0x58 / +0x68）：中了禁（0x10）不可按；
    /// * 陣（+0x60）：我方还有人的行动没收尾时不可按（`FUN_004df3d0` 数 `FUN_004fcda0`）；
    /// * 援（+0x6c）：当前是 0 号位、0 号位站在盘心（阵位 6）、1..3 号位至少一人在场；
    /// * 魔（+0x70）从不显示；其余总能按。
    pub fn entry_enabled(&self, slot: usize, entry: MenuEntry) -> bool {
        let has = |k: usize| self.battle.get(slot).is_some_and(|c| c.has_state(k));
        match entry {
            MenuEntry::Skill => !has(K_SEAL),
            MenuEntry::Item | MenuEntry::Escape => !has(K_FORBID),
            MenuEntry::Lineup => {
                !self.pending_actions.iter().any(|p| p.slot() < PARTY_SLOTS)
                    && !self.escape.is_some_and(|s| s < PARTY_SLOTS)
            }
            MenuEntry::Protect => {
                slot == 0
                    && self.battle.get(0).is_some_and(|c| c.element_position == 6)
                    && (1..=3).any(|s| self.battle.get(s).is_some_and(|c| !c.retired))
            }
            _ => true,
        }
    }

    /// 援护面板上这一格能不能勾：人在场且站着（0x4deb01 查 `+0x241` 和 `FUN_004ed010`）。
    pub fn can_be_protected(&self, slot: usize) -> bool {
        self.battle.get(slot).map(|c| c.in_play()).unwrap_or(false)
    }

    /// 站在五灵位 `spot` 上的我方（在场，倒下的也算——原版按 `+0x241` 找）。
    pub fn lineup_occupant(&self, spot: u8) -> Option<usize> {
        (0..PARTY_SLOTS).find(|s| {
            self.battle
                .get(*s)
                .map(|c| !c.retired && c.element_position == spot)
                .unwrap_or(false)
        })
    }

    /// `FUN_004f0750`：换阵时能不能挪这个人——站着，且没有 0x0e / 0x11 / 0x12 / 0x13。
    pub fn can_rearrange(&self, slot: usize) -> bool {
        self.battle
            .get(slot)
            .map(|c| c.in_play() && ![0x0e, 0x11, 0x12, 0x13].iter().any(|k| c.has_state(*k)))
            .unwrap_or(false)
    }

    /// 把 `slot` 挪到五灵位 `spot`，原来站那儿的人换到 `slot` 的旧位（0x4de581 那一圈）。
    /// 那人动不了就不换，返回 `false`。
    fn rearrange(&mut self, slot: usize, spot: u8) -> bool {
        let Some(old) = self.battle.get(slot).map(|c| c.element_position) else {
            return false;
        };
        if old == spot {
            return true;
        }
        if let Some(other) = self.lineup_occupant(spot) {
            if !self.can_rearrange(other) {
                return false;
            }
            if let Some(c) = self.battle.get_mut(other) {
                c.element_position = old;
            }
        }
        if let Some(c) = self.battle.get_mut(slot) {
            c.element_position = spot;
        }
        true
    }

    /// 我方托管 / 乱 / 狂（0x513a36 那一支 → `FUN_004d86d0`）：挑出来就照常登记、进出手段，
    /// 返回 `true`。什么都没挑出来返回 `false`，调用方让这个人下一帧再试——原版 0x513a48 起
    /// 那条链先查 `+0x243`（我方）就跳到 0x513ac7，不交给怪物那套。
    fn act_by_delegate(&mut self, slot: usize) -> bool {
        self.act_by_plan(slot, AiMode::Delegate)
    }

    /// 怪物中了乱（0x12）：`FUN_004d7120` → `FUN_004d8580`，随便挑人随便出手。
    fn act_confused(&mut self, slot: usize) -> bool {
        self.act_by_plan(slot, AiMode::Confused)
    }

    fn act_by_plan(&mut self, slot: usize, mode: AiMode) -> bool {
        let tool = |index: usize, e: &SkillEffect, usable: bool| Tool {
            index,
            id: e.id,
            kind: e.kind,
            range: e.target_range,
            hp: e.impacts[ATTR_HP],
            states: e.state_codes,
            usable,
        };
        let magic: Vec<Tool> = (0..self.skills(slot).len())
            .map(|i| tool(i, &self.skills(slot)[i].effect, self.can_afford(slot, i)))
            .collect();
        let specials: Vec<Tool> = (0..self.specials(slot).len())
            .map(|i| {
                tool(
                    i,
                    &self.specials(slot)[i].effect,
                    self.can_afford_special(slot, i),
                )
            })
            .collect();
        let items: Vec<Tool> = self
            .items
            .iter()
            .enumerate()
            .map(|(i, it)| Tool {
                index: i,
                id: it.effect.id,
                kind: it.effect.kind,
                range: it.effect.target_range,
                hp: it.effect.impacts[ATTR_HP],
                states: it.effect.state_codes,
                usable: it.count > 0,
            })
            .collect();
        let kit = Kit {
            magic: &magic,
            specials: &specials,
            items: &items,
        };
        let plan = match mode {
            AiMode::Confused => ai::confused(&self.battle, &mut self.ai_rng, slot, &kit),
            AiMode::Delegate => ai::delegate(&self.battle, &mut self.ai_rng, slot, &kit),
            AiMode::AutoPlay => ai::delegate_with(
                &self.battle,
                &mut self.ai_rng,
                slot,
                &kit,
                Delegation::all_on(),
            ),
            // 怪物的阶梯掷的是伤害、暴击用的那一个发生器（`DAT_005dfdec`）。
            AiMode::Monster => {
                let mut rng = self.battle.rng.clone();
                let plan = ai::monster(&self.battle, &mut rng, slot, &kit);
                self.battle.rng = rng;
                plan
            }
        };
        log::info!("Combat: {} {} → {:?}", mode.label(), self.name(slot), plan);
        match plan {
            Some(Plan::Attack { target }) => self.commit(Pending::Attack { slot, target }, 1),
            Some(Plan::Magic { target, index }) => {
                let m = self.cast_multiplier(slot, index, false);
                self.commit(
                    Pending::Cast {
                        slot,
                        target,
                        skill: index,
                        special: false,
                    },
                    m,
                )
            }
            Some(Plan::Special { target, index }) => {
                let m = self.cast_multiplier(slot, index, true);
                self.commit(
                    Pending::Cast {
                        slot,
                        target,
                        skill: index,
                        special: true,
                    },
                    m,
                )
            }
            Some(Plan::Item {
                target,
                index: item,
            }) => self.commit(Pending::Item { slot, target, item }, 1),
            None => return false,
        }
        true
    }

    /// 怪物那套（`FUN_004d7120` → 三个阶梯，见 [`ai::monster`]）；中了乱转 `FUN_004d8580`。
    /// 快进时我方也走这里，按六个开关全开代打。挑不出来返回 `false`，下一帧再试（0x513ab1
    /// 之后直接到 0x513ac7，不清行动条）。
    fn act_by_ai(&mut self, slot: usize, confused: bool) -> bool {
        let mode = match self.battle.get(slot).map(|c| c.side) {
            Some(Side::Enemy) if confused => AiMode::Confused,
            Some(Side::Enemy) => AiMode::Monster,
            _ => AiMode::AutoPlay,
        };
        self.act_by_plan(slot, mode)
    }

    /// `FUN_004f76c0` 给的出手速度倍率，建菜单时已存进 [`SkillChoice::multiplier`]。
    fn cast_multiplier(&self, slot: usize, skill: usize, special: bool) -> i32 {
        let Some(p) = self.profiles.get(slot) else {
            return 0;
        };
        let list = if special { &p.specials } else { &p.skills };
        list.get(skill).map(|s| s.multiplier).unwrap_or(0)
    }

    /// 普攻（`FUN_004f0a10`）：横排 / 竖列打一排人（[`Battle::attack_targets`]），暴击整次
    /// 只掷一次，再按位号逐个结算。
    fn attack(&mut self, slot: usize, target: usize) {
        let targets = self.battle.attack_targets(slot, target);
        let crit = self.battle.roll_crit(slot);
        for t in targets {
            self.land_blow(slot, t, crit);
        }
        self.note_deaths(slot);
    }

    fn land_blow(&mut self, slot: usize, target: usize, crit: bool) {
        let blow = self.battle.normal_attack(slot, target, crit);
        // 按速度躲开或没掉血：目标播回避（`FUN_004feef0`）；睡着（0x11）或定住（0x0e）时
        // 那一步不挂动作。
        if blow.dodged {
            let still = self
                .battle
                .get(target)
                .is_some_and(|c| c.in_play() && !c.has_state(0x11) && !c.has_state(0x0e));
            if still {
                self.dodged.push(target);
            }
        }
        // 暴击：在挨打的人身上放特效组 0x1d6（0x4f0cd9 / 援护那一支 0x4f0d7c）；弹回去的不放。
        if blow.crit && !blow.reflected {
            self.effects.push((EFFECT_CRIT, blow.victim));
        }
        if let Some(p) = blow.protector {
            // 援护挡下（0x4f0d3a）：被护的人情 +1（在规则层做了），放 wa010。
            self.sfx.push(SFX_AFFECTION);
            self.push_log(format!("{} 替 {} 挡下", self.name(p), self.name(target)));
        }
        if blow.reflected {
            self.push_log(format!(
                "{} 的攻击被 {} 弹回 精{:+}",
                self.name(slot),
                self.name(target),
                blow.delta
            ));
        } else {
            self.push_log(format!(
                "{} 攻击 {} 精{:+}{}",
                self.name(slot),
                self.name(blow.victim),
                blow.delta,
                if blow.crit { "（暴击）" } else { "" }
            ));
        }
        if let Some(stolen) = blow.stolen {
            if stolen.money > 0 {
                self.push_log(format!("偷得 {} 文", stolen.money));
            }
            if let Some((id, n)) = stolen.item {
                self.push_log(format!("偷得 #{id} ×{n}"));
            }
        }
        if blow.eat_item {
            self.eat_item(blow.victim);
        }
    }

    /// 饿鬼那一口（攻击方式 0x392 → `FUN_004f1fa0(…, 0x1c)`）：在背包里挑「回复类」道具
    /// （`FUN_0040d010(0xe)`：类型 1，且精/气/神任一项影响类型是 2 或数值 > 0，或者能解死亡），
    /// 按背包顺序 `rand % 个数` 挑一种，整格吃掉（`FUN_0040cf60` 按那一格的数量扣）。
    fn eat_item(&mut self, victim: usize) {
        let edible: Vec<usize> = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, it)| it.count > 0 && !it.pearl && is_restorative(&it.effect))
            .map(|(i, _)| i)
            .collect();
        if edible.is_empty() {
            return;
        }
        let pick = edible[self.battle.rng.rand15() as usize % edible.len()];
        let (id, n, name) = {
            let it = &self.items[pick];
            (it.effect.id, it.count, it.name.clone())
        };
        self.items[pick].count = 0;
        self.eaten_items.push((id, n));
        self.push_log(format!("{} 的 {name} ×{n} 被吃掉了", self.name(victim)));
    }

    /// 这次仙术配不配得上一条合击技；配上了给 (合击技下标, 目标)。
    fn combo_for(&self, action: &Pending) -> Option<(usize, usize)> {
        // 只有仙术配合击技（`FUN_004eea70` 在 0x105 登记时查）。
        let Pending::Cast {
            slot,
            target,
            skill,
            special: false,
        } = *action
        else {
            return None;
        };
        let id = self.skills(slot).get(skill)?.effect.id;
        combo::find(&self.combos, slot, id, &self.battle).map(|c| (c, target))
    }

    /// 合击技生效（`FUN_004ef8b0` → `FUN_004f1c40`）：每个目标按 `FUN_004eef20` 的
    /// 0x10e 分支改精，扣了精的让它出手段从头来；不扣任何消耗，原来那个仙术也不算用过。
    fn combo(&mut self, slot: usize, target: usize, i: usize) {
        let Some(def) = self.combos.get(i).cloned() else {
            return;
        };
        for t in combo::targets(&def, target, &self.battle) {
            let delta = combo::hp_delta(&def, t, &mut self.battle);
            let moved = self.battle.change_hp(t, delta);
            if moved < 0 {
                self.battle.restart_cast(t);
            }
            self.push_log(format!("{} → {} 精{:+}", def.name, self.name(t), moved));
        }
        self.note_deaths(slot);
    }

    /// 放一次仙术 / 特技（`FUN_004f1070` / `FUN_004f1420`）。全体的打一整边（相对施术者），
    /// 复活类连倒下的一起；单体只打选中的那个。消耗打完才扣、只扣一次（`FUN_004f2910`）。
    fn cast(&mut self, slot: usize, target: usize, i: usize, special: bool) {
        let Some(choice) = self.cast_choice(slot, i, special).cloned() else {
            return;
        };
        let caster_side = self.battle.get(slot).map(|c| c.side).unwrap_or(Side::Party);
        // 仙术用一次记一次（升级用）；特技不记（`FUN_004f1420` 收尾没有 `+0x3d8` 那一步）。
        if !special && caster_side == Side::Party {
            self.casts.push((slot, choice.effect.id));
        }
        let targets = if choice.hits_everyone() {
            let side = match (choice.targets_own_side(), caster_side) {
                (true, side) => side,
                (false, Side::Party) => Side::Enemy,
                (false, Side::Enemy) => Side::Party,
            };
            if choice.revives() {
                self.placed(side)
            } else {
                self.standing(side)
            }
        } else {
            vec![target]
        };
        for t in targets {
            // 目标不在原位（`FUN_0050a700` 为否）、又不是复活类（技能记录 `+0x29a`，即状态 24
            // 那一项不是 2）：弹 1 秒「MISS」、不结算（`FUN_004f1070` 0x4f10d5 起）。特技多一道：
            // 打一整边的（`target_range_type` 2/4，`FUN_004f05a0`）照样结算（`FUN_004f1420`）。
            if self.away_from_home.get(t).copied().unwrap_or(false)
                && !choice.revives()
                && !(special && choice.hits_everyone())
            {
                self.battle.popups.push(super::state::Popup {
                    slot: t,
                    kind: super::state::PopupKind::Miss,
                    secs: super::state::POPUP_MISS_SECS,
                });
                self.push_log(format!(
                    "{} {} → {} 不在原位，MISS",
                    self.name(slot),
                    choice.name,
                    self.name(t)
                ));
                continue;
            }
            let r = if special {
                let (r, stolen) = self.battle.special_on(slot, t, &choice.effect);
                log::info!(
                    "Combat: 特技 {} → 位 {t}：{}，精 {:+}，偷得 {} 文、物品 {:?}",
                    choice.name,
                    if r.hit { "命中" } else { "未中" },
                    r.deltas[ATTR_HP],
                    stolen.money,
                    stolen.item
                );
                if stolen.money > 0 {
                    self.push_log(format!("偷得 {} 文", stolen.money));
                }
                if let Some((id, n)) = stolen.item {
                    self.push_log(format!("偷得物品 #{id} × {n}"));
                }
                r
            } else {
                self.battle.cast_on(slot, t, &choice.effect, false)
            };
            let line = if r.hit {
                format!(
                    "{} {} → {} 精{:+}",
                    self.name(slot),
                    choice.name,
                    self.name(t),
                    r.deltas[ATTR_HP]
                )
            } else {
                format!(
                    "{} {} → {} 未命中",
                    self.name(slot),
                    choice.name,
                    self.name(t)
                )
            };
            self.push_log(line);
        }
        self.battle.pay_skill_cost(slot, &choice.effect);
        self.note_deaths(slot);
    }

    /// 用一件道具（`FUN_004f2750` 那条链）。全体的（`target_range` 2/4）打
    /// 一整边，单体的只打选中的那个。用掉的记在 [`Self::used_items`]，散场
    /// 时才写回背包。
    fn use_item(&mut self, slot: usize, target: usize, i: usize) {
        let Some(item) = self.items.get(i).cloned() else {
            return;
        };
        if item.count == 0 {
            return;
        }

        let targets = if item.hits_everyone() {
            let side = if item.targets_own_side() {
                Side::Party
            } else {
                Side::Enemy
            };
            if item.revives() {
                self.placed(side)
            } else {
                self.standing(side)
            }
        } else {
            vec![target]
        };

        let mut flee = false;
        for t in targets {
            let r = self.battle.use_item(slot, t, &item.effect);
            flee |= r.flee;
            let line = if r.instant_kill {
                format!(
                    "{} 对 {} 用了 {}，一击毙命",
                    self.name(slot),
                    self.name(t),
                    item.name
                )
            } else {
                format!(
                    "{} 对 {} 用了 {} 精{:+}",
                    self.name(slot),
                    self.name(t),
                    item.name,
                    r.deltas[ATTR_HP]
                )
            };
            self.push_log(line);
        }

        // 鬼遁水（special 2）：全队直接撤离，和逃跑成功走同一条路。
        if flee {
            self.battle.retire_party();
            self.push_log(format!("{} 用 {} 撤离", self.name(slot), item.name));
        }

        self.used_items.push(item.effect.id);
        if let Some(it) = self.items.get_mut(i) {
            it.count = it.count.saturating_sub(1);
        }
        self.note_deaths(slot);
    }

    /// Anything that has fallen since the last look gets a settlement
    /// entry, credited to `actor` if it is on the party's side.
    fn note_deaths(&mut self, actor: usize) {
        // `FUN_005136d0`：被护的人离场，援护就清掉。
        if let Some(t) = self.battle.get(0).and_then(|c| c.protecting) {
            if !self.can_be_protected(t) {
                if let Some(c) = self.battle.get_mut(0) {
                    c.protecting = None;
                }
            }
        }
        let killer = self
            .battle
            .get(actor)
            .filter(|c| c.side == Side::Party)
            .map(|_| actor);
        let already = self.defeated.len();
        let fallen: Vec<usize> = (0..SLOTS)
            .filter(|s| {
                self.battle
                    .get(*s)
                    .map(|c| c.side == Side::Enemy && !c.alive)
                    .unwrap_or(false)
            })
            .collect();
        // Enemies do not get up again, so "dead and not yet counted" is
        // exactly the ones beyond what was counted before.
        for slot in fallen.into_iter().skip(already) {
            let reward = self.profiles[slot].reward;
            self.push_log(format!("{} 倒下了", self.name(slot)));
            self.defeated.push(DefeatedEnemy {
                experience: reward.experience,
                money: reward.money,
                loot: reward.loot,
                killer: self.battle.get(slot).and_then(|c| c.killed_by).or(killer),
                death_time: self.elapsed,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openpal3::combat::actions::ATTRIBUTE_SLOTS;

    /// 景天放洒金笺、雪见站土位拿刺带状态 15：「连环破斩」自己冒出来。
    #[test]
    fn a_matching_cast_turns_into_a_combo_by_itself() {
        let build = |with_state: bool| {
            let mut b = Battle::new(3);
            let mut jt = fighter(Side::Party, 200, 100);
            jt.actor_id = 101;
            jt.element_position = 3;
            jt.weapon_type = 2;
            let mut xj = fighter(Side::Party, 200, 50);
            xj.actor_id = 104;
            xj.element_position = 5;
            xj.weapon_type = 6;
            xj.states[15].active = with_state;
            b.place(0, jt);
            b.place(1, xj);
            b.place(5, fighter(Side::Enemy, 900, 1));
            let skill = SkillChoice {
                multiplier: 0,
                name: "洒金笺".into(),
                effect: SkillEffect {
                    id: 4102,
                    sp_cost: 4,
                    special_cost_type: -1,
                    ..Default::default()
                },
            };
            let profiles = vec![Profile {
                specials: Vec::new(),
                name: "景天".into(),
                skills: vec![skill],
                reward: Reward::default(),
            }];
            let mut s = CombatSession::new(b, profiles, 3);
            s.set_combos(vec![ComboDef {
                id: 501,
                name: "连环破斩".into(),
                actors: [101, 104, 0, 0],
                positions: [3, 5, 0, 0],
                skill_id: 4102,
                weapons: [2, 6, 0, 0],
                states: [14, -1, -1],
                range: 3,
                hp_sign: -1,
            }]);
            s
        };

        let mut s = build(true);
        s.commit(
            Pending::Cast {
                special: false,
                slot: 0,
                target: 5,
                skill: 0,
            },
            1,
        );
        assert_eq!(
            s.pending(),
            Some(Pending::Combo {
                slot: 0,
                target: 5,
                combo: 0
            })
        );
        // 瞬发：出手条已经满了。
        assert!(s.battle.get(0).unwrap().cast_gauge >= formulas::GAUGE_FULL);
        let (hp, sp) = (s.battle.get(5).unwrap().hp, s.battle.get(0).unwrap().sp);
        s.resolve_pending(0);
        assert!(s.battle.get(5).unwrap().hp < hp, "打中了");
        assert_eq!(s.battle.get(0).unwrap().sp, sp, "合击技不扣气");

        // 雪见身上没那个状态：还是普通的仙术。
        let mut s = build(false);
        s.commit(
            Pending::Cast {
                special: false,
                slot: 0,
                target: 5,
                skill: 0,
            },
            1,
        );
        assert!(matches!(s.pending(), Some(Pending::Cast { .. })));
    }
    use crate::openpal3::combat::formulas;
    use crate::openpal3::combat::state::{Combatant, Phase};

    fn fighter(side: Side, hp: i32, speed: i32) -> Combatant {
        let mut c = Combatant::new(1, side);
        c.level = 5;
        c.hp = hp;
        c.max_hp = hp;
        c.attack = 100;
        c.defense = 10;
        c.speed = speed;
        c.luck = 10;
        c.max_sp = 50;
        c.sp = 50;
        c
    }

    fn heal() -> SkillChoice {
        let mut impacts = [(0u8, 0i16); ATTRIBUTE_SLOTS];
        impacts[ATTR_HP] = (0, 30);
        SkillChoice {
            multiplier: 0,
            name: "治疗".into(),
            effect: SkillEffect {
                id: 1,
                level: 1,
                target_range: 1,
                success_rate_level: 0,
                elements: (None, None),
                impacts,
                sp_cost: 10,
                mp_cost: 0,
                special_cost_type: -1,
                special_cost_value: 0,
                ..Default::default()
            },
        }
    }

    fn session() -> CombatSession {
        let mut b = Battle::new(1);
        // The party member is much faster, so it acts first.
        b.place(0, fighter(Side::Party, 200, 100));
        b.place(5, fighter(Side::Enemy, 60, 1));
        let profiles = vec![
            Profile {
                specials: Vec::new(),
                name: "景天".into(),
                skills: vec![heal()],
                reward: Reward::default(),
            },
            Profile::default(),
            Profile::default(),
            Profile::default(),
            Profile::default(),
            Profile {
                specials: Vec::new(),
                name: "妖".into(),
                skills: vec![],
                reward: Reward {
                    experience: 12,
                    money: 7,
                    loot: Some((3001, 1)),
                },
            },
        ];
        CombatSession::new(b, profiles, 7)
    }

    fn run_until_turn(s: &mut CombatSession) -> usize {
        for _ in 0..10_000 {
            assert!(s.tick(0.05).is_none());
            if let Some(t) = s.turn {
                return t.slot;
            }
        }
        panic!("没人轮到出手");
    }

    /// 我方四个人和怪一样快、玩家一轮到就按：怪照样轮得上（原版 `ProcControl` 挑行动不等别人的
    /// 行动结算完，见 `pending_actions`）。注意这个测试的时序复现不出实机里那种饿死（实机 M06 一场
    /// 40 秒怪一次没出手，出手段和舞台的时长和这里不一样），只守住「怪轮得上」。
    #[test]
    fn enemies_still_act_against_a_full_party() {
        let mut s = session();
        for slot in 0..4 {
            s.battle.place(slot, fighter(Side::Party, 5000, 4));
        }
        {
            let e = s.battle.get_mut(5).unwrap();
            e.speed = 4;
            e.max_hp = 5000;
            e.hp = 5000;
        }
        // 有舞台：每次演出占一秒（舞台演的时候整场时钟停）。
        s.stage_driven = true;
        let mut show = 0.;
        for _ in 0..20_000 {
            s.tick(0.05);
            if s.staging.is_some() {
                show += 0.05;
                if show >= 1. {
                    show = 0.;
                    s.release();
                }
            }
            if s.turn.is_some() {
                s.input(Input::Confirm);
                s.input(Input::Confirm);
            }
            if s.log.iter().any(|l| l.starts_with("妖")) {
                return;
            }
        }
        panic!("怪一直轮不上：{:?}", s.log);
    }

    #[test]
    fn without_a_stage_a_fleeing_enemy_is_gone_at_once() {
        let mut s = session();
        s.battle.get_mut(0).unwrap().speed = 1;
        let e = s.battle.get_mut(5).unwrap();
        e.escape_rate = 100;
        e.gauge = crate::openpal3::combat::formulas::GAUGE_FULL - 2.;
        let mut outcome = None;
        for _ in 0..100 {
            outcome = s.tick(0.05);
            if outcome.is_some() {
                break;
            }
        }
        assert!(s.battle.get(5).unwrap().retired, "快进 / 测试里当场跑完");
        assert_eq!(outcome, Some(crate::openpal3::combat::state::Outcome::Win));
    }

    #[test]
    fn the_last_enemy_running_off_closes_an_open_menu() {
        let mut s = session();
        assert_eq!(run_until_turn(&mut s), 0);
        s.stage_driven = true;
        s.input(Input::Confirm);
        assert!(s.turn.is_some(), "选目标");
        // 舞台把逃的人演到场外（`finish_fleeing`）时，玩家正开着面板。
        s.battle.finish_fleeing(5);
        assert_eq!(
            s.tick(0.05),
            Some(crate::openpal3::combat::state::Outcome::Win)
        );
        assert!(s.turn.is_none(), "面板收回");
    }

    #[test]
    fn the_party_gets_a_menu_and_the_enemy_does_not() {
        let mut s = session();
        assert_eq!(run_until_turn(&mut s), 0);
        assert_eq!(
            s.turn.unwrap().menu,
            Menu::Main { index: 0 },
            "菜单从攻击开始"
        );
        // Time stands still while the menu is up.
        let before = s.elapsed;
        s.tick(1.0);
        assert_eq!(s.elapsed, before);
    }

    /// 一直 tick 到战斗有结果——出手段让行动不再是当帧结算。
    fn run_until_finished(s: &mut CombatSession) -> Option<Outcome> {
        for _ in 0..400 {
            if let Some(o) = s.tick(0.05) {
                return Some(o);
            }
            if s.turn.is_some() {
                s.auto_play();
            }
        }
        None
    }

    #[test]
    fn attacking_through_the_menu_settles_a_kill_with_spoils() {
        let mut s = session();
        run_until_turn(&mut s);
        s.input(Input::Confirm); // 攻击
        assert!(matches!(
            s.turn.unwrap().menu,
            Menu::Target {
                action: Action::Attack,
                ..
            }
        ));
        assert_eq!(s.targets(), vec![5]);
        s.input(Input::Confirm);
        assert!(s.turn.is_none());
        // 确认菜单只是**选定**行动，单位这时进的是出手段；要等出手条走满
        // 才真正打出去（原版 `FUN_004db730` → `FUN_004db6c0`）。
        assert!(s.pending().is_some(), "行动应当挂在出手段等着");
        assert_eq!(s.tick(0.05), None, "出手条还没满");
        // 100 attack against 10 defence at level 5 kills a 60 HP enemy.
        assert_eq!(run_until_finished(&mut s), Some(Outcome::Win));
        let spoils = s.spoils();
        assert_eq!(spoils.money, 7);
        assert_eq!(spoils.loot, vec![(3001, 1)]);
        assert_eq!(spoils.experience.len(), 1);
        assert!(spoils.experience[0].1 > 12, "最后一击有三成加成");
    }

    #[test]
    fn highlighted_follows_the_cursor_and_the_attack_pattern() {
        let mut b = Battle::new(1);
        b.place(0, fighter(Side::Party, 200, 100));
        b.place(5, fighter(Side::Enemy, 60, 1));
        b.place(8, fighter(Side::Enemy, 60, 1));
        let mut profiles = vec![Profile::default(); 9];
        profiles[0].skills = vec![heal()];
        let mut s = CombatSession::new(b, profiles, 7);
        run_until_turn(&mut s);
        assert!(s.highlighted().is_empty(), "盘上没有目标亮");
        s.input(Input::Confirm); // 攻击
        assert_eq!(s.targets(), vec![5, 8]);
        assert_eq!(s.highlighted(), vec![5], "单体只亮光标那个");
        s.battle.get_mut(0).unwrap().pattern = crate::openpal3::combat::state::HitPattern::Column;
        assert_eq!(
            s.highlighted(),
            vec![5, 8],
            "竖列 {{5, 8}} 一起亮（FUN_004efe90）"
        );
    }

    #[test]
    fn a_heal_targets_our_own_side_and_cancel_walks_back() {
        let mut s = session();
        run_until_turn(&mut s);
        s.input(Input::Left); // 仙术
        s.input(Input::Confirm);
        assert_eq!(s.turn.unwrap().menu, Menu::Skills { index: 0 });
        s.input(Input::Confirm);
        assert_eq!(s.targets(), vec![0], "治疗术指向我方");
        s.input(Input::Cancel);
        assert_eq!(s.turn.unwrap().menu, Menu::Skills { index: 0 });
        s.input(Input::Cancel);
        assert_eq!(s.turn.unwrap().menu, Menu::Main { index: 1 });
    }

    fn open_ai_panel(s: &mut CombatSession) {
        run_until_turn(s);
        // 控制盘：擊 → 上「技」→ 左「AI」。
        s.input(Input::Up);
        s.input(Input::Left);
        let ai = MenuEntry::ALL
            .iter()
            .position(|e| *e == MenuEntry::Ai)
            .unwrap();
        assert_eq!(s.turn.unwrap().menu, Menu::Main { index: ai });
        s.input(Input::Confirm);
        assert_eq!(
            s.turn.unwrap().menu,
            Menu::Ai {
                focus: 0,
                draft: Delegation::default()
            }
        );
        assert_eq!(s.sfx.last(), Some(&SFX_NOTE));
    }

    #[test]
    fn the_ai_panel_hands_the_turn_over_once_confirmed() {
        let mut s = session();
        // 怪物挑不出东西会下一帧再试（不再空过），这里只看托管接不接手，别让它先把人打死。
        s.battle.get_mut(5).unwrap().attack = 1;
        open_ai_panel(&mut s);
        s.input(Input::Confirm);
        assert_eq!(s.sfx.last(), Some(&SFX_DENIED), "AI 没开，攻击勾不上");
        // 右下「手动/AI」。
        s.input(Input::Down);
        s.input(Input::Down);
        s.input(Input::Right);
        s.input(Input::Confirm);
        assert!(matches!(
            s.turn.unwrap().menu,
            Menu::Ai { focus: 5, draft } if draft.on && draft.attack
        ));
        s.input(Input::Down);
        s.input(Input::Confirm);
        assert!(s.turn.is_none(), "改过开关，这一回合用掉");
        assert!(s.pending().is_none(), "这一回合不出手");
        assert!(s.battle.get(0).unwrap().delegation.on);
        for _ in 0..10_000 {
            if let Some(outcome) = s.tick(0.05) {
                assert_eq!(outcome, Outcome::Win);
                return;
            }
            assert!(s.turn.is_none(), "托管的人不再出控制盘");
        }
        panic!("没打完");
    }

    #[test]
    fn closing_the_ai_panel_writes_nothing() {
        let mut s = session();
        open_ai_panel(&mut s);
        s.input(Input::Down);
        s.input(Input::Down);
        s.input(Input::Right);
        s.input(Input::Confirm);
        s.input(Input::Cancel);
        let ai = MenuEntry::ALL
            .iter()
            .position(|e| *e == MenuEntry::Ai)
            .unwrap();
        assert_eq!(s.turn.unwrap().menu, Menu::Main { index: ai });
        assert_eq!(s.battle.get(0).unwrap().delegation, Delegation::default());
    }

    #[test]
    fn the_lineup_panel_swaps_places_and_uses_up_the_turn() {
        let mut s = session();
        s.battle.get_mut(0).unwrap().element_position = 1;
        run_until_turn(&mut s);
        // 控制盘：擊 → 左「術」→ 下「陣」。
        s.input(Input::Left);
        s.input(Input::Down);
        let lineup = MenuEntry::ALL
            .iter()
            .position(|e| *e == MenuEntry::Lineup)
            .unwrap();
        assert_eq!(s.turn.unwrap().menu, Menu::Main { index: lineup });
        s.input(Input::Confirm);
        assert_eq!(
            s.turn.unwrap().menu,
            Menu::Lineup {
                cursor: 1,
                held: None,
                moved: false
            }
        );
        assert_eq!(s.camera_requests, vec![6], "打开面板播 6 号序列");
        // 没换位就关：回控制盘。
        s.input(Input::Cancel);
        assert_eq!(s.camera_requests, vec![6, 7], "关面板播 7 号序列");
        assert_eq!(s.turn.unwrap().menu, Menu::Main { index: lineup });
        s.input(Input::Confirm);
        s.input(Input::Up); // 盘心，没人
        s.input(Input::Confirm);
        assert_eq!(s.sfx.last(), Some(&SFX_DENIED), "空位抓不起人");
        s.input(Input::Down); // 回水
        s.input(Input::Confirm);
        assert!(matches!(
            s.turn.unwrap().menu,
            Menu::Lineup { held: Some(0), .. }
        ));
        s.input(Input::Up);
        s.input(Input::Confirm);
        assert_eq!(s.battle.get(0).unwrap().element_position, 6);
        s.input(Input::Cancel);
        assert!(s.turn.is_none(), "换过位，这一回合用掉");
        assert!(s.pending().is_none());
    }

    #[test]
    fn a_member_who_cannot_act_is_not_pushed_off_their_spot() {
        let mut s = session();
        s.battle.get_mut(0).unwrap().element_position = 1;
        let mut ally = fighter(Side::Party, 50, 1);
        ally.element_position = 6;
        ally.alive = false;
        s.battle.place(1, ally);
        assert!(!s.rearrange(0, 6), "倒下的人占着的位换不进去");
        assert_eq!(s.battle.get(0).unwrap().element_position, 1);
        s.battle.get_mut(1).unwrap().alive = true;
        assert!(s.rearrange(0, 6));
        assert_eq!(s.battle.get(1).unwrap().element_position, 1, "对调");
    }

    #[test]
    fn a_blow_that_lands_nothing_makes_the_target_dodge() {
        use crate::openpal3::combat::state::{K_INVISIBLE, K_WALL};
        let mut s = session();
        s.battle.get_mut(5).unwrap().states[K_WALL].active = true;
        s.attack(0, 5);
        assert_eq!(s.battle.get(5).unwrap().hp, 60, "避住了");
        assert!(
            s.dodged.is_empty(),
            "避是整下不结算、只弹数字（FUN_004fdb10），不播回避"
        );
        s.battle.get_mut(5).unwrap().states[K_WALL].active = false;
        s.battle.get_mut(5).unwrap().states[K_INVISIBLE].active = true;
        s.attack(0, 5);
        assert_eq!(s.battle.get(5).unwrap().hp, 60);
        assert_eq!(s.dodged, vec![5], "算出来没掉血就播回避");
        s.dodged.clear();
        s.battle.get_mut(5).unwrap().states[K_INVISIBLE].active = false;
        s.attack(0, 5);
        assert!(s.dodged.is_empty(), "打中了不回避");
    }

    #[test]
    fn the_cast_multiplier_comes_from_the_skill_choice() {
        let mut s = session();
        s.profiles[0].skills = vec![SkillChoice {
            multiplier: 3,
            ..heal()
        }];
        s.profiles[0].specials = vec![heal()];
        assert_eq!(s.cast_multiplier(0, 0, false), 3);
        assert_eq!(s.cast_multiplier(0, 0, true), 0, "特技不在法术表里");
        assert_eq!(s.cast_multiplier(0, 5, false), 0);
    }

    #[test]
    fn seal_and_forbid_grey_out_the_ring_like_the_original() {
        let mut s = session();
        run_until_turn(&mut s);
        assert!(s.entry_enabled(0, MenuEntry::Skill));
        s.battle.get_mut(0).unwrap().states[K_SEAL].active = true;
        assert!(!s.entry_enabled(0, MenuEntry::Skill), "封了不能用仙术");
        assert!(s.entry_enabled(0, MenuEntry::Item));
        s.battle.get_mut(0).unwrap().states[K_FORBID].active = true;
        assert!(!s.entry_enabled(0, MenuEntry::Item), "禁了不能用道具");
        assert!(!s.entry_enabled(0, MenuEntry::Escape), "禁了也不能逃");
        assert!(s.entry_enabled(0, MenuEntry::Attack));
    }

    fn with_ally(s: &mut CombatSession) {
        let mut ally = fighter(Side::Party, 50, 1);
        ally.element_position = 1;
        s.battle.place(1, ally);
    }

    #[test]
    fn protect_takes_the_blow_only_from_the_centre_of_the_disc() {
        let mut s = session();
        with_ally(&mut s);
        let me = s.battle.get_mut(0).unwrap();
        me.protecting = Some(1);
        me.element_position = 3;
        s.attack(5, 1);
        assert!(s.battle.get(1).unwrap().hp < 50, "不在盘心护不了");

        let mut s = session();
        with_ally(&mut s);
        let me = s.battle.get_mut(0).unwrap();
        me.protecting = Some(1);
        me.element_position = 6;
        s.attack(5, 1);
        assert_eq!(s.battle.get(1).unwrap().hp, 50, "被护的人没挨打");
        assert!(s.battle.get(0).unwrap().hp < 200, "景天替他挨了");
        assert_eq!(s.battle.get(1).unwrap().affection, 1);
        assert_eq!(s.sfx.last(), Some(&SFX_AFFECTION));
    }

    #[test]
    fn the_protect_panel_picks_one_and_uses_up_the_turn() {
        let mut s = session();
        assert_eq!(run_until_turn(&mut s), 0);
        with_ally(&mut s);
        assert!(!s.entry_enabled(0, MenuEntry::Protect), "不在盘心按不了援");
        s.battle.get_mut(0).unwrap().element_position = 6;
        // 控制盘：擊 → 上「技」→ 右「援」。
        s.input(Input::Up);
        s.input(Input::Right);
        let protect = MenuEntry::ALL
            .iter()
            .position(|e| *e == MenuEntry::Protect)
            .unwrap();
        assert_eq!(s.turn.unwrap().menu, Menu::Main { index: protect });
        s.input(Input::Confirm);
        assert_eq!(
            s.turn.unwrap().menu,
            Menu::Protect {
                focus: 0,
                draft: None
            }
        );
        s.input(Input::Down);
        s.input(Input::Confirm);
        assert!(
            matches!(s.turn.unwrap().menu, Menu::Protect { draft: None, .. }),
            "龙葵不在场，勾不上"
        );
        s.input(Input::Up);
        s.input(Input::Confirm);
        assert!(matches!(
            s.turn.unwrap().menu,
            Menu::Protect { draft: Some(1), .. }
        ));
        for _ in 0..3 {
            s.input(Input::Down);
        }
        s.input(Input::Confirm);
        assert!(s.turn.is_none(), "改了援护，这一回合用掉");
        assert!(s.pending().is_none());
        assert_eq!(s.battle.get(0).unwrap().protecting, Some(1));
        assert!(!s.entry_enabled(1, MenuEntry::Protect), "「援」只给景天");
    }

    #[test]
    fn the_skill_button_opens_the_specials_and_commits_a_special() {
        let mut s = session();
        let mut impacts = [(0u8, 0i16); ATTRIBUTE_SLOTS];
        impacts[ATTR_HP] = (0, -20);
        s.profiles[0].specials = vec![SkillChoice {
            multiplier: 0,
            name: "飞龙探云手".into(),
            effect: SkillEffect {
                id: 0x1005,
                sp_cost: 2,
                success_rate_level: 100,
                impacts,
                special_cost_type: -1,
                ..Default::default()
            },
        }];
        run_until_turn(&mut s);
        s.input(Input::Up); // 技
        s.input(Input::Confirm);
        assert_eq!(s.turn.unwrap().menu, Menu::Specials { index: 0 });
        s.input(Input::Cancel);
        let back = MenuEntry::ALL
            .iter()
            .position(|e| *e == MenuEntry::Special)
            .unwrap();
        assert_eq!(s.turn.unwrap().menu, Menu::Main { index: back });
        s.input(Input::Confirm);
        s.input(Input::Confirm);
        assert_eq!(s.targets(), vec![5], "打人的特技指向敌方");
        s.input(Input::Confirm);
        assert_eq!(
            s.pending(),
            Some(Pending::Cast {
                slot: 0,
                target: 5,
                skill: 0,
                special: true,
            })
        );
    }

    #[test]
    fn an_all_enemies_skill_hits_every_standing_enemy_and_pays_once() {
        let mut s = session();
        s.battle.place(6, fighter(Side::Enemy, 60, 1));
        while s.profiles.len() <= 6 {
            s.profiles.push(Profile::default());
        }
        let mut impacts = [(0u8, 0i16); ATTRIBUTE_SLOTS];
        impacts[ATTR_HP] = (0, -20);
        s.profiles[0].skills = vec![SkillChoice {
            multiplier: 0,
            name: "群攻".into(),
            effect: SkillEffect {
                id: 77,
                sp_cost: 10,
                success_rate_level: 100,
                impacts,
                target_range: 4,
                special_cost_type: -1,
                ..Default::default()
            },
        }];
        let sp = s.battle.get(0).unwrap().sp;
        s.cast(0, 5, 0, false);
        assert!(s.battle.get(5).unwrap().hp < 60, "选中的那个挨了");
        assert!(s.battle.get(6).unwrap().hp < 60, "没选中的也挨了");
        assert_eq!(s.battle.get(0).unwrap().sp, sp - 10, "全体技能只扣一次气");
    }

    #[test]
    fn a_spell_on_a_target_away_from_home_misses() {
        let mut s = session();
        s.battle.place(6, fighter(Side::Enemy, 60, 1));
        while s.profiles.len() <= 6 {
            s.profiles.push(Profile::default());
        }
        let mut impacts = [(0u8, 0i16); ATTRIBUTE_SLOTS];
        impacts[ATTR_HP] = (0, -20);
        let effect = SkillEffect {
            id: 77,
            sp_cost: 10,
            success_rate_level: 100,
            impacts,
            target_range: 4,
            special_cost_type: -1,
            ..Default::default()
        };
        let choice = SkillChoice {
            multiplier: 0,
            name: "群攻".into(),
            effect,
        };
        s.profiles[0].skills = vec![choice.clone()];
        s.profiles[0].specials = vec![choice];
        s.away_from_home[5] = true;
        let sp = s.battle.get(0).unwrap().sp;
        s.battle.popups.clear();
        // 仙术（`FUN_004f1070`）：打一整边也逐个查，5 号不在原位就 MISS、不掉精。
        s.cast(0, 5, 0, false);
        assert_eq!(s.battle.get(5).unwrap().hp, 60);
        assert!(s.battle.get(6).unwrap().hp < 60);
        assert!(
            s.battle
                .popups
                .iter()
                .any(|p| p.slot == 5 && p.kind == crate::openpal3::combat::state::PopupKind::Miss)
        );
        assert_eq!(s.battle.get(0).unwrap().sp, sp - 10, "消耗照扣");
        // 特技（`FUN_004f1420`）：打一整边的不查原位。
        s.cast(0, 5, 0, true);
        assert!(s.battle.get(5).unwrap().hp < 60);
    }

    #[test]
    fn a_revive_spell_ignores_the_home_check() {
        let mut s = session();
        let mut effect = SkillEffect {
            id: 78,
            success_rate_level: 100,
            target_range: 1,
            special_cost_type: -1,
            ..Default::default()
        };
        effect.state_codes[24] = 2;
        s.profiles[0].skills = vec![SkillChoice {
            multiplier: 0,
            name: "复活".into(),
            effect,
        }];
        s.away_from_home[1] = true;
        s.battle.popups.clear();
        s.cast(0, 1, 0, false);
        assert!(
            !s.battle
                .popups
                .iter()
                .any(|p| p.kind == crate::openpal3::combat::state::PopupKind::Miss)
        );
    }

    #[test]
    fn an_unaffordable_skill_is_refused_with_a_notice() {
        let mut s = session();
        s.battle.get_mut(0).unwrap().sp = 3;
        run_until_turn(&mut s);
        s.input(Input::Left); // 仙术
        s.input(Input::Confirm);
        s.input(Input::Confirm);
        assert_eq!(s.turn.unwrap().menu, Menu::Skills { index: 0 });
        assert!(s.notice.is_some());
    }

    #[test]
    fn a_staged_attack_waits_for_the_strike_and_holds_the_clock() {
        let mut s = session();
        run_until_turn(&mut s);
        s.stage_driven = true;
        let enemy_hp = s.battle.get(5).unwrap().hp;
        s.input(Input::Confirm); // 攻击
        s.input(Input::Confirm); // 第一个目标
        for _ in 0..400 {
            s.tick(0.05);
            match s.staging {
                Some(st) if st.slot == 0 => break,
                // 敌人先出手：它那一场直接演完。
                Some(_) => s.release(),
                None => {}
            }
        }
        let st = s.staging.expect("出手条满后应交给舞台");
        assert_eq!((st.slot, st.landed), (0, false));
        assert_eq!(s.battle.get(5).unwrap().hp, enemy_hp, "出手前不结算");

        // 演出期间时钟不走。
        let before: Vec<f32> = (0..SLOTS)
            .filter_map(|k| s.battle.get(k).map(|c| c.gauge))
            .collect();
        for _ in 0..20 {
            s.tick(0.05);
        }
        let after: Vec<f32> = (0..SLOTS)
            .filter_map(|k| s.battle.get(k).map(|c| c.gauge))
            .collect();
        assert_eq!(before, after);

        s.strike();
        assert!(s.battle.get(5).unwrap().hp < enemy_hp, "出手那一刻结算");
        assert!(s.staging.unwrap().landed);
        s.strike(); // 重复调用不再结算
        s.release();
        assert!(s.staging.is_none());
    }

    #[test]
    fn a_forbidden_escape_burns_the_turn() {
        let mut s = session();
        s.battle.escape_forbidden = true;
        run_until_turn(&mut s);
        s.input(Input::Down);
        s.input(Input::Right); // 逃跑
        s.input(Input::Confirm);
        assert!(s.turn.is_none());
        assert_eq!(s.escape, Some(0));
        assert!(s.pending().is_none(), "逃跑不占行动那一格");
        // 逃跑一样要走一段出手（`LAB_00515200` 的 0x51527a）。
        for _ in 0..400 {
            if s.log.back().map(|l| l.contains("逃跑")).unwrap_or(false) {
                break;
            }
            s.tick(0.05);
        }
        assert_eq!(s.finished(), None);
        assert_eq!(s.log.back().unwrap(), "景天 逃跑失败");
    }

    /// 原版 `ProcFleeWait` 是单开的虚拟线程，不占「一次只排一个行动任务」那一格：
    /// 逃跑蓄力期间怪照样出手，打中了这一次逃跑就作废
    /// （`cbrolestate.cpp:5209` → `cbsystem.cpp:1972` 的「逃跑失败」）。
    #[test]
    fn an_enemy_can_break_the_escape_while_it_winds_up() {
        let mut s = session();
        // 逃的人慢、怪快，蓄力期间轮得到怪；怪打得轻、人厚血，打不死。
        // 蓄力速度是 `(速_eff + 4) × 5` 截成 u8，速 47 正好顶到 255。
        {
            let p = s.battle.get_mut(0).unwrap();
            p.speed = 1;
            p.max_hp = 5000;
            p.hp = 5000;
        }
        {
            let e = s.battle.get_mut(5).unwrap();
            e.speed = 47;
            e.attack = 12;
        }
        let slot = run_until_turn(&mut s);
        assert_eq!(slot, 0);
        s.input(Input::Down);
        s.input(Input::Right); // 逃跑
        s.input(Input::Confirm);
        assert_eq!(s.escape, Some(0));
        // 逃跑不占行动那一格（怪这时可能已经挑好了自己的行动）。
        assert!(s.pending_for(0).is_none());

        let hp = s.battle.get(0).unwrap().hp;
        for _ in 0..2000 {
            if s.escape.is_none() {
                break;
            }
            assert!(s.tick(0.05).is_none(), "战斗不该在这里结束");
        }
        assert_eq!(s.escape, None, "逃跑被打掉了");
        assert!(s.battle.get(0).unwrap().hp < hp, "是挨打打掉的");
        assert!(
            s.log.iter().any(|l| l.contains("逃跑被打断")),
            "{:?}",
            s.log
        );
        assert!(!s.battle.get(0).unwrap().retired, "没跑成");
        assert_eq!(s.battle.get(0).unwrap().phase, Phase::Charging);
    }

    /// 逃跑等待期间中了「禁」，一样打掉（`cbsystem.cpp:1960`）。
    #[test]
    fn a_seal_knocks_the_escape_down_too() {
        let mut s = session();
        run_until_turn(&mut s);
        s.input(Input::Down);
        s.input(Input::Right);
        s.input(Input::Confirm);
        assert_eq!(s.escape, Some(0));
        s.battle.get_mut(0).unwrap().states[K_FORBID].active = true;
        s.tick(0.05);
        assert_eq!(s.escape, None);
        assert_eq!(s.battle.get(0).unwrap().phase, Phase::Charging);
    }

    #[test]
    fn fast_forward_lets_the_ai_take_the_players_turn() {
        let mut s = session();
        run_until_turn(&mut s);
        s.auto_play();
        assert!(s.turn.is_none());
        assert!(!s.log.is_empty());
    }

    #[test]
    fn a_confused_unit_lashes_out_instead_of_getting_a_menu() {
        // 我方中乱走 0x513a36 → `FUN_004d86d0` 的随机支：不出控制盘，挑不出就下一帧再试，
        // 迟早随手出手（这里唯一能打的是 5 号）。
        let mut s = session();
        run_until_turn(&mut s);
        let slot = s.turn.as_ref().unwrap().slot;
        s.turn = None;
        s.battle
            .get_mut(slot)
            .unwrap()
            .apply_state(K_CONFUSED, 0, 50);
        s.battle.get_mut(slot).unwrap().gauge = formulas::GAUGE_FULL;
        s.battle.get_mut(slot).unwrap().phase = Phase::Ready;
        for _ in 0..5000 {
            s.tick(0.01);
            assert!(s.turn.is_none(), "乱了不出控制盘");
            if s.pending().is_some() {
                break;
            }
        }
        assert_eq!(s.pending(), Some(Pending::Attack { slot: 0, target: 5 }));
    }

    #[test]
    fn a_paralysed_unit_loses_the_turn() {
        let mut s = session();
        run_until_turn(&mut s);
        let slot = s.turn.as_ref().unwrap().slot;
        s.turn = None;
        s.battle
            .get_mut(slot)
            .unwrap()
            .apply_state(K_PARALYSIS, 0, 5);
        s.battle.get_mut(slot).unwrap().gauge = formulas::GAUGE_FULL;
        s.battle.get_mut(slot).unwrap().phase = Phase::Ready;
        for _ in 0..40 {
            s.tick(0.05);
            if s.log.back().is_some_and(|l| l.contains("定住了")) {
                break;
            }
        }
        assert!(s.log.back().is_some_and(|l| l.contains("定住了")));
        assert!(s.pending().is_none());
    }

    /// 一件我方单体的疗伤药。
    fn potion(id: u32, hp: i16, count: u32) -> ItemChoice {
        let mut effect = ItemEffect {
            id,
            target_range: 1,
            ..Default::default()
        };
        effect.impacts[ATTR_HP] = (0, hp);
        ItemChoice {
            name: "天香续命露".into(),
            count,
            pearl: false,
            effect,
        }
    }

    #[test]
    fn an_item_heals_its_target_and_comes_off_the_stack() {
        let mut s = session();
        s.set_items(vec![potion(6201, 40, 2)]);
        run_until_turn(&mut s);
        s.battle.get_mut(0).unwrap().hp = 100;

        s.input(Input::Down); // 道具
        s.input(Input::Confirm);
        assert_eq!(s.turn.unwrap().menu, Menu::Items { index: 0 });
        s.input(Input::Confirm);
        assert_eq!(s.targets(), vec![0], "我方单体的药指向自己人");
        s.input(Input::Confirm);
        assert!(matches!(s.pending(), Some(Pending::Item { .. })));

        for _ in 0..400 {
            s.tick(0.05);
            if s.pending().is_none() {
                break;
            }
        }
        assert_eq!(
            s.battle.get(0).unwrap().hp,
            140,
            "原始值直接加，不按等级放大"
        );
        assert_eq!(s.items()[0].count, 1);
        assert_eq!(s.used_items, vec![6201]);
    }

    #[test]
    fn cancel_walks_back_out_of_the_item_list() {
        let mut s = session();
        s.set_items(vec![potion(6201, 40, 1)]);
        run_until_turn(&mut s);
        s.input(Input::Down);
        s.input(Input::Confirm);
        s.input(Input::Confirm);
        s.input(Input::Cancel);
        assert_eq!(s.turn.unwrap().menu, Menu::Items { index: 0 });
        s.input(Input::Cancel);
        assert_eq!(s.turn.unwrap().menu, Menu::Main { index: 2 });
    }

    #[test]
    fn a_used_up_stack_cannot_be_chosen() {
        let mut s = session();
        s.set_items(vec![potion(6201, 40, 0)]);
        run_until_turn(&mut s);
        s.input(Input::Down);
        s.input(Input::Confirm);
        s.input(Input::Confirm);
        assert_eq!(
            s.turn.unwrap().menu,
            Menu::Items { index: 0 },
            "用完的不该进选目标"
        );
        assert!(s.notice.is_some());
    }

    #[test]
    fn a_whole_side_item_hits_everyone_standing() {
        let mut s = session();
        // 敌方全体（range 4）的投掷物。
        let mut effect = ItemEffect {
            id: 6301,
            target_range: 4,
            ..Default::default()
        };
        effect.impacts[ATTR_HP] = (0, -20);
        s.set_items(vec![ItemChoice {
            name: "飞蝗石".into(),
            count: 1,
            pearl: false,
            effect,
        }]);
        run_until_turn(&mut s);
        s.input(Input::Down);
        s.input(Input::Confirm);
        s.input(Input::Confirm);
        assert_eq!(s.targets(), vec![5], "打对面");
        s.input(Input::Confirm);
        for _ in 0..400 {
            s.tick(0.05);
            if s.pending().is_none() {
                break;
            }
        }
        assert_eq!(s.battle.get(5).unwrap().hp, 40);
        assert_eq!(s.used_items, vec![6301]);
    }
}
