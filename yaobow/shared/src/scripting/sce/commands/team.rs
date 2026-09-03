//! 211 进入情节关时队伍散开 · 212 出情节关时队伍合并.
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
//! `FUN_0040e110` (211) multiplies it by **−3** and `FUN_0040e3c0` (212)
//! by **+2**, then builds the offset out of the leader's own axes:
//! `right · x + up · y + forward · z`, added to the leader's position.
//! For scale, 景天 is 76 world units tall, so the spread runs from about
//! one to five body-heights out.
//!
//! What the original then does, and this does not: 211 first drops the
//! member 12 units behind the leader and *walks* them out to the slot,
//! and 212 places them at the doubled offset and walks them **into** the
//! leader. Both wait for those walks. There is no follower pathing here,
//! so each member is simply put at the endpoint. Neither opcode touches
//! whether a member is active — that is opcode 28's job — so this only
//! shows if the script has activated them.

use crate::openpal3::directors::SceneManagerExtensions;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::{IEntityExt, ISceneManager};
use radiance::math::{Transform, Vec3};

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

/// The five party slots the original walks (`&DAT_00c05f60`, stride
/// 0x2200, up to 0xc10960). In yaobow these are the `ROLE_0`..`ROLE_4`
/// entities the scene always builds.
const PARTY_SLOTS: [i32; 5] = [0, 1, 2, 3, 4];

fn place(scene_manager: &ComRc<ISceneManager>, state: &mut SceState, scale: f32) {
    let Some(leader) = scene_manager.get_resolved_role(state, -1) else {
        return;
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
        // + forward · z` in the original, which is a local translation.
        let mut transform = Transform::from_matrix(leader_matrix);
        transform.translate_local(&Vec3::new(
            offset[0] * scale,
            offset[1] * scale,
            offset[2] * scale,
        ));
        let position = transform.position();
        member.transform().borrow_mut().set_position(&position);
    }
}

#[derive(Debug, Clone)]
pub struct SceCommandTeamOpenOnSceneChange {}

impl SceCommand for SceCommandTeamOpenOnSceneChange {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        place(&scene_manager, state, SPREAD);
        true
    }
}

impl SceCommandTeamOpenOnSceneChange {
    pub fn new() -> Self {
        Self {}
    }
}

#[derive(Debug, Clone)]
pub struct SceCommandTeamCloseOnSceneChange {}

impl SceCommand for SceCommandTeamCloseOnSceneChange {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        // The original puts each member at twice the offset and then
        // walks them into the leader; with no pathing the endpoint is the
        // leader's own position.
        let Some(leader) = scene_manager.get_resolved_role(state, -1) else {
            return true;
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
            member.transform().borrow_mut().set_position(&position);
        }
        let _ = GATHER;
        true
    }
}

impl SceCommandTeamCloseOnSceneChange {
    pub fn new() -> Self {
        Self {}
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
    fn spreading_pushes_members_between_one_and_five_body_heights_out() {
        // 景天 is 76 world units tall.
        for row in FORMATION {
            let d = ((row[0] * SPREAD).powi(2) + (row[2] * SPREAD).powi(2)).sqrt();
            assert!((76.0..=400.0).contains(&d), "{d}");
        }
    }
}
