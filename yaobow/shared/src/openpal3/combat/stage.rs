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

use radiance::math::Vec3;
use radiance::scene::{SceneLight, SceneLighting};

use crate::openpal3::states::combat_data::StagePos;

/// 我方五个战斗位用的模型名，和 `PLAYER_COMBAT_IDS` 一一对应
/// （101 景天 · 104 雪见 · 106 龙葵 · 107 紫萱 · 102 长卿）。
///
/// gdb 里主角那五条 `combat_actors` 的 `model_id` 写的是 `"Null"`，名字要从
/// 这里来，不是从数据库。原版写死在 `CCBStageRole::Create`
/// （`cbstagerole.cpp:126`，2 号位 `"106"`）和 `CCBRoleState` 的模型目录表
/// （`cbrolestate.cpp:1062`）两处。
///
/// **龙葵在战斗里一律是 106**，不跟冒险模式的形象走：冒险模式里 SCE 42
/// 「设置龙葵形象」在 105（蓝）/ 106（红）之间换，但战斗那边直接
/// `Init("106")` 再 `LoadRoleAction`，不看那个旗标。发行数据也只有 106 是
/// 完整的一套战斗动作（36 个 `z*.mv3`，含合击 504 的 `z5041/z5042`）；105 只有
/// `z05` / `z17` / `z18` 三个，连普攻 `z01`、待机 `z04` 都没有。先前这里写成
/// 105，龙葵每帧报「没有动作 z061」、普攻只剩兜底的打击时刻。
pub const PARTY_MODELS: [&str; PARTY_SLOTS] = ["101", "104", "106", "107", "102"];

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

/// 引擎的全局环境光。原版 `gbGfxManager` 构造里写死 (0.2, 0.2, 0.2, 1)
/// （gbengine.dll `0x1001d9b9`，字段 `+0x1ac`），而 `SetAmbientLight` /
/// `EnableEmissive` / `SetEmissiveFactor` 三个入口在 exe 的导入表里都没有、
/// gbengine 内部也没人调，所以这个值和自发光系数（0）全程不变。
///
/// 着色器里的环境项 = (全局环境光 + 灯自己的 Ambient) × 材质 Ambient
/// + 自发光系数 × 材质 Emissive。PAL3 的材质 Ambient 一律 (1,1,1)、
/// `.lgt` 每盏灯的 Ambient 全是 0、自发光系数是 0，所以就等于这个常数。
pub const AMBIENT: [f32; 3] = [0.2, 0.2, 0.2];

/// 读一张战场的 `.lgt`，连同 [`AMBIENT`] 一起给战斗场景。
///
/// 原版战斗里灯是挂着的：`FUN_00510ca0`（舞台的 Render）在第一轮
/// `gbSceneGraph::Begin/End` 里依次画舞台 POL、**灯光节点**、指针和阵型标记，
/// 角色也在同一轮，所以角色吃这组灯（舞台本身不吃，它带光照贴图，
/// 见 `combat_flow.md`）。读不到就只给环境光。
pub fn scene_lighting(vfs: &mini_fs::MiniFs, scene: &str) -> SceneLighting {
    use common::store_ext::StoreExt2;

    let path = scene_light_path(scene);
    let lights = vfs
        .read_to_end(&path)
        .ok()
        .and_then(|buf| fileformats::pal3::lgt::read_lgt(&mut std::io::Cursor::new(buf)).ok())
        .map(|lgt| {
            lgt.lights
                .iter()
                .map(|l| {
                    let p = l.position();
                    SceneLight::with_range(Vec3::new(p[0], p[1], p[2]), l.color, l.range)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| {
            log::warn!("Combat: 战场光照读不到 {}", path.display());
            vec![]
        });

    log::debug!("Combat: 战场 {scene} {} 盏灯", lights.len());
    SceneLighting::new(AMBIENT, lights)
}

/// 一台相机的位姿（`cbCam.ini` 的一节，或 `flexcam.dat` 记录里的起止机位）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CombatCamera {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// 弧度。
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
}

impl CombatCamera {
    /// 朝哪边看：`(sin yaw · cos pitch, sin pitch, −cos yaw · cos pitch)`。
    ///
    /// 正负号是拿 `EditorCam0..28` 验出来的：这样算，视线落地点离战场中心
    /// 大多在 200 以内；x 取反则大多在 300~1400。pitch 负 = 往下看
    /// （`EditorCam0` 高 277、离中心 531，俯角 27.5°，pitch −0.502 = −28.8°）。
    pub fn forward(&self) -> (f32, f32, f32) {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        (sy * cp, sp, -cy * cp)
    }
}

/// 战斗相机的视场角（度）。镜头初始化 `FUN_00504540` 往相机 `+4`（即 gbCamera 的
/// `+0`，`SetFOV` 写的那一格）写 50.0。gbengine 的 `gbCamera::GetViewSizeOnNearPlane`
/// （0x10021a10）按**水平**视角用：半宽 = tan(fov/2) × 近平面，半高 = 半宽 × 高 / 宽。
/// 原版 4:3 画面下垂直约 38.6°（Pal3.Unity 用的 39° 就是这个换算），radiance 的 `fov43`
/// 也是 4:3 水平视角。
pub const COMBAT_FOV_DEGREES: f32 = 50.;

/// `cbCam.ini` 在 vfs 里的位置。
pub const COMBAT_CAMERA_INI: &str = "/basedata/basedata/datascript/cbCam.ini";

/// 解析 `cbCam.ini`：照 `FUN_00504130`，从 `EditorCam0` 起逐个读，读到第一个缺的
/// 编号为止（最多 64 台）。`DefaultCam*` 那十八节 exe 里没有这个字符串，游戏不读。
pub fn parse_cameras(text: &str) -> Vec<CombatCamera> {
    let mut sections: std::collections::HashMap<String, CombatCamera> = Default::default();
    let mut current: Option<String> = None;
    for raw in text.lines() {
        let line = raw.split(';').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let name = name.trim().to_string();
            sections.entry(name.clone()).or_default();
            current = Some(name);
            continue;
        }
        let (Some(name), Some((key, value))) = (current.as_ref(), line.split_once('=')) else {
            continue;
        };
        let Ok(v) = value.trim().parse::<f32>() else {
            continue;
        };
        let cam = sections.get_mut(name).unwrap();
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
    (0..64)
        .map_while(|i| sections.get(&format!("EditorCam{i}")).copied())
        .collect()
}

/// 战斗位号 → `[RolePos]` 的下标。
///
/// 我方五个位对应 `Role0..Role4`，但原版读完 combat.ini 后把第 3、4 个我方坐标对调
/// （`FUN_00511790` 末尾 swap `+0x80..+0x88` 和 `+0x8c..+0x94`）：2 号位站 `Role3`、
/// 3 号位站 `Role2`。敌方六个位接在后面，`Role5..Role10`。
pub fn stage_index(slot: usize) -> Option<usize> {
    match slot {
        2 => Some(3),
        3 => Some(2),
        s => (s < STAGE_SLOTS).then_some(s),
    }
}

/// 合击技开演时站到哪（`0x4ff3d0`）：舞台表 `+0x5c + 阵位 × 12`——阵位 1..5 就是我方
/// 第 1..5 个站位（同样对调过），6 是 `[FiveLineup]` 的中心（`+0xa4`）；0 不挪。
pub fn lineup_spot(positions: &[StagePos], ring_center: StagePos, element: u8) -> Option<StagePos> {
    match element {
        1..=5 => stage_position(positions, element as usize - 1),
        6 => Some(ring_center),
        _ => None,
    }
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
        // 我方第 3、4 个坐标对调过。
        assert_eq!(
            (0..PARTY_SLOTS).map(stage_index).collect::<Vec<_>>(),
            [Some(0), Some(1), Some(3), Some(2), Some(4)]
        );
        // 敌方接在后面，一共十一个。
        assert_eq!(stage_index(5), Some(5));
        assert_eq!(stage_index(10), Some(10));
        assert_eq!(stage_index(11), None, "只有十一个站位");
    }

    const SAMPLE_INI: &str = "\
; cbCam.ini
[DefaultCam0]
x=999.0
y=999.0
z=999.0
[EditorCam0]
x=-78.400940
y=277.000031
z=-525.482727
yaw=3.107081
pitch=-0.502480
roll=0.000000
[EditorCam2]
x=2.0
[EditorCam1]
x=1.0
[EditorCam4]
x=4.0
";

    #[test]
    fn only_the_editor_cameras_are_read_in_number_order() {
        let cams = parse_cameras(SAMPLE_INI);
        // 节的先后无所谓；读到第一个缺的编号（3）就停。
        assert_eq!(
            cams.iter().map(|c| c.x).collect::<Vec<_>>(),
            vec![-78.40094, 1.0, 2.0]
        );
        assert!((cams[0].pitch - -0.50248).abs() < 1e-5);
        assert!(cams.iter().all(|c| c.x != 999.0), "DefaultCam 游戏不读");
    }

    #[test]
    fn the_stand_camera_looks_down_at_the_field() {
        let cams = parse_cameras(SAMPLE_INI);
        let c = cams[0];
        let (fx, fy, fz) = c.forward();
        assert!(fy < 0., "往下看");
        // 水平方向几乎正对 +z（敌方在那边）；整体的 z 还要乘 cos(俯角)。
        assert!(fz / fx.hypot(fz) > 0.99, "朝 +z：敌方在那边");
        // 视线落地点离站位重心 (−24, 16) 不远。
        let k = -c.y / fy;
        let (gx, gz) = (c.x + fx * k, c.z + fz * k);
        assert!(((gx + 24.).powi(2) + (gz - 16.).powi(2)).sqrt() < 100.);
    }

    #[test]
    fn our_side_uses_the_adventure_models() {
        // gdb 里主角的 model_id 是 "Null"，模型名只能从这张表来。
        assert_eq!(model_name(0, None).as_deref(), Some("101"));
        assert_eq!(
            model_name(2, None).as_deref(),
            Some("106"),
            "战斗里龙葵一律是 106（cbstagerole.cpp:134），不跟冒险模式的蓝红形象走"
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
    fn a_combo_spot_follows_the_element_position() {
        let positions: Vec<StagePos> = (0..11).map(|i| StagePos { x: i as f32, z: 0. }).collect();
        let ring = StagePos { x: -162., z: -130. };
        assert_eq!(lineup_spot(&positions, ring, 1).unwrap().x, 0.);
        // 3 风、4 雷落在对调过的两个坐标上。
        assert_eq!(lineup_spot(&positions, ring, 3).unwrap().x, 3.);
        assert_eq!(lineup_spot(&positions, ring, 4).unwrap().x, 2.);
        assert_eq!(lineup_spot(&positions, ring, 6), Some(ring));
        assert_eq!(lineup_spot(&positions, ring, 0), None);
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
