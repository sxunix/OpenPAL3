//! 鉴定小游戏（SCE 104）依赖的实机数据：五对文物在 gdb 里的名字和模型、
//! 模型文件都在包里、`AssisLang.lang` / `Appraise.dft` 用 AssisLang 的读法
//! 能读出预期的条目。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_appraise
//! ```

use std::io::Cursor;
use std::path::PathBuf;

use common::store_ext::StoreExt2;
use fileformats::pal3::gdb::GdbFile;
use mini_fs::{MiniFs, StoreExt};
use shared::openpal3::appraise::{
    ASSIS_LANG_PATH, ASSIS_LANG_SECTION, DFT_PATH, DFT_SECTION, MODEL_DIR, PAIRS, lang,
};
use shared::openpal3::assis_lang;

const GDB_PATH: &str = "/basedata/basedata/cbdata/pal3_softstar.gdb";

fn vfs() -> Option<MiniFs> {
    let assets = std::env::var("YAOBOW_PAL3_ASSETS").ok()?;
    Some(packfs::init_virtual_fs(&assets, None))
}

fn basedata(rel: &str) -> PathBuf {
    PathBuf::from("/basedata/basedata").join(rel)
}

#[test]
fn five_pairs_have_names_and_models() {
    let Some(vfs) = vfs() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };
    let raw = vfs.read_to_end(PathBuf::from(GDB_PATH)).unwrap();
    let gdb = <GdbFile as binrw::BinRead>::read(&mut Cursor::new(raw)).unwrap();
    let item = |id: u32| gdb.items.iter().find(|i| i.id == id).unwrap();

    let expected = [
        ("兽面纹爵", "D25", "J01"),
        ("玉双龙首璜", "D21", "J02"),
        ("玉蝉出牙环", "D23", "J03"),
        ("青瓷薰炉", "D10", "J04"),
        ("白釉烛台", "D01", "J05"),
    ];
    for ((real, fake), (name, real_model, fake_model)) in PAIRS.iter().zip(expected) {
        let r = item(*real);
        let f = item(*fake);
        assert_eq!(r.name.as_str().unwrap(), name, "真品 {real}");
        assert_eq!(
            f.name.as_str().unwrap(),
            format!("{name}赝品"),
            "赝品 {fake}"
        );
        assert_eq!(r.model_name.as_str().unwrap(), real_model);
        assert_eq!(f.model_name.as_str().unwrap(), fake_model);
        for m in [real_model, fake_model] {
            let path = basedata(&format!("{MODEL_DIR}/{m}.pol"));
            assert!(vfs.open(&path).is_ok(), "{} 不在包里", path.display());
        }
    }
}

#[test]
fn assis_lang_and_dft_read_the_expected_slots() {
    let Some(vfs) = vfs() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };
    let text = vfs.read_to_end_from_gbk(basedata(ASSIS_LANG_PATH)).unwrap();
    let table = assis_lang::parse_section(&text, ASSIS_LANG_SECTION);
    assert!(table.len() > lang::FAILED, "只读到 {} 条", table.len());
    assert_eq!(table[lang::NONE], "无");
    assert_eq!(table[lang::ASK], "要玩鉴定游戏吗？");
    assert_eq!(table[lang::TIME_LEFT], "剩余时间");
    assert_eq!(table[lang::MAGNIFIERS_LEFT], "剩余放大镜");
    assert_eq!(table[lang::SUCCESS], "鉴定成功");
    assert_eq!(table[lang::TOTAL_SCORE], "总分数");
    assert_eq!(table[lang::ITEMS_GAINED], "获得物品");
    assert_eq!(table[lang::FAILED], "鉴定失败");

    let text = vfs.read_to_end_from_gbk(basedata(DFT_PATH)).unwrap();
    let diffent = assis_lang::parse_section(&text, DFT_SECTION);
    assert_eq!(diffent.len(), 5, "{diffent:?}");
    assert!(diffent.iter().all(|d| !d.is_empty()));
}
