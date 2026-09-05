//! `cylinder` 特效类的运行模型 —— 143 播的特效里仅次于 `quad` 的一类
//! （脚本用到的组里 19 条）。
//!
//! 逆向写在 `switch/re/notes/flexeff_format.md`。原型是 `new(4292)` +
//! `FUN_004c3c90`，实例 vtable 在 0x56f7c8，和 `quad` 同构：`+0x14` 每帧更新、
//! `+0x18` 渲染、`+0x30` 建实例。
//!
//! # 定义记录 224..268 那一批
//!
//! `RotAngle(224)` / `Width(228)` / `DivisionCount(232)` /
//! `Top`+`BottomShrink(236,240)` / `Top`+`BottomRadius(244,248)` /
//! `Top`+`BottomRadiusIncreaseSpeed(252,256)` / `Top`+`BottomRiseSpeed(260,264)`
//! / `TextureRepeat(268)` —— `cylinder` 读得最全（0x4c4119–0x4c41c8 一段连着
//! 搬进对象的 `+0x68`..`+0x94`），Pal3.Unity 给这批起的名字也最贴它。
//!
//! （`grid` 和 `meteor` 同样读这批字段，所以它不是 `cylinder` 专属——先前笔记
//! 里写成「只有 cylinder 在读」是因为当时只扫了 cylinder 自己的地址区间。）
//!
//! # 几何
//!
//! 环的生成在 `FUN_004c5350`：
//!
//! ```text
//! n    = 对象+0x70   // DivisionCount，取整
//! step = 对象+0x68 / n   // RotAngle ÷ 段数，每段的角度增量
//! 每段角 a：
//!   底环顶点 = (sin(a) * BottomRadius, 0,     cos(a) * BottomRadius)
//!   顶环顶点 = (sin(a) * TopRadius,    Width, cos(a) * TopRadius)
//! ```
//!
//! 也就是说 **`Width` 是柱高、`RotAngle` 是总共扫过多少度**——名字有点误导，
//! 但代码就是这么用的。`RotAngle` 取 360 就是整圈，取小一点就是一段弧。
//!
//! # 补间
//!
//! 和 `quad` 一样是「每帧 `字段 += dt × 子块速率`」，但吃的子块偏移不同：
//!
//! | 子块 | 对象 | 补的是 |
//! |---|---|---|
//! | +4 / +8 / +12 | +0xac / +0xb0 / +0xb4 | 位置 x / y / z 的速度 |
//! | +40 / +44 / +48 | +0xa0 / +0xa4 / +0xa8 | 绕 X / Y / Z 的转速 |
//! | +84 | +0x9c | **贴图 v 方向的滚动**，累到超过 1.0 就归零 |
//! | +56 / +60 / +64 / +68 | +0x50..+0x5c | A / R / G / B |
//!
//! ARGB 那四个偏移和 `quad` 一致；`+84` 在 `quad` 里是「重复生成的间隔」，
//! 在这里是滚动速度——**同一段子块各类的解释并不相同**。

use std::rc::Rc;

use crosscom::ComRc;
use fileformats::pal3::flexeff::EffectDef;
use radiance::comdef::{IEntity, IEntityExt};
use radiance::components::mesh::{Geometry, TexCoord};
use radiance::math::{Vec2, Vec3};
use radiance::rendering::{BlendMode, ComponentFactory, ParticleMaterialDef, VertexComponents};
use radiance::scene::CoreEntity;

use super::particles::pack_color;

/// 定义记录里画一个 `cylinder` 用得上的静态量。括号里是它搬到对象的哪个
/// 偏移（`FUN_004c4116` 起那一串）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CylinderDef {
    /// `+224` → 对象 `+0x68`：整个柱面扫过多少度。360 就是整圈。
    pub sweep_deg: f32,
    /// `+228` → 对象 `+0x6c`：柱高（顶环的 y）。
    pub height: f32,
    /// `+232` → 对象 `+0x70`：分几段。原版 `fistp` 取整。
    pub divisions: u32,
    /// `+236` / `+240` → 对象 `+0x74` / `+0x78`。
    pub top_shrink: f32,
    pub bottom_shrink: f32,
    /// `+244` / `+248` → 对象 `+0x7c` / `+0x80`：上下半径。
    pub top_radius: f32,
    pub bottom_radius: f32,
    /// `+252` / `+256` → 对象 `+0x84` / `+0x88`：半径每秒涨多少。
    pub top_radius_speed: f32,
    pub bottom_radius_speed: f32,
    /// `+260` / `+264` → 对象 `+0x8c` / `+0x90`：上下环每秒升多高。
    pub top_rise_speed: f32,
    pub bottom_rise_speed: f32,
    /// `+268` → 对象 `+0x94`：贴图沿柱面重复几次。
    pub texture_repeat: f32,
    /// `+180` → 对象 `+0x98`。
    pub fps: f32,
    /// `+184..192` → 对象 `+0xac..0xb4`，加在特效位置上。
    pub offset: [f32; 3],
    /// `+216` / `+196` / `+220` → 对象 `+0xa0` / `+0xa4` / `+0xa8`。
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    /// `+204`，ARGB，和 `quad` 一样拆成四个 0..255 的浮点。
    pub color: u32,
    /// `+213` / `+215`。`cylinder` 不读 `IsBillboard`——它本来就是世界空间的。
    pub additive: bool,
    pub no_ztest: bool,
}

impl CylinderDef {
    pub fn from_def(def: &EffectDef<'_>) -> Self {
        let b = def.raw();
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        Self {
            sweep_deg: f(224),
            height: f(228),
            divisions: f(232).max(0.) as u32,
            top_shrink: f(236),
            bottom_shrink: f(240),
            top_radius: f(244),
            bottom_radius: f(248),
            top_radius_speed: f(252),
            bottom_radius_speed: f(256),
            top_rise_speed: f(260),
            bottom_rise_speed: f(264),
            texture_repeat: f(268),
            fps: f(180),
            offset: [f(184), f(188), f(192)],
            pitch: f(216),
            yaw: f(196),
            roll: f(220),
            color: u(204),
            additive: b[213] != 0,
            no_ztest: b[215] != 0,
        }
    }
}

/// `cylinder` 吃的那组子块速率。偏移和 `quad` 的不一样，见模块文档。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CylinderRates {
    /// 子块 `+4` / `+8` / `+12`。
    pub velocity: [f32; 3],
    /// 子块 `+40` / `+44` / `+48`。
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    /// 子块 `+84`，贴图 v 的滚动速度。
    pub scroll: f32,
    /// 子块 `+56` / `+60` / `+64` / `+68`。
    pub alpha: f32,
    pub red: f32,
    pub green: f32,
    pub blue: f32,
}

impl CylinderRates {
    pub fn from_sub(sub: &[u8]) -> Self {
        let f = |o: usize| f32::from_le_bytes(sub[o..o + 4].try_into().unwrap());
        Self {
            velocity: [f(4), f(8), f(12)],
            pitch: f(40),
            yaw: f(44),
            roll: f(48),
            scroll: f(84),
            alpha: f(56),
            red: f(60),
            green: f(64),
            blue: f(68),
        }
    }
}

/// 一个正在播的 `cylinder`。
#[derive(Debug, Clone)]
pub struct CylinderEffect {
    pub def: CylinderDef,
    rates: CylinderRates,
    /// 和 `quad` 共用的那两个：子块 `+72` 延迟、`+76` 时长。
    pub delay: f32,
    pub duration: f32,
    /// 当下的位置、转角、半径、上下环高度、颜色、贴图滚动量。
    pub position: [f32; 3],
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    pub top_radius: f32,
    pub bottom_radius: f32,
    pub top_y: f32,
    pub bottom_y: f32,
    pub color: [f32; 4],
    pub scroll: f32,
    elapsed: f32,
}

impl CylinderEffect {
    pub fn new(def: CylinderDef, sub: &[u8], origin: [f32; 3]) -> Self {
        let c = def.color;
        Self {
            rates: CylinderRates::from_sub(sub),
            delay: f32::from_le_bytes(sub[72..76].try_into().unwrap()),
            duration: f32::from_le_bytes(sub[76..80].try_into().unwrap()),
            position: [
                origin[0] + def.offset[0],
                origin[1] + def.offset[1],
                origin[2] + def.offset[2],
            ],
            pitch: def.pitch,
            yaw: def.yaw,
            roll: def.roll,
            top_radius: def.top_radius,
            bottom_radius: def.bottom_radius,
            top_y: def.height,
            bottom_y: 0.,
            color: [
                ((c >> 24) & 0xff) as f32,
                ((c >> 16) & 0xff) as f32,
                ((c >> 8) & 0xff) as f32,
                (c & 0xff) as f32,
            ],
            scroll: 0.,
            elapsed: 0.,
            def,
        }
    }

    pub fn finished(&self) -> bool {
        self.elapsed > self.delay + self.duration
    }

    /// 颜色钳进 0..1，顺序和 `Quad::rgba` 一致（R, G, B, A）。
    pub fn rgba(&self) -> [f32; 4] {
        [
            (self.color[1] / 255.).clamp(0., 1.),
            (self.color[2] / 255.).clamp(0., 1.),
            (self.color[3] / 255.).clamp(0., 1.),
            (self.color[0] / 255.).clamp(0., 1.),
        ]
    }

    pub fn step(&mut self, delta_sec: f32) {
        // 和 quad 一样，本帧的 dt 先裁进 [延迟, 延迟+时长]。
        let from = self.elapsed.max(self.delay);
        let to = (self.elapsed + delta_sec).min(self.delay + self.duration);
        let dt = to - from;
        self.elapsed += delta_sec;
        if dt <= 0. {
            return;
        }
        let r = self.rates;
        for i in 0..3 {
            self.position[i] += dt * r.velocity[i];
        }
        self.pitch += dt * r.pitch;
        self.yaw += dt * r.yaw;
        self.roll += dt * r.roll;
        // 半径和上下环的高度按定义里的速度自己走（对象 +0x84..+0x90）。
        self.top_radius += dt * self.def.top_radius_speed;
        self.bottom_radius += dt * self.def.bottom_radius_speed;
        self.top_y += dt * self.def.top_rise_speed;
        self.bottom_y += dt * self.def.bottom_rise_speed;
        self.color[0] += dt * r.alpha;
        self.color[1] += dt * r.red;
        self.color[2] += dt * r.green;
        self.color[3] += dt * r.blue;
        // 滚动量累到超过 1 就归零（0x4c4c8a 的比较）。
        self.scroll += dt * r.scroll;
        if self.scroll > 1.0 {
            self.scroll = 0.;
        }
    }

    /// 这一帧的柱面：`divisions + 1` 圈顶点，每圈一对（下、上）。
    ///
    /// 顶点在特效自己的局部空间里，调用方按 X → Y → Z 转完再平移到
    /// [`Self::position`]。uv 的 u 沿柱面走 `texture_repeat` 圈，v 是
    /// `0..1` 加上 [`Self::scroll`]。
    pub fn rings(&self) -> Vec<([f32; 3], [f32; 3], f32)> {
        let n = self.def.divisions.max(1);
        let step = self.def.sweep_deg / n as f32;
        let repeat = if self.def.texture_repeat == 0. {
            1.
        } else {
            self.def.texture_repeat
        };
        (0..=n)
            .map(|i| {
                let a = (step * i as f32).to_radians();
                let (s, c) = a.sin_cos();
                let u = i as f32 / n as f32 * repeat;
                (
                    [
                        s * self.bottom_radius,
                        self.bottom_y,
                        c * self.bottom_radius,
                    ],
                    [s * self.top_radius, self.top_y, c * self.top_radius],
                    u,
                )
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_def() -> CylinderDef {
        CylinderDef {
            sweep_deg: 360.,
            height: 4.,
            divisions: 8,
            top_shrink: 0.,
            bottom_shrink: 0.,
            top_radius: 2.,
            bottom_radius: 3.,
            top_radius_speed: 0.,
            bottom_radius_speed: 0.,
            top_rise_speed: 0.,
            bottom_rise_speed: 0.,
            texture_repeat: 1.,
            fps: 0.,
            offset: [0., 0., 0.],
            pitch: 0.,
            yaw: 0.,
            roll: 0.,
            color: 0x00cd_cdff,
            additive: true,
            no_ztest: true,
        }
    }

    fn sub_with(pairs: &[(usize, f32)]) -> Vec<u8> {
        let mut b = vec![0u8; 124];
        for (o, v) in pairs {
            b[*o..*o + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    #[test]
    fn a_full_sweep_closes_the_ring() {
        let e = CylinderEffect::new(a_def(), &sub_with(&[(76, 1.0)]), [0., 0., 0.]);
        let r = e.rings();
        assert_eq!(r.len(), 9, "8 段要 9 圈顶点");
        // 第 0 圈和第 8 圈（转满 360°）应当重合。
        for k in 0..3 {
            assert!((r[0].0[k] - r[8].0[k]).abs() < 1e-4);
            assert!((r[0].1[k] - r[8].1[k]).abs() < 1e-4);
        }
        // 底环在 y = 0、顶环在 y = 柱高。
        assert_eq!(r[0].0[1], 0.);
        assert_eq!(r[0].1[1], 4.);
        // 半径按上下分开。
        assert!((r[0].0[2] - 3.).abs() < 1e-4, "底环半径 3");
        assert!((r[0].1[2] - 2.).abs() < 1e-4, "顶环半径 2");
    }

    #[test]
    fn an_arc_does_not_close() {
        let mut d = a_def();
        d.sweep_deg = 90.;
        let e = CylinderEffect::new(d, &sub_with(&[(76, 1.0)]), [0., 0., 0.]);
        let r = e.rings();
        // 起点在 +z、终点转了 90° 到 +x。
        assert!((r[0].0[0]).abs() < 1e-4 && (r[0].0[2] - 3.).abs() < 1e-4);
        assert!((r[8].0[0] - 3.).abs() < 1e-4 && (r[8].0[2]).abs() < 1e-4);
    }

    #[test]
    fn the_radii_and_rings_move_at_the_definitions_speeds() {
        let mut d = a_def();
        d.top_radius_speed = 1.;
        d.bottom_radius_speed = -1.;
        d.top_rise_speed = 2.;
        let mut e = CylinderEffect::new(d, &sub_with(&[(76, 10.0)]), [0., 0., 0.]);
        e.step(1.0);
        assert!((e.top_radius - 3.).abs() < 1e-4);
        assert!((e.bottom_radius - 2.).abs() < 1e-4);
        assert!((e.top_y - 6.).abs() < 1e-4);
    }

    #[test]
    fn the_scroll_wraps_at_one() {
        // 子块 +84 = 0.8：每秒滚 0.8，超过 1 归零。
        let sub = sub_with(&[(76, 10.0), (84, 0.8)]);
        let mut e = CylinderEffect::new(a_def(), &sub, [0., 0., 0.]);
        e.step(1.0);
        assert!((e.scroll - 0.8).abs() < 1e-4);
        e.step(1.0);
        assert_eq!(e.scroll, 0., "累到 1.6 应当归零");
    }

    #[test]
    fn the_tween_respects_the_delay() {
        let sub = sub_with(&[(72, 1.0), (76, 1.0), (4, 5.0)]);
        let mut e = CylinderEffect::new(a_def(), &sub, [0., 0., 0.]);
        e.step(1.0);
        assert_eq!(e.position[0], 0., "延迟内不该动");
        e.step(0.5);
        assert!((e.position[0] - 2.5).abs() < 1e-4);
        e.step(1.0);
        assert!(e.finished());
    }

    #[test]
    fn the_colour_unpacks_the_same_way_quad_does() {
        let e = CylinderEffect::new(a_def(), &sub_with(&[(76, 1.0)]), [0., 0., 0.]);
        assert_eq!(e.color, [0., 205., 205., 255.]);
        assert!((e.rgba()[2] - 1.0).abs() < 1e-6);
    }
}

/// 一个柱面最多画多少段。实机 184 条定义里最多的是 100 段。
pub const MAX_DIVISIONS: usize = 128;

/// 建柱面实体：`MAX_DIVISIONS` 段的三角带，顶点每帧重写。
pub fn build_cylinder_entity(
    factory: &Rc<dyn ComponentFactory>,
    seq: usize,
    texture_name: &str,
    texture: image::RgbaImage,
    additive: bool,
) -> ComRc<IEntity> {
    let rings = MAX_DIVISIONS + 1;
    let mut verts = Vec::with_capacity(rings * 2);
    let mut uv = Vec::with_capacity(rings * 2);
    let mut packed = Vec::with_capacity(rings * 2);
    for _ in 0..rings * 2 {
        verts.push(Vec3::new(0., 0., 0.));
        uv.push(TexCoord::new(0., 0.));
        packed.push(TexCoord::new(0., 0.));
    }
    let mut indices = Vec::with_capacity(MAX_DIVISIONS * 6);
    for i in 0..MAX_DIVISIONS as u32 {
        // 每段两个三角形：(下i, 上i, 上i+1) 和 (下i, 上i+1, 下i+1)。
        let (b0, t0, b1, t1) = (i * 2, i * 2 + 1, i * 2 + 2, i * 2 + 3);
        indices.extend_from_slice(&[b0, t0, t1, b0, t1, b1]);
    }

    let material = ParticleMaterialDef::create_with_image(
        texture_name,
        Some(texture),
        if additive {
            BlendMode::Additive
        } else {
            BlendMode::AlphaBlend
        },
    );
    let geometry = Geometry::new(&verts, None, &[uv, packed], indices, material);
    let object = factory.create_render_object(
        geometry.vertices.clone(),
        geometry.indices.clone(),
        &geometry.material,
        true,
    );
    let component = factory.create_rendering_component(vec![object]);
    let entity = CoreEntity::create(format!("FLEXEFF_CYL_{seq}"), true);
    entity.set_rendering_component(Some(Rc::new(component)));
    entity
}

/// 绕 X → Y → Z，和 `FUN_004c449b` 那一支的调用顺序一致。
fn rotate_xyz(v: [f32; 3], pitch_deg: f32, yaw_deg: f32, roll_deg: f32) -> [f32; 3] {
    let (sx, cx) = pitch_deg.to_radians().sin_cos();
    let (sy, cy) = yaw_deg.to_radians().sin_cos();
    let (sz, cz) = roll_deg.to_radians().sin_cos();
    let v = [v[0], v[1] * cx - v[2] * sx, v[1] * sx + v[2] * cx];
    let v = [v[0] * cy + v[2] * sy, v[1], -v[0] * sy + v[2] * cy];
    [v[0] * cz - v[1] * sz, v[0] * sz + v[1] * cz, v[2]]
}

/// 把这一帧的柱面写进网格。用不到的段折叠成退化三角形。
pub fn write_cylinder_mesh(entity: &ComRc<IEntity>, effect: &CylinderEffect) {
    let Some(rc) = entity.get_rendering_component() else {
        return;
    };
    let rings = effect.rings();
    let color = pack_color(effect.rgba());
    let (pitch, yaw, roll) = (effect.pitch, effect.yaw, effect.roll);
    let pos = effect.position;
    let scroll = effect.scroll;
    for object in rc.render_objects() {
        object.as_dyn().update_vertices(&|mut buffer| {
            for i in 0..=MAX_DIVISIONS {
                let (lo, hi, u) = match rings.get(i) {
                    Some(r) => *r,
                    // 超出这次实际段数的，塌到最后一圈，画出来是退化三角形。
                    None => *rings.last().unwrap(),
                };
                let place = |v: [f32; 3]| {
                    let r = rotate_xyz(v, pitch, yaw, roll);
                    Vec3::new(pos[0] + r[0], pos[1] + r[1], pos[2] + r[2])
                };
                for (k, (v, vt)) in [(lo, scroll), (hi, 1. + scroll)].into_iter().enumerate() {
                    let idx = i * 2 + k;
                    let p = place(v);
                    buffer.set_component(idx, VertexComponents::POSITION, |o: &mut Vec3| *o = p);
                    buffer.set_component(idx, VertexComponents::TEXCOORD, |o: &mut Vec2| {
                        *o = Vec2::new(u, vt)
                    });
                    buffer.set_component(idx, VertexComponents::TEXCOORD2, |o: &mut Vec2| {
                        *o = Vec2::new(color.0, color.1)
                    });
                }
            }
        });
    }
}
