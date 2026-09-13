//! The original combat system's arithmetic, transcribed instruction for
//! instruction from PAL3.exe v1.4.
//!
//! Every function here carries the address it came from so it can be
//! re-checked against `switch/re/notes/combat_formulas.md`. Nothing is
//! rounded "sensibly" or tidied up: where the original truncates toward
//! zero, so does this; where it divides an already-negative number, so
//! does this. A 1:1 port is only worth anything if the edge cases match.
//!
//! One place still says something surprising and is marked `SUSPECT`; it
//! is transcribed as read, not as it "should" be.

/// The C runtime's LCG, which the combat code inlines in over a hundred
/// places. Kept explicit and seedable so a battle can be replayed.
#[derive(Debug, Clone)]
pub struct CombatRng {
    seed: u32,
}

impl CombatRng {
    pub fn new(seed: u32) -> Self {
        Self { seed }
    }

    /// `rand()` — 15 bits, exactly as MSVC computes it.
    pub fn rand15(&mut self) -> i32 {
        self.seed = self.seed.wrapping_mul(0x343F_D).wrapping_add(0x269E_C3);
        ((self.seed >> 16) & 0x7FFF) as i32
    }

    /// The ten-bit draw (`>> 0x10 & 0x3ff`) the escape roll compares
    /// against a 0..1024 threshold.
    pub fn rand10(&mut self) -> i32 {
        self.seed = self.seed.wrapping_mul(0x343F_D).wrapping_add(0x269E_C3);
        ((self.seed >> 16) & 0x3FF) as i32
    }

    /// The coin flip the damage paths use to pick a spread's sign.
    pub fn coin(&mut self) -> bool {
        self.rand15() & 1 != 0
    }

    /// `n` in `rand() % n`, guarding the zero the original guards too.
    pub fn below(&mut self, n: i32) -> i32 {
        if n <= 0 { 0 } else { self.rand15() % n }
    }
}

/// C's float→int conversion: truncate toward zero.
pub(crate) fn ftol(v: f32) -> i32 {
    v as i32
}

/// The five elements, as the executable numbers them.
pub const ELEM_WATER: u8 = 1;
pub const ELEM_FIRE: u8 = 2;
pub const ELEM_WIND: u8 = 3;
pub const ELEM_THUNDER: u8 = 4;
pub const ELEM_EARTH: u8 = 5;

/// What `e` overcomes (`FUN_004fa700`). The cycle is
/// 水 → 火 → 雷 → 风 → 土 → 水, which is *not* the numeric order.
pub fn overcomes(e: u8) -> u8 {
    match e {
        ELEM_WATER => ELEM_FIRE,
        ELEM_FIRE => ELEM_THUNDER,
        ELEM_WIND => ELEM_EARTH,
        ELEM_THUNDER => ELEM_WIND,
        ELEM_EARTH => ELEM_WATER,
        other => other,
    }
}

/// What overcomes `e` (`FUN_004fa6a0`) — the inverse of [`overcomes`].
pub fn overcome_by(e: u8) -> u8 {
    match e {
        ELEM_WATER => ELEM_EARTH,
        ELEM_FIRE => ELEM_WATER,
        ELEM_WIND => ELEM_THUNDER,
        ELEM_THUNDER => ELEM_FIRE,
        ELEM_EARTH => ELEM_WIND,
        other => other,
    }
}

/// `FUN_004ef710`: −1, 0 or +1 for one attacker element against a
/// defender's element set. `defender` is indexed 1..=5 like the original's
/// `+0x458` array; index 0 is unused.
///
/// +1 means the defender resists (damage ×0.95), −1 that it is weak
/// (×1.05).
pub fn element_relation(attacker_element: Option<u8>, defender: &[bool; 6]) -> i32 {
    if !defender[1..].iter().any(|v| *v) {
        return 0;
    }
    let Some(e) = attacker_element else {
        return 0;
    };
    let resists = defender[overcome_by(e) as usize] as i32;
    let weak = defender[overcomes(e) as usize] as i32;
    resists - weak
}

/// The ±25% a buff/debuff state applies. `FUN_004f37a0` and its three
/// siblings all share this shape — and it really is 25%, not the 10% the
/// game database's own enum comment claims (0x4f37c9).
pub fn buffed(base: i32, up: bool, down: bool) -> i32 {
    if up {
        base + (base as f32 * 0.25).round() as i32
    } else if down {
        base + (base as f32 * -0.25).round() as i32
    } else {
        base
    }
}

/// Whether the combat system's random spreads are enabled at all
/// (`DAT_005dfdeb`). The original can turn every spread off globally.
pub const SPREAD_ENABLED: bool = true;

/// Physical damage, `FUN_004f98b0` kind 7 (0x4f9fdc–0x4fa1ee).
///
/// Returned as a **negative** number, the way the original carries it —
/// it is handed straight to the HP-change function as a delta.
pub fn physical_damage(
    rng: &mut CombatRng,
    attacker_level: i32,
    attack_eff: i32,
    defense_eff: i32,
    sneak: bool,
    element_relation: i32,
) -> i32 {
    let mut dmg = -ftol((attacker_level as f32 * 0.005 + 1.0) * (attack_eff - defense_eff) as f32);

    // ±10% spread.
    if SPREAD_ENABLED {
        let spread = ftol(dmg as f32 * 0.1).abs();
        if spread != 0 {
            let mut r = rng.below(spread);
            if rng.coin() {
                r = -r;
            }
            dmg += r;
        }
    }

    // The sneak/back-attack branch. Note this *reduces* damage: with
    // t ≈ 100 the coefficient is (100−45)/100 ≈ 0.55.
    if sneak {
        let t = if SPREAD_ENABLED {
            if rng.coin() {
                100 + rng.below(5)
            } else {
                100 - rng.below(5)
            }
        } else {
            100
        };
        dmg = ((t - 45) * dmg) / 100;
    }

    // 五灵, ±5% only.
    ftol((1.0 - element_relation as f32 * 0.05) * dmg as f32)
}

/// `FUN_004f98b0` kind 1 (0x4f98d9–0x4f99e4): the percentage chance a
/// skill lands. `skill_bonus` is the caster's accumulated equipment
/// 特技成功率.
pub fn hit_percent(
    rng: &mut CombatRng,
    caster_luck_eff: i32,
    target_luck_eff: i32,
    skill_bonus: f32,
) -> i32 {
    let mut p = (8 - target_luck_eff + caster_luck_eff) * 10;
    p += ftol(p as f32 * skill_bonus * 0.01);

    if SPREAD_ENABLED {
        let spread = ftol(p as f32 * 0.2).abs();
        if spread != 0 {
            let r = rng.below(spread);
            p = if rng.coin() { p + r } else { p - r };
        }
    }
    p
}

/// `FUN_004ee990`: does this skill hit at all?
///
/// `success_rate_level` below 2 rolls against [`hit_percent`]; 100 always
/// lands; everything else never does. The shipped database only ever uses
/// 0, 1 and 100, so those three branches cover all 187 skills.
pub fn skill_hits(
    rng: &mut CombatRng,
    skill_id: u32,
    success_rate_level: u8,
    caster_luck_eff: i32,
    target_luck_eff: i32,
    skill_bonus: f32,
) -> bool {
    if skill_id == 0x105 {
        return true;
    }
    if success_rate_level < 2 {
        let p = hit_percent(rng, caster_luck_eff, target_luck_eff, skill_bonus);
        if p <= 0 {
            return false;
        }
        return rng.rand15() % 1024 < (p as f32 * 10.24).round() as i32;
    }
    success_rate_level == 100
}

/// `FUN_004f98b0` kind 3 (0x4f9c00 area): the percentage chance the party
/// gets away, rolled by the battle menu's 逃跑 entry (`FUN_00500d50`).
///
/// `enemies` are the present (`+0x241`) enemy slots as `(level, 运_eff)`.
/// The original divides by their count without a guard; with nobody
/// present there is nobody to flee from, so 100 is returned instead.
///
/// Transcribed as read: the two differences are *multiplied*, so being
/// both unluckier and lower-levelled than the enemy comes out positive.
pub fn escape_percent(self_level: i32, self_luck_eff: i32, enemies: &[(i32, i32)]) -> i32 {
    if enemies.is_empty() {
        return 100;
    }
    let level_sum: i32 = enemies.iter().map(|(level, _)| *level).sum();
    let average_level = level_sum / enemies.len() as i32;
    let min_luck = enemies
        .iter()
        .map(|(_, luck)| *luck)
        .min()
        .unwrap_or(0xff_ffff);
    ((self_luck_eff - min_luck) * (self_level - average_level) + 3) * 10
}

/// The roll itself (`FUN_00500d50`, 0x500e9c–0x500ef0): below 1 never
/// works; otherwise ten bits of the LCG against `round(p * 10.24)`.
pub fn escape_roll(rng: &mut CombatRng, percent: i32) -> bool {
    chance_roll(rng, percent)
}

/// 战斗里通用的百分数掷骰：不到 1 直接不中、也不掷；否则 `rand & 0x3ff` 比
/// `round(p × 10.24)`（逃跑、普攻暴击/回避/附加状态/顺手偷都是这一套）。
pub fn chance_roll(rng: &mut CombatRng, percent: i32) -> bool {
    if percent < 1 {
        return false;
    }
    rng.rand10() < (percent as f32 * 10.24).round() as i32
}

/// `FUN_004f98b0` kind 6（0x4f9f7b）：普攻暴击率 = `trunc(√(运_eff / 2) × 10)`，
/// 运_eff ≤ 0 时是 0（`运 / 2` 是整数除，`× 10` 之后 `__ftol` 截断）。
pub fn crit_percent(luck_eff: i32) -> i32 {
    if luck_eff <= 0 {
        return 0;
    }
    (((luck_eff / 2) as f64).sqrt() * 10.0) as i32
}

/// `FUN_004f98b0` kind 5（0x4f9f49）：我方挨普攻时的回避率 =
/// `(速_eff(目标) − 速_eff(攻击方)) × 5`，负数算 0。
pub fn dodge_percent(target_speed_eff: i32, attacker_speed_eff: i32) -> i32 {
    ((target_speed_eff - attacker_speed_eff) * 5).max(0)
}

/// `FUN_004f98b0` kind 4（0x4f9f1f）：怪物普攻附带状态的几率 =
/// `5 + 运_eff(攻击方) − 运_eff(目标)`。
pub fn inflict_percent(attacker_luck_eff: i32, victim_luck_eff: i32) -> i32 {
    5 + attacker_luck_eff - victim_luck_eff
}

// ---- The action gauge ----------------------------------------------------

/// A combatant acts once its gauge passes this (`0x2cec`).
pub const GAUGE_FULL: f32 = 11500.0;

/// How fast a gauge fills: `dt * 50 * speed`.
pub fn gauge_step(dt: f32, speed: f32) -> f32 {
    dt * 50.0 * speed
}

/// Seconds until this combatant acts. Anything still charging is biased
/// by 10000 so that whoever is mid-execution always resolves first
/// (`FUN_004db7a0`).
pub fn seconds_until_turn(gauge: f32, speed: f32, charging: bool) -> f32 {
    if speed <= 0.0 {
        return f32::MAX;
    }
    let eta = (GAUGE_FULL - gauge) / (speed * 50.0);
    if charging { eta + 10000.0 } else { eta }
}

// ---- Poison ticks --------------------------------------------------------

/// The five poisons, by their slot in the per-round tick
/// (`FUN_004f48c0`, k = state enum + 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Poison {
    Wind = 1,
    Thunder = 2,
    Water = 3,
    Fire = 4,
    Earth = 5,
}

/// One round of poison damage, as a negative HP delta.
///
/// The five poisons look wildly inconsistent until you line each one up
/// with its duration (`FUN_004f3e20`, 0x4f3fdc onwards) and notice which
/// ones read the *maximum* and which the *current* HP:
///
/// | 毒 | 潜伏 | 持续 | 每回合 | 结果 |
/// |---|---|---|---|---|
/// | 风 | 3 | 5 | −75% 上限 | 三回合内不解就必死 |
/// | 雷 | 3 | 2 | −50% 上限 | 同上 |
/// | 水 | 0 | 5 | −50% **当前** | 每回合减半,永远杀不死 |
/// | 火 | 0 | 永久 | −5% 上限 | 慢性消耗 |
/// | 土 | 0 | 永久 | −2% 上限 ×1.5ⁿ | 越拖越重 |
///
/// So 风毒 and 雷毒 are death timers with a three-round grace period,
/// 水毒 is a drain that asymptotes rather than kills, and 火/土毒 are
/// chip damage. That the water branch reads current HP where the others
/// read the maximum is the detail that shows the split is deliberate —
/// it is the same function, one argument apart (`FUN_004f2bb0(x, 0)`
/// against `(x, 1)`).
///
/// `earth_accumulator` carries 土毒's escalation between rounds; seed it
/// with `-0.02 * max_hp` when the state is first applied.
pub fn poison_tick(
    poison: Poison,
    max_hp: i32,
    current_hp: i32,
    earth_accumulator: &mut f32,
) -> i32 {
    match poison {
        // -(3 * max) / 4, with C's round-toward-zero division.
        Poison::Wind => -((max_hp * 3) / 4),
        Poison::Thunder => -(max_hp / 2),
        Poison::Water => -(current_hp / 2),
        // (5 * max) / -100 — already negative.
        Poison::Fire => (5 * max_hp) / -100,
        Poison::Earth => {
            let delta = earth_accumulator.round() as i32;
            *earth_accumulator *= 1.5;
            delta
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rng_matches_the_c_runtime() {
        // The MSVC sequence from seed 1 is a fixed, well-known series.
        let mut rng = CombatRng::new(1);
        assert_eq!(rng.rand15(), 41);
        assert_eq!(rng.rand15(), 18467);
        assert_eq!(rng.rand15(), 6334);
        assert_eq!(rng.rand15(), 26500);
    }

    #[test]
    fn element_cycle_is_water_fire_thunder_wind_earth() {
        assert_eq!(overcomes(ELEM_WATER), ELEM_FIRE);
        assert_eq!(overcomes(ELEM_FIRE), ELEM_THUNDER);
        assert_eq!(overcomes(ELEM_THUNDER), ELEM_WIND);
        assert_eq!(overcomes(ELEM_WIND), ELEM_EARTH);
        assert_eq!(overcomes(ELEM_EARTH), ELEM_WATER);

        // The two tables must be exact inverses.
        for e in 1..=5u8 {
            assert_eq!(overcome_by(overcomes(e)), e, "element {e}");
        }
    }

    #[test]
    fn element_relation_favours_the_resistant_defender() {
        let none = [false; 6];
        assert_eq!(element_relation(Some(ELEM_WATER), &none), 0, "无属性守方");

        // 水 attacker: 土 resists it, 火 is weak to it.
        let mut earth = [false; 6];
        earth[ELEM_EARTH as usize] = true;
        assert_eq!(element_relation(Some(ELEM_WATER), &earth), 1);

        let mut fire = [false; 6];
        fire[ELEM_FIRE as usize] = true;
        assert_eq!(element_relation(Some(ELEM_WATER), &fire), -1);

        // Both present cancel out.
        let mut both = [false; 6];
        both[ELEM_EARTH as usize] = true;
        both[ELEM_FIRE as usize] = true;
        assert_eq!(element_relation(Some(ELEM_WATER), &both), 0);
    }

    #[test]
    fn buffs_are_twenty_five_percent() {
        assert_eq!(buffed(26, false, false), 26);
        assert_eq!(buffed(26, true, false), 26 + 7, "26*0.25 = 6.5 → 7");
        assert_eq!(
            buffed(26, false, true),
            26 - 7,
            "-6.5 → -7 (round half away)"
        );
        // A buff wins when both flags are somehow set, as in the original.
        assert_eq!(buffed(100, true, true), 125);
    }

    #[test]
    fn physical_damage_without_spread_is_the_base_expression() {
        // Level 1, 武 26 against 防 14, no element relation. The base is
        // -(1*0.005 + 1) * 12 = -12.06 → truncates to -12.
        let mut rng = CombatRng::new(1);
        let dmg = physical_damage(&mut rng, 1, 26, 14, false, 0);
        // The ±10% spread is 1 (|-12 * 0.1| = 1), so the result is within
        // one point of the base either way.
        assert!((-13..=-11).contains(&dmg), "got {dmg}");

        // A level-99 attacker's multiplier is 1.495.
        let base = -((99f32 * 0.005 + 1.0) * 12.0) as i32;
        assert_eq!(base, -17, "1.495 * 12 = 17.94 → 17");
    }

    #[test]
    fn defence_above_attack_deals_no_damage() {
        let mut rng = CombatRng::new(7);
        // 武 10 vs 防 40 gives a positive "damage", i.e. healing — the
        // original does not clamp here, the HP path does.
        let dmg = physical_damage(&mut rng, 1, 10, 40, false, 0);
        assert!(dmg > 0, "原版不在这里钳零，得到 {dmg}");
    }

    #[test]
    fn sneak_branch_actually_weakens_the_hit() {
        // (t - 45)/100 with t≈100 is ~0.55.
        let mut plain = CombatRng::new(3);
        let mut sneaky = CombatRng::new(3);
        let a = physical_damage(&mut plain, 1, 100, 0, false, 0);
        let b = physical_damage(&mut sneaky, 1, 100, 0, true, 0);
        assert!(b.abs() < a.abs(), "偷袭反而更轻: {a} vs {b}");
    }

    #[test]
    fn skill_success_rate_branches() {
        let mut rng = CombatRng::new(11);
        // 100 always lands, 2..99 never do.
        assert!(skill_hits(&mut rng, 1, 100, 0, 0, 0.));
        assert!(!skill_hits(&mut rng, 1, 50, 99, 0, 0.));
        // Skill 0x105 bypasses the roll entirely.
        assert!(skill_hits(&mut rng, 0x105, 50, 0, 99, 0.));
    }

    #[test]
    fn hit_percent_tracks_the_luck_difference() {
        // Spreads make this stochastic, so check the shape over a batch.
        let mut rng = CombatRng::new(5);
        let mut lucky = 0;
        let mut unlucky = 0;
        for _ in 0..200 {
            lucky += hit_percent(&mut rng, 20, 0, 0.);
            unlucky += hit_percent(&mut rng, 0, 20, 0.);
        }
        assert!(lucky > unlucky, "{lucky} vs {unlucky}");
    }

    #[test]
    fn gauge_fills_at_fifty_times_speed() {
        assert_eq!(gauge_step(1.0, 3.0), 150.0);
        // A speed-3 combatant needs 11500 / 150 ≈ 76.7 s to fill from
        // empty, which is why the raw 速度 attribute cannot be what feeds
        // this — see the notes' 未解决 list.
        assert!((seconds_until_turn(0.0, 3.0, false) - 76.666_67).abs() < 0.01);
        // Charging is biased so execution always resolves first.
        assert!(seconds_until_turn(0.0, 3.0, true) > 10000.0);
    }

    #[test]
    fn poison_ticks_match_the_disassembly() {
        let mut acc = 0.0;
        assert_eq!(poison_tick(Poison::Wind, 100, 100, &mut acc), -75);
        assert_eq!(poison_tick(Poison::Thunder, 100, 100, &mut acc), -50);
        assert_eq!(
            poison_tick(Poison::Water, 100, 40, &mut acc),
            -20,
            "取当前值"
        );
        assert_eq!(poison_tick(Poison::Fire, 100, 100, &mut acc), -5);

        // 土毒 escalates from -2% of max.
        acc = -0.02 * 100.0;
        assert_eq!(poison_tick(Poison::Earth, 100, 100, &mut acc), -2);
        assert_eq!(poison_tick(Poison::Earth, 100, 100, &mut acc), -3);
        assert_eq!(
            poison_tick(Poison::Earth, 100, 100, &mut acc),
            -5,
            "-4.5 → -5"
        );

        // Truncation toward zero, not flooring: -(3*7)/4 = -5, not -6.
        assert_eq!(poison_tick(Poison::Wind, 7, 7, &mut acc), -5);
    }
}
