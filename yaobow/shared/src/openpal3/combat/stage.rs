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

/// 我方五个战斗位用的模型名，和 `PLAYER_COMBAT_IDS` 一一对应
/// （101 景天 · 104 雪见 · 105 龙葵 · 107 紫萱 · 102 长卿）。
///
/// gdb 里主角那五条 `combat_actors` 的 `model_id` 写的是 `"Null"`——他们
/// 直接用冒险模式的角色模型，所以名字要从这里来，不是从数据库。
/// 龙葵的战斗 id 是 106（红葵），但模型目录是 105。
pub const PARTY_MODELS: [&str; PARTY_SLOTS] = ["101", "104", "105", "107", "102"];

/// 战斗里一个位子上站着谁的模型。
///
/// 我方（0..4）查 [`PARTY_MODELS`]；敌方（5..10）用 gdb 那条记录的
/// `model_id`（`B01` 之类，对应 `basedata/ROLE/B01`）。
pub fn model_name(slot: usize, enemy_model: Option<&str>) -> Option<String> {
    if slot < PARTY_SLOTS {
        return PARTY_MODELS.get(slot).map(|s| s.to_string());
    }
    let name = enemy_model?.trim();
    // 主角那几条写的是 Null，别拿它去找目录。
    (!name.is_empty() && !name.eq_ignore_ascii_case("null")).then(|| name.to_string())
}

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

/// `cbCam.ini` 里一台相机的位姿。
///
/// 文件头写着「由战斗编辑器自动产生」，一共 47 节：`DefaultCam0..17`
/// 十八台，其余是 `EditorCam*`（编辑器用的，游戏不读）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CombatCamera {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// 弧度。**移植端没用它**——见 [`CombatCamera::look_at_origin`]。
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
}

impl CombatCamera {
    /// 摆相机时朝哪儿看。
    ///
    /// 原版存的 `yaw`/`pitch`/`roll` 用的是哪套约定没定死：拿
    /// `DefaultCam0` 验算，位置 (−70.4, 337, −533.7) 看向原点的话俯仰是
    /// −32°，而 ini 里的 `pitch` 是 −35°（差 3°）；`yaw` 是 −181.7°，加上
    /// π 得 −1.7°，而实际水平角是 7.5°（差 9°）。**方向大致对得上但不
    /// 精确**，所以这里和 Pal3.Unity 一样，只取位置、朝向一律看战场中心
    /// （`CombatManager.SetCameraPosition` 把旋转那行注释掉了，改用
    /// `LookAt(Vector3.zero)`）。
    ///
    /// 战场中心用原点是有依据的：十一个站位的重心在 (−33, 0, 8)。
    pub fn look_at_origin(&self) -> ((f32, f32, f32), (f32, f32, f32)) {
        ((self.x, self.y, self.z), (0., 0., 0.))
    }
}

/// 战斗相机的视场角（度）。取自 Pal3.Unity 的
/// `CombatManager.COMBAT_CAMERA_DEFAULT_FOV`；exe 侧没查。**[推断]**
pub const COMBAT_FOV_DEGREES: f32 = 39.;

/// `cbCam.ini` 在 vfs 里的位置。
pub const COMBAT_CAMERA_INI: &str = "/basedata/basedata/datascript/cbCam.ini";

/// 解析 `cbCam.ini`，只取 `DefaultCam*`，按编号排好。
pub fn parse_cameras(text: &str) -> Vec<CombatCamera> {
    let mut found: Vec<(usize, CombatCamera)> = vec![];
    let mut current: Option<(usize, CombatCamera)> = None;
    for raw in text.lines() {
        let line = raw.split(';').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            if let Some(done) = current.take() {
                found.push(done);
            }
            // EditorCam* 是编辑器留下的，游戏不读。
            current = name
                .strip_prefix("DefaultCam")
                .and_then(|n| n.trim().parse::<usize>().ok())
                .map(|n| (n, CombatCamera::default()));
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some((_, cam)) = current.as_mut() else {
            continue;
        };
        let Ok(v) = value.trim().parse::<f32>() else {
            continue;
        };
        match key.trim() {
            "x" => cam.x = v,
            "y" => cam.y = v,
            "z" => cam.z = v,
            "yaw" => cam.yaw = v,
            "pitch" => cam.pitch = v,
            "roll" => cam.roll = v,
            _ => {}
        }
    }
    if let Some(done) = current.take() {
        found.push(done);
    }
    found.sort_by_key(|(n, _)| *n);
    found.into_iter().map(|(_, c)| c).collect()
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

    const SAMPLE_INI: &str = "\
; cbCam.ini
; 由战斗编辑器自动产生，请不要手动修改

[DefaultCam0]
x=-70.413155
y=337.000000
z=-533.723267
yaw=-3.171740
pitch=-0.611563
roll=0.000000
[DefaultCam1]
x=1.0
y=2.0
z=3.0
[EditorCam0]
x=999.0
y=999.0
z=999.0
";

    #[test]
    fn only_the_default_cameras_are_read() {
        let cams = parse_cameras(SAMPLE_INI);
        assert_eq!(cams.len(), 2, "EditorCam 不该算进来");
        assert!((cams[0].x - -70.413155).abs() < 1e-4);
        assert!((cams[0].y - 337.).abs() < 1e-4);
        assert!((cams[0].pitch - -0.611563).abs() < 1e-5);
        assert_eq!(cams[1].x, 1.0);
        // 编辑器那台的坐标一个都不该混进来。
        assert!(cams.iter().all(|c| c.x != 999.0));
    }

    #[test]
    fn the_camera_only_contributes_its_position() {
        let cam = CombatCamera {
            x: -70.413155,
            y: 337.,
            z: -533.723267,
            yaw: -3.17174,
            pitch: -0.611563,
            roll: 0.,
        };
        let (pos, target) = cam.look_at_origin();
        assert_eq!(pos, (-70.413155, 337., -533.723267));
        assert_eq!(target, (0., 0., 0.), "朝向一律看战场中心");
    }

    #[test]
    fn our_side_uses_the_adventure_models() {
        // gdb 里主角的 model_id 是 "Null"，模型名只能从这张表来。
        assert_eq!(model_name(0, None).as_deref(), Some("101"));
        assert_eq!(
            model_name(2, None).as_deref(),
            Some("105"),
            "龙葵目录是 105"
        );
        assert_eq!(model_name(4, None).as_deref(), Some("102"));
        // 敌方拿数据库那一条。
        assert_eq!(model_name(5, Some("B01")).as_deref(), Some("B01"));
        assert_eq!(model_name(10, Some("B08")).as_deref(), Some("B08"));
        // "Null" 和空的都不能当目录名去找。
        assert_eq!(model_name(5, Some("Null")), None);
        assert_eq!(model_name(5, Some("null")), None);
        assert_eq!(model_name(5, Some("  ")), None);
        assert_eq!(model_name(5, None), None);
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
