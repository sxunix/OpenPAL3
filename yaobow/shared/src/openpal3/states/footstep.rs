//! 玩家的脚步声。
//!
//! 原版在 `FUN_0041eb80` 里每帧挑一条循环播：走一套、跑一套，各四个变体
//! 按脚下的地面类型选。装载在 `FUN_0041ed00`。
//!
//! ```text
//! if (DAT_005833bc & 1) {                       // 总开关
//!     iVar1 = 主角->+0x504;                      // 移动模式
//!     if      (iVar1 == 1) this = 走[FUN_0041eb50(地面)];
//!     else if (iVar1 == 2) this = 跑[FUN_0041eb50(地面)];
//!     else                 this = null;          // 不动，或别的动作
//!
//!     // 和上一帧同一条就不动（继续循环），换了就 Stop 旧的 Play 新的
//! }
//! ```
//!
//! 顺带，这一段是 `+0x504` 在玩家身上取 1 / 2 的**直接证据**——
//! `switch/re/notes/role_move.md` §7 追玩家走路速度时缺的就是它。

use super::role_move::MoveMode;

/// 走路那一套的文件名前缀（`snd\we021%c.wav`，0x58b24c）。
pub const WALK_PREFIX: &str = "we021";

/// 跑步那一套（`snd\we022%c.wav`，0x58b23c）。
pub const RUN_PREFIX: &str = "we022";

/// 每套几个变体。`FUN_0041ed00` 从 `'a'`（0x61）起循环，
/// `while (iVar5 - 0x60 < 4)`，装 `a` `b` `c` `d` 四个。
pub const VARIANTS: usize = 4;

/// 地面类型挑第几个变体（`FUN_0041eb50`）。
///
/// ```text
/// 5  → 2      7  → 1      11 → 3      其余 → 0
/// ```
pub fn variant_for_floor(floor_type: i32) -> usize {
    match floor_type {
        5 => 2,
        7 => 1,
        11 => 3,
        _ => 0,
    }
}

/// 这一帧该循环播哪个文件；`None` 表示该停（没在走也没在跑）。
///
/// 只有走和跑有声，站着挪（`MoveMode::Still`，内部值 3）原版落在
/// `else` 那一支，什么都不播。
pub fn sound_name(mode: Option<MoveMode>, floor_type: i32) -> Option<String> {
    let prefix = match mode? {
        MoveMode::Walk => WALK_PREFIX,
        MoveMode::Run => RUN_PREFIX,
        MoveMode::Still => return None,
    };
    let letter = (b'a' + variant_for_floor(floor_type) as u8) as char;
    Some(format!("{prefix}{letter}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_floor_type_picks_one_of_four_variants() {
        assert_eq!(variant_for_floor(5), 2);
        assert_eq!(variant_for_floor(7), 1);
        assert_eq!(variant_for_floor(11), 3);
        // 别的一律第一个，包括越界时原版填的那个 2。
        assert_eq!(variant_for_floor(0), 0);
        assert_eq!(variant_for_floor(2), 0);
        assert_eq!(variant_for_floor(6), 0);
        assert_eq!(variant_for_floor(-1), 0);
        assert_eq!(variant_for_floor(999), 0);

        // 四个变体都取得到，且都在 a..d 里。
        let picked: Vec<usize> = [5, 7, 11, 0]
            .iter()
            .map(|&f| variant_for_floor(f))
            .collect();
        let mut sorted = picked.clone();
        sorted.sort();
        assert_eq!(sorted, [0, 1, 2, 3], "四个变体应当都用得上");
        assert!(picked.iter().all(|&v| v < VARIANTS));
    }

    #[test]
    fn walking_and_running_use_different_sets() {
        assert_eq!(sound_name(Some(MoveMode::Walk), 0).unwrap(), "we021a");
        assert_eq!(sound_name(Some(MoveMode::Run), 0).unwrap(), "we022a");
        // 地面换了，同一套里换变体。
        assert_eq!(sound_name(Some(MoveMode::Walk), 5).unwrap(), "we021c");
        assert_eq!(sound_name(Some(MoveMode::Walk), 7).unwrap(), "we021b");
        assert_eq!(sound_name(Some(MoveMode::Walk), 11).unwrap(), "we021d");
        assert_eq!(sound_name(Some(MoveMode::Run), 11).unwrap(), "we022d");
    }

    #[test]
    fn standing_still_makes_no_sound() {
        assert_eq!(sound_name(None, 0), None);
        // 站姿移动那一支原版也不播。
        assert_eq!(sound_name(Some(MoveMode::Still), 0), None);
        assert_eq!(sound_name(Some(MoveMode::Still), 5), None);
    }
}
