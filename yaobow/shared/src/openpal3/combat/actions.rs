//! What a combatant can do on its turn: cast a skill, use an item,
//! attack, defend, flee.
//!
//! The value a skill or item moves is `FUN_004f98b0` kind 2
//! (0x4f99e7–0x4f9dfe); how it lands on the target is `FUN_004eef20`
//! (0x4ef43a). Both are transcribed here as they are, including the part
//! that surprised me — see [`element_modifier`].

use super::formulas::{self, CombatRng};
use super::state::{Battle, Combatant, K_POISON_IMMUNE, Phase, Side};

/// The twelve `ActorAttributeType` slots a skill or item can address.
/// Only the first three do anything in combat: `FUN_004eef20` sends
/// everything from 武 onwards to an assert, so the database's 武防速运 and
/// 五灵 impact columns are dead weight in battle.
pub const ATTRIBUTE_SLOTS: usize = 12;
pub const ATTR_HP: usize = 0;
pub const ATTR_SP: usize = 1;
pub const ATTR_MP: usize = 2;
/// The first slot the original refuses to apply.
pub const FIRST_DEAD_ATTRIBUTE: usize = 3;

/// `AttributeImpactType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImpactKind {
    Absolute,
    Percentage,
    RecoverToMax,
    IncreaseMax,
}

impl ImpactKind {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Percentage,
            2 => Self::RecoverToMax,
            3 => Self::IncreaseMax,
            _ => Self::Absolute,
        }
    }
}

/// Just enough of a skill for the combat maths — filled from the game
/// database's `SkillInfo` by the caller, so this module stays independent
/// of the file format.
#[derive(Debug, Clone, Default)]
pub struct SkillEffect {
    pub id: u32,
    /// `SkillType`：0 特技 1 辅助 2 伤害 3 恢复 …（托管挑招时按它分类）。
    pub kind: u8,
    pub level: i32,
    pub success_rate_level: u8,
    /// Up to two elements, from the skill's `element_attributes`.
    pub elements: (Option<u8>, Option<u8>),
    /// Per attribute slot: the impact type and its raw value.
    pub impacts: [(u8, i16); ATTRIBUTE_SLOTS],
    pub sp_cost: i32,
    pub mp_cost: i32,
    /// 技能定义的 `+0x2b8`（`special_consume_type`）：气/神之外还要扣的那一
    /// 项，取值是属性下标，`-1` 表示没有。
    pub special_cost_type: i32,
    /// `+0x2c0`（`special_consume_value`）：上面那一项要扣多少。
    pub special_cost_value: i32,
    /// `TargetRangeType`：1 我方单体 2 我方全体 3 敌单体 4 敌全体（5 一排、6 一列只有怪物的
    /// 「物理攻击」在用）；0 的两条按效果的正负猜。
    pub target_range: u8,
    /// 状态码，按状态**枚举**下标（战斗位的 `k = 下标 + 1`）：1 加、2 解——和道具同一套。
    pub state_codes: [i16; ITEM_STATE_CODES],
}

/// `special_consume_type` == 10：扣钱（全队共用）。
pub const SPECIAL_COST_MONEY: i32 = 10;
/// `special_consume_type` == 16：扣一个「蛊」。
pub const SPECIAL_COST_GU: i32 = 16;
/// 蛊在物品表里的编号。原版把它硬编码在检查与扣除两处（`0x17e1`），
/// 不看技能定义里的量。
pub const GU_ITEM_ID: u32 = 6113;

/// The 五灵 modifier applied to a skill's value.
///
/// SUSPECT, and transcribed as read: the two branches do not mirror each
/// other. A playable caster is compared against **its own** element and
/// gets +50% for a match; a monster caster is compared against the
/// **opponent's** element and gets +10% for overcoming it. Both then lose
/// half if the skill's element is overcome by whatever they compared
/// against. Addresses 0x4f9c09–0x4f9dfe.
pub fn element_modifier(
    mut v: i32,
    caster_is_player: bool,
    elements: (Option<u8>, Option<u8>),
    compared_element: Option<u8>,
) -> i32 {
    let Some(other) = compared_element else {
        return v;
    };
    let (e1, e2) = elements;

    if caster_is_player {
        if e1 == Some(other) || e2 == Some(other) {
            v += v / 2;
        }
        for e in [e1, e2].into_iter().flatten() {
            if formulas::overcome_by(e) == other {
                v -= v / 2;
            }
        }
    } else {
        for e in [e1, e2].into_iter().flatten() {
            if formulas::overcomes(e) == other {
                v += v / 10;
            }
        }
        for e in [e1, e2].into_iter().flatten() {
            if formulas::overcome_by(e) == other {
                v -= v / 2;
            }
        }
    }
    v
}

/// `FUN_004f98b0` kind 2: how much one attribute slot moves before the
/// 五灵 modifier. A raw impact value of zero or less comes back negative,
/// which is how the original encodes damage.
pub fn impact_magnitude(rng: &mut CombatRng, caster_level: i32, impact_value: i16) -> i32 {
    let mut v = ((caster_level as f32 * 0.05 + 1.0) * impact_value as f32) as i32;
    v = v.abs();

    if formulas::SPREAD_ENABLED {
        let spread = (v as f32 * 0.1) as i32;
        if spread != 0 {
            let r = rng.below(spread);
            v = if rng.coin() { v - r } else { v + r };
        }
    }

    if impact_value < 1 { -v } else { v }
}

/// Turn a computed value into an actual change on the target
/// (`FUN_004eef20`, 0x4ef43a). Returns the delta that was applied.
pub fn apply_impact(target: &mut Combatant, slot: usize, kind: ImpactKind, value: i32) -> i32 {
    if slot >= FIRST_DEAD_ATTRIBUTE {
        // The original asserts here rather than doing anything.
        return 0;
    }

    let (current, max) = match slot {
        ATTR_HP => (target.hp, target.max_hp),
        ATTR_SP => (target.sp, target.max_sp),
        _ => (target.mp, target.max_mp),
    };

    let delta = match kind {
        ImpactKind::Absolute => value,
        ImpactKind::Percentage => max * value / 100,
        ImpactKind::RecoverToMax => max - current,
        ImpactKind::IncreaseMax => {
            // Raises the ceiling and leaves the current value alone.
            match slot {
                ATTR_HP => target.max_hp += value,
                ATTR_SP => target.max_sp += value,
                _ => target.max_mp += value,
            }
            return 0;
        }
    };

    match slot {
        ATTR_HP => target.change_hp(delta),
        ATTR_SP => {
            let before = target.sp;
            target.sp = (target.sp + delta).clamp(0, target.max_sp);
            target.sp - before
        }
        _ => {
            let before = target.mp;
            target.mp = (target.mp + delta).clamp(0, target.max_mp);
            target.mp - before
        }
    }
}

/// 战斗里一件道具的效果，从数据库那一行抽出来，和 [`SkillEffect`] 一样只留
/// 战斗算式要用的部分。
#[derive(Debug, Clone)]
pub struct ItemEffect {
    pub id: u32,
    /// `item_type`：1 治疗 2 投掷 3 仙宝 …（托管挑道具时按它分类）。
    pub kind: u8,
    /// `ItemSpecialType`：1 蛊、2 鬼遁水（强制撤离）。
    pub special: u8,
    /// `TargetRangeType`：1 我方单体 2 我方全体 3 敌单体 4 敌全体。
    pub target_range: u8,
    /// 每个属性槽的（影响类型，原始值）。
    pub impacts: [(u8, i16); ATTRIBUTE_SLOTS],
    /// 状态码，按状态**枚举**下标（战斗位的 `k = 下标 + 1`）：1 加、2 解。
    pub state_codes: [i16; ITEM_STATE_CODES],
}

impl Default for ItemEffect {
    fn default() -> Self {
        Self {
            id: 0,
            kind: 0,
            special: 0,
            target_range: 0,
            impacts: [(0, 0); ATTRIBUTE_SLOTS],
            state_codes: [0; ITEM_STATE_CODES],
        }
    }
}

/// `combat_state_impact_value` 的长度。
pub const ITEM_STATE_CODES: usize = 31;
/// 蛊：随机上一种毒（`ItemSpecialType` 1）。
pub const ITEM_SPECIAL_GU: u8 = 1;
/// 鬼遁水：强制撤离（`ItemSpecialType` 2）。
pub const ITEM_SPECIAL_FLEE: u8 = 2;
/// 毒龙胆。打在没中毒的目标上是即死。
pub const POISON_DRAGON_GALL: u32 = 0x1856;

/// 用一件道具的结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemResult {
    /// 每个属性槽实际动了多少。
    pub deltas: [i32; ATTRIBUTE_SLOTS],
    /// 毒龙胆打在没中毒的目标上：直接打死。
    pub instant_kill: bool,
    /// 鬼遁水：这一队强制撤离。
    pub flee: bool,
}

/// What one cast did, for logging and for the UI to replay later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastResult {
    pub hit: bool,
    /// Per attribute slot, how much actually moved.
    pub deltas: [i32; ATTRIBUTE_SLOTS],
}

/// 仙术打不进去的状态（战斗位 k = 22，状态枚举 21）：`FUN_004f1070` 出招前查
/// `FUN_004f3d90(t, 0x16)`，有就跳过这个目标。
pub const MAGIC_SHIELD_STATE: usize = 0x16;

/// 飞龙探云手：`FUN_004f1420` 按 id 调 `FUN_004f17c0` 偷东西。
pub const SKILL_STEAL: u32 = 0x1005;
/// 绿波红露斩：按目标掉的精的一半给施术者回精（`FUN_004f2690`）。
pub const SKILL_DRAIN_HP: u32 = 0x1069;
/// 鬼狱还神：按伤害量给施术者回神（同上，`FUN_004f31c0`）。
pub const SKILL_DRAIN_MP: u32 = 0x10d0;
/// 吸精蛊：在目标身上做标记，之后三个回合吸它的精给紫萱（`FUN_004f2690` 置标记，
/// `FUN_004ecb20` 每回合吸，见 `Battle::own_turn`）。
pub const SKILL_GU_DRAIN: u32 = 0x1132;

/// 特技顺手偷到的东西。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stolen {
    pub money: i32,
    pub item: Option<(u32, i32)>,
}
impl Battle {
    /// 扣掉技能的第三项消耗（`FUN_004f2910` 的那个 switch，分派在 0x4f2985）。
    ///
    /// 原版按 `special_consume_type` 分支：0..4（精气神武防）、9（级）、
    /// 0xb..0xf（五灵）都只弹一个调试提示、不扣任何东西；5 情、6 经、7 速、
    /// 8 运扣在战斗位上；10 扣钱、0x10 扣一个蛊。实机数据里只有后两种真的出
    /// 现过（`pal3_gdb_census`：洒金笺 80 文、乾坤一掷 3000 文，五个蛊术各一
    /// 个蛊），所以这里只实现这两支，其余按原版「不扣」处理。
    ///
    /// 洒金笺（4102）是个例外：原版扣的不是定义里的 80，而是调用方算好后从
    /// `param_3` 传进来的量（可用性检查那一侧按 `2 × (景天等级 + 20) × 敌数`
    /// 再验一遍，`FUN_004f7200` 的 0x4f73b6 分支）。这里先按定义值扣，差额记
    /// 在 `combat_formulas.md`。
    fn pay_special_cost(&mut self, skill: &SkillEffect) {
        match skill.special_cost_type {
            SPECIAL_COST_MONEY => self.spent_money += skill.special_cost_value,
            SPECIAL_COST_GU => self.spent_gu += 1,
            _ => {}
        }
    }

    /// Cast `skill` from `caster` at `target`.
    ///
    /// Costs come off the caster whether or not the skill lands, which is
    /// what the original does — the cost is paid when the action starts.
    pub fn cast_skill(&mut self, caster: usize, target: usize, skill: &SkillEffect) -> CastResult {
        self.pay_skill_cost(caster, skill);
        self.cast_on(caster, target, skill, false)
    }

    /// 扣气、神和第三项消耗（`FUN_004f2910`）。原版在打完所有目标之后扣一次，全体技能
    /// 也只扣一次；在这之前已经用 `FUN_004f7480` / `FUN_004f7200` 按原价确认过付得起。
    /// 气、神走 `FUN_004f3020` / `FUN_004f31c0`，所以装备的减气耗、减神耗在这里生效。
    pub fn pay_skill_cost(&mut self, caster: usize, skill: &SkillEffect) {
        self.hit_sp(caster, -skill.sp_cost);
        self.shift_mp(caster, -skill.mp_cost);
        self.pay_special_cost(skill);
    }

    /// 放一次特技（行动码 0x104，`FUN_004f1420`）。和仙术（`FUN_004f1070`）的差别：
    ///
    /// * 打出伤害才让目标出手段从头来，而且整条清零（`FUN_004db750(t, 0, 0, 1)`）；
    /// * 不记使用次数（调用方负责）；
    /// * 效果后面按 id 多一步（`FUN_004f2690`）：绿波红露斩按目标掉的精的一半给自己回精，
    ///   鬼狱还神按伤害量回神；飞龙探云手打中后偷钱、偷物品（`FUN_004f17c0`）；
    ///   吸精蛊在目标身上做标记（`+0x56e = 1`、`+0x56f = 0`），再中一次从头数。
    pub fn cast_special(
        &mut self,
        caster: usize,
        target: usize,
        skill: &SkillEffect,
    ) -> (CastResult, Stolen) {
        self.pay_skill_cost(caster, skill);
        self.special_on(caster, target, skill)
    }

    /// 特技打在一个目标上（不扣消耗，见 [`Self::cast_special`]）。
    pub fn special_on(
        &mut self,
        caster: usize,
        target: usize,
        skill: &SkillEffect,
    ) -> (CastResult, Stolen) {
        let hp_before = self.get(target).map(|t| t.hp).unwrap_or(0);
        let result = self.cast_on(caster, target, skill, true);
        let mut stolen = Stolen::default();
        if !result.hit {
            return (result, stolen);
        }
        match skill.id {
            SKILL_STEAL => stolen = self.steal(target),
            SKILL_DRAIN_HP => {
                let hp_after = self.get(target).map(|t| t.hp).unwrap_or(hp_before);
                let amount = (hp_before - hp_after).abs() / 2;
                if let Some(c) = self.get_mut(caster) {
                    c.change_hp(amount);
                }
            }
            SKILL_DRAIN_MP => {
                let amount = result.deltas[0].abs();
                if let Some(c) = self.get_mut(caster) {
                    c.mp = (c.mp + amount).min(c.max_mp);
                }
            }
            SKILL_GU_DRAIN => {
                if let Some(t) = self.get_mut(target) {
                    t.gu_drain = Some(0);
                }
            }
            _ => {}
        }
        (result, stolen)
    }

    /// 飞龙探云手（`FUN_004f17c0`）：先偷钱，`rand % 剩余 + 1` 文，从怪物身上扣掉、记到队伍
    /// 身上（和花掉的钱一起散场写回）；再偷物品，`rand % 剩余件数 + 1` 件。
    pub(super) fn steal(&mut self, target: usize) -> Stolen {
        let mut stolen = Stolen::default();
        let money = self.get(target).map(|t| t.steal_money).unwrap_or(0);
        if money > 0 {
            let n = (self.rng.rand15() as i32) % money + 1;
            if let Some(t) = self.get_mut(target) {
                t.steal_money -= n;
            }
            self.spent_money -= n;
            stolen.money = n;
        }
        if let Some((id, count)) = self.get(target).and_then(|t| t.steal_item) {
            if count > 0 {
                let n = (self.rng.rand15() as i32) % count + 1;
                if let Some(t) = self.get_mut(target) {
                    t.steal_item = Some((id, count - n));
                }
                self.stolen_items.push((id, n));
                stolen.item = Some((id, n));
            }
        }
        stolen
    }

    /// 技能打在一个目标上，不扣消耗。`special` 是特技（`FUN_004f1420`），否则仙术（`FUN_004f1070`）。
    ///
    /// 仙术先看目标有没有 22 号状态（`FUN_004f3d90(t, 0x16)`，状态枚举 21，雷界那一类），有就整招
    /// 跳过；特技没有这一步。打中后先上状态、再改属性（`FUN_004f2660` / `FUN_004f2690` 都是先
    /// `FUN_004ef4a0` 再 `FUN_004eef20`）。仙术效果最前面还有一步 `FUN_004f2350`：拿技能的
    /// `special_skill_id` 调 `FUN_004f1fa0`——数据里只有特技带这个值，仙术全是 0，没做。
    pub fn cast_on(
        &mut self,
        caster: usize,
        target: usize,
        skill: &SkillEffect,
        special: bool,
    ) -> CastResult {
        let mut result = CastResult {
            hit: false,
            deltas: [0; ATTRIBUTE_SLOTS],
        };

        let Some(c) = self.get(caster) else {
            return result;
        };
        let caster_level = c.level;
        let caster_luck = c.luck_eff();
        let caster_is_player = c.side == Side::Party;
        let caster_element = (1..=5u8).find(|e| c.elements[*e as usize]);
        // 装备的命中加成（`+0x29c`），`FUN_004f98b0` kind 1 用。
        let caster_bonus = c.skill_bonus as f32;

        let Some(t) = self.get(target) else {
            return result;
        };
        if !special && t.has_state(MAGIC_SHIELD_STATE) {
            return result;
        }
        let target_luck = t.luck_eff();
        let target_element = (1..=5u8).find(|e| t.elements[*e as usize]);

        if !formulas::skill_hits(
            &mut self.rng,
            skill.id,
            skill.success_rate_level,
            caster_luck,
            target_luck,
            caster_bonus,
        ) {
            return result;
        }
        result.hit = true;

        for (i, code) in skill.state_codes.iter().copied().enumerate() {
            match code {
                1 => self.set_state(target, i + 1, true),
                2 => self.set_state(target, i + 1, false),
                _ => {}
            }
        }

        // The side being compared against differs by caster — see
        // `element_modifier`.
        let compared = if caster_is_player {
            caster_element
        } else {
            target_element
        };

        for slot in 0..FIRST_DEAD_ATTRIBUTE {
            let (kind, raw) = skill.impacts[slot];
            if raw == 0 {
                continue;
            }
            let v = impact_magnitude(&mut self.rng, caster_level, raw);
            let v = element_modifier(v, caster_is_player, skill.elements, compared);
            if let Some(t) = self.get_mut(target) {
                result.deltas[slot] = apply_impact(t, slot, ImpactKind::from_u8(kind), v);
            }
        }

        if special {
            // 特技只有打出伤害才打断，而且出手段整条清零（`FUN_004f1420`：`FUN_004f2690`
            // 的结果 < 0 时 `FUN_004db750(t, 0, 0, 1)`）。
            if result.deltas[0] < 0 {
                self.restart_cast(target);
            }
        } else {
            // A landed skill knocks the target out of whatever it was doing.
            self.interrupt(target, Some(skill.level));
        }
        result
    }

    /// 用一件道具打在 `target` 身上（`FUN_004f2750`）。
    ///
    /// 顺序照原版：毒龙胆的即死支 → special（`FUN_004f1fa0`）→ 状态
    /// （`FUN_004ef4a0`）→ 属性（`FUN_004eef20`）。属性用的是数据库里的
    /// **原始值**，不像技能那样按施术者等级放大、也不过五灵修正；原版只在
    /// 我方使用者身上多加 `round(v × 使用者的 +0x2b4 × 0.01)`（装备特殊 0x16
    /// 「道具效果增强」，且只对 v>0 生效；聚宝盆 0x15 也给 +10，见
    /// `Combatant::item_boost`）。
    ///
    /// 扣数量不在这里：战斗里用掉的记在会话上，散场时一起写回。
    pub fn use_item(&mut self, user: usize, target: usize, item: &ItemEffect) -> ItemResult {
        let mut result = ItemResult::default();
        let Some(t) = self.get(target) else {
            return result;
        };

        // 毒龙胆打在**没中毒**的目标上 → `FUN_004f2c70(target, −99,999,999, …)`。
        if item.id == POISON_DRAGON_GALL {
            let poisoned = (1..=5).any(|k| t.has_state(k));
            if !poisoned {
                if let Some(t) = self.get_mut(target) {
                    result.deltas[ATTR_HP] = t.change_hp(-99_999_999);
                }
                result.instant_kill = true;
                return result;
            }
        }

        match item.special {
            // 蛊（`FUN_004f3d10`）：避毒挡住；已中毒的不再上；否则随机一种。
            ITEM_SPECIAL_GU => {
                let (immune, poisoned) = self
                    .get(target)
                    .map(|t| {
                        (
                            t.has_state(K_POISON_IMMUNE),
                            (1..=5).any(|k| t.has_state(k)),
                        )
                    })
                    .unwrap_or((true, true));
                if !immune && !poisoned {
                    let k = (self.rng.rand15() % 5 + 1) as usize;
                    self.set_state(target, k, true);
                }
            }
            // 鬼遁水：强制撤离，禁止逃跑时不行。调用方负责让全队离场。
            ITEM_SPECIAL_FLEE => result.flee = !self.escape_forbidden,
            _ => {}
        }

        // 状态：码 1 加、2 解，下标是状态枚举，战斗位的 k 要 +1。
        for (i, code) in item.state_codes.iter().copied().enumerate() {
            match code {
                1 => self.set_state(target, i + 1, true),
                2 => self.set_state(target, i + 1, false),
                _ => {}
            }
        }

        // 属性。
        for slot in 0..FIRST_DEAD_ATTRIBUTE {
            let (kind, raw) = item.impacts[slot];
            if raw == 0 {
                continue;
            }
            let mut v = raw as i32;
            // 我方使用者的装备「道具效果增强」（`+0x2b4`），只加正数。
            if v > 0 && user < super::state::PARTY_SLOTS {
                let boost = self.get(user).map(|u| u.item_boost).unwrap_or(0);
                v += (v as f64 * boost as f64 * 0.01f32 as f64).round_ties_even() as i32;
            }
            if let Some(t) = self.get_mut(target) {
                result.deltas[slot] = apply_impact(t, slot, ImpactKind::from_u8(kind), v);
            }
        }

        // 掉了血的活目标要被打断（`FUN_004f1a30` 收尾那一步）。
        if result.deltas[ATTR_HP] < 0 && self.get(target).map(|t| t.alive).unwrap_or(false) {
            self.interrupt(target, None);
        }
        result
    }

    /// 防御 —— 置上「受伤减半」，然后结束这一回合。
    ///
    /// 原版的防御任务是 `FUN_0050eda0`（从 `FUN_005014e0` 注册进任务表）：
    /// 播完动画后 `FUN_004f3a30(slot, 1)` 把战斗位的 `+0x474` 置 1
    /// （0x50ee64）。清零在 0x5158bf 的 `FUN_004f3a30(slot, 0)`，条件是该单位
    /// 行动条又满了或战斗阶段不再是 2——**所以减半持续到防御者下次轮到自己**。
    /// 扣血时 `FUN_004f2c70` 对 `delta < -1` 折半。
    pub fn defend(&mut self, slot: usize) {
        if let Some(c) = self.get_mut(slot) {
            c.defending = true;
            c.phase = Phase::Charging;
            c.gauge = 0.;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openpal3::combat::state::{Combatant, K_POISON_FIRE, K_POISON_WATER, K_SLEEP, Side};

    fn caster() -> Combatant {
        let mut c = Combatant::new(0, Side::Party);
        c.level = 10;
        c.hp = 100;
        c.max_hp = 100;
        c.sp = 50;
        c.max_sp = 50;
        c.mp = 40;
        c.max_mp = 40;
        c
    }

    /// 必中的一招：成功率等级大于 10 按百分比算，100 就是必中。
    fn sure_hit(id: u32, hp: i16) -> SkillEffect {
        let mut impacts = [(0u8, 0i16); ATTRIBUTE_SLOTS];
        impacts[ATTR_HP] = (0, hp);
        SkillEffect {
            id,
            success_rate_level: 100,
            impacts,
            special_cost_type: -1,
            ..Default::default()
        }
    }

    #[test]
    fn a_landed_skill_sets_its_states_first() {
        let mut b = Battle::new(3);
        b.place(0, caster());
        b.place(1, caster());
        let mut skill = sure_hit(0x139, 0);
        skill.state_codes[5] = 1; // 天罡战气：状态枚举 5 → 战斗位 k = 6
        let r = b.cast_skill(0, 1, &skill);
        assert!(r.hit);
        assert!(b.get(1).unwrap().has_state(6));
    }

    #[test]
    fn the_magic_shield_stops_magic_but_not_specials() {
        let mut b = Battle::new(3);
        b.place(0, caster());
        let mut monster = caster();
        monster.side = Side::Enemy;
        monster.hp = 500;
        monster.max_hp = 500;
        b.place(5, monster);
        b.set_state(5, MAGIC_SHIELD_STATE, true);
        assert!(b.get(5).unwrap().has_state(MAGIC_SHIELD_STATE));
        let r = b.cast_skill(0, 5, &sure_hit(0x200, -30));
        assert!(!r.hit, "雷界挡仙术");
        assert_eq!(b.get(5).unwrap().hp, 500);
        let (r, _) = b.cast_special(0, 5, &sure_hit(0x1006, -30));
        assert!(r.hit, "特技不看雷界");
        assert!(b.get(5).unwrap().hp < 500);
    }

    #[test]
    fn a_reviving_skill_brings_the_fallen_back() {
        let mut b = Battle::new(3);
        b.place(0, caster());
        let mut fallen = caster();
        fallen.hp = 0;
        fallen.alive = false;
        b.place(1, fallen);
        let mut skill = sure_hit(0x106a, 50);
        skill.impacts[ATTR_HP] = (1, 50); // 百分比回精
        skill.state_codes[24] = 2; // 解「死亡」
        let r = b.cast_skill(0, 1, &skill);
        assert!(r.hit);
        let c = b.get(1).unwrap();
        assert!(c.alive && c.hp > 0, "{} {}", c.alive, c.hp);
    }

    #[test]
    fn the_stealing_special_takes_money_and_an_item() {
        let mut b = Battle::new(11);
        b.place(0, caster());
        let mut monster = caster();
        monster.side = Side::Enemy;
        monster.hp = 500;
        monster.max_hp = 500;
        monster.steal_money = 50;
        monster.steal_item = Some((0x1234, 2));
        b.place(5, monster);

        let (r, stolen) = b.cast_special(0, 5, &sure_hit(SKILL_STEAL, -5));
        assert!(r.hit);
        assert!((1..=50).contains(&stolen.money), "{stolen:?}");
        assert_eq!(
            b.get(5).unwrap().steal_money,
            50 - stolen.money,
            "偷一次少一次"
        );
        assert_eq!(
            b.spent_money, -stolen.money,
            "偷到的钱记成负的花费，散场一起写回"
        );
        let (id, n) = stolen.item.unwrap();
        assert_eq!(id, 0x1234);
        assert!((1..=2).contains(&n));
        assert_eq!(b.stolen_items, vec![(0x1234, n)]);
        assert_eq!(b.get(5).unwrap().steal_item, Some((0x1234, 2 - n)));
    }

    #[test]
    fn the_draining_special_heals_the_caster_by_half_the_damage() {
        let mut b = Battle::new(5);
        let mut me = caster();
        me.hp = 40;
        b.place(0, me);
        let mut monster = caster();
        monster.side = Side::Enemy;
        monster.hp = 500;
        monster.max_hp = 500;
        b.place(5, monster);

        let (r, _) = b.cast_special(0, 5, &sure_hit(SKILL_DRAIN_HP, -30));
        assert!(r.hit);
        let dealt = 500 - b.get(5).unwrap().hp;
        assert!(dealt > 0);
        assert_eq!(b.get(0).unwrap().hp, (40 + dealt / 2).min(100));
    }

    #[test]
    fn the_gu_drain_feeds_slot_three_for_three_rounds() {
        let mut b = Battle::new(5);
        b.place(0, caster());
        let mut zixuan = caster();
        zixuan.hp = 10;
        zixuan.max_hp = 1000;
        b.place(3, zixuan);
        let mut monster = caster();
        monster.side = Side::Enemy;
        monster.hp = 350;
        monster.max_hp = 350;
        b.place(5, monster);

        let (r, _) = b.cast_special(0, 5, &sure_hit(SKILL_GU_DRAIN, 0));
        assert!(r.hit);
        assert_eq!(b.get(5).unwrap().gu_drain, Some(0));
        let hp = |b: &Battle, s| b.get(s).unwrap().hp;
        let before = (hp(&b, 5), hp(&b, 3));
        // 350 × 0.05 = 17.5 → 17，× 0.1 = 35，× 0.15 = 52.5 → 52。
        for (n, amount) in [(1, 17), (2, 35), (3, 52), (4, 0)] {
            let (t, z) = (hp(&b, 5), hp(&b, 3));
            b.own_turn(5);
            assert_eq!(hp(&b, 5), t - amount, "第 {n} 回合");
            assert_eq!(hp(&b, 3), z + amount, "第 {n} 回合");
        }
        assert_eq!(b.get(5).unwrap().gu_drain, None, "第 4 回合摘掉");
        b.own_turn(5);
        assert_eq!((hp(&b, 5), hp(&b, 3)), (before.0 - 104, before.1 + 104));
    }

    #[test]
    fn a_gu_drain_kill_is_credited_to_slot_three() {
        let mut b = Battle::new(5);
        b.place(3, caster());
        let mut monster = caster();
        monster.side = Side::Enemy;
        monster.hp = 10;
        monster.max_hp = 1000;
        monster.gu_drain = Some(0);
        b.place(5, monster);
        b.own_turn(5);
        let m = b.get(5).unwrap();
        assert!(!m.alive);
        assert_eq!(m.killed_by, Some(3));
    }

    #[test]
    fn only_the_first_three_attributes_do_anything() {
        let mut t = caster();
        // 武 is slot 3 — the original asserts instead of applying.
        assert_eq!(apply_impact(&mut t, 3, ImpactKind::Absolute, -50), 0);
        assert_eq!(apply_impact(&mut t, 11, ImpactKind::Absolute, -50), 0);
        assert_eq!(t.hp, 100);
    }

    #[test]
    fn the_four_impact_kinds() {
        let mut t = caster();
        t.hp = 40;

        assert_eq!(apply_impact(&mut t, ATTR_HP, ImpactKind::Absolute, 15), 15);
        assert_eq!(t.hp, 55);

        // Percentage is of the maximum, not of what is left.
        assert_eq!(
            apply_impact(&mut t, ATTR_HP, ImpactKind::Percentage, 10),
            10
        );
        assert_eq!(t.hp, 65);

        assert_eq!(
            apply_impact(&mut t, ATTR_HP, ImpactKind::RecoverToMax, 0),
            35
        );
        assert_eq!(t.hp, 100);

        // Raising the ceiling leaves the current value where it is.
        assert_eq!(
            apply_impact(&mut t, ATTR_HP, ImpactKind::IncreaseMax, 20),
            0
        );
        assert_eq!(t.max_hp, 120);
        assert_eq!(t.hp, 100);
    }

    #[test]
    fn a_negative_impact_value_means_damage() {
        let mut rng = CombatRng::new(4);
        assert!(impact_magnitude(&mut rng, 10, -30) < 0);
        assert!(impact_magnitude(&mut rng, 10, 30) > 0);
    }

    #[test]
    fn impact_scales_with_caster_level() {
        // Spread makes single samples noisy, so compare batches.
        let mut low = CombatRng::new(9);
        let mut high = CombatRng::new(9);
        let a: i32 = (0..100).map(|_| impact_magnitude(&mut low, 1, 100)).sum();
        let b: i32 = (0..100).map(|_| impact_magnitude(&mut high, 50, 100)).sum();
        assert!(b > a * 2, "50 级应显著强于 1 级: {a} vs {b}");
    }

    #[test]
    fn element_modifier_is_asymmetric_between_sides() {
        // A playable caster whose own element matches the skill gets +50%.
        let same = element_modifier(
            100,
            true,
            (Some(formulas::ELEM_WATER), None),
            Some(formulas::ELEM_WATER),
        );
        assert_eq!(same, 150);

        // 水 is overcome by 土, so a 土 caster loses half.
        let weak = element_modifier(
            100,
            true,
            (Some(formulas::ELEM_WATER), None),
            Some(formulas::ELEM_EARTH),
        );
        assert_eq!(weak, 50);

        // A monster instead compares against the target: 水 overcomes 火,
        // which is worth only +10%.
        let monster = element_modifier(
            100,
            false,
            (Some(formulas::ELEM_WATER), None),
            Some(formulas::ELEM_FIRE),
        );
        assert_eq!(monster, 110);
    }

    #[test]
    fn casting_pays_the_cost_even_when_it_misses() {
        let mut b = Battle::new(2);
        b.place(0, caster());
        let mut enemy = Combatant::new(100, Side::Enemy);
        enemy.hp = 200;
        enemy.max_hp = 200;
        enemy.luck = 99;
        b.place(5, enemy);

        let skill = SkillEffect {
            id: 1,
            level: 1,
            // 50 never lands — the original's branch for 2..99.
            success_rate_level: 50,
            sp_cost: 5,
            mp_cost: 7,
            special_cost_type: -1,
            special_cost_value: 0,
            ..Default::default()
        };
        let result = b.cast_skill(0, 5, &skill);
        assert!(!result.hit);
        assert_eq!(b.get(0).unwrap().sp, 45, "气照扣");
        assert_eq!(b.get(0).unwrap().mp, 33, "神照扣");
        assert_eq!(b.get(5).unwrap().hp, 200, "没打中就没伤害");
    }

    #[test]
    fn money_and_gu_skills_charge_the_party_purse() {
        let mut b = Battle::new(2);
        b.place(0, caster());
        let mut enemy = Combatant::new(100, Side::Enemy);
        enemy.hp = 200;
        enemy.max_hp = 200;
        b.place(5, enemy);
        b.money = 500;
        b.gu = 2;

        // 洒金笺那一档：定义值 80。
        let mut skill = SkillEffect {
            id: 4102,
            level: 1,
            success_rate_level: 100,
            special_cost_type: SPECIAL_COST_MONEY,
            special_cost_value: 80,
            ..Default::default()
        };
        b.cast_skill(0, 5, &skill);
        assert_eq!(b.spent_money, 80);
        assert_eq!(b.money_left(), 420);

        // 蛊术：不看定义里的量，一次一个。
        skill.id = 4401;
        skill.special_cost_type = SPECIAL_COST_GU;
        skill.special_cost_value = 1;
        b.cast_skill(0, 5, &skill);
        b.cast_skill(0, 5, &skill);
        assert_eq!(b.spent_gu, 2);
        assert_eq!(b.gu_left(), 0);

        // 别的种类原版只弹提示、什么都不扣。
        skill.special_cost_type = 5;
        skill.special_cost_value = 30;
        b.cast_skill(0, 5, &skill);
        assert_eq!(b.spent_money, 80, "情那一档不该动到钱");
    }

    #[test]
    fn a_damaging_skill_lands_and_interrupts() {
        let mut b = Battle::new(2);
        b.place(0, caster());
        let mut enemy = Combatant::new(100, Side::Enemy);
        enemy.hp = 500;
        enemy.max_hp = 500;
        enemy.phase = Phase::Executing;
        enemy.gauge = 9000.;
        b.place(5, enemy);

        let mut impacts = [(0u8, 0i16); ATTRIBUTE_SLOTS];
        impacts[ATTR_HP] = (0, -60);
        let skill = SkillEffect {
            id: 1,
            level: 2,
            success_rate_level: 100,
            impacts,
            ..Default::default()
        };

        let result = b.cast_skill(0, 5, &skill);
        assert!(result.hit);
        assert!(result.deltas[ATTR_HP] < 0, "应当扣血");
        assert!(b.get(5).unwrap().hp < 500);
        assert_eq!(b.get(5).unwrap().phase, Phase::Charging, "被打断");
    }

    fn item(id: u32) -> ItemEffect {
        ItemEffect {
            id,
            target_range: 1,
            ..Default::default()
        }
    }

    fn two_sides() -> Battle {
        let mut b = Battle::new(9);
        b.place(0, caster());
        let mut enemy = Combatant::new(100, Side::Enemy);
        enemy.hp = 300;
        enemy.max_hp = 300;
        b.place(5, enemy);
        b
    }

    #[test]
    fn an_items_value_is_used_raw() {
        // 技能按施术者等级放大，道具不放大：10 级用一瓶「精 +40」还是 40。
        let mut b = two_sides();
        b.get_mut(0).unwrap().hp = 20;
        let mut it = item(6201);
        it.impacts[ATTR_HP] = (0, 40);
        let r = b.use_item(0, 0, &it);
        assert_eq!(r.deltas[ATTR_HP], 40);
        assert_eq!(b.get(0).unwrap().hp, 60);
    }

    #[test]
    fn the_gall_kills_a_clean_target_and_only_hurts_a_poisoned_one() {
        let mut b = two_sides();
        let mut gall = item(POISON_DRAGON_GALL);
        gall.target_range = 3;
        gall.impacts[ATTR_HP] = (0, -10);

        let r = b.use_item(0, 5, &gall);
        assert!(r.instant_kill);
        assert!(!b.get(5).unwrap().alive, "没中毒的直接打死");

        // 中了毒的走普通那一支。
        let mut b = two_sides();
        b.set_state(5, K_POISON_FIRE, true);
        let r = b.use_item(0, 5, &gall);
        assert!(!r.instant_kill);
        assert_eq!(b.get(5).unwrap().hp, 290);
    }

    #[test]
    fn state_codes_add_and_clear() {
        let mut b = two_sides();
        let mut incense = item(6214);
        // 码[2] = 1 上水毒；码[16] = 2 解眠。
        incense.state_codes[2] = 1;
        incense.state_codes[16] = 2;
        b.set_state(0, K_SLEEP, true);
        b.use_item(0, 0, &incense);
        assert!(b.get(0).unwrap().has_state(K_POISON_WATER));
        assert!(!b.get(0).unwrap().has_state(K_SLEEP));
    }

    #[test]
    fn a_revival_item_stands_the_target_back_up_before_healing_it() {
        let mut b = two_sides();
        let mut pill = item(6231);
        // 码[24] = 2：解「死亡」。
        pill.state_codes[24] = 2;
        pill.impacts[ATTR_HP] = (0, 30);
        b.get_mut(0).unwrap().die();

        b.use_item(0, 0, &pill);
        assert!(b.get(0).unwrap().alive, "先复活");
        assert_eq!(b.get(0).unwrap().hp, 30, "复活是 0 精，再由属性抬起来");
    }

    #[test]
    fn ghost_water_takes_the_party_off_the_field() {
        let mut b = two_sides();
        let mut water = item(6222);
        water.special = ITEM_SPECIAL_FLEE;
        assert!(b.use_item(0, 0, &water).flee);

        // 禁止逃跑的战斗里不行。
        let mut b = two_sides();
        b.escape_forbidden = true;
        assert!(!b.use_item(0, 0, &water).flee);
    }

    #[test]
    fn defending_just_ends_the_turn() {
        let mut b = Battle::new(2);
        b.place(0, caster());
        b.get_mut(0).unwrap().gauge = 11500.;
        b.defend(0);
        assert_eq!(b.get(0).unwrap().gauge, 0.);
        assert_eq!(b.get(0).unwrap().phase, Phase::Charging);
    }
}
