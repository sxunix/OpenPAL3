//! The battle's state and its clock — the part of the combat system that
//! can run with no scene, no models and no UI.
//!
//! Shapes follow the original: eleven fixed slots (five party, six
//! enemies), a two-phase action gauge per slot, and per-state onset and
//! duration counters. See `switch/re/notes/combat_flow.md`.
//!
//! Three findings shape this file and are worth knowing before reading
//! it:
//!
//! * The gauge rate is `(速_eff + 4) * 5`, stored in a **u8**
//!   (`FUN_004db640`/`FUN_004db660`) — see [`Combatant::gauge_rate`].
//! * **Actions have no individual cost.** `FUN_004db730` and
//!   `FUN_004db780` have no callers at all; both gauge phases advance at
//!   the same rate whatever you picked. What differs is how being hit
//!   interrupts you — see [`Battle::interrupt`].
//! * **Escape is a roll on 运 and level, not on `escape_rate`.** The
//!   battle menu (`FUN_00500d50`) rolls [`formulas::escape_percent`];
//!   the encounter's 禁止逃跑 flag (descriptor `+0x50`) forces a failure.
//!   The database's per-monster `escape_rate` has no reader anywhere in
//!   the combat code.
//! * **Running out of rounds is a win, not a loss.** `FUN_004f7910` sends
//!   `round >= max_round` down the same settlement path as "every enemy
//!   is down", unless the script said `CombatMustFail`, in which case the
//!   cap stops ending the battle at all. A successful escape ends the same
//!   way (phase 4 → settlement → result 2).

use super::formulas::{self, CombatRng};

/// Five party slots then six enemy slots (`combat.ini [RolePos]` Role0..
/// Role10).
pub const PARTY_SLOTS: usize = 5;
pub const ENEMY_SLOTS: usize = 6;
pub const SLOTS: usize = PARTY_SLOTS + ENEMY_SLOTS;

/// The combat state flags, indexed as the original does: the flag byte
/// lives at `actor + 0x475 + k` where `k = enum + 1`. Only k = 1..=23
/// take part in the per-round tick.
pub const STATE_SLOTS: usize = 32;
pub const TICKING_STATES: std::ops::RangeInclusive<usize> = 1..=23;

/// The states this code names. `k` values, i.e. already `enum + 1`.
pub const K_POISON_WIND: usize = 1;
pub const K_POISON_WATER: usize = 3;
pub const K_POISON_FIRE: usize = 4;
pub const K_POISON_EARTH: usize = 5;
pub const K_ATTACK_UP: usize = 6;
pub const K_ATTACK_DOWN: usize = 7;
pub const K_DEFENSE_UP: usize = 8;
pub const K_DEFENSE_DOWN: usize = 9;
pub const K_LUCK_UP: usize = 10;
pub const K_LUCK_DOWN: usize = 11;
pub const K_SPEED_UP: usize = 12;
pub const K_SPEED_DOWN: usize = 13;
/// 定（麻痹）：和眠一样，这回合跳过（主调度 0x513938）。
pub const K_PARALYSIS: usize = 14;
/// 封（封咒）：控制盘「術」不可按。
pub const K_SEAL: usize = 15;
/// 禁（禁忌）：控制盘「物」「逃」不可按。
pub const K_FORBID: usize = 16;
pub const K_SLEEP: usize = 17;
pub const K_CONFUSED: usize = 18;
/// 狂：回合交给托管那一支（0x513975），普攻不看开关。
pub const K_BERSERK: usize = 19;
pub const K_EVADE: usize = 21;
/// 濒死（`FUN_004f3e20` 的 `case 0x18`）：不是标记，而是**当场把精压到上限的
/// 十分之一**（`+0x24c = +0x250 / 10`，当前更低就不动）。压完之后每帧钳位
/// `FUN_004f7880` 会据此置濒死位。
pub const K_DYING: usize = 24;
/// 即死（`case 0x19`）：精 > 0 就直接清零。
pub const K_INSTANT_DEATH: usize = 25;
pub const K_INVISIBLE: usize = 23;
/// 避毒（`+0x48f`）：装备可赋予，挂着就加不上五毒。
pub const K_POISON_IMMUNE: usize = 26;

/// A permanent duration (`0x7fff`).
pub const FOREVER: i16 = 0x7fff;

/// Onset delay and duration per poison, straight out of `FUN_004f3e20`
/// (0x4f3fdc onwards). Indexed by `k - 1`, i.e. 风雷水火土.
pub const POISON_TIMERS: [(i16, i16); 5] = [
    (3, 5),       // 风毒
    (3, 2),       // 雷毒
    (0, 5),       // 水毒
    (0, FOREVER), // 火毒
    (0, FOREVER), // 土毒
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Party,
    Enemy,
}

/// One state's two counters, as the original keeps them: an onset delay
/// at `0x490 + k*2` that must run down first, then a remaining-rounds
/// count at `0x4c6 + k*2`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StateTimer {
    pub active: bool,
    pub onset: i16,
    pub remaining: i16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Filling the gauge at the combatant's own speed.
    Charging,
    /// Gauge full, waiting to be scheduled.
    Ready,
    /// Executing a chosen action, filling at cast speed.
    Executing,
}

/// [`Battle::update`] 这一帧算出来的事件。原版没有这个类型，它把两件事
/// 分别用 `FUN_004db6a0`（蓄力条满，可以排行动）和 `FUN_004db6c0`
/// （出手条满，行动生效）去问。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    /// 蓄力条满了，该选行动。
    Ready(usize),
    /// 出手条满了，行动生效。
    Executed(usize),
}

#[derive(Debug, Clone)]
pub struct Combatant {
    pub actor_id: u32,
    pub side: Side,
    pub level: i32,

    pub hp: i32,
    pub max_hp: i32,
    pub sp: i32,
    pub max_sp: i32,
    pub mp: i32,
    pub max_mp: i32,

    pub attack: i32,
    pub defense: i32,
    pub speed: i32,
    pub luck: i32,

    /// 五灵, indexed 1..=5 like the original's `+0x458` array.
    pub elements: [bool; 6],

    /// `+0x242` in the original: cleared on death.
    pub alive: bool,
    /// `+0x228 == 0x2e2`: left the field instead of dying.
    pub retired: bool,

    pub states: [StateTimer; STATE_SLOTS],
    /// 土毒's escalating damage carries between rounds.
    pub earth_poison_acc: f32,

    /// 防御中：受到的伤害减半（原版战斗位的 `+0x474`）。
    ///
    /// 置位在防御任务 `FUN_0050eda0` 的动画收尾处（`FUN_004f3a30(slot, 1)`，
    /// 0x50ee64），清零在 0x5158bf 的 `FUN_004f3a30(slot, 0)`——条件是该单位
    /// 的行动条又满了或战斗阶段不再是 2。**所以减半一直持续到防御者下一次
    /// 轮到自己。**扣血时的用法见 `FUN_004f2c70`：`delta < -1` 才折半。
    pub defending: bool,
    /// 五灵阵位（原版战斗位的 `+0x470`），1 水 2 火 3 风 4 雷 5 土；0 = 没设。
    ///
    /// 我方的值由 `FUN_004ed250`（把冒险侧角色表整块拷进战斗位那一步）从冒险
    /// 记录的 `+0x29a` 搬过来，也就是阵法页存的那个位；敌方是 `槽号 − 4`，
    /// 只是序号。合击技的位置要求（`ComboSkillInfo::element_position_requirements`）
    /// 拿它比对——见 `combat_flow.md` 第 9.7 节。
    pub element_position: u8,
    /// 手上武器的类型（原版战斗位 `+0x288`，gdb `WeaponType`），合击技按它比对；0 = 没拿。
    pub weapon_type: u8,
    /// 飞龙探云手能偷的钱（原版战斗位 `+0x448`，gdb `stealable_money_amount`），偷一次少一次。
    pub steal_money: i32,
    /// 能偷的物品和剩余件数（`+0x44c` / `+0x450`，gdb `stealable_item_id` / `_count`）。
    pub steal_item: Option<(u32, i32)>,
    /// 蓄力条（原版 `+0x20`）。
    pub gauge: f32,
    /// 出手条（原版 `+0x24`）。原版是两条独立的条，不是一条复用。
    pub cast_gauge: f32,
    /// 出手速度（原版 `+0x2d`）。开局由 `FUN_004db660` 设成和蓄力速度一样，
    /// 真正出手时被行动任务改写，见 [`Battle::begin_execute`]。
    pub cast_rate: u8,
    pub phase: Phase,
    /// 我方托管的六个开关（战斗位 `+0x568..+0x56d`），见 [`super::ai::Delegation`]。
    pub delegation: super::ai::Delegation,
    /// 「情」（战斗位 `+0x274`，开战时 `FUN_004ed250` 从冒险侧的好感表拷进来）。援护替人
    /// 挡下时被护的人 +1（`FUN_004f34e0`，夹在 0..999）。
    pub affection: i32,
    /// 援护对象（战斗位 `+0x27c`，只有 0 号位景天的有意义）：护着哪个战斗位。开战清零。
    pub protecting: Option<usize>,
}

impl Combatant {
    pub fn new(actor_id: u32, side: Side) -> Self {
        Self {
            actor_id,
            side,
            level: 1,
            hp: 1,
            max_hp: 1,
            sp: 0,
            max_sp: 0,
            mp: 0,
            max_mp: 0,
            attack: 0,
            defense: 0,
            speed: 1,
            luck: 0,
            elements: [false; 6],
            alive: true,
            retired: false,
            states: [StateTimer::default(); STATE_SLOTS],
            earth_poison_acc: 0.,
            defending: false,
            element_position: 0,
            weapon_type: 0,
            steal_money: 0,
            steal_item: None,
            gauge: 0.,
            cast_gauge: 0.,
            cast_rate: 0,
            phase: Phase::Charging,
            delegation: Default::default(),
            affection: 0,
            protecting: None,
        }
    }

    pub fn has_state(&self, k: usize) -> bool {
        self.states.get(k).map(|s| s.active).unwrap_or(false)
    }

    /// Whether this combatant can still act or be targeted.
    pub fn in_play(&self) -> bool {
        self.alive && !self.retired
    }

    pub fn attack_eff(&self) -> i32 {
        formulas::buffed(
            self.attack,
            self.has_state(K_ATTACK_UP),
            self.has_state(K_ATTACK_DOWN),
        )
    }

    pub fn defense_eff(&self) -> i32 {
        formulas::buffed(
            self.defense,
            self.has_state(K_DEFENSE_UP),
            self.has_state(K_DEFENSE_DOWN),
        )
    }

    pub fn luck_eff(&self) -> i32 {
        formulas::buffed(
            self.luck,
            self.has_state(K_LUCK_UP),
            self.has_state(K_LUCK_DOWN),
        )
    }

    pub fn speed_eff(&self) -> i32 {
        formulas::buffed(
            self.speed,
            self.has_state(K_SPEED_UP),
            self.has_state(K_SPEED_DOWN),
        )
    }

    /// The rate the action gauge fills at: `(速_eff + 4) * 5`
    /// (`FUN_004db640`), which both gauge phases share — the original
    /// writes the same value to its speed and cast-speed bytes.
    ///
    /// It is stored as a **u8**, so the wrap is part of the behaviour:
    /// 速_eff 47 gives the maximum 255, and 48 wraps around to 4. A
    /// character at 速 3 therefore fills one segment in
    /// `11500 / (35 * 50)` ≈ 6.6 seconds.
    pub fn gauge_rate(&self) -> u8 {
        (((self.speed_eff() + 4) * 5) & 0xff) as u8
    }

    fn gauge_speed(&self) -> f32 {
        self.gauge_rate() as f32
    }

    /// 出手速度，普通行动那一支：`min(蓄力速度 × 倍率 × 2, 255)`。
    ///
    /// 出自行动任务 `FUN_00514870` 的 0x514957：
    ///
    /// ```text
    /// eax = FUN_004db640(k, 1);   // (速_eff + 4) * 5
    /// imul eax, ebx;              // × 倍率
    /// shl  eax, 1;                // × 2
    /// if (eax > 0xff) eax = 0xff;
    /// FUN_004db780(k, eax);       // -> +0x2d
    /// ```
    ///
    /// 倍率来自 `FUN_004f76c0`（技能表里的一个字节 +1），找不到就是 0，
    /// 任务里当 1 用。所以**光是普通攻击，出手段也比蓄力段快一倍**。
    pub fn cast_rate_for(&self, multiplier: i32) -> u8 {
        let m = multiplier.max(1);
        (self.gauge_rate() as i32 * m * 2).clamp(0, 255) as u8
    }

    /// 逃跑任务自己的出手速度：`速_eff × 8 + 20`（0x515295 的
    /// `shl al,3; add al,0x14`，按字节截断）。
    pub fn escape_cast_rate(&self) -> u8 {
        ((self.speed_eff() * 8 + 20) & 0xff) as u8
    }

    /// Apply an HP delta the way `FUN_004f2c70` does: clamp into
    /// `0..=max_hp`, wake a sleeper who is hit, and record death.
    /// Returns how much HP actually moved.
    pub fn change_hp(&mut self, delta: i32) -> i32 {
        if !self.alive {
            self.hp = 0;
            return 0;
        }
        if self.has_state(K_INVISIBLE) {
            return 0;
        }

        // 防御减半：原版 `delta < -1` 才折半（刚好 −1 的不折）。
        let delta = if delta < -1 && self.defending {
            delta / 2
        } else {
            delta
        };

        let before = self.hp;
        let mut hp = before + delta;
        if hp > self.max_hp {
            hp = self.max_hp;
        } else if hp < 0 {
            hp = 0;
        }
        self.hp = hp;

        // Being hit wakes you up.
        if delta < 0 && self.has_state(K_SLEEP) {
            self.clear_state(K_SLEEP);
        }

        if self.hp < 1 {
            self.hp = 0;
            if self.retired {
                // Already off the field; nothing further happens.
            } else {
                self.die();
            }
        }
        self.hp - before
    }

    /// `FUN_004f3e20(target, 0x19, ...)`: death clears every state and
    /// both counter banks.
    pub fn die(&mut self) {
        self.alive = false;
        self.hp = 0;
        self.states = [StateTimer::default(); STATE_SLOTS];
        self.earth_poison_acc = 0.;
    }

    /// `FUN_004f3e20(target, 0x19, 0, …)` 的那一支——**解**死亡就是复活
    /// （0x4f4495 附近）：在场位置 1、精归零、状态和两条计时器全清。精是 0，
    /// 起死回生的药随后靠自己的属性效果把它抬起来。已经在场的不再复活。
    pub fn revive(&mut self) {
        if self.alive {
            return;
        }
        self.alive = true;
        self.hp = 0;
        self.states = [StateTimer::default(); STATE_SLOTS];
        self.earth_poison_acc = 0.;
        self.defending = false;
        self.gauge = 0.;
        self.cast_gauge = 0.;
        self.phase = Phase::Charging;
    }

    pub fn apply_state(&mut self, k: usize, onset: i16, duration: i16) {
        if k >= STATE_SLOTS || !self.alive {
            return;
        }
        self.states[k] = StateTimer {
            active: true,
            onset,
            remaining: duration,
        };
        match k {
            K_POISON_EARTH => self.earth_poison_acc = self.max_hp as f32 * -0.02,
            // 这两个不是「挂个状态等它跳」，施加当场就改精——`FUN_004f3e20`
            // 的 case 0x18 / 0x19 直接写 `+0x24c` 然后 return，不落状态槽。
            K_DYING => {
                let floor = self.max_hp / 10;
                if self.hp > floor {
                    self.hp = floor;
                }
                self.states[k] = StateTimer::default();
            }
            K_INSTANT_DEATH => {
                if self.hp > 0 {
                    self.hp = 0;
                }
                self.states[k] = StateTimer::default();
            }
            _ => {}
        }
    }

    pub fn clear_state(&mut self, k: usize) {
        if let Some(slot) = self.states.get_mut(k) {
            *slot = StateTimer::default();
        }
    }

    /// One round of state bookkeeping — `FUN_004f48c0`. Only k = 1..=23
    /// participate; the onset delay runs down first and costs the round.
    pub fn tick_states(&mut self) {
        for k in TICKING_STATES {
            if !self.states[k].active {
                continue;
            }
            if self.states[k].onset > 0 {
                self.states[k].onset -= 1;
                continue;
            }
            if self.states[k].remaining < 1 {
                continue;
            }

            let delta = match k {
                K_POISON_WIND..=K_POISON_EARTH => {
                    let poison = match k {
                        1 => formulas::Poison::Wind,
                        2 => formulas::Poison::Thunder,
                        3 => formulas::Poison::Water,
                        4 => formulas::Poison::Fire,
                        _ => formulas::Poison::Earth,
                    };
                    Some(formulas::poison_tick(
                        poison,
                        self.max_hp,
                        self.hp,
                        &mut self.earth_poison_acc,
                    ))
                }
                _ => None,
            };
            if let Some(delta) = delta {
                self.change_hp(delta);
            }

            // A permanent state still decrements — 0x7fff simply outlasts
            // any battle.
            self.states[k].remaining -= 1;
            if self.states[k].remaining < 1 {
                self.clear_state(k);
            }
        }
    }
}

/// How a battle ended. The original's `CBSystem + 0x7c` result codes are
/// in the comments; the script-visible flag (`GetCombat`, `DAT_005833d0`)
/// is `result == 2`, so an escape reads as a win to the script.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ongoing,
    /// Result 2: every enemy is down, or the round cap ran out.
    Win,
    /// Result 3: the party is down.
    Lose,
    /// Result 2 as well: the party got away (`LAB_00515200` → phase 4).
    /// Whatever fell before that is still paid for.
    Escaped,
}

impl Outcome {
    /// What `GetCombat` will read.
    pub fn counts_as_win(self) -> bool {
        matches!(self, Outcome::Win | Outcome::Escaped)
    }
}

#[derive(Debug, Clone)]
pub struct Battle {
    pub combatants: Vec<Option<Combatant>>,
    pub round: i32,
    /// `+0x21c` from descriptor `+0x1c` (`CombatMaxRound`). Zero or less
    /// means no cap.
    pub max_round: i32,
    /// `+0x224` from descriptor `+0x48` (`CombatMustFail`): the round cap
    /// no longer ends the battle, so the only way out is to be beaten.
    pub must_fail: bool,
    /// Descriptor `+0x50`: the 逃跑 entry always fails. Set for every
    /// scripted `Combat`/`CombatBoss`, clear for field encounters.
    pub escape_forbidden: bool,
    /// The battlefield's own 五灵, from `combatScn.txt`'s `fivemagic`.
    pub scene_element: Option<u8>,
    /// Set by a successful [`Battle::try_escape`].
    pub escaped: bool,
    /// 开战时队伍身上的钱。花钱的技能（`special_consume_type` == 10）从这里
    /// 判断买不买得起，实际扣除记在 [`Self::spent_money`]，散场时一次性写回
    /// 冒险侧。
    pub money: i32,
    /// 开战时背包里的蛊（物品 6113）数量，蛊术的可用条件。
    pub gu: u32,
    /// 这一场里花掉的钱。
    pub spent_money: i32,
    /// 飞龙探云手偷到的物品，散场时进背包（原版 `FUN_0040ccf0` 当场加）。
    pub stolen_items: Vec<(u32, i32)>,
    /// 这一场里用掉的蛊。
    pub spent_gu: u32,
    pub rng: CombatRng,
}

impl Battle {
    pub fn new(seed: u32) -> Self {
        Self {
            combatants: vec![None; SLOTS],
            round: 0,
            max_round: 0,
            must_fail: false,
            escape_forbidden: false,
            scene_element: None,
            escaped: false,
            money: 0,
            gu: 0,
            spent_money: 0,
            stolen_items: Vec::new(),
            spent_gu: 0,
            rng: CombatRng::new(seed),
        }
    }

    /// 现在还剩多少钱。
    pub fn money_left(&self) -> i32 {
        self.money - self.spent_money
    }

    /// 现在还剩几个蛊。
    pub fn gu_left(&self) -> u32 {
        self.gu.saturating_sub(self.spent_gu)
    }

    pub fn place(&mut self, slot: usize, combatant: Combatant) {
        if slot < SLOTS {
            self.combatants[slot] = Some(combatant);
        }
    }

    pub fn get(&self, slot: usize) -> Option<&Combatant> {
        self.combatants.get(slot)?.as_ref()
    }

    pub fn get_mut(&mut self, slot: usize) -> Option<&mut Combatant> {
        self.combatants.get_mut(slot)?.as_mut()
    }

    fn side_in_play(&self, side: Side) -> bool {
        self.combatants
            .iter()
            .flatten()
            .any(|c| c.side == side && c.in_play())
    }

    /// `FUN_004f7910`, in its order: the party-down test comes first
    /// (0x4f7940), then enemy-down or round-out (0x4f7a6a), then the
    /// escape task's phase 4.
    pub fn outcome(&self) -> Outcome {
        // A party that has retired to flee is not a party that is down.
        if self.escaped {
            return Outcome::Escaped;
        }
        if !self.side_in_play(Side::Party) {
            return Outcome::Lose;
        }
        if !self.side_in_play(Side::Enemy) {
            return Outcome::Win;
        }
        // `+0x21c <= +0x220 && +0x224 == 0`: the cap is a win — the
        // settlement runs exactly as if the enemies had fallen — unless
        // the script declared the fight unwinnable, in which case the cap
        // does nothing at all.
        if self.max_round > 0 && self.max_round <= self.round && !self.must_fail {
            return Outcome::Win;
        }
        Outcome::Ongoing
    }

    /// 推进所有行动条，返回这一帧到点的那件事。
    ///
    /// 原版是**两条条两个阶段**（`FUN_004dba60`）：蓄力段推 `+0x20`、出手段
    /// 推 `+0x24`，各自用自己的速度。调度器先等蓄力条满（`FUN_004db6a0`）
    /// 才把单位排上行动任务，任务里 `FUN_004db730` 把它切进出手段，等出手条
    /// 满（`FUN_004db6c0`）行动才真正生效。中间这一段就是能被打断的窗口。
    ///
    /// 排序按 `FUN_004db7a0`：谁离完成最近谁先，还在蓄力的一律加 10000 的
    /// 偏置，所以正在出手的永远排在前面。
    pub fn update(&mut self, dt: f32) -> Option<Due> {
        self.advance(dt);
        self.due(&[])
    }

    /// 推一帧：蓄力条、出手条往前走。
    pub fn advance(&mut self, dt: f32) {
        for slot in 0..SLOTS {
            let Some(c) = self.get_mut(slot) else {
                continue;
            };
            if !c.in_play() {
                continue;
            }
            match c.phase {
                // 条满等着被排上行动，原版此时仍是 phase 0、条停在 11500。
                Phase::Ready => {}
                Phase::Charging => {
                    let speed = c.gauge_speed();
                    c.gauge += formulas::gauge_step(dt, speed);
                    if c.gauge > formulas::GAUGE_FULL - 1.0 {
                        c.gauge = formulas::GAUGE_FULL;
                        c.phase = Phase::Ready;
                        // 行动条一满就解除防御，和 0x515893 的判据一致。
                        c.defending = false;
                    }
                }
                Phase::Executing => {
                    let rate = c.cast_rate as f32;
                    c.cast_gauge += formulas::gauge_step(dt, rate);
                    if c.cast_gauge > formulas::GAUGE_FULL {
                        c.cast_gauge = formulas::GAUGE_FULL;
                    }
                }
            }
        }
    }

    /// 这一帧该谁：出手条满的优先，其次蓄力条满的。`skip` 里的这一帧不再交出——原版主调度
    /// 每帧把 11 个位都过一遍，一个人没挑出行动（托管、怪物 AI 都会这样）就下一帧再试，不耽误
    /// 后面的人。
    pub fn due(&self, skip: &[usize]) -> Option<Due> {
        // 出手完成的优先——原版的 +10000 偏置就是这个意思。
        let mut best: Option<(usize, f32)> = None;
        for slot in 0..SLOTS {
            let Some(c) = self.get(slot) else { continue };
            if !c.in_play() || c.phase != Phase::Executing {
                continue;
            }
            if c.cast_gauge > formulas::GAUGE_FULL - 1.0 {
                let eta = formulas::seconds_until_turn(c.cast_gauge, c.cast_rate as f32, false);
                if best.map(|(_, b)| eta < b).unwrap_or(true) {
                    best = Some((slot, eta));
                }
            }
        }
        if let Some((slot, _)) = best {
            return Some(Due::Executed(slot));
        }

        let mut best: Option<(usize, f32)> = None;
        for slot in 0..SLOTS {
            let Some(c) = self.get(slot) else { continue };
            if !c.in_play() || c.phase != Phase::Ready || skip.contains(&slot) {
                continue;
            }
            let eta = formulas::seconds_until_turn(c.gauge, c.gauge_speed(), false);
            if best.map(|(_, b)| eta < b).unwrap_or(true) {
                best = Some((slot, eta));
            }
        }
        best.map(|(slot, _)| Due::Ready(slot))
    }

    /// `FUN_004db730` 加行动任务里紧跟着的出手速度设定
    /// （`FUN_00514870`，0x514933 起）：
    ///
    /// ```text
    /// FUN_004db730(k);                       // 蓄力条清零、进出手段
    /// mult = FUN_004f76c0(this, k, 技能);     // 找不到返回 0
    /// if (mult > 3) FUN_004db750(k, 0, 11500, 1);   // 出手条直接满
    /// else          FUN_004db780(k, min(基速 × max(mult,1) × 2, 255));
    /// ```
    ///
    /// 这就是单位**自己**从蓄力段进入出手段的写入点。之前笔记里把
    /// `FUN_004db730` 和 `FUN_004db780` 都当成了没有调用方的死 API，是漏扫。
    pub fn begin_execute(&mut self, slot: usize, multiplier: i32) {
        let Some(c) = self.get_mut(slot) else { return };
        c.gauge = 0.;
        c.phase = Phase::Executing;
        if multiplier > 3 {
            // 瞬发：出手条直接拉满，出手速度不改。
            c.cast_gauge = formulas::GAUGE_FULL;
        } else {
            c.cast_gauge = 0.;
            c.cast_rate = c.cast_rate_for(multiplier);
        }
    }

    /// 逃跑任务（`LAB_00515200`，0x51527a 起）走的是同一条路，只是出手速度
    /// 换成 `速_eff × 8 + 20`。
    pub fn begin_escape(&mut self, slot: usize) {
        let Some(c) = self.get_mut(slot) else { return };
        c.gauge = 0.;
        c.phase = Phase::Executing;
        c.cast_gauge = 0.;
        c.cast_rate = c.escape_cast_rate();
    }

    /// `FUN_004db750(k, 0, 0, 1)` —— 留在出手段但把出手条清零，等于重新
    /// 吟唱。四条效果路径用它（`FUN_004f1420` / `004f1a30` / `004f1b20` /
    /// `004f1c40`）。
    pub fn restart_cast(&mut self, slot: usize) {
        let Some(c) = self.get_mut(slot) else { return };
        if c.phase != Phase::Executing {
            return;
        }
        c.gauge = 0.;
        c.cast_gauge = 0.;
    }

    /// A combatant has finished acting: reset its gauge (`FUN_004db710`)
    /// and, if it was the last enemy slot, count a round — which is how
    /// the original decides when to run status effects.
    ///
    /// The original's test is literally "slot 10 acted". Most battles do
    /// not field six enemies, and an empty slot 10 never acts, so taken
    /// literally the round would never advance and no poison would ever
    /// tick. Rolling on the last *occupied* slot instead keeps rounds
    /// moving; it matches the original exactly whenever slot 10 is
    /// filled. Which of the two the original really intends is still
    /// open — see the notes' 未解决 list.
    pub fn finish_turn(&mut self, slot: usize) {
        if let Some(c) = self.get_mut(slot) {
            c.gauge = 0.;
            c.cast_gauge = 0.;
            c.phase = Phase::Charging;
        }
        if Some(slot) == self.last_occupied_slot() {
            self.end_round();
        }
    }

    fn last_occupied_slot(&self) -> Option<usize> {
        (0..SLOTS).rev().find(|s| self.combatants[*s].is_some())
    }

    /// Roll the round over: every combatant's states tick.
    pub fn end_round(&mut self) {
        self.round += 1;
        for slot in 0..SLOTS {
            if let Some(c) = self.get_mut(slot) {
                if c.in_play() {
                    c.tick_states();
                }
            }
        }
    }

    /// `FUN_004f3e20(角色, k, 值, 播音效)`：给 `slot` 挂上或解掉状态 `k`
    /// （`k = 状态枚举 + 1`），连同它的前置规则和持续回合数。音效那一路移植
    /// 还没有通路，不做。
    ///
    /// 前置（0x4f3e20–0x4f3f5c）：
    ///
    /// * 目标有隐形（k23）且要设的不是隐形 → 什么都不做。
    /// * k ∈ 1..=5（五毒）：先清掉五毒（互斥），已有避毒（k26）则不加。
    ///   原版在这之前还查一次免疫 `FUN_004f3cc0`，那张免疫表移植还没有，
    ///   按「不免疫」处理 `[推断]`。
    /// * k ∈ 14..=23（定封禁眠乱狂镜避界隐形）：先把这十种全清掉。
    /// * k ∈ 6..=13（武防运速的增减）：成对互斥，先清掉自己那一对。
    ///
    /// 解除时 k=25 是复活（见 [`Combatant::revive`]），其余就是清位。
    pub fn set_state(&mut self, slot: usize, k: usize, on: bool) {
        if k == 0 || k >= STATE_SLOTS {
            return;
        }
        // 隐形免疫一切状态变化，自己那一位除外。
        if k != K_INVISIBLE
            && self
                .get(slot)
                .map(|c| c.has_state(K_INVISIBLE))
                .unwrap_or(false)
        {
            return;
        }

        if !on {
            if k == K_INSTANT_DEATH {
                if let Some(c) = self.get_mut(slot) {
                    c.revive();
                }
                return;
            }
            if let Some(c) = self.get_mut(slot) {
                c.clear_state(k);
            }
            return;
        }

        if (1..=5).contains(&k) {
            if let Some(c) = self.get_mut(slot) {
                for p in 1..=5 {
                    c.clear_state(p);
                }
            }
            if self
                .get(slot)
                .map(|c| c.has_state(K_POISON_IMMUNE))
                .unwrap_or(false)
            {
                return;
            }
        }
        if (14..=23).contains(&k) {
            if let Some(c) = self.get_mut(slot) {
                for j in 14..=23 {
                    c.clear_state(j);
                }
            }
        }
        if (6..=13).contains(&k) {
            // 6/7 武±，8/9 防±，10/11 运±，12/13 速±.
            let pair = if k % 2 == 0 { (k, k + 1) } else { (k - 1, k) };
            if let Some(c) = self.get_mut(slot) {
                c.clear_state(pair.0);
                c.clear_state(pair.1);
            }
        }

        let (onset, duration) = self.state_timer(slot, k);
        if let Some(c) = self.get_mut(slot) {
            c.apply_state(k, onset, duration);
        }
    }

    /// 施加时写进 `+0x490`（延迟）和 `+0x4c6`（回合数）的那两个数，逐个 case
    /// 抄自 `FUN_004f3e20` 的 switch（0x4f3ffc 起）。
    fn state_timer(&mut self, slot: usize, k: usize) -> (i16, i16) {
        match k {
            1..=5 => POISON_TIMERS[k - 1],
            6..=13 => (0, 5),
            14 => (0, 4),
            15 | 16 => (0, FOREVER),
            // 眠：`rand15() % 7 + 2`。
            17 => (0, (self.rng.below(7) + 2) as i16),
            18 | 19 => (0, FOREVER),
            // 镜：我方三回合，敌方五回合（原版按战斗位号 0..4 分）。
            20 => (
                0,
                if slot < PARTY_SLOTS && self.get(slot).map(|c| c.side) == Some(Side::Party) {
                    3
                } else {
                    5
                },
            ),
            21 | 22 => (0, 4),
            23 => (0, 5),
            K_POISON_IMMUNE => (0, FOREVER),
            // 24 濒死 / 25 即死不落状态槽，`apply_state` 里当场改精。
            _ => (0, 0),
        }
    }

    /// Being hit knocks a target that is mid-action back to charging.
    /// A skill hit sets the gauge back by an eighth per skill level
    /// (`FUN_004f1070`), a physical hit halves it (`FUN_004f0a10`); both
    /// only bite while the target is executing and its cast gauge has not
    /// filled.
    pub fn interrupt(&mut self, target: usize, skill_level: Option<i32>) {
        let Some(c) = self.get_mut(target) else {
            return;
        };
        if c.phase != Phase::Executing {
            return;
        }
        // 两条 API 都把出手条一并清掉：`FUN_004db750(k, g1, 0, 0)` 和
        // `FUN_004db8b0(k)`（g1 = 5750, g2 = 0, phase = 0）。
        c.gauge = match skill_level {
            Some(level) => formulas::GAUGE_FULL - level as f32 * (formulas::GAUGE_FULL / 8.0),
            None => formulas::GAUGE_FULL / 2.0,
        }
        .max(0.);
        c.cast_gauge = 0.;
        c.phase = Phase::Charging;
    }

    /// The 逃跑 menu entry, chosen by party slot `runner`
    /// (`FUN_00500d50`, 0x500e60–0x500f10): roll
    /// [`formulas::escape_percent`] against the present enemies, then let
    /// the encounter's 禁止逃跑 flag veto the result. On success the whole
    /// party leaves the field and the battle settles as a win.
    ///
    /// `CombatActorInfo::escape_rate` is never read by the combat code.
    pub fn try_escape(&mut self, runner: usize) -> bool {
        let Some((level, luck)) = self.get(runner).map(|c| (c.level, c.luck_eff())) else {
            return false;
        };
        let enemies: Vec<(i32, i32)> = self
            .combatants
            .iter()
            .flatten()
            // The original tests `+0x241` (present) only, not `+0x242`
            // (alive): a dead enemy still counts.
            .filter(|c| c.side == Side::Enemy)
            .map(|c| (c.level, c.luck_eff()))
            .collect();
        let percent = formulas::escape_percent(level, luck, &enemies);
        let mut ok = formulas::escape_roll(&mut self.rng, percent);
        // Descriptor `+0x50` is checked *after* the roll, so the LCG
        // still advances on a forbidden attempt.
        if self.escape_forbidden {
            ok = false;
        }
        if ok {
            self.retire_party();
        }
        ok
    }

    /// 全队离场，不掷骰子——鬼遁水（道具 special 2）走的就是这条：原版把
    /// `+0x225` 置 1 再派任务 0x10b，和逃跑成功后的收尾同一路。
    pub fn retire_party(&mut self) {
        self.escaped = true;
        for c in self.combatants.iter_mut().flatten() {
            if c.side == Side::Party {
                c.retired = true;
            }
        }
    }

    /// `FUN_004eee60`：打 `target` 的这一下有没有人替他挨。0 号位得在场站着、护的正是他、
    /// 站在盘心（五灵位 6）、没有 0x0e；另三道（`FUN_0050a770` 站位几何、`FUN_0050a700`、
    /// `FUN_004fca00(0x12)`）是舞台那边的，移植没有。只管我方 1..4 号位（0x4f0c3f 那一段）。
    pub fn protector_for(&self, target: usize) -> Option<usize> {
        if target == 0 || target >= PARTY_SLOTS {
            return None;
        }
        let p = self.get(0)?;
        (p.protecting == Some(target)
            && p.in_play()
            && p.element_position == 6
            && !p.has_state(0x0e))
        .then_some(0)
    }

    /// A plain physical attack from `attacker` onto `target`.
    pub fn basic_attack(&mut self, attacker: usize, target: usize) -> i32 {
        let delta = self.physical_hit(attacker, target);
        // 援护：伤害照打 `target` 算，落到护的人身上（`FUN_004f2c70(0, …)`，0x4f0d3a）。
        let victim = self.protector_for(target).unwrap_or(target);
        // The original does not clamp a negative-damage (defence-heavy)
        // result here; the HP path decides what happens.
        self.get_mut(victim)
            .map(|t| t.change_hp(delta))
            .unwrap_or(0)
    }

    /// `FUN_004f2800`：几个人各打 `target` 一下（`FUN_004ef7e0`，不落到身上）加起来
    /// 翻倍，再 ±10%（`round(和 × 0.1)` 以内，两次随机决定加还是减）。合击技用。
    pub fn joint_physical(&mut self, attackers: &[usize], target: usize) -> i32 {
        let sum: i32 = attackers
            .iter()
            .map(|a| self.physical_hit(*a, target))
            .sum();
        let mut v = sum * 2;
        if formulas::SPREAD_ENABLED {
            let spread = formulas::ftol(v as f32 * 0.1).abs();
            if spread != 0 {
                let r = self.rng.below(spread);
                v += if self.rng.coin() { r } else { -r };
            }
        }
        v
    }

    /// One physical blow's HP delta (negative is damage) without landing it
    /// — `FUN_004ef7e0`'s dodge test plus the kind-7 formula.
    pub fn physical_hit(&mut self, attacker: usize, target: usize) -> i32 {
        let (level, atk, attacker_element) = {
            let Some(a) = self.get(attacker) else {
                return 0;
            };
            let element = (1..=5u8).find(|e| a.elements[*e as usize]);
            (a.level, a.attack_eff(), element)
        };
        let (def, defender_elements, dodges) = {
            let Some(t) = self.get(target) else { return 0 };
            (
                t.defense_eff(),
                t.elements,
                t.has_state(K_EVADE) || t.has_state(K_INVISIBLE),
            )
        };
        // 避 and 隐形 shrug the hit off entirely (0x4ef7f0).
        if dodges {
            return 0;
        }

        let relation = formulas::element_relation(attacker_element, &defender_elements);
        formulas::physical_damage(&mut self.rng, level, atk, def, false, relation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fighter(actor_id: u32, side: Side, hp: i32, atk: i32, def: i32, speed: i32) -> Combatant {
        let mut c = Combatant::new(actor_id, side);
        c.hp = hp;
        c.max_hp = hp;
        c.attack = atk;
        c.defense = def;
        c.speed = speed;
        c.level = 1;
        c
    }

    fn two_sided_battle() -> Battle {
        let mut b = Battle::new(1234);
        b.place(0, fighter(0, Side::Party, 80, 26, 14, 3));
        b.place(5, fighter(100, Side::Enemy, 60, 20, 8, 2));
        b
    }

    #[test]
    fn outcome_follows_who_is_left() {
        let mut b = two_sided_battle();
        assert_eq!(b.outcome(), Outcome::Ongoing);

        b.get_mut(5).unwrap().die();
        assert_eq!(b.outcome(), Outcome::Win);

        let mut b = two_sided_battle();
        b.get_mut(0).unwrap().die();
        assert_eq!(b.outcome(), Outcome::Lose);
    }

    #[test]
    fn running_out_of_rounds_is_a_win_unless_the_script_says_must_fail() {
        let mut b = two_sided_battle();
        b.max_round = 3;
        b.round = 3;
        assert_eq!(b.outcome(), Outcome::Win, "回合打满走结算，不是输");

        b.must_fail = true;
        assert_eq!(b.outcome(), Outcome::Ongoing, "必败战里回合上限不结束战斗");
    }

    #[test]
    fn a_wiped_party_loses_even_when_the_enemies_are_down_too() {
        let mut b = two_sided_battle();
        b.get_mut(0).unwrap().die();
        b.get_mut(5).unwrap().die();
        assert_eq!(b.outcome(), Outcome::Lose, "FUN_004f7910 先看我方");
    }

    #[test]
    fn the_faster_combatant_acts_first_and_more_often() {
        let mut b = two_sided_battle();
        let mut turns = [0usize; SLOTS];
        let mut first: Option<usize> = None;

        for _ in 0..4000 {
            if let Some(Due::Ready(slot)) = b.update(0.1) {
                first.get_or_insert(slot);
                turns[slot] += 1;
                b.finish_turn(slot);
            }
        }

        assert_eq!(first, Some(0), "速度 3 的一方先动");
        assert!(
            turns[0] > turns[5],
            "速度 3 应比速度 2 出手更多: {} vs {}",
            turns[0],
            turns[5]
        );
    }

    #[test]
    fn a_downed_combatant_stops_getting_turns() {
        let mut b = two_sided_battle();
        b.get_mut(5).unwrap().die();
        for _ in 0..2000 {
            if let Some(Due::Ready(slot)) = b.update(0.1) {
                assert_ne!(slot, 5, "死者不该轮到");
                b.finish_turn(slot);
            }
        }
    }

    #[test]
    fn hp_clamps_and_death_wipes_states() {
        let mut c = fighter(0, Side::Party, 50, 10, 0, 3);
        c.apply_state(K_ATTACK_UP, 0, 5);
        assert_eq!(c.attack_eff(), 13, "10 + round(2.5)");

        c.change_hp(999);
        assert_eq!(c.hp, 50, "不超过上限");

        c.change_hp(-999);
        assert_eq!(c.hp, 0);
        assert!(!c.alive);
        assert!(!c.has_state(K_ATTACK_UP), "死亡清空全部状态");
    }

    #[test]
    fn being_hit_wakes_a_sleeper() {
        let mut c = fighter(0, Side::Party, 50, 10, 0, 3);
        c.apply_state(K_SLEEP, 0, 5);
        c.change_hp(-1);
        assert!(!c.has_state(K_SLEEP));

        // Healing does not.
        c.apply_state(K_SLEEP, 0, 5);
        c.change_hp(5);
        assert!(c.has_state(K_SLEEP));
    }

    #[test]
    fn evade_makes_a_basic_attack_whiff() {
        let mut b = two_sided_battle();
        b.get_mut(5).unwrap().apply_state(K_EVADE, 0, 5);
        assert_eq!(b.basic_attack(0, 5), 0);
        assert_eq!(b.get(5).unwrap().hp, 60, "一点没掉");
    }

    #[test]
    fn onset_delay_costs_a_round_before_the_poison_bites() {
        let mut c = fighter(0, Side::Party, 100, 10, 0, 3);
        // 风毒 with the database's 3-round onset.
        c.apply_state(K_POISON_WIND, 3, 5);
        for _ in 0..3 {
            c.tick_states();
            assert_eq!(c.hp, 100, "潜伏期内不掉血");
        }
        c.tick_states();
        assert_eq!(c.hp, 25, "然后每回合 75% 上限");
    }

    #[test]
    fn rounds_advance_even_without_a_full_enemy_line() {
        // Only slots 0 and 5 are filled, so the original's literal "slot
        // 10 acted" test would never fire.
        let mut b = two_sided_battle();
        b.get_mut(0).unwrap().apply_state(K_POISON_FIRE, 0, FOREVER);

        for _ in 0..4000 {
            if let Some(Due::Ready(slot)) = b.update(0.1) {
                b.finish_turn(slot);
            }
        }
        assert!(b.round > 0, "回合从未推进");
        assert!(b.get(0).unwrap().hp < 80, "火毒应该在扣血");
    }

    #[test]
    fn gauge_rate_is_speed_plus_four_times_five() {
        let mut c = fighter(0, Side::Party, 50, 10, 0, 3);
        assert_eq!(c.gauge_rate(), 35, "(3 + 4) * 5");

        // A 速+ state feeds through the same ±25%.
        c.apply_state(K_SPEED_UP, 0, 5);
        assert_eq!(c.speed_eff(), 4, "3 + round(0.75)");
        assert_eq!(c.gauge_rate(), 40);

        // The u8 the original stores it in wraps, and that is behaviour,
        // not an accident: 47 saturates the byte, 48 comes back as 4.
        let mut fast = fighter(0, Side::Party, 50, 10, 0, 47);
        assert_eq!(fast.gauge_rate(), 255);
        fast.speed = 48;
        assert_eq!(fast.gauge_rate(), 4, "(48 + 4) * 5 = 260 → 4");
    }

    #[test]
    fn escape_rolls_on_luck_and_level_and_settles_as_a_win() {
        // Same level, luck 20 vs 0: (20 - 0) * (1 - 1) + 3 = 3 → 30%.
        let mut b = two_sided_battle();
        b.get_mut(0).unwrap().luck = 20;
        assert_eq!(
            formulas::escape_percent(1, 20, &[(1, 0)]),
            30,
            "((运差) * (等级差) + 3) * 10"
        );

        let mut got_away = 0;
        let mut tries = 0;
        while got_away == 0 && tries < 50 {
            tries += 1;
            if b.try_escape(0) {
                got_away += 1;
            }
        }
        assert!(got_away > 0, "30% 五十次一次都没成");
        assert_eq!(b.outcome(), Outcome::Escaped);
        assert!(b.outcome().counts_as_win(), "GetCombat 读到的是 1");
    }

    #[test]
    fn a_forbidden_escape_never_works_and_a_hopeless_one_never_rolls() {
        let mut b = two_sided_battle();
        b.get_mut(0).unwrap().luck = 50;
        b.escape_forbidden = true;
        for _ in 0..50 {
            assert!(!b.try_escape(0), "禁止逃跑");
        }
        assert_eq!(b.outcome(), Outcome::Ongoing);

        // Out-levelling a luckier enemy: (0 - 5) * (10 - 1) + 3 = -42, the
        // product goes negative and the roll is skipped outright. (Being
        // *under*-levelled by the luckier enemy flips both signs and the
        // formula happily reports several hundred percent.)
        assert!(formulas::escape_percent(10, 0, &[(1, 5)]) < 1);
        let mut rng = CombatRng::new(1);
        assert!(!formulas::escape_roll(&mut rng, 0));
        // With nobody to flee from the division would be by zero; 100.
        assert_eq!(formulas::escape_percent(1, 0, &[]), 100);
    }

    #[test]
    fn a_hit_knocks_the_target_out_of_its_action() {
        let mut b = two_sided_battle();
        b.get_mut(5).unwrap().phase = Phase::Executing;
        b.get_mut(5).unwrap().gauge = 9000.;

        b.interrupt(5, None);
        assert_eq!(b.get(5).unwrap().gauge, formulas::GAUGE_FULL / 2.0);
        assert_eq!(b.get(5).unwrap().phase, Phase::Charging);

        // A level-8 skill knocks the gauge all the way to zero.
        b.get_mut(5).unwrap().phase = Phase::Executing;
        b.interrupt(5, Some(8));
        assert_eq!(b.get(5).unwrap().gauge, 0.);

        // Someone merely charging is not affected.
        b.get_mut(0).unwrap().gauge = 3000.;
        b.interrupt(0, None);
        assert_eq!(b.get(0).unwrap().gauge, 3000.);
    }

    #[test]
    fn poison_timers_are_a_coherent_set() {
        // 风毒: three rounds of grace, then five rounds at 75% of the
        // maximum — a death sentence unless it is cured in time.
        let mut c = fighter(0, Side::Party, 100, 10, 0, 3);
        let (onset, duration) = POISON_TIMERS[0];
        c.apply_state(K_POISON_WIND, onset, duration);
        for _ in 0..3 {
            c.tick_states();
        }
        assert_eq!(c.hp, 100, "潜伏期内一点不掉");
        c.tick_states();
        c.tick_states();
        assert!(!c.alive, "两跳之内必死");

        // 水毒 halves what is left, so it never finishes anyone off.
        let mut c = fighter(0, Side::Party, 100, 10, 0, 3);
        let (onset, duration) = POISON_TIMERS[2];
        c.apply_state(K_POISON_WATER, onset, duration);
        for _ in 0..duration {
            c.tick_states();
        }
        assert!(c.alive, "水毒杀不死人");
        // 100 → 50 → 25 → 13 → 7 → 4: each tick subtracts `current / 2`
        // with C's truncation, so the survivor keeps slightly more than a
        // clean halving would leave.
        assert_eq!(c.hp, 4);
    }

    #[test]
    fn a_battle_of_plain_attacks_terminates() {
        let mut b = two_sided_battle();
        let mut guard = 0;

        while b.outcome() == Outcome::Ongoing {
            guard += 1;
            assert!(guard < 100_000, "战斗不收敛");
            let Some(Due::Ready(slot)) = b.update(0.1) else {
                continue;
            };

            let side = b.get(slot).unwrap().side;
            let target = (0..SLOTS).find(|s| {
                b.get(*s)
                    .map(|c| c.side != side && c.in_play())
                    .unwrap_or(false)
            });
            if let Some(target) = target {
                b.basic_attack(slot, target);
            }
            b.finish_turn(slot);
        }

        // 26 vs 8 beats 20 vs 14, so the party should be the one standing.
        assert_eq!(b.outcome(), Outcome::Win);
    }

    #[test]
    fn a_unit_charges_then_casts_and_the_cast_runs_at_twice_the_speed() {
        // 原版：蓄力条满 -> 行动任务调 FUN_004db730 进出手段，出手速度是
        // min(基速 × 倍率 × 2, 255)。倍率 1 时出手段正好是蓄力段的两倍快。
        let mut b = Battle::new(1);
        let mut c = Combatant::new(0, Side::Party);
        c.hp = 100;
        c.max_hp = 100;
        c.speed = 3; // 基速 (3+4)*5 = 35，没到 255 的截断
        b.place(0, c);

        let mut charged = 0f32;
        loop {
            charged += 0.1;
            match b.update(0.1) {
                Some(Due::Ready(0)) => break,
                Some(other) => panic!("蓄力途中不该有 {other:?}"),
                None => assert!(charged < 60., "蓄力条填不满"),
            }
        }
        assert_eq!(b.get(0).unwrap().gauge, formulas::GAUGE_FULL);

        b.begin_execute(0, 1);
        let c = b.get(0).unwrap();
        assert_eq!(c.phase, Phase::Executing);
        assert_eq!(c.gauge, 0., "进出手段时蓄力条清零");
        assert_eq!(c.cast_gauge, 0.);
        assert_eq!(c.cast_rate, 70, "(3+4)*5*1*2");

        let mut cast = 0f32;
        loop {
            cast += 0.1;
            match b.update(0.1) {
                Some(Due::Executed(0)) => break,
                Some(other) => panic!("出手途中不该有 {other:?}"),
                None => assert!(cast < 60., "出手条填不满"),
            }
        }
        // 两段的时间比就是速度比：出手段快一倍，用时约一半。
        assert!(
            (charged / cast - 2.0).abs() < 0.15,
            "蓄力 {charged}s / 出手 {cast}s 不像 2:1"
        );
    }

    #[test]
    fn being_hit_mid_cast_knocks_the_unit_back_to_charging() {
        // 这是两条条模型真正的意义：以前行动是条满当帧结算的，出手段根本
        // 不存在，interrupt 打不到任何人。
        let mut b = Battle::new(1);
        let mut c = Combatant::new(0, Side::Party);
        c.hp = 100;
        c.max_hp = 100;
        c.speed = 10;
        b.place(0, c);

        b.get_mut(0).unwrap().gauge = formulas::GAUGE_FULL;
        b.get_mut(0).unwrap().phase = Phase::Ready;
        b.begin_execute(0, 1);
        b.update(0.05);
        assert!(b.get(0).unwrap().cast_gauge > 0., "出手条应当在走");

        b.interrupt(0, None);
        let c = b.get(0).unwrap();
        assert_eq!(c.phase, Phase::Charging);
        assert_eq!(c.cast_gauge, 0., "出手条被清掉");
        assert_eq!(c.gauge, formulas::GAUGE_FULL / 2.0, "物理打断退一半");
    }

    #[test]
    fn a_multiplier_over_three_finishes_the_cast_at_once() {
        // 0x514a00: mult > 3 -> FUN_004db750(k, 0, 11500, 1)，出手条直接满。
        let mut b = Battle::new(1);
        let mut c = Combatant::new(0, Side::Party);
        c.hp = 100;
        c.max_hp = 100;
        c.speed = 3;
        b.place(0, c);
        b.get_mut(0).unwrap().phase = Phase::Ready;
        b.get_mut(0).unwrap().gauge = formulas::GAUGE_FULL;

        b.begin_execute(0, 4);
        let c = b.get(0).unwrap();
        assert_eq!(c.phase, Phase::Executing);
        assert_eq!(c.cast_gauge, formulas::GAUGE_FULL);
        assert_eq!(b.update(0.01), Some(Due::Executed(0)));
    }

    #[test]
    fn the_escape_task_uses_its_own_cast_speed() {
        // 0x515295: shl al,3; add al,0x14  ->  速_eff * 8 + 20
        let mut c = Combatant::new(0, Side::Party);
        c.speed = 3;
        assert_eq!(c.escape_cast_rate(), 44);
        c.speed = 30;
        assert_eq!(c.escape_cast_rate(), 4, "按字节截断：260 & 0xff");
    }

    #[test]
    fn the_cast_rate_saturates_at_a_byte() {
        let mut c = Combatant::new(0, Side::Party);
        c.speed = 3;
        assert_eq!(c.cast_rate_for(1), 70);
        assert_eq!(c.cast_rate_for(3), 210);
        // 基速 35 × 4 × 2 = 280，钳到 255。倍率 > 3 其实走瞬发那一支，
        // 这里只是确认钳位本身。
        assert_eq!(c.cast_rate_for(4), 255);
        assert_eq!(c.cast_rate_for(0), 70, "倍率 0 当 1 用");
    }

    #[test]
    fn the_element_position_defaults_to_unset() {
        // 敌方在原版里是「槽号 − 4」，只是序号；我方要靠阵法页填。没填过的
        // 是 0，任何 1..6 的合击技位置要求都过不去——和原版一致。
        let c = Combatant::new(0, Side::Party);
        assert_eq!(c.element_position, 0);
    }

    #[test]
    fn defending_halves_incoming_damage_until_the_next_turn() {
        let mut b = Battle::new(1);
        let mut c = Combatant::new(0, Side::Party);
        c.hp = 100;
        c.max_hp = 100;
        c.speed = 10;
        b.place(0, c);

        // 没防御：全额。
        assert_eq!(b.get_mut(0).unwrap().change_hp(-40), -40);
        assert_eq!(b.get(0).unwrap().hp, 60);

        // 防御中：折半（原版 `delta < -1` 才折）。
        b.defend(0);
        assert!(b.get(0).unwrap().defending);
        assert_eq!(b.get_mut(0).unwrap().change_hp(-40), -20);
        assert_eq!(b.get_mut(0).unwrap().change_hp(-1), -1, "刚好 −1 的不折");

        // 行动条一满就解除，和 0x515893 的判据一致。
        for _ in 0..400 {
            b.update(0.05);
            if b.get(0).unwrap().phase == Phase::Ready {
                break;
            }
        }
        assert_eq!(b.get(0).unwrap().phase, Phase::Ready);
        assert!(!b.get(0).unwrap().defending, "轮到自己就该解除防御");
    }

    #[test]
    fn dying_and_instant_death_change_hp_on_the_spot() {
        // FUN_004f3e20 的 case 0x18 / 0x19：不落状态槽，直接改精然后 return。
        let mut c = Combatant::new(0, Side::Party);
        c.max_hp = 100;
        c.hp = 90;
        c.apply_state(K_DYING, 0, 5);
        assert_eq!(c.hp, 10, "濒死把精压到上限的十分之一");
        assert!(!c.has_state(K_DYING), "它不占状态槽");

        // 已经低于十分之一就不动。
        c.hp = 3;
        c.apply_state(K_DYING, 0, 5);
        assert_eq!(c.hp, 3);

        c.hp = 50;
        c.apply_state(K_INSTANT_DEATH, 0, 5);
        assert_eq!(c.hp, 0);
        assert!(!c.has_state(K_INSTANT_DEATH));
    }

    #[test]
    fn the_five_poisons_are_mutually_exclusive_and_poison_immunity_blocks_them() {
        let mut b = two_sided_battle();
        b.set_state(5, K_POISON_WIND, true);
        b.set_state(5, K_POISON_FIRE, true);
        assert!(!b.get(5).unwrap().has_state(K_POISON_WIND), "五毒互斥");
        assert!(b.get(5).unwrap().has_state(K_POISON_FIRE));
        assert_eq!(
            b.get(5).unwrap().states[K_POISON_FIRE].remaining,
            FOREVER,
            "火毒是永久的"
        );

        b.set_state(5, K_POISON_IMMUNE, true);
        b.set_state(5, K_POISON_WATER, true);
        assert!(!b.get(5).unwrap().has_state(K_POISON_WATER), "避毒挡住五毒");
        assert!(
            !b.get(5).unwrap().has_state(K_POISON_FIRE),
            "施加前先清五毒"
        );
    }

    #[test]
    fn invisibility_blocks_everything_and_the_ten_control_states_replace_each_other() {
        let mut b = two_sided_battle();
        b.set_state(5, K_SLEEP, true);
        b.set_state(5, K_CONFUSED, true);
        assert!(!b.get(5).unwrap().has_state(K_SLEEP), "14..23 十种互斥");
        assert!(b.get(5).unwrap().has_state(K_CONFUSED));

        b.set_state(5, K_INVISIBLE, true);
        assert!(!b.get(5).unwrap().has_state(K_CONFUSED));
        b.set_state(5, K_SLEEP, true);
        assert!(!b.get(5).unwrap().has_state(K_SLEEP), "隐形挡住一切状态");
    }

    #[test]
    fn clearing_death_is_a_revival() {
        let mut b = two_sided_battle();
        b.get_mut(0).unwrap().apply_state(K_ATTACK_UP, 0, 5);
        b.get_mut(0).unwrap().die();
        b.set_state(0, K_INSTANT_DEATH, false);
        let c = b.get(0).unwrap();
        assert!(c.alive);
        assert_eq!(c.hp, 0, "复活是 0 精");
        assert!(!c.has_state(K_ATTACK_UP), "状态和计时器全清");
    }

    #[test]
    fn the_paired_buffs_cancel_each_other() {
        let mut b = two_sided_battle();
        b.set_state(0, K_ATTACK_UP, true);
        b.set_state(0, K_ATTACK_DOWN, true);
        assert!(!b.get(0).unwrap().has_state(K_ATTACK_UP));
        assert!(b.get(0).unwrap().has_state(K_ATTACK_DOWN));
        assert_eq!(b.get(0).unwrap().states[K_ATTACK_DOWN].remaining, 5);
    }
}
