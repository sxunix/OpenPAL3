//! 表情气泡 — the sprite that pops up over an actor's head for opcode 72.
//!
//! Each emoji is one horizontally tiled sheet, `ui\EMOTE\EM_%02d.tga`,
//! padded out to a power of two; the frame rectangle inside it is not the
//! sheet's own size, so the used region has to be cropped out before it
//! can be treated as an atlas. Those rectangles and the loop counts come
//! from Pal3.Unity's `ActorEmojiConstants` (GPL-3.0, 0x7c13); the binary
//! agrees on the two that can be cross-checked, since `FUN_0044e8b0`
//! plays `we06` for id 1 and `we05` for id 11, which are exactly the two
//! ids Pal3.Unity gives a sound.

use std::path::Path;
use std::rc::Rc;

use crosscom::ComRc;
use mini_fs::{MiniFs, StoreExt};
use radiance::comdef::{IComponent, IEntity, IFrameAnimationComponent};
use radiance::rendering::ComponentFactory;
use std::io::Read;

use super::effect::build_frame_animated_billboard;

/// Frames run at 5 fps — Pal3.Unity's `EMOJI_ANIMATION_FPS`.
pub const EMOJI_FPS: f32 = 5.0;

/// Name of the child entity an emoji is attached to the actor as. The
/// id is part of the name so a repeat of the same emoji reuses the
/// entity instead of adding another: `IEntity` exposes `attach` but not
/// a detach, so bubbles are hidden rather than removed, and an actor
/// ends up with at most one child per emoji it has ever shown.
pub fn emoji_entity_name(id: i32) -> String {
    format!("EMOJI_{id}")
}

/// Prefix of [`emoji_entity_name`], for finding every bubble on an actor.
pub const EMOJI_ENTITY_PREFIX: &str = "EMOJI_";

/// How far above the actor's origin the bubble sits, and how tall it is,
/// in world units.
///
/// Measured rather than guessed: MV3 vertices are scaled by 0.01562 when
/// the mesh is built, and 景天's `ROLE/101/c01.mv3` spans 4863 model
/// units vertically, so he is 76 world units tall (龙葵 63, 花盈 28).
/// The bubble therefore starts just clear of a head at 80.
///
/// `[推断]` — the original sizes the bubble inside the emoji widget,
/// which was not read; the height still wants a look at the screen.
pub const EMOJI_BASE_Y: f32 = 80.0;
pub const EMOJI_HEIGHT: f32 = 26.0;

/// `(width, height, frames)` of one frame within sheet `EM_<id>.tga`,
/// and how many times the sheet loops, for emoji ids 1..=11.
pub fn emoji_spec(id: i32) -> Option<(u32, u32, u32, u32)> {
    Some(match id {
        1 => (38, 54, 6, 3),  // 睡
        2 => (25, 57, 10, 1), // 惊
        3 => (31, 43, 10, 1), // 疑
        4 => (82, 51, 3, 3),  // 怒
        5 => (54, 32, 2, 3),  // 喜
        6 => (31, 41, 4, 3),  // 心
        7 => (16, 111, 7, 1), // 汗
        8 => (64, 46, 3, 3),  // 乱
        9 => (54, 32, 2, 3),  // 急
        10 => (49, 40, 8, 1), // 泣
        11 => (70, 42, 6, 3), // 晕
        _ => return None,
    })
}

/// The sound the original plays with an emoji — `FUN_0044e8b0` wires one
/// for id 1 (`we06`, 0x5a0f98) and id 11 (`we05`, 0x5a0fa0) and for
/// nothing else.
pub fn emoji_sound(id: i32) -> Option<&'static str> {
    match id {
        1 => Some("we06"),
        11 => Some("we05"),
        _ => None,
    }
}

/// How long one playthrough of an emoji takes, in seconds.
pub fn emoji_duration(id: i32) -> Option<f32> {
    let (_, _, frames, loops) = emoji_spec(id)?;
    Some(frames as f32 * loops as f32 / EMOJI_FPS)
}

/// Build the bubble entity for `id`, ready to attach to an actor.
pub fn build_emoji(
    factory: &Rc<dyn ComponentFactory>,
    vfs: &MiniFs,
    basedata: &Path,
    id: i32,
) -> Option<ComRc<IEntity>> {
    let (frame_w, frame_h, frames, _) = emoji_spec(id)?;

    let path = basedata
        .join("ui")
        .join("EMOTE")
        .join(format!("EM_{id:02}.tga"));
    let mut file = vfs.open(&path).ok()?;
    let mut bytes = vec![];
    file.read_to_end(&mut bytes).ok()?;
    // Flipped like every other PAL3 texture the scene loads (see
    // `effect::load_texture`), so the atlas rows line up with the
    // billboard's v = 0 at the top.
    let sheet = image::load_from_memory(&bytes).ok()?.flipv().to_rgba8();

    // The sheet is padded to a power of two; only `frames * frame_w` by
    // `frame_h` of it is the animation. Crop from the bottom-left, since
    // the flip put the original top-left there.
    let used_w = frame_w * frames;
    if sheet.width() < used_w || sheet.height() < frame_h {
        log::warn!(
            "emoji {id}: 图集 {}x{} 装不下 {}x{}",
            sheet.width(),
            sheet.height(),
            used_w,
            frame_h
        );
        return None;
    }
    let top = sheet.height() - frame_h;
    let atlas = image::imageops::crop_imm(&sheet, 0, top, used_w, frame_h).to_image();

    let width = EMOJI_HEIGHT * frame_w as f32 / frame_h as f32;
    let entity = build_frame_animated_billboard(
        factory,
        atlas,
        "actor_emoji",
        width,
        EMOJI_BASE_Y,
        EMOJI_BASE_Y + EMOJI_HEIGHT,
        emoji_entity_name(id),
        radiance::rendering::BlendMode::AlphaBlend,
    );

    let anim = radiance::components::frame_anim::FrameAnimationComponent::create(
        entity.clone(),
        frames,
        1,
        frames,
        EMOJI_FPS,
    );
    entity.add_component(
        IFrameAnimationComponent::uuid(),
        anim.query_interface::<IComponent>().unwrap(),
    );
    Some(entity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_shipped_sheet_has_a_spec_and_fits() {
        // EM_01..EM_11 ship; the padded sheet sizes are 256x64, 512x64,
        // 128x32, 128x64 and 128x128, and every frame strip has to fit.
        let sheets = [
            (1, 256, 64),
            (2, 256, 64),
            (3, 512, 64),
            (4, 256, 64),
            (5, 128, 32),
            (6, 128, 64),
            (7, 128, 128),
            (8, 256, 64),
            (9, 128, 32),
            (10, 512, 64),
            (11, 512, 64),
        ];
        for (id, sheet_w, sheet_h) in sheets {
            let (w, h, frames, loops) = emoji_spec(id).expect("每个出厂图集都要有参数");
            assert!(w * frames <= sheet_w, "emoji {id} 横向放不下");
            assert!(h <= sheet_h, "emoji {id} 纵向放不下");
            assert!(loops >= 1);
        }
    }

    #[test]
    fn only_the_two_ids_the_binary_wires_have_a_sound() {
        assert_eq!(emoji_sound(1), Some("we06"));
        assert_eq!(emoji_sound(11), Some("we05"));
        for id in [0, 2, 5, 10, 12] {
            assert_eq!(emoji_sound(id), None);
        }
    }

    #[test]
    fn an_unknown_id_has_no_spec_so_nothing_is_waited_on() {
        // FUN_0044e8b0 only accepts 0 <= id < 13 and shows nothing
        // outside its 12 sub-renderers; a script asking for one of those
        // must not leave the vm waiting forever.
        assert!(emoji_spec(0).is_none());
        assert!(emoji_spec(12).is_none());
        assert!(emoji_duration(99).is_none());
    }

    #[test]
    fn a_duration_is_frames_times_loops_over_the_frame_rate() {
        // 睡: 6 frames, 3 loops, 5 fps.
        assert!((emoji_duration(1).unwrap() - 18. / 5.).abs() < 1e-6);
    }
}
