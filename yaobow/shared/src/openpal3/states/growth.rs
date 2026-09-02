//! How a playable character's attributes grow with level.
//!
//! These are the five per-character curves the original evaluates every
//! time it raises a level (`FUN_0040ff70` calls `FUN_004fa210`,
//! `FUN_004fa280`, `FUN_004fa330`, `FUN_004fa3a0`, `FUN_004fa440` in that
//! order). Each is a `switch` on the character index, so every one of the
//! five characters has its own coefficients.
//!
//! The game database's `attribute_values` are **not** a level-1
//! statline — they are these curves' intercepts, i.e. the value at level
//! 0. 景天's database 精 of 80 is the `+80` in `8L + 80`; at level 1 the
//! character actually has 88. That is confirmed against the original's own
//! status page, which shows 精 …/88, 气 …/17, 神 …/29 for a fresh 景天 —
//! exactly `8·1+80`, `9·√1+8` and `4·1+25`.

/// 景天 / 雪见 / 龙葵 / 紫萱 / 长卿.
pub const GROWTH_CHARACTERS: usize = 5;

/// 精 — `FUN_004fa210`, plain linear.
pub fn max_hp(actor: usize, level: i32) -> i32 {
    match actor {
        0 => level * 8 + 80,
        1 => level * 7 + 75,
        2 => level * 9 + 80,
        3 => level * 7 + 60,
        4 => level * 9 + 100,
        _ => 0,
    }
}

/// 气 — `FUN_004fa280`. The only curve with a square root, which is why
/// 气 climbs steeply early and then flattens.
pub fn max_sp(actor: usize, level: i32) -> i32 {
    let l = (level as f32).sqrt();
    let v = match actor {
        0 => l * 9.0 + 8.0,
        1 => l * 9.0 + 6.0,
        2 => l * 9.0 + 7.0,
        3 => l * 9.0 + 5.0,
        4 => (l + 1.0) * 9.0,
        _ => return 0,
    };
    v.round() as i32
}

/// 神 — `FUN_004fa330`. 紫萱's is written `(3L + 15) * 2` in the original
/// rather than `6L + 30`; kept in that shape so a later reader can match
/// it against the disassembly line for line.
pub fn max_mp(actor: usize, level: i32) -> i32 {
    match actor {
        0 => level * 4 + 25,
        1 => level * 5 + 25,
        2 => level * 4 + 22,
        3 => (level * 3 + 15) * 2,
        4 => level * 4 + 20,
        _ => 0,
    }
}

/// 武 — `FUN_004fa3a0`, fractional coefficients rounded at the end.
pub fn attack(actor: usize, level: i32) -> i32 {
    let l = level as f32;
    let v = match actor {
        0 => l * 4.5 + 26.0,
        1 => l * 4.0 + 24.0,
        2 => l * 4.5 + 20.0,
        3 => l * 4.0 + 22.0,
        4 => l * 5.0 + 35.0,
        _ => return 0,
    };
    v.round() as i32
}

/// 防 — `FUN_004fa440`.
pub fn defense(actor: usize, level: i32) -> i32 {
    let l = level as f32;
    let v = match actor {
        0 => l * 3.3 + 17.0,
        1 => l * 3.0 + 13.0,
        2 => l * 3.5 + 16.0,
        3 => l * 3.0 + 16.0,
        4 => l * 3.0 + 18.0,
        _ => return 0,
    };
    v.round() as i32
}

/// Every level-derived attribute at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Growth {
    pub max_hp: i32,
    pub max_sp: i32,
    pub max_mp: i32,
    pub attack: i32,
    pub defense: i32,
}

pub fn at_level(actor: usize, level: i32) -> Growth {
    Growth {
        max_hp: max_hp(actor, level),
        max_sp: max_sp(actor, level),
        max_mp: max_mp(actor, level),
        attack: attack(actor, level),
        defense: defense(actor, level),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jingtian_at_level_one_matches_the_original_status_page() {
        let g = at_level(0, 1);
        assert_eq!(g.max_hp, 88, "8*1 + 80");
        assert_eq!(g.max_sp, 17, "9*sqrt(1) + 8");
        assert_eq!(g.max_mp, 29, "4*1 + 25");
    }

    #[test]
    fn the_database_values_are_the_intercepts() {
        // 精 at level 0 is exactly what pal3_softstar.gdb stores for each
        // character: 80 / 75 / 80 / 60 / 100.
        assert_eq!(max_hp(0, 0), 80);
        assert_eq!(max_hp(1, 0), 75);
        assert_eq!(max_hp(2, 0), 80);
        assert_eq!(max_hp(3, 0), 60);
        assert_eq!(max_hp(4, 0), 100);
    }

    #[test]
    fn sp_growth_is_square_root_shaped() {
        // Doubling the level does not double 气.
        let a = max_sp(0, 4);
        let b = max_sp(0, 16);
        assert_eq!(a, 26, "9*2 + 8");
        assert_eq!(b, 44, "9*4 + 8");
        assert!(b < a * 2);
    }

    #[test]
    fn every_character_grows_monotonically_to_the_cap() {
        for actor in 0..GROWTH_CHARACTERS {
            let mut previous = at_level(actor, 1);
            for level in 2..=99 {
                let g = at_level(actor, level);
                assert!(g.max_hp > previous.max_hp, "actor {actor} lv{level} 精");
                assert!(g.max_sp >= previous.max_sp, "actor {actor} lv{level} 气");
                assert!(g.max_mp > previous.max_mp, "actor {actor} lv{level} 神");
                assert!(g.attack > previous.attack, "actor {actor} lv{level} 武");
                assert!(g.defense > previous.defense, "actor {actor} lv{level} 防");
                previous = g;
            }
        }
    }

    #[test]
    fn an_unknown_character_index_yields_zero() {
        assert_eq!(
            at_level(9, 50),
            Growth {
                max_hp: 0,
                max_sp: 0,
                max_mp: 0,
                attack: 0,
                defense: 0
            }
        );
    }
}
