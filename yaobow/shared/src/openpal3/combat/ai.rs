//! How a combatant that nobody is steering decides what to do.
//!
//! Reverse-engineering write-up: `switch/re/notes/combat_ai.md`. Three
//! findings shape this module and are worth stating up front, because
//! none of them is what you would guess:
//!
//! * **A monster's playstyle is chosen by how badly hurt it is**, not by
//!   any field in its database row — [`hp_band`] picks one of four
//!   behaviours, and 乱 overrides the lot.
//! * **Skills are picked by closest match, not by largest effect**
//!   (`closest_by`). A healer therefore avoids overhealing and an
//!   attacker prefers the spell that just about finishes the job.
//! * **Targets are picked by a coin toss down the slot list** — the
//!   monster ladders ([`monster`]) start at the first slot that passes
//!   `rand % 3 == 1` and then try every slot after it. Not the nearest,
//!   not the weakest.
//!
//! The monster ladders roll the same generator as the damage and crit
//! rolls (`DAT_005dfdec`), so the session hands them [`Battle::rng`].

use super::formulas::CombatRng;
use serde::{Deserialize, Serialize};

use super::state::{Battle, Combatant, K_CONFUSED, K_EVADE, K_INVISIBLE, PARTY_SLOTS, SLOTS, Side};

/// Which behaviour a combatant uses, from `FUN_004f3a50` +
/// `FUN_004d7120`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    /// 满血 or 半血以上 — the normal one.
    Healthy,
    /// 四分之一以上.
    Hurt,
    /// 残血.
    Critical,
    /// 已倒 — takes no turn at all.
    Down,
    /// 乱: overrides every band except 已倒.
    Confused,
}

/// `FUN_004f3a50`: 1 满血 · 2 半血以上 · 3 四分之一以上 · 4 残血 · 5 已倒.
///
/// The quarter test is written `(max + (max >> 31 & 3)) >> 2 <= cur` in
/// the original — an arithmetic shift that rounds toward zero for
/// negative maxima. Kept in that form rather than `max / 4`.
pub fn hp_band(hp: i32, max_hp: i32) -> u8 {
    if hp == max_hp {
        return 1;
    }
    if max_hp / 2 <= hp && hp < max_hp {
        return 2;
    }
    if (max_hp + ((max_hp >> 31) & 3)) >> 2 <= hp {
        return 3;
    }
    if hp < 1 { 5 } else { 4 }
}

/// The behaviour a combatant acts with this turn.
pub fn behaviour(hp: i32, max_hp: i32, confused: bool) -> Behaviour {
    let band = hp_band(hp, max_hp);
    if band == 5 {
        return Behaviour::Down;
    }
    if confused {
        return Behaviour::Confused;
    }
    match band {
        1 | 2 => Behaviour::Healthy,
        3 => Behaviour::Hurt,
        _ => Behaviour::Critical,
    }
}

/// The probability gates the action ladder uses, as raw
/// `rand % 1024 <` thresholds.
pub const CHANCE_HALF: i32 = 0x200;
pub const CHANCE_37: i32 = 0x17b;
pub const CHANCE_30: i32 = 0x133;

/// Roll one of the ladder's gates.
pub fn passes(rng: &mut CombatRng, threshold: i32) -> bool {
    rng.rand15() % 1024 < threshold
}

/// Whether a plain attack is allowed against this target.
///
/// 隐形 refuses outright. Against 避 or 镜 a party member under
/// delegation refuses too, while a monster swings anyway about 30% of the
/// time — the asymmetry is the original's.
pub fn basic_attack_allowed(
    rng: &mut CombatRng,
    attacker_is_party: bool,
    target_invisible: bool,
    target_evading_or_mirrored: bool,
) -> bool {
    if target_invisible {
        return false;
    }
    if !target_evading_or_mirrored {
        return true;
    }
    if attacker_is_party {
        return false;
    }
    passes(rng, CHANCE_30)
}

// ---------------------------------------------------------------- 我方托管

/// 我方托管的六个开关：战斗位 `+0x568..+0x56d`。开战时 `FUN_004ed250` 从冒险
/// 记录 `+0x294..+0x299` 拷进来（`+0x294`→`+0x56d`，`+0x295..+0x299`→
/// `+0x568..+0x56c`），胜利时 `FUN_004ed800` 原样写回。新档（0x40bdbf）只有
/// 「谨慎」是 1，其余全 0。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delegation {
    /// `+0x56d`：面板上「手动 / AI」那一格。开着的人轮到时不出控制盘，直接交给
    /// [`delegate`]（主调度 0x51394b 查战斗位记录 `+0x345`，走 0x513a36 那一支）。
    pub on: bool,
    /// `+0x568` 攻击。
    pub attack: bool,
    /// `+0x569` 仙术。
    pub magic: bool,
    /// `+0x56a` 特技。
    pub special: bool,
    /// `+0x56b` 物品。
    pub item: bool,
    /// `+0x56c`：置位时先走顾自己人那一套（`FUN_004d8760`），再走进攻那一套。
    /// 面板上这一格画的是反的：勾上（`affirmative1`「全力」）= 0。
    pub cautious: bool,
}

impl Delegation {
    /// 六个开关全开：怪物开局 `FUN_004f8bb0` 就这么写；快进代打也用它。
    pub fn all_on() -> Self {
        Self {
            on: true,
            attack: true,
            magic: true,
            special: true,
            item: true,
            cautious: true,
        }
    }
}

impl Default for Delegation {
    fn default() -> Self {
        Self {
            on: false,
            attack: false,
            magic: false,
            special: false,
            item: false,
            cautious: true,
        }
    }
}

/// 托管面板上的六个勾选框，按控件 id 0xd6..0xdb 的顺序——也就是两列三行从左到
/// 右、从上到下：攻击 物品 / 特技 仙术 / 谨慎·全力 手动·AI（`FUN_0051b210`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    Attack,
    Item,
    Special,
    Magic,
    AllOut,
    Ai,
}

impl Switch {
    pub const ALL: [Switch; 6] = [
        Switch::Attack,
        Switch::Item,
        Switch::Special,
        Switch::Magic,
        Switch::AllOut,
        Switch::Ai,
    ];

    /// 贴图主干：`ui\combat\control\AI\<主干><0|1>.tga`，0 是没勾（画着禁止
    /// 符号 / 「谨慎」/「手动」），1 是勾上。
    pub fn stem(self) -> &'static str {
        match self {
            Switch::Attack => "attack",
            Switch::Item => "item",
            Switch::Special => "skill",
            Switch::Magic => "magic",
            Switch::AllOut => "affirmative",
            Switch::Ai => "ai",
        }
    }
}

impl Delegation {
    /// 这一格现在是不是勾上的（画 `<主干>1`）。面板打开时由 `FUN_004f9790` 按
    /// 战斗位记录摆好。
    pub fn shows(&self, s: Switch) -> bool {
        match s {
            Switch::Attack => self.attack,
            Switch::Item => self.item,
            Switch::Special => self.special,
            Switch::Magic => self.magic,
            Switch::AllOut => !self.cautious,
            Switch::Ai => self.on,
        }
    }

    fn set(&mut self, s: Switch, v: bool) {
        match s {
            Switch::Attack => self.attack = v,
            Switch::Item => self.item = v,
            Switch::Special => self.special = v,
            Switch::Magic => self.magic = v,
            Switch::AllOut => self.cautious = !v,
            Switch::Ai => self.on = v,
        }
    }

    /// 点一下勾选框（面板状态函数 0x4dd650，按控件 id 走跳表 0x4ddc70）。返回
    /// `false` 是原版不让改的情形：放 wa004、把框弹回原样；改成了放 wa001。
    ///
    /// * AI（0xdb）：总能改。勾上顺带勾上攻击；取消就把攻击、物品、特技、仙术
    ///   一起清掉（0x4dd7e9）。
    /// * 攻击（0xd6）：勾上要 AI 开着；取消会连带清掉物品、特技、仙术和 AI 本身
    ///   （0x4dd86a）。
    /// * 物品、特技、仙术、全力：AI 没开就不许动。
    pub fn toggle(&mut self, s: Switch) -> bool {
        let v = !self.shows(s);
        match s {
            Switch::Ai => {
                self.on = v;
                self.attack = v;
                if !v {
                    self.item = false;
                    self.special = false;
                    self.magic = false;
                }
            }
            Switch::Attack => {
                if v && !self.on {
                    return false;
                }
                self.attack = v;
                if !v {
                    self.item = false;
                    self.special = false;
                    self.magic = false;
                    self.on = false;
                }
            }
            _ => {
                if !self.on {
                    return false;
                }
                self.set(s, v);
            }
        }
        true
    }
}

/// 托管时掂量的一件家伙：仙术、特技或物品里的一条。
#[derive(Debug, Clone, Copy)]
pub struct Tool {
    /// 在各自列表（仙术 / 特技 / 战斗道具）里的下标。
    pub index: usize,
    /// 技能 / 道具的 id（重楼变身那一招按它找）。
    pub id: u32,
    /// 仙术、特技是 `SkillType`（0 特技 1 辅助 2 伤害 3 恢复 …），物品是
    /// `item_type`（1 治疗 2 投掷 …）。
    pub kind: u8,
    /// `TargetRangeType`：1 我方单体 2 我方全体 3 敌单体 4 敌全体。
    pub range: u8,
    /// 对精的影响：（类型, 值）。技能只看值（`FUN_004f4aa0`）；物品的类型
    /// 0 绝对 1 百分比 2 回满（`FUN_004f4c80`）。
    pub hp: (u8, i16),
    /// 状态码，按状态枚举下标（战斗位 `k = 下标 + 1`）：1 加、2 解。
    pub states: [i16; 31],
    /// 会用、付得起（`FUN_004f7500`）；物品是还有剩。
    pub usable: bool,
}

impl Tool {
    /// `FUN_004f6cd0`：码[24] == 2，解「死亡」。
    fn revives(&self) -> bool {
        self.states[24] == 2
    }

    /// `FUN_004f6d30(cmd, k)`：能解状态 k（1 基）。
    fn cures(&self, k: usize) -> bool {
        k >= 1 && self.states.get(k - 1) == Some(&2)
    }

    /// `FUN_004f6ad0`：治疗类道具——`item_type == 1`，且精那一项是回满，或
    /// 绝对值 / 百分比为正。
    fn heals_as_item(&self) -> bool {
        self.kind == 1
            && match self.hp.0 {
                0 | 1 => self.hp.1 > 0,
                2 => true,
                _ => false,
            }
    }

    /// `FUN_004f6b70`：投掷类伤害道具——`item_type == 2`，精那一项是绝对值 /
    /// 百分比且为负。
    fn harms_as_item(&self) -> bool {
        self.kind == 2 && self.hp.0 < 2 && self.hp.1 < 0
    }
}

/// 托管的人手里的三份家伙。
pub struct Kit<'a> {
    pub magic: &'a [Tool],
    pub specials: &'a [Tool],
    pub items: &'a [Tool],
}

/// 托管挑出来的行动。全体的招式也带一个目标位（原版填 −1），只用来定打哪一边。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Attack { target: usize },
    Magic { target: usize, index: usize },
    Special { target: usize, index: usize },
    Item { target: usize, index: usize },
}

/// 状态 0x13。`combat_ai.md` 叫它「狂」，移植的状态表还没给它起名：中了它的
/// 人所有技能 / 道具动作都不做，普攻照打。
const K_0X13: usize = 0x13;
/// 状态 0x14（与避 0x15 同时在身时，托管的人不普攻它）。
const K_0X14: usize = 0x14;

/// `FUN_004f3c20`：这些状态不算「要解的毛病」（增益、避、镜、护盾、隐形之类）。
fn cure_exempt(k: usize) -> bool {
    matches!(
        k,
        0 | 6 | 8 | 10 | 12 | 0x14 | 0x15 | 0x16 | 0x17 | 0x1a | 0x1b
    )
}

/// `FUN_004d94f0` 按一方掉到四分之一以下（血档 > 2）的人数给的量：不到两个 100、
/// 两个 60、三个 40、更多 20。
fn spread(hurt: usize) -> i32 {
    match hurt {
        0 | 1 => 100,
        2 => 60,
        3 => 40,
        _ => 20,
    }
}

/// 取「分值绝对值」与「想要的量绝对值」相差最小的一条，同分取先到的（原版严格 `<`）。
fn closest_by(tools: impl Iterator<Item = (usize, i32)>, wanted: i32) -> Option<usize> {
    let wanted = wanted.abs();
    let mut best: Option<(usize, i32)> = None;
    for (index, score) in tools {
        let d = (score.abs() - wanted).abs();
        if best.map(|(_, bd)| d < bd).unwrap_or(true) {
            best = Some((index, d));
        }
    }
    best.map(|(index, _)| index)
}

struct Delegate<'a> {
    battle: &'a Battle,
    actor: usize,
    me: &'a Combatant,
    flags: Delegation,
    kit: &'a Kit<'a>,
}

impl Delegate<'_> {
    fn unit(&self, s: usize) -> Option<&Combatant> {
        self.battle.get(s)
    }

    /// 在场（`+0x241`）：倒下的也算，撤走的不算。
    fn placed(&self, s: usize) -> bool {
        self.unit(s).map(|c| !c.retired).unwrap_or(false)
    }

    /// `FUN_004ed010`：还站着。
    fn alive(&self, s: usize) -> bool {
        self.unit(s).map(|c| c.in_play()).unwrap_or(false)
    }

    fn band(&self, s: usize) -> u8 {
        self.unit(s).map(|c| hp_band(c.hp, c.max_hp)).unwrap_or(5)
    }

    fn side_slots(&self, side: Side) -> impl Iterator<Item = usize> + '_ {
        (0..SLOTS).filter(move |s| self.unit(*s).map(|c| c.side == side).unwrap_or(false))
    }

    fn foe_side(&self) -> Side {
        match self.me.side {
            Side::Party => Side::Enemy,
            Side::Enemy => Side::Party,
        }
    }

    /// 每个动作开头那道闸：开关开着（或中了乱 0x12，不看开关），且没中 0x13。
    fn gate(&self, allowed: bool) -> bool {
        (allowed || self.me.has_state(K_CONFUSED))
            && !self.me.has_state(K_0X13)
            && self.me.in_play()
    }

    /// `FUN_004f36d0(side, 0)`：站着的人平均缺多少精。
    fn mean_deficit(&self, side: Side) -> i32 {
        let v: Vec<i32> = self
            .side_slots(side)
            .filter(|s| self.alive(*s))
            .filter_map(|s| self.unit(s).map(|c| c.max_hp - c.hp))
            .collect();
        if v.is_empty() {
            0
        } else {
            v.iter().sum::<i32>() / v.len() as i32
        }
    }

    /// `FUN_004f3570(side, 0)` / `FUN_004f3620(side, 0)`：站着的人平均当前精 /
    /// 平均精上限。
    fn mean_hp(&self, side: Side, max: bool) -> i32 {
        let v: Vec<i32> = self
            .side_slots(side)
            .filter(|s| self.alive(*s))
            .filter_map(|s| self.unit(s).map(|c| if max { c.max_hp } else { c.hp }))
            .collect();
        if v.is_empty() {
            0
        } else {
            v.iter().sum::<i32>() / v.len() as i32
        }
    }

    fn hurt(&self, side: Side) -> usize {
        self.side_slots(side)
            .filter(|s| self.placed(*s) && self.band(*s) > 2)
            .count()
    }

    /// 一方里第一个站着的——全体招式挂的那个目标位。
    fn first_standing(&self, side: Side) -> Option<usize> {
        self.side_slots(side).find(|s| self.alive(*s))
    }

    /// `FUN_004f3dc0`：身上第一个要解的状态（k = 1..26）。
    fn ailment(&self, s: usize) -> Option<usize> {
        let c = self.unit(s)?;
        (1..0x1b).find(|k| c.has_state(*k) && !cure_exempt(*k))
    }

    /// `FUN_004f4c80`：一件道具对精的量。百分比和回满都按**用的人自己**的精上限
    /// 算——原版传进去的是施术者的战斗位，不是目标。
    fn item_hp(&self, t: &Tool) -> i32 {
        match t.hp.0 {
            0 => t.hp.1 as i32,
            1 => self.me.max_hp * t.hp.1 as i32 / 100,
            2 => self.me.max_hp - self.me.hp,
            _ => 0,
        }
    }

    /// `FUN_004f4b90`：和 [`Self::item_hp`] 一样，只是没有「回满」那一支（影响类型 2 算 0）。
    /// 道具攻击（`FUN_004f5b70`）、复活（`FUN_004f61d0`）、解状态（`FUN_004f6860`）三个挑选器
    /// 用它；只有道具治疗（`FUN_004f5970`）用 `FUN_004f4c80`。
    fn item_hp_flat(&self, t: &Tool) -> i32 {
        match t.hp.0 {
            0 => t.hp.1 as i32,
            1 => self.me.max_hp * t.hp.1 as i32 / 100,
            _ => 0,
        }
    }

    // ---- 挑选器（`FUN_004f4da0` 那一族）

    fn pick_skill(tools: &[Tool], wanted: i32, keep: impl Fn(&Tool) -> bool) -> Option<usize> {
        closest_by(
            tools
                .iter()
                .filter(|t| t.usable && keep(t))
                .map(|t| (t.index, t.hp.1 as i32)),
            wanted,
        )
    }

    /// 道具的单体 / 全体按 `+0x275` 排除：单体不要 2/4，全体不要 1/3。
    fn item_range_ok(t: &Tool, all: bool) -> bool {
        if all {
            !matches!(t.range, 1 | 3)
        } else {
            !matches!(t.range, 2 | 4)
        }
    }

    fn pick_item(
        &self,
        wanted: i32,
        all: bool,
        keep: impl Fn(&Tool) -> bool,
        score: impl Fn(&Tool) -> i32,
    ) -> Option<usize> {
        closest_by(
            self.kit
                .items
                .iter()
                .filter(|t| t.usable && Self::item_range_ok(t, all) && keep(t))
                .map(|t| (t.index, score(t))),
            wanted,
        )
    }

    // ---- 动作（`FUN_004d9740` 起那二十个）

    /// `FUN_004d9740` 普攻。
    fn attack(&self, rng: &mut CombatRng, t: usize) -> Option<Plan> {
        if t == self.actor {
            return None;
        }
        let allowed =
            self.flags.attack || self.me.has_state(K_CONFUSED) || self.me.has_state(K_0X13);
        if !allowed || !self.me.in_play() || !self.alive(t) {
            return None;
        }
        let c = self.unit(t)?;
        let ok = basic_attack_allowed(
            rng,
            self.me.side == Side::Party,
            c.has_state(K_INVISIBLE),
            c.has_state(K_EVADE) && c.has_state(K_0X14),
        );
        // `FUN_004f0210`：盘心挡着后位（见 [`Battle::attackable`]）。
        (ok && self.battle.attackable(self.actor, t)).then_some(Plan::Attack { target: t })
    }

    /// `FUN_004d9920` 仙术单体治疗：按目标缺多少精挑。
    fn magic_heal(&self, t: usize) -> Option<Plan> {
        if !self.gate(self.flags.magic) || !self.alive(t) {
            return None;
        }
        let c = self.unit(t)?;
        let i = Self::pick_skill(self.kit.magic, c.max_hp - c.hp, |x| {
            x.kind == 3 && x.range == 1
        })?;
        Some(Plan::Magic {
            target: t,
            index: i,
        })
    }

    /// `FUN_004d9a70` 仙术全体治疗：按全队平均缺多少挑。
    fn magic_heal_all(&self) -> Option<Plan> {
        if !self.gate(self.flags.magic) {
            return None;
        }
        let wanted = self.mean_deficit(self.me.side);
        let i = Self::pick_skill(self.kit.magic, wanted, |x| x.kind == 3 && x.range == 2)?;
        Some(Plan::Magic {
            target: self.actor,
            index: i,
        })
    }

    /// `FUN_004d9ba0` 仙术单体攻击：按目标还剩多少精挑。
    fn magic_attack(&self, t: usize) -> Option<Plan> {
        if !self.gate(self.flags.magic) || !self.alive(t) {
            return None;
        }
        let hp = self.unit(t)?.hp;
        if hp <= 0 {
            return None;
        }
        let i = Self::pick_skill(self.kit.magic, hp, |x| x.kind == 2 && x.range == 3)?;
        Some(Plan::Magic {
            target: t,
            index: i,
        })
    }

    /// `FUN_004d9d00` 仙术全体攻击：按对面站着的人的平均精挑。
    fn magic_attack_all(&self) -> Option<Plan> {
        if !self.gate(self.flags.magic) {
            return None;
        }
        let wanted = self.mean_hp(self.foe_side(), false);
        if wanted == 0 {
            return None;
        }
        let i = Self::pick_skill(self.kit.magic, wanted, |x| x.kind == 2 && x.range == 4)?;
        Some(Plan::Magic {
            target: self.first_standing(self.foe_side())?,
            index: i,
        })
    }

    /// `FUN_004daa40` 仙术复活：想要的量是施术者自己的精上限。
    fn magic_revive(&self, t: usize) -> Option<Plan> {
        if !self.gate(self.flags.magic) || !self.placed(t) || self.alive(t) {
            return None;
        }
        let i = Self::pick_skill(self.kit.magic, self.me.max_hp, |x| {
            x.kind != 0 && x.range == 1 && x.revives()
        })?;
        Some(Plan::Magic {
            target: t,
            index: i,
        })
    }

    /// `FUN_004dadd0` 仙术解状态：挑对精影响最小的那条（`FUN_004f6370` 拿 0 比）。
    fn magic_cure(&self, t: usize, k: usize) -> Option<Plan> {
        if !self.gate(self.flags.magic) || !self.alive(t) {
            return None;
        }
        let i = Self::pick_skill(self.kit.magic, 0, |x| x.range == 1 && x.cures(k))?;
        Some(Plan::Magic {
            target: t,
            index: i,
        })
    }

    /// `FUN_004d9f50` 特技单体（我方）。**闸用的是仙术那一位 `+0x569`**，不是特技的
    /// `+0x56a`——原版如此。
    fn special_ally(&self, t: usize) -> Option<Plan> {
        if !self.gate(self.flags.magic) || !self.alive(t) {
            return None;
        }
        let c = self.unit(t)?;
        let i = Self::pick_skill(self.kit.specials, c.max_hp - c.hp, |x| x.range == 1)?;
        Some(Plan::Special {
            target: t,
            index: i,
        })
    }

    /// `FUN_004da0a0` 特技全体（我方）。
    fn special_ally_all(&self) -> Option<Plan> {
        if !self.gate(self.flags.special) {
            return None;
        }
        let wanted = self.mean_deficit(self.me.side);
        let i = Self::pick_skill(self.kit.specials, wanted, |x| x.range == 2)?;
        Some(Plan::Special {
            target: self.actor,
            index: i,
        })
    }

    /// `FUN_004da1d0` 特技单体（敌方）：想要的量是 `rand % 目标当前精`。
    fn special_enemy(&self, rng: &mut CombatRng, t: usize) -> Option<Plan> {
        if !self.gate(self.flags.special) || !self.alive(t) {
            return None;
        }
        let hp = self.unit(t)?.hp;
        if hp <= 0 {
            return None;
        }
        let wanted = rng.rand15() % hp;
        let i = Self::pick_skill(self.kit.specials, wanted, |x| x.range == 3)?;
        Some(Plan::Special {
            target: t,
            index: i,
        })
    }

    /// `FUN_004da350` 特技全体（敌方）：`rand % 对面平均精上限`。
    fn special_enemy_all(&self, rng: &mut CombatRng) -> Option<Plan> {
        if !self.gate(self.flags.special) {
            return None;
        }
        let mean = self.mean_hp(self.foe_side(), true);
        if mean <= 0 {
            return None;
        }
        let wanted = rng.rand15() % mean;
        let i = Self::pick_skill(self.kit.specials, wanted, |x| x.range == 4)?;
        Some(Plan::Special {
            target: self.first_standing(self.foe_side())?,
            index: i,
        })
    }

    /// `FUN_004dab70` 特技复活。
    fn special_revive(&self, t: usize) -> Option<Plan> {
        if !self.gate(self.flags.special) || !self.placed(t) || self.alive(t) {
            return None;
        }
        let i = Self::pick_skill(self.kit.specials, self.me.max_hp, |x| {
            x.range == 1 && x.revives()
        })?;
        Some(Plan::Special {
            target: t,
            index: i,
        })
    }

    /// `FUN_004daf00` 特技解状态。
    fn special_cure(&self, t: usize, k: usize) -> Option<Plan> {
        if !self.gate(self.flags.special) || !self.alive(t) {
            return None;
        }
        let i = Self::pick_skill(self.kit.specials, 0, |x| x.range == 1 && x.cures(k))?;
        Some(Plan::Special {
            target: t,
            index: i,
        })
    }

    /// `FUN_004da4f0` 道具单体治疗。能复活的药分值加 100000，免得拿去补血。
    fn item_heal(&self, t: usize) -> Option<Plan> {
        if !self.gate(self.flags.item) || !self.alive(t) {
            return None;
        }
        let c = self.unit(t)?;
        let i = self.pick_item(c.max_hp - c.hp, false, Tool::heals_as_item, |x| {
            self.item_hp(x) + if x.revives() { 100_000 } else { 0 }
        })?;
        Some(Plan::Item {
            target: t,
            index: i,
        })
    }

    /// `FUN_004da640` 道具全体治疗。
    fn item_heal_all(&self) -> Option<Plan> {
        if !self.gate(self.flags.item) {
            return None;
        }
        let wanted = self.mean_deficit(self.me.side);
        let i = self.pick_item(wanted, true, Tool::heals_as_item, |x| {
            self.item_hp(x) + if x.revives() { 100_000 } else { 0 }
        })?;
        Some(Plan::Item {
            target: self.actor,
            index: i,
        })
    }

    /// `FUN_004da770` 道具单体攻击：`rand % 目标当前精`。
    fn item_attack(&self, rng: &mut CombatRng, t: usize) -> Option<Plan> {
        if !self.gate(self.flags.item) || !self.alive(t) {
            return None;
        }
        let hp = self.unit(t)?.hp;
        if hp <= 0 {
            return None;
        }
        let wanted = rng.rand15() % hp;
        let i = self.pick_item(wanted, false, Tool::harms_as_item, |x| self.item_hp_flat(x))?;
        Some(Plan::Item {
            target: t,
            index: i,
        })
    }

    /// `FUN_004da900` 道具全体攻击：对面平均当前精。
    fn item_attack_all(&self) -> Option<Plan> {
        if !self.gate(self.flags.item) {
            return None;
        }
        let wanted = self.mean_hp(self.foe_side(), false);
        let i = self.pick_item(wanted, true, Tool::harms_as_item, |x| self.item_hp_flat(x))?;
        Some(Plan::Item {
            target: self.first_standing(self.foe_side())?,
            index: i,
        })
    }

    /// `FUN_004daca0` 道具复活：挑选器 `FUN_004f61d0`，要的量是自己的精上限
    /// （`FUN_004f2bb0(自己, 1)`），分值 `FUN_004f4b90`。
    fn item_revive(&self, t: usize) -> Option<Plan> {
        if !self.gate(self.flags.item) || !self.placed(t) || self.alive(t) {
            return None;
        }
        let i = self.pick_item(self.me.max_hp, false, Tool::revives, |x| {
            self.item_hp_flat(x)
        })?;
        Some(Plan::Item {
            target: t,
            index: i,
        })
    }

    /// `FUN_004db030` 道具解状态：挑选器 `FUN_004f6860` 取 `FUN_004f4b90` 绝对值最小的
    /// （即拿 0 比）。
    fn item_cure(&self, t: usize, k: usize) -> Option<Plan> {
        if !self.gate(self.flags.item) || !self.alive(t) {
            return None;
        }
        let i = self.pick_item(0, false, |x| x.cures(k), |x| self.item_hp_flat(x))?;
        Some(Plan::Item {
            target: t,
            index: i,
        })
    }

    // ---- 两个行为体

    /// `FUN_004d8760`：谨慎时先跑的这一套，只顾自己人——救倒下的、全队掉血多了
    /// 群补、单个掉到四分之一以下单补、身上有毛病一半机会去解。
    fn support(&self, rng: &mut CombatRng) -> Option<Plan> {
        // `+0xdc0`：我方掉到四分之一以下的人数给的量 + 10，两个以上才会 < 100。
        let group = spread(self.hurt(Side::Party)) + 10;
        for s in 0..SLOTS {
            if !self.placed(s) {
                continue;
            }
            if self.me.has_state(K_CONFUSED) {
                // 乱：每个在场的位掷一次 `rand & 0x3f`，落在 0..8 才做（0x4d89d8 的跳表），
                // 目标就是这个位，哪一边都行。
                let p = match rng.rand15() & 0x3f {
                    0 => self.special_revive(s),
                    1 => self.magic_revive(s),
                    2 => self.item_revive(s),
                    3 => self.magic_heal_all(),
                    4 => self.special_ally_all(),
                    5 => self.item_heal_all(),
                    6 => self.magic_heal(s),
                    7 => self.special_ally(s),
                    8 => self.item_heal(s),
                    _ => None,
                };
                if p.is_some() {
                    return p;
                }
                continue;
            }
            let ours = self
                .unit(s)
                .map(|c| c.side == self.me.side)
                .unwrap_or(false);
            // `+0x650`：站着 100，倒下 0。
            if ours && !self.alive(s) {
                let p = self
                    .special_revive(s)
                    .or_else(|| self.magic_revive(s))
                    .or_else(|| self.item_revive(s));
                if p.is_some() {
                    return p;
                }
            }
            if !self.alive(s) {
                continue;
            }
            if group < 100 {
                let p = self
                    .magic_heal_all()
                    .or_else(|| self.special_ally_all())
                    .or_else(|| self.item_heal_all());
                if p.is_some() {
                    return p;
                }
            }
            if !ours {
                continue;
            }
            // `+0x640`：血档 1/2 是 100，4（残血）是 10，其余 40。
            let need = match self.band(s) {
                1 | 2 => 100,
                4 => 10,
                _ => 40,
            };
            if need < 0x32 {
                let p = self
                    .magic_heal(s)
                    .or_else(|| self.special_ally(s))
                    .or_else(|| self.item_heal(s));
                if p.is_some() {
                    return p;
                }
            }
            // `+0x670` / `+0x67c`：有毛病是 50，并记下第一个毛病。
            if let Some(k) = self.ailment(s) {
                if passes(rng, CHANCE_HALF) {
                    let p = self
                        .magic_cure(s, k)
                        .or_else(|| self.special_cure(s, k))
                        .or_else(|| self.item_cure(s, k));
                    if p.is_some() {
                        return p;
                    }
                }
            }
        }
        None
    }

    fn strike(&self, rng: &mut CombatRng, t: usize) -> Option<Plan> {
        if let Some(p) = self.attack(rng, t) {
            return Some(p);
        }
        if let Some(p) = self.magic_attack(t) {
            return Some(p);
        }
        if let Some(p) = self.special_enemy(rng, t) {
            return Some(p);
        }
        self.item_attack(rng, t)
    }

    /// `FUN_004d8af0`：进攻那一套。全体招式先（`+0xd60` 恒 < 100），再从 0 号位往后
    /// 找第一个站着的对手，普攻 → 仙术 → 特技 → 道具，同一个人连试两遍（原版两段
    /// 一模一样的阶梯，第二遍只在随机分值上可能不同）。
    fn offense(&self, rng: &mut CombatRng) -> Option<Plan> {
        if self.me.has_state(K_CONFUSED) {
            return self.offense_confused(rng);
        }
        let aoe = 100 - spread(self.hurt(self.foe_side()));
        for s in 0..SLOTS {
            if !self.alive(s) {
                continue;
            }
            if aoe < 100 {
                if let Some(p) = self.magic_attack_all() {
                    return Some(p);
                }
                if let Some(p) = self.special_enemy_all(rng) {
                    return Some(p);
                }
                if let Some(p) = self.item_attack_all() {
                    return Some(p);
                }
            }
            let foe = self
                .unit(s)
                .map(|c| c.side != self.me.side)
                .unwrap_or(false);
            for _ in 0..2 {
                // 自己人只有中了 0x13 时才有 2/10 的机会被当成目标。
                let go = foe || (self.me.has_state(K_0X13) && rng.rand15() % 10 > 7);
                if go {
                    if let Some(p) = self.strike(rng, s) {
                        return Some(p);
                    }
                }
            }
        }
        None
    }
}

impl Delegate<'_> {
    /// `FUN_004d8af0` 乱的那一支（0x4d8d41 起）：`rand % 11` 抽位，要「在场、`+0x370 != 0`、
    /// 不是自己」才停——而 `+0x370`（战斗位 `+0x598`）从来是 0，所以总是抽满 0x400 次，
    /// 用最后一次抽到的（可能是自己）。之后 `rand % 11`：0 仙术群攻、1 特技群攻、2 道具群攻、
    /// 3/10 普攻、4/7 仙术单攻、5/8 特技单攻、6/9 道具单攻。
    fn offense_confused(&self, rng: &mut CombatRng) -> Option<Plan> {
        let mut u = 0usize;
        for _ in 0..0x400 {
            u = (rng.rand15() % 11) as usize;
        }
        if !self.placed(u) {
            return None;
        }
        match rng.rand15() % 11 {
            0 => self.magic_attack_all(),
            1 => self.special_enemy_all(rng),
            2 => self.item_attack_all(),
            3 | 10 => self.attack(rng, u),
            4 | 7 => self.magic_attack(u),
            5 | 8 => self.special_enemy(rng, u),
            _ => self.item_attack(rng, u),
        }
    }
}

// ---------------------------------------------------------------- 怪物的阶梯

impl Delegate<'_> {
    /// `FUN_004d94f0` 按目标位存的「单体治疗」闸（`+0x640`）：血档 < 3 是 100（关），
    /// 否则 0x28、残血 10——都 < 100，所以就是「这个位掉到一半以下」。原版挂着还没演完的
    /// 同类行动时 `FUN_004d93d0` 会把闸锁成 100；移植端一次只有一个行动在出手段，怪物挑
    /// 行动时不会有别人的挂着，所以不用锁。攻击和单体仙术攻击那几道闸 `FUN_004d94f0`
    /// 不写，恒为 0（放行）。
    fn heal_gate(&self, t: usize) -> bool {
        self.band(t) >= 3
    }

    /// 同上的「解状态」闸（`+0x670`）：`FUN_004f3c70`——身上有要解的状态就开。
    fn cure_gate(&self, t: usize) -> bool {
        self.unit(t)
            .is_some_and(|c| (1..0x1b).any(|k| c.has_state(k) && !cure_exempt(k)))
    }

    /// `+0xd20`（怪物那边的全体治疗）/ `+0xe00`（全体攻击）：那一边在场、掉到一半以下的
    /// 不到两个就是 100 以上（关）。
    fn side_gate(&self, side: Side) -> bool {
        spread(self.hurt(side)) < 100
    }

    /// 起点：从 0 号位往后，每位掷 `rand % 3 == 1` 且在场的第一个；落在 10 号位时改成 0
    /// （原版就这么写）；扫空了这一回合不行动。
    fn sweep(&self, rng: &mut CombatRng) -> Option<usize> {
        (0..SLOTS)
            .find(|s| rng.rand15() % 3 == 1 && self.placed(*s))
            .map(|s| if s == 10 { 0 } else { s })
    }

    /// 狂 / 乱各有两成（`rand % 10 > 7`）让怪物对自己这边也来一下普攻；`with_confused`
    /// 为假时只看狂（后两个阶梯收尾那一处）。
    fn turns_on_own(&self, rng: &mut CombatRng, with_confused: bool) -> bool {
        if self.me.has_state(K_0X13) && rng.rand15() % 10 > 7 {
            return true;
        }
        with_confused && self.me.has_state(K_CONFUSED) && rng.rand15() % 10 > 7
    }

    /// `FUN_004db160`：重楼变身（0x2e3）神满了就放 0x1e1d（它 boss 那一招，打全体），
    /// 不看付不付得起。原版还在原点放特效组 0x22f，移植端没放。
    fn chonglou(&self) -> Option<Plan> {
        if self.me.actor_id != 0x2e3 || self.me.mp != self.me.max_mp {
            return None;
        }
        let tool = self.kit.magic.iter().find(|t| t.id == 0x1e1d)?;
        Some(Plan::Magic {
            target: self.first_standing(self.foe_side())?,
            index: tool.index,
        })
    }

    /// `FUN_004d9e20`：给同一边的 `t` 放一个辅助类（`skill_type` 1）单体（范围 1）仙术，
    /// 在付得起的里面随机挑一个（`FUN_004f5330(自己, 0)`：`rand % 个数`）。
    fn magic_support(&self, rng: &mut CombatRng, t: usize) -> Option<Plan> {
        if !self.gate(self.flags.magic) || !self.placed(t) || !self.alive(t) {
            return None;
        }
        if self.unit(t)?.hp <= 0 {
            return None;
        }
        let pool: Vec<usize> = self
            .kit
            .magic
            .iter()
            .filter(|x| x.usable && x.kind == 1 && x.range == 1)
            .map(|x| x.index)
            .collect();
        if pool.is_empty() {
            return None;
        }
        let i = pool[rng.rand15() as usize % pool.len()];
        Some(Plan::Magic {
            target: t,
            index: i,
        })
    }

    /// `FUN_004dadd0` 那一格：闸开着才掷 50%，再解身上第一个要解的（`FUN_004f3dc0`）。
    fn cure_rung(&self, rng: &mut CombatRng, t: usize) -> Option<Plan> {
        if !self.cure_gate(t) || !passes(rng, CHANCE_HALF) {
            return None;
        }
        let k = self.ailment(t)?;
        self.magic_cure(t, k)
    }

    /// `FUN_004d71e0`：血档 1、2（半血以上）。
    fn ladder_calm(&self, rng: &mut CombatRng) -> Option<Plan> {
        if let Some(p) = self.chonglou() {
            return Some(p);
        }
        let start = self.sweep(rng)?;
        for t in start..SLOTS {
            if !self.placed(t) || !self.alive(t) {
                continue;
            }
            let own = t >= PARTY_SLOTS;
            if !own || self.turns_on_own(rng, true) {
                if let Some(p) = self.attack(rng, t) {
                    return Some(p);
                }
            }
            if self.side_gate(Side::Enemy) {
                if let Some(p) = self.magic_heal_all() {
                    return Some(p);
                }
            }
            if own {
                if self.heal_gate(t) {
                    if let Some(p) = self.magic_heal(t) {
                        return Some(p);
                    }
                }
                if let Some(p) = self.cure_rung(rng, t) {
                    return Some(p);
                }
            }
            // 道具那一支（`FUN_004014f0`）恒返回 0，怪物从不用道具。
            if self.side_gate(Side::Party) && passes(rng, CHANCE_37) {
                if let Some(p) = self.magic_attack_all() {
                    return Some(p);
                }
            }
            if !own {
                if passes(rng, 0x9a) {
                    if let Some(p) = self.magic_attack_all() {
                        return Some(p);
                    }
                    if let Some(p) = self.magic_attack(t) {
                        return Some(p);
                    }
                }
            } else if passes(rng, CHANCE_30) {
                if self.heal_gate(t) {
                    if let Some(p) = self.magic_heal(t) {
                        return Some(p);
                    }
                }
                if passes(rng, CHANCE_30) {
                    if let Some(p) = self.magic_support(rng, t) {
                        return Some(p);
                    }
                }
            }
            if !own || self.turns_on_own(rng, true) {
                if let Some(p) = self.attack(rng, t) {
                    return Some(p);
                }
            }
        }
        None
    }

    /// `FUN_004d7820`：血档 3（四分之一以上）。
    fn ladder_hurt(&self, rng: &mut CombatRng) -> Option<Plan> {
        let start = self.sweep(rng)?;
        for t in start..SLOTS {
            if !self.placed(t) || !self.alive(t) {
                continue;
            }
            let own = t >= PARTY_SLOTS;
            if !own || self.turns_on_own(rng, true) {
                if let Some(p) = self.attack(rng, t) {
                    return Some(p);
                }
            }
            if let Some(p) = self.tend(rng, t, false) {
                return Some(p);
            }
            if let Some(p) = self.finish(rng, t, own) {
                return Some(p);
            }
        }
        None
    }

    /// `FUN_004d7ef0`：血档 4（残血）。开头那一下对自己这边只看狂。
    fn ladder_dying(&self, rng: &mut CombatRng) -> Option<Plan> {
        let start = self.sweep(rng)?;
        for t in start..SLOTS {
            if !self.placed(t) || !self.alive(t) {
                continue;
            }
            let own = t >= PARTY_SLOTS;
            if !own || self.turns_on_own(rng, false) {
                if let Some(p) = self.attack(rng, t) {
                    return Some(p);
                }
            }
            if let Some(p) = self.tend(rng, t, true) {
                return Some(p);
            }
            if let Some(p) = self.finish(rng, t, own) {
                return Some(p);
            }
        }
        None
    }

    /// 后两个阶梯中段：全体治疗 → 轮到自己就先治自己（残血那个治不了再全体治）→ 自己
    /// 这边的治疗、解状态 → 全体攻击（25%）。
    fn tend(&self, rng: &mut CombatRng, t: usize, dying: bool) -> Option<Plan> {
        let own = t >= PARTY_SLOTS;
        if self.side_gate(Side::Enemy) {
            if let Some(p) = self.magic_heal_all() {
                return Some(p);
            }
        }
        if t == self.actor && self.heal_gate(t) {
            if let Some(p) = self.magic_heal(t) {
                return Some(p);
            }
            if dying {
                if let Some(p) = self.magic_heal_all() {
                    return Some(p);
                }
            }
        }
        if own {
            if self.heal_gate(t) {
                if let Some(p) = self.magic_heal(t) {
                    return Some(p);
                }
            }
            if let Some(p) = self.cure_rung(rng, t) {
                return Some(p);
            }
        }
        // 道具那一支（`FUN_004014f0`）恒返回 0。
        if self.side_gate(Side::Party) && passes(rng, 0x100) {
            if let Some(p) = self.magic_attack_all() {
                return Some(p);
            }
        }
        None
    }

    /// 后两个阶梯收尾：我方目标连着几格仙术攻击 / 普攻；自己这边的只在狂掷中时普攻。
    fn finish(&self, rng: &mut CombatRng, t: usize, own: bool) -> Option<Plan> {
        if own {
            return if self.turns_on_own(rng, false) {
                self.attack(rng, t)
            } else {
                None
            };
        }
        if passes(rng, 0x2cd) {
            if let Some(p) = self.magic_attack(t) {
                return Some(p);
            }
        }
        if passes(rng, 0x2cd) {
            if let Some(p) = self.attack(rng, t) {
                return Some(p);
            }
        }
        if let Some(p) = self.magic_attack(t) {
            return Some(p);
        }
        if passes(rng, 0x100) {
            if let Some(p) = self.magic_attack(t) {
                return Some(p);
            }
        }
        if passes(rng, 0x66) {
            if let Some(p) = self.magic_attack_all() {
                return Some(p);
            }
            if let Some(p) = self.magic_attack(t) {
                return Some(p);
            }
        }
        self.attack(rng, t)
    }
}

/// 怪物挑行动（`FUN_004d7120` 不乱的那一支）：按血档走三个阶梯之一——1、2 走
/// `FUN_004d71e0`，3 走 `FUN_004d7820`，4 走 `FUN_004d7ef0`，倒下的不行动。怪物的六个
/// 开关开局全开（`FUN_004f8bb0`）。阶梯的闸（`FUN_004d94f0`）原版每帧重算，这里当场算。
/// 挑不出来返回 `None`，下一帧再试。
pub fn monster(battle: &Battle, rng: &mut CombatRng, actor: usize, kit: &Kit) -> Option<Plan> {
    let me = battle.get(actor)?;
    let d = Delegate {
        battle,
        actor,
        me,
        flags: Delegation::all_on(),
        kit,
    };
    match hp_band(me.hp, me.max_hp) {
        1 | 2 => d.ladder_calm(rng),
        3 => d.ladder_hurt(rng),
        4 => d.ladder_dying(rng),
        _ => None,
    }
}

/// 我方托管 / 乱 / 狂（`FUN_004d86d0`）：谨慎就先 [`Delegate::support`]，没挑出来再
/// [`Delegate::offense`]。中了乱两套各走自己的随机支。都没挑出来返回 `None`——原版这时
/// 什么也不做，下一帧再试（我方不会退给怪物那套，见 `combat_ai.md` §6.3）。
pub fn delegate(battle: &Battle, rng: &mut CombatRng, actor: usize, kit: &Kit) -> Option<Plan> {
    let flags = battle.get(actor)?.delegation;
    delegate_with(battle, rng, actor, kit, flags)
}

/// 同 [`delegate`]，只是开关由调用方给（快进代打用 [`Delegation::all_on`]）。
pub fn delegate_with(
    battle: &Battle,
    rng: &mut CombatRng,
    actor: usize,
    kit: &Kit,
    flags: Delegation,
) -> Option<Plan> {
    let me = battle.get(actor)?;
    if !me.in_play() {
        return None;
    }
    let d = Delegate {
        battle,
        actor,
        me,
        flags,
        kit,
    };
    if d.flags.cautious {
        if let Some(p) = d.support(rng) {
            return Some(p);
        }
    }
    d.offense(rng)
}

/// 乱（0x12）：`FUN_004d7120` 查到乱就走 `FUN_004d8580`——用 `rand % 11` 挑一个在场、
/// 不是自己的位（哪一边都行，最多试 0x200 次），再按 `rand & 7`：1 全体治疗、2 单体
/// 治疗、4 全体攻击、5 单体攻击，其余普攻。各动作的闸因为中了乱一律放行。挑不出来
/// 返回 `None`，这一回合空过。
pub fn confused(battle: &Battle, rng: &mut CombatRng, actor: usize, kit: &Kit) -> Option<Plan> {
    let me = battle.get(actor)?;
    let d = Delegate {
        battle,
        actor,
        me,
        flags: me.delegation,
        kit,
    };
    let mut pick = None;
    for _ in 0..0x200 {
        let u = (rng.rand15() % 11) as usize;
        if d.placed(u) && u != actor {
            pick = Some(u);
            break;
        }
    }
    let u = pick?;
    match rng.rand15() & 7 {
        1 => d.magic_heal_all(),
        2 => d.magic_heal(u),
        4 => d.magic_attack_all(),
        5 => d.magic_attack(u),
        _ => d.attack(rng, u),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hp_bands_split_at_full_half_and_quarter() {
        assert_eq!(hp_band(100, 100), 1, "满血");
        assert_eq!(hp_band(99, 100), 2);
        assert_eq!(hp_band(50, 100), 2, "刚好半血仍是 2");
        assert_eq!(hp_band(49, 100), 3);
        assert_eq!(hp_band(25, 100), 3, "刚好四分之一仍是 3");
        assert_eq!(hp_band(24, 100), 4);
        assert_eq!(hp_band(1, 100), 4);
        assert_eq!(hp_band(0, 100), 5, "已倒");
    }

    #[test]
    fn confusion_overrides_every_band_except_death() {
        assert_eq!(behaviour(100, 100, false), Behaviour::Healthy);
        assert_eq!(behaviour(30, 100, false), Behaviour::Hurt);
        assert_eq!(behaviour(5, 100, false), Behaviour::Critical);

        assert_eq!(behaviour(100, 100, true), Behaviour::Confused);
        assert_eq!(behaviour(5, 100, true), Behaviour::Confused);
        assert_eq!(behaviour(0, 100, true), Behaviour::Down, "倒下不受乱影响");
    }

    #[test]
    fn invisibility_stops_a_plain_attack_from_either_side() {
        let mut rng = CombatRng::new(3);
        assert!(!basic_attack_allowed(&mut rng, true, true, false));
        assert!(!basic_attack_allowed(&mut rng, false, true, false));
    }

    #[test]
    fn a_monster_will_swing_at_a_dodging_target_but_a_companion_will_not() {
        let mut rng = CombatRng::new(11);
        assert!(
            !basic_attack_allowed(&mut rng, true, false, true),
            "我方托管会放弃"
        );

        let mut swings = 0;
        for _ in 0..1000 {
            if basic_attack_allowed(&mut rng, false, false, true) {
                swings += 1;
            }
        }
        // 0x133 / 1024 ≈ 30%.
        assert!((250..=350).contains(&swings), "怪物挥空率不对: {swings}");
    }

    /// 0 号位一个我方，5 号位一只怪。
    fn den(monster_hp: i32) -> Battle {
        let mut b = Battle::new(3);
        b.place(0, unit(Side::Party, 100, 100));
        b.place(5, unit(Side::Enemy, monster_hp, 100));
        b
    }

    #[test]
    fn a_calm_monster_with_no_spells_only_swings_at_the_party() {
        let b = den(100);
        let kit = Kit {
            magic: &[],
            specials: &[],
            items: &[],
        };
        let mut rng = CombatRng::new(9);
        let mut swung = 0;
        for _ in 0..200 {
            match monster(&b, &mut rng, 5, &kit) {
                Some(Plan::Attack { target: 0 }) => swung += 1,
                None => {}
                other => panic!("{other:?}"),
            }
        }
        assert!(swung > 0);
    }

    #[test]
    fn a_hurt_monster_patches_itself_up() {
        let b = den(40);
        let magic = [tool(0, 3, 1, 50)];
        let kit = Kit {
            magic: &magic,
            specials: &[],
            items: &[],
        };
        let mut rng = CombatRng::new(9);
        let mut healed = 0;
        for _ in 0..200 {
            match monster(&b, &mut rng, 5, &kit) {
                Some(Plan::Magic {
                    target: 5,
                    index: 0,
                }) => healed += 1,
                Some(Plan::Attack { target: 0 }) | None => {}
                other => panic!("{other:?}"),
            }
        }
        assert!(healed > 0, "轮到自己、掉到一半以下就先治自己");
    }

    #[test]
    fn chonglou_unleashes_its_boss_skill_on_a_full_tank_of_shen() {
        let mut b = den(100);
        {
            let m = b.get_mut(5).unwrap();
            m.actor_id = 0x2e3;
            m.max_mp = 500;
            m.mp = 500;
        }
        let magic = [
            tool(0, 2, 3, -10),
            Tool {
                id: 0x1e1d,
                usable: false,
                ..tool(1, 2, 4, -300)
            },
        ];
        let kit = Kit {
            magic: &magic,
            specials: &[],
            items: &[],
        };
        let boss = Some(Plan::Magic {
            target: 0,
            index: 1,
        });
        let mut rng = CombatRng::new(1);
        assert_eq!(monster(&b, &mut rng, 5, &kit), boss, "神满了不看付不付得起");
        b.get_mut(5).unwrap().mp = 499;
        for _ in 0..50 {
            assert_ne!(monster(&b, &mut rng, 5, &kit), boss);
        }
    }

    #[test]
    fn a_downed_monster_does_nothing() {
        let b = den(0);
        let kit = Kit {
            magic: &[],
            specials: &[],
            items: &[],
        };
        let mut rng = CombatRng::new(1);
        assert_eq!(monster(&b, &mut rng, 5, &kit), None);
    }

    fn unit(side: Side, hp: i32, max_hp: i32) -> Combatant {
        let mut c = Combatant::new(1, side);
        c.hp = hp;
        c.max_hp = max_hp;
        c.alive = hp > 0;
        c
    }

    fn tool(index: usize, kind: u8, range: u8, hp: i16) -> Tool {
        Tool {
            index,
            id: 0,
            kind,
            range,
            hp: (0, hp),
            states: [0; 31],
            usable: true,
        }
    }

    /// 0 号位是托管的人，1 号位一个队友，5 号位一只怪。
    fn field(flags: Delegation, ally_hp: i32) -> Battle {
        let mut b = Battle::new(1);
        let mut me = unit(Side::Party, 100, 100);
        me.delegation = flags;
        b.place(0, me);
        b.place(1, unit(Side::Party, ally_hp, 100));
        b.place(5, unit(Side::Enemy, 50, 50));
        b.place(6, unit(Side::Enemy, 50, 50));
        b
    }

    const NOTHING: Kit<'static> = Kit {
        magic: &[],
        specials: &[],
        items: &[],
    };

    #[test]
    fn the_panel_switches_follow_the_originals_rules() {
        let mut d = Delegation::default();
        assert!(d.cautious && !d.on, "新档只开着谨慎");
        assert!(!d.toggle(Switch::Magic), "AI 没开不许勾仙术");
        assert!(!d.toggle(Switch::Attack), "AI 没开也不许勾攻击");
        assert!(!d.toggle(Switch::AllOut));
        assert!(!d.magic && !d.attack && d.cautious, "被拒的不改");

        assert!(d.toggle(Switch::Ai));
        assert!(d.on && d.attack, "开 AI 顺带勾上攻击");
        assert!(d.toggle(Switch::Magic) && d.toggle(Switch::Item) && d.toggle(Switch::AllOut));
        assert!(d.magic && d.item && !d.cautious);

        assert!(d.toggle(Switch::Attack));
        assert_eq!(
            d,
            Delegation {
                on: false,
                attack: false,
                magic: false,
                special: false,
                item: false,
                cautious: false,
            },
            "取消攻击连 AI 一起关，全力那一格不动"
        );

        d.toggle(Switch::Ai);
        d.toggle(Switch::Special);
        assert!(d.toggle(Switch::Ai));
        assert!(!d.on && !d.attack && !d.special, "关 AI 清掉四样");
    }

    #[test]
    fn with_only_attack_ticked_it_swings_at_the_first_foe() {
        let flags = Delegation {
            on: true,
            attack: true,
            cautious: false,
            ..Delegation::default()
        };
        let b = field(flags, 100);
        let mut rng = CombatRng::new(7);
        assert_eq!(
            delegate(&b, &mut rng, 0, &NOTHING),
            Some(Plan::Attack { target: 5 })
        );
    }

    #[test]
    fn nothing_ticked_leaves_the_choice_to_the_monster_ladder() {
        let flags = Delegation {
            on: true,
            ..Delegation::default()
        };
        let b = field(flags, 100);
        let mut rng = CombatRng::new(7);
        assert_eq!(delegate(&b, &mut rng, 0, &NOTHING), None);
    }

    #[test]
    fn an_area_spell_goes_before_the_plain_swing() {
        let magic = [tool(0, 2, 3, -30), tool(1, 2, 4, -40)];
        let kit = Kit {
            magic: &magic,
            specials: &[],
            items: &[],
        };
        let mut flags = Delegation {
            on: true,
            attack: true,
            magic: true,
            cautious: false,
            ..Delegation::default()
        };
        let mut rng = CombatRng::new(7);
        assert_eq!(
            delegate(&field(flags, 100), &mut rng, 0, &kit),
            Some(Plan::Magic {
                target: 5,
                index: 1
            })
        );
        flags.magic = false;
        assert_eq!(
            delegate(&field(flags, 100), &mut rng, 0, &kit),
            Some(Plan::Attack { target: 5 }),
            "不勾仙术就不念"
        );
    }

    #[test]
    fn a_cautious_companion_picks_up_the_fallen_first() {
        let mut revive = tool(0, 1, 1, 0);
        revive.states[24] = 2;
        let items = [revive];
        let kit = Kit {
            magic: &[],
            specials: &[],
            items: &items,
        };
        let cautious = Delegation {
            on: true,
            attack: true,
            item: true,
            ..Delegation::default()
        };
        let mut rng = CombatRng::new(7);
        assert_eq!(
            delegate(&field(cautious, 0), &mut rng, 0, &kit),
            Some(Plan::Item {
                target: 1,
                index: 0
            })
        );
        let all_out = Delegation {
            cautious: false,
            ..cautious
        };
        assert_eq!(
            delegate(&field(all_out, 0), &mut rng, 0, &kit),
            Some(Plan::Attack { target: 5 }),
            "全力只管打"
        );
    }

    #[test]
    fn a_badly_hurt_companion_gets_the_right_sized_heal() {
        let magic = [tool(0, 3, 1, 30), tool(1, 3, 1, 90), tool(2, 3, 1, 300)];
        let kit = Kit {
            magic: &magic,
            specials: &[],
            items: &[],
        };
        let flags = Delegation {
            on: true,
            attack: true,
            magic: true,
            ..Delegation::default()
        };
        let mut rng = CombatRng::new(7);
        // 缺 80：90 那条最接近。
        assert_eq!(
            delegate(&field(flags, 20), &mut rng, 0, &kit),
            Some(Plan::Magic {
                target: 1,
                index: 1
            })
        );
        // 只掉到半血以上，不补，去打。
        assert_eq!(
            delegate(&field(flags, 60), &mut rng, 0, &kit),
            Some(Plan::Attack { target: 5 })
        );
    }

    #[test]
    fn a_confused_unit_may_swing_at_its_own_side() {
        let mut b = Battle::new(1);
        let mut me = unit(Side::Party, 100, 100);
        me.states[K_CONFUSED].active = true;
        b.place(0, me);
        b.place(1, unit(Side::Party, 100, 100));
        let mut hit_ally = false;
        for seed in 0..200 {
            let mut rng = CombatRng::new(seed);
            if let Some(Plan::Attack { target }) = confused(&b, &mut rng, 0, &NOTHING) {
                assert_ne!(target, 0, "不会打自己");
                hit_ally |= target == 1;
            }
        }
        assert!(hit_ally, "乱了会打自己人");
    }

    #[test]
    fn a_confused_companion_can_turn_on_its_own_side() {
        let flags = Delegation {
            on: true,
            attack: true,
            cautious: false,
            ..Delegation::default()
        };
        let mut b = field(flags, 100);
        b.get_mut(0).unwrap().states[K_CONFUSED].active = true;
        let mut ally = 0;
        for seed in 0..400 {
            let mut rng = CombatRng::new(seed);
            if let Some(Plan::Attack { target }) = delegate(&b, &mut rng, 0, &NOTHING) {
                assert_ne!(target, 0, "打不了自己（普攻不认自己）");
                ally += (target == 1) as i32;
            }
        }
        assert!(ally > 0, "乱了的队友会打自己人");
    }
}
