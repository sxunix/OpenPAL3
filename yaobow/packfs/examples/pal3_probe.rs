//! Mount a PAL3 install directory with the same code the game uses and
//! report what came up. Run on the host to separate "this data set is fine"
//! from "the target platform cannot read it":
//!
//!     cargo run -p packfs --example pal3_probe -- /path/to/PAL3
//!
//! Lists every mounted package, then opens the handful of paths the game
//! touches first (init.sce, a scene .scn, a music track) and reports their
//! sizes and leading bytes.

use std::io::Read;
use std::path::PathBuf;

use mini_fs::StoreExt;
use packfs::init_virtual_fs_with_catalog;

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: pal3_probe <PAL3 dir>");

    let (vfs, catalog) = init_virtual_fs_with_catalog(&root, None);

    let mut by_type = std::collections::BTreeMap::new();
    for m in catalog.mounts() {
        *by_type.entry(format!("{:?}", m.package_type)).or_insert(0usize) += 1;
    }
    println!("mounted {} package(s) from {}", catalog.mounts().len(), root.display());
    for (t, n) in &by_type {
        println!("  {t}: {n}");
    }
    let mut mounts: Vec<_> = catalog
        .mounts()
        .iter()
        .map(|m| m.vfs_mount_point.display().to_string())
        .collect();
    mounts.sort();
    println!("  mount points: {}", mounts.join(" "));

    // What the game opens first, in the order it opens them.
    let probes = [
        "/basedata/basedata/init.sce",
        "/basedata/basedata/ui/box/box.tga",
        "/scene/Q01/Q01.scn",
        "/scene/Q01/Q01.nav",
        "/scene/M01/M01.scn",
        "/music/music/music/PI01.mp3",
    ];
    let mut ok = 0;
    for p in probes {
        match vfs.open(p) {
            Ok(mut f) => {
                let mut buf = Vec::new();
                match f.read_to_end(&mut buf) {
                    Ok(n) => {
                        let head: Vec<String> =
                            buf.iter().take(8).map(|b| format!("{b:02x}")).collect();
                        println!("  OK   {p:<38} {n:>10} bytes  [{}]", head.join(" "));
                        ok += 1;
                    }
                    Err(e) => println!("  READ {p:<38} {e}"),
                }
            }
            Err(e) => println!("  MISS {p:<38} {e}"),
        }
    }

    // A directory listing through the VFS, which is what the scene loader
    // does when it looks for per-scene variants.
    match vfs.entries("/scene/Q01") {
        Ok(entries) => {
            let names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.name.to_string_lossy().into_owned())
                .take(12)
                .collect();
            println!("  /scene/Q01 first entries: {}", names.join(" "));
        }
        Err(e) => println!("  entries(/scene/Q01) failed: {e}"),
    }

    println!("{ok}/{} probe paths readable", probes.len());
    if catalog.mounts().is_empty() || ok == 0 {
        std::process::exit(1);
    }
}
