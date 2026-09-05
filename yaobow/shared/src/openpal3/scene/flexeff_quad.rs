//! `quad` 特效类的运行模型 —— 143 播的战斗特效里占大头的那个。
//!
//! 逆向写在 `switch/re/notes/flexeff_format.md`。这里只做**模型**：一个
//! `quad` 实例每帧生成哪些四边形、每个四边形此刻的位置/大小/转角/颜色/
//! 图集帧是多少。怎么把它画出来是调用方的事。
//!
//! # 原版是怎么跑的
//!
//! 建实例时（`FUN_004d2080`）把 124 字节子块整块拷进实例的 `+0x18`，再挂两
//! 条时间线（`FUN_004d3010`）：
//!
//! | 回调 | 延迟 | 时长 |
//! |---|---|---|
//! | 生成器 0x4d2e10 | 0 | 子块 `+76` |
//! | 补间器 0x4d22f0 | 子块 `+72` | 子块 `+76` |
//!
//! **生成器**进场先生成一个四边形，之后若子块 `+89` 非零，就每隔子块 `+84`
//! 秒再生成一个，直到时长走完。模板是特效对象的 `+0x4c..+0x9c` 那 80 字节
//! 默认块，初值全部来自定义记录（`FUN_004d1090` / 0x4d1000）。
//!
//! **补间器**把本帧的 `dt` 裁进 `[延迟, 延迟+时长]`，然后一串
//! `字段 += dt × 子块速率`——见 [`Rates`]。
//!
//! # 一个四边形长什么样
//!
//! 渲染分两支（`FUN_004d1300` 按定义的 `IsBillboard` 选）：
//!
//! * **公告板**：在视图空间里摆，四角是 `(±w, ±h)`，再按 `记录+0x30` 那个
//!   角度（度）在平面内转一下。
//! * **非公告板**：在世界空间里摆，局部四顶点是
//!   `(−w, 0, +h) (+w, 0, +h) (+w, 0, −h) (−w, 0, −h)`——**y 一律不写，所以
//!   默认是躺在 XZ 平面上的**，然后依次绕 X（`记录+0x34`）、Y
//!   （`记录+0x30`）、Z（`记录+0x38`）转，最后平移到位。
//!
//! 两支的 `w`/`h` 都是**半长**。

use fileformats::pal3::flexeff::EffectDef;

/// 子块里那组每秒变化率，`FUN_004d22f0` 逐个 `+= dt × 速率` 累到四边形上。
/// 括号里是它落到每四边形 80 字节记录的哪个偏移。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rates {
    /// 子块 `+0` → 记录 `+0x1c`：宽。
    pub width: f32,
    /// 子块 `+92` → 记录 `+0x20`：高。实机数据里它和 [`Self::width`] 恒等，
    /// 也就是等比缩放。
    pub height: f32,
    /// 子块 `+16` 和 `+44` 都累到记录 `+0x30`（绕 Y）。后者只在非零时累。
    pub yaw_a: f32,
    pub yaw_b: f32,
    /// 子块 `+40` → 记录 `+0x34`：绕 X。
    pub pitch: f32,
    /// 子块 `+48` → 记录 `+0x38`：绕 Z。
    pub roll: f32,
    /// 子块 `+56`/`+60`/`+64`/`+68` → 记录 `+0x3c`..`+0x48`：A/R/G/B，
    /// 单位是「每秒多少个 0..255 的色阶」。
    pub alpha: f32,
    pub red: f32,
    pub green: f32,
    pub blue: f32,
}

impl Rates {
    /// 从 124 字节子块取出来。
    pub fn from_sub(sub: &[u8]) -> Self {
        let f = |o: usize| f32::from_le_bytes(sub[o..o + 4].try_into().unwrap());
        Self {
            width: f(0),
            height: f(92),
            yaw_a: f(16),
            yaw_b: f(44),
            pitch: f(40),
            roll: f(48),
            alpha: f(56),
            red: f(60),
            green: f(64),
            blue: f(68),
        }
    }
}

/// 子块里控制节奏的那几个字段。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Timing {
    /// 子块 `+72`：补间从第几秒开始。
    pub delay: f32,
    /// 子块 `+76`：两条时间线共用的时长。
    pub duration: f32,
    /// 子块 `+89`（`+0x58` 起四个标志字节的第 2 个）：生成器是否反复生成。
    pub repeat: bool,
    /// 子块 `+84`：反复生成的间隔（秒）。
    pub interval: f32,
    /// 子块 `+80`：一个四边形把图集播几圈就消失。
    ///
    /// 建实例时 `FUN_004d2090` 把它写进特效对象的 `+0x38`（那里的初值是
    /// `0xffffff`），每帧更新 0x4d17d0 在图集播完一圈后 `记录+7` 加一，
    /// `记录+7 >= 对象+0x38` 就调 `FUN_004d1e50` 把这个四边形释放掉。
    /// 实测取值是 0/1/2/3/4/10/20/100/200/500/1000/10000，正是次数。
    pub loops: u32,
}

impl Timing {
    pub fn from_sub(sub: &[u8]) -> Self {
        let f = |o: usize| f32::from_le_bytes(sub[o..o + 4].try_into().unwrap());
        Self {
            delay: f(72),
            duration: f(76),
            repeat: sub[89] != 0,
            interval: f(84),
            loops: u32::from_le_bytes(sub[80..84].try_into().unwrap()),
        }
    }
}

/// 一个四边形当下的样子——对应原版每四边形那 80 字节记录里用得上的部分。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    /// 记录 `+0x10..0x18` 加 `+0x24..0x2c`。后者初值是 0，这里合并成一个。
    pub position: [f32; 3],
    /// 记录 `+0x1c` / `+0x20`，**半长**。
    pub half_width: f32,
    pub half_height: f32,
    /// 记录 `+0x34` / `+0x30` / `+0x38`，单位是度。公告板只用 `yaw`
    /// （在平面内转），非公告板依次绕 X / Y / Z。
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
    /// 记录 `+0x3c`..`+0x48`，0..255。
    pub color: [f32; 4],
    /// 这个四边形活了多久，用来算图集帧。
    pub age: f32,
    /// 图集已经播完几圈（记录 `+7`）。
    pub loops_done: u32,
}

impl Quad {
    /// 图集第几帧：`age × fps`，按总帧数取模。总帧数为 0 时固定第 0 帧。
    pub fn frame(&self, fps: f32, frame_count: u32) -> u32 {
        if frame_count <= 1 || fps <= 0. {
            return 0;
        }
        ((self.age * fps) as u32) % frame_count
    }

    /// 颜色钳进 0..1。原版把 A/R/G/B 当 0..255 的浮点累加，不做钳位；这里
    /// 只在取用时钳，累加值本身保持原样。
    pub fn rgba(&self) -> [f32; 4] {
        [
            (self.color[1] / 255.).clamp(0., 1.),
            (self.color[2] / 255.).clamp(0., 1.),
            (self.color[3] / 255.).clamp(0., 1.),
            (self.color[0] / 255.).clamp(0., 1.),
        ]
    }
}

/// 定义记录里画一个 `quad` 用得上的静态量。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuadDef {
    /// 图集的列数与行数（定义 `+168` / `+172`）和总帧数（`+176`，取字节）。
    pub cols: u32,
    pub rows: u32,
    pub frame_count: u32,
    /// 定义 `+180`。
    pub fps: f32,
    /// 定义 `+184..192`，加在特效位置上。
    pub offset: [f32; 3],
    /// 定义 `+200`，宽和高都取它。
    pub size: f32,
    /// 定义 `+204`，ARGB。
    pub color: u32,
    /// 定义 `+208`，同时也是每四边形记录数组的长度。
    pub max_quads: u32,
    /// 定义 `+213` / `+214` / `+215`。
    pub additive: bool,
    pub billboard: bool,
    pub no_ztest: bool,
    /// 定义 `+196` / `+216` / `+220`，初始的绕 Y / X / Z 转角。
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
}

impl QuadDef {
    pub fn from_def(def: &EffectDef<'_>) -> Self {
        let b = def.raw();
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let u = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        Self {
            cols: u(168),
            rows: u(172),
            // 原版取的是字节：`mov dl, byte [edi+0xb0]`。
            frame_count: b[176] as u32,
            fps: f(180),
            offset: [f(184), f(188), f(192)],
            size: f(200),
            color: u(204),
            max_quads: u(208),
            additive: b[213] != 0,
            billboard: b[214] != 0,
            no_ztest: b[215] != 0,
            yaw: f(196),
            pitch: f(216),
            roll: f(220),
        }
    }

    /// 图集第 `frame` 帧的 uv 方框 `(u0, v0, u1, v1)`。
    ///
    /// `FUN_004d1fd0` 建的表：外层走行（`rows`）、内层走列（`cols`），行跨度
    /// 是 `cols × 16`，所以**帧序是行优先**；v 是翻的（`1 − 行/行数`）。
    pub fn frame_uv(&self, frame: u32) -> (f32, f32, f32, f32) {
        let cols = self.cols.max(1);
        let rows = self.rows.max(1);
        let col = frame % cols;
        let row = (frame / cols) % rows;
        let (cw, rh) = (1. / cols as f32, 1. / rows as f32);
        let u0 = col as f32 * cw;
        let v0 = 1. - row as f32 * rh;
        (u0, v0, u0 + cw, v0 - rh)
    }

    /// 建一个四边形时的初值——`FUN_004d1090` 那一段。
    fn seed(&self, origin: [f32; 3]) -> Quad {
        let c = self.color;
        Quad {
            position: [
                origin[0] + self.offset[0],
                origin[1] + self.offset[1],
                origin[2] + self.offset[2],
            ],
            half_width: self.size,
            half_height: self.size,
            pitch: self.pitch,
            yaw: self.yaw,
            roll: self.roll,
            color: [
                ((c >> 24) & 0xff) as f32,
                ((c >> 16) & 0xff) as f32,
                ((c >> 8) & 0xff) as f32,
                (c & 0xff) as f32,
            ],
            age: 0.,
            loops_done: 0,
        }
    }
}

/// 一个正在播的 `quad` 特效实例。
#[derive(Debug, Clone)]
pub struct QuadEffect {
    pub def: QuadDef,
    rates: Rates,
    timing: Timing,
    origin: [f32; 3],
    quads: Vec<Quad>,
    elapsed: f32,
    /// 上一次生成是什么时候，`None` 表示还没生成过。
    last_spawn: Option<f32>,
}

impl QuadEffect {
    pub fn new(def: QuadDef, sub: &[u8], origin: [f32; 3]) -> Self {
        Self {
            def,
            rates: Rates::from_sub(sub),
            timing: Timing::from_sub(sub),
            origin,
            quads: vec![],
            elapsed: 0.,
            last_spawn: None,
        }
    }

    pub fn quads(&self) -> &[Quad] {
        &self.quads
    }

    pub fn timing(&self) -> Timing {
        self.timing
    }

    /// 时长走完、并且一个四边形都不剩，就可以扔掉了。原版是靠时间线记录的
    /// 占用位收的，这里用同样的判据。
    pub fn finished(&self) -> bool {
        self.elapsed > self.timing.delay + self.timing.duration && self.quads.is_empty()
    }

    pub fn step(&mut self, delta_sec: f32) {
        self.spawn(delta_sec);
        self.tween(delta_sec);
        self.elapsed += delta_sec;
        self.age_quads(delta_sec);
    }

    /// 每帧更新 0x4d17d0 的图集推进和寿命判定：
    ///
    /// ```text
    /// t = 记录.fps(+8) * dt + 记录.帧累加(+0xc)
    /// if ((int)t >= 对象.FrameCount(+0x36)) {
    ///     记录.圈数(+7)++;
    ///     if (记录.圈数 >= 对象.+0x38)  释放这个四边形;   // +0x38 = 子块 +80
    ///     else                          记录.帧累加 = t − FrameCount;
    /// } else 记录.帧累加 = t;
    /// ```
    ///
    /// 单帧特效（`FrameCount <= 1` 或 `fps == 0`）永远绕不回去，所以自己不会
    /// 消失；它们跟着整个实例走——两条时间线都到期之后没有东西再更新它们，
    /// 这里就一并收掉 `[推断]`（实例本身由特效管理器回收，那一段没跟）。
    fn age_quads(&mut self, delta_sec: f32) {
        let frames = self.def.frame_count;
        let fps = self.def.fps;
        let limit = self.timing.loops.max(1);
        let animated = frames > 1 && fps > 0.;
        for q in &mut self.quads {
            q.age += delta_sec;
            if animated {
                let played = (q.age * fps) as u32 / frames;
                if played > q.loops_done {
                    q.loops_done = played;
                }
            }
        }
        if animated {
            self.quads.retain(|q| q.loops_done < limit);
        }
        if self.elapsed > self.timing.delay + self.timing.duration {
            self.quads.clear();
        }
    }

    /// 生成器 0x4d2e10。
    fn spawn(&mut self, _delta_sec: f32) {
        if self.elapsed > self.timing.duration {
            return;
        }
        let cap = self.def.max_quads.max(1) as usize;
        let due = match self.last_spawn {
            None => true,
            Some(t) => self.timing.repeat && self.elapsed - t >= self.timing.interval,
        };
        if !due || self.quads.len() >= cap {
            return;
        }
        self.quads.push(self.def.seed(self.origin));
        self.last_spawn = Some(self.elapsed);
    }

    /// 补间器 0x4d22f0：本帧的 `dt` 先裁进 `[延迟, 延迟+时长]`，再按速率累加。
    fn tween(&mut self, delta_sec: f32) {
        let start = self.timing.delay;
        let end = start + self.timing.duration;
        let from = self.elapsed.max(start);
        let to = (self.elapsed + delta_sec).min(end);
        let dt = to - from;
        if dt <= 0. {
            return;
        }
        let r = self.rates;
        for q in &mut self.quads {
            q.half_width += dt * r.width;
            q.half_height += dt * r.height;
            q.yaw += dt * r.yaw_a;
            if r.yaw_b != 0. {
                q.yaw += dt * r.yaw_b;
            }
            q.pitch += dt * r.pitch;
            q.roll += dt * r.roll;
            q.color[0] += dt * r.alpha;
            q.color[1] += dt * r.red;
            q.color[2] += dt * r.green;
            q.color[3] += dt * r.blue;
        }
    }
}

/// 一个四边形的四个局部顶点，**非公告板**那一支：
/// `(−w, 0, +h) (+w, 0, +h) (+w, 0, −h) (−w, 0, −h)`，躺在 XZ 平面上。
/// 转角还没应用——调用方按 X → Y → Z 的顺序转完再平移。
pub fn local_corners(q: &Quad) -> [[f32; 3]; 4] {
    let (w, h) = (q.half_width, q.half_height);
    [[-w, 0., h], [w, 0., h], [w, 0., -h], [-w, 0., -h]]
}

/// **公告板**那一支的四个视图空间角点：`(±w, ±h)` 再按 `yaw` 度在平面内转。
pub fn billboard_corners(q: &Quad) -> [[f32; 2]; 4] {
    let (w, h) = (q.half_width, q.half_height);
    let a = q.yaw.to_radians();
    let (s, c) = (a.sin(), a.cos());
    let rot = |x: f32, y: f32| [x * c - y * s, x * s + y * c];
    [rot(-w, h), rot(w, h), rot(w, -h), rot(-w, -h)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `flags` 里偏移 80 特殊：那是四字节的循环次数，不是标志字节。
    fn sub_with(pairs: &[(usize, f32)], flags: &[(usize, u8)]) -> Vec<u8> {
        let mut b = vec![0u8; 124];
        for (o, v) in pairs {
            b[*o..*o + 4].copy_from_slice(&v.to_le_bytes());
        }
        for (o, v) in flags {
            if *o == 80 {
                b[80..84].copy_from_slice(&(*v as u32).to_le_bytes());
            } else {
                b[*o] = *v;
            }
        }
        b
    }

    fn a_def() -> QuadDef {
        QuadDef {
            cols: 8,
            rows: 4,
            frame_count: 32,
            fps: 16.,
            offset: [0., 0., 0.],
            size: 10.,
            // A=0, R=0xcd, G=0xcd, B=0xff —— 实机的 languang。
            color: 0x00cd_cdff,
            max_quads: 4,
            additive: true,
            billboard: true,
            no_ztest: true,
            yaw: 0.,
            pitch: 0.,
            roll: 0.,
        }
    }

    #[test]
    fn the_atlas_runs_row_major_with_v_flipped() {
        let d = a_def();
        // 第 0 帧是左上角，v 从 1 往下。
        let (u0, v0, u1, v1) = d.frame_uv(0);
        assert!((u0 - 0.0).abs() < 1e-6 && (u1 - 0.125).abs() < 1e-6);
        assert!((v0 - 1.0).abs() < 1e-6 && (v1 - 0.75).abs() < 1e-6);
        // 第 8 帧换到第二行（行优先，列数 8）。
        let (u0, v0, _, _) = d.frame_uv(8);
        assert!((u0 - 0.0).abs() < 1e-6, "第 8 帧应当回到第一列");
        assert!((v0 - 0.75).abs() < 1e-6, "第 8 帧应当在第二行");
    }

    #[test]
    fn the_colour_unpacks_as_argb() {
        let d = a_def();
        let q = d.seed([0., 0., 0.]);
        assert_eq!(q.color, [0., 205., 205., 255.], "0x00cdcdff 是 A,R,G,B");
        let rgba = q.rgba();
        assert!(rgba[3].abs() < 1e-6, "alpha 从 0 起，靠补间涨上来");
        assert!((rgba[2] - 1.0).abs() < 1e-6, "蓝满");
    }

    #[test]
    fn alpha_ramps_over_the_tween() {
        // 子块 +56 = 200：alpha 每秒涨 200 个色阶，实机常见值。
        let sub = sub_with(&[(76, 2.0), (56, 200.)], &[]);
        let mut e = QuadEffect::new(a_def(), &sub, [0., 0., 0.]);
        e.step(0.5);
        assert!((e.quads()[0].color[0] - 100.).abs() < 1e-3);
        e.step(0.5);
        assert!((e.quads()[0].color[0] - 200.).abs() < 1e-3);
    }

    #[test]
    fn the_tween_waits_for_the_delay_and_stops_at_the_duration() {
        // 延迟 1 秒、时长 1 秒，缩放每秒 +10。
        let sub = sub_with(&[(72, 1.0), (76, 1.0), (0, 10.), (92, 10.)], &[]);
        let mut e = QuadEffect::new(a_def(), &sub, [0., 0., 0.]);
        e.step(1.0); // 全在延迟里，不该动
        assert_eq!(e.quads()[0].half_width, 10.);
        e.step(0.5); // 补间跑了半秒
        assert!((e.quads()[0].half_width - 15.).abs() < 1e-3);
        // 再走一秒：只有 0.5 秒落在时长内，且之后整个特效收掉。
        e.step(1.0);
        assert!(e.quads().is_empty(), "时长走完四边形应当收掉");
        assert!(e.finished());
    }

    #[test]
    fn the_spawner_repeats_only_when_the_flag_says_so() {
        // 不重复：只有一个。
        let once = sub_with(&[(76, 1.0), (84, 0.2)], &[]);
        let mut e = QuadEffect::new(a_def(), &once, [0., 0., 0.]);
        for _ in 0..10 {
            e.step(0.05);
        }
        assert_eq!(e.quads().len(), 1);

        // 重复：每 0.2 秒一个，上限是定义的 MaxQuadCount。
        let many = sub_with(&[(76, 1.0), (84, 0.2)], &[(89, 1)]);
        let mut e = QuadEffect::new(a_def(), &many, [0., 0., 0.]);
        for _ in 0..18 {
            e.step(0.05);
        }
        assert_eq!(
            e.quads().len(),
            4,
            "0.9 秒内该生成 5 个，被 max_quads 卡在 4"
        );
    }

    #[test]
    fn a_flat_quad_lies_in_the_world_xz_plane() {
        // 非公告板那一支：y 一律是 0，w 沿 X、h 沿 Z。
        let d = a_def();
        let q = d.seed([1., 2., 3.]);
        let c = local_corners(&q);
        assert!(c.iter().all(|v| v[1] == 0.), "y 不该被写");
        assert_eq!(c[0], [-10., 0., 10.]);
        assert_eq!(c[2], [10., 0., -10.]);
    }

    #[test]
    fn a_billboard_rotates_in_its_own_plane() {
        let d = a_def();
        let mut q = d.seed([0., 0., 0.]);
        q.yaw = 90.;
        let c = billboard_corners(&q);
        // (−w, h) 转 90° 变成 (−h, −w)。
        assert!((c[0][0] + 10.).abs() < 1e-3 && (c[0][1] + 10.).abs() < 1e-3);
    }

    #[test]
    fn a_quad_dies_after_the_atlas_has_looped_enough_times() {
        // 子块 +80 = 2：图集播两圈就释放。32 帧 16fps 一圈两秒。
        let sub = sub_with(&[(76, 60.0)], &[(80, 2)]);
        let mut e = QuadEffect::new(a_def(), &sub, [0., 0., 0.]);
        for _ in 0..30 {
            e.step(0.1);
        }
        assert_eq!(e.quads().len(), 1, "三秒时还在第二圈");
        for _ in 0..20 {
            e.step(0.1);
        }
        assert!(e.quads().is_empty(), "五秒时两圈已经播完");
    }

    #[test]
    fn a_single_frame_quad_lives_until_the_timelines_end() {
        // 单帧（fps 0）绕不回去，自己不会消失，跟着实例走。
        let mut d = a_def();
        d.frame_count = 1;
        d.fps = 0.;
        let sub = sub_with(&[(76, 1.0)], &[(80, 1)]);
        let mut e = QuadEffect::new(d, &sub, [0., 0., 0.]);
        e.step(0.5);
        assert_eq!(e.quads().len(), 1);
        e.step(0.6);
        assert!(e.quads().is_empty());
        assert!(e.finished());
    }

    #[test]
    fn the_frame_advances_at_the_definitions_fps() {
        let d = a_def();
        let mut q = d.seed([0., 0., 0.]);
        assert_eq!(q.frame(d.fps, d.frame_count), 0);
        q.age = 1.0; // 16 fps
        assert_eq!(q.frame(d.fps, d.frame_count), 16);
        q.age = 2.0; // 32 帧一圈，绕回 0
        assert_eq!(q.frame(d.fps, d.frame_count), 0);
        // 单帧的特效不动。
        assert_eq!(q.frame(0., 1), 0);
    }
}
