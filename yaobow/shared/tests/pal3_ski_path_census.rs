//! 拿实机的 `Skee_path.ASE` 跑一遍雪道路径解析器。
//!
//! 逆向记录在 `switch/re/notes/ski.md`：原版 `FUN_0052d7c0` 扫这个文件里名叫
//! `skee_path` 的物体，把顶点摊成每点 32 字节（xyz、累计距离、段长），z 取反。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_ski_path_census
//! ```

use std::path::PathBuf;

use common::store_ext::StoreExt2;
use encoding::Encoding;
use mini_fs::MiniFs;
use shared::openpal3::states::ski_path::SkiPath;

/// exe 里写死的是根目录那一份；`SkeeScene/Scene/` 下还有一份一样的。
const CANDIDATES: [&str; 2] = [
    "/basedata/basedata/SubGame/Skee/SkeeScene/Scene/Skee_path.ASE",
    "/basedata/basedata/Skee_path.ASE",
];

fn read_text(vfs: &MiniFs, path: &str) -> Option<String> {
    let bytes = vfs.read_to_end(PathBuf::from(path)).ok()?;
    encoding::all::GBK
        .decode(&bytes, encoding::DecoderTrap::Ignore)
        .ok()
}

fn path() -> Option<SkiPath> {
    let assets = std::env::var("YAOBOW_PAL3_ASSETS").ok()?;
    let vfs = packfs::init_virtual_fs(&assets, None);
    for p in CANDIDATES {
        if let Some(text) = read_text(&vfs, p) {
            return Some(SkiPath::parse(&text));
        }
    }
    None
}

#[test]
fn the_shipped_slope_parses() {
    let Some(path) = path() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    // 文件头写着 *MESH_NUMVERTEX 4950。
    assert_eq!(path.points.len(), 4950, "顶点数和文件头对不上");

    // 雪道沿 x 往下走：起点在 −10220 一带，终点在 +9020 一带。
    let first = path.points[0];
    let last = *path.points.last().unwrap();
    assert!(first.x < -10_000., "起点 x = {}", first.x);
    assert!(last.x > 9_000., "终点 x = {}", last.x);

    // 累计距离单调不减，段长和相邻两点的间距对得上。
    let mut prev = 0.;
    for (i, p) in path.points.iter().enumerate() {
        assert!(p.distance >= prev, "第 {i} 个点的累计距离倒退了");
        assert!(p.step >= 0.);
        prev = p.distance;
    }
    assert_eq!(path.points[0].step, 0., "第一个点没有前一段");
    assert!(path.length() > 19_000., "全长 {}", path.length());

    // 点铺得很密：平均段长在十几个单位，没有大跳。
    let avg = path.length() / (path.points.len() - 1) as f32;
    assert!((1. ..40.).contains(&avg), "平均段长 {avg}");
    let max_step = path
        .points
        .iter()
        .map(|p| p.step)
        .fold(0f32, |a, b| a.max(b));
    assert!(max_step < 200., "最长的一段 {max_step}，路径可能断了");
}

#[test]
fn walking_the_whole_slope_stays_on_it() {
    let Some(path) = path() else { return };
    let len = path.length();
    // 沿路径走一百步，每一步的落点都该离前一步不远。
    let mut last = path.at(0.).unwrap();
    for i in 1..=100 {
        let d = len * i as f32 / 100.;
        let p = path.at(d).unwrap();
        let gap = ((p.0 - last.0).powi(2) + (p.1 - last.1).powi(2) + (p.2 - last.2).powi(2)).sqrt();
        assert!(gap <= len / 100. * 1.5 + 1., "第 {i} 步跳了 {gap}");
        last = p;
    }
    assert_eq!(path.at(len), Some(last), "走到底就是最后一个点");
}
