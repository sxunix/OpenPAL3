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

use std::collections::BTreeMap;
use std::path::PathBuf;

use common::store_ext::StoreExt2;
use fileformats::pal3::flexeff::{
    DEF_COUNT, EFFECT_CLASSES, FlexEff, GROUP_COUNT, GROUP_MAX_ENTRIES, GROUP_SUB_USED, GROUP_SUBS,
    name_crc,
};

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
        // 子块的四个标志字节只取 0/1，最后一个从来没被用过；`+96` 往后的
        // 28 字节也一直是零。
        for k in 0..c {
            for s in 0..GROUP_SUBS {
                let b = g.sub(k, s).unwrap();
                let flags = g.sub_flags(k, s).unwrap();
                assert!(
                    flags.iter().all(|f| *f <= 1),
                    "组 {} 槽 {k} 子块 {s} 的标志字节是 {flags:?}",
                    g.index
                );
                assert_eq!(
                    flags[3], 0,
                    "组 {} 槽 {k} 子块 {s} 用上了第 4 个标志",
                    g.index
                );
                assert!(
                    b[GROUP_SUB_USED..].iter().all(|x| *x == 0),
                    "组 {} 槽 {k} 子块 {s} 的尾部 28 字节非零",
                    g.index
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

#[test]
fn every_definition_names_a_registered_class() {
    let Some(eff) = load() else { return };

    // 我这份 CRC 表是现算的，跟 packfs 给 cpk 文件名用的那份必须一致——
    // `FUN_004d58d0` 和 cpk 的校验和本来就是同一套。
    for n in EFFECT_CLASSES {
        assert_eq!(
            name_crc(n.as_bytes()),
            packfs::cpk::crc_checksum(n.as_bytes()),
            "类名 {n} 的 CRC 两边算不一样"
        );
    }

    // 1263 条有效定义的 +0 全都是这 15 个类之一，没有漏网的。
    let mut per_class: BTreeMap<&str, usize> = BTreeMap::new();
    for d in eff.definitions() {
        let class = d.class().unwrap_or_else(|| {
            panic!(
                "定义 {} 的类 CRC 0x{:08x} 不在注册表里",
                d.index,
                d.class_crc()
            )
        });
        *per_class.entry(class).or_insert(0) += 1;
    }
    assert_eq!(
        per_class.len(),
        EFFECT_CLASSES.len(),
        "只用到了 {} 个类: {per_class:?}",
        per_class.len()
    );

    // 类和资源类型要对得上：particle 一定是 .par，model 一定是模型。
    for d in eff.definitions() {
        let a = d.asset().to_lowercase();
        match d.class().unwrap() {
            "particle" => assert!(a.ends_with(".par"), "particle 却指向 {a:?}"),
            "model" => assert!(
                a.ends_with(".cvd") || a.ends_with(".pol"),
                "model 却指向 {a:?}"
            ),
            _ => {}
        }
    }
}
