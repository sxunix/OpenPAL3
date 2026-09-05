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
use fileformats::pal3::par::{
    EMITTER_TRACK_LABELS, EMITTER_TRACKS, GROUP_PARAM_TRACKS, GROUP_PARAM_UNKNOWN_FLAG,
    GROUP_PARAM_UNUSED, ParBlend, TRACK_LABELS, emitter_name, group_param_blend, group_param_name,
    group_param_texture, read_par, track_curve, track_label,
};
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

        // The group params open with a name and carry a texture path.
        for record in &par.group_params {
            let name_in_record = group_param_name(record);
            assert!(
                name_in_record.is_ascii() && !name_in_record.is_empty(),
                "{name} 的 group param 名字不像字符串: {name_in_record:?}"
            );
            let texture = group_param_texture(record);
            assert!(
                texture.starts_with("partexture\\"),
                "{name} 的 group param 贴图不像路径: {texture:?}"
            );

            // 0x20 引擎不读：出厂数据里有一批就是未初始化填充，正好
            // 反过来证明没人用它。0x2c 没找到读的地方，但值一直干净。
            let raw = |o: usize| u32::from_le_bytes(record[o..o + 4].try_into().unwrap());
            assert!(
                raw(GROUP_PARAM_UNUSED) == 0 || raw(GROUP_PARAM_UNUSED) == 0xCDCD_CDCD,
                "{name}「{name_in_record}」0x20 出现了没见过的值 {:#x}",
                raw(GROUP_PARAM_UNUSED)
            );
            assert!(
                raw(GROUP_PARAM_UNKNOWN_FLAG) <= 1,
                "{name}「{name_in_record}」0x2c 出现了没见过的值 {:#x}",
                raw(GROUP_PARAM_UNKNOWN_FLAG)
            );

            // 混合模式：出厂数据里只有雪走 alpha，其余一律加法。
            let blend = group_param_blend(record);
            if blend == ParBlend::Alpha {
                assert_eq!(
                    name_in_record, "Snow",
                    "{name}「{name_in_record}」不是雪却用了 alpha 混合"
                );
            }

            // 四条轨道、每条三个分量：文件自带的标签要对得上，曲线要
            // 从时间 0 起、严格递增、正好收在 1.0。
            for (t, track) in GROUP_PARAM_TRACKS.iter().enumerate() {
                let label = track_label(record, *track);
                assert!(
                    label.is_empty() || TRACK_LABELS[t].starts_with(&label),
                    "{name}「{name_in_record}」轨道 {t} 的标签是 {label:?}，本该是 {:?}",
                    TRACK_LABELS[t]
                );

                // 前三条是标量曲线，ComputeParticleParam 只取 out[0]；
                // 只有 RgbColor 那条把三个分量都用上（R/G/B）。
                let components = if t == 3 { 3 } else { 1 };
                for comp in 0..components {
                    let curve = track_curve(record, *track, comp);
                    assert!(
                        !curve.is_empty(),
                        "{name}「{name_in_record}」{} 分量 {comp} 是空的",
                        TRACK_LABELS[t]
                    );
                    assert_eq!(
                        curve[0].0, 0.0,
                        "{name}「{name_in_record}」{} 分量 {comp} 不从 0 开始",
                        TRACK_LABELS[t]
                    );
                    for pair in curve.windows(2) {
                        assert!(
                            pair[1].0 > pair[0].0,
                            "{name}「{name_in_record}」{} 分量 {comp} 时间没递增: {pair:?}",
                            TRACK_LABELS[t]
                        );
                    }
                    assert!(
                        (curve.last().unwrap().0 - 1.0).abs() < 1e-6,
                        "{name}「{name_in_record}」{} 分量 {comp} 没收在 1.0: {:?}",
                        TRACK_LABELS[t],
                        curve.last()
                    );
                }
            }
        }

        // 发射器：名字、六条轨道的标签、发射率曲线。
        for record in &par.emitters {
            let emitter = emitter_name(record);
            assert!(
                emitter.is_ascii(),
                "{name} 的 emitter 名字不像字符串: {emitter:?}"
            );
            for (t, track) in EMITTER_TRACKS.iter().enumerate() {
                let label = track_label(record, *track);
                // 标签字段是 char[0x20]，长过 32 字节的会被截断。
                assert!(
                    label.is_empty() || EMITTER_TRACK_LABELS[t].starts_with(&label),
                    "{name}「{emitter}」发射器轨道 {t} 的标签是 {label:?}，本该是 {:?}",
                    EMITTER_TRACK_LABELS[t]
                );
            }

            // EmissionRate 是第三条，每秒发射几颗，必须有值。
            let rate = track_curve(record, EMITTER_TRACKS[2], 0);
            assert!(
                !rate.is_empty() && rate[0].0 == 0.0,
                "{name}「{emitter}」发射率曲线不对: {rate:?}"
            );
            assert!(
                rate.iter().all(|(_, v)| *v >= 0.0),
                "{name}「{emitter}」发射率有负值: {rate:?}"
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
