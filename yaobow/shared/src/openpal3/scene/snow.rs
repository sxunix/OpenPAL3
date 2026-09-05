//! 下雪 — `gbSnow`, the drop group opcode 140 switches on.
//!
//! `PAL3.exe` builds it once at startup (0x405330):
//!
//! ```text
//! gbSnow::gbSnow(this, 500, &desc, "effect\\snow.tga", 2.0)
//! ```
//!
//! and the frame loop renders it after `gbSceneGraph::End` while the
//! flag at `0xbfda40` — the one 140 writes — is set. Everything below is
//! read out of `gbengine.dll`, which exports the whole class with its
//! C++ names: `gbDropGroup::gbDropGroup` at 0x10031e40 copies the nine
//! dwords of `gbDropDesc`, `gbDropGroup::CreateUnit` at 0x10031fe0
//! spawns them, `gbSnow::Update` at 0x100324d0 moves them and
//! `gbSnow::Draw` at 0x100325b0 emits two triangles each.
//!
//! The desc PAL3 passes for snow (0x405330, ascending from `&local_854`)
//! and what each field turns out to be:
//!
//! | 字段 | 雪 | 雨 | 含义 |
//! |---|---|---|---|
//! | 0 | 0.5 | 0.08 | 一片的边长（`Draw` 取一半当半径） |
//! | 1 | 100 | 100 | 生成盒的大小 |
//! | 2 | 200 | 500 | 每秒生成个数（整数） |
//! | 3,4 | 1, 3 | 0.4, 2 | 存活秒数的随机区间 |
//! | 5,6 | 6, 10 | 20, 40 | 下落速度区间（存进去时取负） |
//! | 7,8 | −10, 50 | −10, 50 | 生成高度区间 |
//!
//! Drops live in **camera space**: `CreateUnit` puts them at
//! `x ∈ [−R/2, R/2]`, `z ∈ [−R, −R/2]` (in front of the camera) and
//! `y ∈ [−10, 50]`, so the weather follows the view instead of the
//! world. `Update` integrates the velocity, counts the life down, and
//! every time `(int)(life · 15)` turns even it re-rolls the horizontal
//! velocity within `±2.0` — the float `gbSnow`'s constructor keeps at
//! `+0x50`, which is what makes flakes drift rather than fall straight.

use std::cell::RefCell;
use std::rc::Rc;

use crosscom::ComRc;

use radiance::comdef::{IEntity, IEntityExt};
use radiance::components::mesh::{Geometry, TexCoord};
use radiance::math::Vec3;
use radiance::rendering::{BlendMode, SimpleMaterialDef};
use radiance::scene::CoreEntity;

/// `gbSnow::gbSnow(this, 500, ...)` — the pool size.
pub const MAX_DROPS: usize = 500;

/// `gbDropDesc` as PAL3 fills it for snow.
pub const SIZE: f32 = 0.5;
pub const BOX: f32 = 100.0;
pub const RATE: i32 = 200;
pub const LIFE: (f32, f32) = (1.0, 3.0);
pub const FALL: (f32, f32) = (6.0, 10.0);
pub const SPAWN_Y: (f32, f32) = (-10.0, 50.0);

/// The constructor's last argument, the sideways drift bound.
pub const DRIFT: f32 = 2.0;

/// `gbSnow::Update`'s `(int)(life * 15) & 1` gate on re-rolling the
/// drift — 0x100f5b6c.
pub const DRIFT_TICKS: f32 = 15.0;

/// `gbDropGroup::Render` clamps its own frame delta at 0.2 s
/// (0x100f5b50) before spawning and updating.
pub const MAX_STEP: f32 = 0.2;

/// The texture `PAL3.exe` names at 0x583938.
pub const TEXTURE: &str = "snow.tga";

/// Name of the entity the flakes are drawn as.
pub const SNOW_ENTITY: &str = "SNOW";

/// One drop, laid out as `gbDropUnit`: life, velocity, position.
#[derive(Clone, Copy)]
struct Drop {
    life: f32,
    velocity: Vec3,
    position: Vec3,
}

impl Drop {
    const fn dead() -> Self {
        Self {
            life: 0.,
            velocity: Vec3 {
                x: 0.,
                y: 0.,
                z: 0.,
            },
            position: Vec3 {
                x: 0.,
                y: 0.,
                z: 0.,
            },
        }
    }
}

/// A tiny xorshift, so the look does not depend on the host's rng.
struct Rand(u32);

impl Rand {
    fn next(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x >> 8) as f32 / ((1u32 << 24) as f32)
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }
}

/// The falling-snow simulation, in camera space.
pub struct Snow {
    drops: RefCell<Vec<Drop>>,
    live: RefCell<usize>,
    /// `gbDropGroup::CreateUnit`'s spawn accumulator (`+0x40`).
    pending: RefCell<f32>,
    rand: RefCell<Rand>,
}

impl Default for Snow {
    fn default() -> Self {
        Self::new()
    }
}

impl Snow {
    /// Re-exports so callers do not have to name the module's consts.
    pub const MAX: usize = MAX_DROPS;
    pub const SIZE: f32 = SIZE;
    pub const TEXTURE_NAME: &'static str = TEXTURE;

    pub fn new() -> Self {
        Self {
            drops: RefCell::new(vec![Drop::dead(); MAX_DROPS]),
            live: RefCell::new(0),
            pending: RefCell::new(0.),
            rand: RefCell::new(Rand(0x1234_5678)),
        }
    }

    /// One frame of `gbDropGroup::Render`: spawn, then move.
    pub fn step(&self, delta_sec: f32) {
        let delta_sec = delta_sec.min(MAX_STEP);
        self.spawn(delta_sec);
        self.advance(delta_sec);
    }

    /// `gbDropGroup::CreateUnit`. The accumulator carries the fraction
    /// of a drop left over from the last frame, so the rate holds
    /// whatever the frame time is.
    fn spawn(&self, delta_sec: f32) {
        let mut pending = self.pending.borrow_mut();
        *pending += delta_sec;
        let mut n = (*pending * RATE as f32) as i32;
        if n <= 0 {
            return;
        }
        *pending -= n as f32 / RATE as f32;
        drop(pending);

        let mut drops = self.drops.borrow_mut();
        let mut rand = self.rand.borrow_mut();
        let mut live = self.live.borrow_mut();
        for slot in drops.iter_mut() {
            if n == 0 {
                break;
            }
            if slot.life > 0. {
                continue;
            }
            slot.position = Vec3::new(
                rand.range(-0.5 * BOX, 0.5 * BOX),
                rand.range(SPAWN_Y.0, SPAWN_Y.1),
                rand.range(-BOX, -0.5 * BOX),
            );
            slot.velocity = Vec3::new(0., -rand.range(FALL.0, FALL.1), 0.);
            slot.life = rand.range(LIFE.0, LIFE.1);
            *live += 1;
            n -= 1;
        }
    }

    /// `gbSnow::Update`.
    fn advance(&self, delta_sec: f32) {
        let mut drops = self.drops.borrow_mut();
        let mut rand = self.rand.borrow_mut();
        let mut live = self.live.borrow_mut();
        for slot in drops.iter_mut() {
            if slot.life <= 0. {
                continue;
            }
            slot.position = Vec3::add(&slot.position, &Vec3::scalar_mul(delta_sec, &slot.velocity));
            slot.life -= delta_sec;
            if ((slot.life * DRIFT_TICKS) as i32) & 1 == 0 {
                slot.velocity.x = rand.range(-DRIFT, DRIFT);
                slot.velocity.z = rand.range(-DRIFT, DRIFT);
            }
            if slot.life <= 0. {
                slot.life = 0.;
                *live = live.saturating_sub(1);
            }
        }
    }

    /// The live drops' camera-space positions, for the mesh.
    pub fn positions(&self) -> Vec<Vec3> {
        self.drops
            .borrow()
            .iter()
            .filter(|d| d.life > 0.)
            .map(|d| d.position)
            .collect()
    }
}

/// Build the entity the flakes are drawn as: `MAX_DROPS` quads whose
/// vertices are rewritten every frame. One mesh rather than one entity
/// per flake, which is also how `gbSnow::Draw` does it — a single
/// dynamic vertex buffer of `live * 6` vertices.
pub fn build_snow_entity(
    asset_mgr: &crate::openpal3::asset_manager::AssetManager,
) -> Option<ComRc<IEntity>> {
    let factory = asset_mgr.component_factory();
    // The exe names a `.tga`; the retail install ships `effect\snow.dds`.
    let texture = asset_mgr.read_picture(asset_mgr.effect_root().join(TEXTURE))?;

    let mut verts = Vec::with_capacity(MAX_DROPS * 4);
    let mut texcoords = Vec::with_capacity(MAX_DROPS * 4);
    let mut indices = Vec::with_capacity(MAX_DROPS * 6);
    for i in 0..MAX_DROPS as u32 {
        // Degenerate until the first update moves them somewhere.
        for _ in 0..4 {
            verts.push(Vec3::new(0., 0., 0.));
        }
        texcoords.push(TexCoord::new(0., 1.));
        texcoords.push(TexCoord::new(1., 1.));
        texcoords.push(TexCoord::new(1., 0.));
        texcoords.push(TexCoord::new(0., 0.));
        let b = i * 4;
        indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }

    // `gbDropGroup` 的构造把渲染效果设成 `gfxscript\p_add.gbf`
    // （0x10031ed3 处 push 0x10120210），也就是加法混合。
    let material = SimpleMaterialDef::create_with_image("snow", Some(texture))
        .with_blend(BlendMode::Additive)
        .make_unique();
    let geometry = Geometry::new(
        &verts,
        None,
        std::slice::from_ref(&texcoords),
        indices,
        material,
    );

    // Built by hand rather than through `StaticMeshComponent`, which
    // asks for a device-local buffer: the flakes are rewritten every
    // frame, so the vertex buffer has to be host-dynamic.
    let object = factory.create_render_object(
        geometry.vertices.clone(),
        geometry.indices.clone(),
        &geometry.material,
        true,
    );
    let component = factory.create_rendering_component(vec![object]);
    let entity = CoreEntity::create(SNOW_ENTITY.to_string(), true);
    entity.set_rendering_component(Some(Rc::new(component)));
    Some(entity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_desc_is_the_one_pal3_passes_for_snow() {
        assert_eq!(MAX_DROPS, 500);
        assert_eq!((SIZE, BOX, RATE), (0.5, 100.0, 200));
        assert_eq!(LIFE, (1.0, 3.0));
        assert_eq!(FALL, (6.0, 10.0));
        assert_eq!(SPAWN_Y, (-10.0, 50.0));
        assert_eq!(DRIFT, 2.0);
    }

    #[test]
    fn the_spawn_rate_holds_across_frame_times() {
        // 200 a second, whatever the frame length: one long step and
        // many short ones must produce the same count.
        let coarse = Snow::new();
        coarse.step(0.1);
        let a = coarse.positions().len();

        let fine = Snow::new();
        for _ in 0..10 {
            fine.step(0.01);
        }
        let b = fine.positions().len();
        assert_eq!(a, 20);
        assert_eq!(b, 20);
    }

    #[test]
    fn drops_fall_and_die() {
        let snow = Snow::new();
        snow.step(0.05);
        let before: Vec<Vec3> = snow.positions();
        assert!(!before.is_empty());
        let top = before[0].y;
        snow.step(0.05);
        assert!(snow.positions()[0].y < top, "雪片应当往下走");

        // Nothing outlives the longest life in the desc.
        for _ in 0..100 {
            snow.step(MAX_STEP);
        }
        let live = snow.positions().len();
        assert!(live <= MAX_DROPS);
    }

    #[test]
    fn the_pool_never_overflows() {
        let snow = Snow::new();
        for _ in 0..200 {
            snow.step(MAX_STEP);
        }
        assert!(snow.positions().len() <= MAX_DROPS);
    }
}
