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
//! * **Escape is not a roll.** It succeeds unless the encounter forbids
//!   fleeing outright; the database's per-monster `escape_rate` has no
//!   reader anywhere in the combat code.

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
pub const K_SLEEP: usize = 17;
pub const K_CONFUSED: usize = 18;
pub const K_EVADE: usize = 21;
pub const K_INVISIBLE: usize = 23;

/// A permanent duration (`0x7fff`).
pub const FOREVER: i16 = 0x7fff;

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

    pub gauge: f32,
    pub phase: Phase,
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
            gauge: 0.,
            phase: Phase::Charging,
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

    pub fn apply_state(&mut self, k: usize, onset: i16, duration: i16) {
        if k >= STATE_SLOTS || !self.alive {
            return;
        }
        self.states[k] = StateTimer {
            active: true,
            onset,
            remaining: duration,
        };
        if k == K_POISON_EARTH {
            self.earth_poison_acc = self.max_hp as f32 * -0.02;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Ongoing,
    /// Every enemy is down.
    Win,
    /// The party is down, or the round cap was reached.
    Lose,
}

#[derive(Debug, Clone)]
pub struct Battle {
    pub combatants: Vec<Option<Combatant>>,
    pub round: i32,
    /// `+0x1c`, from `CombatMaxRound`. Zero or less means no cap.
    pub max_round: i32,
    /// `+0x48`, from `CombatNotGameOver`: losing does not end the game.
    pub not_game_over: bool,
    /// The battlefield's own 五灵, from `combatScn.txt`'s `fivemagic`.
    pub scene_element: Option<u8>,
    pub rng: CombatRng,
}

impl Battle {
    pub fn new(seed: u32) -> Self {
        Self {
            combatants: vec![None; SLOTS],
            round: 0,
            max_round: 0,
            not_game_over: false,
            scene_element: None,
            rng: CombatRng::new(seed),
        }
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

    pub fn outcome(&self) -> Outcome {
        if !self.side_in_play(Side::Enemy) {
            return Outcome::Win;
        }
        if !self.side_in_play(Side::Party) {
            return Outcome::Lose;
        }
        // The round cap only loses the battle when the script has not
        // declared it survivable.
        if self.max_round > 0 && self.max_round <= self.round && !self.not_game_over {
            return Outcome::Lose;
        }
        Outcome::Ongoing
    }

    /// Advance every gauge by `dt` seconds and return the slot that is
    /// due to act, if any.
    ///
    /// Ordering follows `FUN_004db7a0`: whoever has the smallest time to
    /// completion goes first, and anyone still charging carries a +10000
    /// bias so an in-flight action always resolves ahead of them.
    pub fn update(&mut self, dt: f32) -> Option<usize> {
        for slot in 0..SLOTS {
            let Some(c) = self.get_mut(slot) else {
                continue;
            };
            if !c.in_play() {
                continue;
            }
            if c.phase == Phase::Ready {
                continue;
            }
            let speed = c.gauge_speed();
            c.gauge += formulas::gauge_step(dt, speed);
            if c.gauge > formulas::GAUGE_FULL - 1.0 {
                c.gauge = formulas::GAUGE_FULL;
                c.phase = Phase::Ready;
            }
        }

        let mut best: Option<(usize, f32)> = None;
        for slot in 0..SLOTS {
            let Some(c) = self.get(slot) else { continue };
            if !c.in_play() || c.phase != Phase::Ready {
                continue;
            }
            let eta = formulas::seconds_until_turn(c.gauge, c.gauge_speed(), false);
            if best.map(|(_, b)| eta < b).unwrap_or(true) {
                best = Some((slot, eta));
            }
        }
        best.map(|(slot, _)| slot)
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
        c.gauge = match skill_level {
            Some(level) => formulas::GAUGE_FULL - level as f32 * (formulas::GAUGE_FULL / 8.0),
            None => formulas::GAUGE_FULL / 2.0,
        }
        .max(0.);
        c.phase = Phase::Charging;
    }

    /// Fleeing. The original runs no roll at all: it either always works
    /// or is refused outright, depending on a flag on the encounter
    /// (`FUN_004f1fa0` case 2). `CombatActorInfo::escape_rate` is never
    /// read by the combat code.
    pub fn try_escape(&self, escape_forbidden: bool) -> bool {
        !escape_forbidden
    }

    /// A plain physical attack from `attacker` onto `target`.
    pub fn basic_attack(&mut self, attacker: usize, target: usize) -> i32 {
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
        let delta = formulas::physical_damage(&mut self.rng, level, atk, def, false, relation);
        // The original does not clamp a negative-damage (defence-heavy)
        // result here; the HP path decides what happens.
        self.get_mut(target)
            .map(|t| t.change_hp(delta))
            .unwrap_or(0)
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
    fn round_cap_only_loses_when_the_script_allows_it() {
        let mut b = two_sided_battle();
        b.max_round = 3;
        b.round = 3;
        assert_eq!(b.outcome(), Outcome::Lose);

        b.not_game_over = true;
        assert_eq!(b.outcome(), Outcome::Ongoing, "剧本声明输了也不结束");
    }

    #[test]
    fn the_faster_combatant_acts_first_and_more_often() {
        let mut b = two_sided_battle();
        let mut turns = [0usize; SLOTS];
        let mut first: Option<usize> = None;

        for _ in 0..4000 {
            if let Some(slot) = b.update(0.1) {
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
            if let Some(slot) = b.update(0.1) {
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
            if let Some(slot) = b.update(0.1) {
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
    fn escape_is_not_a_roll() {
        let b = two_sided_battle();
        assert!(b.try_escape(false));
        assert!(!b.try_escape(true));
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
    fn a_battle_of_plain_attacks_terminates() {
        let mut b = two_sided_battle();
        let mut guard = 0;

        while b.outcome() == Outcome::Ongoing {
            guard += 1;
            assert!(guard < 100_000, "战斗不收敛");
            let Some(slot) = b.update(0.1) else { continue };

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
}
