//! 冒险时的中毒（`PlayerMgr::m_pstate`，`gamebase/playermgr.cpp`；exe 在 `FUN_0040c230` 里每帧推）。
//!
//! 只管受控的那个主角。踩到水浸 / 沼泽 / 熔岩地面（`RoleCtrl::UpdateSnd`，地面类型 6 / 10 / 13）就中
//! 水毒 / 风毒 / 火毒（`SetPoison`）。中着毒的头一帧按毒的种类扣一次精，之后每 4 秒再扣一次；走满 1200
//! （`ROLE_RADIUS × 100`）就解掉。扣精是 `Hurt` / `Obj_Hurt` 的单人版：只扣受控主角，最少留 1。
//!
//! | 毒 | 头一帧 | 之后每 4 秒 |
//! |---|---|---|
//! | 0 风 | 精扣到一半（`c − max(1, c × 0.5)`），有音效 | 1 |
//! | 1 雷 | 最大精的 50%（`Obj_Hurt(0.5)`），有音效 | 1 |
//! | 2 水 | 最大精的 50%，有音效 | 最大精的 50% |
//! | 3 火 | 最大精的 5%，有音效 | 最大精的 5% |
//! | 4 土 | 最大精的一半，有音效；记下这个数 | 上一次 × 1.5 |
//!
//! 之后那几下都不放音效。`SetPoison(p, check, 初始计数)`：`check` 时已经中着同一种毒就什么都不做；状态
//! 清零、记下毒的种类和起点；初始计数为 0 时先把五种毒的状态字节清掉，有避毒（状态 25）就到此为止，否则
//! 中毒的消息栏报 `p<毒>`、状态字节置 1。初始计数为 1（换主角时接着新主角身上已有的毒）跳过头一下。
//! [移植] 原版这份状态是静态变量，没查存档里有没有；这里不进存档。[未做] `Hurt` 扣精时主角头上飘的那个数
//! （`HeadMsg` 类型 0：从头顶 64 处起 2.5 秒升 20、淡掉，16 号数字字体）。

use radiance::math::Vec3;

use super::persistent_state::RoleState;

/// 不中毒（`FS_NUM`，exe 里传的是 0x1f）。
pub const NO_POISON: i32 = 31;
/// 避毒的状态字节（`FS_BIDU`）。
pub const STATE_POISON_IMMUNE: usize = 25;
/// 之后每隔多少秒扣一次（`P_DELAY`）。
pub const POISON_DELAY: f32 = 4.;
/// 走满这么远就解毒（`ROLE_RADIUS × 100`）。
pub const POISON_WALK_OFF: f32 = 1200.;

/// 地面类型（`FLOOR_KIND`）对应的毒：水浸 → 水毒，沼泽 → 风毒，熔岩 → 火毒。
pub fn floor_poison(floor: i32) -> Option<i32> {
    match floor {
        6 => Some(2),
        10 => Some(0),
        13 => Some(3),
        _ => None,
    }
}

/// 扣精的一下。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Hit {
    /// `Hurt(n)`：扣 `n`。
    Flat(i32),
    /// `Obj_Hurt(frac)`：扣最大精的 `frac`。
    Fraction(f32),
}

#[derive(Debug, Clone, Copy)]
pub struct PoisonWalk {
    kind: i32,
    counter: u32,
    delay: f32,
    length: f32,
    last: Vec3,
    last_hurt: f32,
}

impl Default for PoisonWalk {
    fn default() -> Self {
        Self {
            kind: NO_POISON,
            counter: 0,
            delay: 0.,
            length: 0.,
            last: Vec3::new(0., 0., 0.),
            last_hurt: 0.,
        }
    }
}

fn is_poison(p: i32) -> bool {
    (0..=4).contains(&p)
}

impl PoisonWalk {
    pub fn kind(&self) -> i32 {
        self.kind
    }

    /// `PlayerMgr::SetPoison`。`role` 是受控主角的记录，`pos` 是他现在的位置。返回要报的消息键。
    pub fn set(
        &mut self,
        p: i32,
        check: bool,
        init: u32,
        pos: Vec3,
        role: &mut RoleState,
    ) -> Option<String> {
        if check && is_poison(p) && self.kind == p {
            return None;
        }
        *self = Self {
            kind: p,
            counter: init,
            last: pos,
            ..Default::default()
        };
        if init == 0 {
            for s in role.states.iter_mut().take(5) {
                *s = 0;
            }
        }
        if role.states[STATE_POISON_IMMUNE] != 0 {
            return None;
        }
        if is_poison(p) && init == 0 {
            role.states[p as usize] = 1;
            return Some(format!("p{p}"));
        }
        None
    }

    /// 一帧（`FUN_0040c230` 那一段）：要扣的那一下（和放不放音效），以及这一帧走满了要解毒。
    pub fn step(&mut self, dt: f32, pos: Vec3, role: &RoleState) -> (Option<(Hit, bool)>, bool) {
        if !is_poison(self.kind) {
            return (None, false);
        }
        let hit = if self.counter == 0 {
            let first = match self.kind {
                0 => {
                    let c = role.hp;
                    let t = (c as f32 * 0.5).max(1.) as i32;
                    Hit::Flat(c - t)
                }
                1 | 2 => Hit::Fraction(0.5),
                3 => Hit::Flat((role.max_hp as f32 * 0.05) as i32),
                _ => {
                    self.last_hurt = role.max_hp as f32 * 0.5;
                    Hit::Flat(self.last_hurt as i32)
                }
            };
            Some((first, true))
        } else {
            self.delay += dt;
            if self.delay >= POISON_DELAY {
                self.delay = 0.;
                let next = match self.kind {
                    0 | 1 => Hit::Flat(1),
                    2 => Hit::Fraction(0.5),
                    3 => Hit::Flat((role.max_hp as f32 * 0.05) as i32),
                    _ => {
                        self.last_hurt *= 1.5;
                        Hit::Flat(self.last_hurt as i32)
                    }
                };
                Some((next, false))
            } else {
                None
            }
        };
        self.counter += 1;
        // `position` 的 y 恒为 0，量的是 x、z。
        self.length += (pos.x - self.last.x).hypot(pos.z - self.last.z);
        self.last = pos;
        (hit, self.length >= POISON_WALK_OFF)
    }
}

/// `Hurt(n, …, 单人)` / `Obj_Hurt(frac, …, 单人)` 落在一个人身上：精最少留 1，返回增减。
pub fn apply_hit(role: &mut RoleState, hit: Hit) -> i32 {
    let old = role.hp;
    role.hp = match hit {
        Hit::Flat(n) => (old - n).max(1),
        Hit::Fraction(f) => {
            let v = (old as f32 - role.max_hp as f32 * f) as i32;
            if v <= 0 { 1 } else { v }
        }
    };
    role.hp - old
}

/// 扣精的音效，按受控主角（`we015`..`we019`）。
pub fn hurt_sound(role: i32) -> Option<&'static str> {
    ["we015", "we016", "we017", "we018", "we019"]
        .get(role.max(0) as usize)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role(hp: i32, max: i32) -> RoleState {
        let mut r = super::super::persistent_state::growth_test_role();
        r.states = [0; super::super::persistent_state::STATE_BYTES];
        r.hp = hp;
        r.max_hp = max;
        r
    }

    #[test]
    fn wet_floor_poisons_once_and_walking_cures() {
        let mut r = role(100, 200);
        let mut pw = PoisonWalk::default();
        let o = Vec3::new(0., 0., 0.);
        assert_eq!(pw.set(2, true, 0, o, &mut r), Some("p2".into()));
        assert_eq!(r.states[2], 1);
        // 已经中着水毒，再踩不重来。
        assert_eq!(pw.set(2, true, 0, o, &mut r), None);
        let (hit, cured) = pw.step(0.1, o, &r);
        assert_eq!(hit, Some((Hit::Fraction(0.5), true)));
        assert!(!cured);
        assert_eq!(apply_hit(&mut r, Hit::Fraction(0.5)), -99);
        assert_eq!(r.hp, 1);
        // 4 秒后再一下，不放音效。
        let (hit, _) = pw.step(3.95, o, &r);
        assert_eq!(hit, None);
        let (hit, _) = pw.step(0.1, o, &r);
        assert_eq!(hit, Some((Hit::Fraction(0.5), false)));
        // 走满 1200 解毒。
        let (_, cured) = pw.step(0.1, Vec3::new(1200., 0., 0.), &r);
        assert!(cured);
    }

    #[test]
    fn earth_poison_grows_and_immunity_blocks_the_flag() {
        let mut r = role(1000, 1000);
        let mut pw = PoisonWalk::default();
        let o = Vec3::new(0., 0., 0.);
        pw.set(4, true, 0, o, &mut r);
        assert_eq!(pw.step(0.1, o, &r).0, Some((Hit::Flat(500), true)));
        pw.step(4., o, &r);
        assert_eq!(pw.step(0.1, o, &r).0, None);
        let mut r2 = role(1000, 1000);
        r2.states[STATE_POISON_IMMUNE] = 1;
        let mut pw2 = PoisonWalk::default();
        assert_eq!(pw2.set(3, true, 0, o, &mut r2), None);
        assert_eq!(r2.states[3], 0);
        // 避毒挡住的只是状态和消息，毒照样记下、照样扣（原版如此）。
        assert_eq!(pw2.kind(), 3);
    }
}
