//! 战斗舞台：把 `cbScene` 里那张战场摆出来，把参战的人放到各自的位子上。
//!
//! 战场选哪一张已经由 [`combat_data`](crate::openpal3::states::combat_data)
//! 算好了（冒险场景 + 分块 + 脚下地面类型 → `z**`）；这里只管加载和摆放。
//!
//! 资源在 `basedata/cbScene/<大写场景名>/<小写场景名>.pol`，同目录还有
//! `.lgt`（光照）和 `.DKM`。实机 44 张。
//!
//! 站位是 `datascript/combat.ini` 的 `[RolePos]`，十一个 x/z——**前五个是
//! 我方**（0..4），**后六个是敌方**（5..10）。y 由战场地面给，这里先用 0。

use std::path::PathBuf;

use crate::openpal3::states::combat_data::StagePos;

/// `cbScene` 在 vfs 里的位置。
pub const COMBAT_SCENE_DIR: &str = "/basedata/basedata/cbScene";

/// 我方站位占前几个 `RolePos`。
pub const PARTY_SLOTS: usize = 5;
/// `[RolePos]` 一共几个。
pub const STAGE_SLOTS: usize = 11;

/// 战场模型的 vfs 路径。目录名大写、文件名小写——实机就是这么放的
/// （`cbScene/Z01/z01.pol`）。
pub fn scene_model_path(scene: &str) -> PathBuf {
    let upper = scene.to_uppercase();
    let lower = scene.to_lowercase();
    PathBuf::from(format!("{COMBAT_SCENE_DIR}/{upper}/{lower}.pol"))
}

/// 同一张战场的光照文件。
pub fn scene_light_path(scene: &str) -> PathBuf {
    let upper = scene.to_uppercase();
    let lower = scene.to_lowercase();
    PathBuf::from(format!("{COMBAT_SCENE_DIR}/{upper}/{lower}.lgt"))
}

/// 战斗位号 → `[RolePos]` 的下标。
///
/// 我方五个位直接对应 `Role0..Role4`；敌方六个位接在后面，
/// `Role5..Role10`。这和 `combat::state::SLOTS` 的排法一致（0..4 我方、
/// 5..10 敌方）。
pub fn stage_index(slot: usize) -> Option<usize> {
    (slot < STAGE_SLOTS).then_some(slot)
}

/// 一个参战者摆在哪儿。`positions` 就是 `CombatData::role_positions`。
pub fn stage_position(positions: &[StagePos], slot: usize) -> Option<StagePos> {
    positions.get(stage_index(slot)?).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_model_path_mixes_case_the_way_the_pack_does() {
        // 目录 Z01、文件 z01.pol。
        assert_eq!(
            scene_model_path("z01"),
            PathBuf::from("/basedata/basedata/cbScene/Z01/z01.pol")
        );
        // 场景名从 combatScn.txt 来，大小写不定，两种都要归一。
        assert_eq!(scene_model_path("Z01"), scene_model_path("z01"));
        // 带字母后缀的那几张（z04a）也是一样的规矩。
        assert_eq!(
            scene_model_path("z04a"),
            PathBuf::from("/basedata/basedata/cbScene/Z04A/z04a.pol")
        );
        assert_eq!(
            scene_light_path("z01"),
            PathBuf::from("/basedata/basedata/cbScene/Z01/z01.lgt")
        );
    }

    #[test]
    fn the_first_five_slots_are_ours() {
        for slot in 0..PARTY_SLOTS {
            assert_eq!(stage_index(slot), Some(slot));
        }
        // 敌方接在后面，一共十一个。
        assert_eq!(stage_index(5), Some(5));
        assert_eq!(stage_index(10), Some(10));
        assert_eq!(stage_index(11), None, "只有十一个站位");
    }

    #[test]
    fn a_slot_without_a_row_has_no_position() {
        let positions = vec![StagePos { x: 1., z: 2. }, StagePos { x: 3., z: 4. }];
        assert_eq!(
            stage_position(&positions, 0),
            Some(StagePos { x: 1., z: 2. })
        );
        assert_eq!(
            stage_position(&positions, 1),
            Some(StagePos { x: 3., z: 4. })
        );
        // 表短了就没有——不该 panic。
        assert_eq!(stage_position(&positions, 2), None);
        assert_eq!(stage_position(&positions, 99), None);
        assert_eq!(stage_position(&[], 0), None);
    }
}
