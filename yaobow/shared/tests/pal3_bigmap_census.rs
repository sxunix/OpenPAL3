//! 大地图的目的地表必须和实机的 `ui\bigmap\bigmap.sce` 对得上。
//!
//! [`DESTINATIONS`] 的坐标和图标名是从 exe 的状态类构造函数里抽出来的
//! （`esi + 0x204 + i*0x1dc`），过程号是 `FUN_0044a4f0` 的 `i + 0x64`。这两
//! 边是各自独立的来源，所以拿实机脚本对一遍才算数：38 个下标要一一对上
//! `bigmap.sce` 的 100..137，名字也要能在过程名里找到。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_bigmap_census
//! ```

use std::path::PathBuf;

use common::store_ext::StoreExt2;
use shared::openpal3::bigmap::{
    DESTINATIONS, ELEMENT_DIR, MAP_DIR, SCE_PATH, TILES_X, TILES_Y, proc_of,
};
use shared::openpal3::loaders::sce_loader::sce_load_from_file;

fn assets() -> Option<String> {
    std::env::var("YAOBOW_PAL3_ASSETS").ok()
}

#[test]
fn the_table_matches_the_shipped_bigmap_script() {
    let Some(assets) = assets() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };
    let vfs = packfs::init_virtual_fs(&assets, None);
    let sce = sce_load_from_file(&vfs, &PathBuf::from(SCE_PATH));

    for (i, d) in DESTINATIONS.iter().enumerate() {
        let id = proc_of(i).unwrap();
        let proc = sce
            .procs
            .get(&id)
            .unwrap_or_else(|| panic!("bigmap.sce 里没有 proc {id}（下标 {i}「{}」）", d.name));
        // 过程名形如「从大地图进入雪岭镇」。
        assert!(
            proc.name.contains(d.name),
            "下标 {i} 的名字「{}」和 proc {id}「{}」对不上",
            d.name,
            proc.name
        );
    }

    // 反过来：100..137 之外不该再有目的地过程，`FUN_0044a4f0` 只放行 < 0x26。
    let extra: Vec<u32> = sce
        .procs
        .keys()
        .copied()
        .filter(|id| (100..1000).contains(id) && !(100..138).contains(id))
        .collect();
    assert!(extra.is_empty(), "100..137 之外还有过程: {extra:?}");
}

#[test]
fn every_asset_the_screen_draws_exists() {
    let Some(assets) = assets() else { return };
    let vfs = packfs::init_virtual_fs(&assets, None);
    // 实机把这些存成 .tga；read_picture 会退回 .dds，这里只认在不在。
    let exists = |p: String| vfs.read_to_end(PathBuf::from(&p)).is_ok();

    for n in 0..TILES_X * TILES_Y {
        assert!(
            exists(format!("{MAP_DIR}/BigMap{n}.tga")),
            "缺背景分块 BigMap{n}.tga"
        );
    }
    for (i, d) in DESTINATIONS.iter().enumerate() {
        assert!(
            exists(format!("{ELEMENT_DIR}/{}.tga", d.texture)),
            "下标 {i}「{}」的图标 {}.tga 不在",
            d.name,
            d.texture
        );
    }
}

#[test]
fn the_opening_test_proc_sets_every_destination() {
    let Some(assets) = assets() else { return };
    let vfs = packfs::init_virtual_fs(&assets, None);
    let sce = sce_load_from_file(&vfs, &PathBuf::from(SCE_PATH));

    // 2000 号「大地图开启测试」应当连着 38 条指令 125，把整张表设一遍。
    let proc = sce.procs.get(&2000).expect("bigmap.sce 里没有 proc 2000");
    // 这个过程的形状是固定的：一条 `2 2`（脚本运行模式，8 字节），38 条
    // `125 id option`（各 12 字节：opcode 2 + mask 2 + 两个 i32），再一条
    // `2 1`。长度自己就是校验——8 + 38*12 + 8 = 472。
    const HEAD: usize = 8;
    const STRIDE: usize = 12;
    let inst = &proc.inst;
    assert_eq!(
        inst.len(),
        HEAD + DESTINATIONS.len() * STRIDE + HEAD,
        "proc 2000 的长度不是「一条 2 + 38 条 125 + 一条 2」"
    );

    let mut seen = vec![false; DESTINATIONS.len()];
    for k in 0..DESTINATIONS.len() {
        let pos = HEAD + k * STRIDE;
        let op = u16::from_le_bytes(inst[pos..pos + 2].try_into().unwrap());
        assert_eq!(op, 125, "第 {k} 条不是指令 125 而是 {op}");
        // 参数在文件里是**倒序**的（`command!` 宏的 @reverse_arg 就是为了
        // 这个），所以先 option 后 id。
        let option = i32::from_le_bytes(inst[pos + 4..pos + 8].try_into().unwrap());
        let id = i32::from_le_bytes(inst[pos + 8..pos + 12].try_into().unwrap());
        assert_eq!(id as usize, k, "第 {k} 条设的是目的地 {id}，不是按序来的");
        assert!((0..=2).contains(&option), "目的地 {id} 的状态是 {option}");
        seen[k] = true;
    }
    assert!(seen.iter().all(|s| *s));
}
