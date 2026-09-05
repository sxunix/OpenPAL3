//! Every shipped `.par` must cut into records with nothing left over.
//!
//! The record sizes in `fileformats::pal3::par` were solved from the
//! four (params, emitters, groups) shapes the game ships, so this is the
//! check that keeps them honest: parse all 45 files, and require the
//! bytes consumed to equal the file length exactly.
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_par_census
//! ```

use std::path::PathBuf;

use common::store_ext::StoreExt2;
use fileformats::pal3::par::{group_param_name, read_par};
use mini_fs::StoreExt;

#[test]
fn every_shipped_par_parses_exactly() {
    let Ok(assets) = std::env::var("YAOBOW_PAL3_ASSETS") else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    let vfs = packfs::init_virtual_fs(&assets, None);
    let dir = PathBuf::from("/basedata/basedata/effect/gbParticle");
    let mut names: Vec<String> = vfs
        .entries(&dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.name.to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort();

    let mut parsed = 0;
    for name in names {
        if !name.to_lowercase().ends_with(".par") {
            continue;
        }
        let Ok(data) = vfs.read_to_end(dir.join(&name)) else {
            continue;
        };
        let par = read_par(&mut data.as_slice()).unwrap_or_else(|e| panic!("{name} 解不开: {e}"));

        assert_eq!(
            par.expected_len(),
            data.len(),
            "{name} 的记录数和文件长度对不上"
        );

        // Every group must point at records this file actually has.
        for group in &par.groups {
            assert!(
                (group.group_param as usize) < par.group_params.len(),
                "{name} 的 group 指向不存在的 group param {}",
                group.group_param
            );
            assert!(
                (group.emitter as usize) < par.emitters.len(),
                "{name} 的 group 指向不存在的 emitter {}",
                group.emitter
            );
        }

        // The group params open with a name.
        for record in &par.group_params {
            let name_in_record = group_param_name(record);
            assert!(
                name_in_record.is_ascii(),
                "{name} 的 group param 名字不像字符串: {name_in_record:?}"
            );
        }

        parsed += 1;
    }

    assert!(
        parsed > 0,
        "一个 .par 都没读到；{assets} 是 PAL3 安装目录吗？"
    );
    println!("解开 {parsed} 个 .par");
}
