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

/// 控件矩形的宽高 = 贴图里那块画面的宽高，画面贴在 2 的幂画布的**左下角**。
/// 这条规矩是整套版面常量的地基（见 `openpal3::appraise::layout` 开头），
/// 拿包里的 `.tga` 逐张对一遍。
#[test]
fn the_control_rects_match_the_art_inside_each_canvas() {
    use shared::openpal3::appraise::layout;

    let Some(vfs) = vfs() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    // (贴图, 控件矩形, 容差)：画面边上总有一两列几乎全透明的像素，所以
    // 不要求分毫不差；`doingBk` 差得最多，也只有 3×5。
    let cases: [(&str, layout::Rect, i32); 8] = [
        ("SubGame/Appraise/Do/TimeBarbk.tga", layout::TIME_BAR, 2),
        (
            "SubGame/Appraise/Exhibit/word_type.tga",
            layout::WORD_TYPE,
            2,
        ),
        ("SubGame/Appraise/Do/Result0.tga", layout::RESULT0, 2),
        ("SubGame/Appraise/Do/Result1.tga", layout::RESULT1, 2),
        (
            "SubGame/Appraise/Exhibit/ExhibitRemark.tga",
            layout::REMARK,
            2,
        ),
        (
            "SubGame/Appraise/Exhibit/Do0.tga",
            layout::EXHIBIT_DO_BUTTON,
            2,
        ),
        (
            "SubGame/Appraise/Exhibit/ExhibitBk.tga",
            layout::EXHIBIT_BK,
            2,
        ),
        ("SubGame/Appraise/Do/doingBk.tga", layout::DO_BK, 5),
    ];

    for (rel, rect, slack) in cases {
        let raw = vfs.read_to_end(basedata(rel)).unwrap();
        // 32 位未压缩 TGA，image descriptor 的第 5 位为 0 = 左下角原点。
        assert_eq!(raw[2], 2, "{rel} 不是未压缩真彩 TGA");
        assert_eq!(raw[17] & 0x20, 0, "{rel} 不是左下角原点");
        let w = u16::from_le_bytes([raw[12], raw[13]]) as usize;
        let h = u16::from_le_bytes([raw[14], raw[15]]) as usize;
        let bypp = raw[16] as usize / 8;
        let off = 18 + raw[0] as usize;

        // 画面的包围盒，行号按文件顺序（第 0 行是图的最下面一行）。
        let (mut max_x, mut max_row) = (0usize, 0usize);
        let (mut min_x, mut min_row) = (w, h);
        for row in 0..h {
            for x in 0..w {
                let p = off + (row * w + x) * bypp;
                let solid = if bypp == 4 {
                    raw[p + 3] != 0
                } else {
                    raw[p] | raw[p + 1] | raw[p + 2] != 0
                };
                if solid {
                    min_x = min_x.min(x);
                    max_x = max_x.max(x + 1);
                    min_row = min_row.min(row);
                    max_row = max_row.max(row + 1);
                }
            }
        }
        assert_eq!(min_x, 0, "{rel} 的画面没有贴在左边");
        assert_eq!(min_row, 0, "{rel} 的画面没有贴在下边");

        let (rw, rh) = (rect.2 - rect.0, rect.3 - rect.1);
        assert!(
            (max_x as i32 - rw).abs() <= slack,
            "{rel}: 画面宽 {max_x}，控件宽 {rw}"
        );
        assert!(
            (max_row as i32 - rh).abs() <= slack,
            "{rel}: 画面高 {max_row}，控件高 {rh}"
        );
    }
}
