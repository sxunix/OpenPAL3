//! flexeff `grid` 类：一张会扭动、会起伏的方格面（地面的漩涡、魂影之类）。
//! 实机 15 条定义，贴图各不相同（`hhxm.tga`、`tougu.tga`、`kuloutou.tga` ……）。
//!
//! 逆向记在 `switch/re/notes/combat_vfx.md` §10，要点（vtable 0x56f920）：
//!
//! * `+4` 0x4c98f0 读定义（0x4c99af 起）：
//!   - 列数 `(int)+0xe0`、行数 `(int)+0xe8`，面片数 = 列 × 行；格宽 `+0xe4`；
//!   - 扭动幅度 `+0xf4`、扭动频率 `+0xf8`；起伏幅度 `+0x10c`、起伏速度 `+0xfc`；
//!   - 图集、帧率、帧数、偏移、大小、颜色、加法、不测深度和 quad 类读同一批字段（[`QuadDef`]）；
//!   - 转角 `+0xd8 / +0xc4 / +0xdc`（绕 X / Y / Z）。
//! * FUN_004cac70 摆出 (列+1) × (行+1) 个顶点：x 从 −列·宽/2 起每格 +宽，z 从 +行·宽/2 起每格
//!   −宽，y = 0——一张躺在 XZ 平面上的方格。
//! * `+8` 0x4c9750 摆位：位置 = 播放点 + 偏移，转角取定义，绕圈偏移清零，颜色拆成 A/R/G/B，
//!   缩放 x、z 都取 `+0xc8`，不可见。
//! * 时间线（`+0x30` 0x4ca1d0 排、0x4ca450 走）：延迟子块 `+72`，持续 `+76`。这一步里每帧：
//!   - 第一次进来置可见；
//!   - 转角 += `+40/+44/+48`·dt，位置 += `+4..+12`·dt，绕圈（`+20..+36`，同 particle 类）；
//!   - A/R/G/B 各 += `+56/+60/+64/+68`·dt（任一非零才做，截 0..255）；
//!   - 缩放 x += `+0`·dt、z += `+84`·dt；
//!   - 在顶点副本上先扭（FUN_004cb000）再起伏（FUN_004cb0c0），按格子写成面片（FUN_004cad10），
//!     再把顶点换回原样——变形不累积；
//!   - 到时长或被强停结束。
//! * 扭动：顶点 (i 列, j 行)，s = sin((i+1+t)·频率 + j+1)，c = cos((j+1+t)·频率 + i+1)，
//!   x += (s − c)·幅度，z += (s + c)·幅度。起伏：y += sin(i + 速度·t)·幅度。
//! * 每帧（`+0x14` 0x4c9ed0）：帧时钟 += 帧率·dt，帧号 = |round(帧时钟)| % 帧数；把这一帧的
//!   uv 方框按列、行切成小格分给各面片，颜色取打包后的 ARGB。
//! * 画（`+0x18` 0x4c9ca0）：矩阵栈 平移(位置 + 绕圈) → 缩放(x, 1, z) → 绕 X → 绕 Y → 绕 Z，
//!   面片一次画掉；渲染效果按加法 / 不测深度四选一。
//!
//! gbengine 的几条底层约定都在 `engine/math/gbmath.cpp` 里查实了，见
//! [`super::flexeff_explosive`] 开头：角度按度、旋转右手系、矩阵栈
//! 「后压的先作用」。 这一类的实际作用顺序是
//! Z → Y → X → 缩放(x, 1, z) → 平移。
//!
//! 变形公式里的 t，原版（`CEffGrid::DoWave` / `DoFlap`，`EffGrid.cpp:636`、`:661`）
//! 用的是 `m_fCurTime`——任务基类从 `CEffMgr::Update(fCurTime)` 收到的那个绝对时钟
//! （战斗里是 `cbsystem.cpp:345` 的 `m_fTimeFund`，场景里是 `pal3.cpp:649` 的
//! `g_fEftTime`）。这里用实例自己的时钟，正弦里只差一个常数相位，是有意换的。
//!
//! `Render` 带 `m_bIdle || !m_bVisible` 的判断（`EffGrid.cpp:152`）。

use crosscom::ComRc;
use fileformats::pal3::flexeff::EffectDef;
use radiance::comdef::IEntity;
use radiance::math::Vec3;

use super::flexeff_meteor::{QuadVerts, write_quads};
use super::flexeff_quad::QuadDef;
use super::particles::pack_color;

/// 面片数上限，只是给顶点缓冲定个容量。实机最大 30 × 30 = 900。
pub const MAX_GRID_QUADS: usize = 4096;

/// 定义记录里 `grid` 用得上的量。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridDef {
    pub quad: QuadDef,
    /// `(int)+0xe0` / `(int)+0xe8`。
    pub cols: usize,
    pub rows: usize,
    /// `+0xe4`：格宽。
    pub cell: f32,
    /// `+0xf4` / `+0xf8`：平面内扭动的幅度和频率。
    pub wobble: [f32; 2],
    /// `+0x10c` / `+0xfc`：上下起伏的幅度和速度。
    pub wave: [f32; 2],
    /// `+0xd8 / +0xc4 / +0xdc`：绕 X / Y / Z（度）。
    pub rotation: [f32; 3],
}

impl GridDef {
    pub fn from_def(def: &EffectDef<'_>) -> Self {
        let b = def.raw();
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let count = |v: f32| (v as i32).max(0) as usize;
        Self {
            quad: QuadDef::from_def(def),
            cols: count(f(0xe0)),
            rows: count(f(0xe8)),
            cell: f(0xe4),
            wobble: [f(0xf4), f(0xf8)],
            wave: [f(0x10c), f(0xfc)],
            rotation: [f(0xd8), f(0xc4), f(0xdc)],
        }
    }

    pub fn quads(&self) -> usize {
        (self.cols * self.rows).min(MAX_GRID_QUADS)
    }
}

/// 一个正在播的 `grid` 实例。
#[derive(Debug, Clone)]
pub struct GridEffect {
    pub def: GridDef,
    delay: f32,
    duration: f32,
    velocity: [f32; 3],
    spin: [f32; 3],
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
    done: bool,
}

impl GridEffect {
    pub fn new(def: GridDef, sub: &[u8], origin: [f32; 3]) -> Self {
        let f = |o: usize| f32::from_le_bytes(sub[o..o + 4].try_into().unwrap());
        let c = def.quad.color;
        let o = def.quad.offset;
        Self {
            def,
            delay: f(72).max(0.),
            duration: f(76).max(0.),
            velocity: [f(4), f(8), f(12)],
            spin: [f(40), f(44), f(48)],
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
            done: false,
        }
    }

    pub fn finished(&self) -> bool {
        self.done
    }

    pub fn capacity(&self) -> usize {
        self.def.quads().max(1)
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
            for k in 0..3 {
                self.rotation[k] += self.spin[k] * d;
                self.position[k] += self.velocity[k] * d;
            }
            // 绕圈：Rodrigues 转 (半径, 0, 0)，零转轴当 +y。
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
            if local >= self.duration {
                self.done = true;
            }
        }
        self.frame_clock += self.def.quad.fps * delta_sec;
    }

    /// 顶点 (i 列, j 行) 变形后、变换到世界之前的位置。
    fn vertex(&self, i: usize, j: usize) -> [f32; 3] {
        let d = &self.def;
        let (fi, fj, t) = (i as f32, j as f32, self.t);
        let mut v = [
            -(d.cols as f32) * d.cell * 0.5 + fi * d.cell,
            0.,
            d.rows as f32 * d.cell * 0.5 - fj * d.cell,
        ];
        let [amp, freq] = d.wobble;
        let s = ((fi + 1. + t) * freq + fj + 1.).sin();
        let c = ((fj + 1. + t) * freq + fi + 1.).cos();
        v[0] += (s - c) * amp;
        v[2] += (s + c) * amp;
        let [wave_amp, wave_speed] = d.wave;
        v[1] += (fi + wave_speed * t).sin() * wave_amp;
        v
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

    /// 这一帧的面片：`(四角的世界坐标, 四角 uv)`。没开始或已结束为空。
    pub fn quads(&self) -> Vec<[([f32; 3], (f32, f32)); 4]> {
        if !self.started || self.done {
            return vec![];
        }
        let d = &self.def;
        let frames = d.quad.frame_count.max(1);
        let frame = (self.frame_clock.round_ties_even().abs() as u32) % frames;
        // quad 的 frame_uv 给 (u0, v 上沿, u1, v 下沿)；原版从 v 下沿起按行往上切。
        let (u0, v_top, u1, v_bottom) = d.quad.frame_uv(frame);
        let du = (u1 - u0) / d.cols.max(1) as f32;
        let dv = (v_top - v_bottom) / d.rows.max(1) as f32;
        let world: Vec<Vec<[f32; 3]>> = (0..=d.rows)
            .map(|j| {
                (0..=d.cols)
                    .map(|i| self.to_world(self.vertex(i, j)))
                    .collect()
            })
            .collect();
        let mut out = Vec::with_capacity(d.quads());
        for j in 0..d.rows {
            for i in 0..d.cols {
                if out.len() >= MAX_GRID_QUADS {
                    return out;
                }
                let (a0, b0) = (u0 + i as f32 * du, v_bottom + j as f32 * dv);
                let (a1, b1) = (a0 + du, b0 + dv);
                out.push([
                    (world[j][i], (a0, b0)),
                    (world[j][i + 1], (a1, b0)),
                    (world[j + 1][i + 1], (a1, b1)),
                    (world[j + 1][i], (a0, b1)),
                ]);
            }
        }
        out
    }

    pub fn rgba(&self) -> [f32; 4] {
        let [a, r, g, b] = self.color;
        [r / 255., g / 255., b / 255., a / 255.]
    }
}

/// 把这一帧的方格写进网格（实体用 [`super::flexeff_meteor::build_meteor_entity`] 建）。
pub fn write_grid(entity: &ComRc<IEntity>, effect: &GridEffect) {
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

    fn a_def(cols: usize, rows: usize) -> GridDef {
        GridDef {
            quad: QuadDef {
                cols: 1,
                rows: 1,
                frame_count: 1,
                fps: 0.,
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
            wobble: [0., 0.],
            wave: [0., 0.],
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
    fn the_grid_lies_flat_and_centred() {
        let mut e = GridEffect::new(a_def(2, 1), &sub_with(&[(76, 1.)]), [0.; 3]);
        e.step(0.1);
        let q = e.quads();
        assert_eq!(q.len(), 2);
        let corners: Vec<[f32; 3]> = q[0].iter().map(|c| c.0).collect();
        assert_eq!(
            corners,
            vec![[-10., 0., 5.], [0., 0., 5.], [0., 0., -5.], [-10., 0., -5.]]
        );
        // 1×1 图集：两格各占一半 u，v 全高。
        assert_eq!(q[0][0].1, (0., 0.));
        assert_eq!(q[1][2].1, (1., 1.));
    }

    #[test]
    fn it_shows_only_inside_its_window() {
        let mut e = GridEffect::new(a_def(1, 1), &sub_with(&[(72, 0.5), (76, 1.)]), [0.; 3]);
        e.step(0.4);
        assert!(e.quads().is_empty());
        e.step(0.2);
        assert_eq!(e.quads().len(), 1);
        e.step(1.0);
        assert!(e.finished() && e.quads().is_empty());
    }

    #[test]
    fn the_wave_lifts_each_column_by_its_own_phase() {
        let mut d = a_def(2, 1);
        d.wave = [3., 0.];
        let mut e = GridEffect::new(d, &sub_with(&[(76, 1.)]), [0.; 3]);
        e.step(0.1);
        // 起伏速度 0：第 i 列 y = sin(i)·3。
        let q = e.quads();
        assert!((q[0][0].0[1] - 0.).abs() < 1e-5);
        assert!((q[0][1].0[1] - 1f32.sin() * 3.).abs() < 1e-5);
    }

    #[test]
    fn the_alpha_rate_fades_it() {
        let mut e = GridEffect::new(a_def(1, 1), &sub_with(&[(76, 2.), (56, -100.)]), [0.; 3]);
        e.step(1.0);
        assert!((e.rgba()[3] * 255. - 155.).abs() < 1e-3);
    }
}
