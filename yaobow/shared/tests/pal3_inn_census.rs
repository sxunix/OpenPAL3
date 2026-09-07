//! 拿实机的 `DealScript/Rest/*.txt` 跑一遍客栈配置的解析，再把住店规则
//! 在每一家上都算一遍。
//!
//! 逆向记录在 `switch/re/notes/inn.md`。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_inn_census
//! ```

use std::path::PathBuf;

use common::store_ext::StoreExt2;
use encoding::Encoding;
use mini_fs::MiniFs;
use shared::openpal3::states::inn::{
    Answer, RestOutcome, STORY_ADVANCES, Vitals, can_afford, rest, story_after_rest,
};
use shared::openpal3::states::shop::RestScript;

/// 随游戏出厂的十三家客栈。
const FILES: [&str; 13] = [
    "q01res2t.txt",
    "q01rest.txt",
    "q01rest2.txt",
    "q02rest.txt",
    "q03rest.txt",
    "q06rest.txt",
    "q07rest.txt",
    "q09rest.txt",
    "q11rest.txt",
    "q12rest.txt",
    "q13rest.txt",
    "q15rest.txt",
    "q16rest.txt",
];

fn vfs() -> Option<MiniFs> {
    let assets = std::env::var("YAOBOW_PAL3_ASSETS").ok()?;
    Some(packfs::init_virtual_fs(&assets, None))
}

fn read(vfs: &MiniFs, name: &str) -> Option<RestScript> {
    let path = PathBuf::from(format!("/basedata/basedata/DealScript/Rest/{name}"));
    let bytes = vfs.read_to_end(path).ok()?;
    let text = encoding::all::GBK
        .decode(&bytes, encoding::DecoderTrap::Ignore)
        .ok()?;
    Some(RestScript::parse(&text))
}

#[test]
fn every_shipped_inn_parses() {
    let Some(vfs) = vfs() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    let mut found = 0;
    for name in FILES {
        let Some(inn) = read(&vfs, name) else {
            panic!("读不到 {name}");
        };
        found += 1;

        assert!(inn.keeper_id > 0, "{name} 的 M_ID 没读出来");
        assert!(!inn.keeper_name.is_empty(), "{name} 的 M_Name 空的");
        assert!(
            inn.price > 0,
            "{name} 的住宿价 {}，配置里都是正数",
            inn.price
        );
        for (field, text) in [
            ("M_Rest", &inn.ask),
            ("M_Enough", &inn.enough),
            ("M_NoEnough", &inn.not_enough),
            ("M_RestEnd", &inn.rest_end),
        ] {
            assert!(!text.is_empty(), "{name} 的 {field} 空的");
            assert!(
                !text.contains("\\n"),
                "{name} 的 {field} 里 \\n 没有转义开: {text}"
            );
            assert!(!text.contains('&'), "{name} 的 {field} 末尾的 & 没去掉");
        }
        println!(
            "{name}: {} 号掌柜「{}」，住一晚 {} 文",
            inn.keeper_id, inn.keeper_name, inn.price
        );
    }
    assert_eq!(found, FILES.len());
}

#[test]
fn a_night_at_every_inn_refills_and_charges() {
    let Some(vfs) = vfs() else { return };

    for name in FILES {
        let Some(inn) = read(&vfs, name) else {
            continue;
        };

        let hurt = Vitals {
            hp: 1,
            max_hp: 500,
            sp: 2,
            max_sp: 60,
            mp: 3,
            max_mp: 300,
        };

        // 钱正好够。
        let mut money = inn.price;
        let mut story = 0;
        let mut party = [(true, hurt), (true, hurt), (false, hurt)];
        assert_eq!(
            rest(Answer::Rest, &mut money, inn.price, &mut party, &mut story),
            RestOutcome::Rested { paid: inn.price },
            "{name} 钱正好够该住得成"
        );
        assert_eq!(money, 0, "{name} 该正好花光");
        assert_eq!(party[0].1.hp, 500);
        assert_eq!(party[0].1.mp, 300);
        assert_eq!(party[0].1.sp, 2, "{name} 气不该回");
        assert_eq!(party[2].1.hp, 1, "{name} 不在队上的不该回");

        // 差一文就住不成，什么也不动。
        let mut money = inn.price - 1;
        let mut party = [(true, hurt)];
        assert_eq!(
            rest(Answer::Rest, &mut money, inn.price, &mut party, &mut story),
            RestOutcome::TooPoor,
            "{name} 差一文该住不成"
        );
        assert_eq!(money, inn.price - 1);
        assert_eq!(party[0].1.hp, 1);
        assert!(!can_afford(inn.price - 1, inn.price));
    }
}

#[test]
fn resting_advances_exactly_two_story_numbers() {
    // 原版只对这两个值做了特判，别的一律不动。
    assert_eq!(STORY_ADVANCES.len(), 2);
    for (from, to) in STORY_ADVANCES {
        assert_eq!(story_after_rest(from), to);
        assert_ne!(from, to);
        // 推进之后再住一次不会继续动。
        assert_eq!(story_after_rest(to), to, "{to} 不该被再推一次");
    }
    for other in [0, 1, 11100, 11102, 11199, 120300, 120302, 130400, 999_999] {
        assert_eq!(story_after_rest(other), other, "{other} 不该被改");
    }
}
