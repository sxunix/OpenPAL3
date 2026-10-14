//! 冲撞（`.scn` 类型 13，蜀山故道的摆锤 / 石块）——`FUN_00426eb0` case 0xd，源码
//! `gamebase/sceneobj.cpp:2217`。
//!
//! 物件是一个不动的架子（主模型）加一个来回摆的部件（同目录下名字后加 `A.pol` 的子模型，
//! `SceneObj::Create` 里加载，`sceneobj.cpp:134`）。摆的部件每帧：计时 `t += dt × 1.4`，
//! 偏移 = 方向 × 200 × sin t，长度超过 60 就截成 60（这时 sin t > 0 的那一侧不判碰撞），再加上
//! 方向 × −30；部件位置 = 原位 − 偏移。方向是物件绕 Y 转的角度：(sin 角, 0, cos 角)。
//!
//! 部件状态 0 时、这一下判碰撞、离上次受伤超过 [`HURT_DELAY`]：部件的包围盒（子模型的包围盒，
//! 绕 Y 转同一个角度，放到部件位置）和主角的包围盒（模型包围盒，x 边长乘 0.6，`Role::IsCollideOBB`）
//! 相交就撞上——主角站住、受控，速度 = 方向 × 300 + (0, 300, 0)；全队每人扣最大精的 5%
//! （`PlayerMgr::Obj_Hurt(0.05)`，最少剩 1）；受伤计时清零。之后（状态 1）主角按这个速度飞，飞出
//! 画面或者飞满 2 秒，就换到参数 2 那一层、放到参数 (0, 1) 那格，部件回状态 0。
//!
//! [移植] 主角的包围盒按「以主角位置为中心、半宽 = 模型半径 × 0.6（x）/ 模型半径（z）、高 = 模型高」
//! 算，朝向不转；「飞出画面」不判，飞满 2 秒落地。

use radiance::math::Vec3;

/// `HURT_DELAY`（`gamebase/scenemgr.h:16`，exe 里比的 1.2）。
pub const HURT_DELAY: f32 = 1.2;
/// 撞飞后最多飞多久（`m_fParam >= 2.0f`）。
pub const FLY_SECS: f32 = 2.;
/// 扣血比例（`Obj_Hurt(0.05f)`）。
pub const HURT_PERCENT: f32 = 0.05;

/// 子模型（摆的部件）的包围盒：中心相对部件原点的偏移和三边长（未转）。
#[derive(Debug, Clone, Copy)]
pub struct PartBox {
    pub center: [f32; 3],
    pub ext: [f32; 3],
}

#[derive(Debug, Clone)]
pub struct Ram {
    /// `.scn` 序号。
    pub node: usize,
    /// 物件原位（`m_info.pos`）。
    pub base: Vec3,
    /// 绕 Y 的角度（度）。
    pub rotate: f32,
    /// 方向 × 200（`m_subobj.vel`）。
    pub vel: Vec3,
    /// `m_subobj.time`，建的时候是 `rand() % 300 / 100`。
    pub time: f32,
    /// 0 摆着，1 主角正被撞飞。
    pub state: u8,
    /// 撞飞后过了多久（`m_fParam`）。
    pub fly: f32,
    /// 落地的层、格（参数 2、参数 0/1）。
    pub target: (usize, i32, i32),
    pub part: PartBox,
}

impl Ram {
    pub fn new(
        node: usize,
        base: Vec3,
        rotate: f32,
        time: f32,
        target: (usize, i32, i32),
        part: PartBox,
    ) -> Self {
        let r = rotate.to_radians();
        Self {
            node,
            base,
            rotate,
            vel: Vec3::new(r.sin() * 200., 0., r.cos() * 200.),
            time,
            state: 0,
            fly: 0.,
            target,
            part,
        }
    }

    /// 摆一帧：返回部件的位置和这一下判不判碰撞。
    pub fn swing(&mut self, dt: f32) -> (Vec3, bool) {
        self.time += dt * 1.4;
        let s = self.time.sin();
        let mut off = Vec3::new(self.vel.x * s, self.vel.y * s, self.vel.z * s);
        let mut col = true;
        let len = off.norm();
        if len > 60. {
            off = Vec3::scalar_mul(60. / len, &off);
            if s > 0. {
                col = false;
            }
        }
        let n = self.vel.norm();
        let back = Vec3::scalar_mul(-30. / n, &self.vel);
        let off = Vec3::add(&off, &back);
        (Vec3::sub(&self.base, &off), col)
    }

    /// 方向单位向量。
    pub fn dir(&self) -> Vec3 {
        let n = self.vel.norm();
        Vec3::scalar_mul(1. / n, &self.vel)
    }

    /// 撞飞的速度：方向 × 300 + (0, 300, 0)。
    pub fn knock_velocity(&self) -> Vec3 {
        let d = self.dir();
        Vec3::new(d.x * 300., 300., d.z * 300.)
    }

    /// 部件在 `pos` 时，和中心在 `player`、半宽 (`hx`, `hz`)、高 `h`（脚底在 `player.y`）的主角盒子相不相交：
    /// 两个盒子都只绕 Y 转，按 x–z 平面上的分离轴（四根）加 y 区间判。
    pub fn hits(&self, pos: &Vec3, player: &Vec3, hx: f32, hz: f32, h: f32) -> bool {
        let r = self.rotate.to_radians();
        let (c, s) = (r.cos(), r.sin());
        // 子模型局部 (x, z) 绕 Y 转 rotate：x' = x cos + z sin，z' = −x sin + z cos。
        let rot = |x: f32, z: f32| (x * c + z * s, -x * s + z * c);
        let (ox, oz) = rot(self.part.center[0], self.part.center[2]);
        let center = (pos.x + ox, pos.z + oz);
        let cy = pos.y + self.part.center[1];
        let (ey, py0, py1) = (self.part.ext[1] * 0.5, player.y, player.y + h);
        if cy + ey < py0 || cy - ey > py1 {
            return false;
        }
        let a_axes = [rot(1., 0.), rot(0., 1.)];
        let a_half = [self.part.ext[0] * 0.5, self.part.ext[2] * 0.5];
        let b_axes = [(1., 0.), (0., 1.)];
        let b_half = [hx, hz];
        let d = (player.x - center.0, player.z - center.1);
        for axis in a_axes.iter().chain(b_axes.iter()) {
            let proj = |axes: &[(f32, f32); 2], half: &[f32; 2]| {
                (axes[0].0 * axis.0 + axes[0].1 * axis.1).abs() * half[0]
                    + (axes[1].0 * axis.0 + axes[1].1 * axis.1).abs() * half[1]
            };
            let dist = (d.0 * axis.0 + d.1 * axis.1).abs();
            if dist > proj(&a_axes, &a_half) + proj(&b_axes, &b_half) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ram() -> Ram {
        Ram::new(
            0,
            Vec3::new(0., 0., 0.),
            0.,
            0.,
            (1, 10, 20),
            PartBox {
                center: [0., 20., 0.],
                ext: [40., 40., 40.],
            },
        )
    }

    #[test]
    fn swing_stays_between_minus_thirty_and_ninety_along_the_direction() {
        // 方向 (0, 0, 1)：位置 = 原位 − (截到 ±60 的 200 sin t) + 30。
        let mut r = ram();
        let mut seen = (f32::MAX, f32::MIN);
        for _ in 0..400 {
            let (p, _) = r.swing(0.01);
            seen = (seen.0.min(p.z), seen.1.max(p.z));
        }
        assert!((seen.0 - (-30.)).abs() < 0.5, "{seen:?}");
        assert!((seen.1 - 90.).abs() < 0.5, "{seen:?}");
    }

    #[test]
    fn clamped_positive_half_does_not_collide() {
        let mut r = ram();
        // t ≈ π/2：sin t = 1，偏移 200 截到 60，不判碰撞。
        r.time = std::f32::consts::FRAC_PI_2 - 0.001 * 1.4;
        let (_, col) = r.swing(0.001);
        assert!(!col);
        // t ≈ −π/2：截了但 sin t < 0，判。
        r.time = -std::f32::consts::FRAC_PI_2;
        let (_, col) = r.swing(0.);
        assert!(col);
    }

    #[test]
    fn box_overlap_uses_the_part_box_and_height() {
        let r = ram();
        let at = Vec3::new(0., 0., 0.);
        assert!(r.hits(&at, &Vec3::new(25., 0., 0.), 7., 12., 60.));
        assert!(!r.hits(&at, &Vec3::new(30., 0., 0.), 7., 12., 60.));
        // 主角整个在部件上方。
        assert!(!r.hits(&at, &Vec3::new(0., 50., 0.), 7., 12., 60.));
    }
}
