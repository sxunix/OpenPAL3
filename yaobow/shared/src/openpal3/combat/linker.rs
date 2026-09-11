//! `cbdata\efflinker.dat`：技能 / 道具 → 特效组、挂在哪、什么时候结算
//! （`D:\Project\PAL3\combat\CBStageEffect.cpp`）。
//!
//! `FUN_00508e30` 把整个文件（0x5a00 字节）读进 CBStageEffect `+0x98c`：384 条
//! × 60 字节。`FUN_00508880` 按 id 线性查（查不到就占一条空位）；`FUN_00508930`
//! 按「第几段」播一段：
//!
//! ```text
//! +0x00 id            技能 id（实机 187 个技能全在）或可投掷道具 id（59 个），
//!                     另有 501..510 十条对不上数据库
//! +0x04 第一段特效组   -1 = 没有
//! +0x08 四个挂点字节   见 [`Anchor`]
//! +0x0c 三个 float    挂点全 0 时用的坐标
//! +0x18 第一段起播时刻
//! +0x1c 结算时刻       道具编排 FUN_00500550 到这个时刻才调 FUN_004ef8b0
//! +0x20 第二段特效组 / +0x24 挂点 / +0x28 坐标 / +0x34 起播时刻
//! ```
//!
//! 逆向记录在 `switch/re/notes/combat_vfx.md`。

pub const LINKER_PATH: &str = "/basedata/basedata/cbdata/efflinker.dat";
pub const RECORDS: usize = 0x180;
pub const RECORD_SIZE: usize = 0x3c;
pub const FILE_SIZE: usize = RECORDS * RECORD_SIZE;

/// 特效挂在哪（`FUN_00508930` 按四个字节的先后覆盖，最后一个置位的算数）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Anchor {
    /// 字节 0：出手这一方的定点——我方是 CBStage `+0x384` 那个点，敌方是 10 号位。
    ActorSide,
    /// 字节 1：目标那一方的定点（同上，按目标在哪边）。
    TargetSide,
    /// 字节 2：出手的人。
    Actor,
    /// 字节 3：目标（打一整边时退回目标那一方的定点）。
    Target,
    /// 四个字节全 0：记录里的坐标。
    Fixed([f32; 3]),
}

impl Anchor {
    fn decode(flags: [u8; 4], pos: [f32; 3]) -> Self {
        // 覆盖顺序 0 → 1 → 2 → 3，所以从后往前找第一个置位的。
        if flags[3] != 0 {
            Anchor::Target
        } else if flags[2] != 0 {
            Anchor::Actor
        } else if flags[1] != 0 {
            Anchor::TargetSide
        } else if flags[0] != 0 {
            Anchor::ActorSide
        } else {
            Anchor::Fixed(pos)
        }
    }
}

/// 一段特效：哪一组、挂在哪、第几秒起播。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LinkedEffect {
    pub group: i32,
    pub anchor: Anchor,
    pub at: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EffectLink {
    pub id: u32,
    pub first: Option<LinkedEffect>,
    pub second: Option<LinkedEffect>,
    /// 结算时刻（秒，从出手动作开始算）。
    pub result_at: f32,
}

impl EffectLink {
    fn parse(b: &[u8]) -> Self {
        let i = |o: usize| i32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let effect = |group_at: usize, flags_at: usize, pos_at: usize, time_at: usize| {
            let group = i(group_at);
            (group >= 0).then(|| LinkedEffect {
                group,
                anchor: Anchor::decode(
                    b[flags_at..flags_at + 4].try_into().unwrap(),
                    [f(pos_at), f(pos_at + 4), f(pos_at + 8)],
                ),
                at: f(time_at),
            })
        };
        Self {
            id: i(0) as u32,
            first: effect(0x04, 0x08, 0x0c, 0x18),
            second: effect(0x20, 0x24, 0x28, 0x34),
            result_at: f(0x1c),
        }
    }

    /// 两段依次（有的才给）。
    pub fn effects(&self) -> impl Iterator<Item = LinkedEffect> + '_ {
        self.first.into_iter().chain(self.second)
    }
}

#[derive(Debug, Clone)]
pub struct EffectLinker {
    links: Vec<EffectLink>,
}

impl EffectLinker {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        (bytes.len() >= FILE_SIZE).then(|| Self {
            links: (0..RECORDS)
                .map(|k| EffectLink::parse(&bytes[k * RECORD_SIZE..(k + 1) * RECORD_SIZE]))
                .collect(),
        })
    }

    /// `FUN_00508880` 的查法（查不到时原版会占一条空位，这里直接 `None`）。
    pub fn find(&self, id: u32) -> Option<EffectLink> {
        if id == 0 {
            return None;
        }
        self.links.iter().find(|l| l.id == id).copied()
    }

    pub fn len(&self) -> usize {
        self.links.iter().filter(|l| l.id != 0).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: u32, group: i32, flags: [u8; 4], result_at: f32) -> Vec<u8> {
        let mut b = vec![0u8; RECORD_SIZE];
        b[0..4].copy_from_slice(&id.to_le_bytes());
        b[4..8].copy_from_slice(&group.to_le_bytes());
        b[8..12].copy_from_slice(&flags);
        b[0x1c..0x20].copy_from_slice(&result_at.to_le_bytes());
        b[0x20..0x24].copy_from_slice(&(-1i32).to_le_bytes());
        b
    }

    #[test]
    fn the_file_is_384_records_of_60_bytes() {
        assert_eq!(FILE_SIZE, 0x5a00, "FUN_00508e30 读 0x5a00");
        assert_eq!(FILE_SIZE, 23040);
    }

    #[test]
    fn a_record_parses_the_way_fun_00508930_reads_it() {
        let mut bytes = record(245, 371, [0, 1, 0, 0], 1.5);
        bytes.resize(FILE_SIZE, 0);
        let l = EffectLinker::parse(&bytes).unwrap();
        let link = l.find(245).unwrap();
        assert_eq!(link.result_at, 1.5);
        assert_eq!(
            link.first,
            Some(LinkedEffect {
                group: 371,
                anchor: Anchor::TargetSide,
                at: 0.
            })
        );
        assert_eq!(link.second, None, "-1 = 没有第二段");
        assert_eq!(link.effects().count(), 1);
        assert_eq!(l.find(0), None);
        assert_eq!(l.find(9999), None);
        assert!(EffectLinker::parse(&bytes[..FILE_SIZE - 1]).is_none());
    }

    #[test]
    fn the_last_set_anchor_byte_wins() {
        let p = [1., 2., 3.];
        assert_eq!(Anchor::decode([0, 0, 0, 1], p), Anchor::Target);
        assert_eq!(Anchor::decode([1, 1, 1, 1], p), Anchor::Target);
        assert_eq!(Anchor::decode([1, 0, 1, 0], p), Anchor::Actor);
        assert_eq!(Anchor::decode([1, 0, 0, 0], p), Anchor::ActorSide);
        assert_eq!(Anchor::decode([0, 0, 0, 0], p), Anchor::Fixed(p));
    }
}
