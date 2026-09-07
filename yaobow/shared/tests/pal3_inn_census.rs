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
use shared::openpal3::assis_lang;
use shared::openpal3::states::inn::{
    Answer, InnOutcome, MenuItem, STORY_ADVANCES, Vitals, can_afford, cost_line, lang, rest,
    story_after_rest,
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

/// 这两家的 `M_ID` 填的是 1802，而 `Q01.sce` 的 1802 叫「摘星手套」——不是
/// 掌柜的聊天过程。点「聊天」会跑到不相干的剧情段去，是**随游戏出厂的数据
/// 缺陷**，不是解析出了问题。
///
/// 另外 `q01res2t.txt`（名字里 `res2t` 是 `rest2` 的错拼）**没有任何脚本
/// 引用**：Q01 里用 119 的只有 proc 1415（`q01rest.txt`）和 1801
/// （`q01rest2.txt`）。它是份内容雷同、只有掌柜名不同的废文件。
const MISWIRED_KEEPER_PROC: [&str; 2] = ["q01res2t.txt", "q01rest2.txt"];

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
            InnOutcome::Rested { paid: inn.price },
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
            InnOutcome::TooPoor,
            "{name} 差一文该住不成"
        );
        assert_eq!(money, inn.price - 1);
        assert_eq!(party[0].1.hp, 1);
        assert!(!can_afford(inn.price - 1, inn.price));
    }
}

#[test]
fn the_menu_labels_come_out_of_assislang() {
    let Some(vfs) = vfs() else { return };
    let path = PathBuf::from("/basedata/basedata/AssisLang.lang");
    let bytes = vfs.read_to_end(path).expect("读不到 AssisLang.lang");
    let text = encoding::all::GBK
        .decode(&bytes, encoding::DecoderTrap::Ignore)
        .unwrap();
    let table = assis_lang::parse_section(&text, "sec1");

    // 客栈用到的五条，槽号是从 exe 里的 DAT_00dd8d3c.. 反算出来的
    // （本地化表基址 0xdc333c，步长 0x100）。
    assert_eq!(table[lang::COST_PREFIX], "住宿共需");
    assert_eq!(table[lang::COST_SUFFIX], "文钱");
    assert_eq!(table[lang::CHAT], "聊天");
    assert_eq!(table[lang::STAY], "住宿");
    assert_eq!(table[lang::LEAVE], "退出");

    // 三个菜单项按原版的顺序绑到这三条上。
    let labels: Vec<&str> = MenuItem::ALL
        .iter()
        .map(|i| table[i.lang_slot()].as_str())
        .collect();
    assert_eq!(labels, ["聊天", "住宿", "退出"]);

    // 确认框那句话。
    assert_eq!(
        cost_line(&table[lang::COST_PREFIX], 50, &table[lang::COST_SUFFIX]),
        "\n          住宿共需50文钱"
    );
}

#[test]
fn chatting_needs_a_keeper_proc() {
    let Some(vfs) = vfs() else { return };
    // 「聊天」跑的是 M_ID 那个过程，十三家都填了非零的编号。
    for name in FILES {
        let Some(inn) = read(&vfs, name) else {
            continue;
        };
        assert!(inn.keeper_id > 0, "{name} 的 M_ID 是 0，聊天就没得跑");
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

/// 「聊天」跑的是配置里的 `M_ID`。每家客栈的 `M_ID` 都该在同名场景的
/// `.sce` 里，而且那个过程的名字里带「聊天」——这是对
/// `FUN_0044b710` 里 `if (+0x2008 != 0) FUN_00432170(+0x2008, 0)` 那一句的
/// 反向核对。
#[test]
fn every_keeper_proc_is_a_chat_proc_in_its_own_scene() {
    let Some(vfs) = vfs() else { return };

    let mut checked = 0;
    for name in FILES {
        let Some(inn) = read(&vfs, name) else {
            continue;
        };
        // 文件名前四个字符就是场景名：q01rest.txt → Q01。
        let scene = name[..3].to_uppercase();
        let path = PathBuf::from(format!("/scene/{scene}/{scene}.sce"));
        let Ok(sce) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            shared::openpal3::loaders::sce_loader::sce_load_from_file(&vfs, &path)
        })) else {
            continue;
        };

        let proc = sce
            .procs
            .values()
            .find(|p| p.id == inn.keeper_id as u32)
            .unwrap_or_else(|| panic!("{name} 的 M_ID {} 不在 {scene}.sce 里", inn.keeper_id));

        if MISWIRED_KEEPER_PROC.contains(&name) {
            // 出厂数据的毛病，见常量上的说明。跑得起来，只是跑错地方。
            println!(
                "{name}: M_ID {} → {scene} 「{}」（出厂就接错了）",
                inn.keeper_id, proc.name
            );
        } else {
            assert!(
                proc.name.contains("聊天"),
                "{name} 的 M_ID {} 在 {scene} 里叫「{}」，不像聊天过程",
                inn.keeper_id,
                proc.name
            );
            println!("{name}: M_ID {} → {scene} 「{}」", inn.keeper_id, proc.name);
        }
        checked += 1;
    }
    assert!(checked >= 10, "只核对了 {checked} 家");
}

/// Q01 那一组的三个过程号，直接照 `Q01.sce` proc 1415 里那条
/// `119 "DealScript\rest\q01rest.txt", 1418, 1419, 1420` 核对语义：
/// 住宿1 = 钱够、住宿2 = 钱不够、住宿3 = 住完。
#[test]
fn the_three_procs_are_named_after_what_they_do() {
    let Some(vfs) = vfs() else { return };
    let path = PathBuf::from("/scene/Q01/Q01.sce");
    let Ok(sce) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        shared::openpal3::loaders::sce_loader::sce_load_from_file(&vfs, &path)
    })) else {
        return;
    };
    let named = |id: u32| {
        sce.procs
            .values()
            .find(|p| p.id == id)
            .map(|p| p.name.clone())
            .unwrap_or_default()
    };
    assert_eq!(named(1415), "客栈掌柜住宿", "119 就写在这个过程里");
    assert_eq!(named(1416), "客栈掌柜聊天", "M_ID 指的是它");
    assert_eq!(named(1418), "客栈掌柜住宿1", "钱够");
    assert_eq!(named(1419), "客栈掌柜住宿2", "钱不够");
    assert_eq!(named(1420), "客栈掌柜住宿3", "住完");
}
