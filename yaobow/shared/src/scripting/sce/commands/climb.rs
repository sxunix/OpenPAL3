//! 74 攀爬场景物件 — `LAB_00437440`.
//!
//! The handler reads two parameters, resolves role `0xffff` (the actor the
//! player drives) and calls `FUN_00418dc0(object_id, up)`. That does the
//! whole setup:
//!
//! * look the object up and hang it on the role at `+0x584`; a missing
//!   object makes the call return false and the script carries straight on;
//! * take the object's yaw (`+0xe0`), add 180°, and put the actor
//!   `2 × 12.0` units away along that direction — which works out to
//!   `object + 24 · (sin yaw, 0, cos yaw)`, with the actor turned to face
//!   `yaw + 180°`, i.e. back at the ladder;
//! * measure the object and keep `min(0.6 × height, 100.0)` at `+0x4dc` as
//!   how far the climb travels — a **cap**, not a floor;
//! * refuse with the string `outdist` when the actor is further than
//!   `12.0 × 5.0` from that spot **and** the straight nav path to it is
//!   blocked;
//! * switch the role into state 8 with `+0x4d4` = the second parameter.
//!
//! State 8 (`FUN_004155xx`, `case 8:`) then plays action index 5 with
//! velocity `(0, +1, 0)` when that parameter is 1 and action index 6 with
//! `(0, -1, 0)` otherwise, scales the velocity by 40.0, and integrates the
//! position until the vertical travel passes `+0x4dc`. Indices 5 and 6 are
//! `c05` and `c052` in the action table at `0x56a7ac`.
//!
//! The opcode then blocks the script on the role until that finishes.

use crate::openpal3::directors::SceneManagerExtensions;
use crate::openpal3::scene::{RoleAnimationRepeatMode, RoleController, ScnScene};
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::{IEntity, IEntityExt, ISceneManager};
use radiance::math::Vec3;

/// `2 × 0x56a984` — how far in front of the ladder the actor stands.
pub const APPROACH_DISTANCE: f32 = 24.0;

/// The `40.0` the state machine multiplies the unit climb velocity by.
pub const CLIMB_SPEED: f32 = 40.0;

/// `0x56aa4c` and `0x56a774`: the climb covers `0.6 ×` the object's height,
/// capped at 100 units.
///
/// `Role::Script_Climb` (`gamebase/role.cpp:2982`) is one line:
///
/// ```cpp
/// gbOBBox box;  m_talkobj->GetCollideOBB(box);
/// m_specact.end.y = min(100, box.ylen*0.6f);
/// ```
///
/// and `0x418e94` agrees — `fld [esp+0x78]` (the OBB's `ylen`),
/// `fmul [0x56aa4c]`, then `fld [0x56a774]` / `fcomp st(1)` keeping 100
/// only when `100 < 0.6·ylen`.
pub const CLIMB_HEIGHT_RATIO: f32 = 0.6;
pub const MAX_CLIMB_HEIGHT: f32 = 100.0;

/// `0x56a984 × 0x56a744` — past this the original wants a clear nav path
/// before it will start the climb.
pub const OUT_OF_RANGE: f32 = 60.0;

/// Action table entries 5 and 6 (`0x56a7ac`).
const ACTION_UP: &str = "c05";
const ACTION_DOWN: &str = "c052";

/// The object's bounding-box height — `gbOBBox::ylen`, which the engine
/// writes as `aabb.max.y - aabb.min.y` (`engine/actor/gbmorphactor.cpp:132`,
/// and the struct's own comment `center(0,0,0), width = xlen` in
/// `engine/physics/gbcollide.h:41`), **not** the distance from the origin
/// to the top. `FUN_00429ee0` is `SceneObj::GetCollideOBB`
/// (`gamebase/sceneobj.cpp:3241`) and `[esp+0x78]` is that `ylen`.
fn object_height(entity: &ComRc<IEntity>) -> f32 {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    fn walk(entity: &ComRc<IEntity>, lo: &mut f32, hi: &mut f32) {
        if let Some(rc) = entity.get_rendering_component() {
            for object in rc.render_objects() {
                if let Some((min, max)) = object.as_dyn().local_aabb() {
                    *lo = lo.min(min[1]);
                    *hi = hi.max(max[1]);
                }
            }
        }
        for child in entity.children() {
            walk(&child, lo, hi);
        }
    }
    walk(entity, &mut lo, &mut hi);
    if hi > lo { hi - lo } else { 0. }
}

/// 74 攀爬.
#[derive(Debug, Clone)]
pub struct SceCommandClimb {
    object_id: i32,
    up: i32,
    /// Set once the actor is in place and the animation is running.
    climbing: bool,
    /// Vertical distance still to cover, `+0x4dc` minus what is done.
    remaining: f32,
}

impl SceCommand for SceCommandClimb {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if !self.climbing {
            let Some(remaining) = self.start(&scene_manager, state) else {
                return true;
            };
            self.climbing = true;
            self.remaining = remaining;
        }

        let step = if state.fast_forward() {
            self.remaining
        } else {
            (CLIMB_SPEED * delta_sec).min(self.remaining)
        };
        self.remaining -= step;

        let dy = if self.up == 1 { step } else { -step };
        let Some(actor) = scene_manager.get_resolved_role(state, -1) else {
            return true;
        };
        let mut position = actor.transform().borrow().position();
        position.y += dy;
        actor.transform().borrow_mut().set_position(&position);

        if self.remaining > 0. {
            return false;
        }

        // Back to standing. The original restores the role's previous mode
        // (`+0x4f0` ← `+0x4ec`); here the idle animation is the whole of it.
        if let Some(controller) = RoleController::get_role_controller(actor) {
            controller.inner::<RoleController>().idle();
        }
        true
    }
}

impl SceCommandClimb {
    pub fn new(object_id: i32, up: i32) -> Self {
        Self {
            object_id,
            up,
            climbing: false,
            remaining: 0.,
        }
    }

    /// Place the actor at the foot of the ladder and start the animation.
    /// `None` means the climb cannot happen and the script goes on, which
    /// is what `FUN_00418dc0` returning false does.
    fn start(&self, scene_manager: &ComRc<ISceneManager>, state: &mut SceState) -> Option<f32> {
        let scene = scene_manager.scn_scene()?;
        let scene = scene.inner::<ScnScene>();
        let Some((object_position, yaw)) = scene.object_placement(self.object_id) else {
            log::warn!("Climb: 找不到场景物件 {}", self.object_id);
            return None;
        };

        let stand = Vec3::new(
            object_position.x + APPROACH_DISTANCE * yaw.sin(),
            0.,
            object_position.z + APPROACH_DISTANCE * yaw.cos(),
        );

        let Some(actor) = scene_manager.get_resolved_role(state, -1) else {
            log::warn!("Climb: 场景里没有玩家角色");
            return None;
        };
        let Some(controller) = RoleController::get_role_controller(actor.clone()) else {
            log::warn!("Climb: 玩家角色没有 RoleController");
            return None;
        };
        let controller = controller.inner::<RoleController>();

        // `outdist`: too far away, and the original only carries on when
        // the nav map says the two tiles are connected. yaobow's
        // connectivity test works on nav coordinates, so ask it the same
        // question the original does.
        let from = actor.transform().borrow().position();
        // 原版中间那一项是角色 +8（离地的相对高度，站在地上是 0），不是世界高度。
        let flat = Vec3::new(stand.x - from.x, 0., stand.z - from.z);
        if flat.norm() > OUT_OF_RANGE {
            let layer = controller.nav_layer();
            let nav = scene.nav();
            let a = nav.round_nav_coord(layer, scene.scene_coord_to_nav_coord(layer, &from));
            let b = nav.round_nav_coord(layer, scene.scene_coord_to_nav_coord(layer, &stand));
            if !nav.check_connectivity(layer, a, b) {
                log::warn!("Climb: 角色离物件 {} 太远（outdist）", self.object_id);
                return None;
            }
        }

        // The original hands the target to `FUN_00412b60`, which drops the
        // actor onto the floor there; y comes from the nav map, not from
        // the zero the caller passes in.
        let layer = controller.nav_layer();
        let nav_coord = scene.scene_coord_to_nav_coord(layer, &stand);
        let mut placed = stand;
        placed.y = scene.get_height(layer, nav_coord);
        actor.transform().borrow_mut().set_position(&placed);

        // `FUN_00416620(yaw + 180°, 1)` — turn back towards the object.
        let face_to = Vec3::new(object_position.x, placed.y, object_position.z);
        actor.transform().borrow_mut().look_at(&face_to);

        let action = if self.up == 1 { ACTION_UP } else { ACTION_DOWN };
        controller.set_active(true);
        controller.play_anim(action, RoleAnimationRepeatMode::Loop);

        let height = scene
            .get_root_object(self.object_id)
            .map(|e| object_height(&e))
            .unwrap_or(0.);
        let travel = (height * CLIMB_HEIGHT_RATIO).min(MAX_CLIMB_HEIGHT);
        log::debug!(
            "Climb: 物件 {} 高 {height:.1}，爬 {travel:.1}，站位 {:?}",
            self.object_id,
            placed
        );
        Some(travel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_constants_are_the_ones_in_the_binary() {
        // 0x56a984 = 12.0, doubled; 0x56a744 = 5.0 times the same 12.0.
        assert_eq!(APPROACH_DISTANCE, 2. * 12.0);
        assert_eq!(OUT_OF_RANGE, 5. * 12.0);
        assert_eq!(CLIMB_SPEED, 40.0);
        assert_eq!(CLIMB_HEIGHT_RATIO, 0.6);
        assert_eq!(MAX_CLIMB_HEIGHT, 100.0);
    }

    #[test]
    fn the_climb_is_capped_at_a_hundred() {
        let travel = |h: f32| (h * CLIMB_HEIGHT_RATIO).min(MAX_CLIMB_HEIGHT);
        assert_eq!(travel(0.), 0.);
        assert!((travel(100.) - 60.).abs() < 1e-4);
        // 0.6 × 166.7 is where the cap takes over.
        assert_eq!(travel(300.), MAX_CLIMB_HEIGHT);
    }

    #[test]
    fn the_standing_spot_is_along_the_objects_own_facing() {
        // yaw 0 puts the actor at +Z, yaw 90° at +X — the same mapping
        // `rotate_axis_angle_local(UP, yaw)` gives the object itself.
        let spot = |yaw: f32| (APPROACH_DISTANCE * yaw.sin(), APPROACH_DISTANCE * yaw.cos());
        let (x, z) = spot(0.);
        assert!(x.abs() < 1e-4 && (z - APPROACH_DISTANCE).abs() < 1e-4);
        let (x, z) = spot(std::f32::consts::FRAC_PI_2);
        assert!((x - APPROACH_DISTANCE).abs() < 1e-4 && z.abs() < 1e-4);
    }
}
