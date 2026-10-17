//! 菜单页上的几组界面粒子（逻辑在 [`shared::openpal3::ui_particle`]），坐标都是 800×600 画布：
//!
//! - 工具栏：鼠标移到一个能点的按钮上（`UIWM_MOUSEON` → `SetParEffect`/`SetBtnParEffect`），按钮内框
//!   一圈 16 个火花顺时针、16 个逆时针绕，3 秒左右淡没，不再补；移开就停（`UIWM_MOUSEOFF`）。
//! - 头像：仙术页施法（`MagicUI::SetParEffect`）、道具页用道具（`MainPropUI::SetParEffect`）时目标
//!   头像上冒一组上升 + 一组「彩虹」（同一组参数的拷贝，按 `PAR_RAINBOW` 走）。
//! - 合成：等结果那 5 秒，(88,206) 80×80 里 98 个螺旋上升的火花，灭了就地再放（`MagicUI::Render`）。

use std::time::Instant;

use crosscom::ComRc;
use radiance::comdef::IUiHost;
use shared::openpal3::asset_manager::AssetManager;
use shared::openpal3::ui_particle::{Motion, Quad, StepClock, UiParticle};

use super::ui_draw::{Painter, Pics, Rect};

pub const SPARK: &str = "UI\\GameMainUI\\Par_Effect\\spark.tga";

const TB_BUTTONS: usize = 8;
const TB_PARTICLES: usize = 16;
/// `stp_Rect`：第一个按钮 (38, 546) 110×50，往右每个 +88。
const TB_X0: i32 = 38;
const TB_Y: i32 = 600 - (32 + 22);
const TB_STEP: i32 = 110 - 22;
const BTN_W: i32 = 110;
const BTN_H: i32 = 50;

const COMPOSE_RECT: Rect = (88, 206, 168, 286);
const COMPOSE_PARTICLES: usize = 98;

/// 一个粒子和它上一步画的方块（这一帧不走步时照画）。
#[derive(Clone, Default)]
struct Slot {
    p: UiParticle,
    last: Option<Quad>,
}

impl Slot {
    fn new(p: UiParticle) -> Self {
        Self { p, last: None }
    }

    fn advance(&mut self, motion: Motion, steps: u32, rand: &mut dyn FnMut() -> i32) {
        for _ in 0..steps {
            self.last = self.p.step(motion, rand);
        }
    }
}

/// 头像粒子的两种参数。
#[derive(Clone, Copy, Debug)]
pub enum HeadKind {
    /// 仙术页：8 个，色 (219,36,252)+rand%10，alpha 255，淡 2，速 0.5。
    Magic,
    /// 道具页：12 个，色 = 给的色 + rand%10，alpha 120..254，淡 1，速 0.8。
    Item([i32; 3]),
}

pub struct MenuParticles {
    pics: Pics,
    clock: StepClock,
    last: Option<Instant>,
    seed: u32,
    tb_cur: usize,
    tb_on: [bool; TB_BUTTONS],
    tb_hover: i32,
    tb: Vec<[Vec<Slot>; 2]>,
    /// 按角色号：每个 (上升, 彩虹)。
    heads: Vec<Vec<(Slot, Slot)>>,
    head_rects: [Option<Rect>; 5],
    compose: Vec<Slot>,
    compose_on: bool,
}

impl Default for MenuParticles {
    fn default() -> Self {
        Self {
            pics: Pics::default(),
            clock: StepClock::default(),
            last: None,
            seed: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.subsec_nanos()),
            tb_cur: 0,
            tb_on: [false; TB_BUTTONS],
            tb_hover: -1,
            tb: (0..TB_BUTTONS).map(|_| [vec![], vec![]]).collect(),
            heads: vec![vec![]; 5],
            head_rects: [None; 5],
            compose: vec![],
            compose_on: false,
        }
    }
}

/// 第 `i` 个工具栏按钮的粒子范围（`m_parRc`：按钮缩进左 35、右 36、上下 5）。
fn toolbar_rect(i: usize) -> Rect {
    let left = TB_X0 + TB_STEP * i as i32;
    (left + 35, TB_Y + 5, left + BTN_W - 36, TB_Y + BTN_H - 5)
}

impl MenuParticles {
    fn rand(&mut self) -> i32 {
        self.seed = self.seed.wrapping_mul(214013).wrapping_add(2531011);
        ((self.seed >> 16) & 0x7fff) as i32
    }

    /// `SetParEffect(flag)`（工具栏）。
    fn set_toolbar(&mut self, flag: usize) {
        self.tb_on = [false; TB_BUTTONS];
        self.tb_on[flag] = true;
        let r = toolbar_rect(flag);
        let w = r.2 - r.0;
        let mut a = vec![];
        for _ in 0..TB_PARTICLES {
            let mut p = UiParticle::default();
            p.set_size(32, 32);
            let (cr, cg, cb) = (self.rand() % 10, self.rand() % 10, self.rand() % 10);
            p.set_color(250 + cr, 66 + cg, 58 + cb);
            let al = self.rand() % 55;
            p.set_alpha(200 + al);
            p.set_color_fade(true);
            p.set_fade(1.);
            p.set_speed(1.2);
            let x = r.0 + self.rand() % w;
            p.set_position(x as f32, r.1 as f32);
            p.set_area_rect(r);
            let (dx, dy) = (self.rand() % 2 + 1, self.rand() % 2 + 1);
            p.set_direction(dx as f32, dy as f32);
            p.set_gravity(0., 0.);
            p.active = true;
            a.push(Slot::new(p));
        }
        self.tb[flag] = [a.clone(), a];
    }

    /// 工具栏：`hover` 是鼠标下那个能点的按钮（工具栏顺序 0..7），没有就 -1。
    pub fn toolbar_hover(&mut self, hover: i32) {
        if hover != self.tb_hover {
            if self.tb_hover >= 0 {
                self.tb_on[self.tb_hover as usize] = false;
            }
            if hover >= 0 && (hover as usize) < TB_BUTTONS {
                self.tb_cur = hover as usize;
                if !self.tb_on[self.tb_cur] {
                    self.set_toolbar(self.tb_cur);
                }
            }
            self.tb_hover = hover;
        }
    }

    /// 页面重新激活（`UIWM_ACTIVE`）：工具栏粒子全停。
    pub fn page_opened(&mut self) {
        self.tb_on = [false; TB_BUTTONS];
        self.tb_hover = -1;
    }

    /// 在 `role` 头上放一组（`SetParEffect(Role)`）。`head` 是头像 rect（`m_parRc`）。
    pub fn spawn_head(&mut self, role: usize, head: Rect, kind: HeadKind) {
        if role >= 5 {
            return;
        }
        self.head_rects[role] = Some(head);
        let x = head.0 + (head.2 - head.0) / 2;
        let (w, h) = (head.2 - head.0, head.3 - head.1);
        let n = match kind {
            HeadKind::Magic => 8,
            HeadKind::Item(_) => 12,
        };
        let mut set = vec![];
        for _ in 0..n {
            let mut p = UiParticle::default();
            p.set_size(32, 32);
            let (cr, cg, cb) = (self.rand() % 10, self.rand() % 10, self.rand() % 10);
            match kind {
                HeadKind::Magic => {
                    p.set_color(219 + cr, 36 + cg, 252 + cb);
                    p.set_alpha(255);
                    p.set_fade(2.);
                    p.set_speed(0.5);
                }
                HeadKind::Item(c) => {
                    p.set_color(c[0] + cr, c[1] + cg, c[2] + cb);
                    let al = self.rand() % 135;
                    p.set_alpha(120 + al);
                    p.set_fade(1.);
                    p.set_speed(0.8);
                }
            }
            p.set_position(x as f32, head.1 as f32);
            p.set_area_size(w, h);
            let (dx, dy) = (self.rand() % 2 + 1, self.rand() % 2 + 1);
            p.set_direction(dx as f32, dy as f32);
            let (gx, gy) = (self.rand() % 2 + 2, self.rand() % 2 + 2);
            p.set_gravity(gx as f32, gy as f32);
            p.active = true;
            set.push((Slot::new(p.clone()), Slot::new(p)));
        }
        self.heads[role] = set;
    }

    /// 合成开始等结果（`SetComParEffect`）/ 结束。
    pub fn set_compose(&mut self, on: bool) {
        if on && !self.compose_on {
            let r = COMPOSE_RECT;
            let (w, h) = (r.2 - r.0, r.3 - r.1);
            self.compose = (0..COMPOSE_PARTICLES)
                .map(|_| {
                    let mut p = self.compose_particle(r.0 + w / 2, r);
                    p.set_area_size(w, h);
                    Slot::new(p)
                })
                .collect();
        }
        self.compose_on = on;
    }

    fn compose_particle(&mut self, x: i32, r: Rect) -> UiParticle {
        let mut p = UiParticle::default();
        p.set_size(32, 32);
        let (cr, cg, cb) = (self.rand() % 10, self.rand() % 10, self.rand() % 10);
        p.set_color(219 + cr, 36 + cg, 252 + cb);
        let al = self.rand() % 135;
        p.set_alpha(120 + al);
        p.set_fade(1.);
        p.set_speed(0.6);
        p.set_position(x as f32, r.1 as f32);
        p.set_area_size(r.2 - r.0, r.3 - r.1);
        p.set_direction(10., 2.);
        let (gx, gy) = (self.rand() % 8 + 2, self.rand() % 8 + 2);
        p.set_gravity(gx as f32, gy as f32);
        p.active = true;
        p
    }

    /// 每帧：推进并画。`toolbar` / `heads` / `compose` 选这一页要画哪几组。
    pub fn paint(
        &mut self,
        ui: &ComRc<IUiHost>,
        am: &AssetManager,
        register: &dyn Fn(i64, imgui::TextureId),
        canvas: (f32, f32, f32),
        toolbar: bool,
        heads: bool,
    ) {
        let now = Instant::now();
        let dt = self
            .last
            .replace(now)
            .map_or(0., |t| now.duration_since(t).as_secs_f32());
        let steps = self.clock.tick(dt);
        self.pics.preload(am, &[SPARK.to_string()]);
        let mut seed = self.seed;
        let mut rand = move || {
            seed = seed.wrapping_mul(214013).wrapping_add(2531011);
            ((seed >> 16) & 0x7fff) as i32
        };
        let mut quads: Vec<Quad> = vec![];

        if heads {
            for role in self.heads.iter_mut() {
                for (rise, rainbow) in role.iter_mut() {
                    if rise.p.active || rise.last.is_some() {
                        rise.advance(Motion::Rise, steps, &mut rand);
                        quads.extend(rise.last);
                    }
                    if rainbow.p.active || rainbow.last.is_some() {
                        rainbow.advance(Motion::Rainbow, steps, &mut rand);
                        quads.extend(rainbow.last);
                    }
                }
            }
        }
        if self.compose_on {
            // 灭了的就地再放：x 在左半边随机，从上边起。
            let r = COMPOSE_RECT;
            let w = r.2 - r.0;
            for i in 0..self.compose.len() {
                if !self.compose[i].p.active {
                    let x = r.0 + self.rand() % (w / 2);
                    self.compose[i] = Slot::new(self.compose_particle(x, r));
                }
                self.compose[i].advance(Motion::RiseHelix, steps, &mut rand);
                quads.extend(self.compose[i].last);
            }
        }
        if toolbar && self.tb_on[self.tb_cur] {
            let [ccw, cw] = &mut self.tb[self.tb_cur];
            for s in ccw.iter_mut() {
                if s.p.active {
                    s.advance(Motion::SurroundCcw, steps, &mut rand);
                    quads.extend(s.last);
                }
            }
            for s in cw.iter_mut() {
                if s.p.active {
                    s.advance(Motion::SurroundCw, steps, &mut rand);
                    quads.extend(s.last);
                }
            }
        }
        self.seed = self.seed.wrapping_add(steps);

        let p = Painter {
            ui,
            register,
            ox: canvas.0,
            oy: canvas.1,
            scale: canvas.2,
        };
        let pic = self.pics.peek(SPARK);
        for q in quads {
            p.additive_tinted(pic, q.rect, [q.color[0], q.color[1], q.color[2], q.alpha]);
        }
    }
}
