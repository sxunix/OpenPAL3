//! `cbdata\flexeff.data` 的两张表必须互相自洽。
//!
//! 这是把 `fileformats::pal3::flexeff` 里那几个常数钉住的检查。最要紧的
//! 一条是：**特效组的计数字段合法区间是 1..=16**，界来自组内布局对 15940
//! 的闭合。判据对不对，看的是每个有效组里的 id 能不能在定义表里查到——
//! 数组长度猜错，或者忘了先把文件前 0x2c047 字节 LZO 解开，这里都会当场
//! 掉到接近零命中。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_flexeff_census
//! ```

use std::path::PathBuf;

use common::store_ext::StoreExt2;
use fileformats::pal3::flexeff::{DEF_COUNT, FlexEff, GROUP_COUNT, GROUP_MAX_ENTRIES, GROUP_SUBS};

fn load() -> Option<FlexEff> {
    let assets = std::env::var("YAOBOW_PAL3_ASSETS").ok()?;
    let vfs = packfs::init_virtual_fs(&assets, None);
    let raw = vfs
        .read_to_end(PathBuf::from("/basedata/basedata/cbdata/flexeff.data"))
        .expect("读不到 cbdata/flexeff.data");
    Some(FlexEff::decode(&raw).expect("flexeff.data 解不开"))
}

#[test]
fn the_two_tables_reference_each_other() {
    let Some(eff) = load() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    // 定义表：绝大多数记录是有效的，剩下的是没填过的槽位。
    let valid: Vec<_> = eff.definitions().collect();
    assert!(
        (1000..=DEF_COUNT).contains(&valid.len()),
        "有效定义 {} 条，落在合理区间外——多半是解压那步没做对",
        valid.len()
    );

    // 组表：计数只会是 0..=16。任何越界都说明布局读错了。
    let mut valid_groups = 0;
    for i in 0..GROUP_COUNT {
        let g = eff.group(i).unwrap();
        let c = g.count();
        assert!(
            (0..=GROUP_MAX_ENTRIES as i32).contains(&c),
            "组 {i} 的计数 {c} 越出 0..=16"
        );
        if g.is_valid() {
            valid_groups += 1;
        }
    }
    assert!(valid_groups > 400, "只有 {valid_groups} 组有效，太少了");

    // 交叉引用：有效组里的每个 id 都要能按 +36 查到一条 +212 非零的记录。
    let (mut total, mut hit) = (0usize, 0usize);
    for g in eff.groups() {
        for id in g.ids() {
            total += 1;
            if eff.find(id).is_some() {
                hit += 1;
            }
        }
    }
    assert!(total > 0, "有效组里一个 id 都没有");
    assert_eq!(hit, total, "{total} 个 id 里只有 {hit} 个查得到");
}

#[test]
fn unused_slots_are_zeroed_and_names_decode() {
    let Some(eff) = load() else { return };

    for g in eff.groups() {
        let c = g.count() as usize;
        // 计数之后的 id 槽从来没被写过。
        for k in c..GROUP_MAX_ENTRIES {
            let o = 4 + k * 4;
            assert!(
                g.raw()[o..o + 4].iter().all(|b| *b == 0),
                "组 {} 的第 {k} 个 id 槽在计数之外却非零",
                g.index
            );
        }
        // 子块的启用标志只取 0/1。
        for k in 0..c {
            for s in 0..GROUP_SUBS {
                let b = g.sub(k, s).unwrap();
                assert!(
                    b[0x58] <= 1,
                    "组 {} 槽 {k} 子块 {s} 的启用标志是 {}",
                    g.index,
                    b[0x58]
                );
            }
        }
    }

    // 资源名要么为空，要么是引擎认识的几种扩展名之一。
    for d in eff.definitions() {
        let a = d.asset().to_lowercase();
        if a.is_empty() {
            continue;
        }
        assert!(
            [".tga", ".par", ".cvd", ".bmp", ".pol"]
                .iter()
                .any(|e| a.ends_with(e))
                || !a.contains('.'),
            "定义 {} 的资源名 {:?} 不像个资源",
            d.index,
            d.asset()
        );
    }
}
