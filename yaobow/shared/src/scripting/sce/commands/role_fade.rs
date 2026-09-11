//! 135 角色淡出 · 136 角色淡入.

use std::f32::consts::PI;

use crate::openpal3::directors::SceneManagerExtensions;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::{IEntity, IEntityExt, ISceneManager};

/// Both fades run 5 seconds — the constant at `0x56a744`, shared with the
/// world-rotation tasks.
pub const FADE_SECONDS: f32 = 5.0;

/// PAL3's standard ease, `v = (1 - cos(pi * t / T)) / 2`, read off
/// `FUN_00438110`.
fn ease(t: f32) -> f32 {
    (1. - (PI * t.clamp(0., 1.)).cos()) / 2.
}

/// Set the actor's tint alpha, which the shaders apply to both RGB and
/// alpha. `FUN_0041aea0` switches the visual node's blend off at `>= 1.0`
/// and on below that; here the actor material's pipeline blends either
/// way and its cutout discard tests the texture's own alpha, so writing
/// the tint is enough and full opacity needs no special case.
///
/// The blob shadow (`scene::shadow`, entity `ROLE_SHADOW`) is left out: it
/// is a multiply-blended quad, and scaling a premultiplied tint into a
/// multiply blend darkens the white around the disc, so a half-faded actor
/// stood on a dark square. It shows only while the actor is fully opaque
/// **[推断]** — whether `roleshadow.cpp` fades with its role was not traced.
pub fn set_alpha(entity: &ComRc<IEntity>, alpha: f32) -> usize {
    if entity.name() == "ROLE_SHADOW" {
        entity.set_visible(alpha >= 0.999);
        return 0;
    }
    let mut n = 0;
    if let Some(rc) = entity.get_rendering_component() {
        for object in rc.render_objects() {
            object.as_dyn().set_alpha(alpha);
            n += 1;
        }
    }
    for child in entity.children() {
        n += set_alpha(&child, alpha);
    }
    n
}

/// Shared body of 135 and 136.
#[derive(Debug, Clone)]
struct Fade {
    role_id: i32,
    /// Alpha at t = 0 and at t = 1.
    from: f32,
    to: f32,
    elapsed: f32,
    started: bool,
}

impl Fade {
    fn step(
        &mut self,
        scene_manager: &ComRc<ISceneManager>,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        let Some(entity) = scene_manager.get_resolved_role(state, self.role_id) else {
            // Nothing to fade; waiting would strand the script.
            return true;
        };

        if !self.started {
            self.started = true;
            // 136 blanks the actor before the wait starts (`FUN_004385a0`
            // calls the alpha sink with 0 up front), so a fade-in never
            // shows one opaque frame first.
            let _ = set_alpha(&entity, self.from);
        }

        self.elapsed += delta_sec;
        let done = state.fast_forward() || self.elapsed >= FADE_SECONDS;
        let v = if done {
            1.
        } else {
            ease(self.elapsed / FADE_SECONDS)
        };
        let _ = set_alpha(&entity, self.from + (self.to - self.from) * v);
        done
    }
}

/// 135 角色淡出 — `LAB_004384d0`, alpha 1 → 0 over five seconds.
#[derive(Debug, Clone)]
pub struct SceCommandRoleFadeOut(Fade);

impl SceCommand for SceCommandRoleFadeOut {
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

impl SceCommandRoleFadeOut {
    pub fn new(role_id: i32) -> Self {
        Self(Fade {
            role_id,
            from: 1.,
            to: 0.,
            elapsed: 0.,
            started: false,
        })
    }
}

/// 136 角色淡入 — `FUN_004385a0`, alpha 0 → 1 over five seconds.
#[derive(Debug, Clone)]
pub struct SceCommandRoleFadeIn(Fade);

impl SceCommand for SceCommandRoleFadeIn {
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

impl SceCommandRoleFadeIn {
    pub fn new(role_id: i32) -> Self {
        Self(Fade {
            role_id,
            from: 0.,
            to: 1.,
            elapsed: 0.,
            started: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{FADE_SECONDS, ease};

    #[test]
    fn the_ease_starts_and_ends_flat_and_is_symmetric() {
        assert!(ease(0.).abs() < 1e-6);
        assert!((ease(1.) - 1.).abs() < 1e-6);
        assert!((ease(0.5) - 0.5).abs() < 1e-6);
        // Slower than linear at the start, faster in the middle.
        assert!(ease(0.25) < 0.25);
        assert!(ease(0.75) > 0.75);
        // Clamped outside [0, 1] rather than extrapolating.
        assert_eq!(ease(-1.), 0.);
        assert_eq!(ease(2.), 1.);
    }

    #[test]
    fn a_fade_runs_for_the_binarys_five_seconds() {
        assert_eq!(FADE_SECONDS, 5.0);
    }
}
