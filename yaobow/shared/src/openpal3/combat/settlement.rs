//! What winning a battle pays out: experience, money and loot.
//!
//! Experience is `FUN_004fa930`'s `param_1 != 0` branch; money and loot
//! are `FUN_004fb370`. Both walk the enemy slots and pay only for enemies
//! that are actually dead.
//!
//! Two details are easy to miss and are reproduced here:
//!
//! * whoever landed the killing blow on the **last** enemy to fall gets a
//!   30% bonus on top of the even split, computed only over the enemies
//!   that died at that same moment by that same hand;
//! * a party member who is down gets nothing — not a share, not a bonus.

/// One defeated enemy, as far as the payout is concerned.
#[derive(Debug, Clone, Copy)]
pub struct DefeatedEnemy {
    /// `CombatActorInfo::experience`.
    pub experience: i32,
    /// `CombatActorInfo::money_when_killed`.
    pub money: i32,
    /// `CombatActorInfo::normal_loot` and its count. The original rolls no
    /// dice here: if the row names an item with a positive count, it
    /// drops, every time.
    pub loot: Option<(u32, i32)>,
    /// The party slot that landed the killing blow.
    pub killer: Option<usize>,
    /// When it fell. The original stores a float timestamp per combatant
    /// and compares with `>=`, so simultaneous deaths all count.
    pub death_time: f32,
}

/// A party member's standing at the end of the battle.
#[derive(Debug, Clone, Copy)]
pub struct Survivor {
    pub slot: usize,
    pub alive: bool,
    /// The accumulated 经验加成 percentage (`+0x2ac`), from 精进 or from
    /// equipment. Zero means no bonus.
    pub exp_bonus_percent: f32,
    /// The accumulated 金钱加成 percentage (`+0x2b0`).
    ///
    /// UNRESOLVED: the field is summed across the party and applied to
    /// the money total, but which item property feeds it has not been
    /// found — no database column is an obvious match. Zero until then,
    /// which makes the bonus vanish exactly as it does with no bonus
    /// equipment on.
    pub money_bonus_percent: f32,
}

/// What the battle paid out.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Spoils {
    /// Experience per party slot, after the split, the killer's bonus and
    /// each member's own percentage.
    pub experience: Vec<(usize, i32)>,
    pub money: i32,
    /// Item id and count, in the order the enemies fell.
    pub loot: Vec<(u32, i32)>,
}

/// Work out the payout. `enemies` should hold only the defeated ones.
pub fn settle(enemies: &[DefeatedEnemy], party: &[Survivor]) -> Spoils {
    let mut spoils = Spoils::default();

    // ---- money ----
    spoils.money = enemies.iter().map(|e| e.money).sum();
    if spoils.money > 0 {
        // Every *present* member contributes their percentage, dead or
        // not — the original's loop only checks presence here.
        let percent: f32 = party.iter().map(|p| p.money_bonus_percent).sum();
        spoils.money += (spoils.money as f32 * percent * 0.01) as i32;
    }

    // ---- loot ----
    spoils.loot = enemies
        .iter()
        .filter_map(|e| e.loot)
        .filter(|(id, count)| *id != 0 && *count > 0)
        .collect();

    // ---- experience ----
    let total: i32 = enemies.iter().map(|e| e.experience).sum();
    let living: Vec<&Survivor> = party.iter().filter(|p| p.alive).collect();
    if living.is_empty() {
        return spoils;
    }
    let share = total / living.len() as i32;

    // The last enemy to fall, and who felled it. The original's scan
    // updates on a strict `>`, so when several fall at the same instant
    // the FIRST of them is the one whose killer takes the bonus — not the
    // last, which is what a plain `max_by` would pick.
    let mut last: Option<&DefeatedEnemy> = None;
    for e in enemies {
        if last
            .map(|best| e.death_time > best.death_time)
            .unwrap_or(true)
        {
            last = Some(e);
        }
    }
    let killer_bonus = last.and_then(|last| {
        let killer = last.killer?;
        let pool: i32 = enemies
            .iter()
            .filter(|e| e.death_time >= last.death_time && e.killer == Some(killer))
            .map(|e| e.experience)
            .sum();
        Some((killer, pool * 3 / 10))
    });

    for member in &living {
        let mut gained = share;
        if let Some((killer, bonus)) = killer_bonus {
            if killer == member.slot {
                gained += bonus;
            }
        }
        // The member's own percentage lands last, on the total.
        gained = (gained as f32 * (member.exp_bonus_percent + 100.0) * 0.01).round() as i32;
        spoils.experience.push((member.slot, gained));
    }

    spoils
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enemy(experience: i32, money: i32, killer: Option<usize>, death_time: f32) -> DefeatedEnemy {
        DefeatedEnemy {
            experience,
            money,
            loot: None,
            killer,
            death_time,
        }
    }

    fn member(slot: usize, alive: bool) -> Survivor {
        Survivor {
            slot,
            alive,
            exp_bonus_percent: 0.,
            money_bonus_percent: 0.,
        }
    }

    #[test]
    fn experience_splits_evenly_among_the_living() {
        let enemies = [enemy(100, 0, None, 1.0), enemy(50, 0, None, 2.0)];
        let party = [member(0, true), member(1, true), member(2, false)];

        let spoils = settle(&enemies, &party);
        // 150 over the two who are standing; the third gets nothing.
        assert_eq!(spoils.experience, vec![(0, 75), (1, 75)]);
    }

    #[test]
    fn the_last_blow_is_worth_thirty_percent_more() {
        let enemies = [enemy(100, 0, Some(1), 1.0), enemy(100, 0, Some(0), 5.0)];
        let party = [member(0, true), member(1, true)];

        let spoils = settle(&enemies, &party);
        // 200 split two ways is 100 each; slot 0 landed the final blow on
        // a 100-exp enemy, so it also takes 30.
        assert_eq!(spoils.experience, vec![(0, 130), (1, 100)]);
    }

    #[test]
    fn simultaneous_kills_by_the_same_hand_all_count_toward_the_bonus() {
        let enemies = [
            enemy(100, 0, Some(0), 5.0),
            enemy(100, 0, Some(0), 5.0),
            // Same instant, different killer — not in the pool.
            enemy(100, 0, Some(1), 5.0),
        ];
        let party = [member(0, true), member(1, true)];

        let spoils = settle(&enemies, &party);
        // 300 / 2 = 150 each; slot 0's pool is 200, worth 60.
        assert_eq!(spoils.experience, vec![(0, 210), (1, 150)]);
    }

    #[test]
    fn a_members_own_percentage_applies_to_its_whole_share() {
        let enemies = [enemy(100, 0, None, 1.0)];
        let mut keen = member(0, true);
        keen.exp_bonus_percent = 50.;
        let party = [keen, member(1, true)];

        let spoils = settle(&enemies, &party);
        assert_eq!(spoils.experience, vec![(0, 75), (1, 50)], "50 * 1.5");
    }

    #[test]
    fn a_wiped_party_is_paid_nothing() {
        let enemies = [enemy(100, 50, None, 1.0)];
        let party = [member(0, false)];
        let spoils = settle(&enemies, &party);
        assert!(spoils.experience.is_empty());
        // Money is still summed — the original computes it before it ever
        // looks at who is standing.
        assert_eq!(spoils.money, 50);
    }

    #[test]
    fn money_sums_and_takes_the_party_percentage() {
        let enemies = [enemy(0, 30, None, 1.0), enemy(0, 20, None, 2.0)];
        let mut rich = member(0, true);
        rich.money_bonus_percent = 10.;
        let party = [rich, member(1, true)];

        let spoils = settle(&enemies, &party);
        assert_eq!(spoils.money, 55, "50 + 10%");
    }

    #[test]
    fn loot_always_drops_when_the_row_names_one() {
        let mut with_loot = enemy(0, 0, None, 1.0);
        with_loot.loot = Some((80009, 2));
        let mut empty = enemy(0, 0, None, 2.0);
        empty.loot = Some((0, 5));
        let mut zero_count = enemy(0, 0, None, 3.0);
        zero_count.loot = Some((80010, 0));

        let spoils = settle(&[with_loot, empty, zero_count], &[member(0, true)]);
        assert_eq!(spoils.loot, vec![(80009, 2)], "没有掉落判定,有就掉");
    }
}
