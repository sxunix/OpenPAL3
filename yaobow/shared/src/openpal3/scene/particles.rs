//! `.par` 粒子的模拟与绘制 — `gbParticleGroup` 那一套。
//!
//! The format work is in `fileformats::pal3::par` and
//! `switch/re/notes/par_format.md`; this is the runtime half, written
//! against what those say:
//!
//! * `gbParticleGroup::Update` (0x1002b6a0) advances the emitter clock,
//!   asks `ComputeEmitterData` how many particles to make this frame,
//!   makes them, then steps every live one.
//! * `ComputeEmitterData` (0x1002a2c0) samples the emitter's four curves
//!   at its normalised progress; each gives a `(min, max)` from its first
//!   two components, and the emission count carries its fraction over
//!   between frames.
//! * `CreateOneParticle` (0x1002b2a0) rolls a lifetime, a spin direction
//!   and a random start frame, then places and aims the particle with
//!   `ComputeParticleInitPos` / `ComputeParticleInitVel`.
//! * `UpdateOneParticle` (0x1002b590) ages the particle, picks its
//!   texture frame, applies drag, samples the four look curves and adds
//!   gravity into the velocity.
//!
//! Particles live in the effect's own space; the caller places the
//! entity. `Render` (0x1002b770) draws each as a camera-facing quad, so
//! that is what the mesh below does.

use std::cell::RefCell;
use std::rc::Rc;

use crosscom::ComRc;
use fileformats::pal3::par::{ParBlend, ParEffect, ParGroupDef, sample};
use radiance::comdef::{IEntity, IEntityExt};
use radiance::components::mesh::{Geometry, TexCoord};
use radiance::math::{Vec2, Vec3};
use radiance::rendering::{BlendMode, ComponentFactory, ParticleMaterialDef, VertexComponents};
use radiance::scene::CoreEntity;

/// How many particles one group may have alive at once. The original
/// pools 0x6c78 bytes per emitter and drops the oldest when it runs out
/// (`GetOldestParticle`); this is the same idea with a fixed pool.
pub const MAX_PARTICLES: usize = 512;

/// One live particle, laid out like `gbParticleUnit`'s useful half.
#[derive(Clone, Copy)]
pub struct Particle {
    /// Seconds lived and total lifetime (`unit+0x08`, `unit+0x10`).
    pub age: f32,
    pub life: f32,
    /// `unit+0x34..0x3c` and `unit+0x1c..0x24`.
    pub position: Vec3,
    pub velocity: Vec3,
    /// `unit+0x14`, the texture frame.
    pub frame: u32,
    /// `unit+0x18`, the frame it started on.
    pub first_frame: u32,
    /// `unit+0x50`, which way it spins.
    pub spin: f32,
    /// `unit+0x4c`, the accumulated rotation in degrees.
    pub rotation: f32,
    /// Sampled each step: `unit+0x40` and the colour bytes.
    pub size: f32,
    pub color: [f32; 4],
}

impl Particle {
    const fn dead() -> Self {
        Self {
            age: 0.,
            life: 0.,
            position: Vec3 {
                x: 0.,
                y: 0.,
                z: 0.,
            },
            velocity: Vec3 {
                x: 0.,
                y: 0.,
                z: 0.,
            },
            frame: 0,
            first_frame: 0,
            spin: 1.,
            rotation: 0.,
            size: 0.,
            color: [0.; 4],
        }
    }

    pub fn alive(&self) -> bool {
        self.life > 0. && self.age < self.life
    }
}

/// The same xorshift the snow uses, so a scene looks the same twice.
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

/// One `gbParticleGroup` running.
pub struct ParticleGroup {
    def: ParGroupDef,
    /// 形状 2 的发射器撒在这份点云上，见 [`ParticleGroup::spawn_position`]。
    shape_points: Vec<[f32; 3]>,
    particles: RefCell<Vec<Particle>>,
    /// `group+0x40`, how long the emitter has been going.
    elapsed: RefCell<f32>,
    /// `gbEmitterData+0x28`, the fraction of a particle left over.
    pending: RefCell<f32>,
    rand: RefCell<Rand>,
    /// 不再发新粒子（战斗特效的寿命到了）；已有的照常走完。
    stopped: RefCell<bool>,
    /// 整组的缩放，见 [`ParticleEffect::set_scale`]。
    scale: RefCell<f32>,
}

impl ParticleGroup {
    pub fn new(def: ParGroupDef, seed: u32) -> Self {
        Self::with_shape(def, seed, vec![])
    }

    pub fn with_shape(def: ParGroupDef, seed: u32, shape_points: Vec<[f32; 3]>) -> Self {
        Self {
            def,
            shape_points,
            particles: RefCell::new(vec![Particle::dead(); MAX_PARTICLES]),
            elapsed: RefCell::new(0.),
            pending: RefCell::new(0.),
            rand: RefCell::new(Rand(seed | 1)),
            stopped: RefCell::new(false),
            scale: RefCell::new(1.),
        }
    }

    pub fn def(&self) -> &ParGroupDef {
        &self.def
    }

    pub fn blend(&self) -> ParBlend {
        self.def.param.blend
    }

    /// `gbParticleGroup::Update`.
    pub fn step(&self, delta_sec: f32) {
        let mut elapsed = self.elapsed.borrow_mut();
        *elapsed += delta_sec;
        let elapsed_now = *elapsed;
        drop(elapsed);

        self.emit(elapsed_now, delta_sec);
        self.advance(delta_sec);
    }

    /// `ComputeEmitterData` plus the `CreateOneParticle` loop.
    fn emit(&self, elapsed: f32, delta_sec: f32) {
        if *self.stopped.borrow() {
            return;
        }
        let emitter = &self.def.emitter;
        let cycles = (elapsed / emitter.period) as i32;
        if cycles > 1 && !emitter.looping {
            return;
        }
        let t = ((elapsed - cycles as f32 * emitter.period) / emitter.period).clamp(0., 1.);

        let pair = |(lo, hi): &(Vec<(f32, f32)>, Vec<(f32, f32)>)| {
            let a = sample(lo, t);
            let b = sample(hi, t);
            if a <= b { (a, b) } else { (b, a) }
        };
        let (rate_lo, rate_hi) = pair(&emitter.rate);
        let (life_lo, life_hi) = pair(&emitter.life);
        let (speed_lo, speed_hi) = pair(&emitter.speed);
        let direction = [
            sample(&emitter.direction[0], t),
            sample(&emitter.direction[1], t),
            sample(&emitter.direction[2], t),
        ];

        let count = {
            let mut rand = self.rand.borrow_mut();
            let mut pending = self.pending.borrow_mut();
            let exact = rand.range(rate_lo, rate_hi) * delta_sec + *pending;
            let whole = exact.max(0.) as i32;
            *pending = exact - whole as f32;
            whole
        };
        if count <= 0 {
            return;
        }

        let scale = *self.scale.borrow();
        let mut particles = self.particles.borrow_mut();
        let mut rand = self.rand.borrow_mut();
        let mut made = 0;
        for slot in particles.iter_mut() {
            if made == count {
                break;
            }
            if slot.alive() {
                continue;
            }
            // `ComputeParticleInitVel`（gbengine 0x1002a590，exe 里那份是
            // FUN_004b91e0）：速率 × 方向曲线的原值，再逐轴加 `rand(±扩散)`。
            // 方向不归一化，扩散是加在速度上的。
            let speed = rand.range(speed_lo, speed_hi);
            let spread = emitter.spread.map(|v| v * scale);
            let jitter = Vec3::new(
                rand.range(-spread[0], spread[0]),
                rand.range(-spread[1], spread[1]),
                rand.range(-spread[2], spread[2]),
            );
            let velocity = Vec3::new(
                speed * direction[0] + jitter.x,
                speed * direction[1] + jitter.y,
                speed * direction[2] + jitter.z,
            );

            *slot = Particle {
                age: 0.,
                life: rand.range(life_lo, life_hi).max(f32::EPSILON),
                position: self.spawn_position(&mut rand, scale),
                velocity,
                frame: 0,
                first_frame: (rand.range(0., self.def.param.first_frame_range as f32)) as u32,
                spin: if rand.next() < 0.5 { -1. } else { 1. },
                rotation: 0.,
                size: 0.,
                color: [1., 1., 1., 1.],
            };
            made += 1;
        }
    }

    /// `gbPartEmitter::ComputeParticleInitPos`（gbengine.dll 0x1002a400）。
    /// 三个分支，每个都把发射器原点加在最后：
    ///
    /// * 形状 0，盒：三个轴各自在 `[min, max]` 里取一个数。
    /// * 形状 1，球壳：先抛硬币决定半径上界取第一个半径还是两个半径之和
    ///   （0x1002a479 的 `test al,1`），在 `[0, 上界]` 里取半径，再取
    ///   `θ ∈ [0,180°]`、`φ ∈ [0,360°]`，按
    ///   `(r·sinφ·sinθ, r·cosθ, r·cosφ·sinθ)` 摆开——θ 是从 +y 量的极角。
    /// * 其余（形状 2）：在 `.csf` 网格的点云里随机挑一个点再过一次矩阵。
    ///   点云就是 [`fileformats::pal3::csf`]；`gbEmitterShape::get_vec`
    ///   （`engine/physics/gbparteffect.cpp:136`）是
    ///   `gbRand(0, 点数 − 1)` 挑一个点再过 `curmat`——那个矩阵只有
    ///   `set_transform` 会写，粒子这条路上没人调，所以是单位阵。
    ///   实机 65 个发射器里有 27 个是这一种；读不到 `.csf` 时退回原点。
    ///
    /// 缩放（exe 里那份的 FUN_004ba800）：形状上取的那一点乘发射器 `+0xb0`
    /// 的缩放，原点由 FUN_004bb0f0 预先乘过，所以两者都随缩放放大。
    fn spawn_position(&self, rand: &mut Rand, scale: f32) -> Vec3 {
        let e = &self.def.emitter;
        let o = Vec3::new(
            e.origin[0] * scale,
            e.origin[1] * scale,
            e.origin[2] * scale,
        );
        match e.shape {
            0 => Vec3::new(
                o.x + scale * rand.range(e.box_min[0], e.box_max[0]),
                o.y + scale * rand.range(e.box_min[1], e.box_max[1]),
                o.z + scale * rand.range(e.box_min[2], e.box_max[2]),
            ),
            1 => {
                let bound = if rand.next() < 0.5 {
                    e.radii.0
                } else {
                    e.radii.0 + e.radii.1
                };
                let r = scale * rand.range(0., bound);
                let theta = rand.range(0., 180f32).to_radians();
                let phi = rand.range(0., 360f32).to_radians();
                Vec3::new(
                    o.x + r * phi.sin() * theta.sin(),
                    o.y + r * theta.cos(),
                    o.z + r * phi.cos() * theta.sin(),
                )
            }
            _ => {
                if self.shape_points.is_empty() {
                    return o;
                }
                let k = (rand.next() * self.shape_points.len() as f32) as usize;
                let p = self.shape_points[k.min(self.shape_points.len() - 1)];
                Vec3::new(o.x + scale * p[0], o.y + scale * p[1], o.z + scale * p[2])
            }
        }
    }

    /// `UpdateOneParticle`.
    fn advance(&self, delta_sec: f32) {
        let param = &self.def.param;
        let mut particles = self.particles.borrow_mut();
        for p in particles.iter_mut() {
            if !p.alive() {
                continue;
            }
            p.age += delta_sec;
            if !p.alive() {
                p.life = 0.;
                continue;
            }
            let t = (p.age / p.life).clamp(0., 1.);

            // 贴图第几帧。
            let frame = (param.frames as f32 * t) as u32;
            p.frame = (p.first_frame + frame) % param.frames.max(1);

            // 空气阻力：速度低于阈值就不管。
            let speed = p.velocity.norm();
            if speed > param.drag_speed && speed > f32::EPSILON {
                let scale = if param.drag_by_size { p.size } else { 1. };
                let drag = param.drag * scale * delta_sec;
                let factor = (1. - drag / speed).clamp(0., 1.);
                p.velocity = Vec3::scalar_mul(factor, &p.velocity);
            }

            // 四条曲线。
            p.size = sample(&param.size, t);
            p.rotation += sample(&param.rotation, t) * p.spin * delta_sec;
            p.color = [
                sample(&param.color[0], t) / 255.,
                sample(&param.color[1], t) / 255.,
                sample(&param.color[2], t) / 255.,
                sample(&param.alpha, t),
            ];

            // 位置，然后把重力并进速度。
            p.position = Vec3::add(&p.position, &Vec3::scalar_mul(delta_sec, &p.velocity));
            p.velocity.y -= param.gravity * delta_sec;
        }
    }

    /// The live particles, for the mesh. `size` comes out already
    /// multiplied by the group's scale: the exe's Render (FUN_004bc8a0)
    /// takes half the size track's value times group param `+0x164`, which
    /// is where FUN_004bb180 stores the scale. Drag still sees the raw
    /// track value, as `unit+0x40` does.
    pub fn particles(&self) -> Vec<Particle> {
        let scale = *self.scale.borrow();
        self.particles
            .borrow()
            .iter()
            .filter(|p| p.alive())
            .map(|p| Particle {
                size: p.size * scale,
                ..*p
            })
            .collect()
    }

    pub fn alive_count(&self) -> usize {
        self.particles.borrow().iter().filter(|p| p.alive()).count()
    }
}

/// Every group of one `.par`, running together.
pub struct ParticleEffect {
    groups: Vec<ParticleGroup>,
}

impl ParticleEffect {
    pub fn new(effect: &ParEffect) -> Self {
        Self::with_shapes(effect, |_| None)
    }

    /// `load_shape` 按 `.par` 里写的名字（`csf\\star.csf` 这种，相对
    /// `effect\gbParticle` 目录）读那份点云；读不到就让形状 2 的发射器
    /// 退回原点。
    pub fn with_shapes(
        effect: &ParEffect,
        load_shape: impl Fn(&str) -> Option<Vec<[f32; 3]>>,
    ) -> Self {
        Self {
            groups: effect
                .groups
                .iter()
                .enumerate()
                .map(|(i, def)| {
                    let points = if def.emitter.shape > 1 && !def.emitter.mesh.is_empty() {
                        load_shape(&def.emitter.mesh).unwrap_or_default()
                    } else {
                        vec![]
                    };
                    ParticleGroup::with_shape(def.clone(), 0x9e37_79b9 ^ i as u32, points)
                })
                .collect(),
        }
    }

    pub fn step(&self, delta_sec: f32) {
        for group in &self.groups {
            group.step(delta_sec);
        }
    }

    pub fn groups(&self) -> &[ParticleGroup] {
        &self.groups
    }

    /// 整个特效放大 `scale` 倍：粒子尺寸、发射器原点、形状取点和扩散。
    ///
    /// 战斗特效（flexeff `particle` 类）的定义记录 `+200` 就是它：
    /// FUN_004c8b80 取出来交给 FUN_004bd7c0，后者对每个发射器调
    /// FUN_004bb0f0（原点、扩散先除旧值再乘新值，新值记在 `+0xb0`），对每个
    /// 组参数调 FUN_004bb180（写 `+0x164`，Render 拿它乘尺寸）。速度和
    /// 重力不乘。
    pub fn set_scale(&self, scale: f32) {
        for group in &self.groups {
            *group.scale.borrow_mut() = scale;
        }
    }

    /// 不再发新粒子（已有的走完为止）。
    pub fn stop(&self) {
        for group in &self.groups {
            *group.stopped.borrow_mut() = true;
        }
    }

    pub fn alive_count(&self) -> usize {
        self.groups.iter().map(|g| g.alive_count()).sum()
    }
}

/// Name of the entity one group is drawn as.
pub fn particle_entity_name(group: usize) -> String {
    format!("PARTICLE_{group}")
}

/// Pack a particle's colour the way `particle.vert` unpacks it: RGB as
/// `0xRRGGBB` in one float, alpha in the other.
pub fn pack_color(color: [f32; 4]) -> (f32, f32) {
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round();
    let packed = byte(color[0]) * 65536. + byte(color[1]) * 256. + byte(color[2]);
    (packed, color[3].clamp(0., 1.))
}

/// 粒子的顶点色。原版画粒子是 `SRCBLEND = ONE`、`DESTBLEND = ONE`，颜色只取贴图 ×
/// 顶点色的 RGB，alpha 不参与混合——VertexAlpha 轨道在原版里不影响画面。移植端的
/// 加法是「ONE/ONE + 着色器预乘 alpha」，所以这里把 alpha 钉成 1，结果就是原版的 ONE/ONE。
pub fn particle_color(color: [f32; 4]) -> (f32, f32) {
    pack_color([color[0], color[1], color[2], 1.])
}

/// 贴图的 alpha 同理不参与：整张改成不透明。存进纹理缓存时换个名字（`#rgb`），
/// 免得和按 alpha 混合的 quad 用到同一张图时互相顶掉。
pub fn opaque_particle_texture(
    texture_name: &str,
    mut texture: image::RgbaImage,
) -> (String, image::RgbaImage) {
    for px in texture.pixels_mut() {
        px.0[3] = 255;
    }
    (format!("{texture_name}#rgb"), texture)
}

/// Build the mesh one group draws through: `MAX_PARTICLES` quads whose
/// vertices are rewritten every frame, host-dynamic like the snow's.
///
/// 一律加法混合，不看 `.par` 里的混合标记：exe 自带的那份粒子系统画的
/// 时候（FUN_004bc8a0，0x4bcb75 起）无条件 `SetRenderState(ALPHABLENDENABLE,
/// 1)`、`SRCBLEND = ONE`、`DESTBLEND = ONE`，颜色是贴图 × 顶点色，组循环之前
/// 设一次、没有分支。`param+0x164` 选 `p_alpha.gbf` 那一支是 gbengine.dll
/// 里另一份的做法，exe 里连这两个 `.gbf` 的字符串都没有，而且它把 `+0x164`
/// 挪去存缩放了（FUN_004bb180）。三个标成 alpha 的 Snow 组在原版里也是加法，
/// 黑底因此不显。
pub fn build_particle_entity(
    factory: &Rc<dyn ComponentFactory>,
    group: usize,
    texture_name: &str,
    texture: image::RgbaImage,
) -> ComRc<IEntity> {
    let mut verts = Vec::with_capacity(MAX_PARTICLES * 4);
    let mut uv = Vec::with_capacity(MAX_PARTICLES * 4);
    let mut packed = Vec::with_capacity(MAX_PARTICLES * 4);
    let mut indices = Vec::with_capacity(MAX_PARTICLES * 6);
    for i in 0..MAX_PARTICLES as u32 {
        for _ in 0..4 {
            verts.push(Vec3::new(0., 0., 0.));
            uv.push(TexCoord::new(0., 0.));
            packed.push(TexCoord::new(0., 0.));
        }
        let b = i * 4;
        indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }

    let (texture_name, texture) = opaque_particle_texture(texture_name, texture);
    let material =
        ParticleMaterialDef::create_with_image(&texture_name, Some(texture), BlendMode::Additive);
    let geometry = Geometry::new(&verts, None, &[uv, packed], indices, material);

    let object = factory.create_render_object(
        geometry.vertices.clone(),
        geometry.indices.clone(),
        &geometry.material,
        true,
    );
    let component = factory.create_rendering_component(vec![object]);
    let entity = CoreEntity::create(particle_entity_name(group), true);
    entity.set_rendering_component(Some(Rc::new(component)));
    entity
}

/// Lay this frame's particles out as camera-facing quads.
///
/// `right` and `up` are the camera's axes; the quad corners are the
/// particle's position offset along them by half its size, turned by its
/// accumulated rotation. The texture is a horizontal strip of
/// `frames` cells, so the UV picks the particle's current cell.
pub fn write_particle_mesh(
    entity: &ComRc<IEntity>,
    particles: &[Particle],
    frames: u32,
    right: &Vec3,
    up: &Vec3,
) {
    let Some(rc) = entity.get_rendering_component() else {
        return;
    };
    let cell = 1. / frames.max(1) as f32;
    for object in rc.render_objects() {
        object.as_dyn().update_vertices(&|mut buffer| {
            for i in 0..MAX_PARTICLES {
                let quad = particles.get(i);
                let (corners, uvs, color) = match quad {
                    None => ([Vec3::new(0., 0., 0.); 4], [(0., 0.); 4], (0., 0.)),
                    Some(p) => {
                        let half = p.size / 2.;
                        let (sin, cos) = p.rotation.to_radians().sin_cos();
                        // 面片在相机平面里自转。
                        let axis_x = Vec3::add(
                            &Vec3::scalar_mul(cos * half, right),
                            &Vec3::scalar_mul(sin * half, up),
                        );
                        let axis_y = Vec3::add(
                            &Vec3::scalar_mul(-sin * half, right),
                            &Vec3::scalar_mul(cos * half, up),
                        );
                        let corner = |sx: f32, sy: f32| {
                            Vec3::add(
                                &p.position,
                                &Vec3::add(
                                    &Vec3::scalar_mul(sx, &axis_x),
                                    &Vec3::scalar_mul(sy, &axis_y),
                                ),
                            )
                        };
                        let u0 = p.frame as f32 * cell;
                        (
                            [
                                corner(-1., -1.),
                                corner(1., -1.),
                                corner(1., 1.),
                                corner(-1., 1.),
                            ],
                            [(u0, 1.), (u0 + cell, 1.), (u0 + cell, 0.), (u0, 0.)],
                            particle_color(p.color),
                        )
                    }
                };
                for k in 0..4 {
                    let position = corners[k];
                    let (u, v) = uvs[k];
                    buffer.set_component(i * 4 + k, VertexComponents::POSITION, |p: &mut Vec3| {
                        *p = position
                    });
                    buffer.set_component(i * 4 + k, VertexComponents::TEXCOORD, |t: &mut Vec2| {
                        *t = Vec2::new(u, v)
                    });
                    buffer.set_component(i * 4 + k, VertexComponents::TEXCOORD2, |t: &mut Vec2| {
                        *t = Vec2::new(color.0, color.1)
                    });
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_alpha_never_reaches_the_blend() {
        // 顶点色：RGB 照打包，alpha 一律 1。
        let (rgb, a) = particle_color([1., 0.5, 0., 0.2]);
        assert_eq!((rgb, a), (pack_color([1., 0.5, 0., 1.]).0, 1.));
        // 贴图：alpha 全改 255，RGB 不动，缓存名换掉。
        let mut img = image::RgbaImage::new(2, 1);
        img.put_pixel(0, 0, image::Rgba([10, 20, 30, 0]));
        img.put_pixel(1, 0, image::Rgba([40, 50, 60, 128]));
        let (name, out) = opaque_particle_texture("a.tga", img);
        assert_eq!(name, "a.tga#rgb");
        assert_eq!(out.get_pixel(0, 0).0, [10, 20, 30, 255]);
        assert_eq!(out.get_pixel(1, 0).0, [40, 50, 60, 255]);
    }
    use fileformats::pal3::par::{ParEmitter, ParGroupParam};

    fn constant(v: f32) -> Vec<(f32, f32)> {
        vec![(0., v), (1., v)]
    }

    fn group(rate: f32, life: f32) -> ParGroupDef {
        ParGroupDef {
            param: ParGroupParam {
                name: "test".into(),
                texture: String::new(),
                blend: ParBlend::Additive,
                frames: 4,
                first_frame_range: 1,
                gravity: 0.,
                drag_speed: 1e9,
                drag_by_size: false,
                drag: 0.,
                size: constant(1.),
                rotation: constant(0.),
                alpha: constant(1.),
                color: [constant(255.), constant(255.), constant(255.)],
            },
            emitter: ParEmitter {
                name: "test".into(),
                mesh: String::new(),
                origin: [0., 0., 0.],
                spread: [0., 0., 0.],
                shape: 0,
                box_min: [0., 0., 0.],
                box_max: [0., 0., 0.],
                radii: (0., 0.),
                period: 1.,
                looping: true,
                direction: [constant(0.), constant(1.), constant(0.)],
                speed: (constant(1.), constant(1.)),
                rate: (constant(rate), constant(rate)),
                life: (constant(life), constant(life)),
                size_mul: constant(1.),
                rotation_mul: constant(1.),
            },
        }
    }

    #[test]
    fn the_emission_rate_holds_across_frame_times() {
        // 每秒 100 颗，无论一帧走多久。
        let coarse = ParticleGroup::new(group(100., 10.), 1);
        coarse.step(0.1);
        assert_eq!(coarse.alive_count(), 10);

        let fine = ParticleGroup::new(group(100., 10.), 1);
        for _ in 0..10 {
            fine.step(0.01);
        }
        assert_eq!(fine.alive_count(), 10);
    }

    #[test]
    fn particles_die_at_their_lifetime() {
        let g = ParticleGroup::new(group(100., 0.25), 1);
        g.step(0.1);
        assert!(g.alive_count() > 0);
        for _ in 0..10 {
            g.step(0.1);
        }
        // 一直在发射，所以还有活的，但没有一颗能活过 0.25 秒。
        assert!(g.particles().iter().all(|p| p.age < 0.25));
    }

    #[test]
    fn a_one_shot_emitter_stops_after_its_period() {
        let mut def = group(100., 0.1);
        def.emitter.looping = false;
        let g = ParticleGroup::new(def, 1);
        for _ in 0..30 {
            g.step(0.1);
        }
        assert_eq!(g.alive_count(), 0, "不循环的发射器过了周期还在发");
    }

    #[test]
    fn gravity_pulls_the_velocity_down() {
        let mut def = group(100., 10.);
        def.param.gravity = 9.81;
        let g = ParticleGroup::new(def, 1);
        g.step(0.1);
        let before = g.particles()[0].velocity.y;
        g.step(0.1);
        assert!(g.particles()[0].velocity.y < before, "重力没把速度往下拉");
    }

    #[test]
    fn the_direction_curve_is_not_normalised() {
        // 方向 (0,10,0)、速率 2、无扩散：初速就是 (0,20,0)。
        let mut def = group(100., 10.);
        def.emitter.direction = [constant(0.), constant(10.), constant(0.)];
        def.emitter.speed = (constant(2.), constant(2.));
        let g = ParticleGroup::new(def, 1);
        g.step(0.01);
        let v = g.particles()[0].velocity;
        assert!((v.y - 20.).abs() < 1e-4 && v.x == 0. && v.z == 0., "{v:?}");
    }

    #[test]
    fn the_scale_grows_size_and_spawn_box_but_not_speed() {
        let mut def = group(100., 10.);
        def.emitter.box_min = [-1., 0., -1.];
        def.emitter.box_max = [1., 0., 1.];
        def.emitter.origin = [0., 2., 0.];
        let effect = ParticleEffect {
            groups: vec![ParticleGroup::new(def, 1)],
        };
        effect.set_scale(10.);
        effect.step(0.1);
        let ps = effect.groups()[0].particles();
        assert!(!ps.is_empty());
        for p in &ps {
            assert_eq!(p.size, 10., "尺寸曲线 1 × 缩放 10");
            assert!(p.position.x.abs() <= 10. + 1e-3 && p.position.z.abs() <= 10. + 1e-3);
            assert!((p.velocity.y - 1.).abs() < 1e-4, "速度不随缩放变");
        }
        // 盒子放大到 ±10，二十来颗里总有落在 ±1 之外的；原点抬到 20。
        assert!(ps.iter().any(|p| p.position.x.abs() > 1.));
        assert!(ps.iter().all(|p| p.position.y >= 20.));
    }

    #[test]
    fn the_pool_never_overflows() {
        let g = ParticleGroup::new(group(100_000., 100.), 1);
        for _ in 0..20 {
            g.step(0.1);
        }
        assert!(g.alive_count() <= MAX_PARTICLES);
    }
}
