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

/// How tall the bubble is drawn, in world units, and how far its base
/// clears the actor's head.
///
/// The original is **screen-space**, not a world-space billboard:
/// `UIEmote::ComputePos` (`gameui/UIEmote.cpp:128`) projects
/// `Role::GetHeadPos()` with `g_camera->PointWorldToScr`, then puts the
/// sheet's own pixel rect there — horizontally centred on the point and
/// with its **bottom** exactly on it (`left = ix - w/2; top = iy - h`),
/// at the native frame size from the table in `UIEmote::Create`. The
/// head point itself is `m_renderpos.y + box.ylen`, i.e. the actor's own
/// model height (`gamebase/role.cpp:3202`), and `EM_HAN` (汗) alone
/// lifts it by another 20 world units before projecting.
///
/// yaobow draws a billboard in the scene instead, so a sprite that is a
/// fixed number of screen pixels in the original has to become some
/// number of world units here. These two constants are that conversion
/// and they are a port decision, not a measurement: model heights vary
/// a lot (景天 76 world units, 龙葵 63, 花盈 28), so a fixed world size
/// that sits just over 景天's head floats half a body above a shorter
/// NPC — which is what the first pass did on screen.
///
/// Still missing: the 汗 bubble's extra 20-unit lift.
pub const EMOJI_HEIGHT_RATIO: f32 = 0.34;
pub const EMOJI_CLEARANCE: f32 = 2.0;

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
/// `actor_height` is the model's own height in world units — see
/// [`RoleController::model_height`](super::RoleController::model_height).
pub fn build_emoji(
    factory: &Rc<dyn ComponentFactory>,
    vfs: &MiniFs,
    basedata: &Path,
    id: i32,
    actor_height: f32,
) -> Option<ComRc<IEntity>> {
    let (frame_w, frame_h, frames, _) = emoji_spec(id)?;

    let path = basedata
        .join("ui")
        .join("EMOTE")
        .join(format!("EM_{id:02}.tga"));
    let mut file = match vfs.open(&path) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("emoji {id}: 读不到 {} ({e})", path.display());
            return None;
        }
    };
    let mut bytes = vec![];
    file.read_to_end(&mut bytes).ok()?;
    // The format has to be named: TGA has no magic number, so
    // `load_from_memory`'s sniffing always fails on these sheets. Unlike
    // the `.dds` scene textures, no flip: the sheets are bottom-left-origin
    // TGAs (descriptor 0x08) and the decoder already returns them upright,
    // which is what the billboard's v = 0-at-the-top wants. Flipping them
    // as well drew every bubble upside down (the 惊's dot on top).
    let sheet = match image::load_from_memory_with_format(&bytes, image::ImageFormat::Tga) {
        Ok(img) => img.to_rgba8(),
        Err(e) => {
            log::warn!("emoji {id}: {} 解不开 ({e})", path.display());
            return None;
        }
    };

    // The sheet is padded to a power of two and the frames sit against
    // its **bottom** edge, not its top: `EM_06.tga` is 128 x 64 with the
    // four hearts occupying rows 23..64, i.e. the strip starts at
    // `y = height - frame_h`.
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
    let atlas =
        image::imageops::crop_imm(&sheet, 0, sheet.height() - frame_h, used_w, frame_h).to_image();

    let height = actor_height * EMOJI_HEIGHT_RATIO;
    let base_y = actor_height + EMOJI_CLEARANCE;
    let width = height * frame_w as f32 / frame_h as f32;
    // One texture name per sheet: the texture store is keyed by name, and
    // a shared one gave every later bubble the first sheet loaded (the
    // opening's 惊 — every 心 / 汗 / 乱 after it came out as a strip of
    // red exclamation marks cut at the wrong frame size).
    let entity = build_frame_animated_billboard(
        factory,
        atlas,
        &format!("actor_emoji_{id:02}"),
        width,
        base_y,
        base_y + height,
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
