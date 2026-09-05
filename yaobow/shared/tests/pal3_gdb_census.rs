//! 技能表里的「第三项消耗」到底用到了哪几种。
//!
//! 原版扣消耗的 `FUN_004f2910`（分派在 0x4f2985）按 `special_consume_type`
//! 分了十七支，但实机数据只可能走到其中两支——这个普查就是拿实机的
//! `pal3_softstar.gdb` 把这件事钉死，免得移植去实现一堆没人用的分支，也免
//! 得漏掉真的在用的那两支。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_gdb_census
//! ```

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::PathBuf;

use common::store_ext::StoreExt2;
use fileformats::pal3::gdb::GdbFile;
use shared::openpal3::combat::actions::{GU_ITEM_ID, SPECIAL_COST_GU, SPECIAL_COST_MONEY};

const GDB_PATH: &str = "/basedata/basedata/cbdata/pal3_softstar.gdb";

fn gdb() -> Option<GdbFile> {
    let assets = std::env::var("YAOBOW_PAL3_ASSETS").ok()?;
    let vfs = packfs::init_virtual_fs(&assets, None);
    let raw = vfs.read_to_end(PathBuf::from(GDB_PATH)).ok()?;
    Some(<GdbFile as binrw::BinRead>::read(&mut Cursor::new(raw)).expect("parse shipped gdb"))
}

#[test]
fn only_money_and_gu_are_ever_charged_as_a_third_cost() {
    let Some(gdb) = gdb() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    let mut hist: BTreeMap<i32, usize> = BTreeMap::new();
    for s in &gdb.skills {
        *hist.entry(s.special_consume_type).or_default() += 1;
    }
    let kinds: Vec<i32> = hist.keys().copied().collect();
    assert_eq!(
        kinds,
        vec![-1, SPECIAL_COST_MONEY, SPECIAL_COST_GU],
        "第三项消耗出现了预期外的种类: {hist:?}"
    );

    // 花钱的两招：洒金笺 80 文、乾坤一掷 3000 文。
    let money: Vec<(u32, String, i32)> = gdb
        .skills
        .iter()
        .filter(|s| s.special_consume_type == SPECIAL_COST_MONEY)
        .map(|s| (s.id, s.name.as_str().unwrap(), s.special_consume_value))
        .collect();
    assert_eq!(
        money,
        vec![
            (4102, "洒金笺".to_string(), 80),
            (4305, "乾坤一掷".to_string(), 3000)
        ],
        "花钱的技能对不上"
    );

    // 五个蛊术，各扣一个蛊。原版检查那一侧只问背包里有没有，不看这个量。
    let gu: Vec<(u32, String, i32)> = gdb
        .skills
        .iter()
        .filter(|s| s.special_consume_type == SPECIAL_COST_GU)
        .map(|s| (s.id, s.name.as_str().unwrap(), s.special_consume_value))
        .collect();
    assert_eq!(
        gu,
        vec![
            (4401, "万蚁蛊".to_string(), 1),
            (4402, "吸精蛊".to_string(), 1),
            (4403, "梦蛇蛊".to_string(), 1),
            (4404, "潜身蛊".to_string(), 1),
            (4405, "万蛊蚀天".to_string(), 1),
        ],
        "蛊术对不上"
    );
}

#[test]
fn the_hard_coded_gu_item_id_is_the_gu() {
    let Some(gdb) = gdb() else { return };
    let gu = gdb
        .items
        .iter()
        .find(|i| i.id == GU_ITEM_ID)
        .expect("物品表里没有 6113");
    assert_eq!(gu.name.as_str().unwrap(), "蛊");
}
