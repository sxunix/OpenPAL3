//! 战斗舞台上的人做什么动作。
//!
//! 原版每个舞台角色（`CBStageRole`）把当前动作的**编号**存在 `+0x19a8`，
//! `FUN_0050a210` 拿它去 `PTR_DAT_0056fdac` 那张名字表里取名字播放。编号由
//! 各个行动任务写（攻击、仙术、防御……每个任务一个函数，注册进角色的任务
//! 表 `+0x1a0` 起、每项 0x60 字节，`FUN_0050b430` 逐项跑）。
//!
//! 同一件事我方和怪物用两套名字：我方是 `z01`…`z20`，怪物是 `z1` `g` `b`
//! `x1`…。选哪套看战斗位 `+0x243`（`FUN_004ed040`）：非零走我方那套
//! （舞台角色初始化 0x509843：非零写 0x20 = `z04`，否则写 0x32 = `z1`）。
//! 敌人上场时 `FUN_004f8bb0` 先把整个战斗位清零、再把这个字节写成 0，所以
//! 敌人恒走怪物那套；我方的战斗位是从冒险角色记录整块拷来的。
//!
//! **时序是移植端的**：原版攻击要先走到目标跟前（`z03`）、打完再走回来，
//! 这些位移任务还没逆；这里在规则层结算那一刻让出手的人和挨打的人同时做
//! 动作。逆向记录在 `switch/re/notes/combat_motion.md`。

use super::state::{Combatant, K_SLEEP, Phase};

/// `PTR_DAT_0056fdac`：动作编号 → 名字，59 项（0x56fdac..0x56fe98）。
/// 和 Pal3.Unity 的 `ActorConstants.ActionToNameMap` 逐项一致。
pub const ACTION_NAMES: [&str; 59] = [
    // 0..15 冒险动作
    "c01", "c02", "c03", "c15", "c04", "c05", "c052", "c06", "c07", "c08", "c09", "c10", "c11",
    "c12", "c13", "c14", //
    // 16..28 剧情动作
    "j01", "j02", "j03", "j04", "j05", "j06", "j07", "j08", "j09", "j10", "j11", "j12", "j13",
    // 29..49 我方战斗动作
    "z01", "z02", "z03", "z04", "z05", "z061", "z06", "z09", "z10", "z11", "z12", "z13", "z14",
    "z141", "z142", "z15", "z16", "z17", "z18", "z19", "z20", //
    // 50..58 怪物动作（58 又是 z3：怪物「跑」拿走路顶）
    "z1", "z2", "z3", "g", "b", "x1", "x2", "x3", "z3",
];

/// 一件要做的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// 站着等（我方「备战」z04，怪物 z1）
    Idle,
    /// 攻击
    Attack,
    /// 挨打
    BeAttacked,
    /// 仙术预备（出手段里一直做）
    MagicWait,
    /// 仙术
    Magic,
    /// 用恢复类道具
    ItemRecover,
    /// 用攻击类道具
    ItemAttack,
    /// 开始防御
    StartDefence,
    /// 防御中
    Defence,
    /// 防御中挨打
    DefenceBeAttacked,
    /// 眠定（睡着）
    Freeze,
    /// 倒下
    Dead,
    /// 逃跑
    Flee,
}

/// 用哪套名字。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionSet {
    /// `+0x243 != 0`
    Party,
    /// `+0x243 == 0`
    Monster,
}

impl MotionSet {
    /// 动作编号。`None` = 这套里没有，别播。编号就是各任务写进 `+0x19a8`
    /// 的立即数（地址见 `combat_motion.md` 的表）。
    pub fn index(self, motion: Motion) -> Option<usize> {
        use Motion::*;
        match self {
            MotionSet::Party => Some(match motion {
                Attack => 29,
                Idle => 32,
                BeAttacked => 33,
                MagicWait => 34,
                Magic => 35,
                ItemRecover => 38,
                ItemAttack => 39,
                StartDefence => 41,
                Defence => 42,
                DefenceBeAttacked => 43,
                Freeze => 45,
                Dead => 47,
                Flee => 48,
            }),
            MotionSet::Monster => match motion {
                Idle => Some(50),
                Attack => Some(53),
                BeAttacked => Some(54),
                Magic => Some(55),
                _ => None,
            },
        }
    }

    pub fn name(self, motion: Motion) -> Option<&'static str> {
        self.index(motion).map(|i| ACTION_NAMES[i])
    }
}

/// 出手段里登记的是哪一类行动。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActKind {
    Attack,
    Cast,
    /// `hostile`：目标在对面（攻击类道具）。
    Item {
        hostile: bool,
    },
    Escape,
}

/// 一帧里看得见的状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub hp: i32,
    pub alive: bool,
    pub defending: bool,
    pub asleep: bool,
    /// 在出手段里，登记的是什么。
    pub acting: Option<ActKind>,
}

impl Snapshot {
    pub fn of(c: &Combatant, acting: Option<ActKind>) -> Self {
        Self {
            hp: c.hp,
            alive: c.alive,
            defending: c.defending,
            asleep: c.has_state(K_SLEEP),
            acting: if c.phase == Phase::Executing {
                acting
            } else {
                None
            },
        }
    }
}

/// 怎么播。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Play {
    /// 播一遍，完了回到歇着的动作。
    Once(Motion),
    /// 一直循环。
    Loop(Motion),
    /// 播一遍停在最后一帧（倒下）。
    Stay(Motion),
    /// 回到歇着的动作。
    Rest,
}

/// 歇着的时候做什么：睡着 > 防御 > 待机。
pub fn rest_motion(now: &Snapshot) -> Motion {
    if now.asleep {
        Motion::Freeze
    } else if now.defending {
        Motion::Defence
    } else {
        Motion::Idle
    }
}

/// 从上一帧到这一帧，这个人该换什么动作。`None` = 保持现状。
pub fn react(prev: &Snapshot, now: &Snapshot) -> Option<Play> {
    // 倒下压过一切。
    if prev.alive && !now.alive {
        return Some(Play::Stay(Motion::Dead));
    }
    if !prev.alive && now.alive {
        return Some(Play::Rest);
    }
    if !now.alive {
        return None;
    }

    // 出手段结束：行动生效的那一刻。
    if let (Some(kind), None) = (prev.acting, now.acting) {
        return Some(match kind {
            ActKind::Attack => Play::Once(Motion::Attack),
            ActKind::Cast => Play::Once(Motion::Magic),
            ActKind::Item { hostile: false } => Play::Once(Motion::ItemRecover),
            ActKind::Item { hostile: true } => Play::Once(Motion::ItemAttack),
            ActKind::Escape => Play::Rest,
        });
    }
    // 进入出手段：仙术先摆预备架势，逃跑开始跑。
    if let (None, Some(kind)) = (prev.acting, now.acting) {
        match kind {
            ActKind::Cast => return Some(Play::Loop(Motion::MagicWait)),
            ActKind::Escape => return Some(Play::Loop(Motion::Flee)),
            _ => {}
        }
    }

    // 挨打。
    if now.hp < prev.hp {
        return Some(Play::Once(if now.defending {
            Motion::DefenceBeAttacked
        } else {
            Motion::BeAttacked
        }));
    }

    // 摆出 / 收起防御，睡着 / 醒来。
    if now.defending && !prev.defending {
        return Some(Play::Once(Motion::StartDefence));
    }
    if (prev.defending && !now.defending) || (prev.asleep != now.asleep) {
        return Some(Play::Rest);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standing() -> Snapshot {
        Snapshot {
            hp: 100,
            alive: true,
            ..Default::default()
        }
    }

    #[test]
    fn the_table_is_the_one_the_tasks_index() {
        // 各任务写进 +0x19a8 的立即数 → 名字（地址见 combat_motion.md）。
        for (index, name) in [
            (29, "z01"),
            (31, "z03"),
            (32, "z04"),
            (33, "z05"),
            (35, "z06"),
            (36, "z09"),
            (37, "z10"),
            (38, "z11"),
            (39, "z12"),
            (40, "z13"),
            (41, "z14"),
            (42, "z141"),
            (44, "z15"),
            (48, "z19"),
            (49, "z20"),
            (50, "z1"),
            (52, "z3"),
            (53, "g"),
            (54, "b"),
            (55, "x1"),
            (56, "x2"),
            (57, "x3"),
        ] {
            assert_eq!(ACTION_NAMES[index], name, "#{index}");
        }
        // FUN_0050b360 给我方预载的十个。
        for name in [
            "z01", "z09", "z06", "z03", "z04", "z05", "z15", "z17", "z18", "z20",
        ] {
            assert!(ACTION_NAMES[29..50].contains(&name), "{name}");
        }
    }

    #[test]
    fn the_two_sets_start_where_the_stage_init_puts_them() {
        // 0x50984c / 0x509858：我方 0x20，怪物 0x32。
        assert_eq!(MotionSet::Party.index(Motion::Idle), Some(0x20));
        assert_eq!(MotionSet::Monster.index(Motion::Idle), Some(0x32));
        assert_eq!(MotionSet::Party.name(Motion::Idle), Some("z04"));
        assert_eq!(MotionSet::Monster.name(Motion::Idle), Some("z1"));
        // 怪物没有防御、道具、倒下这些。
        assert_eq!(MotionSet::Monster.name(Motion::Defence), None);
        assert_eq!(MotionSet::Monster.name(Motion::Dead), None);
        assert_eq!(MotionSet::Monster.name(Motion::Attack), Some("g"));
        assert_eq!(MotionSet::Monster.name(Motion::BeAttacked), Some("b"));
        // 我方每一件都有名字，而且都在我方那一段里。
        for m in [
            Motion::Idle,
            Motion::Attack,
            Motion::BeAttacked,
            Motion::MagicWait,
            Motion::Magic,
            Motion::ItemRecover,
            Motion::ItemAttack,
            Motion::StartDefence,
            Motion::Defence,
            Motion::DefenceBeAttacked,
            Motion::Freeze,
            Motion::Dead,
            Motion::Flee,
        ] {
            let i = MotionSet::Party.index(m).unwrap();
            assert!((29..50).contains(&i), "{m:?} → {i}");
        }
    }

    #[test]
    fn an_attack_lands_when_the_cast_gauge_runs_out() {
        let mut acting = standing();
        acting.acting = Some(ActKind::Attack);
        // 进出手段时攻击不摆架势（原版此时是往目标跟前走，没做）。
        assert_eq!(react(&standing(), &acting), None);
        // 出手段结束 → 出手。
        assert_eq!(
            react(&acting, &standing()),
            Some(Play::Once(Motion::Attack))
        );
    }

    #[test]
    fn a_spell_waits_then_fires() {
        let mut casting = standing();
        casting.acting = Some(ActKind::Cast);
        assert_eq!(
            react(&standing(), &casting),
            Some(Play::Loop(Motion::MagicWait))
        );
        assert_eq!(
            react(&casting, &standing()),
            Some(Play::Once(Motion::Magic))
        );
    }

    #[test]
    fn items_split_by_which_side_they_hit() {
        let mut healing = standing();
        healing.acting = Some(ActKind::Item { hostile: false });
        assert_eq!(
            react(&healing, &standing()),
            Some(Play::Once(Motion::ItemRecover))
        );
        let mut throwing = standing();
        throwing.acting = Some(ActKind::Item { hostile: true });
        assert_eq!(
            react(&throwing, &standing()),
            Some(Play::Once(Motion::ItemAttack))
        );
    }

    #[test]
    fn losing_hp_flinches_and_a_guard_changes_the_flinch() {
        let mut hurt = standing();
        hurt.hp = 60;
        assert_eq!(
            react(&standing(), &hurt),
            Some(Play::Once(Motion::BeAttacked))
        );
        let mut guard = standing();
        guard.defending = true;
        let mut guard_hurt = guard;
        guard_hurt.hp = 80;
        assert_eq!(
            react(&guard, &guard_hurt),
            Some(Play::Once(Motion::DefenceBeAttacked))
        );
        // 回血不算挨打。
        assert_eq!(react(&hurt, &standing()), None);
    }

    #[test]
    fn falling_overrides_everything_and_stays_down() {
        let mut dead = standing();
        dead.hp = 0;
        dead.alive = false;
        assert_eq!(react(&standing(), &dead), Some(Play::Stay(Motion::Dead)));
        // 倒着的时候什么都不换。
        assert_eq!(react(&dead, &dead), None);
        // 被救起来回到歇着的样子。
        assert_eq!(react(&dead, &standing()), Some(Play::Rest));
    }

    #[test]
    fn guarding_and_sleeping_change_the_resting_pose() {
        let mut guard = standing();
        guard.defending = true;
        assert_eq!(
            react(&standing(), &guard),
            Some(Play::Once(Motion::StartDefence))
        );
        assert_eq!(rest_motion(&guard), Motion::Defence);
        assert_eq!(react(&guard, &standing()), Some(Play::Rest));

        let mut asleep = standing();
        asleep.asleep = true;
        asleep.defending = true;
        assert_eq!(rest_motion(&asleep), Motion::Freeze, "睡着压过防御");
        assert_eq!(
            react(&standing(), &asleep),
            Some(Play::Once(Motion::StartDefence))
        );
        assert_eq!(react(&asleep, &standing()), Some(Play::Rest));
    }

    #[test]
    fn running_away_loops_until_the_attempt_resolves() {
        let mut fleeing = standing();
        fleeing.acting = Some(ActKind::Escape);
        assert_eq!(react(&standing(), &fleeing), Some(Play::Loop(Motion::Flee)));
        assert_eq!(react(&fleeing, &standing()), Some(Play::Rest));
    }
}
