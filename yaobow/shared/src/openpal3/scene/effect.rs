//! PAL3 scene-effect (`EffectScn`) construction.
//!
//! `+`-prefixed scene nodes mark `EffectScn` effects (candle / oil-lamp
//! / torch / fire). The visual class is node_type 17 and the variant is
//! selected by the node's `dw184[3]` field (the *effect id*).
//!
//! The id → asset mapping is **not** stored in any data file: it is a
//! hardcoded switch in the original `PAL3.exe`. Reverse-engineered from
//! the binary, each effect is composed of (additively blended):
//!   * 16 animated flame frames (folder + name pattern per id),
//!   * a radial glow billboard (`effect/flare4`) — only ids 0, 2, 4 —, and
//!   * for ids 2 (candle) and 4 (lamp), a static holder mesh.

use std::io::Read;
use std::path::Path;
use std::rc::Rc;

use crosscom::ComRc;
use mini_fs::{MiniFs, StoreExt};

use radiance::comdef::{IComponent, IEntity, IFrameAnimationComponent, IStaticMeshComponent};
use radiance::components::frame_anim::FrameAnimationComponent;
use radiance::components::mesh::{Geometry, StaticMeshComponent, TexCoord};
use radiance::math::Vec3;
use radiance::rendering::{BlendMode, ComponentFactory, SimpleMaterialDef};
use radiance::scene::CoreEntity;

use crate::openpal3::loaders::pol::create_entity_from_pol_model;

/// Static description of one `EffectScn` effect variant.
struct EffectSpec {
    /// Holder mesh path relative to the `EffectScn` root (e.g.
    /// `"Candle/candle.pol"`), or `None` for holderless fires/torches.
    /// Only the candle (id 2) and lamp (id 4) have one.
    holder_pol: Option<&'static str>,
    /// `EffectScn` sub-folder the animated flame frames live in.
    flame_folder: &'static str,
    /// Animated flame sprite frame file names (relative to
    /// `flame_folder`).
    frames: Vec<String>,
    /// Atlas grid the frames are packed into.
    cols: u32,
    rows: u32,
    /// Flame billboard width + height + base height, in the effect's
    /// local space (holder base / node origin at y = 0).
    width: f32,
    height: f32,
    base_y: f32,
    /// Frames per second. **No binary source found**: `FUN_004b7980`
    /// only pins the frame count (`while (i < 0x10)`), and the render
    /// method at `0x4b7590` just indexes `+0x5c` — whoever advances it
    /// was not located. Pal3.Unity uses 10
    /// (`EffectConstants.AnimatedFireEffectFrameRate`); this 12 predates
    /// that comparison and neither is backed by the exe.
    fps: f32,
    /// Whether this variant gets the `flare4` glow at all. See
    /// [`effect_spec`]'s table — ids 1 and 3 do not.
    glow: bool,
    /// Radial glow (`flare4`) billboard: square side length and the
    /// height of its centre.
    glow_size: f32,
    glow_y: f32,
}

/// Resolve a PAL3 scene-effect id (`dw184[3]`) to its asset spec.
///
/// This table is the hardcoded switch from the original `PAL3.exe`
/// (reverse-engineered from the binary — it is not data-driven). The
/// effect-loader builds `frame_table[id]` for the 16 flame frames and
/// loads a holder `.pol` only for ids 2 and 4:
///
/// | id | holder | flame frames | flare4 glow |
/// |----|--------|--------------|-------------|
/// | 0  | —          | `Fire1/torch01-16` | yes |
/// | 1  | —          | `Fire2/01-16`      | **no** |
/// | 2  | Candle.pol | `Candle/001-016`   | yes |
/// | 3  | —          | `Fire4/01-16`      | **no** |
/// | 4  | Lamp.pol   | `Candle/001-016`   | yes |
///
/// Verified live against q01/yn09a, whose `+3` node has id 4 → the brass
/// oil lamp with the candle flame.
///
/// The glow column comes from `FUN_004b7980`, which after loading the 16
/// frames does:
///
/// ```text
/// if (id != 2 && id != 4 && id != 0) goto done;   // 1 and 3 skip the rest
/// new gbGeomNode -> +0xf0
/// if (id == 2) load Candle\Candle.pol
/// else if (id == 4) load Lamp\Lamp.pol            // id 0 loads no holder
/// load effect\Flare4.bmp -> +0xec                 // the glow
/// done:
/// ```
///
/// So ids 1 and 3 get **no glow at all** — this used to give every
/// variant one.
fn effect_spec(effect_id: u32) -> Option<EffectSpec> {
    // `prefixNN.dds` for NN in 1..=16, zero-padded to `width`.
    let seq = |prefix: &str, width: usize| -> Vec<String> {
        (1..=16)
            .map(|i| format!("{}{:0width$}.dds", prefix, i, width = width))
            .collect()
    };
    // Holderless fire/torch: flame rooted at the node origin.
    let fire = |folder: &'static str, frames: Vec<String>, glow: bool| EffectSpec {
        holder_pol: None,
        glow,
        flame_folder: folder,
        frames,
        cols: 4,
        rows: 4,
        width: 28.0,
        height: 40.0,
        base_y: 0.0,
        fps: 12.0,
        glow_size: 34.0,
        glow_y: 16.0,
    };
    Some(match effect_id {
        0 => fire("Fire1", seq("torch", 2), true),
        1 => fire("Fire2", seq("", 2), false),
        2 => EffectSpec {
            // The tall wax candle (candle.pol is ~30.6 tall).
            holder_pol: Some("Candle/candle.pol"),
            glow: true,
            flame_folder: "Candle",
            frames: seq("", 3),
            cols: 4,
            rows: 4,
            width: 12.0,
            height: 16.0,
            base_y: 27.0,
            fps: 12.0,
            glow_size: 24.0,
            glow_y: 32.0,
        },
        3 => fire("Fire4", seq("", 2), false),
        4 => EffectSpec {
            // The brass oil lamp (lamp.pol is ~10.8 tall). Its folder
            // ships no frames, so the exe feeds it the candle flame.
            holder_pol: Some("Lamp/lamp.pol"),
            glow: true,
            flame_folder: "Candle",
            frames: seq("", 3),
            cols: 4,
            rows: 4,
            width: 10.0,
            height: 12.0,
            base_y: 9.0,
            fps: 12.0,
            glow_size: 30.0,
            glow_y: 13.0,
        },
        _ => return None,
    })
}

/// Build the entity for a scene effect identified by `effect_id`.
///
/// `effect_root` is the `EffectScn` directory. Returns the holder entity
/// (with the flame billboard attached as a child) when the effect has a
/// holder, otherwise the flame entity directly, or `None` if the effect
/// id is unknown / its assets are missing.
pub fn build_effect(
    factory: &Rc<dyn ComponentFactory>,
    vfs: &MiniFs,
    effect_root: &Path,
    effect_id: u32,
    index: u16,
) -> Option<ComRc<IEntity>> {
    let spec = effect_spec(effect_id)?;

    let holder = spec.holder_pol.and_then(|pol| {
        let path = effect_root.join(pol);
        vfs.open(&path).ok().map(|_| {
            create_entity_from_pol_model(factory, vfs, &path, format!("EFFECT_{}", index), true)
        })
    });

    let flame_dir = effect_root.join(spec.flame_folder);
    let flame = build_flame(
        factory,
        vfs,
        &flame_dir,
        &spec.frames,
        spec.cols,
        spec.rows,
        spec.width,
        spec.height,
        spec.base_y,
        spec.fps,
        format!("EFFECT_FLAME_{}", index),
    );

    // Shared radial glow (`effect/flare4`), additive, centred on the
    // flame. The exe loads `effect\Flare4.bmp` relative to `basedata`,
    // i.e. a sibling of the `EffectScn` root.
    let glow = spec
        .glow
        .then_some(())
        .and(effect_root.parent())
        .and_then(|basedata| {
            build_glow(
                factory,
                vfs,
                &basedata.join("effect").join("flare4.dds"),
                spec.glow_size,
                spec.glow_y,
                format!("EFFECT_GLOW_{}", index),
            )
        });

    // Parent the flame + glow under the holder so the caller-applied
    // position/rotation carries them. With no holder, the flame entity
    // is the root and the glow hangs off it.
    let root = match holder {
        Some(holder) => {
            if let Some(flame) = flame {
                holder.attach(flame);
            }
            holder
        }
        None => flame?,
    };
    if let Some(glow) = glow {
        root.attach(glow);
    }
    Some(root)
}

/// Build an additive, double-sided "crossed quads" billboard (two
/// perpendicular quads) carrying `atlas` as an additive texture. The
/// quads span `y0..y1` vertically and `width` horizontally, centred on
/// the local origin in X/Z. UVs map the full `[0, 1]` cell so a
/// [`FrameAnimationComponent`] can later address atlas cells.
pub(crate) fn build_frame_animated_billboard(
    factory: &Rc<dyn ComponentFactory>,
    atlas: image::RgbaImage,
    mat_name: &str,
    width: f32,
    y0: f32,
    y1: f32,
    name: String,
    blend: BlendMode,
) -> ComRc<IEntity> {
    let hw = width / 2.0;
    let verts = vec![
        Vec3::new(-hw, y0, 0.0),
        Vec3::new(hw, y0, 0.0),
        Vec3::new(hw, y1, 0.0),
        Vec3::new(-hw, y1, 0.0),
        Vec3::new(0.0, y0, -hw),
        Vec3::new(0.0, y0, hw),
        Vec3::new(0.0, y1, hw),
        Vec3::new(0.0, y1, -hw),
    ];
    // Cell-local UVs (origin top-left): bottom edge v = 1, top edge v = 0.
    let base_uv = [(0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)];
    let texcoords: Vec<TexCoord> = (0..8)
        .map(|i| {
            let (u, v) = base_uv[i % 4];
            TexCoord::new(u, v)
        })
        .collect();
    let mut indices: Vec<u32> = vec![];
    for q in 0..2u32 {
        let b = q * 4;
        // Double-sided: emit both windings so cull mode is irrelevant.
        indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
        indices.extend_from_slice(&[b, b + 2, b + 1, b, b + 3, b + 2]);
    }

    let material = SimpleMaterialDef::create_with_image(mat_name, Some(atlas))
        .with_blend(blend)
        .make_unique();
    let geometry = Geometry::new(
        &verts,
        None,
        std::slice::from_ref(&texcoords),
        indices,
        material,
    );

    let entity = CoreEntity::create(name, true);
    let mesh = StaticMeshComponent::new(entity.clone(), vec![geometry], factory.clone());
    entity.add_component(IStaticMeshComponent::uuid(), ComRc::from_object(mesh));
    entity
}

/// Decode a PAL3 `.dds`/`.tga` texture, flipping it upright (PAL3 DDS
/// are bottom-up D3D9 textures, like the rest of the loaders).
pub(crate) fn load_effect_texture(vfs: &MiniFs, path: &Path) -> Option<image::RgbaImage> {
    load_texture(vfs, path)
}

fn load_texture(vfs: &MiniFs, path: &Path) -> Option<image::RgbaImage> {
    let mut file = vfs.open(path).ok()?;
    let mut bytes = vec![];
    file.read_to_end(&mut bytes).ok()?;
    Some(image::load_from_memory(&bytes).ok()?.flipv().to_rgba8())
}

/// Build an additive, frame-animated flame billboard entity.
///
/// `frame_files` are decoded, packed into a `cols × rows` atlas, and
/// shown one cell at a time by a [`FrameAnimationComponent`].
/// `base_y`/`width`/`height` are in the effect's local space (base at
/// y = 0).
fn build_flame(
    factory: &Rc<dyn ComponentFactory>,
    vfs: &MiniFs,
    dir: &Path,
    frame_files: &[String],
    cols: u32,
    rows: u32,
    width: f32,
    height: f32,
    base_y: f32,
    fps: f32,
    name: String,
) -> Option<ComRc<IEntity>> {
    let mut frames = vec![];
    for f in frame_files {
        frames.push(load_texture(vfs, &dir.join(f))?);
    }
    if frames.is_empty() {
        return None;
    }

    let (fw, fh) = frames[0].dimensions();
    let mut atlas = image::RgbaImage::new(fw * cols, fh * rows);
    for (i, fr) in frames.iter().enumerate() {
        let col = (i as u32) % cols;
        let row = (i as u32) / cols;
        image::imageops::replace(&mut atlas, fr, col * fw, row * fh);
    }

    // The texture store is keyed by name, so the name has to say which
    // frames these are: a shared literal handed every later flame the
    // first one's atlas.
    let texture_name = format!(
        "effect_flame:{}:{}",
        dir.display(),
        frame_files.first().map(String::as_str).unwrap_or_default()
    );
    let entity = build_frame_animated_billboard(
        factory,
        atlas,
        &texture_name,
        width,
        base_y,
        base_y + height,
        name,
        BlendMode::Additive,
    );

    let anim =
        FrameAnimationComponent::create(entity.clone(), cols, rows, frames.len() as u32, fps);
    entity.add_component(
        IFrameAnimationComponent::uuid(),
        anim.query_interface::<IComponent>().unwrap(),
    );

    Some(entity)
}

/// Build the shared static radial-glow billboard (`effect/flare4`), an
/// additive square of side `size` centred at height `center_y`.
fn build_glow(
    factory: &Rc<dyn ComponentFactory>,
    vfs: &MiniFs,
    path: &Path,
    size: f32,
    center_y: f32,
    name: String,
) -> Option<ComRc<IEntity>> {
    let atlas = load_texture(vfs, path)?;
    let half = size / 2.0;
    Some(build_frame_animated_billboard(
        factory,
        atlas,
        &format!("effect_glow:{}", path.display()),
        size,
        center_y - half,
        center_y + half,
        name,
        BlendMode::Additive,
    ))
}

#[cfg(test)]
mod tests {
    use super::effect_spec;

    #[test]
    fn only_three_of_the_five_variants_get_a_glow() {
        // FUN_004b7980：`if (id != 2 && id != 4 && id != 0) goto done;`
        // ——1 和 3 连 Flare4 都不加载。
        for (id, want) in [(0, true), (1, false), (2, true), (3, false), (4, true)] {
            let spec = effect_spec(id).unwrap_or_else(|| panic!("id {id} 没有 spec"));
            assert_eq!(spec.glow, want, "id {id} 的光晕");
        }
    }

    #[test]
    fn only_the_candle_and_the_lamp_have_a_holder() {
        // 同一段：id 0 也建了 gbGeomNode，但一个 .pol 都没加载。
        assert_eq!(effect_spec(0).unwrap().holder_pol, None);
        assert_eq!(effect_spec(1).unwrap().holder_pol, None);
        assert_eq!(
            effect_spec(2).unwrap().holder_pol,
            Some("Candle/candle.pol")
        );
        assert_eq!(effect_spec(3).unwrap().holder_pol, None);
        assert_eq!(effect_spec(4).unwrap().holder_pol, Some("Lamp/lamp.pol"));
    }

    #[test]
    fn every_variant_has_sixteen_frames() {
        // `do { ... } while (iVar2 < 0x10)`。
        for id in 0..5 {
            let spec = effect_spec(id).unwrap();
            assert_eq!(spec.frames.len(), 16, "id {id} 的帧数");
        }
        // 灯没有自己的帧，用蜡烛的（三位补零）。
        assert_eq!(effect_spec(4).unwrap().flame_folder, "Candle");
        assert_eq!(effect_spec(4).unwrap().frames[0], "001.dds");
        assert_eq!(effect_spec(0).unwrap().frames[0], "torch01.dds");
    }

    #[test]
    fn there_are_exactly_five_variants() {
        assert!(effect_spec(5).is_none());
        assert!(effect_spec(99).is_none());
    }
}
