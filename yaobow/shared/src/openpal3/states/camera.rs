//! The PAL3 camera, as the original models it.
//!
//! `DAT_00bfda68`'s camera block is polar, not a transform: yaw, pitch and
//! roll, a distance, the camera position (`+0x1ec`) and the look-at point
//! (`+0x1e0`), with one function — `FUN_0041c710` — deriving whichever of
//! the last two is not being held fixed:
//!
//! ```text
//! dir = (sin yaw · cos pitch,  sin pitch,  −cos pitch · cos yaw)
//! mode 0:  look_at  = position + distance · dir     // camera held
//! mode 1:  position = look_at  − distance · dir     // look-at held
//! ```
//!
//! `dir` points from the camera **towards** the look-at point: the render
//! call is `gbCamera::SetLookAt(+0x1ec, +0x1e0, +0x204)`, whose parameters
//! are `(pfrom, pto, pup)` (`engine/globalvr/gbcamera.h:50`), and
//! Pal3.Unity's `CameraSetTransformCommand` agrees — the triple it gets is
//! the camera position and the look-at is `position + forward · distance`.
//! (This model used to have the two points the other way round, which put
//! every scripted shot at its own target looking back at where the camera
//! should be.) yaobow uses the game's coordinates unconverted, so the
//! formula applies as written.
//!
//! Everything the camera opcodes do is a move within this model: 30/31/251
//! change what is looked at, 32 changes the distance, 33 and 35 animate the
//! angles about one fixed end or the other, 38/39 push and pop the lot.

use radiance::math::Vec3;

/// What the camera keeps in frame.
#[derive(Debug, Clone)]
pub enum Focus {
    /// Follow whoever the player controls — the engine's own default, and
    /// what `CameraFollowPlayer` (250) turns back on.
    Player,
    /// `CameraFocusOnActor` (30) — `FUN_0041c170` stores the role at
    /// `+0x17c` and clears `+0x180`.
    Role(i32),
    /// `CameraFocusOnSceneObject` (251) — `FUN_0041c190`, `+0x180`.
    Object(i32),
    /// `CameraFocusOnPosition` (31) — `FUN_0041c1b0` clears both target
    /// pointers and stores the point at `+0x184`.
    Point(Vec3),
}

/// One entry of the camera state stack — the seven dwords `FUN_0041d220`
/// pushes and `FUN_0041d410` restores: yaw, pitch, distance and the
/// look-at point (`+0x1e0..+0x1e8`). The camera position is derived again
/// on pop, and the focus target survives it unchanged.
#[derive(Debug, Clone, Copy)]
pub struct Saved {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub look_at: Vec3,
}

#[derive(Debug, Clone)]
pub struct Pal3Camera {
    /// Radians. `+0x1c8`.
    yaw: f32,
    /// Radians. `+0x1c4`.
    pitch: f32,
    /// Radians. `+0x1d0`.
    roll: f32,
    /// `+0x1d4`.
    distance: f32,
    /// `+0x1e0`.
    look_at: Vec3,
    /// `+0x1ec`.
    position: Vec3,
    focus: Focus,
    /// `+0x1c0`. Cleared by a script that takes the camera over and set
    /// again by `CameraFollowPlayer(1)`.
    follow_player: bool,
    stack: Vec<Saved>,
    /// Horizontal field of view at 4:3, degrees (`gbCamera::SetFOV`; set
    /// by `Default`). 30 until the first `Default` — the engine camera's.
    fov: f32,
    /// `FlushSpring` asked for: put the look-at point straight on the
    /// target next frame instead of springing towards it.
    flush: bool,
}

/// Distance of yaobow's default follow camera, which sits at the offset
/// `(400, 400, 400)` from the player. Kept exactly so switching the
/// director onto this model changes nothing on screen.
pub const DEFAULT_OFFSET: f32 = 400.;

impl Default for Pal3Camera {
    fn default() -> Self {
        let (yaw, pitch, distance) =
            polar_of(&Vec3::new(DEFAULT_OFFSET, DEFAULT_OFFSET, DEFAULT_OFFSET));
        Self {
            yaw,
            pitch,
            roll: 0.,
            distance,
            look_at: Vec3::new(0., 0., 0.),
            position: Vec3::new(0., 0., 0.),
            focus: Focus::Player,
            follow_player: true,
            stack: vec![],
            fov: 30.,
            flush: true,
        }
    }
}

/// `FUN_0041c710`'s direction: from the camera towards the look-at point.
/// A negative pitch looks down — q01/xa's cutscene camera is `(yaw
/// -167.1°, pitch -16.42°, distance 664)` from a point 187 units up.
pub fn direction(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(
        yaw.sin() * pitch.cos(),
        pitch.sin(),
        -pitch.cos() * yaw.cos(),
    )
}

/// The yaw, pitch and distance for a camera sitting at `offset` from its
/// look-at point (`offset = position − look_at`, so `direction` is
/// `−offset / distance`). Used to express an offset the rest of the engine
/// already works in as this model's angles.
pub fn polar_of(offset: &Vec3) -> (f32, f32, f32) {
    let distance = (offset.x * offset.x + offset.y * offset.y + offset.z * offset.z).sqrt();
    if distance <= f32::EPSILON {
        return (0., 0., 0.);
    }
    let pitch = (-offset.y / distance).asin();
    let yaw = (-offset.x).atan2(offset.z);
    (yaw, pitch, distance)
}

impl Pal3Camera {
    pub fn yaw(&self) -> f32 {
        self.yaw
    }

    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    pub fn distance(&self) -> f32 {
        self.distance
    }

    pub fn position(&self) -> Vec3 {
        self.position
    }

    pub fn look_at(&self) -> Vec3 {
        self.look_at
    }

    pub fn focus(&self) -> &Focus {
        &self.focus
    }

    pub fn follow_player(&self) -> bool {
        self.follow_player
    }

    /// 250 `CameraFollowPlayer` — `LAB_00435380` writes `+0x1c0` and
    /// nothing else. In particular it does **not** clear the focus target,
    /// though Pal3.Unity's executor does.
    pub fn set_follow_player(&mut self, follow: bool) {
        self.follow_player = follow;
    }

    pub fn set_focus(&mut self, focus: Focus) {
        self.focus = focus;
    }

    pub fn set_yaw(&mut self, yaw: f32) {
        self.yaw = yaw;
    }

    pub fn set_pitch(&mut self, pitch: f32) {
        self.pitch = pitch;
    }

    pub fn set_distance(&mut self, distance: f32) {
        self.distance = distance;
    }

    pub fn set_look_at(&mut self, look_at: Vec3) {
        self.look_at = look_at;
    }

    pub fn set_position(&mut self, position: Vec3) {
        self.position = position;
    }

    /// `FUN_0041c710(1)`: hold the look-at point, move the camera.
    pub fn recompute_position(&mut self) {
        let dir = direction(self.yaw, self.pitch);
        self.position = Vec3::sub(&self.look_at, &Vec3::scalar_mul(self.distance, &dir));
    }

    /// `FUN_0041c710(0)`: hold the camera, move the look-at point.
    pub fn recompute_look_at(&mut self) {
        let dir = direction(self.yaw, self.pitch);
        self.look_at = Vec3::add(&self.position, &Vec3::scalar_mul(self.distance, &dir));
    }

    /// 38 `CameraPushState` — `FUN_0041d220` pushes yaw, pitch, distance
    /// and the look-at point.
    pub fn push_state(&mut self) {
        self.stack.push(Saved {
            yaw: self.yaw,
            pitch: self.pitch,
            distance: self.distance,
            look_at: self.look_at,
        });
    }

    /// 39 `CameraPopState` — `FUN_0041d410` restores those four and then
    /// recomputes with the look-at held (`FUN_0041c710(1)`), so the camera
    /// position is derived rather than restored.
    pub fn pop_state(&mut self) -> bool {
        let Some(saved) = self.stack.pop() else {
            return false;
        };
        self.yaw = saved.yaw;
        self.pitch = saved.pitch;
        self.distance = saved.distance;
        self.look_at = saved.look_at;
        self.recompute_position();
        true
    }

    pub fn stack_depth(&self) -> usize {
        self.stack.len()
    }

    pub fn fov(&self) -> f32 {
        self.fov
    }

    /// `RoleCamera::Default(type)` (gamebase/rolecamera.cpp): the stock
    /// shot per scene type, looking at the origin until the spring brings
    /// it onto the target, and a pending `FlushSpring`. Type 0 is outdoors
    /// (`SCENE_STORYA`), 1 indoors (`SCENE_STORYB`), 2 and 3 the indoor
    /// shot from the other side and straight on. Other values change
    /// nothing but the flush, as in the original's switch. [移植]
    pub fn set_default(&mut self, kind: i32) {
        let preset = match kind {
            0 => Some((26., 1144., -52.65_f32, -0.53)),
            1 => Some((24.05, 688., 33.24, -0.34)),
            2 => Some((24.05, 688., -33.24, -0.34)),
            3 => Some((24.05, 688., 0., -0.34)),
            _ => None,
        };
        if let Some((fov, distance, yaw_deg, pitch)) = preset {
            self.fov = fov;
            self.distance = distance;
            self.yaw = yaw_deg.to_radians();
            self.pitch = pitch;
            self.roll = 0.;
            self.look_at = Vec3::new(0., 0., 0.);
            self.recompute_position();
        }
        self.flush = true;
    }

    /// `FlushSpring` on the next follow step.
    pub fn request_flush(&mut self) {
        self.flush = true;
    }

    /// One `RoleCamera::Update` step while free and locked on `target`
    /// (the role's feet): the look-at point hangs on a spring (constant
    /// 4.6, rest length 20) and snaps onto the target once it would pass
    /// it; indoors the target is raised by the room height (32) and
    /// `UpdateRoomMode` keeps the look-at point 120 inside the nav bounds.
    /// Then the camera is placed behind it (`ComputeInfo(TRUE)`). [移植]
    pub fn follow(&mut self, target: Vec3, delta_sec: f32, room: Option<(Vec3, Vec3)>) {
        let mut target = target;
        if room.is_some() {
            target.y += 32.;
        }
        if std::mem::take(&mut self.flush) {
            self.look_at = target;
        } else {
            let disp = Vec3::sub(&self.look_at, &target);
            let len = disp.norm();
            if len > 0.05 {
                let old_dir = Vec3::scalar_mul(1. / len, &disp);
                let force = 4.6 * (20. - len);
                self.look_at = Vec3::add(
                    &self.look_at,
                    &Vec3::scalar_mul(force * delta_sec, &old_dir),
                );
                let new_dir = Vec3::sub(&self.look_at, &target);
                if Vec3::dot(&old_dir, &new_dir) <= 0. {
                    self.look_at = target;
                }
            }
        }
        if let Some((min, max)) = room {
            const SIZE: f32 = 120.;
            self.look_at.x = clip(self.look_at.x, min.x + SIZE, max.x - SIZE);
            self.look_at.z = clip(self.look_at.z, min.z + SIZE, max.z - SIZE);
        }
        self.recompute_position();
    }

    /// Aim at `target` while keeping the current angles and distance —
    /// what the per-frame camera does once a focus target is resolved.
    pub fn look_at_target(&mut self, target: Vec3) {
        self.look_at = target;
        self.recompute_position();
    }
}

/// The original's `clip(val, min, max)` (pal3/gbext.h): high bound first,
/// so a range narrower than the room margin pins to `min`.
fn clip(v: f32, lo: f32, hi: f32) -> f32 {
    let v = if v > hi { hi } else { v };
    if v < lo { lo } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    fn same(a: &Vec3, b: &Vec3) -> bool {
        close(a.x, b.x) && close(a.y, b.y) && close(a.z, b.z)
    }

    #[test]
    fn the_two_recomputes_are_inverses() {
        let mut camera = Pal3Camera::default();
        camera.set_yaw(0.7);
        camera.set_pitch(0.4);
        camera.set_distance(500.);
        camera.set_look_at(Vec3::new(10., 20., 30.));
        camera.recompute_position();

        let position = camera.position();
        camera.set_look_at(Vec3::new(0., 0., 0.));
        camera.recompute_look_at();
        assert!(close(camera.look_at().x, 10.));
        assert!(close(camera.look_at().y, 20.));
        assert!(close(camera.look_at().z, 30.));
        assert!(same(&camera.position(), &position));
    }

    #[test]
    fn polar_round_trips_through_the_direction_formula() {
        for offset in [
            Vec3::new(400., 400., 400.),
            Vec3::new(-120., 60., 300.),
            Vec3::new(0., 500., 0.),
        ] {
            let (yaw, pitch, distance) = polar_of(&offset);
            let back = Vec3::scalar_mul(-distance, &direction(yaw, pitch));
            assert!(close(back.x, offset.x), "{back:?} vs {offset:?}");
            assert!(close(back.y, offset.y), "{back:?} vs {offset:?}");
            assert!(close(back.z, offset.z), "{back:?} vs {offset:?}");
        }
    }

    #[test]
    fn the_default_camera_sits_where_the_old_follow_rig_did() {
        // The ADV director used to hard-code position (400, 400, 400)
        // relative to the player. Expressed as angles it has to land in
        // the same place, or switching the director onto this model would
        // move every shot.
        let mut camera = Pal3Camera::default();
        camera.look_at_target(Vec3::new(100., 5., -60.));
        assert!(close(camera.position().x, 100. + 400.));
        assert!(close(camera.position().y, 5. + 400.));
        assert!(close(camera.position().z, -60. + 400.));
    }

    #[test]
    fn a_pop_restores_the_look_at_and_derives_the_camera() {
        let mut camera = Pal3Camera::default();
        camera.look_at_target(Vec3::new(0., 0., 0.));
        let before = camera.position();
        camera.push_state();

        camera.set_yaw(2.5);
        camera.set_distance(50.);
        camera.look_at_target(Vec3::new(900., 0., 900.));
        assert!(!same(&camera.position(), &before));

        assert!(camera.pop_state());
        assert!(same(&camera.look_at(), &Vec3::new(0., 0., 0.)));
        // The camera comes back out of the formula, and for these angles
        // that is where it was.
        assert!(same(&camera.position(), &before));
        assert!(!camera.pop_state(), "空栈再弹应当无事发生");
    }

    #[test]
    fn a_set_transform_triple_is_the_camera_and_a_negative_pitch_looks_down() {
        // 36 `CameraSet(88.5°, -19.27°, 904, -2275.31, 298.29, -56.95)`
        // (M25 proc 1002): `FUN_0041c9a0` writes the triple to `+0x1ec`
        // and calls `FUN_0041c710(0)`.
        let mut camera = Pal3Camera::default();
        camera.set_yaw(88.5f32.to_radians());
        camera.set_pitch((-19.27f32).to_radians());
        camera.set_distance(904.);
        camera.set_position(Vec3::new(-2275.31, 298.29, -56.95));
        camera.recompute_look_at();
        assert!(same(
            &camera.position(),
            &Vec3::new(-2275.31, 298.29, -56.95)
        ));
        assert!(close(
            camera.look_at().y,
            298.29 - 904. * 19.27f32.to_radians().sin()
        ));
        assert!(camera.look_at().x > camera.position().x + 800.);
    }
}
