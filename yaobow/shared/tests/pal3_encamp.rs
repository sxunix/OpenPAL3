//! 宿营小游戏（SCE 106）依赖的实机数据：营地场景、四面方位旗、UI 贴图、
//! 五个角色的 `c09` 动作都在包里；`Encamp.rmk` 的说明文字和
//! `AssisLang.lang` 里用到的槽位读出来是预期的字。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_encamp
//! ```

use std::path::PathBuf;

use common::store_ext::StoreExt2;
use mini_fs::{MiniFs, StoreExt};
use shared::openpal3::assis_lang;
use shared::openpal3::encamp::{
    ASSIS_LANG_PATH, ASSIS_LANG_SECTION, DIR, RMK_PATH, RMK_SECTION, ROLE_ACTION, ROLE_MODELS,
    SCENE_POL, flag_pol, lang,
};

fn vfs() -> Option<MiniFs> {
    let assets = std::env::var("YAOBOW_PAL3_ASSETS").ok()?;
    Some(packfs::init_virtual_fs(&assets, None))
}

fn basedata(rel: &str) -> PathBuf {
    PathBuf::from("/basedata/basedata").join(rel)
}

#[test]
fn scene_flags_textures_and_roles_are_in_the_pack() {
    let Some(vfs) = vfs() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };
    let exists = |rel: &str| {
        let p = basedata(rel);
        assert!(vfs.open(&p).is_ok(), "{} 不在包里", p.display());
    };
    exists(SCENE_POL);
    for site in 0..4 {
        exists(&flag_pol(site).unwrap());
    }
    assert!(flag_pol(-1).is_none());
    assert!(flag_pol(4).is_none());
    for tex in [
        "Remark.tga",
        "TurnTable.tga",
        "Pointer.tga",
        "BlanceBk.tga",
        "JingText.tga",
        "ShenText.tga",
        "BarBk.tga",
        "JingProgress.tga",
        "ShenProgress.tga",
    ] {
        exists(&format!("{DIR}/{tex}"));
    }
    for i in 0..5 {
        exists(&format!("{DIR}/head{i}.tga"));
        exists(&format!("{DIR}/BlanceHead{i}.tga"));
    }
    for model in ROLE_MODELS {
        exists(&format!("ROLE/{model}/{}.MV3", ROLE_ACTION.to_uppercase()));
    }
}

#[test]
fn remark_and_assis_lang_read_the_expected_slots() {
    let Some(vfs) = vfs() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };
    let text = vfs.read_to_end_from_gbk(basedata(RMK_PATH)).unwrap();
    let remark = assis_lang::parse_section(&text, RMK_SECTION);
    assert_eq!(remark.len(), 1, "{remark:?}");
    let remark = assis_lang::unescape(&remark[0]);
    assert!(remark.contains("四方象枢"), "{remark}");
    assert!(remark.contains("青龙、白虎、朱雀、玄武"), "{remark}");

    let text = vfs.read_to_end_from_gbk(basedata(ASSIS_LANG_PATH)).unwrap();
    let table = assis_lang::parse_section(&text, ASSIS_LANG_SECTION);
    assert!(table.len() > lang::ALL, "只读到 {} 条", table.len());
    assert_eq!(table[lang::JING], "精");
    assert_eq!(
        &table[lang::NAMES..lang::NAMES + 5],
        ["景天", "雪见", "龙葵", "紫萱", "长卿"]
    );
    assert_eq!(table[lang::ALL_FULL], "精气神全满");
    assert_eq!(table[lang::FAVOR], "好感");
    assert_eq!(table[lang::RECOVER], "恢复");
    assert_eq!(table[lang::ALL], "全体");
}
