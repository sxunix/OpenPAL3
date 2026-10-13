//! 几条写死 id 的规矩，拿实机的 `pal3_softstar.gdb` 核一遍。
//!
//! 移植端有两处是照源码抄的写死 id：「千金散去」那套五件装备
//! （`cbrolestate.cpp:1358`）和 `CheckPerRound` 收尾的三只怪
//! （`cbrolestate.cpp:367..405`）。源码只是源码，这里确认发行数据里确有其物、
//! 而且类型放得进对应的装备格。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_special_rules_census
//! ```

use std::io::Cursor;
use std::path::PathBuf;

use common::store_ext::StoreExt2;
use fileformats::pal3::gdb::{GdbFile, ItemType};
use shared::openpal3::combat::state::{
    MONSTER_BLOOD_MIST, MONSTER_CHONGLOU, MONSTER_CHONGLOU_AWAKENED, MONSTER_SWAMP,
    MONSTER_SWORD_OF_EMPERORS, SUMMONED_DEMONS,
};

const GDB_PATH: &str = "/basedata/basedata/cbdata/pal3_softstar.gdb";

fn gdb() -> Option<GdbFile> {
    let assets = std::env::var("YAOBOW_PAL3_ASSETS").ok()?;
    let vfs = packfs::init_virtual_fs(&assets, None);
    let raw = vfs.read_to_end(PathBuf::from(GDB_PATH)).ok()?;
    Some(<GdbFile as binrw::BinRead>::read(&mut Cursor::new(raw)).expect("parse shipped gdb"))
}

#[test]
fn the_lost_money_set_is_five_real_pieces() {
    let Some(gdb) = gdb() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    // id、期望的名字、期望的装备格。右利 / 左利都是饰品，所以戴反了也成立。
    let want = [
        (80015u32, "绿幞头", ItemType::Hat),
        (84018, "通神", ItemType::Cloth),
        (83012, "元宝靴", ItemType::Shoes),
        (8153, "右利", ItemType::Wearable),
        (8154, "左利", ItemType::Wearable),
    ];
    for (id, name, kind) in want {
        let item = gdb
            .items
            .iter()
            .find(|i| i.id == id)
            .unwrap_or_else(|| panic!("物品 {id} 不在发行数据里"));
        assert_eq!(item.name.as_str().unwrap(), name, "物品 {id} 的名字对不上");
        assert_eq!(
            ItemType::from_i32(item.item_type as i32),
            Some(kind),
            "物品 {id}（{name}）的装备格对不上"
        );
    }
}

#[test]
fn the_three_per_round_monsters_and_chonglou_exist() {
    let Some(gdb) = gdb() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    let name = |id: u32| {
        gdb.combat_actors
            .iter()
            .find(|a| a.id == id)
            .unwrap_or_else(|| panic!("怪物 {id} 不在发行数据里"))
            .name
            .as_str()
            .unwrap()
    };
    assert_eq!(name(MONSTER_SWORD_OF_EMPERORS), "五帝之首剑");
    assert_eq!(name(MONSTER_SWAMP), "沼泽怪");
    assert_eq!(name(MONSTER_BLOOD_MIST), "血云雾");
    assert_eq!(name(MONSTER_CHONGLOU), "重楼");
    assert_eq!(name(MONSTER_CHONGLOU_AWAKENED), "重楼变身");
    for id in SUMMONED_DEMONS {
        // 唤妖术召的三只，名字不钉死，只确认确有其怪。
        let _ = name(id);
    }
}

/// `special_skill_id`（源码里的 `pFS->special`）在发行数据里只有这九个取值。
/// 移植端在 `Battle::skill_spec_affect` 里按这张表分工，所以表变了要来改。
#[test]
fn the_skill_special_column_holds_only_nine_values() {
    let Some(gdb) = gdb() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    let mut rows: Vec<(u8, u32, String)> = gdb
        .skills
        .iter()
        .filter(|s| s.special_skill_id != 0)
        .map(|s| (s.special_skill_id, s.id, s.name.as_str().unwrap()))
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        vec![
            // 1 随机一种毒：仙术那一路做了，915 是普攻方式那一路。
            (1, 254, "五彩光波".to_string()),
            (1, 915, "物理攻击15".to_string()),
            // 16..19：移植端按技能 id 在 `special_on` 里做。
            (16, 4101, "飞龙探云手".to_string()),
            (17, 4201, "绿波红露斩".to_string()),
            (18, 258, "鬼狱还神".to_string()),
            (18, 4304, "鬼狱还神".to_string()),
            (19, 4402, "吸精蛊".to_string()),
            // 25 大挪移：做了。
            (25, 263, "大挪移".to_string()),
            // 26/27/28/30：普攻方式那一路（`attack_mode_effect`）。
            (26, 912, "物理攻击12".to_string()),
            (27, 913, "物理攻击13".to_string()),
            (28, 914, "物理攻击14".to_string()),
            // 29 唤妖术：做了。
            (29, 7710, "唤妖术".to_string()),
            (30, 929, "物理攻击29".to_string()),
            // 31：`SpecAffect` 里没有 case 31，什么也不做。
            (31, 7711, "仙术2".to_string()),
        ]
    );
}
/// `FightItem::special`（移植端的 `item_special_type`）和仙术共用同一张
/// `SpecAffect` 表，但道具只用得到前四个号：1 随机一种毒、2 100% 脱离战斗、
/// 3 引路、4 等级提升一（`fightdata.h:131`）。战斗里真正会走 `SpecAffect` 的
/// 只有 1 和 2 两组，这张表钉住它们，别的号全是装备侧的（20..24 由
/// `InitRoleSpecEquipAttrib` 读，而且**只读帽衣鞋和两个饰品，不读武器**，
/// 所以刀剑弓杖身上的 22/23/24 不起作用）。
#[test]
fn only_four_items_reach_spec_affect_in_combat() {
    let Some(gdb) = gdb() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    let mut rows: Vec<(u8, u32, String)> = gdb
        .items
        .iter()
        .filter(|i| (1..=4).contains(&i.item_special_type))
        .map(|i| {
            (
                i.item_special_type,
                i.id,
                i.name.as_str().unwrap().to_string(),
            )
        })
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        vec![
            // 1 随机一种毒：`use_item` 的 `ITEM_SPECIAL_GU`。
            (1, 6113, "蛊".to_string()),
            (1, 6125, "五毒珠".to_string()),
            // 2 脱离战斗：`use_item` 的 `ITEM_SPECIAL_FLEE`。
            (2, 6114, "鬼遁水".to_string()),
            // 3 引路、4 升一级：都是战斗外的，在 `states::item_use` 里。
            (3, 6401, "引路蜂".to_string()),
            (3, 6427, "土灵珠".to_string()),
            (4, 6405, "紫幽粉".to_string()),
        ]
    );
}
