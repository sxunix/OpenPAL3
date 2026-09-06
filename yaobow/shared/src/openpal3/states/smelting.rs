//! 冶炼（SCE 146）的配方：拿背包里的尸块和矿石配对，配得上的能炼出一件
//! 东西。
//!
//! 逆向记录在 `switch/re/notes/smelting.md`。列表是 `FUN_0047a580` 建的：
//! 先 `FUN_0040d010(10)` 取尸块（`ItemType::Corpse`），再
//! `FUN_0040d010(3)` 取矿石和尸块（type 5 或 12），两两比
//! `尸块.ore_id == 候选.id`，命中就是一条配方，产物是 `尸块.product_id`。

use std::collections::BTreeMap;

use fileformats::pal3::gdb::ItemType;

use super::game_db::GameDb;

/// 一条配方：一块尸块 + 一份矿石 → 一件产物。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recipe {
    /// 尸块的物品 id。
    pub corpse: u32,
    /// 要搭的矿石 id（`GameItemInfo::ore_id`）。
    pub ore: u32,
    /// 炼出来的东西（`GameItemInfo::product_id`）。
    pub product: u32,
    /// 背包里还有几块尸块。
    pub corpse_count: u32,
    /// 背包里还有几份矿石。
    pub ore_count: u32,
}

impl Recipe {
    /// 两样材料都还有才炼得动。
    pub fn available(&self) -> bool {
        self.corpse_count > 0 && self.ore_count > 0
    }
}

/// 按 `FUN_0047a580` 的两重循环列出配方。
///
/// `inventory` 是背包（id → 数量），`BTreeMap` 的顺序就是原版列表的顺序
/// ——两张表都是从背包按 id 升序抓出来的。
pub fn recipes(db: &GameDb, inventory: &BTreeMap<u32, u32>) -> Vec<Recipe> {
    let type_of = |id: u32| db.item(id).map(|i| i.item_type);
    // `FUN_0040d010(3)`：矿石或尸块都算候选。
    let candidates: Vec<(u32, u32)> = inventory
        .iter()
        .filter(|(id, count)| {
            **count > 0
                && matches!(
                    type_of(**id),
                    Some(t) if t == ItemType::Ore as u8 || t == ItemType::Corpse as u8
                )
        })
        .map(|(id, count)| (*id, *count))
        .collect();

    let mut out = Vec::new();
    // `FUN_0040d010(10)`：只要尸块。
    for (corpse, corpse_count) in inventory.iter().map(|(a, b)| (*a, *b)) {
        if corpse_count == 0 || type_of(corpse) != Some(ItemType::Corpse as u8) {
            continue;
        }
        let Some(row) = db.item(corpse) else { continue };
        if row.ore_id == 0 || row.product_id == 0 {
            continue;
        }
        for (id, ore_count) in candidates.iter().copied() {
            if id != row.ore_id {
                continue;
            }
            out.push(Recipe {
                corpse,
                ore: id,
                product: row.product_id,
                corpse_count,
                ore_count,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use fileformats::pal3::gdb::GameItemInfo;

    fn item(id: u32, item_type: ItemType) -> GameItemInfo {
        let mut i = GameItemInfo::blank(id);
        i.item_type = item_type as u8;
        i
    }

    fn db() -> GameDb {
        // 一块尸块要一份矿石，炼出 7001；另一块尸块的矿石没有配方。
        let mut corpse = item(6001, ItemType::Corpse);
        corpse.ore_id = 5001;
        corpse.product_id = 7001;
        let mut lonely = item(6002, ItemType::Corpse);
        lonely.ore_id = 5999;
        lonely.product_id = 7002;
        GameDb::from_tables(
            vec![
                corpse,
                lonely,
                item(5001, ItemType::Ore),
                item(3001, ItemType::Healing),
            ],
            vec![],
        )
    }

    fn bag(entries: &[(u32, u32)]) -> BTreeMap<u32, u32> {
        entries.iter().copied().collect()
    }

    #[test]
    fn a_corpse_pairs_with_the_ore_it_names() {
        let r = recipes(&db(), &bag(&[(5001, 2), (6001, 3), (3001, 9)]));
        assert_eq!(
            r,
            vec![Recipe {
                corpse: 6001,
                ore: 5001,
                product: 7001,
                corpse_count: 3,
                ore_count: 2,
            }]
        );
        assert!(r[0].available());
    }

    #[test]
    fn no_ore_means_no_recipe() {
        // 尸块在，矿石不在——原版也不会把这一行摆出来。
        assert!(recipes(&db(), &bag(&[(6001, 1)])).is_empty());
        // 矿石在，但没有哪块尸块要它。
        assert!(recipes(&db(), &bag(&[(5001, 1), (6002, 1)])).is_empty());
    }

    #[test]
    fn only_corpse_pieces_start_a_recipe() {
        // 治疗药不是尸块，不该配出东西。
        assert!(recipes(&db(), &bag(&[(3001, 5), (5001, 5)])).is_empty());
    }
}
