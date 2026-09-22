//! 主角跳（`FUN_00414a30` 起跳、`FUN_004150c0` 行为 3 的动作 0 每帧推、落地后 `FUN_00420b40(主角, dt, 1)`）。
//!
//! 原版只在迷宫里（`DAT_00d8bbec == 2`）、主角脚下那格登记的是跳跃区（类型 0x2c）或神树传送点（0x26）时能跳
//! （`FUN_0041ee60`），右键点地面给落点（`FUN_0041d9a0`）。落点离主角（x、z）超过 168 就拉到 168 那一点，高度取
//! 连物件的地面高度；再从主角那格到落点那格走一条格子直线（`FUN_004218b0`），不通就落在直线上最后一个能走的格子
//! 中心。飞行：先起跳（等动画消息 `holds`，最多 0.5 秒，不动），再按初速度加重力 490 飞，下落到低于落点高度就放到
//! 落点，等落地动画播完。初速度让主角正好在落到落点高度时到达落点的 x、z：竖直初速 √(2h/490)·490（h = 往上的
//! 高差 + 50），水平速度 = 水平距离 / T，T = √(2(|高差| + 50)/490) + √(100/490)。

use radiance::math::Vec3;

/// 最远跳多远（`FUN_00414a10`：`[0x56a984] × [0x56aa10]` = 12 × 14）。
pub const MAX_DIST: f32 = 12. * 14.;
pub const GRAVITY: f32 = 490.;
/// 起跳高过落点的那一截（`[0x56a998]`）。
pub const LIFT: f32 = 50.;
/// 跳的动作（`FUN_00536c50(4, …)`：动作名表 0x570904 第 4 项）。
pub const ANIM: &str = "c04";
/// 起跳等动画消息 `holds` 的上限。
pub const WINDUP_SECS: f32 = 0.5;
/// [移植] 落地动画一直没播完时最多等这么久（原版只等动画）。
pub const LAND_TIMEOUT_SECS: f32 = 2.;

/// [移植] 蓄力跳：按住跳键这么久就到最远（[`MAX_DIST`]），松手起跳。
pub const CHARGE_SECS: f32 = 0.8;
/// [移植] 点一下就松手时跳多远（两格）。
pub const MIN_DIST: f32 = 24.;

/// [移植] 原版右键点地面选落点，手柄上没有这一下，改成按住跳键蓄力：按住 `held` 秒对应的距离，
/// 从 [`MIN_DIST`] 线性涨到 [`MAX_DIST`]。挑完的点照样走原版那一套（拉回 168、取地面高度、格子直线）。
pub fn charge_distance(held: f32) -> f32 {
    let t = (held / CHARGE_SECS).clamp(0., 1.);
    MIN_DIST + (MAX_DIST - MIN_DIST) * t
}

/// `FUN_00414a30` 开头：落点离 `from`（x、z）超过 [`MAX_DIST`] 就拉到 [`MAX_DIST`] 那一点；y 置 0，调用方再取地面高度。
pub fn clamp_aim(from: &Vec3, aim: &Vec3) -> Vec3 {
    let (dx, dz) = (aim.x - from.x, aim.z - from.z);
    let len = (dx * dx + dz * dz).sqrt();
    if len <= MAX_DIST {
        Vec3::new(aim.x, 0., aim.z)
    } else {
        let k = MAX_DIST / len;
        Vec3::new(from.x + dx * k, 0., from.z + dz * k)
    }
}

/// `FUN_004218b0`：从 `from` 格到 `to` 格走一条格子直线（和 `FUN_004215b0` 一样的整数 Bresenham，终点那格不在循环里）。
/// 路上有一格 `passable` 不成立就不通；记下路上最后一个 `walkable` 的格子（起点算在内）。返回（通不通，最后能走的那格）：
/// 通要路上都 `passable` 且起点、终点都 `walkable`。
pub fn line(
    from: (i32, i32),
    to: (i32, i32),
    passable: impl Fn(i32, i32) -> bool,
    walkable: impl Fn(i32, i32) -> bool,
) -> (bool, (i32, i32)) {
    let mut out = from;
    let (sx, dx) = if to.0 < from.0 {
        (-1, from.0 - to.0)
    } else {
        (1, to.0 - from.0)
    };
    let (sz, dz) = if to.1 < from.1 {
        (-1, from.1 - to.1)
    } else {
        (1, to.1 - from.1)
    };
    let x_major = dz <= dx;
    let (major, minor) = if x_major { (dx, dz) } else { (dz, dx) };
    let mut e = -major;
    let (mut x, mut z) = from;
    for _ in 0..major {
        e += minor * 2;
        if !passable(x, z) {
            return (false, out);
        }
        if walkable(x, z) {
            out = (x, z);
        }
        if e >= 0 {
            if x_major {
                z += sz;
            } else {
                x += sx;
            }
            e -= major * 2;
        }
        if x_major {
            x += sx;
        } else {
            z += sz;
        }
    }
    (walkable(from.0, from.1) && walkable(to.0, to.1), out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// `+0x4d0 = 0`：起跳，不动。
    Windup,
    /// 1：往上。
    Rise,
    /// 2：往下，低于落点高度就落。
    Fall,
    /// 3：落地动画。
    Land,
}

/// 一次跳（角色 +0x4cc = 0 那一套：落点 +0x4d8、速度 +0x510、计时 +0x4e8）。
#[derive(Debug, Clone)]
pub struct Jump {
    pub target: Vec3,
    pos: Vec3,
    vel: Vec3,
    phase: Phase,
    t: f32,
}

impl Jump {
    /// `FUN_00414a30` 后半：从 `from`（主角现在的位置，y 是世界高度）跳到 `target`。
    pub fn new(from: Vec3, target: Vec3) -> Self {
        let dy = target.y - from.y;
        let flight = Self::flight_time(dy);
        let (dx, dz) = (target.x - from.x, target.z - from.z);
        let len = (dx * dx + dz * dz).sqrt();
        let (vx, vz) = if len > 0. {
            (dx / len * len / flight, dz / len * len / flight)
        } else {
            (0., 0.)
        };
        let h = dy.max(0.) + LIFT;
        let vy = (h * 2. / GRAVITY).sqrt() * GRAVITY;
        Self {
            target,
            pos: from,
            vel: Vec3::new(vx, vy, vz),
            phase: Phase::Windup,
            t: 0.,
        }
    }

    /// 起跳后到落到落点高度的秒数：往上到顶 √(2h/490)，再落 |高差| + 50 或 50。
    pub fn flight_time(dy: f32) -> f32 {
        ((dy.abs() + LIFT) * 2. / GRAVITY).sqrt() + (LIFT * 2. / GRAVITY).sqrt()
    }

    /// 水平速度（测试用）。
    pub fn velocity(&self) -> Vec3 {
        self.vel
    }

    /// 现在在哪一段（调用方按段换动作倍率：起跳 4、飞 1、空中定格 0、落地 2）。
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// 推一帧（`FUN_004150c0` case 0）：返回（主角这一帧的位置，跳完没有）。`holds` 是动作放出了 `holds`
    /// 标记（起跳那一下做完了），`anim_done` 是落地动作播完没有。
    pub fn step(&mut self, dt: f32, holds: bool, anim_done: bool) -> (Vec3, bool) {
        let mut done = false;
        match self.phase {
            Phase::Windup => {
                // 原版等动画消息 `holds`，最多等 0.5 秒。
                if holds || self.t > WINDUP_SECS {
                    self.phase = Phase::Rise;
                }
            }
            Phase::Rise | Phase::Fall => {
                self.pos = Vec3::new(
                    self.pos.x + self.vel.x * dt,
                    self.pos.y + self.vel.y * dt,
                    self.pos.z + self.vel.z * dt,
                );
                self.vel.y -= GRAVITY * dt;
                if self.phase == Phase::Rise {
                    if self.vel.y < 0. {
                        self.phase = Phase::Fall;
                    }
                } else if self.pos.y < self.target.y {
                    self.pos = self.target;
                    self.phase = Phase::Land;
                    self.t = 0.;
                }
            }
            Phase::Land => {
                if anim_done || self.t > LAND_TIMEOUT_SECS {
                    self.pos = self.target;
                    done = true;
                }
            }
        }
        self.t += dt;
        (self.pos, done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aim_is_clamped_to_168() {
        let a = clamp_aim(
            &Vec3::new(10., 99., 20.),
            &Vec3::new(10. + 300., 5., 20. + 400.),
        );
        assert!((a.x - (10. + 168. * 0.6)).abs() < 1e-3);
        assert!((a.z - (20. + 168. * 0.8)).abs() < 1e-3);
        assert_eq!(a.y, 0.);
        let b = clamp_aim(&Vec3::new(0., 0., 0.), &Vec3::new(100., 7., 0.));
        assert_eq!((b.x, b.y, b.z), (100., 0., 0.));
    }

    #[test]
    fn charge_maps_hold_time_to_distance() {
        assert_eq!(charge_distance(0.), MIN_DIST);
        assert_eq!(charge_distance(CHARGE_SECS), MAX_DIST);
        assert_eq!(charge_distance(5.), MAX_DIST);
        let half = charge_distance(CHARGE_SECS * 0.5);
        assert!((half - (MIN_DIST + MAX_DIST) * 0.5).abs() < 1e-3, "{half}");
    }

    #[test]
    fn line_stops_at_the_last_walkable_tile() {
        // 0..=3 能走，4 是能跳过去的缺口，5 以后是墙。
        let passable = |x: i32, _: i32| x <= 4;
        let walkable = |x: i32, _: i32| x <= 3 || x == 6;
        assert_eq!(line((0, 0), (6, 0), passable, walkable), (false, (3, 0)));
        // 缺口也能跳过去。
        let passable = |_: i32, _: i32| true;
        let walkable = |x: i32, _: i32| x <= 3 || x >= 6;
        assert_eq!(line((0, 0), (8, 0), passable, walkable), (true, (7, 0)));
        // 终点那格走不了：不通，最后能走的是路上那格。
        let walkable = |x: i32, _: i32| x <= 3;
        assert_eq!(line((0, 0), (8, 0), passable, walkable), (false, (3, 0)));
    }

    fn fly(from: Vec3, to: Vec3, dt: f32) -> (Vec3, f32) {
        let mut j = Jump::new(from, to);
        let mut t = 0.;
        loop {
            let (p, done) = j.step(dt, false, true);
            t += dt;
            if done {
                return (p, t);
            }
            assert!(t < 5.);
        }
    }

    #[test]
    fn flat_jump_arrives_where_it_lands() {
        let from = Vec3::new(0., 100., 0.);
        let to = Vec3::new(120., 100., 0.);
        assert!((Jump::flight_time(0.) - 2. * (100f32 / 490.).sqrt()).abs() < 1e-6);
        let j = Jump::new(from, to);
        assert!((j.velocity().x - 120. / Jump::flight_time(0.)).abs() < 1e-3);
        assert!((j.velocity().y - (100f32 / 490.).sqrt() * 490.).abs() < 1e-3);
        // 落地前一帧的 x 已经在落点附近（不是飞过头很远再拉回来）。
        let mut j = Jump::new(from, to);
        let dt = 1. / 240.;
        let mut last = from;
        loop {
            let (p, _) = j.step(dt, false, false);
            if j.phase() == Phase::Land {
                break;
            }
            last = p;
        }
        assert!((last.x - 120.).abs() < 2., "{}", last.x);
        let (p, t) = fly(from, to, dt);
        assert_eq!((p.x, p.y, p.z), (120., 100., 0.));
        // 起跳 0.5 秒 + 飞行 ≈ 0.9 秒。
        assert!((t - (0.5 + Jump::flight_time(0.))).abs() < 0.03, "{t}");
    }

    #[test]
    fn jumping_up_and_down() {
        let dt = 1. / 240.;
        let (p, t) = fly(Vec3::new(0., 0., 0.), Vec3::new(0., 60., 100.), dt);
        assert_eq!((p.y, p.z), (60., 100.));
        assert!((t - (0.5 + Jump::flight_time(60.))).abs() < 0.03, "{t}");
        let (p, t) = fly(Vec3::new(0., 200., 0.), Vec3::new(0., 0., -150.), dt);
        assert_eq!((p.y, p.z), (0., -150.));
        assert!((t - (0.5 + Jump::flight_time(-200.))).abs() < 0.03, "{t}");
    }
}
