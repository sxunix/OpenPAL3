//! 25 队伍散开 · 26 队伍合并 · 211 进情节关散开 · 212 出情节关合并.
//!
//! Both walk the five party slots and place every active member that is
//! not the leader, taking the next entry of a five-row offset table as
//! they go. The table lives at `0xc042f8` and is written literally at
//! 0x4064fe onwards:
//!
//! | slot | x | y | z |
//! |---|---|---|---|
//! | 0 |  21.59 | 0 |  -37.31 |
//! | 1 | -42.37 | 0 |  -32.08 |
//! | 2 | -25.69 | 0 |  -73.67 |
//! | 3 |  28.96 | 0 |  -82.99 |
//! | 4 |  28.96 | 0 | -122.99 |
//!
//! `FUN_0040de10` (25) uses the table as it stands, `FUN_0040e110` (211)
//! multiplies it by **−3** and `FUN_0040e3c0` (212) by **+2**, then each
//! builds the offset out of the leader's own axes and
//! adds it to the leader's position. Every row's z is negative and 211's
//! multiplier is negative, so members end up **behind** the leader — the
//! basis at `role+0x14` is the backward direction, not the facing one.
//! For scale, 景天 is 76 world units tall, so the spread runs from about
//! one to five body-heights out.
//!
//! 全部四条都是「先摆起点、再走到终点、脚本等走完」——原版每个成员都
//! 挂一个 `FUN_00434c40` 等待任务。走法取 `+0x504` 那个内部值
//! （见 `openpal3::states::role_move`）：
//!
//! | opcode | 起点 | 终点 | 走法 |
//! |---|---|---|---|
//! | 25 | 主角 + 局部 −Z × 12 | 主角 + 表 × 1 | 走 |
//! | 26 | 原地 | 主角位置 | 走 |
//! | 211 | 主角 + 局部 −Z × 12 | 主角 + 表 × (−3) | 走 |
//! | 212 | 瞬移到 主角 + 表 × 2 | 主角位置 | **跑** |
//!
//! 25/211 走 `FUN_00416fa0`、26 走 `FUN_00416fe0`，两者都以
//! `FUN_00412b60(目标, 1)` 收尾——内部值 1 就是「走」。212 走
//! `FUN_004198c0(主角位置, 2)`，那个 2 直接写进 `+0x504`，是「跑」。
//!
//! 这几条都不管成员是不是激活——那是 opcode 28 的事，所以只有脚本先
//! 激活过才看得见。

use crate::openpal3::directors::SceneManagerExtensions;
use crate::openpal3::scene::RoleController;
use crate::openpal3::states::role_move::{MIN_MOVE_DISTANCE, MoveMode};
use crate::scripting::sce::commands::role_move_to::move_speed;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::{IEntityExt, ISceneManager};
use radiance::math::{Transform, Vec3};

/// 25 和 211 把成员先挪到这个偏移上再让他们走出去
/// （`local_54 = 主角[+0x14] × -12.0`，和队形表用的是同一个基）。
const NUDGE: f32 = -12.0;

/// `DAT_00c042f8`, five rows of three floats.
const FORMATION: [[f32; 3]; 5] = [
    [21.590, 0., -37.310],
    [-42.370, 0., -32.080],
    [-25.690, 0., -73.670],
    [28.960, 0., -82.990],
    [28.960, 0., -122.990],
];

/// 211's multiplier — `fmul dword ptr [0x56a780]` at 0x40e24b, and that
/// constant is −3.0.
const SPREAD: f32 = -3.0;

/// 212's, which the decompiler shows as `x + x`.
const GATHER: f32 = 2.0;

/// 25 uses the table unscaled — `FUN_0040de10` multiplies each basis by
/// `local_5c[-2]` and friends directly, with no constant. The rows are
/// 43 to 126 units out, well under 景天's 76-unit height at the near end,
/// so this is the tight fan the party opens into rather than 211's spread.
const OPEN: f32 = 1.0;

/// The five party slots the original walks (`&DAT_00c05f60`, stride
/// 0x2200, up to 0xc10960). In yaobow these are the `ROLE_0`..`ROLE_4`
/// entities the scene always builds.
const PARTY_SLOTS: [i32; 5] = [0, 1, 2, 3, 4];

/// 一次队形移动：每个成员一个目标点，走到为止。
#[derive(Debug, Clone, Default)]
struct FormationWalk {
    /// (角色号, 目标点)。
    targets: Vec<(i32, Vec3)>,
    mode: Option<MoveMode>,
}

impl FormationWalk {
    fn is_started(&self) -> bool {
        self.mode.is_some()
    }

    /// 每帧把还没到位的成员往各自目标推一步；全部到位返回 `true`。
    fn step(
        &mut self,
        scene_manager: &ComRc<ISceneManager>,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        let Some(mode) = self.mode else {
            return true;
        };
        if state.fast_forward() {
            for (slot, target) in &self.targets {
                if let Some(m) = scene_manager.get_resolved_role(state, *slot) {
                    m.transform().borrow_mut().set_position(target);
                }
            }
            self.targets.clear();
        }

        let mut done = true;
        for (slot, target) in self.targets.clone() {
            let Some(member) = scene_manager.get_resolved_role(state, slot) else {
                continue;
            };
            let position = member.transform().borrow().position();
            let remain = Vec3::sub(&target, &position);
            let speed = move_speed(state, slot, mode);
            let step = speed * delta_sec;

            if remain.norm() < MIN_MOVE_DISTANCE || remain.norm() < step {
                member.transform().borrow_mut().set_position(&target);
                if let Some(rc) = RoleController::get_role_controller(member) {
                    rc.inner::<RoleController>().idle();
                }
                continue;
            }
            done = false;
            let next = Vec3::add(
                &position,
                &Vec3::scalar_mul(step, &Vec3::normalized(&remain)),
            );
            let look_at = Vec3::new(target.x, position.y, target.z);
            member
                .transform()
                .borrow_mut()
                .look_at(&look_at)
                .set_position(&next);
        }
        done
    }
}

/// 25 / 211 / 212 共用：按队形表算每个成员的落点。`scale` 是那三个乘数。
fn formation_targets(
    scene_manager: &ComRc<ISceneManager>,
    state: &mut SceState,
    scale: f32,
) -> Vec<(i32, Vec3)> {
    let mut out = vec![];
    let Some(leader) = scene_manager.get_resolved_role(state, -1) else {
        return out;
    };
    let leader_name = leader.name();
    let leader_matrix = *leader.transform().borrow().matrix();

    let mut row = 0usize;
    for slot in PARTY_SLOTS {
        let Some(member) = scene_manager.get_resolved_role(state, slot) else {
            continue;
        };
        if member.name() == leader_name {
            continue;
        }
        let Some(offset) = FORMATION.get(row) else {
            break;
        };
        row += 1;

        // The offset is in the leader's own frame — `right · x + up · y
        // + basis_z · z` in the original, which is a local translation.
        // `basis_z` (the original's `role+0x14`) is the **backward**
        // direction, not the facing one: the table's z is negative and
        // the multiplier is -3, so a member lands behind the leader,
        // which is where Pal3.Unity independently puts them
        // (`-playerActorTransform.Forward` in `CalculateSpawnPosition`).
        // radiance agrees: `Transform::look_at` writes
        // `normalize(position - target)` into the local Z column, so
        // local +Z already points away from what the actor faces.
        let mut transform = Transform::from_matrix(leader_matrix);
        transform.translate_local(&Vec3::new(
            offset[0] * scale,
            offset[1] * scale,
            offset[2] * scale,
        ));
        out.push((slot, transform.position()));
    }
    out
}

/// 25 / 211 起步：先把成员挪到主角局部 −Z 方向 12 单位处。
fn nudge_to_start(scene_manager: &ComRc<ISceneManager>, state: &mut SceState) {
    let Some(leader) = scene_manager.get_resolved_role(state, -1) else {
        return;
    };
    let leader_name = leader.name();
    let leader_matrix = *leader.transform().borrow().matrix();
    let mut transform = Transform::from_matrix(leader_matrix);
    transform.translate_local(&Vec3::new(0., 0., NUDGE));
    let start = transform.position();

    for slot in PARTY_SLOTS {
        let Some(member) = scene_manager.get_resolved_role(state, slot) else {
            continue;
        };
        if member.name() == leader_name {
            continue;
        }
        member.transform().borrow_mut().set_position(&start);
    }
}

/// 26 / 212 的终点：主角脚下。
fn leader_targets(scene_manager: &ComRc<ISceneManager>, state: &mut SceState) -> Vec<(i32, Vec3)> {
    let mut out = vec![];
    let Some(leader) = scene_manager.get_resolved_role(state, -1) else {
        return out;
    };
    let leader_name = leader.name();
    let position = leader.transform().borrow().position();
    for slot in PARTY_SLOTS {
        let Some(member) = scene_manager.get_resolved_role(state, slot) else {
            continue;
        };
        if member.name() == leader_name {
            continue;
        }
        out.push((slot, position));
    }
    out
}

/// 212 起步：把成员摆到两倍队形偏移处（原版是直接写坐标，不是走过去的）。
fn place_at(scene_manager: &ComRc<ISceneManager>, state: &mut SceState, scale: f32) {
    for (slot, position) in formation_targets(scene_manager, state, scale) {
        if let Some(member) = scene_manager.get_resolved_role(state, slot) {
            member.transform().borrow_mut().set_position(&position);
        }
    }
}

/// 25 玩家队伍散开 — `LAB_004365a0` → `FUN_0040de10(1)`：先把成员挪到
/// 主角局部 −Z 12 单位处，再让他们**走**到队形表原样的落点，脚本等走完。
#[derive(Debug, Clone, Default)]
pub struct SceCommandTeamOpen {
    walk: FormationWalk,
}

impl SceCommand for SceCommandTeamOpen {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if !self.walk.is_started() {
            nudge_to_start(&scene_manager, state);
            self.walk.targets = formation_targets(&scene_manager, state, OPEN);
            self.walk.mode = Some(MoveMode::Walk);
        }
        self.walk.step(&scene_manager, state, delta_sec)
    }
}

impl SceCommandTeamOpen {
    pub fn new() -> Self {
        Self::default()
    }
}
/// 26 玩家队伍合并 — `LAB_004365b0` → `FUN_0040e0a0(1)`：每个成员原地
/// 起步，**走**向主角当前位置，脚本等走完。
#[derive(Debug, Clone, Default)]
pub struct SceCommandTeamClose {
    walk: FormationWalk,
}

impl SceCommand for SceCommandTeamClose {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if !self.walk.is_started() {
            self.walk.targets = leader_targets(&scene_manager, state);
            self.walk.mode = Some(MoveMode::Walk);
        }
        self.walk.step(&scene_manager, state, delta_sec)
    }
}

impl SceCommandTeamClose {
    pub fn new() -> Self {
        Self::default()
    }
}
/// 211 进情节关散开 — `FUN_0040e110(1)`：和 25 一样先挪起点，只是队形表
/// 乘 −3，成员落到主角另一侧。
#[derive(Debug, Clone, Default)]
pub struct SceCommandTeamOpenOnSceneChange {
    walk: FormationWalk,
}

impl SceCommand for SceCommandTeamOpenOnSceneChange {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if !self.walk.is_started() {
            nudge_to_start(&scene_manager, state);
            self.walk.targets = formation_targets(&scene_manager, state, SPREAD);
            self.walk.mode = Some(MoveMode::Walk);
        }
        self.walk.step(&scene_manager, state, delta_sec)
    }
}

impl SceCommandTeamOpenOnSceneChange {
    pub fn new() -> Self {
        Self::default()
    }
}
/// 212 出情节关合并 — `FUN_0040e3c0(1)`：先把成员**瞬移**到两倍队形偏移
/// 处，再让他们**跑**回主角身边（`FUN_004198c0(主角位置, 2)`，那个 2 直接
/// 写进 `+0x504`）。
#[derive(Debug, Clone, Default)]
pub struct SceCommandTeamCloseOnSceneChange {
    walk: FormationWalk,
}

impl SceCommand for SceCommandTeamCloseOnSceneChange {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if !self.walk.is_started() {
            place_at(&scene_manager, state, GATHER);
            self.walk.targets = leader_targets(&scene_manager, state);
            self.walk.mode = Some(MoveMode::Run);
        }
        self.walk.step(&scene_manager, state, delta_sec)
    }
}

impl SceCommandTeamCloseOnSceneChange {
    pub fn new() -> Self {
        Self::default()
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_the_one_written_at_0x4064fe() {
        // 0x41acb852 = 21.59, 0xc2153d70 = -37.31, and so on.
        assert_eq!(FORMATION.len(), 5);
        assert!((FORMATION[0][0] - 21.590).abs() < 1e-3);
        assert!((FORMATION[0][2] + 37.310).abs() < 1e-3);
        assert!((FORMATION[4][2] + 122.990).abs() < 1e-3);
        // Every row is level with the leader, which is why the `up` term
        // of the original's basis sum never contributes.
        assert!(FORMATION.iter().all(|row| row[1] == 0.));
    }

    #[test]
    fn a_positive_local_z_offset_is_behind_the_actor() {
        // The whole formation hinges on this: the original composes its
        // offsets against a backward-pointing basis, so mapping them to
        // a local translation is only right while radiance's local +Z
        // points away from what an actor faces.
        let mut t = Transform::new();
        t.set_position(&Vec3::new(0., 0., 0.))
            .look_at(&Vec3::new(0., 0., -100.));
        t.translate_local(&Vec3::new(0., 0., 10.));
        let p = t.position();
        assert!(p.z > 9.9, "局部 +Z 应当指向身后，实得 {p:?}");
        assert!(p.x.abs() < 1e-3 && p.y.abs() < 1e-3);
    }

    #[test]
    fn the_start_nudge_is_twelve_units_along_the_same_basis() {
        // `local_54 = 主角[+0x14] × -12.0`，和队形表共用一个基。
        assert_eq!(NUDGE, -12.0);
        // 挪的这一下比最近的队形点还短，所以成员确实要往外走一段。
        let nearest = FORMATION
            .iter()
            .map(|r| (r[0] * r[0] + r[2] * r[2]).sqrt())
            .fold(f32::MAX, f32::min);
        assert!(
            NUDGE.abs() < nearest,
            "起点 {} 该在 {nearest} 之内",
            NUDGE.abs()
        );
    }

    #[test]
    fn a_fresh_walk_has_nothing_to_do() {
        // 没起步的 FormationWalk 不该假装自己还在走。
        let mut w = FormationWalk::default();
        assert!(!w.is_started());
        assert!(w.mode.is_none());
        w.mode = Some(MoveMode::Walk);
        assert!(w.is_started());
        // 目标表空 = 全部到位。
        assert!(w.targets.is_empty());
    }

    #[test]
    fn the_three_multipliers_scale_the_same_table_differently() {
        // 25 takes the table as it stands, 211 spreads by -3 and 212
        // gathers at +2. Only 211's is negative, which is what flips its
        // members to the other side of the leader.
        assert_eq!(OPEN, 1.0);
        assert_eq!(SPREAD, -3.0);
        assert_eq!(GATHER, 2.0);
        assert!(SPREAD < 0. && OPEN > 0. && GATHER > 0.);

        // 25's fan is tight: under two body heights at its widest.
        let widest = FORMATION
            .iter()
            .map(|r| ((r[0] * OPEN).powi(2) + (r[2] * OPEN).powi(2)).sqrt())
            .fold(0f32, f32::max);
        assert!(widest < 2. * 76.0, "{widest}");
    }

    #[test]
    fn spreading_pushes_members_between_one_and_five_body_heights_out() {
        // 景天 is 76 world units tall.
        for row in FORMATION {
            let d = ((row[0] * SPREAD).powi(2) + (row[2] * SPREAD).powi(2)).sqrt();
            assert!((76.0..=400.0).contains(&d), "{d}");
        }
    }
}
