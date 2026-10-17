//! 菜单页的界面粒子（参考源码 `gamemainui/uiparticle.cpp` 的 `UIParticle`）。只搬了菜单页用到的五种
//! 运动：`PAR_RISE`、`PAR_RAINBOW`、`PAR_RISE_HELIX`、`PAR_SURROUND_CW` / `CCW`。
//!
//! 原版每次 `Render` 先画（`RISE_HELIX` 是后画）再走一步，步长按帧算；调用方按每秒 60 步推进 [近似]。
//! 画法是 `UIRenderPar`：`ui_par_tex_color.gbf`，SRCALPHA / ONE 加法混合，顶点色就是粒子色和 alpha，
//! 32×32 贴在粒子位置为中心的方块上。

/// 运动方式（`UIPar_Type` 里用到的那几种）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Motion {
    Rise,
    Rainbow,
    RiseHelix,
    SurroundCw,
    SurroundCcw,
}

/// 这一步要画的方块。
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Quad {
    pub rect: (i32, i32, i32, i32),
    /// `gbColorQuad` 的 RGB（字节，`Set_Color` 给的 int 截成字节）。
    pub color: [u8; 3],
    pub alpha: u8,
}

#[derive(Clone, Debug)]
pub struct UiParticle {
    pub active: bool,
    fade: f32,
    x: f32,
    y: f32,
    xi: f32,
    yi: f32,
    xg: f32,
    yg: f32,
    size: (i32, i32),
    speed: f32,
    color: [u8; 3],
    alpha: i32,
    view: (i32, i32, i32, i32),
    w: i32,
    h: i32,
    got_bottom: bool,
    got_top: bool,
    rise_y: f32,
    xflag: u8,
    state: u8,
    color_fade: bool,
}

impl Default for UiParticle {
    fn default() -> Self {
        Self {
            active: false,
            fade: 1.,
            x: 0.,
            y: 0.,
            xi: 0.,
            yi: 0.,
            xg: 0.,
            yg: 0.,
            size: (0, 0),
            speed: 0.6,
            color: [0; 3],
            alpha: 255,
            view: (100, 100, 200, 400),
            w: 100,
            h: 200,
            got_bottom: false,
            got_top: false,
            rise_y: 0.,
            xflag: 0,
            state: 0,
            color_fade: true,
        }
    }
}

/// 原版 `rand() % n`；n 为 0 时原版会除零崩溃，这里当 0。
fn modulo(rand: &mut dyn FnMut() -> i32, n: i32) -> i32 {
    if n == 0 { 0 } else { rand() % n }
}

impl UiParticle {
    pub fn set_fade(&mut self, fade: f32) {
        self.fade = fade;
    }

    /// `Set_Color`：int 截成字节（`219 + rand()%10` 这类超过 255 的会绕回去）。
    pub fn set_color(&mut self, r: i32, g: i32, b: i32) {
        self.color = [r as u8, g as u8, b as u8];
    }

    pub fn set_alpha(&mut self, a: i32) {
        self.alpha = a as u8 as i32;
    }

    pub fn set_size(&mut self, w: i32, h: i32) {
        self.size = (w, h);
    }

    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed;
    }

    pub fn set_color_fade(&mut self, fade: bool) {
        self.color_fade = fade;
    }

    pub fn set_direction(&mut self, x: f32, y: f32) {
        self.xi = x;
        self.yi = y;
    }

    pub fn set_gravity(&mut self, x: f32, y: f32) {
        self.xg = x;
        self.yg = y;
    }

    /// `Set_Position`：以 (x, y) 为上边中点撑开 w×h 的范围。
    pub fn set_position(&mut self, x: f32, y: f32) {
        self.x = x;
        self.y = y;
        self.view = (
            (x - (self.w / 2) as f32) as i32,
            y as i32,
            (x + (self.w / 2) as f32) as i32,
            (y + self.h as f32) as i32,
        );
        self.rise_y = y + self.h as f32;
    }

    /// `Set_AreaSize(w, h)`。
    pub fn set_area_size(&mut self, w: i32, h: i32) {
        self.w = w;
        self.h = h;
        self.set_position_keep_view();
    }

    fn set_position_keep_view(&mut self) {
        let (x, y) = (self.x, self.y);
        self.view = (
            (x - (self.w / 2) as f32) as i32,
            y as i32,
            (x + (self.w / 2) as f32) as i32,
            (y + self.h as f32) as i32,
        );
        self.rise_y = y + self.h as f32;
    }

    /// `Set_AreaSize(RECT)`（环绕用）。
    pub fn set_area_rect(&mut self, r: (i32, i32, i32, i32)) {
        self.w = r.2 - r.0;
        self.h = r.3 - r.1;
        self.view = r;
        self.rise_y = self.y + self.h as f32;
    }

    fn quad(&self) -> Quad {
        let (rx, ry) = (self.size.0 / 2, self.size.1 / 2);
        Quad {
            rect: (
                (self.x - rx as f32) as i32,
                (self.y - ry as f32) as i32,
                (self.x + rx as f32) as i32,
                (self.y + ry as f32) as i32,
            ),
            color: self.color,
            alpha: self.alpha.clamp(0, 255) as u8,
        }
    }

    /// alpha 每步减 `by`，减到 0 就灭；返回灭了没有。
    fn decay(&mut self, by: f32) -> bool {
        let a = self.alpha as f32 - by;
        if a <= 0. {
            self.active = false;
            self.alpha = 0;
            true
        } else {
            self.alpha = a as i32;
            false
        }
    }

    /// x 跑出范围就随机拉回范围里。
    fn confine_x(&mut self, rand: &mut dyn FnMut() -> i32) {
        let width = self.view.2 - self.view.0;
        if self.x > self.view.2 as f32 {
            self.x = (self.view.2 - modulo(rand, width)) as f32;
        }
        if self.x < self.view.0 as f32 {
            self.x = (self.view.0 + modulo(rand, width)) as f32;
        }
    }

    /// `Render(motion)`：走一步，返回这一步画的方块。
    pub fn step(&mut self, motion: Motion, rand: &mut dyn FnMut() -> i32) -> Option<Quad> {
        if !self.active {
            return None;
        }
        let ry = (self.size.1 / 2) as f32;
        match motion {
            Motion::Rainbow => {
                let drawn = (!self.got_bottom).then(|| self.quad());
                let temp = self.xi as i32;
                self.x += (modulo(rand, temp * 2) - temp) as f32;
                self.xi += self.xg;
                self.confine_x(rand);
                if !self.got_bottom {
                    self.y += self.yi;
                    if self.y >= self.view.3 as f32 - ry {
                        self.got_bottom = true;
                        self.active = false;
                    }
                    self.yi += self.yg;
                    self.yi *= self.speed;
                }
                if self.decay(self.fade) {
                    self.got_bottom = false;
                }
                drawn
            }
            Motion::Rise => {
                let drawn = (!self.got_top).then(|| self.quad());
                self.x += self.xi;
                self.xi += self.xg * self.speed;
                self.confine_x(rand);
                if !self.got_top {
                    self.y = self.rise_y - self.yi;
                    if self.y <= self.view.1 as f32 + ry {
                        self.got_top = true;
                        self.active = false;
                    }
                    self.yi += self.yg * self.speed;
                }
                if self.decay(self.fade) {
                    self.got_top = false;
                }
                drawn
            }
            Motion::RiseHelix => {
                let temp = (self.xi * self.speed) as i32 as f32;
                match self.xflag {
                    0 => {
                        self.x += temp;
                        if self.x > self.view.2 as f32 {
                            self.xflag = 1;
                        }
                        if self.x < self.view.0 as f32 {
                            self.xflag = 2;
                        }
                    }
                    1 => {
                        self.x -= temp;
                        if self.x <= self.view.0 as f32 {
                            self.xflag = 2;
                        }
                    }
                    _ => {
                        self.x += temp;
                        if self.x >= self.view.2 as f32 {
                            self.xflag = 1;
                        }
                    }
                }
                self.y = self.rise_y - self.yi;
                if self.y <= self.view.1 as f32 + ry {
                    self.got_top = true;
                    self.active = false;
                }
                self.yi += self.yg * self.speed;
                if self.decay(self.fade) {
                    self.got_top = false;
                }
                (self.active || !self.got_top).then(|| self.quad())
            }
            Motion::SurroundCw | Motion::SurroundCcw => {
                let drawn = Some(self.quad());
                let (l, t, r, b) = (
                    self.view.0 as f32,
                    self.view.1 as f32,
                    self.view.2 as f32,
                    self.view.3 as f32,
                );
                let cw = motion == Motion::SurroundCw;
                // 顺时针：左 → 下 → 右 → 上；逆时针：右 → 下 → 左 → 上（原版注释的「顺/逆」和名字是反的）。
                match self.state {
                    0 => {
                        let more = if cw { self.x > l } else { self.x < r };
                        if more {
                            let d = self.xi * self.speed;
                            self.x += if cw { -d } else { d };
                            self.xi += self.xg;
                            self.y = t + self.yg;
                        } else {
                            self.state = 1;
                        }
                    }
                    1 => {
                        if self.y < b {
                            self.y += self.yi * self.speed;
                            self.yi += self.yg;
                            self.x = if cw { l } else { r } + self.xg;
                        } else {
                            self.state = 2;
                        }
                    }
                    2 => {
                        let more = if cw { self.x < r } else { self.x > l };
                        if more {
                            let d = self.xi * self.speed;
                            self.x += if cw { d } else { -d };
                            self.xi += self.xg;
                            self.y = b + self.yg;
                        } else {
                            self.state = 3;
                        }
                    }
                    _ => {
                        if self.y > t {
                            self.y -= self.yi * self.speed;
                            self.yi += self.yg;
                            self.x = if cw { r } else { l } + self.xg;
                        } else {
                            self.state = 0;
                        }
                    }
                }
                if self.color_fade {
                    self.decay(self.fade * self.speed);
                }
                drawn
            }
        }
    }
}

/// 按每秒 60 步推进的时钟：`tick(dt)` 返回这一帧该走几步。
#[derive(Default, Clone, Debug)]
pub struct StepClock {
    acc: f32,
}

impl StepClock {
    pub fn tick(&mut self, dt: f32) -> u32 {
        self.acc += dt.clamp(0., 0.25) * 60.;
        let n = self.acc.floor();
        self.acc -= n;
        n as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg() -> impl FnMut() -> i32 {
        let mut s = 7u32;
        move || {
            s = s.wrapping_mul(214013).wrapping_add(2531011);
            ((s >> 16) & 0x7fff) as i32
        }
    }

    #[test]
    fn rise_climbs_to_the_top_and_dies() {
        let mut rand = lcg();
        let mut p = UiParticle::default();
        p.set_size(32, 32);
        p.set_color(219, 36, 252);
        p.set_alpha(255);
        p.set_fade(2.);
        p.set_speed(0.5);
        p.set_area_size(65, 86);
        p.set_position(743., 72.);
        p.set_direction(1., 1.);
        p.set_gravity(2., 2.);
        p.active = true;
        let first = p.step(Motion::Rise, &mut rand).unwrap();
        assert_eq!(first.rect, (727, 56, 759, 88));
        let mut n = 1;
        while p.active && n < 1000 {
            p.step(Motion::Rise, &mut rand);
            n += 1;
        }
        assert!(!p.active && n < 100, "{n}");
    }

    #[test]
    fn surround_goes_round_the_rect() {
        let mut rand = lcg();
        let mut p = UiParticle::default();
        p.set_size(32, 32);
        p.set_alpha(255);
        p.set_fade(0.);
        p.set_speed(1.2);
        p.set_position(100., 10.);
        p.set_area_rect((80, 10, 140, 50));
        p.set_direction(2., 2.);
        p.active = true;
        let mut corners = vec![];
        for _ in 0..400 {
            let before = p.state;
            p.step(Motion::SurroundCcw, &mut rand);
            if p.state != before {
                corners.push(p.state);
            }
        }
        assert_eq!(&corners[..4], &[1, 2, 3, 0]);
    }

    #[test]
    fn color_bytes_wrap_like_the_original() {
        let mut p = UiParticle::default();
        p.set_color(219, 36, 260);
        assert_eq!(p.color, [219, 36, 4]);
    }

    #[test]
    fn clock_steps_at_sixty_hertz() {
        let mut c = StepClock::default();
        assert_eq!(c.tick(1. / 60.), 1);
        assert_eq!(c.tick(1. / 120.) + c.tick(1. / 120.), 1);
    }
}
