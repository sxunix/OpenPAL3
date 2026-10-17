//! 御剑页魔剑周围的粒子（参考源码 `swordui.cpp` 的 `SetSwordEffect` / `SetPurgeEffect` / `Render`），范围
//! (370,62) 80×368：
//!
//! - 入场转完、每次翻阴阳面：168 个 `star.tga` 一齐螺旋上升（`PAR_RISE_HELIX`），色 (222,98,38)+rand；
//! - 之后每 2 秒开 / 关一次，开着时每帧把一个灭了的格子补成 `flare.tga`（阴面紫 (138,84,224)、阳面橙
//!   (198,128,58)），也是螺旋上升；
//! - 确认净化时 168 个绿色 `spark.tga` 上升（`PAR_RISE`），回到主画面才画；
//! - 只在主画面、入场转完以后画（`m_CanCtrlBtn && m_bSwordEff`），净化界面里不画也不走。

use shared::openpal3::asset_manager::AssetManager;
use shared::openpal3::ui_particle::{Motion, Quad, StepClock, UiParticle};

use super::ui_draw::{Painter, Pics, Rect};

const RECT: Rect = (370, 62, 450, 430);
const COUNT: usize = 168;
const TOGGLE_SECS: f32 = 2.;

const STAR: &str = "UI\\GameMainUI\\Par_Effect\\star.tga";
const FLARE: &str = "UI\\GameMainUI\\Par_Effect\\flare.tga";
const SPARK: &str = "UI\\GameMainUI\\Par_Effect\\spark.tga";

#[derive(Clone, Default)]
struct Slot {
    p: UiParticle,
    tex: &'static str,
    last: Option<Quad>,
}

pub struct SwordFx {
    sword: Vec<Slot>,
    purge: Vec<Slot>,
    /// `m_bSwordEff`：入场转完了。
    on: bool,
    /// `m_bEff`：这 2 秒在补 flare。
    eff: bool,
    eff_t: f32,
    cur: usize,
    clock: StepClock,
    seed: u32,
    pics: Pics,
}

impl Default for SwordFx {
    fn default() -> Self {
        Self {
            sword: vec![Slot::default(); COUNT],
            purge: vec![Slot::default(); COUNT],
            on: false,
            eff: true,
            eff_t: 0.,
            cur: 0,
            clock: StepClock::default(),
            seed: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.subsec_nanos()),
            pics: Pics::default(),
        }
    }
}

impl SwordFx {
    fn rand(&mut self) -> i32 {
        self.seed = self.seed.wrapping_mul(214013).wrapping_add(2531011);
        ((self.seed >> 16) & 0x7fff) as i32
    }

    fn base(&mut self) -> UiParticle {
        let (w, h) = (RECT.2 - RECT.0, RECT.3 - RECT.1);
        let x = RECT.0 + self.rand() % (w / 2);
        let mut p = UiParticle::default();
        p.set_size(32, 32);
        p.set_speed(0.6);
        p.set_position(x as f32, RECT.1 as f32);
        p.set_area_size(w, h);
        p
    }

    fn rgb(&mut self, r: i32, g: i32, b: i32) -> (i32, i32, i32) {
        let (a, c, d) = (self.rand() % 10, self.rand() % 10, self.rand() % 10);
        (r + a, g + c, b + d)
    }

    /// 关页面再开（`m_bSwordEff = false`、`m_bEff = true`）。
    pub fn reset(&mut self) {
        self.on = false;
        self.eff = true;
        self.eff_t = 0.;
    }

    /// `SetSwordEffect`：入场转完 / 翻面。
    pub fn burst(&mut self) {
        self.on = true;
        for i in 0..COUNT {
            let mut p = self.base();
            let (r, g, b) = self.rgb(222, 98, 38);
            p.set_color(r, g, b);
            let a = self.rand() % 46;
            p.set_alpha(198 + a);
            p.set_fade(2.);
            let (dx, dy) = (5 + self.rand() % 6, 3 + self.rand() % 6);
            p.set_direction(dx as f32, dy as f32);
            let (gx, gy) = (6 + self.rand() % 6, 4 + self.rand() % 6);
            p.set_gravity(gx as f32, gy as f32);
            p.active = true;
            self.sword[i] = Slot {
                p,
                tex: STAR,
                last: None,
            };
        }
    }

    /// `SetPurgeEffect`：确认净化。
    pub fn purge(&mut self) {
        for i in 0..COUNT {
            let mut p = self.base();
            let (r, g, b) = self.rgb(68, 188, 98);
            p.set_color(r, g, b);
            let a = self.rand() % 10;
            p.set_alpha(228 + a);
            p.set_fade(2.);
            let (dx, dy) = (self.rand() % 5 + 1, self.rand() % 8 + 1);
            p.set_direction(dx as f32, dy as f32);
            let (gx, gy) = (self.rand() % 6 + 1, self.rand() % 12 + 1);
            p.set_gravity(gx as f32, gy as f32);
            p.active = true;
            self.purge[i] = Slot {
                p,
                tex: SPARK,
                last: None,
            };
        }
    }

    fn flare(&mut self, lunar: bool) -> UiParticle {
        let mut p = self.base();
        let (r, g, b) = if lunar {
            self.rgb(138, 84, 224)
        } else {
            self.rgb(198, 128, 58)
        };
        p.set_color(r, g, b);
        let a = self.rand() % 25;
        p.set_alpha(230 + a);
        let f = 4 + self.rand() % 2;
        p.set_fade(f as f32);
        p.set_direction(5., 3.);
        p.set_gravity(6., 4.);
        p.active = true;
        p
    }

    /// 每帧：`active` 是这一帧画不画（主画面、入场转完）；`lunar` 现在朝外的是不是阴面。
    pub fn paint(&mut self, p: &Painter, am: &AssetManager, dt: f32, active: bool, lunar: bool) {
        let steps = self.clock.tick(dt);
        if !active || !self.on {
            return;
        }
        self.pics.preload(
            am,
            &[STAR.to_string(), FLARE.to_string(), SPARK.to_string()],
        );
        self.eff_t += dt;
        if self.eff_t >= TOGGLE_SECS {
            self.eff_t = 0.;
            self.eff = !self.eff;
        }
        for _ in 0..steps {
            if self.eff {
                let c = self.cur;
                if !self.sword[c].p.active {
                    let flare = self.flare(lunar);
                    self.sword[c] = Slot {
                        p: flare,
                        tex: FLARE,
                        last: None,
                    };
                }
                self.cur = (self.cur + 1) % COUNT;
            }
            let mut seed = self.seed;
            let mut rand = || {
                seed = seed.wrapping_mul(214013).wrapping_add(2531011);
                ((seed >> 16) & 0x7fff) as i32
            };
            for i in 0..COUNT {
                if self.purge[i].p.active {
                    self.purge[i].last = self.purge[i].p.step(Motion::Rise, &mut rand);
                } else {
                    self.purge[i].last = None;
                }
                if self.sword[i].p.active {
                    self.sword[i].last = self.sword[i].p.step(Motion::RiseHelix, &mut rand);
                } else {
                    self.sword[i].last = None;
                }
            }
            self.seed = seed;
        }
        for s in self.purge.iter().chain(self.sword.iter()) {
            if let Some(q) = s.last {
                p.additive_tinted(
                    self.pics.peek(s.tex),
                    q.rect,
                    [q.color[0], q.color[1], q.color[2], q.alpha],
                );
            }
        }
    }
}
