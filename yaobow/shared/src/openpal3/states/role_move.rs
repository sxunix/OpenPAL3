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

    /// 原版动作名表（`(&PTR_DAT_00570904)[id]`）里对应的那一条。
    /// 走和跑是**同一个** `z3`，靠动画倍速区分。
    pub fn action_name(self) -> &'static str {
        match self {
            MoveMode::Walk | MoveMode::Run => "z3",
            MoveMode::Still => "z1",
        }
    }

    /// 动画播放倍速（`FUN_00412940` 末尾：内部值 2 给 2.4，其余给 1.0）。
    pub fn anim_scale(self) -> f32 {
        match self {
            MoveMode::Run => 2.4,
            _ => 1.0,
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
    fn walking_and_running_share_one_action() {
        // 动作名表里 52 和 58 都是 z3，区分只在倍速上。
        assert_eq!(MoveMode::Walk.action_name(), "z3");
        assert_eq!(MoveMode::Run.action_name(), "z3");
        assert_eq!(MoveMode::Still.action_name(), "z1");
        assert_eq!(MoveMode::Walk.anim_scale(), 1.0);
        assert_eq!(MoveMode::Run.anim_scale(), 2.4);
        assert_eq!(MoveMode::Still.anim_scale(), 1.0);
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
