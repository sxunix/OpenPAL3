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
