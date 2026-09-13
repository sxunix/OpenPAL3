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
//!   ([`closest_skill`]). A healer therefore avoids overhealing and an
//!   attacker prefers the spell that just about finishes the job.
//! * **Targets are picked by a coin toss down the slot list**
//!   ([`pick_target`]) — the first slot that passes `rand % 3 == 1`. Not
//!   the nearest, not the weakest.
//!
//! The AI draws from its own generator seeded separately from the combat
//! one (`DAT_005dfdec` against `DAT_005dfde8`), so its choices do not
//! disturb the damage rolls.

use super::formulas::CombatRng;
use serde::{Deserialize, Serialize};

use super::state::{Battle, Combatant, K_CONFUSED, K_EVADE, K_INVISIBLE, SLOTS, Side};

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

/// One skill the AI is weighing up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillOption {
    pub id: u32,
    /// `FUN_004f4aa0`'s score: the skill's `attribute_impact_value[0]`,
    /// i.e. what it does to 精. Zero means the combatant cannot actually
    /// use it, and the original then skips it.
    pub hp_impact: i32,
}

/// Pick the skill whose effect is *closest in magnitude* to what is
/// wanted — `argmin | |impact| − |wanted| |`.
///
/// This is the heart of the AI and the reason it feels sensible: asked to
/// heal 40 points it reaches for the 50-point spell rather than the
/// 300-point one. Ties go to the earlier entry, matching the original's
/// strict `<` when it scans.
pub fn closest_skill(options: &[SkillOption], wanted: i32) -> Option<SkillOption> {
    let wanted = wanted.abs();
    let mut best: Option<(SkillOption, i32)> = None;
    for option in options {
        if option.hp_impact == 0 {
            continue;
        }
        let distance = (option.hp_impact.abs() - wanted).abs();
        if best.map(|(_, d)| distance < d).unwrap_or(true) {
            best = Some((*option, distance));
        }
    }
    best.map(|(option, _)| option)
}

/// Walk the slots and take the first that passes `rand % 3 == 1`.
///
/// `candidates` is the slot list in the order the original walks it
/// (0..=10). Returns `None` when the sweep falls off the end, which the
/// original also allows — the caller then does nothing this turn.
pub fn pick_target(rng: &mut CombatRng, candidates: &[usize]) -> Option<usize> {
    candidates.iter().copied().find(|_| rng.rand15() % 3 == 1)
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

/// How many times one action may be taken in a battle before the AI stops
/// offering it (`< 100` in every branch).
pub const ACTION_USE_CAP: u32 = 100;

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

/// What the AI decided to do this turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Too badly hurt to act.
    Pass,
    Attack {
        target: usize,
    },
    Cast {
        target: usize,
        skill: u32,
    },
}

/// A first cut at the action ladder.
///
/// The original walks a long priority list — 普攻, then party-wide heal,
/// single heal, cure, party-wide attack, item, and so on — each gated on
/// a per-action usage counter and often on one of the [`CHANCE_HALF`]
/// style probabilities. That ladder's ordering and its counters are not
/// fully read yet (see the notes' 未解决), so this reproduces only the
/// parts that are: the behaviour band, the target sweep, and the
/// closest-match skill choice against how much HP the target has left.
///
/// One consequence to keep in mind while reading a battle log: with a
/// single rung, a sweep that falls off the end simply passes the turn.
/// Against two enemies that is `(2/3)² ≈ 44%` of turns spent doing
/// nothing. The original does not idle that much because each further
/// rung sweeps again for its own action — so a high pass rate here is a
/// symptom of the missing ladder, not of the sweep.
pub fn choose(
    battle: &Battle,
    rng: &mut CombatRng,
    actor: usize,
    skills: &[SkillOption],
    confused: bool,
) -> Decision {
    let Some(me) = battle.get(actor) else {
        return Decision::Pass;
    };
    if behaviour(me.hp, me.max_hp, confused) == Behaviour::Down {
        return Decision::Pass;
    }

    let side = me.side;
    let candidates: Vec<usize> = (0..SLOTS)
        .filter(|s| {
            battle
                .get(*s)
                .map(|c| c.side != side && c.in_play())
                .unwrap_or(false)
        })
        .collect();
    if candidates.is_empty() {
        return Decision::Pass;
    }

    // The sweep can fall off the end; the original then does nothing.
    let Some(target) = pick_target(rng, &candidates) else {
        return Decision::Pass;
    };

    // Aim a damaging skill at what the target still has standing.
    let remaining = battle.get(target).map(|c| c.hp).unwrap_or(0);
    match closest_skill(skills, remaining) {
        Some(skill) => Decision::Cast {
            target,
            skill: skill.id,
        },
        None => Decision::Attack { target },
    }
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
        basic_attack_allowed(
            rng,
            self.me.side == Side::Party,
            c.has_state(K_INVISIBLE),
            c.has_state(K_EVADE) && c.has_state(K_0X14),
        )
        .then_some(Plan::Attack { target: t })
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
        let i = self.pick_item(wanted, false, Tool::harms_as_item, |x| self.item_hp(x))?;
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
        let i = self.pick_item(wanted, true, Tool::harms_as_item, |x| self.item_hp(x))?;
        Some(Plan::Item {
            target: self.first_standing(self.foe_side())?,
            index: i,
        })
    }

    /// `FUN_004daca0` 道具复活。分值函数 `FUN_004f61d0` 里没细看，按 `FUN_004f4c80`
    /// 算 `[推断]`。
    fn item_revive(&self, t: usize) -> Option<Plan> {
        if !self.gate(self.flags.item) || !self.placed(t) || self.alive(t) {
            return None;
        }
        let i = self.pick_item(self.me.max_hp, false, Tool::revives, |x| self.item_hp(x))?;
        Some(Plan::Item {
            target: t,
            index: i,
        })
    }

    /// `FUN_004db030` 道具解状态。原版分值用 `FUN_004f4b90`，这里按 `FUN_004f4c80`
    /// 算 `[推断]`，拿 0 比。
    fn item_cure(&self, t: usize, k: usize) -> Option<Plan> {
        if !self.gate(self.flags.item) || !self.alive(t) {
            return None;
        }
        let i = self.pick_item(0, false, |x| x.cures(k), |x| self.item_hp(x))?;
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

/// 我方托管 / 乱 / 狂（`FUN_004d86d0`）：谨慎就先 [`Delegate::support`]，没挑出来再
/// [`Delegate::offense`]。中了乱两套各走自己的随机支。都没挑出来返回 `None`——原版这时
/// 什么也不做，下一帧再试（我方不会退给怪物那套，见 `combat_ai.md` §6.3）。
pub fn delegate(battle: &Battle, rng: &mut CombatRng, actor: usize, kit: &Kit) -> Option<Plan> {
    let me = battle.get(actor)?;
    if !me.in_play() {
        return None;
    }
    let d = Delegate {
        battle,
        actor,
        me,
        flags: me.delegation,
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
    fn a_healer_reaches_for_the_right_sized_spell() {
        let options = [
            SkillOption {
                id: 1,
                hp_impact: 20,
            },
            SkillOption {
                id: 2,
                hp_impact: 50,
            },
            SkillOption {
                id: 3,
                hp_impact: 300,
            },
        ];
        // Missing 40 points: the 50 heals, not the 300.
        assert_eq!(closest_skill(&options, 40).unwrap().id, 2);
        // Missing 8: the small one.
        assert_eq!(closest_skill(&options, 8).unwrap().id, 1);
        // Missing 400: nothing is big enough, so the biggest wins.
        assert_eq!(closest_skill(&options, 400).unwrap().id, 3);
    }

    #[test]
    fn magnitude_is_what_matters_so_damage_sorts_the_same_way() {
        let options = [
            SkillOption {
                id: 1,
                hp_impact: -20,
            },
            SkillOption {
                id: 2,
                hp_impact: -60,
            },
        ];
        // The target has 55 HP left: the 60-damage spell is the closer fit.
        assert_eq!(closest_skill(&options, -55).unwrap().id, 2);
    }

    #[test]
    fn an_unusable_skill_is_skipped() {
        let options = [
            SkillOption {
                id: 1,
                hp_impact: 0,
            },
            SkillOption {
                id: 2,
                hp_impact: 999,
            },
        ];
        assert_eq!(closest_skill(&options, 10).unwrap().id, 2);
        assert_eq!(
            closest_skill(
                &[SkillOption {
                    id: 1,
                    hp_impact: 0
                }],
                10
            ),
            None
        );
    }

    #[test]
    fn ties_go_to_the_earlier_entry() {
        let options = [
            SkillOption {
                id: 1,
                hp_impact: 30,
            },
            SkillOption {
                id: 2,
                hp_impact: 50,
            },
        ];
        // Wanted 40 sits exactly between them.
        assert_eq!(closest_skill(&options, 40).unwrap().id, 1);
    }

    #[test]
    fn targeting_is_a_sweep_of_coin_tosses() {
        let mut rng = CombatRng::new(7);
        let candidates = [0usize, 1, 2, 3, 4];
        // Over many sweeps every slot should come up, and sometimes none.
        let mut seen = [0usize; 5];
        let mut misses = 0;
        for _ in 0..2000 {
            match pick_target(&mut rng, &candidates) {
                Some(slot) => seen[slot] += 1,
                None => misses += 1,
            }
        }
        assert!(
            seen.iter().all(|c| *c > 0),
            "每个位置都该被选到过: {seen:?}"
        );
        assert!(misses > 0, "一次都没落空反而可疑");
        // Earlier slots are reached first, so they win more often.
        assert!(seen[0] > seen[4], "{seen:?}");
    }

    #[test]
    fn a_combatant_with_no_usable_skill_falls_back_to_swinging() {
        use crate::openpal3::combat::state::{Combatant, Side};

        let mut battle = Battle::new(5);
        let mut me = Combatant::new(0, Side::Party);
        me.hp = 100;
        me.max_hp = 100;
        battle.place(0, me);
        let mut foe = Combatant::new(100, Side::Enemy);
        foe.hp = 80;
        foe.max_hp = 80;
        battle.place(5, foe);

        let mut rng = CombatRng::new(1);
        // Repeat until the target sweep lands, then check the shape.
        let decision = (0..100)
            .map(|_| choose(&battle, &mut rng, 0, &[], false))
            .find(|d| *d != Decision::Pass)
            .expect("总该选中一次");
        assert_eq!(decision, Decision::Attack { target: 5 });
    }

    #[test]
    fn a_downed_combatant_passes() {
        use crate::openpal3::combat::state::{Combatant, Side};

        let mut battle = Battle::new(5);
        let mut me = Combatant::new(0, Side::Party);
        me.hp = 0;
        me.max_hp = 100;
        battle.place(0, me);
        let mut foe = Combatant::new(100, Side::Enemy);
        foe.hp = 80;
        foe.max_hp = 80;
        battle.place(5, foe);

        let mut rng = CombatRng::new(1);
        assert_eq!(choose(&battle, &mut rng, 0, &[], false), Decision::Pass);
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
