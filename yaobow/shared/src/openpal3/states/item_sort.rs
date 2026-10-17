//! 物品页、装备页列表的排序（参考源码 `gamebase/playermgr.h` 的 `PlayerItemSortFun` 和
//! `Inventory::TypeArraySort`）。
//!
//! 页面上三个排序按钮：「使用序」（`SortUse`，源码叫默认排序）不排序，按背包里获得的先后列；「价格」是 `SORT_PRICE`；
//! 「功能」是 `SORT_FUNC`，实际按物品 id。都是升序。原版用 `std::sort`，相等的几件谁先谁后由实现决定；
//! 这里用稳定排序，相等的保持获得先后 [近似]（MSVC 的 `std::sort` 在 32 件以内是插入排序，本来就稳定；
//! 超过 32 件时相等项的次序可能和原版不同）。

use fileformats::pal3::gdb::GameItemInfo;

/// 不排序（页面上的「使用序」）。
pub const SORT_NONE: i32 = -1;
pub const SORT_PRICE: i32 = 0;
pub const SORT_GU: i32 = 1;
pub const SORT_NUM: i32 = 2;
pub const SORT_FUNC: i32 = 3;
pub const SORT_ITEM_TYPE: i32 = 4;
pub const SORT_WEAPON_TYPE: i32 = 5;

/// 把 `ids`（已按获得先后）按 `by` 排好。`item` 查数据库，`count` 查背包里的数量。
pub fn sort_items<'a>(
    ids: &mut [u32],
    by: i32,
    item: impl Fn(u32) -> Option<&'a GameItemInfo>,
    count: impl Fn(u32) -> u32,
) {
    let key = |id: u32| -> i64 {
        let info = item(id);
        match by {
            SORT_PRICE => info.map_or(0, |i| i.price as i64),
            SORT_GU => info.map_or(0, |i| i.ancient_value as i64),
            SORT_NUM => count(id) as i64,
            SORT_FUNC => id as i64,
            SORT_ITEM_TYPE => info.map_or(0, |i| i.item_type as i64),
            SORT_WEAPON_TYPE => info.map_or(0, |i| i.weapon_type as i64),
            _ => 0,
        }
    };
    if (SORT_PRICE..=SORT_WEAPON_TYPE).contains(&by) {
        ids.sort_by_key(|id| key(*id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: u32, price: i32) -> GameItemInfo {
        let mut i = GameItemInfo::default();
        i.id = id;
        i.price = price;
        i
    }

    #[test]
    fn price_and_func_sort_ascending_and_none_keeps_order() {
        let db = [info(30, 50), info(10, 200), info(20, 50)];
        let item = |id: u32| db.iter().find(|i| i.id == id);
        let mut ids = vec![30, 10, 20];
        sort_items(&mut ids, SORT_NONE, item, |_| 1);
        assert_eq!(ids, [30, 10, 20]);
        sort_items(&mut ids, SORT_PRICE, item, |_| 1);
        assert_eq!(ids, [30, 20, 10]);
        sort_items(&mut ids, SORT_FUNC, item, |_| 1);
        assert_eq!(ids, [10, 20, 30]);
    }
}
