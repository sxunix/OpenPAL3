//! flexeff `meteor` 类（`CEffMeteor`，源文件 `D:\Project\PAL3\combat\EffMeteor.cpp`）：
//! 一串追着目标飞的面片，每颗身后拖一条带子。夜游 / 日游的 413 等特效靠它出画面。
//!
//! 逆向记在 `switch/re/notes/combat_vfx.md` §7，要点：
//!
//! * 定义记录（原型 `FUN_004cb800` 建，vtable 0x56f978 的 `+8` 0x4cb5e0 摆位）：
//!   - 颗数取 `|round(+0xe0)|`，`+0xe0 > 0` 才画头部面片；
//!   - 起点 = 播放点 + `+0xb8..`，目标 = 播放点 + `+0xfc..`；
//!   - 初速 `(+0x108, sin θ·+0xec, cos θ·+0xf0)`，θ = i·2π/颗数（只有一颗时不乘 sin/cos，
//!     FUN_004ccda0）；
//!   - 拖尾：节点数 `round(+0xe8)`，宽 `+0xe4`，alpha `+0x10c`；每秒分别减 `+0xf8` / `+0xf4`；
//!   - 头部面片的图集、帧率、大小、颜色、自转、加法标记和 quad 类读的是同一批字段，
//!     直接用 [`QuadDef`]。
//! * 实例时间线（`+0x30` 0x4cc290 排、0x4cc4c0 走）：延迟子块 `+72`，持续 `+76`。
//!   这一步里每帧：
//!   - 到 `+60` 秒发射；
//!   - 自转 += `+48`·dt，起点 += `+4..+12`·dt；
//!   - 颜色 A/R/G/B 各 += `+56/+60/+64/+68`·dt（四个里有一个非零才做，截在 0..255，
//!     拖尾 alpha 跟着 A 变）；
//!   - 半宽 += `+0`·dt、半高 += `+84`·dt；
//!   - 已经发射就推运动。
//!
//!   这一步走完就清空拖尾，实例结束。
//! * 运动（FUN_004ccef0）：固定步长 `0.02 × 速度倍率`（倍率由 `FUN_004cdc50` 写，缺省 1）。
//!   每步：加速度 = 目标 − 位置，速度 += 步长·加速度，位置 += 步长·速度。这是一个没有阻尼
//!   的弹簧，陨石扇形飞出，再绕向目标。每步把新位置压进拖尾（FUN_004cd340）。
//! * 拖尾（FUN_004cd3d0 / FUN_004cd500）：
//!   - 节点按压入先后排，年龄 = 前面各节点的步长之和；
//!   - alpha = 初值 − 年龄·衰减，宽 = 初值 − 年龄·衰减；
//!   - 相邻节点之间拼朝向相机的带子，侧向 = 归一化((本节点 − 下一节点) × (本节点 − 相机))
//!     × 宽/2；
//!   - 颜色取当前颜色的 RGB，alpha 用节点的；一律加法混合、做深度测试；
//!   - 有头部面片时带子不贴图，否则 u 沿长度按节点序号走。

use std::collections::VecDeque;
use std::rc::Rc;

use crosscom::ComRc;
use fileformats::pal3::flexeff::EffectDef;
use radiance::comdef::{IEntity, IEntityExt};
use radiance::components::mesh::{Geometry, TexCoord};
use radiance::math::{Vec2, Vec3};
use radiance::rendering::{BlendMode, ComponentFactory, ParticleMaterialDef, VertexComponents};
use radiance::scene::CoreEntity;

use super::flexeff_quad::QuadDef;
use super::particles::pack_color;

/// 运动的固定步长：FUN_004ccef0 的 `+0xa92c × 0.02`，`+0xa92c` 构造时是 1.0，
/// 只有战斗改速度（FUN_00513600）会动它。
pub const STEP_SECONDS: f32 = 0.02;

/// 拖尾节点数的上限，只是给顶点缓冲定个容量。实机最大 600。
pub const MAX_TRAIL_NODES: usize = 1024;

/// 定义记录里 `meteor` 用得上的量。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeteorDef {
    /// 头部面片：图集 168/172/176、帧率 180、起点偏移 184..192、大小 200、
    /// 颜色 204、加法 213、自转 220。
    pub quad: QuadDef,
    /// `+0xfc..+0x104`：目标相对播放点的偏移。
    pub end: [f32; 3],
    /// `round(+0xe0)`：绝对值是颗数，正数才画头部面片。
    pub count: i32,
    /// `+0xe4` / `+0x10c`：拖尾节点的初始宽度和 alpha。
    pub trail_width: f32,
    pub trail_alpha: f32,
    /// `round(+0xe8)`：每条拖尾几个节点。FUN_004cd0d0 少于两个就抛异常。
    pub trail_nodes: usize,
    /// `+0xec` / `+0xf0`：初速在 y / z 上的扇形幅度。
    pub spread: [f32; 2],
    /// `+0xf4` / `+0xf8`：拖尾 alpha / 宽度每秒减多少。
    pub alpha_fade: f32,
    pub width_fade: f32,
    /// `+0x108`：初速的 x。
    pub speed: f32,
}

impl MeteorDef {
    pub fn from_def(def: &EffectDef<'_>) -> Self {
        let b = def.raw();
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        Self {
            quad: QuadDef::from_def(def),
            end: [f(0xfc), f(0x100), f(0x104)],
            count: f(0xe0).round_ties_even() as i32,
            trail_width: f(0xe4),
            trail_alpha: f(0x10c),
            trail_nodes: (f(0xe8).round_ties_even().max(2.) as usize).min(MAX_TRAIL_NODES),
            spread: [f(0xec), f(0xf0)],
            alpha_fade: f(0xf4),
            width_fade: f(0xf8),
            speed: f(0x108),
        }
    }

    /// 几颗陨石。
    pub fn meteors(&self) -> usize {
        self.count.unsigned_abs() as usize
    }

    /// 画不画头部面片（`+0x47 = +0xe0 > 0`）。
    pub fn has_heads(&self) -> bool {
        self.count > 0
    }
}

/// 子块里 `meteor` 用到的字段（实例 `+0x18` 起的那 124 字节）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MeteorSub {
    /// `+72` / `+76`：时间线这一步的延迟和时长。
    pub delay: f32,
    pub duration: f32,
    /// `+60`：这一步开始后第几秒发射。实机全是 0。
    pub launch_at: f32,
    /// `+0` / `+84`：半宽 / 半高每秒长多少。
    pub grow: [f32; 2],
    /// `+48`：自转每秒几度。
    pub spin: f32,
    /// `+4..+12`：发射前起点每秒挪多少。
    pub drift: [f32; 3],
    /// `+56/+60/+64/+68`：A/R/G/B 每秒变多少。
    pub color_rate: [f32; 4],
}

impl MeteorSub {
    pub fn from_sub(sub: &[u8]) -> Self {
        let f = |o: usize| f32::from_le_bytes(sub[o..o + 4].try_into().unwrap());
        Self {
            delay: f(72),
            duration: f(76),
            launch_at: f(60),
            grow: [f(0), f(84)],
            spin: f(48),
            drift: [f(4), f(8), f(12)],
            color_rate: [f(56), f(60), f(64), f(68)],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Node {
    position: [f32; 3],
    /// 压入这个节点的那一步的步长，用来累年龄。
    step: f32,
}

/// 一颗陨石（实例 `+0x38` 数组里 0x30 字节的一条）。
#[derive(Debug, Clone)]
pub struct Meteor {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    target: [f32; 3],
    /// 最新的在前。
    nodes: VecDeque<Node>,
}

/// 拖尾上一个还亮着的节点。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrailPoint {
    pub position: [f32; 3],
    /// 0..255。
    pub alpha: f32,
    pub width: f32,
}

/// 一颗陨石的头部面片。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Head {
    pub position: [f32; 3],
    pub half_width: f32,
    pub half_height: f32,
    /// 度。
    pub roll: f32,
    pub rgba: [f32; 4],
    pub frame: u32,
}

/// 一个正在播的 `meteor` 实例。
#[derive(Debug, Clone)]
pub struct MeteorEffect {
    pub def: MeteorDef,
    sub: MeteorSub,
    start: [f32; 3],
    target: [f32; 3],
    t: f32,
    launched: bool,
    done: bool,
    half_width: f32,
    half_height: f32,
    roll: f32,
    /// A, R, G, B，0..255。
    color: [f32; 4],
    trail_alpha: f32,
    frame_clock: f32,
    physics_clock: f32,
    meteors: Vec<Meteor>,
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

impl MeteorEffect {
    pub fn new(def: MeteorDef, sub: &[u8], origin: [f32; 3]) -> Self {
        let c = def.quad.color;
        Self {
            def,
            sub: MeteorSub::from_sub(sub),
            start: add(origin, def.quad.offset),
            target: add(origin, def.end),
            t: 0.,
            launched: false,
            done: false,
            half_width: def.quad.size,
            half_height: def.quad.size,
            roll: def.quad.roll,
            color: [
                ((c >> 24) & 0xff) as f32,
                ((c >> 16) & 0xff) as f32,
                ((c >> 8) & 0xff) as f32,
                (c & 0xff) as f32,
            ],
            trail_alpha: def.trail_alpha,
            frame_clock: 0.,
            physics_clock: 0.,
            meteors: vec![],
        }
    }

    pub fn finished(&self) -> bool {
        self.done
    }

    pub fn meteors(&self) -> &[Meteor] {
        &self.meteors
    }

    /// 头部面片的顶点缓冲要几个四边形。
    pub fn head_capacity(&self) -> usize {
        self.def.meteors().max(1)
    }

    /// 拖尾的顶点缓冲要几个四边形。
    pub fn trail_capacity(&self) -> usize {
        (self.def.meteors() * (self.def.trail_nodes - 1)).max(1)
    }

    /// FUN_004ccda0：摆好每颗的起点、目标和初速。
    fn launch(&mut self) {
        let n = self.def.meteors();
        let [sy, sz] = self.def.spread;
        self.meteors = (0..n)
            .map(|i| {
                // `sin(i × 1130.9734 / n × (1/180))`，1130.9734 / 180 = 2π。
                let (s, c) = if n == 1 {
                    (1., 1.)
                } else {
                    (i as f32 * std::f32::consts::TAU / n as f32).sin_cos()
                };
                Meteor {
                    position: self.start,
                    velocity: [self.def.speed, s * sy, c * sz],
                    target: self.target,
                    nodes: VecDeque::new(),
                }
            })
            .collect();
        self.physics_clock = 0.;
        self.launched = true;
    }

    /// FUN_004ccef0：按固定步长推运动。步数 `round(累计 / 步长)`，x87 缺省的
    /// 就近取偶，余数可能是负的。
    fn integrate(&mut self, delta_sec: f32) {
        self.physics_clock += delta_sec;
        let h = STEP_SECONDS;
        let n = (self.physics_clock / h).round_ties_even();
        self.physics_clock -= n * h;
        let keep = self.def.trail_nodes;
        for _ in 0..n.max(0.) as u32 {
            for m in self.meteors.iter_mut() {
                let a = sub(m.target, m.position);
                m.velocity = add(m.velocity, scale(a, h));
                m.position = add(m.position, scale(m.velocity, h));
                m.nodes.push_front(Node {
                    position: m.position,
                    step: h,
                });
                m.nodes.truncate(keep);
            }
        }
    }

    pub fn step(&mut self, delta_sec: f32) {
        if self.done {
            return;
        }
        self.t += delta_sec;
        if self.t > self.sub.delay {
            // 0x4cc4c0：这一步用整帧的 dt，只在收尾那帧截掉超出的部分。
            let local = self.t - self.sub.delay;
            let mut d = delta_sec;
            if local > self.sub.duration {
                d -= local - self.sub.duration;
            }
            let d = d.max(0.);
            if !self.launched && local >= self.sub.launch_at {
                self.launch();
            }
            self.roll += d * self.sub.spin;
            self.start = add(self.start, scale(self.sub.drift, d));
            if self.sub.color_rate.iter().any(|r| *r != 0.) {
                for k in 0..4 {
                    self.color[k] = (self.color[k] + d * self.sub.color_rate[k]).clamp(0., 255.);
                }
                self.trail_alpha = (self.trail_alpha + d * self.sub.color_rate[0]).clamp(0., 255.);
            }
            self.half_width += d * self.sub.grow[0];
            self.half_height += d * self.sub.grow[1];
            if self.launched {
                self.integrate(d);
            }
            if local >= self.sub.duration {
                // FUN_004cd3a0 清空每条拖尾，时间线这一步置 4，实例随之结束。
                self.meteors.clear();
                self.done = true;
                return;
            }
        }
        // FUN_004cbfc0：有头部面片才走图集帧。
        if self.def.has_heads() {
            self.frame_clock += self.def.quad.fps * delta_sec;
        }
    }

    /// 当前颜色的 R、G、B（0..255）。
    pub fn rgb(&self) -> [f32; 3] {
        [self.color[1], self.color[2], self.color[3]]
    }

    /// FUN_004cd3d0：一条拖尾上还亮着的节点，最新的在前。
    ///
    /// 节点 alpha 或宽度到 0、而且下一个节点也到 0 时，从这里截断。
    pub fn trail(&self, m: &Meteor) -> Vec<TrailPoint> {
        let at = |age: f32| {
            (
                self.trail_alpha - age * self.def.alpha_fade,
                self.def.trail_width - age * self.def.width_fade,
            )
        };
        let mut out = vec![];
        let mut age = 0.;
        for (i, node) in m.nodes.iter().enumerate() {
            let (alpha, width) = at(age);
            if alpha <= 0. || width <= 0. {
                let next_dead = m.nodes.get(i + 1).is_none_or(|_| {
                    let (a, w) = at(age + node.step);
                    a <= 0. || w <= 0.
                });
                if next_dead {
                    break;
                }
            }
            out.push(TrailPoint {
                position: node.position,
                alpha: alpha.clamp(0., 255.),
                width,
            });
            age += node.step;
        }
        out
    }

    /// 这一帧的头部面片。没发射、已结束或定义不画头部时为空。
    pub fn heads(&self) -> Vec<Head> {
        if self.done || !self.def.has_heads() {
            return vec![];
        }
        let frames = self.def.quad.frame_count.max(1);
        let frame = (self.frame_clock.round_ties_even().abs() as u32) % frames;
        let [a, r, g, b] = self.color;
        let rgba = [r / 255., g / 255., b / 255., a / 255.];
        self.meteors
            .iter()
            .map(|m| Head {
                position: m.position,
                half_width: self.half_width,
                half_height: self.half_height,
                roll: self.roll,
                rgba,
                frame,
            })
            .collect()
    }
}

/// 建一个动态网格实体：`capacity` 个四边形，顶点每帧重写（meteor / shock 共用）。
pub fn build_meteor_entity(
    factory: &Rc<dyn ComponentFactory>,
    name: String,
    texture_name: &str,
    texture: image::RgbaImage,
    additive: bool,
    capacity: usize,
) -> ComRc<IEntity> {
    let mut verts = Vec::with_capacity(capacity * 4);
    let mut uv = Vec::with_capacity(capacity * 4);
    let mut packed = Vec::with_capacity(capacity * 4);
    let mut indices = Vec::with_capacity(capacity * 6);
    for i in 0..capacity as u32 {
        for _ in 0..4 {
            verts.push(Vec3::new(0., 0., 0.));
            uv.push(TexCoord::new(0., 0.));
            packed.push(TexCoord::new(0., 0.));
        }
        let b = i * 4;
        indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
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
    let entity = CoreEntity::create(name, true);
    entity.set_rendering_component(Some(Rc::new(component)));
    entity
}

/// 一个四边形的四个顶点：位置、uv、打包好的颜色。
pub(crate) type QuadVerts = [(Vec3, (f32, f32), (f32, f32)); 4];

/// 把一批四边形写进 [`build_meteor_entity`] 建的网格，多出来的位置清零。
pub(crate) fn write_quads(entity: &ComRc<IEntity>, capacity: usize, quads: &[QuadVerts]) {
    let Some(rc) = entity.get_rendering_component() else {
        return;
    };
    let empty = (Vec3::new(0., 0., 0.), (0., 0.), (0., 0.));
    for object in rc.render_objects() {
        object.as_dyn().update_vertices(&|mut buffer| {
            for i in 0..capacity {
                for k in 0..4 {
                    let (p, uv, c) = quads.get(i).map(|q| q[k]).unwrap_or(empty);
                    let v = i * 4 + k;
                    buffer.set_component(v, VertexComponents::POSITION, |x: &mut Vec3| *x = p);
                    buffer.set_component(v, VertexComponents::TEXCOORD, |t: &mut Vec2| {
                        *t = Vec2::new(uv.0, uv.1)
                    });
                    buffer.set_component(v, VertexComponents::TEXCOORD2, |t: &mut Vec2| {
                        *t = Vec2::new(c.0, c.1)
                    });
                }
            }
        });
    }
}

/// 头部面片：陨石位置上的公告板（原版把位置变换到视图空间再摆 `(±w, ±h)`），
/// 按自转角在平面内转。
pub fn write_meteor_heads(entity: &ComRc<IEntity>, effect: &MeteorEffect, right: &Vec3, up: &Vec3) {
    let def = effect.def.quad;
    let quads: Vec<QuadVerts> = effect
        .heads()
        .iter()
        .map(|h| {
            let (s, c) = h.roll.to_radians().sin_cos();
            let rot = |x: f32, y: f32| (x * c - y * s, x * s + y * c);
            let (w, hh) = (h.half_width, h.half_height);
            let corners = [rot(-w, hh), rot(w, hh), rot(w, -hh), rot(-w, -hh)];
            let (u0, v0, u1, v1) = def.frame_uv(h.frame);
            let uvs = [(u0, v0), (u1, v0), (u1, v1), (u0, v1)];
            let color = pack_color(h.rgba);
            let centre = Vec3::new(h.position[0], h.position[1], h.position[2]);
            std::array::from_fn(|k| {
                let p = Vec3::add(
                    &centre,
                    &Vec3::add(
                        &Vec3::scalar_mul(corners[k].0, right),
                        &Vec3::scalar_mul(corners[k].1, up),
                    ),
                );
                (p, uvs[k], color)
            })
        })
        .collect();
    write_quads(entity, effect.head_capacity(), &quads);
}

/// 拖尾：相邻亮着的节点之间一段带子。
///
/// 侧向照 FUN_004cd500：`(本节点 − 下一节点) × (本节点 − 相机)` 归一化（零向量按
/// `gbVec3D::Normalize` 当 (0,1,0)），乘本节点宽度的一半；最后一个节点没有下一个，
/// 沿用上一个的方向。贴图时 u 按节点序号 `i / 节点数` 走（FUN_004cd3d0 写的 uv 表），
/// v 横跨带子 0..1 **[推断]**（哪一侧是 0 没核）。
pub fn write_meteor_trails(entity: &ComRc<IEntity>, effect: &MeteorEffect, eye: &Vec3) {
    let e = [eye.x, eye.y, eye.z];
    let [r, g, b] = effect.rgb();
    let textured = !effect.def.has_heads();
    let mut quads: Vec<QuadVerts> = vec![];
    for m in effect.meteors() {
        let pts = effect.trail(m);
        let n = pts.len();
        if n < 2 {
            continue;
        }
        let mut dir = [0., 1., 0.];
        let sides: Vec<[f32; 3]> = (0..n)
            .map(|i| {
                if i + 1 < n {
                    let c = cross(
                        sub(pts[i].position, pts[i + 1].position),
                        sub(pts[i].position, e),
                    );
                    let len = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
                    dir = if len > 1e-6 {
                        scale(c, 1. / len)
                    } else {
                        [0., 1., 0.]
                    };
                }
                scale(dir, pts[i].width * 0.5)
            })
            .collect();
        let vert = |i: usize, sign: f32, v: f32| {
            let p = add(pts[i].position, scale(sides[i], sign));
            let u = if textured { i as f32 / n as f32 } else { 0. };
            let color = pack_color([r / 255., g / 255., b / 255., pts[i].alpha / 255.]);
            (Vec3::new(p[0], p[1], p[2]), (u, v), color)
        };
        for i in 0..n - 1 {
            quads.push([
                vert(i, -1., 0.),
                vert(i, 1., 1.),
                vert(i + 1, 1., 1.),
                vert(i + 1, -1., 0.),
            ]);
        }
    }
    write_quads(entity, effect.trail_capacity(), &quads);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_def(count: i32) -> MeteorDef {
        MeteorDef {
            quad: QuadDef {
                cols: 1,
                rows: 1,
                frame_count: 1,
                fps: 0.,
                offset: [0., 0., 0.],
                size: 10.,
                color: 0xff80_8080,
                max_quads: 1,
                additive: true,
                billboard: true,
                no_ztest: false,
                yaw: 0.,
                pitch: 0.,
                roll: 0.,
            },
            end: [100., 0., 0.],
            count,
            trail_width: 20.,
            trail_alpha: 255.,
            trail_nodes: 10,
            spread: [50., 60.],
            alpha_fade: 100.,
            width_fade: 0.,
            speed: 30.,
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
    fn the_meteors_fan_out_on_launch() {
        let mut e = MeteorEffect::new(a_def(4), &sub_with(&[(76, 1.)]), [0.; 3]);
        // 不到半步：发射了，但还没推。
        e.step(0.001);
        let m = e.meteors();
        assert_eq!(m.len(), 4);
        // θ = 0：(速度, 0, z 幅度)；θ = 90°：(速度, y 幅度, 0)。
        assert_eq!(m[0].velocity, [30., 0., 60.]);
        assert!((m[1].velocity[1] - 50.).abs() < 1e-3 && m[1].velocity[2].abs() < 1e-3);
    }

    #[test]
    fn a_single_meteor_takes_the_spread_as_is() {
        let mut e = MeteorEffect::new(a_def(1), &sub_with(&[(76, 1.)]), [0.; 3]);
        e.step(0.001);
        assert_eq!(e.meteors()[0].velocity, [30., 50., 60.]);
    }

    #[test]
    fn each_step_pulls_toward_the_target() {
        let mut e = MeteorEffect::new(a_def(4), &sub_with(&[(76, 1.)]), [0.; 3]);
        // 正好一步：v += 0.02 × (目标 − 位置)，p += 0.02 × v。
        e.step(STEP_SECONDS);
        let m = &e.meteors()[0];
        assert!((m.velocity[0] - 32.).abs() < 1e-4);
        assert!((m.position[0] - 0.64).abs() < 1e-4);
        assert!((m.position[2] - 1.2).abs() < 1e-4);
    }

    #[test]
    fn the_trail_fades_with_age_and_keeps_its_node_count() {
        let mut e = MeteorEffect::new(a_def(1), &sub_with(&[(76, 10.)]), [0.; 3]);
        for _ in 0..5 {
            e.step(STEP_SECONDS);
        }
        let t = e.trail(&e.meteors()[0]);
        assert_eq!(t.len(), 5);
        // 每老一步（0.02 秒）alpha 少 100 × 0.02 = 2。
        assert!((t[0].alpha - 255.).abs() < 1e-3);
        assert!((t[1].alpha - 253.).abs() < 1e-3);
        assert!((t[4].alpha - 247.).abs() < 1e-3);
        for _ in 0..50 {
            e.step(STEP_SECONDS);
        }
        assert_eq!(e.trail(&e.meteors()[0]).len(), 10, "节点数封顶");
    }

    #[test]
    fn it_waits_for_the_delay_and_ends_with_the_duration() {
        let mut e = MeteorEffect::new(a_def(2), &sub_with(&[(72, 0.5), (76, 1.)]), [0.; 3]);
        e.step(0.4);
        assert!(e.meteors().is_empty(), "延迟里还没发射");
        e.step(0.2);
        assert_eq!(e.meteors().len(), 2);
        assert_eq!(e.heads().len(), 2);
        e.step(1.0);
        assert!(e.finished());
        assert!(e.heads().is_empty() && e.meteors().is_empty());
    }

    #[test]
    fn a_negative_count_means_trails_only() {
        let mut e = MeteorEffect::new(a_def(-3), &sub_with(&[(76, 1.)]), [0.; 3]);
        e.step(0.1);
        assert_eq!(e.meteors().len(), 3);
        assert!(e.heads().is_empty(), "+0xe0 为负不画头部");
    }

    #[test]
    fn colour_rates_change_the_trail_alpha_too() {
        // A 每秒 −100：头部 alpha 和拖尾初始 alpha 一起降。
        let mut e = MeteorEffect::new(a_def(1), &sub_with(&[(76, 10.), (56, -100.)]), [0.; 3]);
        e.step(0.5);
        let h = e.heads()[0];
        assert!((h.rgba[3] * 255. - 205.).abs() < 1e-2);
        let t = e.trail(&e.meteors()[0]);
        assert!((t[0].alpha - 205.).abs() < 1e-2);
    }
}
