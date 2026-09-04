//! 141 设置全局镜头（屏幕）特效 — `LAB_004388c0`.
//!
//! The handler turns both screen effects off, then switches one back on
//! by the parameter:
//!
//! | 参数 | 效果 |
//! |---|---|
//! | 0 | `FUN_004bfd20` 那个：把上一帧的画面和这一帧混在一起，拖影 |
//! | 1 | `FUN_004b6d30` 那个：全屏盖一张 `EffectScn\Frame.Tga` |
//! | 其它（脚本里都是 -1） | 两个都关 |
//!
//! Effect 1 is `FUN_004b6e20`: a single `RenderUIQuad` spanning the whole
//! viewport in NDC (`-1..1` both ways), uv `(0,0)-(1,1)`, vertex colour
//! white, alpha blended — so the picture stretches over the frame rather
//! than fitting inside it. That is what this reproduces.
//!
//! Effect 0 is `FUN_004bfdd0`: it binds the previous frame's render
//! target alongside the current one and runs a two-stage texture blend.
//! Reproducing it needs a previous-frame target radiance does not hand
//! out, so it is skipped with a warning rather than faked — 罐中小妖 and
//! 迴魂仙梦分支四 use it.
//!
//! The command does not block: it flips a flag the renderer reads every
//! frame, so the effect stays up until another 141 clears it.

use std::rc::Rc;

use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;
use radiance::rendering::Sprite;

/// The overlay `FUN_004b6d80` loads at construction.
pub const FRAME_TEXTURE: &str = "Frame.Tga";

/// 141 ScrEft.
#[derive(Debug, Clone)]
pub struct SceCommandScreenEffect {
    effect: i32,
}

impl SceCommand for SceCommandScreenEffect {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        match self.effect {
            0 => {
                log::warn!("ScrEft: 拖影特效还没做（需要上一帧的渲染目标）");
                state.set_screen_effect(None);
            }
            1 => {
                let sprite = load_vignette(state);
                if sprite.is_none() {
                    log::warn!("ScrEft: 读不到 EffectScn\\{FRAME_TEXTURE}");
                }
                state.set_screen_effect(sprite);
            }
            _ => state.set_screen_effect(None),
        }
        true
    }
}

/// Turn the frame picture into something the UI layer can draw.
///
/// `frame.dds` is opaque greyscale — white in the middle, black towards
/// the border — and the original hands it to `RenderUIQuad` with a
/// render effect that modulates the frame buffer, i.e. it multiplies the
/// picture by that greyscale and darkens the edges. The UI path here only
/// does ordinary alpha blending, so the same result is built into the
/// texture instead: black RGB with `alpha = 255 - luma`, which composites
/// to `dst · luma`. Identical output for a greyscale mask, which is what
/// this one is.
fn load_vignette(state: &SceState) -> Option<Rc<Sprite>> {
    let asset_mgr = state.asset_mgr();
    let mut image = asset_mgr.read_picture(asset_mgr.effect_scn_path(FRAME_TEXTURE))?;
    for pixel in image.pixels_mut() {
        let [r, g, b, _] = pixel.0;
        // Rec. 601 luma, the usual greyscale weighting.
        let luma = (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32).round() as u8;
        pixel.0 = [0, 0, 0, 255 - luma];
    }
    Some(Rc::new(Sprite::load_from_image(
        image,
        asset_mgr.component_factory().as_ref(),
    )))
}

impl SceCommandScreenEffect {
    pub fn new(effect: i32) -> Self {
        Self { effect }
    }
}

#[cfg(test)]
mod tests {
    /// The mask conversion, in the same form `load_vignette` applies it.
    fn mask(rgb: [u8; 3]) -> [u8; 4] {
        let [r, g, b] = rgb;
        let luma = (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32).round() as u8;
        [0, 0, 0, 255 - luma]
    }

    #[test]
    fn white_becomes_clear_and_black_becomes_opaque() {
        // `frame.dds` is white in the middle and black at the border, so
        // the middle must let the picture through untouched and the
        // border must darken it all the way.
        assert_eq!(mask([255, 255, 255]), [0, 0, 0, 0]);
        assert_eq!(mask([0, 0, 0]), [0, 0, 0, 255]);
        // Half grey darkens by half.
        assert_eq!(mask([128, 128, 128]), [0, 0, 0, 127]);
    }
}
