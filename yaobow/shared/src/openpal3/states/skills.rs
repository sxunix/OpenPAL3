//! A character's own skills: what they know, how far each has been
//! trained, and when training pays off.
//!
//! Skills level up by being used. The victory settlement (`FUN_004fa930`)
//! walks every learned skill and compares its use count against the
//! database row's `times_before_level_up`, which holds one threshold per
//! level. The scan runs from the highest threshold downwards and takes
//! the first that the count has reached, so a burst of use can carry a
//! skill up more than one level at once.

/// The original caps a skill at level 3 (`*(byte *)(… + level) < 3`
/// guards the whole block).
pub const MAX_SKILL_LEVEL: u8 = 3;

/// One skill a character knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LearnedSkill {
    pub id: u32,
    /// 0..=3.
    pub level: u8,
    /// How many times it has been cast since it was learned.
    pub uses: u8,
}

impl LearnedSkill {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            level: 0,
            uses: 0,
        }
    }
}

/// The level a skill should be at, given its use count and its database
/// row's thresholds — `FUN_004fa930`'s inner scan (0x4fa9d0 area).
///
/// Returns `None` when nothing changes. The original only enters the scan
/// when the *current* level's own threshold has been reached, so a skill
/// that is already ahead of its thresholds is left alone.
pub fn levelled_up_to(current_level: u8, uses: u8, times_before_level_up: &[u8; 4]) -> Option<u8> {
    if current_level >= MAX_SKILL_LEVEL {
        return None;
    }
    // The gate: this level's own threshold must have been met.
    if times_before_level_up[current_level as usize] > uses {
        return None;
    }
    // Then take the highest level whose threshold is also met.
    for index in (0..3).rev() {
        if times_before_level_up[index] <= uses {
            let level = index as u8 + 1;
            return (level > current_level).then_some(level);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thresholds of 3 / 8 / 20 uses for levels 1 / 2 / 3.
    const TIMES: [u8; 4] = [3, 8, 20, 0];

    #[test]
    fn a_skill_levels_when_its_threshold_is_reached() {
        assert_eq!(levelled_up_to(0, 2, &TIMES), None, "还差一次");
        assert_eq!(levelled_up_to(0, 3, &TIMES), Some(1));
        assert_eq!(levelled_up_to(1, 8, &TIMES), Some(2));
        assert_eq!(levelled_up_to(2, 20, &TIMES), Some(3));
    }

    #[test]
    fn a_burst_of_use_can_skip_a_level() {
        // Never levelled but already used 20 times: the downward scan
        // finds the level-3 threshold first.
        assert_eq!(levelled_up_to(0, 20, &TIMES), Some(3));
    }

    #[test]
    fn a_maxed_skill_stops() {
        assert_eq!(levelled_up_to(MAX_SKILL_LEVEL, 250, &TIMES), None);
    }

    #[test]
    fn already_ahead_of_the_thresholds_changes_nothing() {
        // Level 2 with only 8 uses: level 2's threshold is met, but the
        // scan finds nothing higher, so it stays put.
        assert_eq!(levelled_up_to(2, 8, &TIMES), None);
    }

    #[test]
    fn a_zero_threshold_levels_immediately() {
        // Some rows really do store 0, which the `<=` lets through at
        // once — transcribed rather than guarded against.
        let instant = [0u8, 0, 0, 0];
        assert_eq!(levelled_up_to(0, 0, &instant), Some(3));
    }
}
