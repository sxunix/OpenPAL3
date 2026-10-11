//! flexeff `explosive` 类（`CEffExplosive`，源文件 `D:\Project\PAL3\combat\EffExplosive.cpp`）：
//! 把一张图切成 列 × 行 块，炸开、翻滚着飞散（「电光暴烈」「心心炸」「m02 雷暴」……）。
//!
//! 逆向记在 `switch/re/notes/combat_vfx.md` §13，要点（vtable 0x56f878）：
//!
//! * `+4` 0x4c6c10 读定义：
//!   - 列 `(int)+0xe0`、行 `(int)+0xe8`、格宽 `+0xe4`；
//!   - 基准尺寸 `+0xf4`、最小速度 `+0xf8`、速度偏置 (`+0xfc`, `+0x100`, `+0x10c`)、衰减 `+0x104`；
//!   - 图集、帧率、偏移、大小、颜色、加法、不测深度和 quad 类读同一批字段（[`QuadDef`]）；
//!   - 转角 `+0xd8 / +0xc4 / +0xdc`（绕 X / Y / Z）。
//! * `+8` 0x4c6a70 摆位，末尾 FUN_004c7d40 重铺碎片：第 (i 列, j 行) 块的四角是
//!   x = −列·宽/2 + i·宽 起、z = 行·宽/2 − j·宽 起、边长一格，y = 0。
//! * 时间线（`+0x30` 0x4c7450 排、0x4c7680 走）：延迟子块 `+72`，持续 `+76`。这一步里每帧：
//!   - 第一次进来置可见；到 `+60` 秒发射（FUN_004c7e60）；
//!   - 转角 += `+40/+44/+48`·dt，位置 += `+4..+12`·dt，绕圈（`+20..+36`）；
//!   - A/R/G/B += `+56..+68`·dt（任一非零才做，截 0..255）；缩放 x += `+0`·dt、z += `+84`·dt；
//!   - 发射之后推运动（FUN_004c8370）。
//! * 发射：每块的尺寸 = round(基准 − |a − 列/2|·|b − 行/2|·衰减)，不小于最小速度（原版这里把
//!   外层 / 内层下标和列 / 行交叉着用，照抄）；初速三轴各取 `rand() % 尺寸 − 尺寸/2`，长度不足
//!   最小速度的拉到最小速度，再加偏置；自转三轴各取 `(rand() % 160 − 80) × 16` 度/秒。
//! * 运动：每块绕自己的中心 `SetYawPitchRoll(dt × 自转)` 翻滚（gbengine 0x10026670，参数是度），
//!   中心按速度直线飞，没有重力。
//! * 每帧（`+0x14` 0x4c7150）：帧时钟 += 帧率·dt，当前帧的 uv 方框按列、行切成小格分给各块。
//! * 画（`+0x18` 0x4c6f20）：平移(位置 + 绕圈) → 缩放(x, 1, z) → 绕 X → 绕 Y → 绕 Z。
//!
//! 几件底层约定（都在 `engine/math/gbmath.cpp` 里查实，不再是推断）：
//!
//! * 角度按**度**：`gbCos(x)` 就是 `cos(PI*x/180)`（`gbmath.h:262`）。
//! * 旋转是右手系：`gbMatrix4::SetAxisAngle(GB_AXIS_Y, θ)` 给的是
//!   `xx=cos, xz=−sin, zx=sin, zz=cos`，矩阵按行向量用
//!   （`Transform`: `out.x = in.x·xx + in.y·yx + in.z·zx`），
//!   所以模型 +Z 转到 `(sin θ, cos θ)`；走四元数那条路
//!   （`gbQuaternion::SetAxisAngle` + `GetMatrix`）出来的矩阵一模一样。
//! * 矩阵栈确实「后压的先作用」：`gbMatrixStack::Rotate` 做的是
//!   `LeftMultiply`，而 `LeftMultiply(mat)` 的实现是 `this = mat · this`
//!   （`gbmath.cpp:132` + `operator*=`），配上行向量 `v' = v · M`，
//!   最后压进去的那一步最先作用在顶点上。
//! * **翻滚的顺序是 roll(Z) → yaw(Y) → pitch(X)**，不是 D3DX 的 Z → X → Y。
//!   `gbQuaternion::SetYawPitchRoll(yaw, pitch, roll)`（`gbmath.cpp:737`）算的是
//!   `q = q_X(pitch) ⊗ q_Y(yaw) ⊗ q_Z(roll)`（展开式正好是标准 Hamilton 积），
//!   对应矩阵 `R_X · R_Y · R_Z`，作用在顶点上就是 Z 先、Y 次、X 最后。
//!   `EffExplosive.cpp:635` 把自转的 x 分量当 yaw（绕 Y）、y 分量当 pitch（绕 X）
//!   传进去，这里的 `spin[0]` / `spin[1]` 也照这个对应。
//!
//! 还剩一处移植端自己定的：`rand()` 用每个实例自己的种子，原版是全局的 MSVC `rand`。

use crosscom::ComRc;
use fileformats::pal3::flexeff::EffectDef;
use radiance::comdef::IEntity;
use radiance::math::Vec3;

use super::flexeff_meteor::{QuadVerts, write_quads};
use super::flexeff_quad::QuadDef;
use super::particles::pack_color;

/// 碎片数上限，只是给顶点缓冲定个容量。实机最大 50 × 50 = 2500。
pub const MAX_EXPLOSIVE_TILES: usize = 4096;

/// 定义记录里 `explosive` 用得上的量。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExplosiveDef {
    pub quad: QuadDef,
    pub cols: usize,
    pub rows: usize,
    pub cell: f32,
    /// `+0xf4`：发射时的基准尺寸。
    pub base: f32,
    /// `+0xf8`：初速的最小长度（也是尺寸下限）。
    pub min_speed: f32,
    /// `+0xfc / +0x100 / +0x10c`：加在每块初速上的偏置。
    pub bias: [f32; 3],
    /// `+0x104`：越靠边尺寸减得越多。
    pub falloff: f32,
    /// `+0xd8 / +0xc4 / +0xdc`：绕 X / Y / Z（度）。
    pub rotation: [f32; 3],
}

impl ExplosiveDef {
    pub fn from_def(def: &EffectDef<'_>) -> Self {
        let b = def.raw();
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let count = |v: f32| (v as i32).max(0) as usize;
        Self {
            quad: QuadDef::from_def(def),
            cols: count(f(0xe0)),
            rows: count(f(0xe8)),
            cell: f(0xe4),
            base: f(0xf4),
            min_speed: f(0xf8),
            bias: [f(0xfc), f(0x100), f(0x10c)],
            falloff: f(0x104),
            rotation: [f(0xd8), f(0xc4), f(0xdc)],
        }
    }

    pub fn tiles(&self) -> usize {
        (self.cols * self.rows).min(MAX_EXPLOSIVE_TILES)
    }

    /// FUN_004c7d40：第 `k` 块（行优先）的四个局部角点。
    fn layout(&self, k: usize) -> [[f32; 3]; 4] {
        let (i, j) = (k % self.cols.max(1), k / self.cols.max(1));
        let x0 = -(self.cols as f32) * self.cell * 0.5 + i as f32 * self.cell;
        let z0 = self.rows as f32 * self.cell * 0.5 - j as f32 * self.cell;
        let (x1, z1) = (x0 + self.cell, z0 - self.cell);
        [[x0, 0., z0], [x1, 0., z0], [x1, 0., z1], [x0, 0., z1]]
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Tile {
    corners: [[f32; 3]; 4],
    velocity: [f32; 3],
    /// yaw / pitch / roll，度每秒。
    spin: [f32; 3],
}

/// 一个正在播的 `explosive` 实例。
#[derive(Debug, Clone)]
pub struct ExplosiveEffect {
    pub def: ExplosiveDef,
    delay: f32,
    duration: f32,
    launch_at: f32,
    drift: [f32; 3],
    turn: [f32; 3],
    axis: [f32; 3],
    radius: f32,
    orbit_speed: f32,
    color_rate: [f32; 4],
    grow: [f32; 2],
    position: [f32; 3],
    orbit: [f32; 3],
    rotation: [f32; 3],
    scale: [f32; 2],
    /// A, R, G, B（0..255）。
    color: [f32; 4],
    frame_clock: f32,
    t: f32,
    started: bool,
    launched: bool,
    done: bool,
    rng: u32,
    tiles: Vec<Tile>,
}

/// `gbQuaternion::SetYawPitchRoll` 的展开：先绕 Z 转 roll，再绕 Y 转 yaw，
/// 最后绕 X 转 pitch（见模块开头）。
fn rotate_ypr(v: [f32; 3], yaw: f32, pitch: f32, roll: f32) -> [f32; 3] {
    let (sz, cz) = roll.to_radians().sin_cos();
    let v = [v[0] * cz - v[1] * sz, v[0] * sz + v[1] * cz, v[2]];
    let (sy, cy) = yaw.to_radians().sin_cos();
    let v = [v[0] * cy + v[2] * sy, v[1], -v[0] * sy + v[2] * cy];
    let (sx, cx) = pitch.to_radians().sin_cos();
    [v[0], v[1] * cx - v[2] * sx, v[1] * sx + v[2] * cx]
}

impl ExplosiveEffect {
    pub fn new(def: ExplosiveDef, sub: &[u8], origin: [f32; 3], seed: u32) -> Self {
        let f = |o: usize| f32::from_le_bytes(sub[o..o + 4].try_into().unwrap());
        let c = def.quad.color;
        let o = def.quad.offset;
        let tiles = (0..def.tiles())
            .map(|k| Tile {
                corners: def.layout(k),
                velocity: [0.; 3],
                spin: [0.; 3],
            })
            .collect();
        Self {
            def,
            delay: f(72).max(0.),
            duration: f(76).max(0.),
            launch_at: f(60),
            drift: [f(4), f(8), f(12)],
            turn: [f(40), f(44), f(48)],
            axis: [f(20), f(24), f(28)],
            radius: f(32),
            orbit_speed: f(36),
            color_rate: [f(56), f(60), f(64), f(68)],
            grow: [f(0), f(84)],
            position: [origin[0] + o[0], origin[1] + o[1], origin[2] + o[2]],
            orbit: [0.; 3],
            rotation: def.rotation,
            scale: [def.quad.size; 2],
            color: [
                ((c >> 24) & 0xff) as f32,
                ((c >> 16) & 0xff) as f32,
                ((c >> 8) & 0xff) as f32,
                (c & 0xff) as f32,
            ],
            frame_clock: 0.,
            t: 0.,
            started: false,
            launched: false,
            done: false,
            rng: seed,
            tiles,
        }
    }

    pub fn finished(&self) -> bool {
        self.done
    }

    pub fn capacity(&self) -> usize {
        self.def.tiles().max(1)
    }

    fn rand(&mut self) -> u32 {
        self.rng = self.rng.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        (self.rng >> 16) & 0x7fff
    }

    /// FUN_004c7e60。
    fn launch(&mut self) {
        let d = self.def;
        let base = {
            let b = d.base.round_ties_even() as i32;
            if b < 1 { 2 } else { b }
        };
        let falloff = d.falloff.max(0.);
        let min = d.min_speed;
        let (cols, rows) = (d.cols as i32, d.rows as i32);
        let mut k = 0;
        for outer in 0..cols {
            for inner in 0..rows {
                if k >= self.tiles.len() {
                    return;
                }
                let a = (inner - cols / 2).abs() as f32;
                let b = (outer - rows / 2).abs() as f32;
                let mut size = (base as f32 - a * b * falloff).round_ties_even() as i32;
                if (size as f32) < min {
                    size = min.round_ties_even() as i32;
                }
                let size = size.max(1);
                let half = (size / 2) as f32;
                let mut v = [0f32; 3];
                for c in v.iter_mut() {
                    *c = (self.rand() as i32 % size) as f32 - half;
                }
                let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                if len < min {
                    // gbVec3D::Normalize：零向量当 (0,1,0)。
                    let n = if len > 1e-6 {
                        [v[0] / len, v[1] / len, v[2] / len]
                    } else {
                        [0., 1., 0.]
                    };
                    v = n.map(|x| x * min);
                }
                for c in 0..3 {
                    v[c] += d.bias[c];
                }
                let mut spin = [0f32; 3];
                for s in spin.iter_mut() {
                    *s = ((self.rand() % 160) as f32 - 80.) * 16.;
                }
                self.tiles[k].velocity = v;
                self.tiles[k].spin = spin;
                k += 1;
            }
        }
        self.launched = true;
    }

    /// FUN_004c8370：每块绕自己的中心翻滚，中心按速度飞。
    fn fly(&mut self, dt: f32) {
        for tile in self.tiles.iter_mut() {
            let mut centre = [0f32; 3];
            for c in &tile.corners {
                for a in 0..3 {
                    centre[a] += c[a] * 0.25;
                }
            }
            for a in 0..3 {
                centre[a] += dt * tile.velocity[a];
            }
            let [yaw, pitch, roll] = tile.spin.map(|s| s * dt);
            let old_centre: [f32; 3] = {
                let mut c = [0f32; 3];
                for p in &tile.corners {
                    for a in 0..3 {
                        c[a] += p[a] * 0.25;
                    }
                }
                c
            };
            for corner in tile.corners.iter_mut() {
                let local = [
                    corner[0] - old_centre[0],
                    corner[1] - old_centre[1],
                    corner[2] - old_centre[2],
                ];
                let r = rotate_ypr(local, yaw, pitch, roll);
                *corner = [r[0] + centre[0], r[1] + centre[1], r[2] + centre[2]];
            }
        }
    }

    pub fn step(&mut self, delta_sec: f32) {
        if self.done {
            return;
        }
        self.t += delta_sec;
        if self.t > self.delay {
            let local = self.t - self.delay;
            let mut d = delta_sec;
            if local > self.duration {
                d -= local - self.duration;
            }
            let d = d.max(0.);
            self.started = true;
            if !self.launched && local >= self.launch_at {
                self.launch();
            }
            for k in 0..3 {
                self.rotation[k] += self.turn[k] * d;
                self.position[k] += self.drift[k] * d;
            }
            let a = self.axis;
            let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
            let k = if len < 1e-6 {
                [0., 1., 0.]
            } else {
                [a[0] / len, a[1] / len, a[2] / len]
            };
            let (s, c) = (local.min(self.duration) * self.orbit_speed)
                .to_radians()
                .sin_cos();
            let r = self.radius;
            let dot = k[0] * r;
            self.orbit = [
                r * c + k[0] * dot * (1. - c),
                k[2] * r * s + k[1] * dot * (1. - c),
                -k[1] * r * s + k[2] * dot * (1. - c),
            ];
            if self.color_rate.iter().any(|v| *v != 0.) {
                for k in 0..4 {
                    self.color[k] = (self.color[k] + d * self.color_rate[k]).clamp(0., 255.);
                }
            }
            self.scale[0] += self.grow[0] * d;
            self.scale[1] += self.grow[1] * d;
            if self.launched {
                self.fly(d);
            }
            if local >= self.duration {
                self.done = true;
            }
        }
        self.frame_clock += self.def.quad.fps * delta_sec;
    }

    /// 局部点到世界：绕 Z → 绕 Y → 绕 X → 缩放 (x, 1, z) → 平移。
    fn to_world(&self, v: [f32; 3]) -> [f32; 3] {
        let [rx, ry, rz] = self.rotation.map(|a| a.to_radians());
        let (sz, cz) = rz.sin_cos();
        let v = [v[0] * cz - v[1] * sz, v[0] * sz + v[1] * cz, v[2]];
        let (sy, cy) = ry.sin_cos();
        let v = [v[0] * cy + v[2] * sy, v[1], -v[0] * sy + v[2] * cy];
        let (sx, cx) = rx.sin_cos();
        let v = [v[0], v[1] * cx - v[2] * sx, v[1] * sx + v[2] * cx];
        [
            v[0] * self.scale[0] + self.position[0] + self.orbit[0],
            v[1] + self.position[1] + self.orbit[1],
            v[2] * self.scale[1] + self.position[2] + self.orbit[2],
        ]
    }

    /// 这一帧的碎片：`(四角的世界坐标, 四角 uv)`。没开始或已结束为空。
    pub fn quads(&self) -> Vec<[([f32; 3], (f32, f32)); 4]> {
        if !self.started || self.done {
            return vec![];
        }
        let d = &self.def;
        let frames = d.quad.frame_count.max(1);
        let frame = (self.frame_clock.round_ties_even().abs() as u32) % frames;
        let (u0, v_top, u1, v_bottom) = d.quad.frame_uv(frame);
        let du = (u1 - u0) / d.cols.max(1) as f32;
        let dv = (v_top - v_bottom) / d.rows.max(1) as f32;
        self.tiles
            .iter()
            .enumerate()
            .map(|(k, tile)| {
                let (i, j) = (k % d.cols.max(1), k / d.cols.max(1));
                let (a0, b0) = (u0 + i as f32 * du, v_bottom + j as f32 * dv);
                let (a1, b1) = (a0 + du, b0 + dv);
                let uvs = [(a0, b0), (a1, b0), (a1, b1), (a0, b1)];
                std::array::from_fn(|c| (self.to_world(tile.corners[c]), uvs[c]))
            })
            .collect()
    }

    pub fn rgba(&self) -> [f32; 4] {
        let [a, r, g, b] = self.color;
        [r / 255., g / 255., b / 255., a / 255.]
    }
}

/// 把这一帧的碎片写进网格（实体用 [`super::flexeff_meteor::build_meteor_entity`] 建）。
pub fn write_explosive(entity: &ComRc<IEntity>, effect: &ExplosiveEffect) {
    let color = pack_color(effect.rgba());
    let quads: Vec<QuadVerts> = effect
        .quads()
        .iter()
        .map(|q| {
            std::array::from_fn(|k| {
                let (p, uv) = q[k];
                (Vec3::new(p[0], p[1], p[2]), uv, color)
            })
        })
        .collect();
    write_quads(entity, effect.capacity(), &quads);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_def(cols: usize, rows: usize) -> ExplosiveDef {
        ExplosiveDef {
            quad: QuadDef {
                cols: 1,
                rows: 1,
                frame_count: 1,
                fps: 1.,
                offset: [0., 0., 0.],
                size: 1.,
                color: 0xff80_8080,
                max_quads: 1,
                additive: true,
                billboard: false,
                no_ztest: false,
                yaw: 0.,
                pitch: 0.,
                roll: 0.,
            },
            cols,
            rows,
            cell: 10.,
            base: 512.,
            min_speed: 20.,
            bias: [0., 0., 10.],
            falloff: 1.,
            rotation: [0., 0., 0.],
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
    fn the_tiles_start_as_a_flat_picture() {
        let d = a_def(2, 1);
        assert_eq!(
            d.layout(0),
            [[-10., 0., 5.], [0., 0., 5.], [0., 0., -5.], [-10., 0., -5.]]
        );
        assert_eq!(d.layout(1)[1], [10., 0., 5.]);
    }

    #[test]
    fn launched_tiles_fly_at_least_the_minimum_speed() {
        let mut e = ExplosiveEffect::new(a_def(4, 4), &sub_with(&[(76, 2.)]), [0.; 3], 7);
        e.step(0.001);
        assert!(e.launched);
        for t in &e.tiles {
            let v = [t.velocity[0], t.velocity[1], t.velocity[2] - 10.];
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            assert!(len >= 20. - 1e-3, "去掉偏置后不小于最小速度：{len}");
            assert!(t.spin.iter().all(|s| s.abs() <= 1280.));
        }
    }

    #[test]
    fn a_tile_flies_by_its_velocity() {
        let mut e = ExplosiveEffect::new(a_def(1, 1), &sub_with(&[(76, 2.)]), [0.; 3], 3);
        e.step(0.001);
        let v = e.tiles[0].velocity;
        let before: Vec<[f32; 3]> = e.quads()[0].iter().map(|c| c.0).collect();
        let centre = |q: &[[f32; 3]]| {
            let mut c = [0f32; 3];
            for p in q {
                for a in 0..3 {
                    c[a] += p[a] * 0.25;
                }
            }
            c
        };
        let c0 = centre(&before);
        e.step(0.1);
        let after: Vec<[f32; 3]> = e.quads()[0].iter().map(|c| c.0).collect();
        let c1 = centre(&after);
        for a in 0..3 {
            assert!((c1[a] - c0[a] - v[a] * 0.1).abs() < 1e-2, "轴 {a}");
        }
    }

    #[test]
    fn it_shows_only_inside_its_window_and_fades() {
        let mut e = ExplosiveEffect::new(
            a_def(2, 2),
            &sub_with(&[(72, 0.5), (76, 1.), (56, -100.)]),
            [0.; 3],
            1,
        );
        e.step(0.4);
        assert!(e.quads().is_empty());
        e.step(0.6);
        assert_eq!(e.quads().len(), 4);
        // 越过延迟的那一帧用整帧 dt（0.6 秒），原版只在收尾那帧截掉超出的部分：255 − 60。
        assert!((e.rgba()[3] * 255. - 195.).abs() < 1e-2);
        e.step(1.0);
        assert!(e.finished() && e.quads().is_empty());
    }
}
