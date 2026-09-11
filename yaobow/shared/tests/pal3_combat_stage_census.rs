//! 拿实机的 `cbScene` 核对战斗舞台：每一张 `combatScn.txt` 会派到的战场，
//! 模型和光照都得在包里，而且路径的大小写规矩要对。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_combat_stage_census
//! ```

use std::collections::BTreeSet;
use std::path::PathBuf;

use common::store_ext::StoreExt2;
use mini_fs::StoreExt;
use shared::openpal3::combat::stage::{
    PARTY_SLOTS, STAGE_SLOTS, scene_light_path, scene_model_path,
};
use shared::openpal3::states::combat_data::CombatData;

fn data() -> Option<(mini_fs::MiniFs, CombatData)> {
    let assets = std::env::var("YAOBOW_PAL3_ASSETS").ok()?;
    let vfs = packfs::init_virtual_fs(&assets, None);
    let ini = vfs
        .read_to_end_from_gbk("/basedata/basedata/datascript/combat.ini")
        .ok()?;
    let scn = vfs
        .read_to_end_from_gbk("/basedata/basedata/datascript/combatScn.txt")
        .ok()?;
    Some((vfs, CombatData::parse(&ini, &scn)))
}

#[test]
fn every_battlefield_the_tables_can_pick_is_shipped() {
    let Some((vfs, data)) = data() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    // combatScn.txt 里所有会被派到的战场名。
    let mut scenes: BTreeSet<String> = BTreeSet::new();
    for table in data.scene_tables.values() {
        scenes.insert(table.default.to_lowercase());
        for name in table.by_floor.values() {
            scenes.insert(name.to_lowercase());
        }
    }
    assert!(!scenes.is_empty(), "一张战场都没派到，表大概没解析对");

    let mut missing = vec![];
    for scene in &scenes {
        let model = scene_model_path(scene);
        if vfs.open(&model).is_err() {
            missing.push(model.display().to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "{} 张战场的模型不在包里：{:?}",
        missing.len(),
        &missing[..missing.len().min(5)]
    );
    println!("{} 张战场，模型全部在包里", scenes.len());

    // 光照文件按同一个规矩放，缺的只报数不失败——渲染侧还没用它。
    let no_light = scenes
        .iter()
        .filter(|s| vfs.open(&scene_light_path(s)).is_err())
        .count();
    println!("其中 {no_light} 张没有 .lgt");
}

#[test]
fn the_stage_has_eleven_slots_five_of_them_ours() {
    let Some((_, data)) = data() else { return };
    assert_eq!(
        data.role_positions.len(),
        STAGE_SLOTS,
        "[RolePos] 该有十一行"
    );
    // 我方五个、敌方六个。
    assert_eq!(STAGE_SLOTS - PARTY_SLOTS, 6);

    // 两军分居 **z** 的两侧：我方 z 全负（−209..−47），敌方 z 全正
    // （+4..+257）。x 上并不分开——敌方的 Role9 是 −40，比我方的 Role1
    // （−75）还靠里。
    for (i, p) in data.role_positions.iter().enumerate() {
        if i < PARTY_SLOTS {
            assert!(p.z < 0., "我方第 {i} 位 z = {} 该在负半边", p.z);
        } else {
            assert!(p.z > 0., "敌方第 {i} 位 z = {} 该在正半边", p.z);
        }
    }
    // 我方的 x 确实全在负半边，敌方的不是。
    assert!(data.role_positions[..PARTY_SLOTS].iter().all(|p| p.x < 0.));
    assert!(data.role_positions[PARTY_SLOTS..].iter().any(|p| p.x < 0.));
    println!(
        "我方 {:?}",
        &data.role_positions[..PARTY_SLOTS]
            .iter()
            .map(|p| (p.x as i32, p.z as i32))
            .collect::<Vec<_>>()
    );
    println!(
        "敌方 {:?}",
        &data.role_positions[PARTY_SLOTS..]
            .iter()
            .map(|p| (p.x as i32, p.z as i32))
            .collect::<Vec<_>>()
    );
}

/// `cbCam.ini` 的 `EditorCam*`（游戏只读这些）和 `flexcam.dat` 的镜头序列。
#[test]
fn the_combat_camera_data_hangs_together() {
    let Some((vfs, _data)) = data() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS");
        return;
    };
    use shared::openpal3::combat::camera::{
        FILE_SIZE, FLEXCAM_PATH, Flexcam, SNEAK_SEQUENCE, STAND_SEQUENCE,
    };
    use shared::openpal3::combat::stage::{COMBAT_CAMERA_INI, parse_cameras};

    let text = vfs
        .read_to_end_from_gbk(COMBAT_CAMERA_INI)
        .expect("cbCam.ini");
    let cams = parse_cameras(&text);
    assert_eq!(cams.len(), 29, "EditorCam0..28");

    let bytes = vfs.read_to_end(FLEXCAM_PATH).expect("flexcam.dat");
    assert_eq!(bytes.len(), FILE_SIZE);
    let flex = Flexcam::parse(&bytes).unwrap();

    // 平时开战：0 号序列一步，0.8 秒拉到 EditorCam0。
    let stand = flex.sequence(STAND_SEQUENCE);
    assert_eq!(stand.len(), 1);
    assert!((stand[0].duration - 0.8).abs() < 1e-4);
    let end = stand.last().unwrap().to;
    let c0 = cams[0];
    for (a, b) in [
        (end.x, c0.x),
        (end.y, c0.y),
        (end.z, c0.z),
        (end.yaw, c0.yaw),
        (end.pitch, c0.pitch),
    ] {
        assert!(
            (a - b).abs() < 1e-3,
            "0 号序列终点 ≠ EditorCam0：{a} vs {b}"
        );
    }
    // 偷袭：20 号序列五步，最后也落在 EditorCam0。
    let sneak = flex.sequence(SNEAK_SEQUENCE);
    assert_eq!(sneak.len(), 5);
    assert!((sneak.last().unwrap().to.x - c0.x).abs() < 1e-3);

    // 散场三段：胜 3、败 10、逃 8，都得有内容，而且都从 EditorCam0 出发。
    use shared::openpal3::combat::camera::{ESCAPE_SEQUENCE, LOSE_SEQUENCE, WIN_SEQUENCE};
    for seq in [WIN_SEQUENCE, LOSE_SEQUENCE, ESCAPE_SEQUENCE] {
        let steps = flex.sequence(seq);
        assert!(!steps.is_empty(), "{seq} 号序列是空的");
        let start = steps[0].from;
        assert!(
            (start.x - c0.x).abs() < 1e-3 && (start.z - c0.z).abs() < 1e-3,
            "{seq} 号序列不是从 EditorCam0 起的"
        );
        let total: f32 = steps.iter().map(|r| r.duration).sum();
        println!("{seq} 号序列 {} 步，共 {total:.2} 秒", steps.len());
    }

    let used: Vec<usize> = (0..flex.sequences.len())
        .filter(|&s| !flex.sequences[s].is_empty())
        .collect();
    println!("有内容的镜头序列 {} 个：{used:?}", used.len());
}
