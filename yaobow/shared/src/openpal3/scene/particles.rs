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

use fileformats::pal3::par::{ParBlend, ParEffect, ParGroupDef, sample};
use radiance::math::Vec3;

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
    particles: RefCell<Vec<Particle>>,
    /// `group+0x40`, how long the emitter has been going.
    elapsed: RefCell<f32>,
    /// `gbEmitterData+0x28`, the fraction of a particle left over.
    pending: RefCell<f32>,
    rand: RefCell<Rand>,
}

impl ParticleGroup {
    pub fn new(def: ParGroupDef, seed: u32) -> Self {
        Self {
            def,
            particles: RefCell::new(vec![Particle::dead(); MAX_PARTICLES]),
            elapsed: RefCell::new(0.),
            pending: RefCell::new(0.),
            rand: RefCell::new(Rand(seed | 1)),
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
            let speed = rand.range(speed_lo, speed_hi);
            let spread = emitter.spread;
            let aim = Vec3::new(
                direction[0] + rand.range(-spread[0], spread[0]),
                direction[1] + rand.range(-spread[1], spread[1]),
                direction[2] + rand.range(-spread[2], spread[2]),
            );
            let aim = if aim.norm() > f32::EPSILON {
                Vec3::normalized(&aim)
            } else {
                Vec3::new(0., 1., 0.)
            };

            *slot = Particle {
                age: 0.,
                life: rand.range(life_lo, life_hi).max(f32::EPSILON),
                position: self.spawn_position(&mut rand),
                velocity: Vec3::scalar_mul(speed, &aim),
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

    /// `ComputeParticleInitPos`. The three shapes need the box ranges at
    /// `emitter+0xac..0xc0`, the radii at `0xc4`/`0xc8` and the point
    /// list at `0xcc`; [`ParEmitter`] does not carry them yet, so every
    /// shape starts at the emitter's origin for now.
    ///
    /// [`ParEmitter`]: fileformats::pal3::par::ParEmitter
    fn spawn_position(&self, _rand: &mut Rand) -> Vec3 {
        let o = self.def.emitter.origin;
        Vec3::new(o[0], o[1], o[2])
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

    /// The live particles, for the mesh.
    pub fn particles(&self) -> Vec<Particle> {
        self.particles
            .borrow()
            .iter()
            .filter(|p| p.alive())
            .copied()
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
        Self {
            groups: effect
                .groups
                .iter()
                .enumerate()
                .map(|(i, def)| ParticleGroup::new(def.clone(), 0x9e37_79b9 ^ i as u32))
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
}

#[cfg(test)]
mod tests {
    use super::*;
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
                period: 1.,
                looping: true,
                direction: [constant(0.), constant(1.), constant(0.)],
                speed: (constant(1.), constant(1.)),
                rate: (constant(rate), constant(rate)),
                life: (constant(life), constant(life)),
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
    fn the_pool_never_overflows() {
        let g = ParticleGroup::new(group(100_000., 100.), 1);
        for _ in 0..20 {
            g.step(0.1);
        }
        assert!(g.alive_count() <= MAX_PARTICLES);
    }
}
