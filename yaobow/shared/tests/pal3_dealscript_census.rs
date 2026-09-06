//! 拿实机的全部 `basedata/DealScript/**/*.txt` 跑一遍解析器。
//!
//! 解析器是照 `FUN_00457490` 写的（见 `switch/re/notes/shop.md`），这里查的
//! 是「每个文件都能解出该有的东西」：商人 id 和名字、五个折扣档、至少一段
//! 商品清单，以及商品 id 在数据库里查得到。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_dealscript_census
//! ```

use std::path::{Path, PathBuf};

use common::store_ext::StoreExt2;
use encoding::Encoding;
use mini_fs::{MiniFs, StoreExt};
use shared::openpal3::states::game_db::GameDb;
use shared::openpal3::states::shop::{DealScript, RestScript, TIERS};

/// 卖东西的店。`Rest` 是客栈，另一套格式，单独查。
const SHOP_DIRS: [&str; 3] = ["AnyShop", "DragShop", "WeaponShop"];

fn assets() -> Option<String> {
    std::env::var("YAOBOW_PAL3_ASSETS").ok()
}

fn read_gbk(vfs: &MiniFs, path: &Path) -> Option<String> {
    let bytes = vfs.read_to_end(path).ok()?;
    encoding::all::GBK
        .decode(&bytes, encoding::DecoderTrap::Ignore)
        .ok()
}

fn scripts(vfs: &MiniFs, dirs: &[&str]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for dir in dirs {
        let base = PathBuf::from("/basedata/basedata/DealScript").join(dir);
        let Ok(entries) = vfs.entries(&base) else {
            continue;
        };
        let mut names: Vec<String> = entries
            .flatten()
            .map(|e| e.name.to_string_lossy().to_string())
            .collect();
        names.sort();
        for name in names {
            if let Some(text) = read_gbk(vfs, &base.join(&name)) {
                out.push((format!("{dir}/{name}"), text));
            }
        }
    }
    out
}

#[test]
fn every_shipped_deal_script_parses() {
    let Some(assets) = assets() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };
    let vfs = packfs::init_virtual_fs(&assets, None);
    let scripts = scripts(&vfs, &SHOP_DIRS);
    assert!(
        scripts.len() >= 35,
        "只找到 {} 个店铺脚本，实机有 36 个（另有 13 个客栈）",
        scripts.len()
    );

    for (name, text) in &scripts {
        let shop = DealScript::parse(text);
        assert!(shop.shop_type <= 3, "{name}: 店铺类型 {}", shop.shop_type);
        assert!(shop.temperament <= 5, "{name}: 性格 {}", shop.temperament);
        assert!(!shop.merchant_name.is_empty(), "{name}: 没有商人名字");
        assert!(shop.merchant_id > 0, "{name}: 没有商人 id");
        assert!(
            !shop.begin_talk.is_empty(),
            "{name}: 没有 BeginTalk，多半是分词错了"
        );

        // 能还价的店，五个折扣档都要齐。
        if shop.haggling {
            assert!(shop.patience > 0, "{name}: 能还价却没有忍耐度");
            for t in 0..TIERS {
                let tier = &shop.tiers[t];
                assert!(
                    tier.accept_chance != 0. || tier.refuse_chance != 0.,
                    "{name}: 第 {t} 档没有概率"
                );
                assert!(
                    !tier.accept_talk.is_empty(),
                    "{name}: 第 {t} 档没有接受台词"
                );
                assert!(
                    !tier.force_accept_talk.is_empty(),
                    "{name}: 第 {t} 档没有强制成交台词"
                );
            }
        }
    }
}

#[test]
fn every_item_on_sale_is_in_the_database() {
    let Some(assets) = assets() else { return };
    let vfs = packfs::init_virtual_fs(&assets, None);
    let Some(db) = GameDb::load(&vfs, &PathBuf::from("/basedata/basedata")) else {
        eprintln!("skipping: 数据库读不出来");
        return;
    };

    let mut goods = 0usize;
    for (name, text) in scripts(&vfs, &SHOP_DIRS) {
        let shop = DealScript::parse(&text);
        for id in shop.goods() {
            let item = db
                .item(id)
                .unwrap_or_else(|| panic!("{name}: 卖的 {id} 不在物品表里"));
            assert!(item.price >= 0, "{name}: {id} 的价钱是负的");
            goods += 1;
        }
    }
    assert!(goods > 0, "所有店都没有商品，多半是段落解析错了");
}

#[test]
fn every_shipped_inn_config_parses() {
    let Some(assets) = assets() else { return };
    let vfs = packfs::init_virtual_fs(&assets, None);
    let inns = scripts(&vfs, &["Rest"]);
    assert!(inns.len() >= 10, "只找到 {} 个客栈配置", inns.len());

    for (name, text) in inns {
        let inn = RestScript::parse(&text);
        assert!(!inn.keeper_name.is_empty(), "{name}: 没有掌柜名字");
        assert!(inn.price > 0, "{name}: 住宿价 {}", inn.price);
        assert!(!inn.ask.is_empty(), "{name}: 没有问住不住的话");
        assert!(!inn.rest_end.is_empty(), "{name}: 没有住宿完毕的话");
    }
}
