//! 屏幕淡入淡出。
//!
//! 四条 SCE 命令走的是同一个单例里的四个效果对象
//! （`FUN_004be590` 取单例，`FUN_004be1a0(n)` 按 `n` 取对象）：
//!
//! | opcode | handler | `n` | 对象 | 是什么 |
//! |---|---|---|---|---|
//! | 69 `CameraFadeOut` | `00437190` | 1 | `+0x20` | 黑淡出 |
//! | 70 `CameraFadeIn` | `004371b0` | 2 | `+0x04` | 黑淡入 |
//! | 81 `CameraFadeOutWhite` | `004371d0` | 3 | `+0x38` | 白淡出 |
//! | 91 `CameraFadeInWhite` | `004371f0` | 4 | `+0x50` | 白淡入 |
//!
//! 客栈住店那段黑屏用的也是这一套（`FUN_004be590(1)` 淡出、`(2)` 淡入，
//! 见 `switch/re/notes/inn.md`）。

/// 一次淡入或淡出多久，秒。
///
/// 四个效果对象的构造函数——`FUN_004be8d0` / `FUN_004bea60` /
/// `FUN_004bebb0` / `FUN_004bed00`——都把 `+0xc` 填成 `0x40200000`，
/// 每帧推进是 `FUN_004bdf30`：
///
/// ```text
/// +0x10 += dt;                     // 走了多久
/// if (+0x10 > +0xc) {              // +0xc 就是这个时长
///     +0x8  = 0;                   // 标记「完成」
///     +0x10 = +0xc;
/// }
/// ```
///
/// 所以四条命令的时长是同一个值。移植端原先按 3.0 秒做，是猜的。
pub const FADE_SECONDS: f32 = 2.5;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fade_takes_two_and_a_half_seconds() {
        // 0x40200000 读作 f32。
        assert_eq!(FADE_SECONDS, f32::from_bits(0x4020_0000));
    }
}
