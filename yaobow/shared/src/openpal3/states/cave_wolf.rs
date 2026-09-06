//! 山洞初体验（山洞之狼，SCE 112）的规则。
//!
//! 原版把它做成一个游戏状态：`LAB_00437c20` → `FUN_00406ff0(0)` 置
//! `DAT_00bfda6c = 7`、清计时器 `DAT_00bfda58`、跑 `init.sce` 的过程 99
//! 「初始化山洞之狼」；主循环的 `case 7`（0x405690 起）每帧加时间，
//! 满 60 秒就 `FUN_004071c0(-1)`，否则扫一遍角色表，**活着的角色**
//! （`+0x278`，从 `.scn` 角色记录的 `+0x54` 抄来）离主角不到 24 就
//! `FUN_004071c0(角色下标)`。
//!
//! `FUN_004071c0` 收尾（`DAT_00bfda2c == 0`，也就是从剧情里进来那条路）：
//! 回冒险模式，参数非 −1 → 过程 98「胜利结束山洞之狼」并发一件奖品，
//! 参数 −1 → 过程 97「失败结束山洞之狼」。四件奖品发完就不再发
//! （`DAT_00bfda44` 计数，`< 4` 才给）。
//!
//! 目标是**抓住她**，不是躲开：M18 的说明写着「在规定时间内，用任何方式
//! 捉住她」。

/// 时限，`0x56a6c0` 这个浮点常量。
pub const TIME_LIMIT: f32 = 60.0;
/// 抓到的距离。
pub const CATCH_DISTANCE: f32 = 24.0;

/// `init.sce` 里的三个过程。
pub const SETUP_PROC: u32 = 99;
pub const WIN_PROC: u32 = 98;
pub const LOSE_PROC: u32 = 97;

/// 前四次胜利各发一件（0x40726f 起压进栈的那张表）。
pub const REWARDS: [u32; 4] = [0x1388f, 0x14444, 0x1fd9, 0x1fda];

/// 这一局怎么结束的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 抓到了 → 过程 98，外加奖品。
    Caught,
    /// 时间到 → 过程 97。
    TimedOut,
}

/// 一局的计时。
#[derive(Debug, Clone, Default)]
pub struct CaveWolf {
    pub elapsed: f32,
}

impl CaveWolf {
    pub fn new() -> Self {
        Self { elapsed: 0. }
    }

    /// 走一帧。`nearest` 是活着的角色里离主角最近的那个距离，没有角色就是
    /// `None`。
    ///
    /// 顺序照原版：先加时间、先判时限，再看有没有抓到。
    pub fn tick(&mut self, delta_sec: f32, nearest: Option<f32>) -> Option<Outcome> {
        self.elapsed += delta_sec;
        if self.elapsed >= TIME_LIMIT {
            return Some(Outcome::TimedOut);
        }
        match nearest {
            Some(d) if d < CATCH_DISTANCE => Some(Outcome::Caught),
            _ => None,
        }
    }

    /// 还剩多少秒。
    pub fn remaining(&self) -> f32 {
        (TIME_LIMIT - self.elapsed).max(0.)
    }
}

/// 第 `wins` 次胜利（0 基）该发哪件奖品，发完四件就没有了。
pub fn reward_for(wins: u32) -> Option<u32> {
    REWARDS.get(wins as usize).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catching_her_inside_the_time_limit_wins() {
        let mut g = CaveWolf::new();
        assert_eq!(g.tick(0.5, Some(30.)), None, "还没够近");
        assert_eq!(g.tick(0.5, Some(23.9)), Some(Outcome::Caught));
    }

    #[test]
    fn the_clock_is_checked_before_the_distance() {
        // 原版 case 7 先判 60 秒再扫角色，最后一帧同时满足时算时间到。
        let mut g = CaveWolf::new();
        g.elapsed = TIME_LIMIT - 0.1;
        assert_eq!(g.tick(0.2, Some(0.)), Some(Outcome::TimedOut));
    }

    #[test]
    fn an_empty_scene_just_runs_the_clock_out() {
        let mut g = CaveWolf::new();
        for _ in 0..119 {
            assert_eq!(g.tick(0.5, None), None);
        }
        assert_eq!(g.tick(0.5, None), Some(Outcome::TimedOut));
        assert_eq!(g.remaining(), 0.);
    }

    #[test]
    fn only_the_first_four_wins_pay_out() {
        assert_eq!(reward_for(0), Some(0x1388f));
        assert_eq!(reward_for(3), Some(0x1fda));
        assert_eq!(reward_for(4), None);
    }
}
