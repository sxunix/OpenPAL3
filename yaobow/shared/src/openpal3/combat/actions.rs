//! What a combatant can do on its turn: cast a skill, use an item,
//! attack, defend, flee.
//!
//! The value a skill or item moves is `FUN_004f98b0` kind 2
//! (0x4f99e7–0x4f9dfe); how it lands on the target is `FUN_004eef20`
//! (0x4ef43a). Both are transcribed here as they are, including the part
//! that surprised me — see [`element_modifier`].

use super::formulas::{self, CombatRng};
use super::state::{Battle, Combatant, Phase, Side};

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
    pub level: i32,
    pub success_rate_level: u8,
    /// Up to two elements, from the skill's `element_attributes`.
    pub elements: (Option<u8>, Option<u8>),
    /// Per attribute slot: the impact type and its raw value.
    pub impacts: [(u8, i16); ATTRIBUTE_SLOTS],
    pub sp_cost: i32,
    pub mp_cost: i32,
}

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

/// What one cast did, for logging and for the UI to replay later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CastResult {
    pub hit: bool,
    /// Per attribute slot, how much actually moved.
    pub deltas: [i32; ATTRIBUTE_SLOTS],
}

impl Battle {
    /// Cast `skill` from `caster` at `target`.
    ///
    /// Costs come off the caster whether or not the skill lands, which is
    /// what the original does — the cost is paid when the action starts.
    pub fn cast_skill(&mut self, caster: usize, target: usize, skill: &SkillEffect) -> CastResult {
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

        if let Some(c) = self.get_mut(caster) {
            c.sp = (c.sp - skill.sp_cost).max(0);
            c.mp = (c.mp - skill.mp_cost).max(0);
        }

        let Some(t) = self.get(target) else {
            return result;
        };
        let target_luck = t.luck_eff();
        let target_element = (1..=5u8).find(|e| t.elements[*e as usize]);

        if !formulas::skill_hits(
            &mut self.rng,
            skill.id,
            skill.success_rate_level,
            caster_luck,
            target_luck,
            0.,
        ) {
            return result;
        }
        result.hit = true;

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

        // A landed skill knocks the target out of whatever it was doing.
        self.interrupt(target, Some(skill.level));
        result
    }

    /// 防御 — the combatant simply ends its turn. The original has no
    /// defence bonus state to set; the value of defending is that the
    /// gauge resets without the combatant having exposed itself to an
    /// interrupt mid-action.
    pub fn defend(&mut self, slot: usize) {
        if let Some(c) = self.get_mut(slot) {
            c.phase = Phase::Charging;
            c.gauge = 0.;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openpal3::combat::state::{Combatant, Side};

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
            ..Default::default()
        };
        let result = b.cast_skill(0, 5, &skill);
        assert!(!result.hit);
        assert_eq!(b.get(0).unwrap().sp, 45, "气照扣");
        assert_eq!(b.get(0).unwrap().mp, 33, "神照扣");
        assert_eq!(b.get(5).unwrap().hp, 200, "没打中就没伤害");
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
