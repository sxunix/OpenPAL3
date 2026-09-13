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
//! 时序照原版：出手段里原地摆 z061 等出手条满（`FUN_00514870` 调
//! `FUN_0050a8e0(4)`），条满后才排走位、出手、结算（任务 0x514ae0）。攻击
//! 走位照 `FUN_0050da90`（480/秒、停在离目标 42 处）。出手动作开始后隔多久
//! 结算是 **[推断]**（[`HIT_DELAY`]），原版由步骤函数按数据表的时间点触发，
//! 还没逆。演出由舞台驱动（`combat.rs`），这里只放规则和常量。逆向记录在
//! `switch/re/notes/combat_motion.md`。

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
    /// 特技（我方 z09，特技任务 `FUN_0050f080` 写 36）
    Special,
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
    /// 走位（攻击时去目标跟前、再回来）
    Move,
    /// 濒死（精不到四分之一）
    Dying,
    /// 胜利（打赢后每人排的任务，`FUN_0050fa20` 写 49）
    Win,
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
                Special => 36,
                ItemRecover => 38,
                ItemAttack => 39,
                StartDefence => 41,
                Defence => 42,
                DefenceBeAttacked => 43,
                Freeze => 45,
                Dead => 47,
                Flee => 48,
                Move => 31,
                Dying => 46,
                Win => 49,
            }),
            MotionSet::Monster => match motion {
                Idle => Some(50),
                Attack => Some(53),
                BeAttacked => Some(54),
                // 怪物那套没有特技动作，照仙术播 [推断]。
                Magic | Special => Some(55),
                Move => Some(52),
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
    /// 特技（行动码 0x104）。
    Special,
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
    pub max_hp: i32,
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
            max_hp: c.max_hp,
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

/// 精的状况（`FUN_004f3a50`）：1 满、2 半数以上、3 四分之一以上、4 不到
/// 四分之一、5 倒下（不到 1）。
pub fn hp_condition(hp: i32, max: i32) -> u8 {
    if hp == max {
        1
    } else if max / 2 <= hp && hp < max {
        2
    } else if max / 4 <= hp {
        3
    } else if hp < 1 {
        5
    } else {
        4
    }
}

/// 歇着的时候做什么——`FUN_0050bbd0`（任务表空了时的默认动作）的顺序：
/// 仙术预备（`+0x19a4 == 4`）> 防御（战斗位 `+0x474`）> 眠定（状态 0x11 睡、
/// 0x0e 封；封移植端没有）> 濒死（状况 4）> 待机。倒下（状况 5）另走
/// [`Play::Stay`]。
///
/// `+0x19a4 = 4` 是行动任务一进出手段就写的（`FUN_0050a8e0(4)`，0x51498f），
/// **不分行动种类**——攻击、道具也一样站成 z061。怪物那套没有这个名字，
/// 照旧站着。逃跑除外（它有自己的 z19）。
pub fn rest_motion(now: &Snapshot) -> Motion {
    if now.acting.is_some_and(|k| k != ActKind::Escape) {
        Motion::MagicWait
    } else if now.defending {
        Motion::Defence
    } else if now.asleep {
        Motion::Freeze
    } else if hp_condition(now.hp, now.max_hp) == 4 {
        Motion::Dying
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

    // 出手、结算那一段由舞台演（[`strike_motion`]），这里不管。
    // 进入出手段：摆预备架势；逃跑开始跑。
    if let (None, Some(kind)) = (prev.acting, now.acting) {
        return Some(match kind {
            ActKind::Escape => Play::Loop(Motion::Flee),
            _ => Play::Loop(Motion::MagicWait),
        });
    }
    // 逃跑结果出来了（没逃掉）：回去站着。
    if prev.acting == Some(ActKind::Escape) && now.acting.is_none() {
        return Some(Play::Rest);
    }

    // 挨打。
    if now.hp < prev.hp {
        return Some(Play::Once(if now.defending {
            Motion::DefenceBeAttacked
        } else {
            Motion::BeAttacked
        }));
    }

    // 摆出防御先做一遍「开始防御」。
    if now.defending && !prev.defending {
        return Some(Play::Once(Motion::StartDefence));
    }
    // 其余歇着的样子变了（收起防御、睡着 / 醒来、回血脱离濒死……）。出手段
    // 结束引起的变化不算：那时舞台正在演出手动作，不能打断。
    if prev.acting == now.acting && rest_motion(prev) != rest_motion(now) {
        return Some(Play::Rest);
    }
    None
}

/// 出手那一下做什么动作。攻击要先走过去（见 [`approach_point`]）。
pub fn strike_motion(kind: ActKind) -> Option<Motion> {
    match kind {
        ActKind::Attack => Some(Motion::Attack),
        ActKind::Cast => Some(Motion::Magic),
        ActKind::Special => Some(Motion::Special),
        ActKind::Item { hostile: false } => Some(Motion::ItemRecover),
        ActKind::Item { hostile: true } => Some(Motion::ItemAttack),
        ActKind::Escape => None,
    }
}

/// 出手动作开始后多久结算（对方挨打、掉血）**[推断]**：原版由步骤函数按
/// 数据表里的时间点触发（道具编排 `FUN_00500550` 读 `+0x9a8` 那一格），攻击
/// 那一格还没逆。
pub const HIT_DELAY: f32 = 0.3;

// ---------------------------------------------------------------- 走位

/// 走位速度（`FUN_0050da90`：方向 × 480，每帧乘帧时长）。
pub const WALK_SPEED: f32 = 480.;
/// 离终点不到这么远就算到了、直接对齐（同上，`< 4.0`）。
pub const ARRIVE_DISTANCE: f32 = 4.;
/// 停在离目标多远：先从目标朝自己挪 32，再朝自己挪 10。
pub const STOP_SHORT: f32 = 32. + 10.;

/// 去程终点：目标位置朝自己退 [`STOP_SHORT`]。两个人站在一处时原地不动。
pub fn approach_point(target: (f32, f32), me: (f32, f32)) -> (f32, f32) {
    let (dx, dz) = (me.0 - target.0, me.1 - target.1);
    let len = (dx * dx + dz * dz).sqrt();
    if len < f32::EPSILON {
        return me;
    }
    (
        target.0 + dx / len * STOP_SHORT,
        target.1 + dz / len * STOP_SHORT,
    )
}

/// 往 `to` 走一帧。返回新位置和到没到。原版还有一条「走过头（和出发时的
/// 朝向差到 90°）就当到了」，按匀速直线走不会发生，这里用步长盖过剩余
/// 距离来判。
pub fn step_toward(pos: (f32, f32), to: (f32, f32), dt: f32) -> ((f32, f32), bool) {
    let (dx, dz) = (to.0 - pos.0, to.1 - pos.1);
    let left = (dx * dx + dz * dz).sqrt();
    let step = WALK_SPEED * dt;
    if left < ARRIVE_DISTANCE || step >= left {
        return (to, true);
    }
    ((pos.0 + dx / left * step, pos.1 + dz / left * step), false)
}

/// 站好时面朝哪边（x, z）。舞台初始化 `FUN_005096d0`：我方 yaw 45°
/// （0x42340000），怪物那套 −135°（0xc3070000），偷袭时再加 180°（没做）。
/// 45° 按哪根轴量起不影响结果：两种量法都落在 (+x, +z) 这条对角线上，和
/// `[RolePos]` 里我方重心指向敌方重心的方向（42°~48°）一致。
pub fn rest_facing(set: MotionSet) -> (f32, f32) {
    let d = std::f32::consts::FRAC_1_SQRT_2;
    match set {
        MotionSet::Party => (d, d),
        MotionSet::Monster => (-d, -d),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standing() -> Snapshot {
        Snapshot {
            hp: 100,
            max_hp: 100,
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
            Motion::Move,
            Motion::Dying,
            Motion::Win,
        ] {
            let i = MotionSet::Party.index(m).unwrap();
            assert!((29..50).contains(&i), "{m:?} → {i}");
        }
    }

    #[test]
    fn every_action_waits_in_the_same_pose() {
        // FUN_0050a8e0(4) 不分种类：攻击、仙术、道具进出手段都摆 z061。
        for kind in [
            ActKind::Attack,
            ActKind::Cast,
            ActKind::Item { hostile: false },
            ActKind::Item { hostile: true },
        ] {
            let mut acting = standing();
            acting.acting = Some(kind);
            assert_eq!(
                react(&standing(), &acting),
                Some(Play::Loop(Motion::MagicWait))
            );
            assert_eq!(rest_motion(&acting), Motion::MagicWait);
            // 出手结算那一段归舞台，react 不插手。
            assert_eq!(react(&acting, &standing()), None);
        }
    }

    #[test]
    fn the_strike_motion_depends_on_the_action() {
        assert_eq!(strike_motion(ActKind::Attack), Some(Motion::Attack));
        assert_eq!(strike_motion(ActKind::Cast), Some(Motion::Magic));
        assert_eq!(
            strike_motion(ActKind::Item { hostile: false }),
            Some(Motion::ItemRecover)
        );
        assert_eq!(
            strike_motion(ActKind::Item { hostile: true }),
            Some(Motion::ItemAttack)
        );
        assert_eq!(strike_motion(ActKind::Escape), None);
        assert_eq!(MotionSet::Party.name(Motion::MagicWait), Some("z061"));
        assert_eq!(
            MotionSet::Monster.name(Motion::MagicWait),
            None,
            "怪物照旧站着"
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
        assert_eq!(
            rest_motion(&asleep),
            Motion::Defence,
            "FUN_0050bbd0 先查防御"
        );
        let mut only_asleep = standing();
        only_asleep.asleep = true;
        assert_eq!(rest_motion(&only_asleep), Motion::Freeze);
        assert_eq!(
            react(&standing(), &asleep),
            Some(Play::Once(Motion::StartDefence))
        );
        assert_eq!(react(&asleep, &standing()), Some(Play::Rest));
    }

    #[test]
    fn hp_condition_matches_the_quarter_and_half_marks() {
        assert_eq!(hp_condition(100, 100), 1);
        assert_eq!(hp_condition(99, 100), 2);
        assert_eq!(hp_condition(50, 100), 2);
        assert_eq!(hp_condition(49, 100), 3);
        assert_eq!(hp_condition(25, 100), 3);
        assert_eq!(hp_condition(24, 100), 4);
        assert_eq!(hp_condition(1, 100), 4);
        assert_eq!(hp_condition(0, 100), 5);
        // 88 精的景天：21 就算濒死（88/4 = 22）。
        assert_eq!(hp_condition(21, 88), 4);
        assert_eq!(hp_condition(22, 88), 3);
    }

    #[test]
    fn a_blow_that_leaves_under_a_quarter_rests_as_dying() {
        let mut low = standing();
        low.hp = 20;
        assert_eq!(rest_motion(&low), Motion::Dying);
        // 挨打那一下先做挨打，做完才轮到濒死（由舞台在回到待机时补上）。
        assert_eq!(
            react(&standing(), &low),
            Some(Play::Once(Motion::BeAttacked))
        );
        // 回血脱离濒死：换回待机。
        assert_eq!(react(&low, &standing()), Some(Play::Rest));
        // casting 压过濒死。
        let mut casting_low = low;
        casting_low.acting = Some(ActKind::Attack);
        assert_eq!(rest_motion(&casting_low), Motion::MagicWait);
    }

    #[test]
    fn the_walk_stops_42_short_and_arrives_within_4() {
        let dest = approach_point((0., 0.), (100., 0.));
        assert!((dest.0 - 42.).abs() < 1e-4 && dest.1.abs() < 1e-4);
        // 斜着来也是 42。
        let d = approach_point((10., 10.), (-90., -90.));
        let off = ((d.0 - 10.).powi(2) + (d.1 - 10.).powi(2)).sqrt();
        assert!((off - 42.).abs() < 1e-3);
        // 一帧 1/60 秒走 8。
        let (p, arrived) = step_toward((0., 0.), (100., 0.), 1. / 60.);
        assert!(!arrived && (p.0 - 8.).abs() < 1e-3);
        // 剩不到 4 直接到。
        assert_eq!(step_toward((97., 0.), (100., 0.), 0.), ((100., 0.), true));
        // 步子大过剩余距离也不冲过头。
        assert_eq!(step_toward((0., 0.), (5., 0.), 1.), ((5., 0.), true));
    }

    #[test]
    fn the_two_sides_face_each_other_along_the_diagonal() {
        let (p, m) = (
            rest_facing(MotionSet::Party),
            rest_facing(MotionSet::Monster),
        );
        assert!(
            (p.0 + m.0).abs() < 1e-6 && (p.1 + m.1).abs() < 1e-6,
            "正好相反"
        );
        assert!(p.0 > 0. && p.1 > 0., "我方朝 +x+z，敌方在那边");
    }

    #[test]
    fn running_away_loops_until_the_attempt_resolves() {
        let mut fleeing = standing();
        fleeing.acting = Some(ActKind::Escape);
        assert_eq!(react(&standing(), &fleeing), Some(Play::Loop(Motion::Flee)));
        assert_eq!(react(&fleeing, &standing()), Some(Play::Rest));
    }
}
