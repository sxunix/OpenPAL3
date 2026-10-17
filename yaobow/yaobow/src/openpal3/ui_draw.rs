//! 在 p7 菜单页上，从 Rust 这边按原版控件的方式画东西（御剑页）。贴图按原版的名字取（先查
//! `UI_opt.tli` 图集，再找散文件，[`UiTextures`]），登记进界面纹理表后经 `IUiHost` 画；坐标都是
//! 800×600 画布上的像素。

use std::collections::HashMap;
use std::rc::Rc;

use crosscom::ComRc;
use radiance::comdef::IUiHost;
use radiance::rendering::Sprite;
use shared::openpal3::asset_manager::AssetManager;
use shared::openpal3::ui_tex::UiTextures;

pub type Rect = (i32, i32, i32, i32);

/// 一张贴图和它在界面纹理表里的 id。
pub struct Pic {
    com_id: i64,
    sprite: Rc<Sprite>,
}

impl Pic {
    pub fn width(&self) -> i32 {
        self.sprite.width() as i32
    }

    pub fn height(&self) -> i32 {
        self.sprite.height() as i32
    }
}

/// 按名字取过的贴图。
#[derive(Default)]
pub struct Pics {
    tex: Option<UiTextures>,
    cache: HashMap<String, Option<Pic>>,
}

impl Pics {
    /// 名字照原版写（`UI\GameMainUI\Sword\SwordFrame.tga`）。
    pub fn get(&mut self, am: &AssetManager, name: &str) -> Option<&Pic> {
        if !self.cache.contains_key(name) {
            let tex = self.tex.get_or_insert_with(|| UiTextures::new(am));
            let pic = tex.get(am, name).map(|sprite| Pic {
                com_id: super::service::next_ui_texture_com_id(),
                sprite,
            });
            self.cache.insert(name.to_string(), pic);
        }
        self.cache.get(name).and_then(|p| p.as_ref())
    }

    /// 先把一批名字取好（画的时候只读）。
    pub fn preload(&mut self, am: &AssetManager, names: &[String]) {
        for n in names {
            let _ = self.get(am, n);
        }
    }

    pub fn peek(&self, name: &str) -> Option<&Pic> {
        self.cache.get(name).and_then(|p| p.as_ref())
    }

    /// 用一张现成的图（存档缩略图）顶掉 `name` 这个名字。
    pub fn insert_image(&mut self, am: &AssetManager, name: &str, image: image::RgbaImage) {
        let pic = Pic {
            com_id: super::service::next_ui_texture_com_id(),
            sprite: am.sprite_from_image(image),
        };
        self.cache.insert(name.to_string(), Some(pic));
    }

    pub fn forget(&mut self, name: &str) {
        self.cache.remove(name);
    }
}

/// 这一帧按下了键盘上的 `key` 没有（imgui 帧外调用恒为 false）。
pub fn key_pressed(key: imgui::Key) -> bool {
    unsafe {
        !imgui::sys::igGetCurrentContext().is_null()
            && imgui::sys::igIsKeyPressed_Bool(key as imgui::sys::ImGuiKey, false)
    }
}

/// 这一帧画在哪。
pub struct Painter<'a> {
    pub ui: &'a ComRc<IUiHost>,
    pub register: &'a dyn Fn(i64, imgui::TextureId),
    pub ox: f32,
    pub oy: f32,
    pub scale: f32,
}

/// 界面上的字号（16 像素一格，`menu_chrome.p7` 的 `TXT`）。
pub const GLYPH: f32 = 16.;

impl Painter<'_> {
    pub fn at(&self, x: f32, y: f32) -> (f32, f32) {
        (self.ox + x * self.scale, self.oy + y * self.scale)
    }

    /// 屏幕坐标换回画布坐标。
    pub fn to_canvas(&self, sx: f32, sy: f32) -> (f32, f32) {
        ((sx - self.ox) / self.scale, (sy - self.oy) / self.scale)
    }

    fn quad(
        &self,
        p: &Pic,
        r: (f32, f32, f32, f32),
        uv: (f32, f32, f32, f32),
        col: Option<[f32; 4]>,
    ) {
        (self.register)(p.com_id, p.sprite.imgui_texture_id());
        let (x0, y0) = self.at(r.0, r.1);
        let (x1, y1) = self.at(r.2, r.3);
        match col {
            None => self
                .ui
                .image_rect(p.com_id as i32, x0, y0, x1, y1, uv.0, uv.1, uv.2, uv.3),
            Some(c) => self.ui.image_rect_additive(
                p.com_id as i32,
                x0,
                y0,
                x1,
                y1,
                uv.0,
                uv.1,
                uv.2,
                uv.3,
                c[0],
                c[1],
                c[2],
                c[3],
            ),
        }
    }

    /// 原尺寸贴在 (x, y)。
    pub fn blit(&self, p: Option<&Pic>, x: i32, y: i32) {
        if let Some(p) = p {
            let r = (
                x as f32,
                y as f32,
                (x + p.width()) as f32,
                (y + p.height()) as f32,
            );
            self.quad(p, r, (0., 0., 1., 1.), None);
        }
    }

    /// 贴进控件的 rect：界面图 1:1，取贴图左下角那一块（rect 比图大时整张贴、不拉伸）。
    pub fn blit_rect(&self, p: Option<&Pic>, r: Rect) {
        let Some(p) = p else { return };
        let (tw, th) = (p.width().max(1) as f32, p.height().max(1) as f32);
        let w = ((r.2 - r.0) as f32).min(tw);
        let h = ((r.3 - r.1) as f32).min(th);
        let rr = (r.0 as f32, r.1 as f32, r.0 as f32 + w, r.1 as f32 + h);
        self.quad(p, rr, (0., 1. - h / th, w / tw, 1.), None);
    }

    /// 整张拉伸进 rect。
    pub fn stretch(&self, p: Option<&Pic>, r: Rect) {
        if let Some(p) = p {
            let rr = (r.0 as f32, r.1 as f32, r.2 as f32, r.3 as f32);
            self.quad(p, rr, (0., 0., 1., 1.), None);
        }
    }

    /// 加法混合原尺寸贴（列表选中条，`ui_tex_color.gbf`，顶点色 168,168,168,200）。
    pub fn additive(&self, p: Option<&Pic>, x: i32, y: i32) {
        if let Some(p) = p {
            let r = (
                x as f32,
                y as f32,
                (x + p.width()) as f32,
                (y + p.height()) as f32,
            );
            let c = [168. / 255., 168. / 255., 168. / 255., 200. / 255.];
            self.quad(p, r, (0., 0., 1., 1.), Some(c));
        }
    }

    /// 加法混合整张拉进 rect，顶点色 `rgba`（界面粒子，`ui_par_tex_color.gbf`）。
    pub fn additive_tinted(&self, p: Option<&Pic>, r: Rect, rgba: [u8; 4]) {
        if let Some(p) = p {
            let rr = (r.0 as f32, r.1 as f32, r.2 as f32, r.3 as f32);
            let c = rgba.map(|v| v as f32 / 255.);
            self.quad(p, rr, (0., 0., 1., 1.), Some(c));
        }
    }

    /// 横向三段条（`UIFlexBar::HORIZONTAL`）：贴图切成宽 `cap` 的三列，左右各一段，中间拉伸。
    pub fn bar(&self, p: Option<&Pic>, r: Rect, cap: i32) {
        let Some(p) = p else { return };
        let tw = p.width().max(1) as f32;
        let th = p.height().max(1) as f32;
        let h = ((r.3 - r.1) as f32).min(th);
        let v0 = 1. - h / th;
        let cap = cap.min((r.2 - r.0) / 2);
        let cols = [
            (r.0, r.0 + cap, 0),
            (r.0 + cap, r.2 - cap, 1),
            (r.2 - cap, r.2, 2),
        ];
        for (x0, x1, col) in cols {
            if x0 >= x1 {
                continue;
            }
            let u0 = (col * cap) as f32 / tw;
            let u1 = (((col + 1) * cap) as f32 / tw).min(1.);
            let rr = (x0 as f32, r.1 as f32, x1 as f32, r.1 as f32 + h);
            self.quad(p, rr, (u0, v0, u1, 1.), None);
        }
    }

    /// 一行字，左上角在 (x, y)。
    pub fn text(&self, x: f32, y: f32, rgb: [u8; 3], s: &str) {
        let (sx, sy) = self.at(x, y);
        self.ui.text_at_sized(
            sx,
            sy,
            GLYPH * self.scale,
            rgb[0] as f32 / 255.,
            rgb[1] as f32 / 255.,
            rgb[2] as f32 / 255.,
            1.,
            s,
        );
    }

    /// 控件里的字（`UIStatic`）：从 rect 左上角缩 8 起，按宽度折行（汉字 16、ASCII 8），`\n` 换行，
    /// 超出 rect 底边的行不画。
    pub fn paragraph(&self, r: Rect, rgb: [u8; 3], s: &str) {
        let width = (r.2 - r.0 - 16).max(16);
        for (k, line) in wrap(s, width).iter().enumerate() {
            let y = r.1 + 8 + 16 * k as i32;
            if y + 16 > r.3 + 8 {
                break;
            }
            self.text((r.0 + 8) as f32, y as f32, rgb, line);
        }
    }

    pub fn over(&self, mx: f32, my: f32, r: Rect) -> bool {
        let (x, y) = self.to_canvas(mx, my);
        x >= r.0 as f32 && x < r.2 as f32 && y >= r.1 as f32 && y < r.3 as f32
    }
}

/// 一个字的宽：ASCII 8、其它 16。
pub fn char_w(c: char) -> i32 {
    if c.is_ascii() { 8 } else { 16 }
}

pub fn text_w(s: &str) -> i32 {
    s.chars().map(char_w).sum()
}

/// 按像素宽度折行，`\n` 硬换行。
pub fn wrap(s: &str, width: i32) -> Vec<String> {
    let mut out = vec![];
    for para in s.split('\n') {
        let mut line = String::new();
        let mut w = 0;
        for c in para.chars() {
            let cw = char_w(c);
            if w + cw > width && !line.is_empty() {
                out.push(std::mem::take(&mut line));
                w = 0;
            }
            line.push(c);
            w += cw;
        }
        out.push(line);
    }
    out
}

/// 原版的 `"%s%d"` 风格补空格到 `width` 个字节（`CheckPurge` 里那个 22，GBK：汉字两个字节）。
pub fn pad_bytes(s: &str, width: usize) -> String {
    let bytes: usize = s.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum();
    let mut out = s.to_string();
    for _ in bytes..width {
        out.push(' ');
    }
    out
}
