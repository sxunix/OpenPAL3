//! 系统页（参考源码 `gamemainui/SystemUI.cpp`，拖动条 `ui/uidragbar.cpp`，按钮 `ui/uibutton.cpp` /
//! `ui/uipushbutton.cpp`）。p7 那边只画底部工具栏，其余都在这里，坐标是 800×600 画布上的像素。
//!
//! - 36 条存档（`MAX_NOTE_NUM`），每条 610×92，从 (12, 40) 往下排，一次露 5 条（`SHOW_ITEM_NUM`），
//!   靠右边的翻页键、滑块和滚轮一条一条地挪；
//! - 顶上「储存进度 / 读取进度」两个按下式按钮切模式，存不了档时「储存」禁用、开页进读取；
//! - 单击一条选中（wa001），对选中的那条再点一下（原版是双击）或按回车：储存 → 「覆盖 / 存到这里吗」，
//!   读取 → 「读这个吗」/「空记录」，船迷宫（m04、m05）里不能读；
//! - 右边框里两条竖的音量条：左边音乐（往下拖变大）、右边音效（往上拖变大），拖的时候直接改音量；
//! - 「离开游戏」问一句（AssisLang 261），是就回标题。
//!
//! 开始菜单「旧的回忆」（`gameui/UITitle_JDHY.cpp`）是同一套存档条，只换了版式（[`Layout::Title`]）：
//! 全屏 `UI\Menu\JDHY\background.tga`，存档条从 (80, 64) 排，翻页键和滑块在 702 / 708，
//! 上下装饰条在 108，右下角 `ui\Menu\Sub_Exit` 退回标题；只能读，没有储存 / 读取按钮、音量条和
//! 时间金钱牌；确认框在 (250, 200)、空档提示框在 (300, 200)。

use crosscom::ComRc;
use radiance::comdef::IUiHost;
use shared::openpal3::asset_manager::AssetManager;

use super::muidialog::{self, Answer, Kind, MuiDialog};
use super::ui_draw::{Painter, Pics, Rect, key_pressed};

pub const NOTES: usize = 36;
const SHOW: i32 = 5;
const ITEM_W: i32 = 610;
const ITEM_H: i32 = 92;
const ITEM_X: i32 = 12;
const ITEM_Y: i32 = 40;

const FRAME: (i32, i32) = (670, 38);
const TIMEMONEY: (i32, i32) = (596, 0);
const MONEY_RECT: Rect = (618, -4, 738, 28);
const TIME_RECT: Rect = (668, 16, 748, 48);
const TOPBAR: (i32, i32) = (40, 23);
const BOTTOMBAR: (i32, i32) = (40, 500);
const SAVE_BTN: Rect = (146, 5, 264, 37);
const LOAD_BTN: Rect = (398, 5, 516, 37);
const EXIT_BTN: Rect = (696, 456, 769, 531);
/// `IDB_SYSTEM_PAGEUP` 用的是 `pgup` 图、在上面，点了列表往回挪（`MoveDw`）；另一个反过来。
const PGUP_BTN: Rect = (634, 38, 659, 66);
const PGDW_BTN: Rect = (634, 502, 659, 530);
const MUSIC_TEXT: (i32, i32) = (684, 368);
const EFFECT_TEXT: (i32, i32) = (726, 105);

const INK: [u8; 3] = [31, 251, 255];
/// `UIWnd` 的默认字色——存档编号没设颜色。
const ID_INK: [u8; 3] = [5, 5, 0];
const DLG_INK: [u8; 3] = [219, 219, 0];
const DLG_AT: (i32, i32) = (250, 200);

const SYS: &str = "UI\\GameMainUI\\System\\";
/// 「旧的回忆」的背景和退出按钮（`TITLE_JDHY_BACKGROUND`、`MENU_TEX_SUBEXIT`）。
const TITLE_BG: &str = "UI\\Menu\\JDHY\\background.tga";
const TITLE_EXIT: &str = "ui\\Menu\\Sub_Exit";
const HEADS: [&str; 5] = ["jt", "xj", "lk", "zx", "cq"];

/// 一条存档的摘要（`ArcBrief`）。
#[derive(Clone, Debug)]
pub struct Brief {
    pub time: f32,
    pub money: i32,
    pub scene: String,
    pub team: [bool; 5],
}

/// 用在哪：游戏里的系统页，还是开始菜单的「旧的回忆」。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    System,
    Title,
}

/// 两种版式里挪了位置的那些。
struct Geo {
    item: (i32, i32),
    pgup: Rect,
    pgdw: Rect,
    topbar: (i32, i32),
    bottombar: (i32, i32),
    exit: Rect,
}

impl Layout {
    fn geo(self) -> Geo {
        match self {
            Layout::System => Geo {
                item: (ITEM_X, ITEM_Y),
                pgup: PGUP_BTN,
                pgdw: PGDW_BTN,
                topbar: TOPBAR,
                bottombar: BOTTOMBAR,
                exit: EXIT_BTN,
            },
            // `UITitle_JDHY::Create` / `AdjustNoteItem`。
            Layout::Title => Geo {
                item: (80, 64),
                pgup: (702, 62, 727, 90),
                pgdw: (702, 528, 727, 556),
                topbar: (108, 46),
                bottombar: (108, 526),
                exit: (745, 537, 745 + 56, 537 + 62),
            },
        }
    }

    fn slide_bk(self) -> Rect {
        match self {
            Layout::System => (640, 66, 654, 503),
            Layout::Title => (708, 90, 722, 527),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    Save,
    Load,
}

/// 竖的拖动条（`UIDragBar`，`VERTICAL_DRAG`）。`right` 是 `DRAG_RIGHT`：手柄在条的右边、值从下往上涨。
#[derive(Clone, Copy, Debug)]
struct Drag {
    bk: Rect,
    sx: i32,
    sy: i32,
    t: i32,
    b: i32,
    drag_radius: i32,
    max_val: f32,
    right: bool,
    progress: f32,
    val: f32,
    /// `DRAG_OFF` / `DRAG_ON` / `DRAG_DOWN`，也是手柄贴图的下标。
    state: u8,
    dragging: bool,
}

const RADIUS: i32 = 2;

impl Drag {
    fn new(bk: Rect, size: (i32, i32), t: i32, b: i32, drag_radius: i32, max_val: f32) -> Self {
        Self {
            bk,
            sx: size.0,
            sy: size.1,
            t,
            b,
            drag_radius,
            max_val,
            right: false,
            progress: 0.,
            val: 0.,
            state: 0,
            dragging: false,
        }
    }

    fn length(&self) -> f32 {
        ((self.bk.3 - RADIUS + self.b) - (self.bk.1 + RADIUS + self.t)) as f32
    }

    fn step(&self) -> f32 {
        self.max_val / self.length()
    }

    fn seek(&mut self, val: f32) {
        self.val = val;
        self.progress = val / self.step();
    }

    fn handle(&self) -> Rect {
        let progress = self.progress as i32;
        let (left, top) = if self.right {
            (
                self.bk.2 - self.sx + self.drag_radius,
                self.bk.3 - self.sy / 2 - RADIUS + self.b - progress,
            )
        } else {
            (
                self.bk.0 - self.sx + self.drag_radius,
                self.bk.1 - self.sy / 2 + RADIUS + self.t + progress,
            )
        };
        (left, top, left + self.sx, top + self.sy)
    }

    /// `UIDragBar::Update`；返回这一帧有没有在拖（原版每帧都发 `UIWM_DRAGBAR_CHANGE`）。
    fn update(&mut self, mx: f32, my: f32, held: bool, dy: f32) -> bool {
        let r = self.handle();
        let inside = mx >= r.0 as f32 && mx < r.2 as f32 && my >= r.1 as f32 && my < r.3 as f32;
        if self.state != 1 && held {
            let h = (r.3 - r.1) as f32;
            if my > r.3 as f32 + h || my < r.1 as f32 - h {
                self.dragging = false;
                self.state = 0;
            }
        } else if inside {
            self.state = 1;
            if held {
                self.state = 2;
                self.dragging = true;
            } else {
                self.dragging = false;
            }
        } else if self.state == 2 {
            self.state = 0;
            self.dragging = true;
        } else {
            self.state = 0;
            self.dragging = false;
        }
        if self.dragging {
            // 两个方向都夹在 [0, 长度] 里，到头时值正好是 0 / 最大。
            let d = if self.right { -dy } else { dy };
            self.progress = (self.progress + d).clamp(0., self.length());
            self.val = self.progress * self.step();
        }
        self.dragging
    }
}

/// 一条存档的显示开关（原版每个控件各自 `ShowWindow`，几处改法不一样，照搬）。
#[derive(Clone, Copy, Default, Debug)]
struct Shown {
    bright: bool,
    dark: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Asking {
    Save,
    Load,
    Exit,
    Info,
}

pub struct Input {
    pub mx: f32,
    pub my: f32,
    pub down: bool,
    pub click: bool,
    pub wheel: f32,
}

pub struct Ctx<'a> {
    pub am: &'a AssetManager,
    pub lang: &'a dyn Fn(usize) -> String,
    pub sound: &'a dyn Fn(&str),
    /// 第 `i` 条（0 起）的缩略图。
    pub thumb: &'a dyn Fn(usize) -> Option<image::RgbaImage>,
    pub play_time: f32,
    pub money: i32,
    /// 在船迷宫（m04、m05）里：不能读档。
    pub boat_maze: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Action {
    /// 存到第 `i` 条（0 起）。
    Save(usize),
    Load(usize),
    Title,
    /// 开始菜单的读档页按了退出。
    Back,
    /// 音效、音乐音量 0..1。
    Volumes(f32, f32),
}

#[derive(Default)]
pub struct Out {
    pub actions: Vec<Action>,
    /// 有框开着：工具栏不收。
    pub locked: bool,
}

pub struct SystemPage {
    layout: Layout,
    briefs: Vec<Option<Brief>>,
    shown: [Shown; NOTES],
    mode: Mode,
    save_enabled: bool,
    first: i32,
    sel: i32,
    curpos: i32,
    slide_val: i32,
    seek_index: usize,
    slide: Drag,
    music: Drag,
    effect: Drag,
    dialog: MuiDialog,
    asking: Option<Asking>,
    last_my: Option<f32>,
    pics: Pics,
    /// 这条的缩略图读过没有（没有图的档别每帧都去读文件）。
    thumb_tried: [bool; NOTES],
}

impl Default for SystemPage {
    fn default() -> Self {
        let mut effect = Drag::new((744, 136, 764, 444), (26, 30), 15, -20, 2, 100.);
        effect.right = true;
        Self {
            layout: Layout::System,
            briefs: vec![None; NOTES],
            shown: [Shown::default(); NOTES],
            mode: Mode::Save,
            save_enabled: true,
            first: 0,
            sel: 0,
            curpos: 1,
            slide_val: 0,
            seek_index: 0,
            slide: Drag::new(
                (640, 66, 654, 503),
                (26, 28),
                14,
                -14,
                20,
                ((NOTES as i32 - 4) * ITEM_H - 28) as f32,
            ),
            music: Drag::new((698, 56, 718, 364), (26, 30), 20, -15, 23, 100.),
            effect,
            dialog: MuiDialog::default(),
            asking: None,
            last_my: None,
            pics: Pics::default(),
            thumb_tried: [false; NOTES],
        }
    }
}

fn clock(t: f32) -> String {
    let s = t.max(0.) as i32;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h >= 100 {
        format!("{h:03}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}")
    }
}

fn thumb_name(i: usize) -> String {
    format!("*system-thumb-{i}")
}

impl SystemPage {
    /// 开始菜单「旧的回忆」用的版式。
    pub fn title() -> Self {
        let mut page = Self {
            layout: Layout::Title,
            ..Self::default()
        };
        page.slide.bk = Layout::Title.slide_bk();
        page
    }

    /// `UIWM_ACTIVE`：读所有存档摘要（`LoadAllNote`），能存就进储存、不能就进读取，`Reset`。
    pub fn activate(
        &mut self,
        briefs: Vec<Option<Brief>>,
        can_save: bool,
        seek: usize,
        volumes: (f32, f32),
    ) {
        self.briefs = briefs;
        self.briefs.resize(NOTES, None);
        for (i, b) in self.briefs.iter().enumerate() {
            self.shown[i] = Shown {
                bright: false,
                dark: b.is_some(),
            };
            self.pics.forget(&thumb_name(i));
        }
        self.thumb_tried = [false; NOTES];
        self.save_enabled = can_save;
        self.mode = if can_save { Mode::Save } else { Mode::Load };
        self.seek_index = seek;
        self.dialog = MuiDialog::default();
        self.asking = None;
        self.reset();
        self.music.seek(volumes.1 * 100.);
        self.effect.seek(volumes.0 * 100.);
    }

    /// 存完：那一条换成新的摘要和缩略图（`SaveNote` 的后半）。
    pub fn saved(
        &mut self,
        i: usize,
        brief: Brief,
        thumb: Option<image::RgbaImage>,
        am: &AssetManager,
    ) {
        if i >= NOTES {
            return;
        }
        self.briefs[i] = Some(brief);
        self.shown[i] = Shown {
            bright: true,
            dark: false,
        };
        match thumb {
            Some(img) => self.pics.insert_image(am, &thumb_name(i), img),
            None => self.pics.forget(&thumb_name(i)),
        }
        self.thumb_tried[i] = true;
        self.seek_index = i;
    }

    fn visible(&self, i: i32) -> bool {
        i >= self.first && i < self.first + SHOW
    }

    fn has_head(&self, i: usize) -> bool {
        self.briefs[i].is_some()
    }

    fn item_rect(&self, i: i32) -> Rect {
        let (x, y) = self.layout.geo().item;
        let top = y + (i - self.first) * ITEM_H;
        (x, top, x + ITEM_W, top + ITEM_H)
    }

    /// `CheckItemShow`。
    fn check_item_show(&mut self) {
        for i in 0..NOTES {
            self.shown[i] = Shown::default();
        }
        for j in self.first..(self.first + SHOW).min(NOTES as i32) {
            let j = j as usize;
            if self.has_head(j) {
                self.shown[j] = Shown {
                    bright: true,
                    dark: j as i32 != self.sel,
                };
            }
        }
    }

    /// `Reset`：回到第一条，再 `SeekNote` 到上次存 / 读的那条。
    fn reset(&mut self) {
        self.first = 0;
        self.slide_val = 0;
        self.curpos = 1;
        self.slide.seek(0.);
        self.sel = 0;
        self.check_item_show();
        self.seek_note();
    }

    /// `SeekNote`：把上次存 / 读的那条选中，滚到可见区的最下面（前 5 条不滚）。
    fn seek_note(&mut self) {
        let mut index = self.seek_index.min(NOTES - 1) as i32;
        let selitem = index;
        let offset = if index > 4 {
            4
        } else {
            index = 0;
            0
        };
        self.first = index - offset;
        self.sel = selitem;
        self.curpos = selitem - offset;
        if self.curpos >= NOTES as i32 - 5 {
            self.curpos = NOTES as i32 - 5;
            self.slide_val = self.slide.max_val as i32;
        } else {
            self.slide_val = self.curpos * ITEM_H;
        }
        self.slide.seek(self.slide_val as f32);
        self.check_item_show();
        let s = self.sel as usize;
        if self.has_head(s) {
            self.shown[s] = Shown {
                bright: true,
                dark: false,
            };
        }
    }

    /// `MoveUp`：列表往上挪一条（看后面的）。
    fn move_up(&mut self, move_slide: bool, ctx: &Ctx) -> bool {
        self.first += 1;
        if move_slide {
            self.curpos = (self.curpos + 1).min(NOTES as i32 - 4);
            self.slide_val = self.curpos * ITEM_H - 28;
            self.slide.seek(self.slide_val as f32);
        }
        if self.first > NOTES as i32 - SHOW {
            self.first = NOTES as i32 - SHOW;
            (ctx.sound)("wa004");
            return false;
        }
        self.check_item_show();
        (ctx.sound)("wa016");
        true
    }

    /// `MoveDw`：列表往下挪一条（看前面的）。
    fn move_down(&mut self, move_slide: bool, ctx: &Ctx) -> bool {
        self.first -= 1;
        if move_slide {
            self.curpos = (self.curpos - 1).max(0);
            self.slide_val = (self.curpos * ITEM_H - 28).max(0);
            self.slide.seek(self.slide_val as f32);
        }
        if self.first < 0 {
            self.first = 0;
            (ctx.sound)("wa004");
            return false;
        }
        self.check_item_show();
        (ctx.sound)("wa016");
        true
    }

    fn ask(&mut self, asking: Asking, kind: Kind, pad: &str, lang: usize, ctx: &Ctx) {
        let text = format!("\n{pad}{}", (ctx.lang)(lang));
        // `m_YesNo.DoModel(false)`：存 / 读的确认框点按钮不出声；其余的框出声。
        let sound = !(kind == Kind::YesNo && matches!(asking, Asking::Save | Asking::Load));
        // 「旧的回忆」的两个框各有位置：`m_YesNo` 在 (250, 200)、`m_MsgBox` 在 (300, 200)。
        let at = match (self.layout, kind) {
            (Layout::Title, Kind::Yes) => (300, 200),
            _ => DLG_AT,
        };
        self.dialog.show(kind, at, text, DLG_INK, sound);
        self.asking = Some(asking);
        (ctx.sound)("wa003");
    }

    /// 双击 / 回车：对第 `sel` 条问一句。
    fn confirm(&mut self, ctx: &Ctx) {
        let exists = self.briefs[self.sel as usize].is_some();
        match self.mode {
            Mode::Save => {
                if exists {
                    self.ask(Asking::Save, Kind::YesNo, "      ", 286, ctx);
                } else {
                    self.ask(Asking::Save, Kind::YesNo, " ", 287, ctx);
                }
            }
            Mode::Load => {
                if ctx.boat_maze {
                    self.ask(Asking::Info, Kind::Yes, "        ", 398, ctx);
                } else if exists {
                    self.ask(Asking::Load, Kind::YesNo, "      ", 283, ctx);
                } else {
                    self.ask(Asking::Info, Kind::Yes, " ", 284, ctx);
                }
            }
        }
    }

    /// 选中第 `i` 条（`CheckSelItem` 的单击那一支）。
    fn select(&mut self, i: i32, ctx: &Ctx) {
        let old = self.sel as usize;
        if self.visible(self.sel) && self.has_head(old) {
            self.shown[old] = Shown {
                bright: false,
                dark: true,
            };
        }
        self.sel = i;
        if self.has_head(i as usize) {
            self.shown[i as usize] = Shown {
                bright: true,
                dark: false,
            };
        }
        (ctx.sound)("wa001");
    }

    fn switch_mode(&mut self, mode: Mode, ctx: &Ctx) {
        if self.mode != mode {
            self.mode = mode;
            self.reset();
        }
        (ctx.sound)("wa016");
    }

    fn refuse_save(&mut self, ctx: &Ctx) {
        self.ask(Asking::Info, Kind::Yes, "        ", 343, ctx);
    }

    /// 键盘（`UpdateKeyCtrl`）。
    fn keys(&mut self, ctx: &Ctx) {
        use imgui::Key;
        if key_pressed(Key::DownArrow) {
            self.sel = (self.sel + 1).min(NOTES as i32 - 1);
            if self.sel >= self.first + SHOW {
                self.move_up(true, ctx);
            }
        } else if key_pressed(Key::UpArrow) {
            self.sel -= 1;
            if self.sel < 0 {
                self.slide.seek(0.);
                self.sel = 0;
            }
            if self.sel < self.first {
                self.move_down(true, ctx);
            }
        } else if self.layout == Layout::System && key_pressed(Key::PageUp) {
            // [推断] 原版只在「战斗场景类型」且不能存时拦；移植端存不了就拦。
            if !self.save_enabled {
                self.refuse_save(ctx);
            } else {
                self.switch_mode(Mode::Save, ctx);
            }
        } else if self.layout == Layout::System && key_pressed(Key::PageDown) {
            self.switch_mode(Mode::Load, ctx);
        } else if key_pressed(Key::Enter) {
            self.confirm(ctx);
        }
    }

    /// 读档页（[`Layout::Title`]）的退出：原版只有右下角那个按钮，[移植] Esc / 手柄 B 也算。
    fn title_back(&mut self, input: &Input, over: &dyn Fn(Rect) -> bool) -> bool {
        self.layout == Layout::Title
            && (key_pressed(imgui::Key::Escape) || (input.click && over(self.layout.geo().exit)))
    }

    pub fn paint(
        &mut self,
        ui: &ComRc<IUiHost>,
        register: &dyn Fn(i64, imgui::TextureId),
        canvas: (f32, f32, f32),
        input: &Input,
        ctx: &Ctx,
    ) -> Out {
        let mut out = Out::default();
        let p = Painter {
            ui,
            register,
            ox: canvas.0,
            oy: canvas.1,
            scale: canvas.2,
        };
        let (mx, my) = p.to_canvas(input.mx, input.my);
        let dy = self.last_my.replace(my).map_or(0., |l| my - l);
        let over = |r: Rect| p.over(input.mx, input.my, r);

        // ---- 输入 ----
        // 有框开着时只有框收输入（框在最后画、在那里收）。
        if !self.dialog.busy() {
            self.keys(ctx);
            if input.wheel > 0. {
                self.move_down(true, ctx);
            } else if input.wheel < 0. {
                self.move_up(true, ctx);
            }
            if self.title_back(input, &over) {
                (ctx.sound)("wa016");
                out.actions.push(Action::Back);
                return out;
            }
            if input.click {
                // 选存档（`CheckSelItem`）：点中已选中的那条 = 双击。
                for i in self.first..(self.first + SHOW).min(NOTES as i32) {
                    if over(self.item_rect(i)) {
                        if i == self.sel {
                            self.confirm(ctx);
                        } else {
                            self.select(i, ctx);
                        }
                        break;
                    }
                }
                let geo = self.layout.geo();
                let system = self.layout == Layout::System;
                if over(geo.pgup) {
                    self.move_down(true, ctx);
                } else if over(geo.pgdw) {
                    self.move_up(true, ctx);
                } else if !system {
                } else if over(SAVE_BTN) {
                    if self.save_enabled {
                        self.switch_mode(Mode::Save, ctx);
                    } else {
                        // 禁用的按下式按钮：自己放 wa004，页面再弹 343。
                        (ctx.sound)("wa004");
                        self.refuse_save(ctx);
                    }
                } else if over(LOAD_BTN) {
                    self.switch_mode(Mode::Load, ctx);
                } else if over(EXIT_BTN) {
                    self.ask(Asking::Exit, Kind::YesNo, "        ", 261, ctx);
                }
            }
            if !self.dialog.busy() {
                if self.slide.update(mx, my, input.down, dy) {
                    let val = self.slide.val as i32;
                    if val > self.slide_val {
                        if val >= self.curpos * ITEM_H {
                            self.move_up(false, ctx);
                            self.slide_val = val;
                            self.curpos = (self.curpos + 1).min(NOTES as i32);
                        }
                    } else if val < self.slide_val && val <= (self.curpos - 1) * ITEM_H {
                        self.move_down(false, ctx);
                        self.slide_val = val;
                        self.curpos = (self.curpos - 1).max(1);
                    }
                }
                let system = self.layout == Layout::System;
                let m = system && self.music.update(mx, my, input.down, dy);
                let e = system && self.effect.update(mx, my, input.down, dy);
                if m || e {
                    out.actions.push(Action::Volumes(
                        self.effect.val / 100.,
                        self.music.val / 100.,
                    ));
                }
            }
        }

        // ---- 画 ----
        self.paint_body(&p, mx, my, input, ctx);

        let dlg_input = muidialog::Input {
            mx: input.mx,
            my: input.my,
            down: input.down,
            click: input.click,
        };
        let was_busy = self.dialog.busy();
        if let Some(answer) = self
            .dialog
            .paint(ui, ctx.am, register, canvas, &dlg_input, ctx.sound)
        {
            match (self.asking.take(), answer) {
                (Some(Asking::Save), Answer::Yes) => {
                    out.actions.push(Action::Save(self.sel as usize));
                    (ctx.sound)("wa029");
                }
                (Some(Asking::Load), Answer::Yes) => {
                    out.actions.push(Action::Load(self.sel as usize));
                    (ctx.sound)("wa030");
                }
                (Some(Asking::Exit), Answer::Yes) => out.actions.push(Action::Title),
                _ => {}
            }
        }
        out.locked |= was_busy || self.dialog.busy();
        out
    }

    fn paint_body(&mut self, p: &Painter, mx: f32, my: f32, input: &Input, ctx: &Ctx) {
        let am = ctx.am;
        let mut names: Vec<String> = [
            "Frame.tga",
            "TopBar.tga",
            "BottomBar.tga",
            "Save0.tga",
            "Save1.tga",
            "Saveon.tga",
            "Load0.tga",
            "Load1.tga",
            "Loadon.tga",
            "Note0.tga",
            "Note1.tga",
            "Note2.tga",
            "cc.tga",
            "dq.tga",
            "hdt.tga",
            "MusicText.tga",
            "MusicEffectText.tga",
            "MusicBar.tga",
            "MusicEffect.tga",
        ]
        .iter()
        .map(|n| format!("{SYS}{n}"))
        .collect();
        for base in ["ExitGame", "pgup", "pgdw"] {
            for i in 0..3 {
                names.push(format!("{SYS}{base}{i}.tga"));
            }
        }
        for h in HEADS {
            names.push(format!("{SYS}{h}.tga"));
            names.push(format!("{SYS}{h}dk.tga"));
        }
        for i in 0..3 {
            names.push(format!(
                "UI\\GameMainUI\\Communal\\scroll\\scrollbar{i}.tga"
            ));
        }
        names.push("UI\\GameMainUI\\Communal\\TimeMoney.tga".into());
        if self.layout == Layout::Title {
            names.push(TITLE_BG.into());
            for i in 0..3 {
                names.push(format!("{TITLE_EXIT}{i}.tga"));
            }
        }
        self.pics.preload(am, &names);
        let sel = self.sel as usize;
        if self.briefs[sel].is_some() && !self.thumb_tried[sel] {
            self.thumb_tried[sel] = true;
            if let Some(img) = (ctx.thumb)(sel) {
                self.pics.insert_image(am, &thumb_name(sel), img);
            }
        }
        let pic = |n: &str| self.pics.peek(&format!("{SYS}{n}"));
        let inside =
            |r: Rect| mx >= r.0 as f32 && mx < r.2 as f32 && my >= r.1 as f32 && my < r.3 as f32;
        let locked = self.dialog.busy();
        // 三态按钮（`UIButton`）：悬停 1、按着 2。
        let tri = |base: &str, r: Rect| {
            let state = if locked || !inside(r) {
                0
            } else if input.down {
                2
            } else {
                1
            };
            p.blit_rect(pic(&format!("{base}{state}.tga")), r);
        };

        let geo = self.layout.geo();
        let system = self.layout == Layout::System;
        if !system {
            p.blit(self.pics.peek(TITLE_BG), 0, 0);
        }
        // 右侧边框、时间金钱牌（本页不画 Frame_Line，原版那行注释掉了）。
        if system {
            p.blit(pic("Frame.tga"), FRAME.0, FRAME.1);
            p.blit(
                self.pics.peek("UI\\GameMainUI\\Communal\\TimeMoney.tga"),
                TIMEMONEY.0,
                TIMEMONEY.1,
            );
            p.text(
                (MONEY_RECT.0 + 8) as f32,
                (MONEY_RECT.1 + 8) as f32,
                INK,
                &ctx.money.to_string(),
            );
            p.text(
                (TIME_RECT.0 + 8) as f32,
                (TIME_RECT.1 + 8) as f32,
                INK,
                &clock(ctx.play_time),
            );
        }

        p.blit(pic("TopBar.tga"), geo.topbar.0, geo.topbar.1);
        p.blit(pic("BottomBar.tga"), geo.bottombar.0, geo.bottombar.1);
        if !system {
            let state = if locked || !inside(geo.exit) {
                0
            } else if input.down {
                2
            } else {
                1
            };
            p.blit_rect(
                self.pics.peek(&format!("{TITLE_EXIT}{state}.tga")),
                geo.exit,
            );
        }
        if system {
            // 储存 / 读取（`UIPushButton`）：按下的是 1，悬停且没按下换 `on` 图，禁用的不亮悬停。
            for (base, r, mode, enabled) in [
                ("Save", SAVE_BTN, Mode::Save, self.save_enabled),
                ("Load", LOAD_BTN, Mode::Load, true),
            ] {
                let pushed = self.mode == mode;
                let name = if pushed {
                    format!("{base}1.tga")
                } else if enabled && !locked && inside(r) {
                    format!("{base}on.tga")
                } else {
                    format!("{base}0.tga")
                };
                p.blit_rect(pic(&name), r);
            }
            tri("ExitGame", EXIT_BTN);
        }

        // 存档条。
        let lang = |n: usize| (ctx.lang)(n);
        for i in self.first..(self.first + SHOW).min(NOTES as i32) {
            let iu = i as usize;
            let r = self.item_rect(i);
            if i == self.sel {
                // 选中的那条禁用：常态图上再盖 `Note2.tga`。
                p.blit_rect(pic("Note0.tga"), r);
                p.blit_rect(pic("Note2.tga"), r);
            } else {
                tri("Note", r);
            }
            let id = (i + 1).to_string();
            let id_rect = (r.0 + 29 + 8, r.1 + 30 + 8, r.0 + 61 - 8, r.1 + 62 - 8);
            let xbase = ((id_rect.2 - id_rect.0) - id.len() as i32 * 8) / 2;
            let ybase = ((id_rect.3 - id_rect.1) - 16) / 2;
            p.text(
                (id_rect.0 + xbase) as f32,
                (id_rect.1 + ybase) as f32,
                ID_INK,
                &id,
            );
            let flag = if self.mode == Mode::Save {
                "cc.tga"
            } else {
                "dq.tga"
            };
            p.blit_rect(pic(flag), (r.0 + 62, r.1 + 12, r.0 + 91, r.1 + 76));
            if let Some(b) = self.briefs[iu].clone() {
                let shown = self.shown[iu];
                let mut x = r.0 + 96;
                for (k, h) in HEADS.iter().enumerate() {
                    if !b.team[k] {
                        continue;
                    }
                    let hr = (x, r.1 + 12, x + 58, r.1 + 76);
                    if shown.bright {
                        p.blit_rect(pic(&format!("{h}.tga")), hr);
                    }
                    if shown.dark {
                        p.blit_rect(pic(&format!("{h}dk.tga")), hr);
                    }
                    x += 58;
                }
                let tx = (r.0 + 312 + 8) as f32;
                p.text(
                    tx,
                    (r.1 + 6 + 8) as f32,
                    INK,
                    &format!("{} {}", lang(49), clock(b.time)),
                );
                p.text(
                    tx,
                    (r.1 + 29 + 8) as f32,
                    INK,
                    &format!("{} {}", lang(48), b.money),
                );
                p.text(
                    tx,
                    (r.1 + 52 + 8) as f32,
                    INK,
                    &format!("{} {}", lang(352), b.scene),
                );
                if i == self.sel {
                    p.stretch(
                        self.pics.peek(&thumb_name(iu)),
                        (r.0 + 490, r.1 + 12, r.0 + 583, r.1 + 81),
                    );
                }
            }
        }

        tri("pgup", geo.pgup);
        tri("pgdw", geo.pgdw);

        let scroll = |state: u8| {
            self.pics.peek(&format!(
                "UI\\GameMainUI\\Communal\\scroll\\scrollbar{state}.tga"
            ))
        };
        p.blit_rect(pic("hdt.tga"), self.slide.bk);
        p.blit_rect(scroll(self.slide.state), self.slide.handle());
        if !system {
            return;
        }
        p.blit(pic("MusicText.tga"), MUSIC_TEXT.0, MUSIC_TEXT.1);
        p.blit(pic("MusicEffectText.tga"), EFFECT_TEXT.0, EFFECT_TEXT.1);
        p.blit_rect(pic("MusicBar.tga"), self.music.bk);
        p.blit_rect(scroll(self.music.state), self.music.handle());
        p.blit_rect(pic("MusicEffect.tga"), self.effect.bk);
        p.blit_rect(scroll(self.effect.state), self.effect.handle());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_bars_land_where_the_original_puts_them() {
        let page = SystemPage::default();
        // 滑块：长度 405，手柄 26×28 在 x=634、顶 68。
        assert_eq!(page.slide.length(), 405.);
        assert_eq!(page.slide.handle(), (634, 68, 660, 96));
        let mut music = page.music;
        music.seek(100.);
        assert_eq!(music.handle(), (695, 63 + 269, 721, 63 + 269 + 30));
        let mut effect = page.effect;
        effect.seek(0.);
        assert_eq!(effect.handle(), (740, 407, 766, 437));
        effect.seek(100.);
        assert_eq!(effect.handle().1, 407 - 269);
    }

    #[test]
    fn seek_note_scrolls_the_last_slot_to_the_bottom_row() {
        let mut page = SystemPage::default();
        page.seek_index = 10;
        page.reset();
        assert_eq!((page.first, page.sel, page.curpos), (6, 10, 6));
        page.seek_index = 3;
        page.reset();
        assert_eq!((page.first, page.sel), (0, 3));
        page.seek_index = 35;
        page.reset();
        assert_eq!((page.first, page.sel, page.curpos), (31, 35, 31));
    }

    #[test]
    fn clock_switches_to_three_digit_hours() {
        assert_eq!(clock(69120.45), "19:12:00");
        assert_eq!(clock(360000.), "100:00:00");
    }
}
