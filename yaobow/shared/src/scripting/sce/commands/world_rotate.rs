//! 117 / 128 整个场景的旋转 — `LAB_00438030` and `LAB_004381e0`.
//!
//! Both read three ints (degrees about X, Y and Z), convert them to
//! float, build a five-second task and block the script on it. They
//! differ only in which end they start from:
//!
//! | op | 构造时 | 每帧写入 | 效果 |
//! |---|---|---|---|
//! | 128 | 全局清零 | `v · angle` | 从正常转到 `angle` |
//! | 117 | 全局设为 `angle` | `(1 - v) · angle` | 从 `angle` 转回正常 |
//!
//! with `v = (1 - cos(pi · t / 5)) / 2` — PAL3's standard ease over the
//! 5.0 at `0x56a744`, read straight off `0x438110` and `0x4382a0`
//! (`0x56ab50` = 0.2, `0x56aa30` = pi, and the two doubles `0x56a710` = 1
//! and `0x56aa28` = 0.5).
//!
//! So the names are the wrong way round: **128** is the one that tilts
//! the world, **117** puts it back. The scripts bear that out — every use
//! is in M15, where a room transition opens with 128 and closes with 117
//! (`「从c1到b1」`, `「从b1到d」`, …), and the two angles always negate.
//!
//! The three globals at `0xc12b28` are consumed once per frame right
//! after `gbSceneGraph::Begin`: three `gbMatrixStack::Rotate` calls about
//! (1,0,0), (0,1,0) and (0,0,1), so the whole world is turned before
//! anything is drawn. `Camera::set_world_rotation` is the same thing on
//! this side — it folds into the view matrix, which the frustum reads
//! too, so culling turns with the picture.

use std::f32::consts::PI;

use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::{ISceneExt, ISceneManager};

/// `0x56a744` — both rotations take five seconds.
pub const ROTATE_SECONDS: f32 = 5.0;

/// PAL3's standard ease.
fn ease(t: f32) -> f32 {
    (1. - (PI * t.clamp(0., 1.)).cos()) / 2.
}

/// Shared body of 117 and 128.
#[derive(Debug, Clone)]
struct Rotate {
    /// Degrees about X, Y and Z.
    angles: [f32; 3],
    /// `true` for 128 (0 → angle), `false` for 117 (angle → 0).
    towards: bool,
    elapsed: f32,
    started: bool,
}

impl Rotate {
    fn new(x: i32, y: i32, z: i32, towards: bool) -> Self {
        Self {
            angles: [x as f32, y as f32, z as f32],
            towards,
            elapsed: 0.,
            started: false,
        }
    }

    fn step(
        &mut self,
        scene_manager: &ComRc<ISceneManager>,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if !self.started {
            self.started = true;
            // Both constructors write the starting angles before the
            // first tick: 128 zeroes the globals, 117 fills them in.
            self.apply(scene_manager, if self.towards { 0. } else { 1. });
        }

        self.elapsed += delta_sec;
        let done = state.fast_forward() || self.elapsed >= ROTATE_SECONDS;
        let v = if done {
            1.
        } else {
            ease(self.elapsed / ROTATE_SECONDS)
        };
        let factor = if self.towards { v } else { 1. - v };
        self.apply(scene_manager, factor);
        if done {
            log::debug!(
                "世界旋转结束 towards={} angles={:?} factor={factor} elapsed={}",
                self.towards,
                self.angles,
                self.elapsed
            );
        }
        done
    }

    fn apply(&self, scene_manager: &ComRc<ISceneManager>, factor: f32) {
        let Some(scene) = scene_manager.scene() else {
            return;
        };
        let [x, y, z] = self.angles;
        scene.camera_mut().set_world_rotation(
            (x * factor).to_radians(),
            (y * factor).to_radians(),
            (z * factor).to_radians(),
        );
    }
}

/// 128 场景旋转到指定角度 — `LAB_004381e0`, despite being named
/// `SceneRotateWorldInverse`.
#[derive(Debug, Clone)]
pub struct SceCommandRotateWorld(Rotate);

impl SceCommand for SceCommandRotateWorld {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        self.0.step(&scene_manager, state, delta_sec)
    }
}

impl SceCommandRotateWorld {
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        Self(Rotate::new(x, y, z, true))
    }
}

/// 117 场景从指定角度转回 — `LAB_00438030`, named `SceneRotateWorld`.
#[derive(Debug, Clone)]
pub struct SceCommandRotateWorldBack(Rotate);

impl SceCommand for SceCommandRotateWorldBack {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        self.0.step(&scene_manager, state, delta_sec)
    }
}

impl SceCommandRotateWorldBack {
    pub fn new(x: i32, y: i32, z: i32) -> Self {
        Self(Rotate::new(x, y, z, false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ease_is_pal3s_and_runs_five_seconds() {
        assert_eq!(ROTATE_SECONDS, 5.0);
        assert!(ease(0.).abs() < 1e-6);
        assert!((ease(1.) - 1.).abs() < 1e-6);
        assert!((ease(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_two_commands_run_opposite_ways() {
        let towards = Rotate::new(40, 0, 0, true);
        let back = Rotate::new(40, 0, 0, false);
        // 128 starts at nothing and ends at the full angle; 117 the
        // other way. `step` multiplies the angles by these.
        let factor = |r: &Rotate, v: f32| if r.towards { v } else { 1. - v };
        assert_eq!(factor(&towards, 0.), 0.);
        assert_eq!(factor(&towards, 1.), 1.);
        assert_eq!(factor(&back, 0.), 1.);
        assert_eq!(factor(&back, 1.), 0.);
    }
}
