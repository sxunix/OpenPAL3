//! `audio` 特效类 —— 143 播的特效组里管声音的那一类。
//!
//! 逆向写在 `switch/re/notes/flexeff_format.md`。原型是 `new(304)` +
//! `FUN_004c1570`，实例 vtable 在 0x56f674。它没有画面，整个类就干一件事：
//! 按定义记录的资源名当前缀，拼出 `前缀 + %03d` 的声音名去播。
//!
//! 名字是 `sprintf(buf, "%s%03d", 前缀, (int)号码)`（格式串在 0x5d4e54），
//! **号码取自子块的 `+0` / `+8` / `+32` / `+44` 四个浮点**——预载那一支
//! （`FUN_004c1700`）读这四个位置，播放那一支读实例的
//! `+0x18`/`+0x20`/`+0x38`/`+0x44`，而子块是拷在实例 `+0x18` 的，换算过来
//! 正是同样四个。非零的各播一次，播完在实例上打标记（`+0x2c`..`+0x38`），
//! 所以一个实例每个号码只响一次。
//!
//! 这四个偏移和按类统计对得上：`audio` 那一行密集用到的就是
//! `+0`(96%) / `+8`(55%) / `+32`(42%) / `+44`(33%)。
//!
//! 实机里所有被特效组引用到的 `audio` 定义前缀都是 `wd`，拼出来 118 个名字
//! ，其中 117 个能在 `/snd/*.wav` 找到；剩下那个号码是 14200000，明显是没清
//! 干净的字段，原版拼出来同样找不到文件。

use fileformats::pal3::flexeff::EffectDef;

/// 号码在子块里的四个位置。
pub const SOUND_SLOTS: [usize; 4] = [0, 8, 32, 44];

/// 声音都在 `/snd` 下，`.wav`。
pub const SND_DIR: &str = "/snd";

/// 一条 `audio` 定义加一个子块，拼出这次要播的声音名。
///
/// 号码是浮点，原版用 `fistp` 取整（朝零截断）。为零的槽跳过。
pub fn sound_names(def: &EffectDef<'_>, sub: &[u8]) -> Vec<String> {
    let prefix = def.asset();
    if prefix.is_empty() {
        return vec![];
    }
    SOUND_SLOTS
        .iter()
        .filter_map(|o| {
            let v = f32::from_le_bytes(sub[*o..*o + 4].try_into().unwrap());
            (v != 0. && v.is_finite()).then(|| format!("{prefix}{:03}", v as i32))
        })
        .collect()
}

/// 一个排着队的 `audio` 特效：等 [`Self::delay`] 秒后把 [`Self::names`] 里的
/// 声音各播一次。
///
/// 原版是挂在时间线上的，延迟同样取子块 `+72`；四个号码在同一次回调里一起
/// 播出去，所以这里也一次放完。
#[derive(Debug, Clone)]
pub struct AudioEffect {
    pub names: Vec<String>,
    pub delay: f32,
    elapsed: f32,
    fired: bool,
}

impl AudioEffect {
    pub fn new(names: Vec<String>, sub: &[u8]) -> Self {
        Self {
            names,
            delay: f32::from_le_bytes(sub[72..76].try_into().unwrap()),
            elapsed: 0.,
            fired: false,
        }
    }

    pub fn finished(&self) -> bool {
        self.fired
    }

    /// 推进一帧；到点了就把要播的名字交出去（只交一次）。
    pub fn step(&mut self, delta_sec: f32) -> Option<&[String]> {
        if self.fired {
            return None;
        }
        self.elapsed += delta_sec;
        if self.elapsed < self.delay {
            return None;
        }
        self.fired = true;
        Some(&self.names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub_with(pairs: &[(usize, f32)]) -> Vec<u8> {
        let mut b = vec![0u8; 124];
        for (o, v) in pairs {
            b[*o..*o + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    #[test]
    fn the_delay_comes_from_the_sub_block_and_it_fires_once() {
        let sub = sub_with(&[(72, 0.5)]);
        let mut a = AudioEffect::new(vec!["wd001".into()], &sub);
        assert_eq!(a.delay, 0.5);
        assert!(a.step(0.2).is_none(), "延迟没到不该响");
        assert!(a.step(0.4).is_some(), "到点该响");
        assert!(a.step(1.0).is_none(), "只响一次");
        assert!(a.finished());
    }

    #[test]
    fn a_zero_slot_is_skipped() {
        // 四个槽里只有两个有号码。
        let sub = sub_with(&[(0, 1.0), (8, 0.0), (32, 27.0), (44, 0.0)]);
        // 不经过 EffectDef 直接验槽位的取法。
        let picked: Vec<i32> = SOUND_SLOTS
            .iter()
            .map(|o| f32::from_le_bytes(sub[*o..*o + 4].try_into().unwrap()))
            .filter(|v| *v != 0.)
            .map(|v| v as i32)
            .collect();
        assert_eq!(picked, vec![1, 27]);
    }

    #[test]
    fn the_number_is_three_digits_zero_padded() {
        assert_eq!(format!("{}{:03}", "wd", 1), "wd001");
        assert_eq!(format!("{}{:03}", "wd", 27), "wd027");
        assert_eq!(format!("{}{:03}", "wd", 142), "wd142");
    }
}
