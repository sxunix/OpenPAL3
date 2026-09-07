//! 拿实机的 `datascript\scene.txt` 核对走跑速度那一组常数。
//!
//! 原版启动时用 `FUN_0041efa0` 读它，名字表在 0x58b32c 起、步长 0x14，
//! **值在名字前面四个字节**。逆向记录在 `switch/re/notes/role_move.md`。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_scene_constants_census
//! ```

use std::path::PathBuf;

use common::store_ext::StoreExt2;
use encoding::Encoding;
use shared::openpal3::states::role_move::{MoveMode, RoleConstants, SCENE_TXT};

fn constants() -> Option<RoleConstants> {
    let assets = std::env::var("YAOBOW_PAL3_ASSETS").ok()?;
    let vfs = packfs::init_virtual_fs(&assets, None);
    let bytes = vfs.read_to_end(PathBuf::from(SCENE_TXT)).ok()?;
    let text = encoding::all::GBK
        .decode(&bytes, encoding::DecoderTrap::Ignore)
        .ok()?;
    Some(RoleConstants::parse(&text))
}

#[test]
fn the_shipped_scene_txt_matches_the_built_in_defaults() {
    let Some(c) = constants() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    // 出厂那一份就是 `RoleConstants::default()` 抄的，一个字段都不该差。
    assert_eq!(
        c,
        RoleConstants::default(),
        "scene.txt 的 role 节和内置的兜底值对不上"
    );

    // 逐条写死，免得哪天默认值被顺手改了还测得过。
    assert_eq!(c.talk_range, 40.);
    assert_eq!(c.walk_speed, 80., "NPC走的速度");
    assert_eq!(c.run_speed, 180., "NPC跑的速度");
    assert_eq!(c.walk_force, 800.);
    assert_eq!(c.run_force, 1200.);
    assert_eq!(c.player_walk_speed, 100., "主角走的速度");
    assert_eq!(c.player_run_speed, 200., "主角跑的速度");
    assert_eq!(c.jump_height, 64.);
    assert_eq!(c.jump_dist, 160.);
    assert_eq!(c.g_force, 392., "9.8 × 40");
}

#[test]
fn the_three_move_modes_pick_the_right_speed() {
    let Some(c) = constants() else { return };

    // 跑：无条件 PRunSpeed，NPC 也一样（FUN_00412710 的 case 2）。
    assert_eq!(c.speed(MoveMode::Run, true, 0.), 200.);
    assert_eq!(c.speed(MoveMode::Run, false, 0.), 200.);

    // 走：主角和 NPC 分开。
    assert_eq!(c.speed(MoveMode::Walk, true, 0.), 100.);
    assert_eq!(c.speed(MoveMode::Walk, false, 0.), 80.);

    // 站着挪和走同速。
    assert_eq!(
        c.speed(MoveMode::Still, false, 0.),
        c.speed(MoveMode::Walk, false, 0.)
    );

    // 跑总比走快，主角总比 NPC 快——移植端原先两条命令一个用 175、一个
    // 无条件 175，这几条关系都不成立。
    assert!(c.speed(MoveMode::Run, true, 0.) > c.speed(MoveMode::Walk, true, 0.));
    assert!(c.speed(MoveMode::Walk, true, 0.) > c.speed(MoveMode::Walk, false, 0.));
}

#[test]
fn the_monster_section_is_left_alone() {
    let Some(c) = constants() else { return };
    // `monster` 节里也有速度键，但 role 节读到 END 就停，不能被它盖掉。
    assert_eq!(c.walk_speed, 80.);
    assert_ne!(c.walk_speed, 30., "这是 monster 节的 FreeSpeedS");
}

/// 八个脚步声都得在包里——四种地面 × 走/跑。
#[test]
fn every_footstep_variant_is_shipped() {
    use mini_fs::StoreExt;
    use shared::openpal3::states::footstep::{RUN_PREFIX, VARIANTS, WALK_PREFIX};

    let Ok(assets) = std::env::var("YAOBOW_PAL3_ASSETS") else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS");
        return;
    };
    let vfs = packfs::init_virtual_fs(&assets, None);

    let mut found = 0;
    for prefix in [WALK_PREFIX, RUN_PREFIX] {
        for i in 0..VARIANTS {
            let letter = (b'a' + i as u8) as char;
            let path = PathBuf::from(format!("/snd/{prefix}{letter}.wav"));
            assert!(vfs.open(&path).is_ok(), "{} 不在包里", path.display());
            found += 1;
        }
    }
    assert_eq!(found, VARIANTS * 2, "走和跑各四个");
    println!("脚步声 {found} 个都在：{WALK_PREFIX}a–d / {RUN_PREFIX}a–d");
}
