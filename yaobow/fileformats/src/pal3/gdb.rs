//! PAL3 `cbdata\pal3_softstar.gdb` — the game database.
//!
//! One binary file holding the four tables the original engine loads in
//! `Fight\LoadData.cpp` (`FUN_00478e00` in the v1.4 executable): combat
//! actors (playable characters *and* monsters), skills, items and combo
//! skills (合击技).
//!
//! ```text
//! 0x00  char[128]  header string
//! 0x80  u32 u32    combat actor table: file offset, record count
//! 0x88  u32 u32    skill table
//! 0x90  u32 u32    item table
//! 0x98  u32 u32    combo skill table
//! ```
//!
//! Records are fixed size and tightly packed at their table offset:
//! actors 852 (0x354), skills 760 (0x2f8), items 800, combo skills 632
//! (0x278) bytes. Those four sizes are read straight out of the four
//! `gbVFileSystem::Read` calls in `FUN_00478e00`, and the field layouts
//! below reproduce them exactly — see the `record_sizes` test.
//!
//! Text fields are fixed-capacity GBK buffers terminated by the first NUL.
//!
//! # Provenance
//!
//! The field-level layout was ported from `Pal3.Core/DataReader/Gdb/`
//! (`GdbFile.cs`, `GdbFileReader.cs`) of **Pal3.Unity**, Copyright (c)
//! 2021-2025 Jiaqi (0x7c13) Liu, licensed GPL-3.0 —
//! <https://github.com/0x7c13/Pal3.Unity>. yaobow is GPL-3.0 as well, so
//! the port is license-compatible; this notice is the attribution.
//! Record sizes and table offsets were independently confirmed against the
//! original executable (see `switch/re/notes/`).
//!
//! Only the PAL3 layout is implemented. 仙剑三外传 (PAL3A) reuses the file
//! name but has a different item record tail; see the `PAL3A` branches in
//! the upstream reader when that game is added.

use std::io::{Read, Seek, SeekFrom};

use binrw::{BinRead, BinResult};
use serde::Serialize;

use crate::utils::StringWithCapacity;

/// Number of `ActorAttributeType` slots stored per record (精气神武防速运
/// plus the five elements).
pub const ATTRIBUTE_COUNT: usize = 12;

/// Number of `ActorCombatStateType` slots stored per record.
pub const COMBAT_STATE_COUNT: usize = 31;

/// Number of playable characters an item/skill can be restricted to.
pub const PLAYER_ACTOR_COUNT: usize = 5;

/// Number of elements (五灵: 水火风雷土).
pub const ELEMENT_COUNT: usize = 5;

/// The parsed database.
#[derive(Debug, Default, BinRead, Serialize)]
#[brw(little)]
pub struct GdbFile {
    #[brw(args(128))]
    pub header: StringWithCapacity,

    pub combat_actor_offset: u32,
    pub combat_actor_count: u32,
    pub skill_offset: u32,
    pub skill_count: u32,
    pub item_offset: u32,
    pub item_count: u32,
    pub combo_skill_offset: u32,
    pub combo_skill_count: u32,

    #[br(seek_before = SeekFrom::Start(combat_actor_offset as u64), count = combat_actor_count)]
    pub combat_actors: Vec<CombatActorInfo>,

    #[br(seek_before = SeekFrom::Start(skill_offset as u64), count = skill_count)]
    pub skills: Vec<SkillInfo>,

    #[br(seek_before = SeekFrom::Start(item_offset as u64), count = item_count)]
    pub items: Vec<GameItemInfo>,

    #[br(seek_before = SeekFrom::Start(combo_skill_offset as u64), count = combo_skill_count)]
    pub combo_skills: Vec<ComboSkillInfo>,
}

impl GdbFile {
    pub fn read(reader: &mut (impl Read + Seek)) -> BinResult<Self> {
        Self::read_le(reader)
    }
}

/// A combat participant: the six playable characters (`type == MainActor`)
/// and every monster. 852 bytes.
#[derive(Debug, BinRead, Serialize)]
#[brw(little)]
pub struct CombatActorInfo {
    pub id: u32,
    /// [`CombatActorType`]: 主角/人/妖/仙/神/鬼/魔.
    pub actor_type: u16,
    #[brw(args(512))]
    pub description: StringWithCapacity,
    #[brw(args(30))]
    pub model_id: StringWithCapacity,
    #[brw(args(32))]
    pub icon_id: StringWithCapacity,
    /// Base 五灵 values, indexed by [`ElementType`] minus one.
    pub element_attribute_values: [i32; ELEMENT_COUNT],
    #[brw(args(32))]
    pub name: StringWithCapacity,
    pub level: i32,
    /// Base attributes, indexed by [`ActorAttributeType`].
    pub attribute_values: [i32; ATTRIBUTE_COUNT],
    /// Per-[`ActorCombatStateType`] [`CombatStateImpactType`].
    pub combat_state_impact_types: [u8; COMBAT_STATE_COUNT],
    _pad_0: u8,
    /// Round cap; `-1` in the shipped database, `0` for the main characters.
    pub max_round: i32,
    pub special_action_id: i32,
    pub escape_rate: f32,
    /// Favour towards each playable character (save-game field).
    pub main_actor_favor: [u16; 6],
    pub experience: i32,
    /// Save-game field.
    pub money: u16,
    _pad_1: u16,
    /// Normal-attack pattern; 916/917/921/922/926/927 hit a row,
    /// 918/919/920/923/928 hit a column.
    pub normal_attack_mode_id: u32,
    /// Dungeon height band: 0-2 = 低/中/高.
    pub height_level: u8,
    /// Roaming range: 0-3 = 不动/小/中/大.
    pub move_range_level: u8,
    /// Aggro range: 0-2 = 小/中/大.
    pub attack_range_level: u8,
    /// Move speed: 0-3 = 不动/小/中/大.
    pub move_speed_level: u8,
    /// Chase speed: 0-2 = 小/中/大.
    pub chase_speed_level: u8,
    _pad_2: [u8; 3],
    /// `[0..3]` normal skills, `[3]` the boss-tier skill.
    pub skill_ids: [u32; 4],
    /// Levels of the three normal skills (the boss skill has none).
    pub skill_levels: [u8; 4],
    /// 气 a party member **gains** when this actor's physical hit lands on
    /// them (battle record `+0x284`, added by `FUN_004f3020(target, v, 0)`
    /// in `FUN_004f0a10`). Every monster row is positive.
    pub sp_impact_value: i32,
    /// Extra properties, mainly used to pick combat sound effects.
    pub properties: [f32; 10],
    pub normal_loot: u32,
    pub normal_loot_count: i16,
    _pad_3: i16,
    /// 尸块 (corpse chunk) obtainable from this actor.
    pub corpse_id: u32,
    pub corpse_skill_id: u32,
    pub corpse_success_rate: i16,
    pub stealable_money_amount: i16,
    pub stealable_item_id: u32,
    pub stealable_item_count: i16,
    pub money_when_killed: i16,
}

/// A skill / 仙术. 760 bytes.
///
/// `Default` is derived for the same reason as on [`GameItemInfo`]: tests
/// build rows without naming the padding.
#[derive(Debug, Default, BinRead, Serialize)]
#[brw(little)]
pub struct SkillInfo {
    pub id: u32,
    /// [`SkillType`].
    pub skill_type: u8,
    _pad_0: [u8; 3],
    /// Element flags; stored as `i32` but only the low byte is meaningful.
    pub element_attributes: [i32; ELEMENT_COUNT],
    #[brw(args(32))]
    pub name: StringWithCapacity,
    #[brw(args(512))]
    pub description: StringWithCapacity,
    /// Non-zero for each playable character allowed to use this skill.
    pub applicable_actors: [u8; PLAYER_ACTOR_COUNT],
    /// [`TargetRangeType`].
    pub target_range_type: u8,
    /// 0 = no special effect; values starting with 1 steal from the enemy.
    pub special_skill_id: u8,
    /// Per-[`ActorAttributeType`] [`AttributeImpactType`].
    pub attribute_impact_type: [u8; ATTRIBUTE_COUNT],
    _pad_1: u8,
    pub attribute_impact_value: [i16; ATTRIBUTE_COUNT],
    /// 0-10 use the engine's own curve, anything larger is a percentage.
    pub success_rate_level: u8,
    _pad_2: u8,
    /// Per-[`ActorCombatStateType`] [`CombatStateImpactType`], stored wide.
    pub combat_state_impact_types: [i16; COMBAT_STATE_COUNT],
    /// [`AttributeImpactType`] of the 气 cost.
    pub sp_consume_impact_type: i32,
    /// [`AttributeImpactType`] of the 神 cost.
    pub mp_consume_impact_type: i32,
    pub sp_consume_value: i32,
    pub mp_consume_value: i32,
    /// `-1` none; otherwise an attribute index
    /// (0 精 1 气 2 神 3 武 4 防 5 情 6 经 7 速 8 运 9 级 10 钱 11-15 五灵 16 蛊).
    pub special_consume_type: i32,
    /// [`AttributeImpactType`] of the special cost.
    pub special_consume_impact_type: u8,
    _pad_3: [u8; 3],
    pub special_consume_value: i32,
    pub level: u8,
    /// Uses required at each level before it advances.
    pub times_before_level_up: [u8; 4],
    pub required_actor_level: u8,
    pub magic_level: u8,
    _pad_4: u8,
    pub next_level_skill_id: u32,
    pub is_usable_outside_combat: u8,
    _pad_5: [u8; 3],
    /// Skills this one can be composed into.
    pub composite_skill_ids: [u32; 3],
    /// The other skill each composition needs.
    pub composite_required_skill_ids: [u32; 3],
    pub composite_required_skill_levels: [u8; 3],
    pub composite_required_current_skill_levels: [u8; 3],
    pub composite_required_actor_levels: [u8; 3],
    pub can_trigger_combo_skill: u8,
    _pad_6: [u8; 2],
}

impl SkillInfo {
    /// A zeroed row carrying only an id.
    pub fn blank(id: u32) -> Self {
        Self {
            id,
            ..Default::default()
        }
    }
}

/// An item. 800 bytes (PAL3 layout).
///
/// `Default` is derived so callers (tests, editors) can build a blank row
/// without naming the padding fields, which are private on purpose.
#[derive(Debug, Default, BinRead, Serialize)]
#[brw(little)]
pub struct GameItemInfo {
    pub id: u32,
    #[brw(args(32))]
    pub name: StringWithCapacity,
    #[brw(args(30))]
    pub model_name: StringWithCapacity,
    #[brw(args(30))]
    pub icon_name: StringWithCapacity,
    #[brw(args(512))]
    pub description: StringWithCapacity,
    pub price: i32,
    /// [`ItemType`].
    pub item_type: u8,
    /// [`WeaponType`]; only meaningful when `item_type == Weapon`.
    pub weapon_type: u8,
    /// Non-zero for each playable character allowed to use this item.
    pub applicable_actors: [u8; PLAYER_ACTOR_COUNT],
    pub element_attributes: [u8; ELEMENT_COUNT],
    /// Value when sold as an antique (古董).
    pub ancient_value: i32,
    /// [`ItemSpecialType`].
    pub item_special_type: u8,
    /// [`TargetRangeType`].
    pub target_range_type: u8,
    /// [`PlaceOfUseType`].
    pub place_of_use_type: u8,
    /// Per-[`ActorAttributeType`] [`AttributeImpactType`].
    pub attribute_impact_type: [u8; ATTRIBUTE_COUNT],
    _pad_0: u8,
    pub attribute_impact_value: [i16; ATTRIBUTE_COUNT],
    /// Per-[`ActorCombatStateType`] flag. On weapons it marks which states a
    /// physical hit may inflict, with the chance (percent) in the same slot
    /// of [`Self::combat_state_impact_value`] — `FUN_004f2560` reads the
    /// first 0x17 of them. Other item types leave it zero.
    pub combat_state_impact_types: [u8; COMBAT_STATE_COUNT],
    _pad_1: u8,
    /// Per-[`ActorCombatStateType`], what using the item does to that
    /// state: **1 = 加上，2 = 解掉**, 0 = leave alone. Indexed by the
    /// state *enum* (the combat flag array numbers the same states from
    /// 1, i.e. `k = index + 1`). Both paths that use an item read it:
    /// `FUN_0049cf80` out of combat and `FUN_004ef4a0` in it.
    pub combat_state_impact_value: [i16; COMBAT_STATE_COUNT],
    _pad_2: [u8; 2],

    // ---- equipment / crafting tail (PAL3 only) ----
    pub combo_count: i32,
    /// Percentage cut on 气 losses — skill costs and hits alike (`FUN_004f3020`).
    pub sp_saving_percentage: i16,
    /// Percentage cut on 神 losses, skill costs included (`FUN_004f31c0`).
    pub mp_saving_percentage: i16,
    pub critical_attack_amplify_percentage: i16,
    pub special_skill_success_rate: i16,
    /// Ore consumed when refining this 尸块.
    pub ore_id: u32,
    /// Item produced by refining.
    pub product_id: u32,
    pub product_price: i32,
    /// The two materials this item is synthesised from.
    pub synthesis_material_ids: [u32; 2],
    pub synthesis_product_id: u32,
}

impl GameItemInfo {
    /// An empty row carrying only an id — for tests and for tools that
    /// build a record up field by field. The padding fields stay private,
    /// so struct-update syntax cannot do this from another crate.
    pub fn blank(id: u32) -> Self {
        Self {
            id,
            ..Default::default()
        }
    }
}

/// A combo skill / 合击技. 632 bytes.
#[derive(Debug, BinRead, Serialize)]
#[brw(little)]
pub struct ComboSkillInfo {
    #[brw(args(32))]
    pub name: StringWithCapacity,
    pub id: u32,
    /// Playable characters that must take part (0 = slot unused).
    pub main_actor_requirements: [u32; 4],
    /// [`ElementPositionRequirementType`] for each participant above.
    pub element_position_requirements: [u8; 4],
    /// The skill actually cast.
    pub skill_id: u32,
    /// [`WeaponType`] each participant must be wielding.
    pub weapon_type_requirements: [u8; 4],
    _unused_0: [u8; 4],
    /// [`ActorCombatStateType`] each participant must be in.
    pub combat_state_requirements: [i32; 3],
    #[brw(args(512))]
    pub description: StringWithCapacity,
    /// [`TargetRangeType`].
    pub target_range_type: u8,
    /// Per-[`ActorAttributeType`] [`AttributeImpactType`].
    pub attribute_impact_type: [u8; ATTRIBUTE_COUNT],
    _pad_0: u8,
    /// `+0x25e`：`FUN_004eef20` 的 0x10e 分支按 `combo+0x25e+i*2` 取。实机只有精
    /// （下标 0）一格非零：九条是 −300..−1200，「珠联璧合」是 +500。原版只拿它的
    /// **正负号**，数值由 `FUN_004f2800` 算。（以前按 3 字节填充读在 0x260，读偏了。）
    pub attribute_impact_value: [i16; ATTRIBUTE_COUNT],
    _pad_1: [u8; 2],
}

macro_rules! primitive_enum {
    ($(#[$meta:meta])* $name:ident {
        $($(#[$vmeta:meta])* $variant:ident = $value:expr),* $(,)?
    }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
        pub enum $name {
            $($(#[$vmeta])* $variant = $value),*
        }

        impl $name {
            pub fn from_i32(value: i32) -> Option<Self> {
                match value {
                    $($value => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
}

primitive_enum! {
    /// 五灵.
    ElementType {
        None = 0,
        Water = 1,
        Fire = 2,
        Wind = 3,
        Thunder = 4,
        Earth = 5,
    }
}

primitive_enum! {
    /// Where a participant of a combo skill must stand.
    ElementPositionRequirementType {
        Any = 0,
        Water = 1,
        Fire = 2,
        Wind = 3,
        Thunder = 4,
        Earth = 5,
        Middle = 6,
        /// 风火雷任意.
        WindFireThunder = 7,
        /// 水火土任意.
        WaterFireEarth = 8,
    }
}

primitive_enum! {
    /// Who a skill or item can be aimed at.
    TargetRangeType {
        None = 0,
        /// 我方单体.
        FirstPartySingle = 1,
        /// 我方全体.
        FirstPartyAll = 2,
        /// 敌方单体.
        EnemyPartySingle = 3,
        /// 敌方全体.
        EnemyPartyAll = 4,
        /// 敌方一排.
        EnemyPartyOneRow = 5,
        /// 敌方一列.
        EnemyPartyOneColumn = 6,
    }
}

primitive_enum! {
    /// Where an item may be used.
    PlaceOfUseType {
        None = 0,
        Combat = 1,
        OutOfCombat = 2,
        Anywhere = 3,
    }
}

primitive_enum! {
    /// Index into the 12-slot attribute arrays.
    ActorAttributeType {
        /// 精.
        Hp = 0,
        /// 气.
        Sp = 1,
        /// 神.
        Mp = 2,
        /// 武.
        Attack = 3,
        /// 防.
        Defense = 4,
        /// 速.
        Speed = 5,
        /// 运.
        Luck = 6,
        Water = 7,
        Fire = 8,
        Wind = 9,
        Thunder = 10,
        Earth = 11,
    }
}

primitive_enum! {
    /// 战斗角色种类.
    CombatActorType {
        MainActor = 0,
        Human = 1,
        Monster = 2,
        Fairy = 3,
        God = 4,
        Ghost = 5,
        Demon = 6,
    }
}

primitive_enum! {
    SkillType {
        StandardMagic = 0,
        AssistMagic = 1,
        DamageMagic = 2,
        RecoverMagic = 3,
        WeaponMagic = 4,
    }
}

primitive_enum! {
    /// How a skill or item changes a combat state.
    CombatStateImpactType {
        None = 0,
        Increase = 1,
        Remove = 2,
    }
}

primitive_enum! {
    /// How an attribute impact value is applied.
    AttributeImpactType {
        Absolute = 0,
        Percentage = 1,
        RecoverToMax = 2,
        IncreaseMax = 3,
    }
}

primitive_enum! {
    ItemType {
        None = 0,
        /// 治疗.
        Healing = 1,
        /// 投掷.
        Throwable = 2,
        /// 仙宝.
        Treasure = 3,
        /// 古董.
        Antique = 4,
        /// 矿石.
        Ore = 5,
        /// 剧情.
        Plot = 6,
        /// 衣服.
        Cloth = 7,
        /// 帽子.
        Hat = 8,
        /// 鞋子.
        Shoes = 9,
        /// 饰品.
        Wearable = 10,
        /// 武器.
        Weapon = 11,
        /// 尸块.
        Corpse = 12,
    }
}

primitive_enum! {
    WeaponType {
        None = 0,
        /// 长矛.
        Spear = 1,
        /// 剑.
        Sword = 2,
        /// 杖.
        Staff = 3,
        /// 刀.
        Machete = 4,
        /// 弓.
        Bow = 5,
        /// 刺.
        Spine = 6,
        /// 镰刀.
        Sickle = 7,
    }
}

primitive_enum! {
    /// Special one-off item effects. The gaps are intentional — the shipped
    /// database only uses these values.
    ItemSpecialType {
        None = 0,
        /// 随机中毒.
        Poison = 1,
        /// 100% 逃跑.
        Flee = 2,
        /// 等级提升一级.
        LevelIncrease = 15,
        /// 经验增加 5%.
        ExpIncrease5Percent = 20,
        /// 经验增加 15%.
        ExpIncrease15Percent = 21,
    }
}

/// The 31 combat states, in the order they appear in every record's
/// `combat_state_impact_types` array.
pub const COMBAT_STATE_NAMES: [&str; COMBAT_STATE_COUNT] = [
    "风毒",
    "雷毒",
    "水毒",
    "火毒",
    "土毒",
    "武+10%",
    "武-10%",
    "防+10%",
    "防-10%",
    "运+10%",
    "运-10%",
    "速+10%",
    "速-10%",
    "定",
    "封",
    "禁",
    "眠",
    "乱",
    "狂",
    "镜",
    "避",
    "界",
    "隐形",
    "濒死",
    "死亡",
    "避毒",
    "任意毒",
    "辟邪",
    "退魔",
    "神眼",
    "精进",
];

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    /// Each record must consume exactly as many bytes as the original
    /// engine reads per element in `Fight\LoadData.cpp`.
    fn record_size<T: for<'a> BinRead<Args<'a> = ()>>() -> u64 {
        let buf = vec![0u8; 4096];
        let mut cursor = Cursor::new(buf);
        T::read_le(&mut cursor).expect("record should parse from zeroed bytes");
        cursor.position()
    }

    #[test]
    fn record_sizes() {
        assert_eq!(record_size::<CombatActorInfo>(), 852);
        assert_eq!(record_size::<SkillInfo>(), 760);
        assert_eq!(record_size::<GameItemInfo>(), 800);
        assert_eq!(record_size::<ComboSkillInfo>(), 632);
    }

    /// Parse the shipped database when it is available. Skipped (not failed)
    /// without game data so CI stays green. Produce the fixture with:
    ///
    /// ```text
    /// cargo run -p packfs --example pal3_probe -- <PAL3 dir> \
    ///     --dump /basedata/basedata/cbdata/pal3_softstar.gdb /tmp/pal3_softstar.gdb
    /// ```
    #[test]
    fn parses_shipped_database_if_present() {
        let path = std::env::var("YAOBOW_PAL3_GDB")
            .unwrap_or_else(|_| "/tmp/pal3_softstar.gdb".to_string());
        let Ok(raw) = std::fs::read(&path) else {
            eprintln!("skipping: no gdb fixture at {path}");
            return;
        };

        let len = raw.len() as u64;
        let mut cursor = Cursor::new(raw);
        let gdb = GdbFile::read(&mut cursor).expect("parse shipped gdb");

        assert!(
            gdb.header
                .as_str()
                .unwrap()
                .starts_with("__PAL(Softstar_Ltd"),
            "unexpected header: {:?}",
            gdb.header.as_str()
        );

        // The four tables are laid out back to back and fill the file
        // exactly, so a wrong record size would break the chain.
        assert_eq!(gdb.combat_actor_offset, 0xa0);
        let table_end = |offset: u32, count: u32, size: u32| offset + count * size;
        assert_eq!(
            table_end(gdb.combat_actor_offset, gdb.combat_actor_count, 852),
            gdb.skill_offset
        );
        assert_eq!(
            table_end(gdb.skill_offset, gdb.skill_count, 760),
            gdb.item_offset
        );
        assert_eq!(
            table_end(gdb.item_offset, gdb.item_count, 800),
            gdb.combo_skill_offset
        );
        assert_eq!(
            table_end(gdb.combo_skill_offset, gdb.combo_skill_count, 632) as u64,
            len
        );

        assert_eq!(gdb.combat_actors.len(), gdb.combat_actor_count as usize);
        assert_eq!(gdb.skills.len(), gdb.skill_count as usize);
        assert_eq!(gdb.items.len(), gdb.item_count as usize);
        assert_eq!(gdb.combo_skills.len(), gdb.combo_skill_count as usize);

        // The playable characters lead the actor table.
        let jingtian = &gdb.combat_actors[0];
        assert_eq!(jingtian.name.as_str().unwrap(), "景天");
        assert_eq!(
            CombatActorType::from_i32(jingtian.actor_type as i32),
            Some(CombatActorType::MainActor)
        );
        assert_eq!(jingtian.level, 1);

        // Every record should carry decodable GBK text and sane enum tags.
        for actor in &gdb.combat_actors {
            actor.name.as_str().expect("actor name decodes");
            assert!(CombatActorType::from_i32(actor.actor_type as i32).is_some());
        }
        for item in &gdb.items {
            item.name.as_str().expect("item name decodes");
            assert!(ItemType::from_i32(item.item_type as i32).is_some());
            assert!(WeaponType::from_i32(item.weapon_type as i32).is_some());
        }
        for skill in &gdb.skills {
            skill.name.as_str().expect("skill name decodes");
            assert!(SkillType::from_i32(skill.skill_type as i32).is_some());
        }
        for combo in &gdb.combo_skills {
            combo.name.as_str().expect("combo skill name decodes");
        }
    }
}
