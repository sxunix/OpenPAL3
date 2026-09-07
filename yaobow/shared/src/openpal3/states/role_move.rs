//! 脚本驱动的角色移动：走多快、播什么动作。
//!
//! 两条命令共用这一套——opcode **20** `ActorPathTo`（`FUN_00434f90`）和
//! opcode **214** `ActorMoveTo`（`FUN_00435010`）。两者的第四个参数都过
//! `FUN_00434f60` 映射，再进 `FUN_00412b60` → `FUN_00412710`。
//!
//! 速度常数不在 exe 里，是启动时从 `datascript\scene.txt` 读的
//! （`FUN_0041efa0`，名字表在 0x58b32c 起、步长 0x14，**值在名字前面四个
//! 字节**）。逆向记录在 `switch/re/notes/role_move.md`。

/// 脚本第四个参数的三种取值。原版 `FUN_00434f60`：0（和其他值）→ 1、
/// 1 → 2、2 → 3；再由 `FUN_004126d0` 换成动作号，`FUN_00412710` 换成速度。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveMode {
    /// 参数 0：走。动作 `z3`，动画原速。
    Walk,
    /// 参数 1：跑。动作还是 `z3`，但动画放 **2.4 倍速**
    /// （`FUN_00412940` 末尾 `FUN_00536d00(2.4)`），速度取 `PRunSpeed`。
    Run,
    /// 参数 2：保持站立姿势移动。动作 `z1`，动画原速，速度同走。
    Still,
}

impl MoveMode {
    /// 照 `FUN_00434f60` 映射：只有 1 和 2 有专门的含义，其余都当 0。
    pub fn from_param(param: i32) -> Self {
        match param {
            1 => MoveMode::Run,
            2 => MoveMode::Still,
            _ => MoveMode::Walk,
        }
    }

    /// 播哪条动画，按优先级排——挑第一个这个模型真有的。
    ///
    /// 见 [`ActionSet`]。三套的差别：
    ///
    /// - `Cee`（主角）：内部值直接当动作号查表，1 → `c02`、2 → `c03`、
    ///   3 → `c15`；没有 `c03` 的退回 `c02`。
    /// - `Zed`（NPC）：经 `FUN_004126d0` 映射，走和跑落到同一个 `z3`
    ///   ——Pal3.Unity 那边这一条的注释就写着「跑,用走代替」。
    /// - `Monster`：走路另有 `z4`，没有才退回 `z3`。
    pub fn action_names(self, set: ActionSet) -> &'static [&'static str] {
        match set {
            ActionSet::Cee => match self {
                MoveMode::Walk => &["c02"],
                MoveMode::Run => &["c03", "c02"],
                MoveMode::Still => &["c15"],
            },
            ActionSet::Zed => match self {
                MoveMode::Walk | MoveMode::Run => &["z3"],
                MoveMode::Still => &["z1"],
            },
            ActionSet::Monster => match self {
                MoveMode::Walk | MoveMode::Run => &["z4", "z3"],
                MoveMode::Still => &["z0", "z1"],
            },
        }
    }

    /// 首选的那一条。
    pub fn action_name(self, set: ActionSet) -> &'static str {
        self.action_names(set)[0]
    }

    /// 动画播放倍速。
    ///
    /// z 系列走和跑是同一条 `z3`，只能靠倍速区分——`FUN_00412940` 末尾
    /// `+0x504 == 2` 给 2.4、其余给 1.0。c 系列走 `FUN_00412a14` 那条路，
    /// 跑有自己的 `c03`，倍速**恒为 1.0**。
    pub fn anim_scale(self, set: ActionSet) -> f32 {
        match (set, self) {
            (ActionSet::Zed | ActionSet::Monster, MoveMode::Run) => 2.4,
            _ => 1.0,
        }
    }
}

/// 角色用哪一套动作命名。原版在 `FUN_00412940` 开头按 `+0xae` 分岔：
///
/// ```text
/// if (+0xae == '1') { FUN_00412a14(内部值); return; }   // 直接当动作号
/// iVar1 = FUN_004126d0(内部值, +0xad);                  // 映射到 z 系列
/// ```
///
/// 动作名表 `(&PTR_DAT_00570904)[id]` 前几项是
/// `c01 c02 c03 c15 c04 c05 …`，所以走 `FUN_00412a14` 的角色，内部值
/// 1/2/3 直接落在 `c02`（走）/ `c03`（跑）/ `c15` 上，各有各的动画；
/// `FUN_004126d0` 那条只会返回 `0x32`/`0x34`/`0x3a`，也就是 `z1`/`z3`/`z3`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionSet {
    /// `+0xae == '1'`：主角那一类，有完整的 `c` 系列动画。
    Cee,
    /// 普通 NPC，只有 `z` 系列。
    Zed,
    /// 怪物。也是 `z` 系列，但站立和行走另有 `z0` / `z4`。
    Monster,
}

impl ActionSet {
    /// 按模型名分类。
    ///
    /// 照 Pal3.Unity 的 `ActorBase`（`IsMainActor` = 名字以 `1` 开头、
    /// `IsMonsterActor` = 既不以 `1` 也不以 `2` 开头）。原版看的是角色对象
    /// 上的 `+0xae` / `+0xad`，但模型名本来就是按类型编号的，两者等价：
    /// 实机 `basedata/ROLE` 下 269 个角色里，`c01`/`c02`/`c03` 只有 10 个
    /// 角色有（101–110），`z0`/`z4` 全在 3xx 和 Bxx。
    pub fn of(model_name: &str) -> Self {
        match model_name.chars().next() {
            Some('1') => ActionSet::Cee,
            Some('2') => ActionSet::Zed,
            _ => ActionSet::Monster,
        }
    }

    /// 站着不动播什么，按优先级排。挑第一个这个模型真有的。
    pub fn idle_names(self) -> &'static [&'static str] {
        match self {
            ActionSet::Cee => &["c01"],
            ActionSet::Zed => &["z1", "z2"],
            ActionSet::Monster => &["z0", "z1", "z2"],
        }
    }
}

/// `datascript\scene.txt` 的 `role` 一节。原版启动时读进 0x58b378 起那串槽位。
///
/// 出厂值（连注释一起抄在这儿）：
///
/// ```text
/// TalkRange    40
/// WalkSpeed    80     ;NPC走的速度
/// RunSpeed     180    ;NPC跑的速度
/// WalkForce    800    ;NPC走的加速度
/// RunForce     1200   ;NPC跑的加速度
/// PWalkSpeed   100    ;主角走的速度
/// PRunSpeed    200    ;主角跑的速度
/// JumpHeight   64     ;跳高
/// JumpDist     160    ;跳远
/// GForce       392    ;重力加速度,9.8*40
/// ```
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoleConstants {
    pub talk_range: f32,
    pub walk_speed: f32,
    pub run_speed: f32,
    pub walk_force: f32,
    pub run_force: f32,
    pub player_walk_speed: f32,
    pub player_run_speed: f32,
    pub jump_height: f32,
    pub jump_dist: f32,
    pub g_force: f32,
}

impl Default for RoleConstants {
    /// 随游戏出厂的那一组，读不到文件时兜底。
    fn default() -> Self {
        Self {
            talk_range: 40.,
            walk_speed: 80.,
            run_speed: 180.,
            walk_force: 800.,
            run_force: 1200.,
            player_walk_speed: 100.,
            player_run_speed: 200.,
            jump_height: 64.,
            jump_dist: 160.,
            g_force: 392.,
        }
    }
}

impl RoleConstants {
    /// 解析 `datascript\scene.txt` 的 `role` 一节。
    ///
    /// 格式是 `Key<空白>$值&`，`;` 起是注释，`BEGIN,<节名>` 开头、`END` 结尾。
    /// 没写的键保持出厂值。
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        let mut in_section = false;
        for raw in text.lines() {
            let line = raw.split(';').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(rest) = line.strip_prefix("BEGIN,") {
                in_section = rest.trim().eq_ignore_ascii_case("role");
                continue;
            }
            if line.eq_ignore_ascii_case("END") {
                if in_section {
                    break;
                }
                continue;
            }
            if !in_section {
                continue;
            }
            let mut it = line.split_whitespace();
            let (Some(key), Some(value)) = (it.next(), it.next()) else {
                continue;
            };
            let Ok(v) = value.trim_matches(|c| c == '$' || c == '&').parse::<f32>() else {
                continue;
            };
            let slot = match key {
                "TalkRange" => &mut out.talk_range,
                "WalkSpeed" => &mut out.walk_speed,
                "RunSpeed" => &mut out.run_speed,
                "WalkForce" => &mut out.walk_force,
                "RunForce" => &mut out.run_force,
                "PWalkSpeed" => &mut out.player_walk_speed,
                "PRunSpeed" => &mut out.player_run_speed,
                "JumpHeight" => &mut out.jump_height,
                "JumpDist" => &mut out.jump_dist,
                "GForce" => &mut out.g_force,
                _ => continue,
            };
            *slot = v;
        }
        out
    }

    /// 这一次移动走多快。
    ///
    /// `FUN_00412710` 的 `switch (+0x504)`：
    ///
    /// - case 2（跑）无条件取 `PRunSpeed`——脚本让 NPC 跑也用主角的跑速。
    /// - case 1/3（走、站着挪）按角色类型 `+0xad` 分：
    ///   `-1`（主角）取 `PWalkSpeed`，其余 NPC 先看角色自带的 `+0x1fc`，
    ///   为 0 才取 `WalkSpeed`。怪物（`+0xad == 2`）另有
    ///   `AttackSpeedS/M/L` 三档，不走这条路。
    ///
    /// `own_speed` 就是那个 `+0x1fc`：角色自带的速度，0 表示没有。
    pub fn speed(&self, mode: MoveMode, is_player: bool, own_speed: f32) -> f32 {
        match mode {
            MoveMode::Run => self.player_run_speed,
            MoveMode::Walk | MoveMode::Still => {
                if is_player {
                    self.player_walk_speed
                } else if own_speed > 0. {
                    own_speed
                } else {
                    self.walk_speed
                }
            }
        }
    }
}

/// 目标离得比这还近就当作已经到了，一步都不迈。
///
/// `FUN_00412b60` 一进来就量一次距离：
///
/// ```text
/// fVar1 = gbVec3D::Length(目标 − 当前);
/// if (fVar1 < 12.0) { +0x4f4 = 0; return; }    // 连移动状态都不进
/// +0x4f4 = 4;                                   // 否则才开始走
/// ```
pub const MIN_MOVE_DISTANCE: f32 = 12.0;

/// 摇杆推力超过这个数就是跑，不到就是走。
///
/// **这一条来自 Pal3.Unity**（`PlayerGamePlayManager.Movement.cs`：
/// `movement.magnitude < 0.7f ? MovementMode.Walk : MovementMode.Run`），
/// 不是从 exe 逆出来的——玩家那条输入路径还没定位到，见
/// `switch/re/notes/role_move.md` §6。键盘按方向键推力恒为 1，所以键盘
/// 走路就是跑，和实机一致。
pub const RUN_THROW_THRESHOLD: f32 = 0.7;

/// 玩家这一帧是走还是跑。
pub fn player_mode(throw: f32) -> MoveMode {
    if throw < RUN_THROW_THRESHOLD {
        MoveMode::Walk
    } else {
        MoveMode::Run
    }
}

/// exe 里写死的路径。
pub const SCENE_TXT: &str = "/basedata/basedata/datascript/scene.txt";

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
BEGIN,role\t;角色常数,含NPC和主角\r
;NPC\r
TalkRange\t$40&\r
WalkSpeed\t$80&\t;NPC走的速度\r
RunSpeed\t$180&\t;NPC跑的速度\r
WalkForce\t$800&\t;NPC走的加速度\r
RunForce\t$1200&\t;NPC跑的加速度\r
\r
;Player\r
PWalkSpeed\t$100&\t;主角走的速度\r
PRunSpeed\t$200&\t;主角跑的速度\r
\r
JumpHeight\t$64&\r
JumpDist\t$160&\r
GForce\t\t$392&\t;重力加速度,9.8*40\r
END\t;endof角色常数\r
\r
BEGIN,monster\r
FreeSpeedS\t$30&\r
WalkSpeed\t$99999&\t;不该读到——这是别的节\r
END\r
";

    #[test]
    fn the_three_modes_come_from_the_fourth_parameter() {
        assert_eq!(MoveMode::from_param(0), MoveMode::Walk);
        assert_eq!(MoveMode::from_param(1), MoveMode::Run);
        assert_eq!(MoveMode::from_param(2), MoveMode::Still);
        // FUN_00434f60 把别的值都归到 0 那一支。
        assert_eq!(MoveMode::from_param(3), MoveMode::Walk);
        assert_eq!(MoveMode::from_param(-1), MoveMode::Walk);
        assert_eq!(MoveMode::from_param(99), MoveMode::Walk);
    }

    #[test]
    fn the_c_set_has_its_own_run_animation() {
        // 内部值直接当动作号：表的前四项是 c01 c02 c03 c15。
        assert_eq!(MoveMode::Walk.action_name(ActionSet::Cee), "c02");
        assert_eq!(MoveMode::Run.action_name(ActionSet::Cee), "c03");
        assert_eq!(MoveMode::Still.action_name(ActionSet::Cee), "c15");
        // 跑有自己的动画，就不需要加速播了。
        assert_eq!(MoveMode::Run.anim_scale(ActionSet::Cee), 1.0);
    }

    #[test]
    fn the_model_name_says_which_set_a_role_uses() {
        // 101–110 是主角，2xx 是普通 NPC，3xx / Bxx 是怪物。
        assert_eq!(ActionSet::of("101"), ActionSet::Cee);
        assert_eq!(ActionSet::of("110"), ActionSet::Cee);
        assert_eq!(ActionSet::of("201"), ActionSet::Zed);
        assert_eq!(ActionSet::of("299"), ActionSet::Zed);
        assert_eq!(ActionSet::of("335"), ActionSet::Monster);
        assert_eq!(ActionSet::of("B06"), ActionSet::Monster);
        assert_eq!(ActionSet::of("b38"), ActionSet::Monster);
        assert_eq!(ActionSet::of(""), ActionSet::Monster, "名字空的当怪物");
    }

    #[test]
    fn monsters_have_their_own_idle_and_walk() {
        assert_eq!(
            MoveMode::Walk.action_names(ActionSet::Monster),
            ["z4", "z3"]
        );
        assert_eq!(MoveMode::Run.action_names(ActionSet::Monster), ["z4", "z3"]);
        assert_eq!(ActionSet::Monster.idle_names(), ["z0", "z1", "z2"]);
        // 实机 19 个有 z4 的角色全都同时有 z3，所以退路一定走得通。
        assert_eq!(MoveMode::Walk.action_names(ActionSet::Monster)[1], "z3");
    }

    #[test]
    fn every_set_has_an_idle_fallback() {
        for set in [ActionSet::Cee, ActionSet::Zed, ActionSet::Monster] {
            assert!(!set.idle_names().is_empty());
        }
        assert_eq!(ActionSet::Cee.idle_names(), ["c01"]);
        assert_eq!(ActionSet::Zed.idle_names(), ["z1", "z2"]);
    }

    #[test]
    fn the_z_set_fakes_running_by_playing_walk_faster() {
        // FUN_004126d0 只返回 0x32/0x34/0x3a，也就是 z1/z3/z3。
        assert_eq!(MoveMode::Walk.action_name(ActionSet::Zed), "z3");
        assert_eq!(MoveMode::Run.action_name(ActionSet::Zed), "z3");
        assert_eq!(MoveMode::Still.action_name(ActionSet::Zed), "z1");
        // 走跑同一条，只能靠倍速分。
        assert_eq!(MoveMode::Walk.anim_scale(ActionSet::Zed), 1.0);
        assert_eq!(MoveMode::Run.anim_scale(ActionSet::Zed), 2.4);
        assert_eq!(MoveMode::Still.anim_scale(ActionSet::Zed), 1.0);
    }

    #[test]
    fn the_role_section_parses() {
        let c = RoleConstants::parse(SAMPLE);
        assert_eq!(c.talk_range, 40.);
        assert_eq!(c.walk_speed, 80.);
        assert_eq!(c.run_speed, 180.);
        assert_eq!(c.walk_force, 800.);
        assert_eq!(c.run_force, 1200.);
        assert_eq!(c.player_walk_speed, 100.);
        assert_eq!(c.player_run_speed, 200.);
        assert_eq!(c.jump_height, 64.);
        assert_eq!(c.jump_dist, 160.);
        assert_eq!(c.g_force, 392.);
    }

    #[test]
    fn the_monster_section_is_not_read() {
        let c = RoleConstants::parse(SAMPLE);
        assert_eq!(c.walk_speed, 80., "monster 节里的同名键不能覆盖 role 节");
    }

    #[test]
    fn an_empty_file_keeps_the_shipped_numbers() {
        assert_eq!(RoleConstants::parse(""), RoleConstants::default());
        assert_eq!(
            RoleConstants::parse(";只有注释\n"),
            RoleConstants::default()
        );
    }

    #[test]
    fn a_full_throw_runs_and_a_light_one_walks() {
        assert_eq!(player_mode(1.0), MoveMode::Run, "键盘按下推力就是 1");
        assert_eq!(player_mode(0.7), MoveMode::Run, "正好到阈值算跑");
        assert_eq!(player_mode(0.69), MoveMode::Walk);
        assert_eq!(player_mode(0.2), MoveMode::Walk, "轻推摇杆是走");

        // 玩家跑 200、走 100，都从 scene.txt 来。
        let c = RoleConstants::default();
        assert_eq!(c.speed(player_mode(1.0), true, 0.), 200.);
        assert_eq!(c.speed(player_mode(0.3), true, 0.), 100.);
    }

    #[test]
    fn a_target_within_twelve_units_is_already_reached() {
        // FUN_00412b60 的第一道闸：比这近就不进移动状态。
        assert_eq!(MIN_MOVE_DISTANCE, 12.0);
    }

    #[test]
    fn running_always_uses_the_player_speed() {
        let c = RoleConstants::default();
        // 跑那一支无条件取 PRunSpeed，NPC 也一样。
        assert_eq!(c.speed(MoveMode::Run, true, 0.), 200.);
        assert_eq!(c.speed(MoveMode::Run, false, 0.), 200.);
        assert_eq!(c.speed(MoveMode::Run, false, 55.), 200., "自带速度不参与跑");
    }

    #[test]
    fn walking_splits_player_from_npc() {
        let c = RoleConstants::default();
        assert_eq!(c.speed(MoveMode::Walk, true, 0.), 100., "主角 PWalkSpeed");
        assert_eq!(c.speed(MoveMode::Walk, false, 0.), 80., "NPC WalkSpeed");
        assert_eq!(
            c.speed(MoveMode::Walk, false, 55.),
            55.,
            "角色自带的速度优先于 WalkSpeed"
        );
        // 站着挪和走同速。
        assert_eq!(
            c.speed(MoveMode::Still, true, 0.),
            c.speed(MoveMode::Walk, true, 0.)
        );
    }
}
