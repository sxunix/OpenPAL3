//! The static game data PAL3 ships alongside its scenes: the item / skill /
//! combo-skill / combat-actor tables in `cbdata\pal3_softstar.gdb`, and the
//! level→experience curve in `datascript\combat.ini`.
//!
//! Loaded once per run and shared read-only; nothing here changes as the
//! game is played (that lives in `PersistentState`).

use std::collections::HashMap;
use std::io::Cursor;
use std::rc::Rc;

use common::store_ext::StoreExt2;
use encoding::types::Encoding;
use fileformats::pal3::gdb::{CombatActorInfo, ComboSkillInfo, GameItemInfo, GdbFile, SkillInfo};
use mini_fs::MiniFs;

/// The five playable characters, in the order they occupy the head of the
/// combat-actor table (and the order every 5-slot "which actor may use
/// this" array in the database uses).
pub const PLAYER_ACTOR_IDS: [u32; 5] = [0, 1, 2, 3, 4];

/// Highest level the original engine allows.
pub const MAX_LEVEL: i32 = 99;

pub struct GameDb {
    gdb: GdbFile,
    combat_actors_by_id: HashMap<u32, usize>,
    skills_by_id: HashMap<u32, usize>,
    items_by_id: HashMap<u32, usize>,

    /// Cumulative experience needed to *be* level N, indexed by `N - 1`.
    /// From `combat.ini [Level]`; 99 entries.
    level_exp: Vec<i32>,
}

impl GameDb {
    /// Load from a mounted PAL3 VFS. `basedata` is the asset manager's
    /// basedata root (`/basedata/basedata`).
    pub fn load(vfs: &MiniFs, basedata: &std::path::Path) -> Option<Rc<Self>> {
        let gdb_bytes = vfs
            .read_to_end(basedata.join("cbdata").join("pal3_softstar.gdb"))
            .map_err(|e| log::error!("Cannot read pal3_softstar.gdb: {}", e))
            .ok()?;

        let gdb = GdbFile::read(&mut Cursor::new(gdb_bytes))
            .map_err(|e| log::error!("Cannot parse pal3_softstar.gdb: {}", e))
            .ok()?;

        let index = |ids: Vec<u32>| -> HashMap<u32, usize> {
            ids.into_iter().enumerate().map(|(i, id)| (id, i)).collect()
        };

        let combat_actors_by_id = index(gdb.combat_actors.iter().map(|a| a.id).collect());
        let skills_by_id = index(gdb.skills.iter().map(|s| s.id).collect());
        let items_by_id = index(gdb.items.iter().map(|i| i.id).collect());

        let level_exp = Self::load_level_table(vfs, basedata).unwrap_or_else(|| {
            log::error!("Cannot read combat.ini [Level]; experience curve unavailable");
            Vec::new()
        });

        Some(Rc::new(Self {
            gdb,
            combat_actors_by_id,
            skills_by_id,
            items_by_id,
            level_exp,
        }))
    }

    /// `combat.ini` is GBK and its `[Level]` values carry trailing spaces
    /// and `;` comments, so it is parsed by hand rather than through the
    /// ini crate's stricter reader.
    fn load_level_table(vfs: &MiniFs, basedata: &std::path::Path) -> Option<Vec<i32>> {
        let bytes = vfs
            .read_to_end(basedata.join("datascript").join("combat.ini"))
            .ok()?;
        let text = encoding::all::GBK
            .decode(&bytes, encoding::DecoderTrap::Ignore)
            .ok()?;

        let mut table = vec![0i32; MAX_LEVEL as usize];
        let mut in_level_section = false;
        let mut seen = 0;

        for line in text.lines() {
            let line = line.split(';').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') {
                in_level_section = line.eq_ignore_ascii_case("[Level]");
                continue;
            }
            if !in_level_section {
                continue;
            }
            let Some((level, exp)) = line.split_once('=') else {
                continue;
            };
            let (Ok(level), Ok(exp)) = (level.trim().parse::<i32>(), exp.trim().parse::<i32>())
            else {
                continue;
            };
            if (1..=MAX_LEVEL).contains(&level) {
                table[(level - 1) as usize] = exp;
                seen += 1;
            }
        }

        (seen == MAX_LEVEL).then_some(table)
    }

    pub fn combat_actor(&self, id: u32) -> Option<&CombatActorInfo> {
        self.combat_actors_by_id
            .get(&id)
            .map(|i| &self.gdb.combat_actors[*i])
    }

    pub fn skill(&self, id: u32) -> Option<&SkillInfo> {
        self.skills_by_id.get(&id).map(|i| &self.gdb.skills[*i])
    }

    pub fn item(&self, id: u32) -> Option<&GameItemInfo> {
        self.items_by_id.get(&id).map(|i| &self.gdb.items[*i])
    }

    pub fn combo_skills(&self) -> &[ComboSkillInfo] {
        &self.gdb.combo_skills
    }

    /// Total experience required to reach `level`. Level 1 is 0.
    pub fn exp_for_level(&self, level: i32) -> i32 {
        if self.level_exp.is_empty() || level < 1 {
            return 0;
        }
        let level = level.min(MAX_LEVEL);
        self.level_exp[(level - 1) as usize]
    }

    /// Experience still needed to reach the next level, clamped at zero.
    /// `None` at level 99, where the original shows a dash instead.
    pub fn exp_to_next_level(&self, level: i32, exp: i32) -> Option<i32> {
        if level >= MAX_LEVEL {
            return None;
        }
        Some((self.exp_for_level(level + 1) - exp).max(0))
    }

    /// Level implied by a raw experience total (used when a script grants
    /// experience directly).
    pub fn level_for_exp(&self, exp: i32) -> i32 {
        let mut level = 1;
        while level < MAX_LEVEL && exp >= self.exp_for_level(level + 1) {
            level += 1;
        }
        level
    }
}
