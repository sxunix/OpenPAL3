//! 五灵环：战斗舞台上的两个环（`CBStageScene`，`D:\Project\PAL3\Combat\CBStageScene.cpp`）。逆向记在
//! `switch/re/notes/combat_flow.md`「五灵环」一节。
//!
//! 构造（`FUN_0050fee0`）载 `cbscene\fivelineup.tga`（环）和 `fivelineup_center.tga`（环心），走加色混合
//! （`gfxscript\additiveblend.gbf`）；`combat.ini` 的 `[FiveLineup]` / `[EnemyFiveLineup]` 给中心、半径、roll
//! （`FUN_00511970`）。三块四边形（`FUN_005107c0`）：我方环、环心、敌方环。平时不画——只有时间轴往绘制命令表
//! 里压命令时才画（`FUN_004dfa10`：类型 5 画我方环和环心，类型 6 画敌方环）：
//! - 开场（`LAB_00513eb0`）：敌方偷袭成立时只排横幅；否则我方环 4 秒（`FUN_004fce70`）、敌方环 4 秒
//!   （`FUN_004fcfb0`），前 3/4 透明度 0 → 255、后 1/4 降回 0。
//! - 阵型面板：打开 0.5 秒淡入（`FUN_004fd0b0`），关上 0.5 秒淡出（`FUN_004fd1a0`）。

use std::rc::Rc;

use common::store_ext::StoreExt2;
use crosscom::ComRc;
use mini_fs::MiniFs;
use radiance::comdef::IEntity;
use radiance::math::Vec3;
use radiance::rendering::ComponentFactory;

use crate::openpal3::scene::flexeff_meteor::{QuadVerts, build_meteor_entity, write_quads};
use crate::openpal3::scene::pack_color;
use crate::openpal3::states::combat_data::LineupRing;

pub const RING_TEXTURE: &str = "/basedata/basedata/cbScene/FiveLineup.tga";
pub const CENTER_TEXTURE: &str = "/basedata/basedata/cbScene/fivelineup_center.tga";
/// 开场两步各自的时长（`push 0x40800000`）。
pub const OPENING_STEP: f32 = 4.;
/// 阵型面板开关时淡入 / 淡出的时长（`push 0x3f000000`）。
pub const PANEL_FADE: f32 = 0.5;
/// 环的四个角 y 写死 0.5，环心写 0。
const RING_Y: f32 = 0.5;

/// 环：以中心为中心、半边长 = 半径的正方形，按 roll（弧度）转——x' = x cos + z sin、z' = z cos − x sin。
pub fn ring_corners(ring: &LineupRing, roll: f32) -> [[f32; 3]; 4] {
    let r = ring.radius;
    let (s, c) = roll.sin_cos();
    [(-r, -r), (r, -r), (r, r), (-r, r)]
        .map(|(x, z)| [c * x + s * z + ring.x, RING_Y, c * z - s * x + ring.z])
}

/// 环心：半边长 = 半径的一半，不转，y = 0。
pub fn center_corners(ring: &LineupRing) -> [[f32; 3]; 4] {
    let h = ring.radius * 0.5;
    let (x, z) = (ring.x, ring.z);
    [
        [x - h, 0., z - h],
        [x + h, 0., z - h],
        [x + h, 0., z + h],
        [x - h, 0., z + h],
    ]
}

/// `FUN_00510180`：按场景五灵属性转我方环——先过 `FUN_004fa6a0`（1→5、2→1、3→4、4→2、5→3），
/// 结果 1..4 各加 2π/5 的 1、2、4、3 倍，5 不加。
pub fn element_roll(roll: f32, element: Option<u8>) -> f32 {
    roll + match element {
        Some(2) => 1.256_636,
        Some(4) => 2.513_272,
        Some(5) => 5.026_544,
        Some(3) => 3.769_908,
        _ => 0.,
    }
}

/// 环心的颜色（`FUN_00510180` 写 `+0x330`，ARGB 里的 RGB；alpha 画的时候换成透明度）。
/// [推断] 没有场景五灵属性时原版不写，这里用白色。
pub fn element_color(element: Option<u8>) -> [u8; 3] {
    match element {
        Some(1) => [0x39, 0x9a, 0xad],
        Some(2) => [0xde, 0x75, 0x31],
        Some(3) => [0x52, 0xb6, 0x6b],
        Some(4) => [0xde, 0x75, 0xbd],
        Some(5) => [0xd6, 0xae, 0x5a],
        _ => [0xff; 3],
    }
}

/// x87 `FISTP`（四舍六入五成双）。
fn round(v: f32) -> i64 {
    v.round_ties_even() as i64
}

/// `FUN_004fce70` / `FUN_004fcfb0`：前 3/4 从 0 升到 255，后 1/4 降回 0。降的那半截算出来超出 0..=255
/// （负数当无符号数）就是 0。
pub fn opening_alpha(t: f32, dur: f32) -> u8 {
    let rise = dur * 0.25 * 3.0;
    if rise <= t {
        let v = round((1.0 - (t - rise) / (dur * 0.25)) * 255.0);
        if (0..=255).contains(&v) { v as u8 } else { 0 }
    } else {
        round(t / rise * 255.0).clamp(0, 255) as u8
    }
}

/// `FUN_004fd0b0`：用时长从 0 升到 255，到点停在 255。
pub fn fade_in_alpha(t: f32, dur: f32) -> u8 {
    if dur <= t {
        255
    } else {
        round(t / dur * 255.0).clamp(0, 255) as u8
    }
}

/// `FUN_004fd1a0`：用时长从 255 降到 0。
pub fn fade_out_alpha(t: f32, dur: f32) -> u8 {
    if dur <= t {
        0
    } else {
        let v = round((1.0 - t / dur) * 255.0);
        if (0..=255).contains(&v) { v as u8 } else { 0 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Panel {
    Closed { since: Option<f32> },
    Open { since: f32 },
}

/// 这一帧要画的：我方环（可能两条命令同时在画，各画一次）和敌方环的透明度。
pub fn ring_alphas(
    clock: f32,
    enemy_sneak: bool,
    panel: (bool, Option<f32>, Option<f32>),
) -> (Vec<u8>, Option<u8>) {
    let (open, opened_at, closed_at) = panel;
    let mut ours = vec![];
    let mut enemy = None;
    if !enemy_sneak {
        if clock < OPENING_STEP {
            ours.push(opening_alpha(clock, OPENING_STEP));
        } else if clock < OPENING_STEP * 2. {
            enemy = Some(opening_alpha(clock - OPENING_STEP, OPENING_STEP));
        }
    }
    if open {
        // [推断] 淡入走完以后一直亮着到关面板（关的时候从 255 淡出，说明中间是亮的）。
        if let Some(at) = opened_at {
            ours.push(fade_in_alpha(clock - at, PANEL_FADE));
        }
    } else if let Some(at) = closed_at.filter(|at| clock - at < PANEL_FADE) {
        ours.push(fade_out_alpha(clock - at, PANEL_FADE));
    }
    (ours, enemy)
}

/// 舞台上的五灵环：两个实体（环一张贴图、环心一张贴图），顶点每帧重写。
#[derive(Clone)]
pub struct LineupRingFx {
    ring: ComRc<IEntity>,
    center: ComRc<IEntity>,
    ours: LineupRing,
    enemy: LineupRing,
    /// 按场景五灵属性转过的 roll 和环心颜色，第一次更新时定。
    applied: Option<(f32, [u8; 3])>,
    panel: Panel,
}

fn load_texture(vfs: &MiniFs, path: &str) -> Option<image::RgbaImage> {
    let data = vfs.read_to_end(path).ok()?;
    // gbengine 读 TGA 时最下面一行在前（v = 0 是图底），image 是上行在前，所以翻一下。
    // TGA 没有文件头标识，猜不出来时按 TGA 解。
    let img = image::load_from_memory(&data)
        .or_else(|_| image::load_from_memory_with_format(&data, image::ImageFormat::Tga))
        .ok()?;
    Some(img.flipv().to_rgba8())
}

fn quad(corners: [[f32; 3]; 4], rgb: [u8; 3], alpha: u8) -> QuadVerts {
    let c = pack_color([
        rgb[0] as f32 / 255.,
        rgb[1] as f32 / 255.,
        rgb[2] as f32 / 255.,
        alpha as f32 / 255.,
    ]);
    let uv = [(0., 0.), (1., 0.), (1., 1.), (0., 1.)];
    [0, 1, 2, 3].map(|k| {
        let p = corners[k];
        (Vec3::new(p[0], p[1], p[2]), uv[k], c)
    })
}

impl LineupRingFx {
    pub fn build(
        factory: &Rc<dyn ComponentFactory>,
        vfs: &MiniFs,
        ours: LineupRing,
        enemy: LineupRing,
    ) -> Option<Self> {
        let ring_tex = load_texture(vfs, RING_TEXTURE);
        let center_tex = load_texture(vfs, CENTER_TEXTURE);
        let (ring_tex, center_tex) = (ring_tex?, center_tex?);
        let ring = build_meteor_entity(
            factory,
            "COMBAT_LINEUP_RING".into(),
            RING_TEXTURE,
            ring_tex,
            true,
            3,
        );
        let center = build_meteor_entity(
            factory,
            "COMBAT_LINEUP_CENTER".into(),
            CENTER_TEXTURE,
            center_tex,
            true,
            2,
        );
        write_quads(&ring, 3, &[]);
        write_quads(&center, 2, &[]);
        Some(Self {
            ring,
            center,
            ours,
            enemy,
            applied: None,
            panel: Panel::Closed { since: None },
        })
    }

    pub fn entities(&self) -> [ComRc<IEntity>; 2] {
        [self.ring.clone(), self.center.clone()]
    }

    /// 每帧：`clock` 是战斗时钟，`enemy_sneak` 是敌方偷袭成立，`panel_open` 是阵型面板开着。
    pub fn update(&mut self, clock: f32, enemy_sneak: bool, element: Option<u8>, panel_open: bool) {
        let (roll, color) = *self.applied.get_or_insert_with(|| {
            (
                element_roll(self.ours.roll, element),
                element_color(element),
            )
        });
        self.panel = match (panel_open, self.panel) {
            (true, Panel::Closed { .. }) => Panel::Open { since: clock },
            (false, Panel::Open { .. }) => Panel::Closed { since: Some(clock) },
            (_, p) => p,
        };
        let panel = match self.panel {
            Panel::Open { since } => (true, Some(since), None),
            Panel::Closed { since } => (false, None, since),
        };
        let (ours, enemy) = ring_alphas(clock, enemy_sneak, panel);
        let mut rings: Vec<QuadVerts> = ours
            .iter()
            .map(|&a| quad(ring_corners(&self.ours, roll), [0xff; 3], a))
            .collect();
        if let Some(a) = enemy {
            rings.push(quad(
                ring_corners(&self.enemy, self.enemy.roll),
                [0xff; 3],
                a,
            ));
        }
        let centers: Vec<QuadVerts> = ours
            .iter()
            .map(|&a| quad(center_corners(&self.ours), color, a))
            .collect();
        write_quads(&self.ring, 3, &rings);
        write_quads(&self.center, 2, &centers);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring() -> LineupRing {
        LineupRing {
            x: 100.,
            y: 0.,
            z: -50.,
            radius: 207.,
            roll: 0.,
        }
    }

    #[test]
    fn the_ring_is_a_rotated_square_half_a_unit_above_the_floor() {
        let c = ring_corners(&ring(), 0.);
        assert_eq!(c[0], [100. - 207., 0.5, -50. - 207.]);
        assert_eq!(c[2], [100. + 207., 0.5, -50. + 207.]);
        let c = ring_corners(&ring(), std::f32::consts::FRAC_PI_2);
        // 转 90°：(−r, −r) → (x' = z = −r, z' = −x = r)。
        assert!((c[0][0] - (100. - 207.)).abs() < 1e-3 && (c[0][2] - (-50. + 207.)).abs() < 1e-3);
        let m = center_corners(&ring());
        assert_eq!(m[1], [100. + 103.5, 0., -50. - 103.5]);
    }

    #[test]
    fn the_element_turns_our_ring_by_fifths_of_a_turn() {
        assert_eq!(element_roll(0.5, Some(1)), 0.5);
        assert_eq!(element_roll(0., Some(2)), 1.256_636);
        assert_eq!(element_roll(0., Some(4)), 2.513_272);
        assert_eq!(element_roll(0., Some(3)), 3.769_908);
        assert_eq!(element_roll(0., Some(5)), 5.026_544);
        assert_eq!(element_color(Some(2)), [0xde, 0x75, 0x31]);
    }

    #[test]
    fn opening_fades_in_for_three_quarters_and_out_for_the_last() {
        assert_eq!(opening_alpha(0., 4.), 0);
        assert_eq!(opening_alpha(1.5, 4.), 128, "127.5 四舍六入五成双");
        assert_eq!(opening_alpha(3., 4.), 255);
        assert_eq!(opening_alpha(3.5, 4.), 128);
        assert_eq!(opening_alpha(4., 4.), 0);
        assert_eq!(fade_in_alpha(0.25, 0.5), 128);
        assert_eq!(fade_in_alpha(0.6, 0.5), 255);
        assert_eq!(fade_out_alpha(0.25, 0.5), 128);
        assert_eq!(fade_out_alpha(0.5, 0.5), 0);
    }

    #[test]
    fn the_schedule_shows_ours_then_the_enemy_unless_they_sneak_on_us() {
        let closed = (false, None, None);
        assert_eq!(ring_alphas(1.5, false, closed), (vec![128], None));
        assert_eq!(ring_alphas(5.5, false, closed), (vec![], Some(128)));
        assert_eq!(ring_alphas(9., false, closed), (vec![], None));
        assert_eq!(
            ring_alphas(1.5, true, closed),
            (vec![], None),
            "敌方偷袭只排横幅"
        );
        assert_eq!(
            ring_alphas(20., false, (true, Some(10.), None)),
            (vec![255], None)
        );
        assert_eq!(
            ring_alphas(20.25, false, (false, None, Some(20.))),
            (vec![128], None)
        );
        assert_eq!(
            ring_alphas(21., false, (false, None, Some(20.))),
            (vec![], None)
        );
    }
}
