//! The static game data PAL3 ships alongside its scenes: the item / skill /
//! combo-skill / combat-actor tables in `cbdata\pal3_softstar.gdb`, and the
//! level→experience curve in `datascript\combat.ini`.
//!
//! Loaded once per run and shared read-only; nothing here changes as the
//! game is played (that lives in `PersistentState`).

use std::collections::HashMap;
use std::io::Cursor;
use std::rc::Rc;

use super::combat_data::CombatData;
use common::store_ext::StoreExt2;
use encoding::types::Encoding;
use fileformats::pal3::gdb::{CombatActorInfo, ComboSkillInfo, GameItemInfo, GdbFile, SkillInfo};
use mini_fs::MiniFs;

/// The five playable characters as the adventure side numbers them —
/// SCE scripts, the party list and every 5-slot "which actor may use
/// this" array in the database all use these.
pub const PLAYER_ACTOR_IDS: [u32; 5] = [0, 1, 2, 3, 4];

/// The same five characters as the **combat-actor table** numbers them.
/// The two numbering schemes are unrelated: 景天 is party 0 but combat
/// actor 101, 长卿 is party 4 but combat actor 102. The combat code's own
/// id→slot table (0x65→0, 0x66→4, 0x68→1, 0x69/0x6a→2, 0x6b→3) is what
/// ties them together.
pub const PLAYER_COMBAT_IDS: [u32; 5] = [101, 104, 106, 107, 102];

/// Party index → its row in the combat-actor table.
pub fn combat_id_of_party_member(party_index: u32) -> Option<u32> {
    PLAYER_COMBAT_IDS.get(party_index as usize).copied()
}

/// The inverse: combat-actor id → party index.
pub fn party_member_of_combat_id(combat_id: u32) -> Option<u32> {
    PLAYER_COMBAT_IDS
        .iter()
        .position(|id| *id == combat_id)
        .map(|i| i as u32)
}

/// Highest level the original engine allows.
pub const MAX_LEVEL: i32 = 99;

/// The characters the favour (好感) system tracks. By the original's
/// design only the female companions have favour: 雪见 1, 龙葵 2,
/// 紫萱 3, 花楹 5 — 景天 and 长卿 are not scored.
pub const FAVOR_ACTOR_IDS: [u32; 4] = [1, 2, 3, 5];

/// Favour every tracked character starts with; scripts add and subtract
/// from it. Taken from Pal3.Unity's favour system rather than read out of
/// the executable, so treat the exact number as unverified.
pub const BASE_FAVOR: i32 = 20;

pub struct GameDb {
    gdb: GdbFile,
    combat_actors_by_id: HashMap<u32, usize>,
    skills_by_id: HashMap<u32, usize>,
    items_by_id: HashMap<u32, usize>,

    /// Cumulative experience needed to *be* level N, indexed by `N - 1`.
    /// From `combat.ini [Level]`; 99 entries.
    level_exp: Vec<i32>,

    /// Stage placement and the battlefield lookup, from `combat.ini` and
    /// `combatScn.txt`.
    combat: CombatData,
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

        let combat_ini = Self::read_gbk(vfs, &basedata.join("datascript").join("combat.ini"));
        let combat_scn = Self::read_gbk(vfs, &basedata.join("datascript").join("combatScn.txt"));
        let level_exp = Self::parse_level_table(&combat_ini).unwrap_or_else(|| {
            log::error!("Cannot read combat.ini [Level]; experience curve unavailable");
            Vec::new()
        });
        let combat = CombatData::parse(&combat_ini, &combat_scn);

        Some(Rc::new(Self::from_parts(gdb, level_exp, combat)))
    }

    fn from_parts(gdb: GdbFile, level_exp: Vec<i32>, combat: CombatData) -> Self {
        let index = |ids: Vec<u32>| -> HashMap<u32, usize> {
            ids.into_iter().enumerate().map(|(i, id)| (id, i)).collect()
        };
        let combat_actors_by_id = index(gdb.combat_actors.iter().map(|a| a.id).collect());
        let skills_by_id = index(gdb.skills.iter().map(|s| s.id).collect());
        let items_by_id = index(gdb.items.iter().map(|i| i.id).collect());
        Self {
            gdb,
            combat_actors_by_id,
            skills_by_id,
            items_by_id,
            level_exp,
            combat,
        }
    }

    /// A database holding only the given item and skill rows — no
    /// experience curve, no stage data. For tests and tools that exercise
    /// item logic without a mounted game.
    pub fn from_tables(items: Vec<GameItemInfo>, skills: Vec<SkillInfo>) -> Self {
        let gdb = GdbFile {
            items,
            skills,
            ..Default::default()
        };
        Self::from_parts(gdb, Vec::new(), CombatData::default())
    }

    /// Read a GBK text file, or an empty string when it is missing.
    fn read_gbk(vfs: &MiniFs, path: &std::path::Path) -> String {
        vfs.read_to_end(path)
            .ok()
            .and_then(|bytes| {
                encoding::all::GBK
                    .decode(&bytes, encoding::DecoderTrap::Ignore)
                    .ok()
            })
            .unwrap_or_else(|| {
                log::error!("Cannot read {}", path.display());
                String::new()
            })
    }

    /// `combat.ini`'s `[Level]` values carry trailing spaces and `;`
    /// comments, so they are parsed by hand rather than through the ini
    /// crate's stricter reader.
    fn parse_level_table(text: &str) -> Option<Vec<i32>> {
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

    /// Stage placement and the battlefield lookup.
    pub fn combat(&self) -> &CombatData {
        &self.combat
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

    /// 整张物品表，顺序就是数据库里的顺序——当铺按比例随机摸死当物时
    /// （`source.txt` 的 `GDB_GetItemID(rand()%item_num)`）要的就是它。
    pub fn items(&self) -> &[GameItemInfo] {
        &self.gdb.items
    }

    /// Every combat-actor row, playable characters first then monsters.
    pub fn combat_actors(&self) -> &[CombatActorInfo] {
        &self.gdb.combat_actors
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
