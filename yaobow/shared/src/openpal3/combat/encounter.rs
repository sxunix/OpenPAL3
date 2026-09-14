//! What a script (or the field) hands to the combat system to start a
//! battle: the original's descriptor at `DAT_00c11700` and the globals
//! the `Combat*` opcodes poke before it is filled in.
//!
//! The pieces, with where they come from:
//!
//! * `Combat(count, m1, m2, m3)` (opcode 73, handler `0x437280`) builds
//!   the monster list with [`pick_monsters`] (`FUN_018c59e0`), sets the
//!   禁止逃跑 byte (`DAT_00c11750 = 1`), and starts the battle;
//! * `CombatBoss(m1..m6)` (opcode 80, `0x4373d0`) copies six ids as
//!   given, sets the same byte, and starts with the BGM muted;
//! * `CombatMaxRound(n)` (82) → `DAT_00c1171c`, `CombatMustFail` (83) →
//!   `DAT_00c11748`, `CombatNotGameOver` (131) → `DAT_005833cc = 0`;
//!   all three are one-shot and are reset at the end of the next battle
//!   (`FUN_004055d0` case 3, 0x405b1f–0x405b31).

/// The one-shot flags a script sets *before* the `Combat` instruction.
///
/// Their defaults are the values the post-battle reset writes back:
/// `+0x1c = 0x7fffffff`, `+0x48 = 0`, `DAT_005833cc = 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingFlags {
    /// `CombatMaxRound`. `0x7fffffff` when unset; the battle treats
    /// anything that large as "no cap".
    pub max_round: i32,
    /// `CombatMustFail`.
    pub must_fail: bool,
    /// `DAT_005833cc`: losing throws the game over. `CombatNotGameOver`
    /// clears it for one battle.
    pub game_over_on_loss: bool,
}

impl Default for PendingFlags {
    fn default() -> Self {
        Self {
            max_round: NO_ROUND_CAP,
            must_fail: false,
            game_over_on_loss: true,
        }
    }
}

/// `_DAT_00c1171c`'s idle value.
pub const NO_ROUND_CAP: i32 = 0x7fff_ffff;

/// The battle descriptor as the entry point `FUN_00407320` sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encounter {
    /// Six enemy slots (`+0x00..+0x14`); 0 = empty. Slot i goes to
    /// battle position 5 + i.
    pub monsters: [u32; 6],
    pub flags: PendingFlags,
    /// `DAT_00c11750`: the 逃跑 entry always fails. Every scripted battle
    /// sets it; field encounters do not.
    pub escape_forbidden: bool,
    /// `FUN_00407320`'s `param_4`: `CombatBoss` keeps the current BGM
    /// instead of switching to the battle theme.
    pub keep_bgm: bool,
    /// `FUN_00407320`'s `param_2` → `+0x44`: 1 = party sneaks up on the
    /// enemy, 2 = the enemy sneaks up on the party, 0 = neither. Scripts
    /// always pass 0.
    pub sneak: u8,
}

impl Encounter {
    /// Opcode 73 `Combat(count, m1, m2, m3)` — `0x437280`.
    pub fn scripted(rng: &mut CrtRand, count: i32, ids: [u32; 3], flags: PendingFlags) -> Self {
        Self {
            monsters: pick_monsters(rng, count, ids, 0),
            flags,
            escape_forbidden: true,
            keep_bgm: false,
            sneak: 0,
        }
    }

    /// Opcode 80 `CombatBoss(m1..m6)` — `0x4373d0`. The six ids are copied
    /// verbatim; nothing is drawn.
    pub fn boss(ids: [u32; 6], flags: PendingFlags) -> Self {
        Self {
            monsters: ids,
            flags,
            escape_forbidden: true,
            keep_bgm: true,
            sneak: 0,
        }
    }

    /// 野外遇敌（`FUN_00417420` → `FUN_00407560`）：`.scn` 角色的三个怪物 id 和个数照 73 号一样抽
    /// （`FUN_018c59e0`，没有 boss）；进之前写死 `DAT_00c11748 = 0`、`DAT_00c11750 = 0`（能逃）、回合不限、
    /// `DAT_005833cc = 1`（输了 game over），脚本先设的那几个标志不管。
    pub fn field(rng: &mut CrtRand, count: i32, ids: [u32; 3], sneak: u8) -> Self {
        Self {
            monsters: pick_monsters(rng, count, ids, 0),
            flags: PendingFlags::default(),
            escape_forbidden: false,
            keep_bgm: false,
            sneak,
        }
    }

    pub fn enemy_count(&self) -> usize {
        self.monsters.iter().filter(|id| **id != 0).count()
    }
}

/// The C runtime's `rand()` (`FUN_005541f8`), which the monster picker
/// calls directly — a separate stream from the combat LCG in
/// `formulas::CombatRng`, though the arithmetic is the same.
#[derive(Debug, Clone)]
pub struct CrtRand {
    seed: u32,
}

impl CrtRand {
    pub fn new(seed: u32) -> Self {
        Self { seed }
    }

    pub fn rand(&mut self) -> i32 {
        self.seed = self.seed.wrapping_mul(0x343F_D).wrapping_add(0x269E_C3);
        ((self.seed >> 16) & 0x7FFF) as i32
    }
}

/// `FUN_018c59e0(count, m1, m2, m3, boss, out)`: fill the six monster
/// slots for a normal encounter.
///
/// The non-zero ids among `m1..m3` are compacted to the front (order
/// kept) and form the pool; then **every** slot up to `count` draws
/// `pool[rand() % n]` independently. Nothing guarantees each id appears,
/// and the same id can fill every slot — Pal3.Unity's "one of each, then
/// shuffle" is not what the binary does.
///
/// With `boss != 0` the count grows by one and the boss takes slot 0,
/// the draws filling slots 1.. after it. Scripted `Combat` always passes
/// 0 here; the boss path is the field's.
///
/// The original writes `out[6] = count` and never clamps; six is the
/// most the descriptor holds, so the count is clamped here and the
/// clamp is the one deliberate departure.
pub fn pick_monsters(rng: &mut CrtRand, count: i32, ids: [u32; 3], boss: u32) -> [u32; 6] {
    let mut out = [0u32; 6];
    let pool: Vec<u32> = ids.iter().copied().filter(|id| *id != 0).collect();

    let mut count = count;
    let mut first = 0usize;
    if boss != 0 {
        count += 1;
        out[0] = boss;
        first = 1;
    }
    let count = count.clamp(0, 6) as usize;
    if pool.is_empty() {
        return out;
    }
    for slot in out.iter_mut().take(count).skip(first) {
        let draw = rng.rand() % pool.len() as i32;
        *slot = pool[draw as usize];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_id_in_the_middle_is_skipped_not_drawn() {
        let mut rng = CrtRand::new(7);
        let picked = pick_monsters(&mut rng, 6, [3001, 0, 3002], 0);
        assert!(picked.iter().all(|id| *id == 3001 || *id == 3002));
        assert_eq!(picked.iter().filter(|id| **id != 0).count(), 6);
    }

    #[test]
    fn a_single_id_fills_every_slot_and_the_rest_stay_empty() {
        let mut rng = CrtRand::new(1);
        assert_eq!(
            pick_monsters(&mut rng, 3, [3001, 0, 0], 0),
            [3001, 3001, 3001, 0, 0, 0]
        );
    }

    #[test]
    fn draws_are_independent_so_one_id_can_dominate() {
        // Walk seeds until a 3-of-3 draw of the same id shows up; with
        // two ids that is a 1-in-4 event, so it must appear quickly.
        let found = (0..64u32).any(|seed| {
            let mut rng = CrtRand::new(seed);
            let picked = pick_monsters(&mut rng, 3, [3001, 3002, 0], 0);
            picked[..3].iter().all(|id| *id == picked[0])
        });
        assert!(found, "每格独立抽取，同一只怪应能占满三格");
    }

    #[test]
    fn a_boss_takes_slot_zero_and_adds_to_the_count() {
        let mut rng = CrtRand::new(3);
        let picked = pick_monsters(&mut rng, 2, [3001, 0, 0], 9001);
        assert_eq!(picked, [9001, 3001, 3001, 0, 0, 0]);
    }

    #[test]
    fn nothing_to_draw_from_leaves_the_slots_empty() {
        let mut rng = CrtRand::new(3);
        assert_eq!(pick_monsters(&mut rng, 3, [0, 0, 0], 0), [0; 6]);
    }

    #[test]
    fn the_count_is_clamped_to_the_descriptor() {
        let mut rng = CrtRand::new(3);
        let picked = pick_monsters(&mut rng, 9, [3001, 0, 0], 0);
        assert_eq!(picked, [3001; 6]);
    }

    #[test]
    fn scripted_battles_forbid_escape_and_bosses_keep_the_music() {
        let mut rng = CrtRand::new(3);
        let normal = Encounter::scripted(&mut rng, 2, [3001, 0, 0], PendingFlags::default());
        assert!(normal.escape_forbidden);
        assert!(!normal.keep_bgm);
        assert_eq!(normal.enemy_count(), 2);

        let boss = Encounter::boss([5001, 5002, 0, 0, 0, 0], PendingFlags::default());
        assert!(boss.escape_forbidden);
        assert!(boss.keep_bgm);
        assert_eq!(boss.monsters[..2], [5001, 5002]);
    }

    #[test]
    fn pending_flags_default_to_the_post_battle_reset_values() {
        let f = PendingFlags::default();
        assert_eq!(f.max_round, NO_ROUND_CAP);
        assert!(!f.must_fail);
        assert!(f.game_over_on_loss);
    }
}
