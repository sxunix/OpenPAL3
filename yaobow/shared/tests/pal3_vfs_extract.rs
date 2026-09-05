//! Copy a subtree of the mounted PAL3 vfs to a plain directory, so that
//! resources living inside `.cpk` packages can be looked at with ordinary
//! tools. Study aid only; skipped unless both variables are set.
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 \
//! YAOBOW_EXTRACT=/basedata/basedata/SubGame YAOBOW_EXTRACT_TO=/tmp/out \
//!   cargo test -p shared --test pal3_vfs_extract -- --ignored --nocapture
//! ```

use std::path::{Path, PathBuf};

use common::store_ext::StoreExt2;
use mini_fs::{MiniFs, StoreExt};

fn walk(vfs: &MiniFs, dir: &Path, out: &Path, copied: &mut usize) {
    let Ok(entries) = vfs.entries(dir) else {
        return;
    };
    let entries: Vec<_> = entries.flatten().collect();
    for e in entries {
        let name = e.name.to_string_lossy().to_string();
        let src = dir.join(&name);
        let dst = out.join(&name);
        match vfs.read_to_end(&src) {
            Ok(data) => {
                let _ = std::fs::create_dir_all(dst.parent().unwrap());
                std::fs::write(&dst, data).unwrap();
                *copied += 1;
            }
            // 读不到就当目录再往下走
            Err(_) => walk(vfs, &src, &dst, copied),
        }
    }
}

#[test]
#[ignore]
fn extract_vfs_subtree() {
    let (Ok(assets), Ok(from), Ok(to)) = (
        std::env::var("YAOBOW_PAL3_ASSETS"),
        std::env::var("YAOBOW_EXTRACT"),
        std::env::var("YAOBOW_EXTRACT_TO"),
    ) else {
        eprintln!("set YAOBOW_PAL3_ASSETS, YAOBOW_EXTRACT and YAOBOW_EXTRACT_TO");
        return;
    };
    let vfs = packfs::init_virtual_fs(&assets, None);
    let mut copied = 0;
    // 单个文件也可以：YAOBOW_EXTRACT 指向文件时 TO 就是目标文件名
    if let Ok(data) = vfs.read_to_end(PathBuf::from(&from)) {
        let dst = PathBuf::from(&to);
        let _ = std::fs::create_dir_all(dst.parent().unwrap());
        std::fs::write(&dst, data).unwrap();
        copied = 1;
    } else {
        walk(
            &vfs,
            &PathBuf::from(&from),
            &PathBuf::from(&to),
            &mut copied,
        );
    }
    println!("{from} -> {to}: {copied} 个文件");
    assert!(copied > 0, "{from} 下没有文件");
}
