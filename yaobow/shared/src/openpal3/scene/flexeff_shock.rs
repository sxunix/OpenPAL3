//! flexeff `shock` 类（`CEffShock`，源文件 `D:\Project\PAL3\combat\EffShock.cpp`）：一道闪电。
//! 「烈焰燃雷」等仙术里的电光靠它出画面；实机 35 条定义全用 `effect\dian.0.tga`。
//!
//! 逆向记在 `switch/re/notes/combat_vfx.md` §8，要点（vtable 0x56fadc）：
//!
//! * `+4` 0x4d3ad0 读定义：
//!   - 行数 `(int)+0xe0`，每行叠 `(int)+0xe4` 张，共 行 × 叠 张面片；
//!   - 面片宽 `+0xe8`、高 `+0xf4`，相邻两行重叠 `+0x10c`，叠序号每多一张寿命多 `+0xf8`；
//!   - 图集、帧率、帧数、偏移、大小、颜色、加法、不测深度和 quad 类读同一批字段（[`QuadDef`]）；
//!   - 转角 `+0xd8 / +0xc4 / +0xdc`，分别绕 X / Y / Z。
//! * FUN_004d48b0 摆几何：
//!   - 第 r 行的上沿 y = 高 + r·(高 − 重叠)，下沿 = 上沿 − 高；
//!   - x ∈ [−宽/2, 宽/2]，z = 0；
//!   - 同一行叠的几张完全重合。
//!
//!   整体是一根从原点往上长的闪电柱。
//! * 时间线（`+0x30` 0x4d4340 排、FUN_004d4570 走）：延迟子块 `+72`，持续 `+76`。
//!   这一步第一次进来调 FUN_004d45d0：
//!   - 每张面片的时钟取 `rand() % (帧数·100) / 100`，也就是随机相位；
//!   - 寿命上限 = 时钟 + 子块 `+80`（整数）+ 叠序号 × `+0xf8`。
//! * 每帧（`+0x14` FUN_004d3dc0）：时钟 += dt × 帧率。时钟 ≤ 帧数 × 寿命上限时画第
//!   `round(时钟) % 帧数` 帧，否则这张隐藏（uv 和颜色清零）。叠着的几张因此先后熄灭，形成闪烁。
//! * 画（`+0x18` 0x4d4110）：矩阵栈依次 平移(播放点 + 偏移) → 绕 X 转 → 绕 Y 转 → 绕 Z 转 →
//!   按大小等比缩放，再把全部面片一次画掉。渲染效果按加法 / 不测深度四选一。
//!
//! [推断] 的地方：
//! - 矩阵栈的乘法顺序按「后压的先作用」理解，即 缩放 → Z → Y → X → 平移；
//! - 转角按度、右手系；
//! - 起播前原版没初始化时钟（`operator_new` 的数组），这里按「不画」处理。

use radiance::comdef::IEntity;
use radiance::math::Vec3;

use crosscom::ComRc;
use fileformats::pal3::flexeff::EffectDef;

use super::flexeff_meteor::{QuadVerts, write_quads};
use super::flexeff_quad::QuadDef;
use super::particles::pack_color;

/// 一个闪电实体最多画多少张面片，只是给顶点缓冲定个容量。实机最大 5 × 8 = 40。
pub const MAX_SHOCK_QUADS: usize = 256;

/// 定义记录里 `shock` 用得上的量。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShockDef {
    pub quad: QuadDef,
    /// `(int)+0xe0` / `(int)+0xe4`（`__ftol`，截断）。
    pub rows: usize,
    pub copies: usize,
    /// `+0xe8` / `+0xf4`：一张面片的宽和高。
    pub width: f32,
    pub height: f32,
    /// `+0xf8`：叠序号每多一，寿命上限多这么多。
    pub stagger: f32,
    /// `+0x10c`：相邻两行重叠多少。
    pub overlap: f32,
    /// `+0xd8 / +0xc4 / +0xdc`：绕 X / Y / Z 的转角（度）。
    pub rotation: [f32; 3],
}

impl ShockDef {
    pub fn from_def(def: &EffectDef<'_>) -> Self {
        let b = def.raw();
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let count = |v: f32| (v as i32).max(0) as usize;
        Self {
            quad: QuadDef::from_def(def),
            rows: count(f(0xe0)),
            copies: count(f(0xe4)),
            width: f(0xe8),
            height: f(0xf4),
            stagger: f(0xf8),
            overlap: f(0x10c),
            rotation: [f(0xd8), f(0xc4), f(0xdc)],
        }
    }

    /// 一共几张面片。
    pub fn segments(&self) -> usize {
        (self.rows * self.copies).min(MAX_SHOCK_QUADS)
    }

    /// FUN_004d48b0：第 `row` 行面片的四个局部角点
    /// `(−w/2, 上) (w/2, 上) (w/2, 下) (−w/2, 下)`，z = 0。
    pub fn local_corners(&self, row: usize) -> [[f32; 3]; 4] {
        let top = self.height + row as f32 * (self.height - self.overlap);
        let bottom = top - self.height;
        let x0 = self.width * -0.5;
        let x1 = x0 + self.width;
        [
            [x0, top, 0.],
            [x1, top, 0.],
            [x1, bottom, 0.],
            [x0, bottom, 0.],
        ]
    }
}

/// 一张要画的面片：世界坐标的四个角、图集第几帧。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShockQuad {
    pub corners: [[f32; 3]; 4],
    pub frame: u32,
}

/// 一个正在播的 `shock` 实例。
#[derive(Debug, Clone)]
pub struct ShockEffect {
    pub def: ShockDef,
    origin: [f32; 3],
    delay: f32,
    duration: f32,
    /// 子块 `+80`，整数：加在每张面片的寿命上限上。
    extra: f32,
    t: f32,
    started: bool,
    done: bool,
    rng: u32,
    /// 每张面片的 `(寿命上限, 时钟)`，对象 `+0x1040` 那张 8 字节一条的表。
    segments: Vec<[f32; 2]>,
}

impl ShockEffect {
    pub fn new(def: ShockDef, sub: &[u8], origin: [f32; 3], seed: u32) -> Self {
        let f = |o: usize| f32::from_le_bytes(sub[o..o + 4].try_into().unwrap());
        let extra = i32::from_le_bytes(sub[80..84].try_into().unwrap()) as f32;
        let o = def.quad.offset;
        Self {
            def,
            origin: [origin[0] + o[0], origin[1] + o[1], origin[2] + o[2]],
            delay: f(72).max(0.),
            duration: f(76).max(0.),
            extra,
            t: 0.,
            started: false,
            done: false,
            rng: seed,
            segments: vec![[0., 0.]; def.segments()],
        }
    }

    pub fn finished(&self) -> bool {
        self.done
    }

    pub fn capacity(&self) -> usize {
        self.def.segments().max(1)
    }

    /// MSVC 的 `rand()`：`s = s·214013 + 2531011`，取 `(s >> 16) & 0x7fff`。
    fn rand(&mut self) -> u32 {
        self.rng = self.rng.wrapping_mul(0x343fd).wrapping_add(0x269ec3);
        (self.rng >> 16) & 0x7fff
    }

    /// FUN_004d45d0：随机相位，寿命上限按叠序号错开。
    fn reset(&mut self) {
        let frames = self.def.quad.frame_count.max(1);
        for i in 0..self.segments.len() {
            let clock = (self.rand() % (frames * 100)) as f32 * 0.01;
            let copy = if self.def.copies > 0 {
                i % self.def.copies
            } else {
                0
            };
            self.segments[i] = [clock + self.extra + copy as f32 * self.def.stagger, clock];
        }
    }

    pub fn step(&mut self, delta_sec: f32) {
        if self.done {
            return;
        }
        self.t += delta_sec;
        if !self.started && self.t >= self.delay {
            self.reset();
            self.started = true;
        }
        if self.started {
            let rate = self.def.quad.fps;
            for s in self.segments.iter_mut() {
                s[1] += delta_sec * rate;
            }
        }
        if self.t >= self.delay + self.duration {
            self.done = true;
        }
    }

    /// 把局部点变到世界：缩放 → 绕 Z → 绕 Y → 绕 X → 平移。
    fn to_world(&self, p: [f32; 3]) -> [f32; 3] {
        let s = self.def.quad.size;
        let [rx, ry, rz] = self.def.rotation.map(|d| d.to_radians());
        let v = [p[0] * s, p[1] * s, p[2] * s];
        let (sz, cz) = rz.sin_cos();
        let v = [v[0] * cz - v[1] * sz, v[0] * sz + v[1] * cz, v[2]];
        let (sy, cy) = ry.sin_cos();
        let v = [v[0] * cy + v[2] * sy, v[1], -v[0] * sy + v[2] * cy];
        let (sx, cx) = rx.sin_cos();
        let v = [v[0], v[1] * cx - v[2] * sx, v[1] * sx + v[2] * cx];
        [
            v[0] + self.origin[0],
            v[1] + self.origin[1],
            v[2] + self.origin[2],
        ]
    }

    /// 这一帧还亮着的面片。
    pub fn quads(&self) -> Vec<ShockQuad> {
        if !self.started || self.done || self.def.copies == 0 {
            return vec![];
        }
        let frames = self.def.quad.frame_count.max(1);
        self.segments
            .iter()
            .enumerate()
            .filter(|(_, [limit, clock])| *clock <= frames as f32 * *limit)
            .map(|(i, [_, clock])| ShockQuad {
                corners: self
                    .def
                    .local_corners(i / self.def.copies)
                    .map(|p| self.to_world(p)),
                frame: (clock.round_ties_even().abs() as u32) % frames,
            })
            .collect()
    }
}

/// 把这一帧的闪电写进网格（实体用 [`super::flexeff_meteor::build_meteor_entity`] 建）。
pub fn write_shock(entity: &ComRc<IEntity>, effect: &ShockEffect) {
    let def = effect.def.quad;
    let c = def.color;
    let color = pack_color([
        ((c >> 16) & 0xff) as f32 / 255.,
        ((c >> 8) & 0xff) as f32 / 255.,
        (c & 0xff) as f32 / 255.,
        ((c >> 24) & 0xff) as f32 / 255.,
    ]);
    let quads: Vec<QuadVerts> = effect
        .quads()
        .iter()
        .map(|q| {
            let (u0, v0, u1, v1) = def.frame_uv(q.frame);
            let uvs = [(u0, v0), (u1, v0), (u1, v1), (u0, v1)];
            std::array::from_fn(|k| {
                let p = q.corners[k];
                (Vec3::new(p[0], p[1], p[2]), uvs[k], color)
            })
        })
        .collect();
    write_quads(entity, effect.capacity(), &quads);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_def(rows: usize, copies: usize) -> ShockDef {
        ShockDef {
            quad: QuadDef {
                cols: 2,
                rows: 3,
                frame_count: 6,
                fps: 24.,
                offset: [0., 0., 0.],
                size: 1.,
                color: 0xff7d_9bff,
                max_quads: 1,
                additive: true,
                billboard: true,
                no_ztest: false,
                yaw: 0.,
                pitch: 0.,
                roll: 0.,
            },
            rows,
            copies,
            width: 2.,
            height: 4.,
            stagger: 100.,
            overlap: 1.,
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
    fn the_rows_stack_upward_with_overlap() {
        let d = a_def(2, 1);
        assert_eq!(
            d.local_corners(0),
            [[-1., 4., 0.], [1., 4., 0.], [1., 0., 0.], [-1., 0., 0.]]
        );
        // 第二行抬高 高 − 重叠 = 3。
        assert_eq!(d.local_corners(1)[0], [-1., 7., 0.]);
        assert_eq!(d.local_corners(1)[3], [-1., 3., 0.]);
    }

    #[test]
    fn nothing_shows_before_the_delay_or_after_the_duration() {
        let mut e = ShockEffect::new(a_def(2, 2), &sub_with(&[(72, 0.5), (76, 1.)]), [0.; 3], 1);
        e.step(0.4);
        assert!(e.quads().is_empty());
        e.step(0.2);
        assert!(!e.quads().is_empty());
        e.step(1.0);
        assert!(e.finished() && e.quads().is_empty());
    }

    #[test]
    fn later_copies_outlive_the_first() {
        // 叠序号 1 的寿命上限多 100：一秒后（时钟 ≈ 24 + 相位）肯定还亮；
        // 叠序号 0 的上限只是 相位，时钟涨过 6×相位 就熄。
        let mut e = ShockEffect::new(a_def(1, 2), &sub_with(&[(76, 5.)]), [0.; 3], 7);
        e.step(1.0);
        let q = e.quads();
        assert_eq!(q.len(), 1, "只剩叠序号 1 那张");
    }

    #[test]
    fn size_scales_and_the_offset_moves() {
        let mut d = a_def(1, 1);
        d.quad.size = 10.;
        d.quad.offset = [5., 0., 0.];
        d.stagger = 0.;
        let mut e = ShockEffect::new(d, &sub_with(&[(76, 5.), (80, 0.)]), [1., 2., 3.], 3);
        // 叠序号 0、extra 0：刚起播那一刻 时钟 ≤ 6 × 时钟 成立。
        e.step(0.0);
        let q = e.quads();
        assert_eq!(q.len(), 1);
        assert_eq!(q[0].corners[0], [-10. + 6., 40. + 2., 3.]);
    }
}
