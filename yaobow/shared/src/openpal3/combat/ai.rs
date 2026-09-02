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
use super::state::{Battle, SLOTS, Side};

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
}
