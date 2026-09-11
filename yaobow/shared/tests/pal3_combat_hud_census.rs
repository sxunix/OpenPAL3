//! 拿实机的包核对战斗界面（`combat::hud`）用到的贴图：每一张都在，
//! 尺寸和版面里的假设对得上。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_combat_hud_census
//! ```

use std::path::PathBuf;

use common::store_ext::StoreExt2;
use shared::openpal3::combat::hud;

fn vfs() -> Option<mini_fs::MiniFs> {
    let assets = std::env::var("YAOBOW_PAL3_ASSETS").ok()?;
    Some(packfs::init_virtual_fs(&assets, None))
}

fn path(rel: &str) -> PathBuf {
    PathBuf::from("/basedata/basedata")
        .join(hud::UI_DIR)
        .join(rel)
}

/// TGA 头里的宽高（偏移 12、14，小端 u16）。
fn tga_size(vfs: &mini_fs::MiniFs, rel: &str) -> Option<(u32, u32)> {
    let bytes = vfs.read_to_end(path(rel)).ok()?;
    let w = u16::from_le_bytes([*bytes.get(12)?, *bytes.get(13)?]) as u32;
    let h = u16::from_le_bytes([*bytes.get(14)?, *bytes.get(15)?]) as u32;
    Some((w, h))
}

#[test]
fn every_texture_the_hud_names_is_shipped() {
    let Some(vfs) = vfs() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };
    let all = hud::every_texture();
    let missing: Vec<&String> = all
        .iter()
        .filter(|rel| vfs.read_to_end(path(rel)).is_err())
        .collect();
    assert!(missing.is_empty(), "包里没有：{missing:?}");
    println!("战斗界面贴图 {} 张都在", all.len());
}

#[test]
fn texture_sizes_match_what_the_layout_assumes() {
    let Some(vfs) = vfs() else { return };
    let size = |rel: &str| tga_size(&vfs, rel).unwrap_or_else(|| panic!("{rel} 读不出尺寸"));

    // 盘底 256×256，正好是面板 rect。
    assert_eq!(size(hud::CONTROL_BACK), (256, 256));
    // 八个按钮 64×64，rect 46×46 截左下角；魔 128×128，rect 84×84。
    for b in hud::RingButton::ALL {
        let expect = if b == hud::RingButton::Mj {
            (128, 128)
        } else {
            (64, 64)
        };
        assert_eq!(
            size(&hud::button_texture(b, hud::ButtonLook::Normal)),
            expect,
            "{b:?}"
        );
    }
    // 状态面板 256×128，rect 157×106。
    assert_eq!(size(hud::panel_texture(0)), (256, 128));
    assert_eq!(size(hud::panel_texture(1)), (256, 128));
    // 槽 128×16、填充 128×8。
    assert_eq!(size(hud::GAUGE_BACK), (128, 16));
    for g in hud::Gauge::ALL {
        assert_eq!(size(g.fill_texture()), (128, 8), "{g:?}");
    }
    // 头像 64×128，名字 32×32。
    for slot in 0..hud::PORTRAIT_NAMES.len() {
        assert_eq!(size(&hud::portrait_texture(slot).unwrap()), (64, 128));
        assert_eq!(size(&hud::name_texture(slot).unwrap()), (32, 32));
    }
    // 行动条底 512×32，rect 只有 320 宽。
    assert_eq!(size(hud::SEQUENCE_BACK), (512, 32));
    let r = hud::SEQUENCE_BACK_RECT;
    assert_eq!((r.2 - r.0, r.3 - r.1), (320, 32));
    // 列表底 256×256，rect 249×206。
    assert_eq!(size(hud::MAGIC_BACK), (256, 256));
    assert_eq!(size(hud::ITEM_BACK), (256, 256));
    // 招式名横幅 256×64。
    assert_eq!(size(hud::TRICK_BACK), (256, 64));
}

#[test]
fn every_party_member_has_a_sequence_head() {
    let Some(vfs) = vfs() else { return };
    for model in hud::PARTY_HEADS {
        let rel = hud::head_texture(model);
        assert!(vfs.read_to_end(path(&rel)).is_ok(), "{rel} 不在包里");
    }
    // 舞台用的 105 在这里没有图——龙葵的头像只能是 106。
    assert!(vfs.read_to_end(path(&hud::head_texture("105"))).is_err());
}
