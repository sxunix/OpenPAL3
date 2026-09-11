//! 156 UI 展示雪见图片 — `LAB_00438d20`.
//!
//! The handler builds a UI sprite at `0xbfda34`, points it at
//! `EffectScn\104s.tga` (the string at 0x59628c), gives it the rect
//! `{0, 0, 0x100, 0x100}` and then blocks the script on a four-second
//! task (`0x56a6c8` = 4.0).
//!
//! That task's update (`0x438e30`) is the whole animation:
//!
//! ```text
//! t += dt;  if t >= 4.0 { done; t = 4.0 }
//! alpha = (int)(sin(t * 0.785398) * 160.0)      // 0x56ac18, 0x56ac14
//! sprite.color = 0xffffff | alpha << 24
//! ```
//!
//! `0.785398` is pi/4, so over the four seconds the picture fades up to
//! `160/255` at t = 2 and back to nothing — one soft pulse. Q08's
//! 迴魂仙梦分支二 is the only script that uses it.

use std::f32::consts::PI;
use std::rc::Rc;

use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;
use radiance::rendering::Sprite;

/// `EffectScn\104s.tga`, 雪见's portrait.
pub const PICTURE: &str = "104s.tga";

/// `0x56a6c8`.
pub const SECONDS: f32 = 4.0;

/// `0x56ac18` — the sine's rate, pi/4 per second, so one half period
/// fills the four seconds exactly.
pub const RATE: f32 = PI / 4.;

/// `0x56ac14` — the peak alpha, out of 255.
pub const PEAK_ALPHA: f32 = 160.0;

/// The rect the original gives the sprite: `{0, 0, 0x100, 0x100}` in the
/// same 800×600 UI space every other widget uses — the picture sits in the
/// top-left corner at its native 256×256.
pub const RECT: (i32, i32, i32, i32) = (0, 0, 256, 256);

/// Alpha at `t` seconds, as a 0..1 multiplier.
fn alpha(t: f32) -> f32 {
    // `__ftol` truncates towards zero, which is what the original does
    // with the sine before stuffing it into the colour's alpha byte.
    ((RATE * t.clamp(0., SECONDS)).sin() * PEAK_ALPHA)
        .trunc()
        .max(0.)
        / 255.
}

/// 156 XJ_Pic.
#[derive(Clone)]
pub struct SceCommandShowXueJianPicture {
    elapsed: f32,
    sprite: Option<Rc<Sprite>>,
    loaded: bool,
}

impl std::fmt::Debug for SceCommandShowXueJianPicture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandShowXueJianPicture")
            .field("elapsed", &self.elapsed)
            .finish()
    }
}

impl SceCommand for SceCommandShowXueJianPicture {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if !self.loaded {
            self.loaded = true;
            let asset_mgr = state.asset_mgr();
            self.sprite = asset_mgr.load_ui_sprite(asset_mgr.effect_scn_path(PICTURE));
            if self.sprite.is_none() {
                log::warn!("XJ_Pic: 读不到 EffectScn\\{PICTURE}");
            }
        }

        let Some(sprite) = self.sprite.clone() else {
            return true;
        };
        if state.fast_forward() {
            return true;
        }

        self.elapsed += delta_sec;
        let a = alpha(self.elapsed);
        super::appraise::overlay(ui, "XJPic", |ui, at, _scale| {
            ui.get_window_draw_list()
                .add_image(
                    sprite.imgui_texture_id(),
                    at(RECT.0, RECT.1),
                    at(RECT.2, RECT.3),
                )
                .col([1., 1., 1., a])
                .build();
        });

        self.elapsed >= SECONDS
    }
}

impl SceCommandShowXueJianPicture {
    pub fn new() -> Self {
        Self {
            elapsed: 0.,
            sprite: None,
            loaded: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_pulses_once_over_four_seconds() {
        assert!(alpha(0.).abs() < 1e-6);
        // Peak at the halfway point: sin(pi/2) * 160 out of 255.
        assert!((alpha(2.) - 160. / 255.).abs() < 1e-3);
        // sin(pi) is zero again at the end.
        assert!(alpha(4.).abs() < 1e-6);
        // Never more than the 160 the original caps at.
        for i in 0..=40 {
            assert!(alpha(i as f32 / 10.) <= 160. / 255. + 1e-6);
        }
    }
}
