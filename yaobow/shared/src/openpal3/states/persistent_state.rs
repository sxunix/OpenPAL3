use radiance::math::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use crate::ydirs;

use super::game_db::{BASE_FAVOR, FAVOR_ACTOR_IDS, GameDb, MAX_LEVEL, PLAYER_ACTOR_IDS};

pub const PAL3_APP_NAME: &str = "OpenPAL3";

/// Equipment slots, in the order the original status page lists them.
pub const EQUIPMENT_SLOTS: usize = 5;

/// One playable character's mutable state.
///
/// The level-1 values come straight from `pal3_softstar.gdb`; growth per
/// level is applied by the (not yet reversed) level-up path, so for now a
/// character keeps the stats it was seeded or scripted with.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RoleState {
    pub level: i32,
    pub exp: i32,

    /// 精 — the original's "HP".
    pub hp: i32,
    pub max_hp: i32,
    /// 气 — spent by weapon skills.
    pub sp: i32,
    pub max_sp: i32,
    /// 神 — spent by magic.
    pub mp: i32,
    pub max_mp: i32,

    /// 武.
    pub attack: i32,
    /// 防.
    pub defense: i32,
    /// 速.
    pub speed: i32,
    /// 运.
    pub luck: i32,

    /// 五灵: 水火风雷土.
    pub elements: [i32; 5],

    /// Learned skill ids, in acquisition order.
    pub skills: Vec<u32>,

    /// Item id per slot (0 = empty): 武器/衣服/帽子/鞋子/饰品.
    pub equipment: [u32; EQUIPMENT_SLOTS],
}

impl RoleState {
    /// Seed from the character's level-1 row in the game database.
    pub fn from_db(db: &GameDb, actor_id: u32) -> Option<Self> {
        let a = db.combat_actor(actor_id)?;
        let attr = &a.attribute_values;
        Some(Self {
            level: a.level.clamp(1, MAX_LEVEL),
            exp: db.exp_for_level(a.level),
            hp: attr[0],
            max_hp: attr[0],
            sp: attr[1],
            max_sp: attr[1],
            mp: attr[2],
            max_mp: attr[2],
            attack: attr[3],
            defense: attr[4],
            speed: attr[5],
            luck: attr[6],
            elements: a.element_attribute_values,
            skills: a.skill_ids.iter().copied().filter(|id| *id != 0).collect(),
            equipment: [0; EQUIPMENT_SLOTS],
        })
    }
}

#[derive(Serialize, Deserialize)]
pub struct PersistentState {
    app_name: String,
    global_vars: HashMap<i16, i32>,
    position: Vec3,
    scene: Option<String>,
    sub_scene: Option<String>,

    // Everything below is `#[serde(default)]` so saves written before the
    // gameplay state existed still load.
    /// Per playable character, keyed by actor id.
    #[serde(default)]
    roles: HashMap<u32, RoleState>,
    /// Actor ids currently in the party, leader first.
    #[serde(default)]
    team: Vec<u32>,
    /// Item id → count. Ordered so saves diff cleanly.
    #[serde(default)]
    inventory: BTreeMap<u32, u32>,
    #[serde(default)]
    money: i32,
    /// Favour towards each playable character, keyed by actor id.
    #[serde(default)]
    favor: HashMap<u32, i32>,
    /// Elapsed play time in seconds; shown on the status page.
    #[serde(default)]
    play_time: f32,
}

impl PersistentState {
    pub fn new(app_name: String) -> Self {
        Self {
            app_name,
            global_vars: HashMap::new(),
            position: Vec3::new(0., 0., 0.),
            scene: None,
            sub_scene: None,
            roles: HashMap::new(),
            team: Vec::new(),
            inventory: BTreeMap::new(),
            money: 0,
            favor: HashMap::new(),
            play_time: 0.,
        }
    }

    /// Fill in the party from the game database if it hasn't been set up
    /// yet — a new game, or a save written before the gameplay state
    /// existed. Never overwrites state that is already there.
    pub fn seed_from_db(&mut self, db: &GameDb) {
        for actor_id in PLAYER_ACTOR_IDS {
            if let Some(role) = RoleState::from_db(db, actor_id) {
                self.roles.entry(actor_id).or_insert(role);
            }
        }

        if self.team.is_empty() {
            // 景天 starts out alone; the rest join through AddTeam.
            self.team.push(PLAYER_ACTOR_IDS[0]);
        }
    }

    fn get_data_dir(app_name: &str) -> PathBuf {
        ydirs::save_dir().join(app_name)
    }

    /// Whether a save exists in `slot`. `load` panics on a missing or
    /// unreadable file, so callers that offer "resume if present" must
    /// check here first.
    pub fn slot_exists(app_name: &str, slot: i32) -> bool {
        Self::get_data_dir(app_name)
            .join("Save")
            .join(format!("{}.json", slot))
            .is_file()
    }

    pub fn load(app_name: &str, slot: i32) -> Self {
        let path = Self::get_data_dir(app_name)
            .join("Save")
            .join(format!("{}.json", slot));
        let content = std::fs::read_to_string(path).unwrap();
        serde_json::from_str(&content).unwrap()
    }

    pub fn save(&self, slot: i32) {
        if slot >= 0 {
            let path = Self::get_data_dir(&self.app_name).join("Save");
            if let Err(e) = std::fs::create_dir_all(&path) {
                log::error!("Cannot create save dir: {}", e);
                return;
            }

            let result = serde_json::to_string_pretty(self);
            match result {
                Ok(content) => {
                    if let Err(e) = std::fs::write(path.join(format!("{}.json", slot)), content) {
                        log::error!("Cannot save: {}", e);
                    } else {
                        log::info!("Game saved");
                    }
                }
                Err(e) => log::error!("Cannot serialize persistent state: {}", e),
            };
        }
    }

    pub fn app_name(&self) -> &str {
        self.app_name.as_str()
    }

    pub fn set_global(&mut self, var: i16, value: i32) {
        self.global_vars.insert(var, value);
    }

    pub fn get_global(&self, var: i16) -> Option<i32> {
        self.global_vars.get(&var).and_then(|v| Some(*v))
    }

    pub fn position(&mut self) -> Vec3 {
        self.position
    }

    pub fn set_position(&mut self, position: Vec3) {
        self.position = position;
    }

    pub fn set_scene_name(&mut self, scene: String, sub_scene: String) {
        self.scene = Some(scene);
        self.sub_scene = Some(sub_scene);
    }

    pub fn scene_name(&self) -> Option<String> {
        self.scene.clone()
    }

    pub fn sub_scene_name(&self) -> Option<String> {
        self.sub_scene.clone()
    }

    // ---- party ----

    pub fn role(&self, actor_id: u32) -> Option<&RoleState> {
        self.roles.get(&actor_id)
    }

    pub fn role_mut(&mut self, actor_id: u32) -> Option<&mut RoleState> {
        self.roles.get_mut(&actor_id)
    }

    /// Actor ids in the party, leader first.
    pub fn team(&self) -> &[u32] {
        &self.team
    }

    pub fn is_in_team(&self, actor_id: u32) -> bool {
        self.team.contains(&actor_id)
    }

    /// Appends to the party if not already in it. The original caps the
    /// active party at five.
    pub fn add_to_team(&mut self, actor_id: u32) {
        if !self.team.contains(&actor_id) && self.team.len() < PLAYER_ACTOR_IDS.len() {
            self.team.push(actor_id);
        }
    }

    pub fn remove_from_team(&mut self, actor_id: u32) {
        self.team.retain(|id| *id != actor_id);
    }

    // ---- inventory ----

    /// Item id → count, ascending by id.
    pub fn inventory(&self) -> &BTreeMap<u32, u32> {
        &self.inventory
    }

    pub fn item_count(&self, item_id: u32) -> u32 {
        self.inventory.get(&item_id).copied().unwrap_or(0)
    }

    pub fn add_item(&mut self, item_id: u32, count: u32) {
        if count == 0 {
            return;
        }
        *self.inventory.entry(item_id).or_insert(0) += count;
    }

    /// Removes up to `count`; returns how many were actually removed.
    pub fn remove_item(&mut self, item_id: u32, count: u32) -> u32 {
        let Some(held) = self.inventory.get_mut(&item_id) else {
            return 0;
        };
        let removed = count.min(*held);
        *held -= removed;
        if *held == 0 {
            self.inventory.remove(&item_id);
        }
        removed
    }

    // ---- money, favour, clock ----

    pub fn money(&self) -> i32 {
        self.money
    }

    /// Adds (or, with a negative delta, spends) money. The original clamps
    /// at zero and at 999999.
    pub fn add_money(&mut self, delta: i32) {
        self.money = (self.money + delta).clamp(0, 999999);
    }

    /// Favour towards `actor_id`, including the base every tracked
    /// character starts with.
    pub fn favor(&self, actor_id: u32) -> i32 {
        self.favor.get(&actor_id).copied().unwrap_or(0) + BASE_FAVOR
    }

    /// Only the female companions are scored; a script aiming favour at
    /// anyone else is a no-op (and worth knowing about).
    pub fn add_favor(&mut self, actor_id: u32, delta: i32) {
        if !FAVOR_ACTOR_IDS.contains(&actor_id) {
            log::warn!(
                "FavorAdd: actor {} is not part of the favour system",
                actor_id
            );
            return;
        }
        *self.favor.entry(actor_id).or_insert(0) += delta;
    }

    /// The companion with the highest favour — the original's
    /// `GetFavorite`, used to pick endings and a few dialogue branches.
    pub fn favorite(&self) -> u32 {
        self.favor_extreme(true)
    }

    /// The companion with the lowest favour.
    pub fn least_favorite(&self) -> u32 {
        self.favor_extreme(false)
    }

    fn favor_extreme(&self, most: bool) -> u32 {
        let mut best = FAVOR_ACTOR_IDS[0];
        for id in FAVOR_ACTOR_IDS {
            let better = if most {
                self.favor(id) > self.favor(best)
            } else {
                self.favor(id) < self.favor(best)
            };
            if better {
                best = id;
            }
        }
        best
    }

    pub fn play_time(&self) -> f32 {
        self.play_time
    }

    pub fn advance_play_time(&mut self, delta_sec: f32) {
        self.play_time += delta_sec;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saves written before the gameplay fields existed must still load.
    #[test]
    fn loads_legacy_save_without_gameplay_fields() {
        let json = r#"{
            "app_name": "OpenPAL3",
            "global_vars": { "-32768": 10200 },
            "position": { "x": 22.28, "y": 0.12, "z": -82.72 },
            "scene": "q01",
            "sub_scene": "yn09a"
        }"#;

        let state: PersistentState = serde_json::from_str(json).unwrap();
        assert_eq!(state.get_global(-32768), Some(10200));
        assert_eq!(state.scene_name().as_deref(), Some("q01"));
        assert!(state.team().is_empty());
        assert_eq!(state.money(), 0);
        assert_eq!(state.item_count(1), 0);
    }

    #[test]
    fn inventory_and_money_clamp() {
        let mut state = PersistentState::new(PAL3_APP_NAME.to_string());

        state.add_item(80009, 2);
        state.add_item(80009, 1);
        assert_eq!(state.item_count(80009), 3);
        assert_eq!(state.remove_item(80009, 5), 3);
        assert_eq!(state.item_count(80009), 0);
        assert!(state.inventory().is_empty());

        state.add_money(100);
        state.add_money(-500);
        assert_eq!(state.money(), 0, "money never goes negative");
        state.add_money(2_000_000);
        assert_eq!(state.money(), 999999);
    }

    #[test]
    fn favor_is_scored_for_companions_only() {
        let mut state = PersistentState::new(PAL3_APP_NAME.to_string());

        // Everyone reads back the base value to begin with.
        assert_eq!(state.favor(1), BASE_FAVOR);
        assert_eq!(state.favorite(), FAVOR_ACTOR_IDS[0]);

        state.add_favor(3, 15); // 紫萱
        state.add_favor(1, 5); // 雪见
        assert_eq!(state.favor(3), BASE_FAVOR + 15);
        assert_eq!(state.favorite(), 3);
        assert_eq!(state.least_favorite(), 2, "龙葵 is still untouched");

        // 景天 is not part of the favour system.
        state.add_favor(0, 100);
        assert_eq!(state.favor(0), BASE_FAVOR);
    }

    #[test]
    fn team_membership() {
        let mut state = PersistentState::new(PAL3_APP_NAME.to_string());
        state.add_to_team(0);
        state.add_to_team(1);
        state.add_to_team(1);
        assert_eq!(state.team(), &[0, 1]);
        assert!(state.is_in_team(1));
        state.remove_from_team(1);
        assert!(!state.is_in_team(1));
    }
}
