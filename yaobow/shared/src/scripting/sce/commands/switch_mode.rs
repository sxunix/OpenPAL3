//! 102 全局游戏模式切换 — `LAB_00436f90`.
//!
//! 指令表里这条记的是「选择当前渲染状态」，处理函数说的不是这回事。它只做
//! 一件事：
//!
//! ```text
//! FUN_0044f610(this = 0xdb9fd0, 取参数());
//! ```
//!
//! `FUN_0044f610` 是全局模式切换：把新模式存进 `[管理器+0x38c8]`、旧的存进
//! `+0x38cc`，再让 `FUN_0044f8d0(模式)` 造出该模式的状态对象放进 `+0x38c4`，
//! 之后每帧的更新、输入、绘制全走那个对象的虚表。战斗笔记里「战败走
//! `SwitchMode(0xf)` 回标题」说的就是它。
//!
//! 脚本里这条指令只出现 16 处，参数只有两种：**13** 在 11 个
//! 「从XX进入大地图」里，**0** 在 5 个 `T_mXX` 里。所以移植只需要认这几个
//! 模式：
//!
//! | 参数 | 原版 | 这里 |
//! |---|---|---|
//! | 0 | 回冒险模式 | 已经在冒险模式，什么都不用做 |
//! | 13 | 大地图 | [`BigMapScreen`] |
//! | 15 | 回标题 | 同 147，`request_title` |
//! | 其余 | 战斗/菜单/小游戏等 | 记一条日志放过 |
//!
//! 大地图的完整逆向在 `switch/re/notes/bigmap.md`，目的地表在
//! [`crate::openpal3::bigmap`]。这里照原版的行为做：
//!
//! * 1536×1024 的图在 800×600 的窗口里，鼠标贴边卷动（每秒 240 像素）；
//! * 状态 1 的目的地画出来但点了是「不能去“地名”」，状态 2 的弹「要去“地名”
//!   吗？」，选「是」后小人 `Spirit` 从当前场景那个目的地走过去（每秒 120
//!   像素，两轴各自走），到了跑 `bigmap.sce` 的 `i + 100` 号过程；
//! * 125 新点亮的目的地进图时闪两轮。
//!
//! 手柄：十字键在亮着的目的地之间挑（画面跟着挪到它），A 确认、B 选「否」。
//! 这一套是移植端加的——原版确实是纯鼠标：`UIBigMap::Update`
//! （`gameui/UIBigMap.cpp:938`）里只有鼠标，目的地是一个个
//! `UIButtonEXA`，命中判定还要采贴图那一点的 alpha ≥ 6
//! （`ui/uibutton.cpp:764`），所以点的是图标的不透明部分、不是矩形。
//!
//! 悬停的样子原版也有：`UIButtonEXA::Render`（`ui/uibutton.cpp:640`）
//! 在 `MOUSEON` 这一支走 `UIRenderQuad(..., m_color, tex, mode = 1, info)`，
//! mode 1 取 `UI_EFFECT_TEX_VCOLOR` = `gfxscript\ui_tex_vcolor.gbf`
//! ——普通 alpha 混合，但 `ColorOp = ADD`，**把顶点色加到贴图上**。
//! 那个顶点色是每个目的地自己的一种暗色，在 `UIBigMap.cpp` 建控件时逐个
//! `SetColor` 写死（雪岭镇 (95,71,51)、德阳 (58,80,113)、蜀山 (40,76,93)……
//! 38 个各不相同）。imgui 只会乘不会加，照搬要给每个图标再烘一张
//! 「贴图 + 这个颜色」的精灵；这里先用描一圈边顶着，是有意选的。

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use super::appraise::{overlay, virtual_mouse};
use super::dlg_sel::frame_pieces;
use super::encamp::draw_sprite;
use crate::openpal3::assis_lang;
use crate::openpal3::bigmap::{
    self as bm, DESTINATIONS, Destination, ELEMENT_DIR, MAP_DIR, SCE_PATH, SCENE_CODES,
    SCENE_NAME_TXT, SPIRIT_DIR, SPIRIT_FPS, SPIRIT_SIZE, TILE, TILES_X, TILES_Y, VIEW_H, VIEW_W,
    proc_of,
};
use crate::openpal3::loaders::sce_loader::sce_load_from_file;
use crate::openpal3::ui_tex::UiTextures;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{MouseButton, Ui};
use radiance::comdef::ISceneManager;
use radiance::imgui::{GameFontSize, game_font};
use radiance::input::Key;
use radiance::rendering::Sprite;

/// 回冒险模式。
pub const MODE_ADVENTURE: i32 = 0;
/// 标题画面——147 走的也是这个（原版 `SwitchMode(4)` 建标题状态对象，
/// 15 是「结束当前游戏回标题」）。
pub const MODE_TITLE: i32 = 15;
/// 大地图。
pub const MODE_BIGMAP: i32 = 13;

type Tex = Option<Rc<Sprite>>;
type Rect = (i32, i32, i32, i32);

/// 两个对话框的 rect（`0x449858` 起），种类 2 是/否、种类 3 提示。
const DIALOG: Rect = (250, 200, 550, 300);
/// 种类 2 的底图宽 0x134、高 0x6d（`FUN_004a16c0`）。
const CONFIRM_W: i32 = 0x134;
const CONFIRM_H: i32 = 0x6d;
/// 按钮 89×25，离底边 6、离左右边 0x1a。
const BUTTON_W: i32 = 89;
const BUTTON_H: i32 = 25;
/// 对话框的字色 RGB(0xff,0xfb,0x1f)（`FUN_004a14d0(0x1f,0xfb,0xff,0xff)`）。
const DIALOG_INK: [f32; 4] = [1.0, 0xfb as f32 / 255., 0x1f as f32 / 255., 1.0];

/// AssisLang 里两句话的前后半截。
const LANG_PATH: &str = "/basedata/basedata/AssisLang.lang";
const LANG_GO: (usize, usize) = (354, 355);
const LANG_CANT: (usize, usize) = (381, 382);

fn confirm_rect() -> Rect {
    (
        DIALOG.0,
        DIALOG.1,
        DIALOG.0 + CONFIRM_W,
        DIALOG.1 + CONFIRM_H,
    )
}

fn yes_rect() -> Rect {
    let c = confirm_rect();
    let y = c.3 - BUTTON_H - 6;
    (c.0 + 0x1a, y, c.0 + 0x1a + BUTTON_W, y + BUTTON_H)
}

fn no_rect() -> Rect {
    let c = confirm_rect();
    let y = c.3 - BUTTON_H - 6;
    (c.2 - 0x1a - BUTTON_W, y, c.2 - 0x1a, y + BUTTON_H)
}

fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 as f32 && x < r.2 as f32 && y >= r.1 as f32 && y < r.3 as f32
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Dialog {
    /// 「要去“地名”吗？」，`yes` 是手柄挑中的那个钮。
    Confirm { index: usize, yes: bool },
    /// 「不能去“地名”」，按任意键关。
    Refuse,
}

/// 新点亮的目的地闪两轮（`FUN_0044a2e0`）：alpha 每帧 ±10，0 → 255 为升。
#[derive(Debug, Clone, Copy)]
struct Flash {
    alpha: i32,
    rising: bool,
    rounds: i32,
}

impl Flash {
    fn step(&mut self) {
        if self.rounds <= 0 {
            return;
        }
        if self.rising {
            self.alpha += 10;
            if self.alpha > 254 {
                self.alpha = 255;
                self.rising = false;
                self.rounds -= 1;
            }
        } else {
            self.alpha -= 10;
            if self.alpha < 1 {
                self.alpha = 0;
                self.rising = true;
            }
        }
    }
}

/// 大地图那一屏。
struct BigMapScreen {
    tiles: Vec<Tex>,
    icons: Vec<Tex>,
    spirit: HashMap<&'static str, [Tex; 2]>,
    dlg_bg: Tex,
    yes_tex: [Tex; 3],
    no_tex: [Tex; 3],
    note: Vec<Tex>,
    /// 这次进图时画出来的目的地（状态 1、2）。
    visible: Vec<usize>,
    enterable: Vec<bool>,
    flashing: Vec<usize>,
    flash: Flash,
    names: Vec<String>,
    lang: Vec<String>,
    /// 视口左上角在 1536×1024 图上的位置。
    view: (f32, f32),
    cursor: usize,
    /// 用过十字键之后才画选中描边。
    pad_used: bool,
    avatar: (f32, f32),
    facing: &'static str,
    anim: f32,
    walking: Option<usize>,
    dialog: Option<Dialog>,
    last_mouse: (f32, f32),
    /// 进图第一帧看到的鼠标位置；鼠标离开过它才算有人在用鼠标。没鼠标的
    /// 机器（Switch）或者鼠标一直停在窗口角上时，不能因为「贴着边」一直卷。
    mouse_origin: Option<(f32, f32)>,
    mouse_active: bool,
}

impl BigMapScreen {
    fn load(state: &mut SceState) -> Self {
        let asset_mgr = state.asset_mgr().clone();
        let tiles = (0..TILES_X * TILES_Y)
            .map(|n| asset_mgr.load_sprite(format!("{MAP_DIR}/BigMap{n}.tga")))
            .collect();

        let (visible, enterable) = {
            let ps = state.global_state().persistent_state();
            let visible: Vec<usize> = (0..DESTINATIONS.len())
                .filter(|i| ps.bigmap_element_visible(*i as i32))
                .collect();
            let enterable: Vec<bool> = (0..DESTINATIONS.len())
                .map(|i| ps.bigmap_element_enterable(i as i32))
                .collect();
            (visible, enterable)
        };
        // 新点亮的读一次就清掉（原版闪完两轮清 `0xdba0ac`）。
        let flashing: Vec<usize> = state
            .global_state_mut()
            .persistent_state_mut()
            .take_bigmap_new()
            .into_iter()
            .filter_map(|i| usize::try_from(i).ok())
            .filter(|i| visible.contains(i))
            .collect();

        let icons = DESTINATIONS
            .iter()
            .enumerate()
            .map(|(i, d)| {
                visible
                    .contains(&i)
                    .then(|| asset_mgr.load_sprite(format!("{ELEMENT_DIR}/{}.tga", d.texture)))
                    .flatten()
            })
            .collect();
        let spirit = ["LU", "RU", "LD", "RD"]
            .into_iter()
            .map(|f| {
                let tex =
                    |k: usize| asset_mgr.load_sprite(format!("{SPIRIT_DIR}/Spirit_{f}{k}.tga"));
                (f, [tex(0), tex(1)])
            })
            .collect();
        let mut ui_tex = UiTextures::new(&asset_mgr);
        let mut three = |base: &str| -> [Tex; 3] {
            std::array::from_fn(|k| ui_tex.get(&asset_mgr, &format!("{base}{k}.tga")))
        };
        let yes_tex = three("UI\\GameMainUI\\MUIDialog\\Normal\\Yes");
        let no_tex = three("UI\\GameMainUI\\MUIDialog\\Normal\\No");
        let dlg_bg = ui_tex.get(
            &asset_mgr,
            "UI\\GameMainUI\\MUIDialog\\Normal\\DlgBackground.tga",
        );
        let note = (0..9)
            .map(|i| asset_mgr.load_sprite(format!("/basedata/basedata/ui/flex/note{i}.tga")))
            .collect();

        let vfs = asset_mgr.vfs();
        let scene_names = read_gbk(vfs, SCENE_NAME_TXT);
        let names = (0..DESTINATIONS.len())
            .map(|i| {
                scene_names
                    .as_deref()
                    .and_then(|t| bm::place_name(t, SCENE_CODES[i]))
                    .unwrap_or_else(|| DESTINATIONS[i].name.to_string())
            })
            .collect();
        let lang = read_gbk(vfs, LANG_PATH)
            .map(|t| assis_lang::parse_section(&t, "sec1"))
            .unwrap_or_default();

        // 起点：当前场景对应的那个目的地（`FUN_00449b90`）。小人左上角放在它的
        // 中心，再把图挪到它的右下角进窗口（`FUN_00443ce0(x2, y2)`）。
        let scene = state
            .global_state()
            .persistent_state()
            .scene_name()
            .unwrap_or_default();
        let start = bm::index_of_scene(&scene);
        let (avatar, view) = match start {
            Some(i) => {
                let d = &DESTINATIONS[i];
                let (cx, cy) = bm::center(d);
                (
                    (cx as f32, cy as f32),
                    bm::reveal((0., 0.), d.x + d.width, d.y + d.height),
                )
            }
            None => ((0., 0.), (0., 0.)),
        };
        let cursor = start
            .filter(|i| visible.contains(i))
            .or_else(|| visible.first().copied())
            .unwrap_or(0);

        Self {
            tiles,
            icons,
            spirit,
            dlg_bg,
            yes_tex,
            no_tex,
            note,
            visible,
            enterable,
            flash: Flash {
                alpha: if flashing.is_empty() { 255 } else { 0 },
                rising: true,
                rounds: if flashing.is_empty() { 0 } else { 2 },
            },
            flashing,
            names,
            lang,
            view,
            cursor,
            pad_used: false,
            avatar,
            facing: "LU",
            anim: 0.,
            walking: None,
            dialog: None,
            last_mouse: (-1., -1.),
            mouse_origin: None,
            mouse_active: false,
        }
    }

    fn text(&self, slot: usize, fallback: &str) -> String {
        self.lang
            .get(slot)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
    }

    fn screen_rect(&self, d: &Destination) -> Rect {
        let (x, y) = (d.x - self.view.0 as i32, d.y - self.view.1 as i32);
        (x, y, x + d.width, y + d.height)
    }

    fn hovered(&self, mx: f32, my: f32) -> Option<usize> {
        // 后画的盖在上面，所以倒着找。
        self.visible
            .iter()
            .rev()
            .copied()
            .find(|&i| inside(self.screen_rect(&DESTINATIONS[i]), mx, my))
    }

    /// 点了第 `i` 个（`0x44a5b2` 起）：能去弹确认框，不能去弹提示。
    fn activate(&mut self, i: usize) {
        if self.walking.is_some() {
            return;
        }
        self.dialog = Some(if self.enterable.get(i).copied().unwrap_or(false) {
            Dialog::Confirm {
                index: i,
                yes: true,
            }
        } else {
            Dialog::Refuse
        });
        self.cursor = i;
    }

    fn start_walk(&mut self, i: usize) {
        let target = bm::center(&DESTINATIONS[i]);
        let from = (self.avatar.0 as i32, self.avatar.1 as i32);
        if let Some(f) = bm::facing(from, target) {
            self.facing = f;
        }
        self.walking = Some(i);
    }

    fn reveal_cursor(&mut self) {
        let d = &DESTINATIONS[self.cursor];
        self.view = bm::reveal(self.view, d.x, d.y);
        self.view = bm::reveal(self.view, d.x + d.width, d.y + d.height);
    }

    /// 一帧。返回 `Some(i)` = 小人走到了第 `i` 个，该跑过程了。
    fn frame(&mut self, ui: &Ui, state: &SceState, dt: f32) -> Option<usize> {
        self.anim += dt;
        self.flash.step();
        if self.flash.rounds <= 0 {
            self.flashing.clear();
        }

        let (mx, my) = virtual_mouse(ui);
        let origin = *self.mouse_origin.get_or_insert((mx, my));
        if (mx, my) != origin {
            self.mouse_active = true;
        }
        let moved = self.mouse_active && (mx, my) != self.last_mouse;
        self.last_mouse = (mx, my);
        let click = ui.is_mouse_clicked(MouseButton::Left);
        let keys = Keys::poll(state);

        // 小人在走：什么都不理，到了就交出去。
        if let Some(i) = self.walking {
            let target = bm::center(&DESTINATIONS[i]);
            self.avatar = bm::walk(self.avatar, target, dt);
            if (self.avatar.0 as i32, self.avatar.1 as i32) == target {
                // 到了换成朝上的站姿（`FUN_0044a1b0`：朝左 LU、朝右 RU）。
                self.facing = if self.facing.starts_with('R') {
                    "RU"
                } else {
                    "LU"
                };
                self.draw(ui, None);
                return Some(i);
            }
            self.draw(ui, None);
            return None;
        }

        match self.dialog {
            Some(Dialog::Refuse) => {
                if click || keys.any {
                    self.dialog = None;
                }
            }
            Some(Dialog::Confirm { index, yes }) => {
                let on_yes = inside(yes_rect(), mx, my);
                let on_no = inside(no_rect(), mx, my);
                if moved && (on_yes || on_no) {
                    self.dialog = Some(Dialog::Confirm { index, yes: on_yes });
                }
                if keys.left || keys.right {
                    self.dialog = Some(Dialog::Confirm { index, yes: !yes });
                }
                let chosen = if click && (on_yes || on_no) {
                    Some(on_yes)
                } else if keys.confirm {
                    Some(yes)
                } else if keys.cancel {
                    Some(false)
                } else {
                    None
                };
                match chosen {
                    Some(true) => {
                        self.dialog = None;
                        self.start_walk(index);
                    }
                    Some(false) => self.dialog = None,
                    None => {}
                }
            }
            None => {
                if self.mouse_active {
                    self.scroll(mx, my, dt);
                }
                let hovered = self.hovered(mx, my);
                if let (true, Some(i)) = (moved, hovered) {
                    self.cursor = i;
                    self.pad_used = false;
                }
                if click {
                    if let Some(i) = hovered {
                        self.activate(i);
                    }
                } else if !self.visible.is_empty() {
                    let n = self.visible.len();
                    let at = self
                        .visible
                        .iter()
                        .position(|i| *i == self.cursor)
                        .unwrap_or(0);
                    if keys.right || keys.down {
                        self.cursor = self.visible[(at + 1) % n];
                        self.pad_used = true;
                        self.reveal_cursor();
                    } else if keys.left || keys.up {
                        self.cursor = self.visible[(at + n - 1) % n];
                        self.pad_used = true;
                        self.reveal_cursor();
                    } else if keys.confirm {
                        self.activate(self.cursor);
                    }
                }
            }
        }

        let hover = self.hovered(mx, my);
        self.draw(ui, hover);
        None
    }

    /// 鼠标贴到窗口边上时卷（`FUN_00443fc0`）。
    fn scroll(&mut self, mx: f32, my: f32, dt: f32) {
        let step = bm::SCROLL_SPEED * dt;
        let (mut dx, mut dy) = (0., 0.);
        if mx <= 0. {
            dx = -step;
        } else if mx >= (VIEW_W - 1) as f32 {
            dx = step;
        }
        if my <= 0. {
            dy = -step;
        } else if my >= (VIEW_H - 1) as f32 {
            dy = step;
        }
        if dx != 0. || dy != 0. {
            self.view = bm::clamp_view(self.view.0 + dx, self.view.1 + dy);
        }
    }

    fn draw(&self, ui: &Ui, hover: Option<usize>) {
        overlay(ui, "BigMap", |ui, at, _scale| {
            let list = ui.get_window_draw_list();
            let (vx, vy) = (self.view.0 as i32, self.view.1 as i32);
            for (n, tile) in self.tiles.iter().enumerate() {
                let (x, y) = bm::tile_origin(n as i32);
                let r = (x - vx, y - vy, x - vx + TILE, y - vy + TILE);
                draw_sprite(&list, tile.as_deref(), r, at);
            }
            let alpha = self.flash.alpha as f32 / 255.;
            for &i in &self.visible {
                let r = self.screen_rect(&DESTINATIONS[i]);
                let Some(icon) = self.icons[i].as_deref() else {
                    continue;
                };
                if self.flashing.contains(&i) {
                    let (uv0, uv1) = super::encamp::sprite_uv(icon, r);
                    list.add_image(icon.imgui_texture_id(), at(r.0, r.1), at(r.2, r.3))
                        .uv_min(uv0)
                        .uv_max(uv1)
                        .col([1., 1., 1., alpha])
                        .build();
                } else {
                    draw_sprite(&list, Some(icon), r, at);
                }
                let lit = hover == Some(i) || (self.pad_used && i == self.cursor);
                if lit && self.walking.is_none() {
                    // 原版是把目的地自己那种暗色「加」到图标上（见模块开头），
                    // 这里先描一圈边顶着。
                    list.add_rect(at(r.0, r.1), at(r.2, r.3), [1., 0.92, 0.45, 1.])
                        .thickness(2.)
                        .build();
                }
            }

            // 小人：两帧、每秒 5 帧。
            let frame = (self.anim * SPIRIT_FPS) as usize % 2;
            if let Some(frames) = self.spirit.get(self.facing) {
                let (x, y) = (self.avatar.0 as i32 - vx, self.avatar.1 as i32 - vy);
                draw_sprite(
                    &list,
                    frames[frame].as_deref(),
                    (x, y, x + SPIRIT_SIZE, y + SPIRIT_SIZE),
                    at,
                );
            }

            let _font = game_font(GameFontSize::SMALL).map(|f| ui.push_font(f));
            match self.dialog {
                Some(Dialog::Confirm { index, yes }) => {
                    let c = confirm_rect();
                    draw_sprite(&list, self.dlg_bg.as_deref(), c, at);
                    let (hover_yes, hover_no) = (yes, !yes);
                    let y = yes_rect();
                    draw_sprite(
                        &list,
                        self.yes_tex[usize::from(hover_yes)].as_deref(),
                        y,
                        at,
                    );
                    let n = no_rect();
                    draw_sprite(&list, self.no_tex[usize::from(hover_no)].as_deref(), n, at);
                    let text = format!(
                        "{}{}{}",
                        self.text(LANG_GO.0, "要去“"),
                        self.names[index],
                        self.text(LANG_GO.1, "”吗？")
                    );
                    list.add_text(at(c.0 + 6 + 8, c.1 + 6 + 8), DIALOG_INK, text);
                }
                Some(Dialog::Refuse) => {
                    for (k, r) in frame_pieces(DIALOG).iter().enumerate() {
                        draw_sprite(&list, self.note.get(k).and_then(|t| t.as_deref()), *r, at);
                    }
                    let text = format!(
                        "{}{}{}",
                        self.text(LANG_CANT.0, "不能去“"),
                        self.names[self.cursor],
                        self.text(LANG_CANT.1, "”")
                    );
                    list.add_text(at(DIALOG.0 + 6 + 8, DIALOG.1 + 6 + 8), DIALOG_INK, text);
                }
                None => {}
            }
        });
    }
}

/// 这一帧按了什么（和买卖界面同一套键）。
struct Keys {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    confirm: bool,
    cancel: bool,
    any: bool,
}

impl Keys {
    fn poll(state: &SceState) -> Self {
        let input = state.input();
        let p = |k: Key| input.get_key_state(k).pressed();
        let up = p(Key::Up) || p(Key::GamePadDPadUp);
        let down = p(Key::Down) || p(Key::GamePadDPadDown);
        let left = p(Key::Left) || p(Key::GamePadDPadLeft);
        let right = p(Key::Right) || p(Key::GamePadDPadRight);
        let confirm = p(Key::Space) || p(Key::GamePadEast);
        let cancel = p(Key::Escape) || p(Key::GamePadSouth);
        let any = up
            || down
            || left
            || right
            || confirm
            || cancel
            || p(Key::GamePadNorth)
            || p(Key::GamePadWest);
        Self {
            up,
            down,
            left,
            right,
            confirm,
            cancel,
            any,
        }
    }
}

fn read_gbk(vfs: &mini_fs::MiniFs, path: &str) -> Option<String> {
    use common::store_ext::StoreExt2;
    use encoding::Encoding;
    let bytes = vfs.read_to_end(PathBuf::from(path)).ok()?;
    encoding::all::GBK
        .decode(&bytes, encoding::DecoderTrap::Ignore)
        .ok()
}

/// 102 SwitchMode.
pub struct SceCommandSwitchMode {
    mode: i32,
    screen: Option<BigMapScreen>,
    /// 读过一次就不再重试，读不到就直接放过，免得卡死脚本。
    tried: bool,
}

impl Clone for SceCommandSwitchMode {
    fn clone(&self) -> Self {
        Self::new(self.mode)
    }
}

impl std::fmt::Debug for SceCommandSwitchMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandSwitchMode")
            .field("mode", &self.mode)
            .finish()
    }
}

impl SceCommand for SceCommandSwitchMode {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        match self.mode {
            MODE_BIGMAP => self.big_map(ui, state, delta_sec),
            MODE_ADVENTURE => true,
            MODE_TITLE => {
                log::info!("SwitchMode: 15 回标题");
                state.request_title();
                true
            }
            other => {
                log::warn!("SwitchMode: 模式 {other} 没有对应实现，跳过");
                true
            }
        }
    }
}

impl SceCommandSwitchMode {
    pub fn new(mode: i32) -> Self {
        Self {
            mode,
            screen: None,
            tried: false,
        }
    }

    fn big_map(&mut self, ui: &Ui, state: &mut SceState, dt: f32) -> bool {
        if !self.tried {
            self.tried = true;
            let screen = BigMapScreen::load(state);
            if screen.visible.is_empty() {
                log::warn!("SwitchMode: 大地图上一个目的地都没亮，直接返回");
                return true;
            }
            log::info!("SwitchMode: 进入大地图，{} 个目的地", screen.visible.len());
            self.screen = Some(screen);
        }
        let Some(screen) = self.screen.as_mut() else {
            return true;
        };

        // 自动化跑图时不能停在这里等人点，取第一个能去的目的地。
        let picked = if state.fast_forward() {
            screen
                .visible
                .iter()
                .copied()
                .find(|i| screen.enterable[*i])
                .or_else(|| screen.visible.first().copied())
        } else {
            screen.frame(ui, state, dt)
        };
        let Some(index) = picked else {
            return false;
        };

        self.screen = None;
        Self::enter(state, index);
        true
    }

    /// `FUN_0044a4f0`：跑 `bigmap.sce` 的 `index + 100` 号过程，再回冒险
    /// 模式。回模式这一步在移植里是隐式的——目的地过程自己带着一条
    /// `102 0`。
    fn enter(state: &mut SceState, index: usize) {
        let Some(proc_id) = proc_of(index) else {
            return;
        };
        let name = DESTINATIONS
            .get(index)
            .map(|d: &Destination| d.name)
            .unwrap_or("?");
        let sce = Rc::new(sce_load_from_file(
            state.asset_mgr().vfs(),
            std::path::PathBuf::from(SCE_PATH),
        ));
        log::info!("SwitchMode: 大地图选中 {index} 「{name}」，跑 bigmap proc {proc_id}");
        if !state.call_proc_in(sce, "bigmap", proc_id) {
            log::warn!("SwitchMode: bigmap.sce 里没有 proc {proc_id}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_modes_the_scripts_actually_use() {
        // 全部 16 处 102 只传 13 和 0；15 是战斗那边走的回标题。
        assert_eq!((MODE_ADVENTURE, MODE_BIGMAP, MODE_TITLE), (0, 13, 15));
    }

    #[test]
    fn the_destination_procs_line_up_with_the_table() {
        assert_eq!(proc_of(0), Some(100));
        assert_eq!(proc_of(DESTINATIONS.len() - 1), Some(137));
    }
}
