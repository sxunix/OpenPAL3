use radiance::math::Vec3;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;

use crate::openpal3::combat::ai::Delegation;
use crate::ydirs;

use super::game_db::{
    BASE_FAVOR, FAVOR_ACTOR_IDS, GameDb, MAX_LEVEL, PLAYER_ACTOR_IDS, combat_id_of_party_member,
};
use super::growth;
use super::skills::{LearnedSkill, levelled_up_to};

pub const PAL3_APP_NAME: &str = "OpenPAL3";

/// A character wears six pieces, not five — 饰品 has two slots. The order
/// is the original role struct's (`role + 0x354`, 12 bytes per entry).
pub const EQUIPMENT_SLOTS: usize = 6;

/// 阵法有六个站位：五灵五个加盘心。
pub const LINEUP_POSITIONS: u8 = 6;

/// 新游戏的阵形（`FUN_00408fd0` 写角色记录的 `+0x29a`）：景天 3 风、雪见 5 土、龙葵 1 水、
/// 紫萱 2 火、长卿 4 雷。这里存格子号，也就是值 − 1（阵法页写回时 `FUN_004845f0` 是格子号 + 1）。
pub const DEFAULT_LINEUP: [(u32, u8); 5] = [(0, 2), (1, 4), (2, 0), (3, 1), (4, 3)];

pub const SLOT_HAT: usize = 0;
pub const SLOT_CLOTH: usize = 1;
pub const SLOT_SHOES: usize = 2;
pub const SLOT_ADORN1: usize = 3;
pub const SLOT_ADORN2: usize = 4;
pub const SLOT_WEAPON: usize = 5;

/// Which slots an item type may go in, in preference order. Empty when the
/// item is not equipment. 饰品 (`ItemType::Wearable`) is the only type with
/// two slots.
pub fn equipment_slots_for_type(item_type: u8) -> &'static [usize] {
    match item_type {
        7 => &[SLOT_CLOTH],
        8 => &[SLOT_HAT],
        9 => &[SLOT_SHOES],
        10 => &[SLOT_ADORN1, SLOT_ADORN2],
        11 => &[SLOT_WEAPON],
        _ => &[],
    }
}

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

    /// Learned skills, in acquisition order, each with its own level and
    /// use count — the original tracks all three per skill slot.
    pub skills: Vec<LearnedSkill>,

    /// Item id per slot, 0 = empty:
    /// 0 帽 · 1 衣 · 2 鞋 · 3 饰品一 · 4 饰品二 · 5 武器
    /// (see [`equipment_slots_for_type`]).
    pub equipment: [u32; EQUIPMENT_SLOTS],

    /// 冒险侧的 31 个状态字节——原版角色记录 `+0x54` 起，下标是
    /// `ActorCombatStateType`（0–4 五毒 … 27 辟邪 28 退魔 29 神眼 30 精进）。
    /// 战斗外用道具时 `FUN_0049cf80` 直接往这里写（见
    /// `switch/re/notes/item_use.md` §1.4）。
    #[serde(default = "zero_states")]
    pub states: [u8; STATE_BYTES],

    /// 战斗托管的六个开关（冒险记录 `+0x294..+0x299`），开战拷进战斗位、战后写回。
    /// 新档只有「谨慎」是开的（0x40bdbf）。
    #[serde(default)]
    pub delegation: Delegation,
}

/// 角色记录里的状态字节数（`COMBAT_STATE_COUNT`）。
pub const STATE_BYTES: usize = 31;

fn zero_states() -> [u8; STATE_BYTES] {
    [0; STATE_BYTES]
}

impl RoleState {
    /// Seed a character at its starting level.
    ///
    /// 精/气/神/武/防 come from the level curves, not from the database
    /// row: those columns are the curves' level-0 intercepts, so reading
    /// them directly would leave everyone one level's growth short (景天
    /// would start on 精 80 instead of the original's 88). 速 and 运 have
    /// no curve and are taken from the database as-is.
    pub fn from_db(db: &GameDb, actor_id: u32) -> Option<Self> {
        // `actor_id` is the party index; the combat-actor table is keyed
        // by a different set of numbers entirely.
        let a = db.combat_actor(combat_id_of_party_member(actor_id)?)?;
        let attr = &a.attribute_values;
        let level = a.level.clamp(1, MAX_LEVEL);
        let g = growth::at_level(actor_id as usize, level);
        Some(Self {
            level,
            exp: db.exp_for_level(level),
            hp: g.max_hp,
            max_hp: g.max_hp,
            sp: g.max_sp,
            max_sp: g.max_sp,
            mp: g.max_mp,
            max_mp: g.max_mp,
            attack: g.attack,
            defense: g.defense,
            speed: attr[5],
            luck: attr[6],
            elements: a.element_attribute_values,
            skills: a
                .skill_ids
                .iter()
                .copied()
                .filter(|id| *id != 0)
                .map(LearnedSkill::new)
                .collect(),
            equipment: [0; EQUIPMENT_SLOTS],
            states: [0; STATE_BYTES],
            delegation: Delegation::default(),
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
    /// 阵法: which of the six formation positions (0..5 = 水火风雷土 + the
    /// middle of the disc) each character stands in. A position holds at
    /// most one character.
    #[serde(default)]
    lineup: HashMap<u32, u8>,
    /// Combat actors recorded in the 侠义榜. Populated when a monster is
    /// fought, so it stays empty until combat exists.
    #[serde(default)]
    seen_monsters: BTreeSet<u32>,
    /// Which menu pages the story has unlocked. The original keeps these
    /// as three bytes next to each other — `DAT_00de5f92` 魔剑养成 (opcode
    /// 109), `DAT_00de5f93` 降妖谱 (110), `DAT_00de625a` 魔剑技 (138) — all
    /// three cleared on a new game and written to the save file.
    #[serde(default)]
    features: BTreeSet<Feature>,
    /// 龙葵 blue (0) or red (1) — opcode 42. Saved by the original as part
    /// of the actor's model name; kept here because reloading a scene
    /// rebuilds ROLE_2 from the default table.
    #[serde(default)]
    longkui_mode: i32,
    /// 花盈's behaviour — opcode 88: 0 隐藏, 1 跟随雪见, 2 单飞. The original
    /// keeps it in `DAT_00c10ccc` and writes it to the save file.
    #[serde(default)]
    huaying_mode: i32,
    /// 合成术 recipes the party has learnt — opcode 111.
    #[serde(default)]
    compose_recipes: BTreeSet<u32>,
    /// 山洞之狼（SCE 112）赢过几次。原版是 `DAT_00bfda44`，前四次各发一件
    /// 奖品，之后就只有对白。
    #[serde(default)]
    cave_wolf_wins: u32,
    /// 当铺累计投进去的资本。原版存在当铺状态块里（`DAT_00d92788` 那一坨的
    /// `+0x1488`），等级是拿它算出来的，见
    /// [`super::hockshop::level_for_capital`]。
    #[serde(default)]
    hockshop_capital: i32,
    /// 当铺的四个经营选择：经营方针 / 当期 / 利率 / 拆当率。原版在状态块的
    /// `+0x27844` / `+0x27838` / `+0x2783c` / `+0x27840`，SCE 120–123 读的
    /// 就是它们。
    #[serde(default)]
    hockshop_mode: i32,
    #[serde(default)]
    hockshop_period: i32,
    #[serde(default)]
    hockshop_accrual: i32,
    #[serde(default)]
    hockshop_unhock: i32,
    /// 死当物池：四种经营方针各一池，每件（物品 id，卖掉没有）。原版在经营对象 `+0x1f74`
    /// 起（每件 0x328 字节：物品记录，`+0x320` 卖掉、`+0x321` 这次摆出来），当铺升级时
    /// （增资页 `+0xf1c` 置位 → `FUN_0051f1b0` 0x5210ce）按这一级的 `PropNum` 重新摸，随存档保存。
    #[serde(default)]
    hockshop_pools: Vec<Vec<(u32, bool)>>,
    /// Per-scene-object state the story writes and later reads back —
    /// mazes remember which switches have been thrown. The original
    /// keeps a table in the save-state store at `DAT_017b1550`, looked up
    /// by `FUN_00539540(city, scene, objectId)`; the switch byte is the
    /// record's `+3`. Keyed here by [`scene_object_key`] because JSON
    /// object keys have to be strings.
    #[serde(default)]
    scene_object_states: BTreeMap<String, i32>,
    /// Objects that must be built from the scene file rather than from
    /// saved state — opcode 158 (`FUN_00539e70`). The original keeps one
    /// list for the whole store, not one per scene.
    #[serde(default)]
    objects_not_from_save: BTreeSet<i32>,
    /// 物件在不在场（脚本 85 ObjectActive 改的 +0xad）。原版把它写进存档里那条物件记录，
    /// 换场景回来照样生效（`FUN_00538fa0` 存、建物件时取）。
    #[serde(default)]
    scene_object_activations: BTreeMap<String, bool>,
    /// 物件在第几层（+0xf6）。上下传送板把自己挪到另一层之后要记住。
    #[serde(default)]
    scene_object_layers: BTreeMap<String, u8>,
    /// Where opcode 90 has slid a scene object to. `LAB_004378e0` sets
    /// the object's "position was modified" flag at `+0x244` after the
    /// move, except in the nine scenes listed at 0x437997–0x437a67.
    #[serde(default)]
    scene_object_positions: BTreeMap<String, [f32; 3]>,
    /// 大地图上每个目的地的状态——指令 125 写的那张表。原版是
    /// `i32 状态[38] @0xdba014` 加 `u8 可见[38] @0xdba0ac`，后者恒等于
    /// `状态 > 0`；绘制只看可见位，i32 那份只进存档，所以这里只留一份。
    /// 见 `switch/re/notes/bigmap.md`。
    #[serde(default)]
    bigmap_elements: BTreeMap<i32, i32>,
    /// 125 新点亮的目的地（原版 `u8 [0xdba0ac + id] = option > 0`）：下次进大地图
    /// 闪两轮后清掉。原版存读档只存到 0xdba0ac 之前那张 i32 表，这个不进存档。
    #[serde(skip)]
    bigmap_new: std::collections::BTreeSet<i32>,
    /// 紫幽粉在身——`DAT_00c10cc1`，`FUN_00410580(0x1f)` 置 1。和角色 0 的
    /// 状态 27..30 一样受下面那个 60 秒计时器管。
    #[serde(default)]
    purple_powder: bool,
    /// 迷宫仙宝的计时器 `DAT_00c10cd0`：每次用仙宝清零，累计到 60 秒把
    /// 角色 0 的状态 27..30（辟邪/退魔/神眼/精进）和紫幽粉一起清掉
    /// （decompiled.c ~8323）。
    #[serde(default)]
    maze_effect_timer: f32,
    /// 五灵珠的充能 `DAT_00bfdb70..77`：新游戏全 3，战斗里用一次扣一格，
    /// 物品本身不消耗。下标见 [`pearl_charge_index`]。
    #[serde(default = "full_pearl_charges")]
    pearl_charges: [u8; PEARL_SLOTS],
}

/// 五灵珠充能表的格数。原版存 8 字节，只用到 7 格。
pub const PEARL_SLOTS: usize = 8;
/// 新游戏时每颗珠子的充能。
pub const PEARL_FULL_CHARGE: u8 = 3;
/// 迷宫仙宝效果的持续时间（秒）。
pub const MAZE_EFFECT_SECONDS: f32 = 60.0;

fn full_pearl_charges() -> [u8; PEARL_SLOTS] {
    [PEARL_FULL_CHARGE; PEARL_SLOTS]
}

/// 五灵珠物品号 → 充能表下标（`FUN_004f7140` 的 switch，基址 `DAT_00bfdb70`）：
/// 0x17e7→+3、0x17e8→+4、0x17e9→+1、0x17ea→+2、0x17eb→+5、0x17ec→+7、
/// 0x17ed→+6。不是五灵珠返回 `None`。
pub fn pearl_charge_index(item_id: u32) -> Option<usize> {
    Some(match item_id {
        0x17e7 => 3,
        0x17e8 => 4,
        0x17e9 => 1,
        0x17ea => 2,
        0x17eb => 5,
        0x17ec => 7,
        0x17ed => 6,
        _ => return None,
    })
}

/// `city/scene/objectId` — `city` is the scene's cpk name (the original's
/// `scene+0x00`) and `scene` the block within it (`scene+0x20`).
pub fn scene_object_key(city: &str, scene: &str, object_id: i32) -> String {
    format!(
        "{}/{}/{}",
        city.to_lowercase(),
        scene.to_lowercase(),
        object_id
    )
}

/// A menu page the story unlocks part-way through.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum Feature {
    /// 魔剑养成 — `DAT_00de5f92`.
    SwordPurifying,
    /// 降妖谱 — `DAT_00de5f93`.
    JusticeBook,
    /// 魔剑技 — `DAT_00de625a`.
    SwordSkill,
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
            lineup: DEFAULT_LINEUP.into_iter().collect(),
            seen_monsters: BTreeSet::new(),
            features: BTreeSet::new(),
            longkui_mode: 0,
            huaying_mode: 0,
            compose_recipes: BTreeSet::new(),
            cave_wolf_wins: 0,
            hockshop_capital: 0,
            hockshop_mode: 0,
            hockshop_period: 0,
            hockshop_accrual: 0,
            hockshop_unhock: 0,
            hockshop_pools: Vec::new(),
            scene_object_states: BTreeMap::new(),
            objects_not_from_save: BTreeSet::new(),
            scene_object_positions: BTreeMap::new(),
            scene_object_activations: BTreeMap::new(),
            scene_object_layers: BTreeMap::new(),
            bigmap_elements: BTreeMap::new(),
            bigmap_new: Default::default(),
            purple_powder: false,
            maze_effect_timer: 0.,
            pearl_charges: full_pearl_charges(),
        }
    }

    /// A scene object's switch state, or 0 when the story has not
    /// touched it. The original returns the raw byte, not a bool.
    /// 指令 125 给目的地 `id` 设的状态。没设过的算 0（不显示）。
    pub fn bigmap_element(&self, id: i32) -> i32 {
        self.bigmap_elements.get(&id).copied().unwrap_or(0)
    }

    pub fn set_bigmap_element(&mut self, id: i32, option: i32) {
        self.bigmap_elements.insert(id, option);
        if option > 0 {
            self.bigmap_new.insert(id);
        } else {
            self.bigmap_new.remove(&id);
        }
    }

    /// 目的地 `id` 画不画在地图上：状态 1、2 画，0 不画（`FUN_004499a0`）。
    pub fn bigmap_element_visible(&self, id: i32) -> bool {
        matches!(self.bigmap_element(id), 1 | 2)
    }

    /// 目的地 `id` 能不能去：只有状态 2 能；1 点了是「不能去“地名”」。
    pub fn bigmap_element_enterable(&self, id: i32) -> bool {
        self.bigmap_element(id) == 2
    }

    /// 取走「新点亮、要闪」的那几个（大地图进场时读一次就清掉）。
    pub fn take_bigmap_new(&mut self) -> Vec<i32> {
        std::mem::take(&mut self.bigmap_new).into_iter().collect()
    }

    pub fn scene_object_state(&self, city: &str, scene: &str, object_id: i32) -> i32 {
        self.scene_object_states
            .get(&scene_object_key(city, scene, object_id))
            .copied()
            .unwrap_or(0)
    }

    pub fn set_scene_object_state(&mut self, city: &str, scene: &str, object_id: i32, state: i32) {
        self.scene_object_states
            .insert(scene_object_key(city, scene, object_id), state);
    }

    pub fn scene_object_position(&self, city: &str, scene: &str, object_id: i32) -> Option<Vec3> {
        self.scene_object_positions
            .get(&scene_object_key(city, scene, object_id))
            .map(|p| Vec3::new(p[0], p[1], p[2]))
    }

    pub fn set_scene_object_position(
        &mut self,
        city: &str,
        scene: &str,
        object_id: i32,
        position: Vec3,
    ) {
        self.scene_object_positions.insert(
            scene_object_key(city, scene, object_id),
            [position.x, position.y, position.z],
        );
    }

    pub fn scene_object_active(&self, city: &str, scene: &str, object_id: i32) -> Option<bool> {
        self.scene_object_activations
            .get(&scene_object_key(city, scene, object_id))
            .copied()
    }

    pub fn set_scene_object_active(
        &mut self,
        city: &str,
        scene: &str,
        object_id: i32,
        active: bool,
    ) {
        self.scene_object_activations
            .insert(scene_object_key(city, scene, object_id), active);
    }

    pub fn scene_object_layer(&self, city: &str, scene: &str, object_id: i32) -> Option<u8> {
        self.scene_object_layers
            .get(&scene_object_key(city, scene, object_id))
            .copied()
    }

    pub fn set_scene_object_layer(&mut self, city: &str, scene: &str, object_id: i32, layer: u8) {
        self.scene_object_layers
            .insert(scene_object_key(city, scene, object_id), layer);
    }

    pub fn do_not_load_object_from_save(&mut self, object_id: i32) {
        self.objects_not_from_save.insert(object_id);
    }

    pub fn object_loads_from_save(&self, object_id: i32) -> bool {
        !self.objects_not_from_save.contains(&object_id)
    }

    pub fn feature_enabled(&self, feature: Feature) -> bool {
        self.features.contains(&feature)
    }

    pub fn set_feature(&mut self, feature: Feature, enabled: bool) {
        if enabled {
            self.features.insert(feature);
        } else {
            self.features.remove(&feature);
        }
    }

    /// 当铺的资本，以及拿它算出来的等级（0..4 = E D C B A）。
    pub fn hockshop_capital(&self) -> i32 {
        self.hockshop_capital
    }

    pub fn hockshop_level(&self) -> i32 {
        super::hockshop::level_for_capital(self.hockshop_capital)
    }

    /// 增资：钱和资本一起动，等级跟着资本走（`FUN_00524b00`）。返回实际投
    /// 进去的钱。
    pub fn hockshop_invest(&mut self, amount: i32) -> i32 {
        let spent = amount.clamp(
            0,
            super::hockshop::max_investment(self.hockshop_capital, self.money()),
        );
        self.hockshop_capital += spent;
        self.add_money(-spent);
        spent
    }

    /// 某种经营方针的死当物池（空的时候是空切片）。
    pub fn hockshop_pool(&self, mode: usize) -> &[(u32, bool)] {
        self.hockshop_pools.get(mode).map_or(&[], |p| p.as_slice())
    }

    /// 摸过池子没有（旧存档、没升过级时没有）。
    pub fn has_hockshop_pools(&self) -> bool {
        !self.hockshop_pools.is_empty()
    }

    /// 换一套新池子，全部没卖。
    pub fn set_hockshop_pools(&mut self, pools: Vec<Vec<u32>>) {
        self.hockshop_pools = pools
            .into_iter()
            .map(|p| p.into_iter().map(|id| (id, false)).collect())
            .collect();
    }

    /// 买走一件：那一池里第一件还没卖的这个 id 标成卖掉。返回找没找到。
    pub fn mark_hockshop_sold(&mut self, mode: usize, id: u32) -> bool {
        let Some(pool) = self.hockshop_pools.get_mut(mode) else {
            return false;
        };
        match pool.iter_mut().find(|(i, sold)| *i == id && !*sold) {
            Some(entry) => {
                entry.1 = true;
                true
            }
            None => false,
        }
    }

    /// 分红：按当前等级和四个选择算一笔，加进钱里，返回分了多少
    /// （`FUN_0051f130`）。
    pub fn hockshop_dividend(&mut self, table: &super::hockshop::HockShopLevels) -> i32 {
        let level = table.level(self.hockshop_level());
        let paid = level.dividend(
            self.hockshop_capital,
            self.hockshop_mode.max(0) as usize,
            self.hockshop_period.max(0) as usize,
            self.hockshop_accrual.max(0) as usize,
            self.hockshop_unhock.max(0) as usize,
        );
        self.add_money(paid);
        paid
    }

    /// 四个经营选择：0 经营方针 · 1 当期 · 2 利率 · 3 拆当率。
    pub fn hockshop_choice(&self, which: usize) -> i32 {
        match which {
            0 => self.hockshop_mode,
            1 => self.hockshop_period,
            2 => self.hockshop_accrual,
            _ => self.hockshop_unhock,
        }
    }

    pub fn set_hockshop_choice(&mut self, which: usize, value: i32) {
        match which {
            0 => self.hockshop_mode = value,
            1 => self.hockshop_period = value,
            2 => self.hockshop_accrual = value,
            _ => self.hockshop_unhock = value,
        }
    }

    /// 山洞之狼赢过几次（`DAT_00bfda44`）。
    pub fn cave_wolf_wins(&self) -> u32 {
        self.cave_wolf_wins
    }

    pub fn record_cave_wolf_win(&mut self) {
        self.cave_wolf_wins += 1;
    }

    pub fn longkui_mode(&self) -> i32 {
        self.longkui_mode
    }

    pub fn set_longkui_mode(&mut self, mode: i32) {
        self.longkui_mode = mode;
    }

    pub fn huaying_mode(&self) -> i32 {
        self.huaying_mode
    }

    pub fn set_huaying_mode(&mut self, mode: i32) {
        self.huaying_mode = mode;
    }

    pub fn learn_compose(&mut self, recipe_id: u32) -> bool {
        self.compose_recipes.insert(recipe_id)
    }

    pub fn compose_recipes(&self) -> &BTreeSet<u32> {
        &self.compose_recipes
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

    /// 全部全局变量，按变量号排好。PAL3 的剧情变量号多是负的（主线进度是
    /// `-32768`），只往正数方向探是探不到的。
    pub fn globals(&self) -> Vec<(i16, i32)> {
        let mut v: Vec<(i16, i32)> = self.global_vars.iter().map(|(k, v)| (*k, *v)).collect();
        v.sort_unstable();
        v
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

    /// Put a character record in place, replacing any existing one.
    /// New-game seeding goes through [`Self::seed_from_db`]; this is for
    /// tools and tests that build a party by hand.
    pub fn insert_role(&mut self, actor_id: u32, role: RoleState) {
        self.roles.insert(actor_id, role);
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

    /// Wear `item_id`, taking it out of the inventory and putting whatever
    /// it displaces back in — which is what the original does
    /// (`FUN_00469eb0`: inventory count −1 for the new piece, +1 for the
    /// old one). Picks the first free slot the type allows, else swaps the
    /// first. Returns the displaced item id, or `None` when nothing was
    /// displaced. Fails (returns `None` without changing anything) when the
    /// item is not equipment, the character is unknown, or none is held.
    pub fn equip(&mut self, actor_id: u32, item_id: u32, item_type: u8) -> Option<u32> {
        let slots = equipment_slots_for_type(item_type);
        if slots.is_empty() || self.item_count(item_id) == 0 || !self.roles.contains_key(&actor_id)
        {
            return None;
        }

        let role = self.roles.get_mut(&actor_id)?;
        let slot = slots
            .iter()
            .copied()
            .find(|s| role.equipment[*s] == 0)
            .unwrap_or(slots[0]);
        let previous = role.equipment[slot];
        role.equipment[slot] = item_id;

        self.remove_item(item_id, 1);
        if previous != 0 {
            self.add_item(previous, 1);
        }
        (previous != 0).then_some(previous)
    }

    /// 穿到指定的那一格（饰品有两格，买装备页点的是哪一格就穿哪一格）。和 [`Self::equip`]
    /// 一样从背包拿一件、换下来的放回背包；类型不配这一格、背包里没有、人不在都不动。
    pub fn equip_in_slot(
        &mut self,
        actor_id: u32,
        item_id: u32,
        item_type: u8,
        slot: usize,
    ) -> Option<u32> {
        if !equipment_slots_for_type(item_type).contains(&slot) || self.item_count(item_id) == 0 {
            return None;
        }
        let role = self.roles.get_mut(&actor_id)?;
        let previous = role.equipment[slot];
        role.equipment[slot] = item_id;
        self.remove_item(item_id, 1);
        if previous != 0 {
            self.add_item(previous, 1);
        }
        (previous != 0).then_some(previous)
    }

    /// Take a slot's piece off and put it back in the inventory.
    pub fn unequip_to_inventory(&mut self, actor_id: u32, slot: usize) -> Option<u32> {
        let item_id = self.unequip(actor_id, slot)?;
        self.add_item(item_id, 1);
        Some(item_id)
    }

    /// Empty one slot, returning what was in it.
    pub fn unequip(&mut self, actor_id: u32, slot: usize) -> Option<u32> {
        if slot >= EQUIPMENT_SLOTS {
            return None;
        }
        let role = self.roles.get_mut(&actor_id)?;
        let previous = role.equipment[slot];
        role.equipment[slot] = 0;
        (previous != 0).then_some(previous)
    }

    /// Put a character *at* `level`, the way the script-driven path does
    /// (`FUN_0040ff70`): recompute every level-derived attribute from
    /// scratch, then copy the whole maximum bank over the current one —
    /// so this is also a full heal.
    ///
    /// This is not how winning a battle levels someone up; that path adds
    /// only the difference between the two levels and heals nothing. See
    /// [`Self::gain_level`].
    pub fn set_level(&mut self, actor_id: u32, level: i32) {
        let level = level.clamp(1, MAX_LEVEL);
        let g = growth::at_level(actor_id as usize, level);
        let Some(role) = self.roles.get_mut(&actor_id) else {
            return;
        };
        role.level = level;
        role.max_hp = g.max_hp;
        role.max_sp = g.max_sp;
        role.max_mp = g.max_mp;
        role.attack = g.attack;
        role.defense = g.defense;
        role.hp = g.max_hp;
        role.sp = g.max_sp;
        role.mp = g.max_mp;
    }

    /// Level a character up the way the victory settlement does
    /// (`FUN_004fa930`): add the **difference** between the old and new
    /// level's curves to whatever the character currently has, and do not
    /// heal. Adding the delta is what lets a permanent bonus — an item
    /// that raised a maximum, say — survive a level-up, which recomputing
    /// from scratch would silently wipe.
    ///
    /// Returns the levels actually gained.
    pub fn gain_level(&mut self, actor_id: u32, new_level: i32) -> i32 {
        let new_level = new_level.clamp(1, MAX_LEVEL);
        let Some(role) = self.roles.get(&actor_id) else {
            return 0;
        };
        let old_level = role.level;
        if new_level <= old_level {
            return 0;
        }

        let actor = actor_id as usize;
        let before = growth::at_level(actor, old_level);
        let after = growth::at_level(actor, new_level);
        let Some(role) = self.roles.get_mut(&actor_id) else {
            return 0;
        };

        role.level = new_level;
        role.max_hp += after.max_hp - before.max_hp;
        role.max_sp += after.max_sp - before.max_sp;
        role.max_mp += after.max_mp - before.max_mp;
        role.attack += after.attack - before.attack;
        role.defense += after.defense - before.defense;
        new_level - old_level
    }

    /// Award experience and level up as far as it carries. Returns the
    /// levels gained.
    pub fn award_experience(&mut self, actor_id: u32, db: &GameDb, amount: i32) -> i32 {
        let Some(role) = self.roles.get_mut(&actor_id) else {
            return 0;
        };
        role.exp += amount;
        let exp = role.exp;
        self.gain_level(actor_id, db.level_for_exp(exp))
    }

    /// Record one use of a skill and level it up if the use count has
    /// reached the next threshold. Returns the new level when it changed.
    pub fn use_skill(
        &mut self,
        actor_id: u32,
        skill_id: u32,
        times_before_level_up: &[u8; 4],
    ) -> Option<u8> {
        let role = self.roles.get_mut(&actor_id)?;
        let skill = role.skills.iter_mut().find(|s| s.id == skill_id)?;
        skill.uses = skill.uses.saturating_add(1);
        let level = levelled_up_to(skill.level, skill.uses, times_before_level_up)?;
        skill.level = level;
        Some(level)
    }

    /// 升级学会特技（`FUN_004fa930` 升级那一支里对特技库 `+0x2c8` 的扫描）。特技库是
    /// `FUN_0040eb90` 按 gdb 的顺序把本人能用（`applicable_actors`）的 `SkillType` 0 技能
    /// 排进去的，学没学会另有一格标志；升级时逐格看，还没学会、学会等级 `required_actor_level`
    /// 非 0 且不超过新等级的就学会。学会等级是 0 的那几条由剧情脚本教，这里不管。返回这次
    /// 新学会的，按库里的次序（即 gdb 次序；发行的数据里正好也是 id 升序）。`old_level`
    /// 原版不看——漏学的在下一次升级时补上。
    pub fn learn_level_specials(
        &mut self,
        db: &GameDb,
        actor_id: u32,
        _old_level: i32,
        new_level: i32,
    ) -> Vec<u32> {
        let mut ids: Vec<u32> = db
            .skills()
            .iter()
            .filter(|s| {
                let req = s.required_actor_level as i32;
                s.skill_type == 0
                    && req != 0
                    && req <= new_level
                    && s.applicable_actors
                        .get(actor_id as usize)
                        .is_some_and(|v| *v != 0)
            })
            .map(|s| s.id)
            .collect();
        ids.retain(|id| self.learn_skill(actor_id, *id));
        ids
    }

    /// Teach a skill, if it is not already known.
    pub fn learn_skill(&mut self, actor_id: u32, skill_id: u32) -> bool {
        let Some(role) = self.roles.get_mut(&actor_id) else {
            return false;
        };
        if role.skills.iter().any(|s| s.id == skill_id) {
            return false;
        }
        role.skills.push(LearnedSkill::new(skill_id));
        true
    }

    /// Which slot `item_id` is worn in, if any.
    pub fn equipped_slot_of(&self, actor_id: u32, item_id: u32) -> Option<usize> {
        let role = self.roles.get(&actor_id)?;
        role.equipment.iter().position(|id| *id == item_id)
    }

    /// Item id worn in `slot`, or `None` when it is empty.
    pub fn equipped(&self, actor_id: u32, slot: usize) -> Option<u32> {
        let role = self.roles.get(&actor_id)?;
        let id = *role.equipment.get(slot)?;
        (id != 0).then_some(id)
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

    // ---- 阵法 ----

    /// The 五灵 position (0..=4) this character stands in, if any.
    pub fn lineup_position(&self, actor_id: u32) -> Option<u8> {
        self.lineup.get(&actor_id).copied()
    }

    /// Which character stands in `position`, if any.
    pub fn lineup_occupant(&self, position: u8) -> Option<u32> {
        self.lineup
            .iter()
            .find(|(_, p)| **p == position)
            .map(|(id, _)| *id)
    }

    /// Move a character into a formation position, evicting whoever was
    /// standing there. Positions above 4 are rejected.
    pub fn set_lineup_position(&mut self, actor_id: u32, position: u8) -> bool {
        if position >= LINEUP_POSITIONS {
            return false;
        }
        self.lineup.retain(|_, p| *p != position);
        self.lineup.insert(actor_id, position);
        true
    }

    pub fn clear_lineup_position(&mut self, actor_id: u32) {
        self.lineup.remove(&actor_id);
    }

    /// 阵法页打开时重读站位（`FUN_00485df0` 消息 1000 → `FUN_00484660` → `FUN_00485980`）。
    /// 原版从角色记录 `+0x29a` 读回格子是 值 1→格 0、2→1、**3→3、4→2**、5→4、6→5（跳转表
    /// 0x485cb8），关页写回（`FUN_00485cd0` → `FUN_004845f0`，跳转表 0x484644）却是 格子号 + 1。
    /// 移植端存的就是格子号（= 值 − 1），所以打开一次 = 在队的人里格 2、3 对调；关页不用另做。
    /// 结果是每开关一次，站这两格的人的阵位值 3（风）/ 4（雷）互换一次，合击技的位置要求跟着
    /// 变——原版就是这样。
    pub fn reread_lineup_for_page(&mut self) {
        for id in self.team.clone() {
            if let Some(p) = self.lineup.get_mut(&id) {
                *p = match *p {
                    2 => 3,
                    3 => 2,
                    p => p,
                };
            }
        }
    }

    // ---- 侠义榜 ----

    pub fn monster_seen(&self, actor_id: u32) -> bool {
        self.seen_monsters.contains(&actor_id)
    }

    pub fn mark_monster_seen(&mut self, actor_id: u32) {
        self.seen_monsters.insert(actor_id);
    }

    pub fn monsters_seen_count(&self) -> usize {
        self.seen_monsters.len()
    }

    pub fn play_time(&self) -> f32 {
        self.play_time
    }

    pub fn advance_play_time(&mut self, delta_sec: f32) {
        self.play_time += delta_sec;
        self.tick_maze_effects(delta_sec);
    }

    /// 主循环里那段计时：`DAT_00c10cd0` 累计到 60 秒，把角色 0 的状态
    /// 27..30 和紫幽粉一起清掉，然后计时器归零。
    fn tick_maze_effects(&mut self, delta_sec: f32) {
        self.maze_effect_timer += delta_sec;
        if self.maze_effect_timer < MAZE_EFFECT_SECONDS {
            return;
        }
        self.maze_effect_timer = 0.;
        self.purple_powder = false;
        if let Some(role) = self.roles.get_mut(&0) {
            for k in 27..=30 {
                role.states[k] = 0;
            }
        }
    }

    /// 紫幽粉是否生效（`DAT_00c10cc1`）。
    pub fn purple_powder(&self) -> bool {
        self.purple_powder
    }

    /// 撒紫幽粉——`FUN_00410580(0x1f)`：置位并把计时器归零。
    pub fn set_purple_powder(&mut self) {
        self.purple_powder = true;
        self.maze_effect_timer = 0.;
    }

    /// `FUN_00410580(k)` 的另一支：角色 0 的状态字节 `k` 置 1，计时器归零。
    pub fn set_leader_maze_state(&mut self, k: usize) {
        if let Some(role) = self.roles.get_mut(&0) {
            if k < STATE_BYTES {
                role.states[k] = 1;
            }
        }
        self.maze_effect_timer = 0.;
    }

    /// 一颗五灵珠还剩几格充能；不是五灵珠返回 0。
    pub fn pearl_charge(&self, item_id: u32) -> u8 {
        pearl_charge_index(item_id)
            .map(|i| self.pearl_charges[i])
            .unwrap_or(0)
    }

    /// `FUN_004f7140`：扣一格充能。到 0 就不再动。
    pub fn spend_pearl_charge(&mut self, item_id: u32) {
        if let Some(i) = pearl_charge_index(item_id) {
            self.pearl_charges[i] = self.pearl_charges[i].saturating_sub(1);
        }
    }
}

/// A level-1 景天 built off the curves, for tests in this module.
#[cfg(test)]
pub(super) fn growth_test_role() -> RoleState {
    let g = growth::at_level(0, 1);
    RoleState {
        level: 1,
        exp: 0,
        hp: g.max_hp,
        max_hp: g.max_hp,
        sp: g.max_sp,
        max_sp: g.max_sp,
        mp: g.max_mp,
        max_mp: g.max_mp,
        attack: g.attack,
        defense: g.defense,
        speed: 3,
        luck: 2,
        elements: [3, 3, 6, 1, 2],
        skills: Vec::new(),
        equipment: [0; EQUIPMENT_SLOTS],
        states: [0; STATE_BYTES],
        delegation: Delegation::default(),
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
    fn equipment_slots_follow_item_type() {
        assert_eq!(equipment_slots_for_type(7), &[SLOT_CLOTH]);
        assert_eq!(equipment_slots_for_type(11), &[SLOT_WEAPON]);
        assert_eq!(
            equipment_slots_for_type(10),
            &[SLOT_ADORN1, SLOT_ADORN2],
            "饰品有两个位置"
        );
        assert!(equipment_slots_for_type(1).is_empty(), "治疗品不是装备");
        assert!(equipment_slots_for_type(12).is_empty(), "尸块不是装备");

        let mut state = PersistentState::new(PAL3_APP_NAME.to_string());
        state.roles.insert(
            0,
            RoleState {
                level: 1,
                exp: 0,
                hp: 80,
                max_hp: 80,
                sp: 8,
                max_sp: 8,
                mp: 25,
                max_mp: 25,
                attack: 26,
                defense: 14,
                speed: 3,
                luck: 2,
                elements: [3, 3, 6, 1, 2],
                skills: Vec::new(),
                equipment: [0; EQUIPMENT_SLOTS],
                states: [0; STATE_BYTES],
                delegation: Delegation::default(),
            },
        );

        // Nothing happens without the item in hand.
        assert_eq!(state.equip(0, 80009, 8), None);
        assert_eq!(state.equipped(0, SLOT_HAT), None);

        state.add_item(80009, 1);
        assert_eq!(state.equip(0, 80009, 8), None, "帽子槽本来是空的");
        assert_eq!(state.equipped(0, SLOT_HAT), Some(80009));
        assert_eq!(state.item_count(80009), 0, "戴上后离开背包");

        state.add_item(80010, 1);
        assert_eq!(state.equip(0, 80010, 8), Some(80009), "换下旧的那顶");
        assert_eq!(state.item_count(80009), 1, "换下来的回到背包");
        assert_eq!(state.equipped(0, SLOT_HAT), Some(80010));

        // 饰品 fills its second slot before swapping the first.
        state.add_item(80200, 1);
        state.add_item(80201, 1);
        assert_eq!(state.equip(0, 80200, 10), None);
        assert_eq!(state.equipped(0, SLOT_ADORN1), Some(80200));
        assert_eq!(state.equip(0, 80201, 10), None, "第二个饰品位还空着");
        assert_eq!(state.equipped(0, SLOT_ADORN2), Some(80201));

        assert_eq!(state.unequip_to_inventory(0, SLOT_HAT), Some(80010));
        assert_eq!(state.equipped(0, SLOT_HAT), None);
        assert_eq!(state.item_count(80010), 1);
    }

    #[test]
    fn using_a_skill_trains_it() {
        let mut state = PersistentState::new(PAL3_APP_NAME.to_string());
        state.roles.insert(0, super::growth_test_role());
        assert!(state.learn_skill(0, 4101));
        assert!(!state.learn_skill(0, 4101), "不重复学");

        let times = [3u8, 8, 20, 0];
        assert_eq!(state.use_skill(0, 4101, &times), None);
        assert_eq!(state.use_skill(0, 4101, &times), None);
        assert_eq!(state.use_skill(0, 4101, &times), Some(1), "第三次到门槛");

        let skill = state.role(0).unwrap().skills[0];
        assert_eq!(skill.level, 1);
        assert_eq!(skill.uses, 3);

        // An unknown skill is simply not tracked.
        assert_eq!(state.use_skill(0, 9999, &times), None);
    }

    #[test]
    fn a_battle_level_up_keeps_bonuses_that_a_reset_would_wipe() {
        let mut state = PersistentState::new(PAL3_APP_NAME.to_string());
        let mut role = super::growth_test_role();
        // A permanent +30 max HP on top of the level-1 curve.
        role.max_hp += 30;
        state.roles.insert(0, role);

        let gained = state.gain_level(0, 3);
        assert_eq!(gained, 2);

        let role = state.role(0).unwrap();
        assert_eq!(role.level, 3);
        // 景天's 精 curve is 8L + 80, so two levels are worth 16 — added
        // on top of the bonus rather than replacing it.
        assert_eq!(role.max_hp, growth::max_hp(0, 3) + 30);
        assert_eq!(role.hp, growth::max_hp(0, 1), "升级不回血");
    }

    #[test]
    fn setting_a_level_outright_heals_and_forgets_bonuses() {
        let mut state = PersistentState::new(PAL3_APP_NAME.to_string());
        let mut role = super::growth_test_role();
        role.max_hp += 30;
        role.hp = 1;
        state.roles.insert(0, role);

        state.set_level(0, 3);
        let role = state.role(0).unwrap();
        assert_eq!(role.max_hp, growth::max_hp(0, 3), "重算,加成没了");
        assert_eq!(role.hp, role.max_hp, "顺带全满");
    }

    #[test]
    fn levelling_down_does_nothing() {
        let mut state = PersistentState::new(PAL3_APP_NAME.to_string());
        state.roles.insert(0, super::growth_test_role());
        state.set_level(0, 10);
        assert_eq!(state.gain_level(0, 5), 0);
        assert_eq!(state.role(0).unwrap().level, 10);
    }

    #[test]
    fn lineup_positions_are_exclusive() {
        let mut state = PersistentState::new(PAL3_APP_NAME.to_string());
        assert!(state.set_lineup_position(0, 2));
        assert_eq!(state.lineup_position(0), Some(2));
        assert_eq!(state.lineup_occupant(2), Some(0));

        // Moving someone else in evicts the previous occupant.
        assert!(state.set_lineup_position(1, 2));
        assert_eq!(state.lineup_occupant(2), Some(1));
        assert_eq!(state.lineup_position(0), None);

        assert!(!state.set_lineup_position(3, 9), "只有六个阵位");
        state.clear_lineup_position(1);
        assert_eq!(state.lineup_occupant(2), None);
    }

    #[test]
    fn opening_the_lineup_page_crosses_places_two_and_three() {
        let mut state = PersistentState::new(PAL3_APP_NAME.to_string());
        for id in [0, 1, 2] {
            state.add_to_team(id);
        }
        state.set_lineup_position(0, 2);
        state.set_lineup_position(1, 3);
        state.set_lineup_position(2, 4);
        state.reread_lineup_for_page();
        assert_eq!(state.lineup_position(0), Some(3));
        assert_eq!(state.lineup_position(1), Some(2));
        assert_eq!(state.lineup_position(2), Some(4));
        state.reread_lineup_for_page();
        assert_eq!(state.lineup_position(0), Some(2), "再开一次换回来");
    }

    #[test]
    fn a_piece_can_go_into_a_chosen_slot() {
        let mut state = PersistentState::new(PAL3_APP_NAME.to_string());
        state.roles.insert(0, super::growth_test_role());
        state.add_item(7001, 2);
        // 饰品穿到第二格，第一格空着。
        assert_eq!(state.equip_in_slot(0, 7001, 10, SLOT_ADORN2), None);
        assert_eq!(state.equipped(0, SLOT_ADORN2), Some(7001));
        assert_eq!(state.equipped(0, SLOT_ADORN1), None);
        assert_eq!(state.item_count(7001), 1);
        // 类型对不上这一格：不动。
        assert_eq!(state.equip_in_slot(0, 7001, 10, SLOT_WEAPON), None);
        assert_eq!(state.item_count(7001), 1);
        // 同一格换一件：旧的回背包。
        state.add_item(7002, 1);
        assert_eq!(state.equip_in_slot(0, 7002, 10, SLOT_ADORN2), Some(7001));
        assert_eq!(state.item_count(7001), 2);
        assert_eq!(state.item_count(7002), 0);
    }

    #[test]
    fn a_dead_pawn_sells_once() {
        let mut state = PersistentState::new(PAL3_APP_NAME.to_string());
        assert!(!state.has_hockshop_pools());
        state.set_hockshop_pools(vec![vec![5, 6], vec![7], vec![], vec![]]);
        assert!(state.has_hockshop_pools());
        assert!(state.mark_hockshop_sold(0, 6));
        assert!(!state.mark_hockshop_sold(0, 6), "卖掉的不能再卖");
        assert!(!state.mark_hockshop_sold(1, 5), "别的方针的池子里没有");
        assert_eq!(state.hockshop_pool(0), &[(5, false), (6, true)]);
        assert_eq!(state.hockshop_pool(9), &[]);
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
