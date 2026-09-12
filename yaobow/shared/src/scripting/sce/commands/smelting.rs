//! 146 冶炼 — `LAB_00436a10`，模式 3。
//!
//! 配方在 `openpal3::states::smelting`（照 `FUN_0047a580`），逆向记录在
//! `switch/re/notes/smelting.md`。命令在冶炼期间阻塞脚本（原版是
//! `FUN_0044f610(3)` 进模式 3 再挂一个等待任务）。
//!
//! 界面照原版画：
//!
//! * 前台是和买卖同一套的竖幡菜单（`FUN_0047be20`）：「冶 炼 / 聊 天 / 退 出」
//!   （AssisLang 181 / 163 / 166，按钮 0x404 / 0x403 / 0x405）。**只有「聊天」跑
//!   参数里的那个过程**（`FUN_0047c430`：`0x403` → `FUN_00432170(DAT_00dba240)`），
//!   实机那一处传的正是「莫子牙聊天」；退出什么都不跑。
//! * 冶炼页（`FUN_00479890`）：列表列出能配上的「尸块 / 数量 / 矿石 / 数量」，
//!   选一行点「是」当场扣材料、莫子牙问「你确定吗？」；点他那段话才开炉，
//!   5 秒后出一件，弹「得到 / 失去」框（`FUN_0047b6c0` / `0x47b3b0`）。「否」把
//!   扣掉的材料还回去。离开钮先说告别语，再点一次回菜单。
//!
//! 手柄：十字键上下选行，A 在有话时点那段话、否则按「是」，B 在问「确定吗」时
//! 按「否」、否则按离开。开炉期间说明面板的黑框里冒火花（[`spark`]）。
//!
//! 出货后黑框里摆产物的 3D 模型（[`ItemPreview`]，冶炼页那台相机是固定的），换一行
//! 就收掉。
//!
//! 音效（[`ui_sound`]）：菜单钮 wa001，「是」放上材料 wa003、放不上 wa004，「否」wa001，
//! 开炉 wa026。
//!
//! 还没做：滚动条。

use super::item_preview::{ItemPreview, PreviewCamera};
use super::ui_sound;
use crate::openpal3::states::shop::ShopRng;
use std::path::PathBuf;
use std::rc::Rc;

use super::appraise::{overlay, virtual_mouse};
use super::dealer::{Tex, bar, blit, button, label, paragraph};
use super::dealer_layout::{
    self as lay, INK, flag_bottom_rect, flag_mid_rect, flag_top_rect, inside, menu_button_rect,
};
use super::encamp::draw_sprite;
use crate::openpal3::assis_lang;
use crate::openpal3::states::smelting::{Recipe, SMELT_SECONDS, recipes};
use crate::openpal3::ui_tex::UiTextures;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{MouseButton, Ui};
use radiance::comdef::ISceneManager;
use radiance::imgui::{GameFontSize, game_font};
use radiance::input::Key;

const LANG_PATH: &str = "/basedata/basedata/AssisLang.lang";

/// AssisLang 里用到的几格。
mod lang {
    pub const CORPSE: usize = 52;
    pub const COUNT: usize = 4;
    pub const ORE: usize = 53;
    pub const GAIN: usize = 157;
    pub const LOSE: usize = 158;
    pub const CHAT: usize = 163;
    pub const LEAVE: usize = 166;
    pub const SMELT: usize = 181;
    /// 莫子牙：进页、问确定、点否、成功、（未用）、告别、没材料。
    pub const INTRO: usize = 356;
    pub const SURE: usize = 357;
    pub const THINK: usize = 358;
    pub const DONE: usize = 359;
    pub const BYE: usize = 361;
    pub const NOTHING: usize = 362;
}

/// 冶炼页的矩形（`FUN_00479890`，800×600）。
mod rect {
    pub type Rect = (i32, i32, i32, i32);
    pub const LINE_L: (i32, i32) = (0, 0);
    pub const LINE_R: (i32, i32) = (786, 0);
    pub const FRAME_BAR: Rect = (0, 565, 800, 600);
    pub const FLAG: (i32, i32) = (220, 6);
    pub const INFO: (i32, i32) = (80, 132);
    pub const WORD: (i32, i32) = (265, 150);
    pub const DESC: Rect = (92, 380, 312, 508);
    pub const STAT: Rect = (92, 510, 312, 542);
    pub const LIST_FRAME: (i32, i32) = (342, 20);
    /// 四个表头：尸块、数量、矿石、数量。
    pub const HEADERS: [Rect; 4] = [
        (360, 36, 468, 68),
        (468, 36, 532, 68),
        (506, 36, 614, 68),
        (614, 36, 678, 68),
    ];
    /// 列表（id 0x1107），行高 29，十行网格；列宽 111 / 36 / 111 / 36。
    pub const LIST: Rect = (362, 68, 698, 368);
    pub const ROW_H: i32 = 29;
    pub const ROWS: usize = 10;
    pub const COLUMNS: [i32; 4] = [111, 36, 111, 36];
    /// 莫子牙说话的文本框（id 0x1116）和它右下角的翻页箭头。
    pub const TALK: Rect = (395, 428, 677, 556);
    pub const ARROW: (i32, i32) = (637, 510);
    /// 「是」「否」（MUIDialog）和离开钮。
    pub const YES: Rect = (206, 570, 298, 596);
    pub const NO: Rect = (410, 570, 502, 596);
    pub const EXIT: Rect = (762, 549, 800, 597);
    /// 出货框：种类 0（只有一个「是」），底图在 (300,200) 起 0x134×0x6d。
    pub const RESULT: Rect = (300, 200, 300 + 0x134, 200 + 0x6d);
}

fn grid_rect(k: usize) -> rect::Rect {
    let y = rect::LIST.1 + 2 + rect::ROW_H * k as i32;
    (rect::LIST.0, y, rect::LIST.0 + 0x14d, y + rect::ROW_H)
}

fn result_yes() -> rect::Rect {
    let r = rect::RESULT;
    let x = r.0 + (r.2 - r.0) / 2 - 89 / 2;
    let y = r.3 - 25 - 6;
    (x, y, x + 89, y + 25)
}

/// 一行配方外加两边的名字。
#[derive(Debug, Clone)]
struct Row {
    recipe: Recipe,
    corpse_name: String,
    ore_name: String,
    product_desc: String,
}

/// 莫子牙那段话处在哪一步（`+0x27c3c`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Talk {
    None,
    Intro,
    /// 材料扣了，问「你确定吗？」——点这段话开炉，点「否」退回。
    Sure,
    Think,
    Done,
    /// 告别；再点一次回菜单。
    Bye,
    Nothing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Menu,
    Page,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Menu(usize),
    Row(usize),
    Yes,
    No,
    Exit,
    Talk,
    ResultYes,
}

/// 冶炼页的预览相机（`FUN_00479890`：`0x47a249` / `0x47a25f`）。
const PREVIEW_CAMERA: PreviewCamera = PreviewCamera {
    size: 210.,
    x: 198,
    y: 264,
    tilt: Some(600.),
};

/// 预览贴到的那一块：说明面板（`rect::INFO`）的黑框，和买卖页一样是面板里
/// (38,17)-(182,198)（右上角压着页签，只贴到 182）。
const PREVIEW_AREA: (i32, i32, i32, i32) = (80 + 38, 132 + 17, 80 + 182, 132 + 198);

/// 正在炼的那一件。
#[derive(Debug, Clone)]
struct Firing {
    row: Row,
    elapsed: f32,
}

/// 开炉的火花。页面上有 60 个 UI 粒子（`+0x2a77c` 起，每个 0x248 字节，粒子类
/// 构造 `FUN_004b4690`）。开炉时 `FUN_0047a400` 全部撒出去；之后每帧
/// `FUN_0047b140` 挨个推：活着的走粒子类的模式 12（`FUN_004b5d20`，推一步再画），
/// 死了的照撒的参数重撒（这一帧不画）。停炉（`+0x2a774 = 0`）就整个不画。
mod spark {
    /// 撒的范围（页面 `+0x3305c..+0x33068`）。
    pub const AREA: (i32, i32, i32, i32) = (168, 168, 268, 328);
    pub const COUNT: usize = 60;
    /// 32×32（`FUN_004b48e0(0x20, 0x20)`），以 (x, y) 为中心画。
    pub const HALF: f32 = 16.;
    /// 每帧横移 `ftol(+0x90 × +0x244)` = `ftol(10 × 0.8)` = 8。
    pub const DX: f32 = 8.;
    /// `+0x244` = 0.8：上升速度的倍率。
    pub const SCALE: f32 = 0.8;
    /// `+0x84` = 1.6：每帧 alpha 减这么多，再截成整数。
    pub const FADE: f32 = 1.6;
    /// 原版按帧推，这里按 60 帧每秒折算 [推断]。
    pub const FRAME: f32 = 1. / 60.;

    #[derive(Debug, Clone, Copy)]
    pub struct Spark {
        pub x: f32,
        pub y: f32,
        /// 横向的三个状态（`+0x60`）：0 起步往右，1 往左，2 往右。
        dir: u8,
        x0: f32,
        x1: f32,
        top: f32,
        bottom: f32,
        /// 升了多高（`+0x94`，起始 2）。
        rise: f32,
        vy: f32,
        pub alpha: u8,
        pub rgb: [u8; 3],
        /// `+0x80`。
        pub alive: bool,
        /// `+0x58`：升到顶了。
        done: bool,
        /// 这一帧画不画。
        pub shown: bool,
    }

    impl Spark {
        /// `FUN_0047a400` 里撒一颗。`rand` 是 MSVC 的 `rand()`（0..32767），调用
        /// 顺序照原版：横坐标、B、G、R、alpha、上升速度、横向速度（模式 12 不用）。
        pub fn spawn(rand: &mut dyn FnMut() -> i32) -> Self {
            let (x0, y0, x1, y1) = AREA;
            let half_w = (x1 - x0) / 2;
            let x = (rand() % half_w + x0) as f32;
            // B = rand%10 + 0xfc 写进一个字节，超过 255 的绕回 0..5——照原样。
            let b = ((rand() % 10 + 0xfc) & 0xff) as u8;
            let g = (rand() % 10 + 0x24) as u8;
            let r = (rand() % 10 + 0xdb) as u8;
            let alpha = (rand() % 0x87 + 0x78) as u8;
            let vy = (rand() % 8 + 2) as f32;
            let _vx = rand() % 8 + 2;
            Self {
                x,
                y: y1 as f32,
                dir: 0,
                // `FUN_004b4b10(w, h)`：左右各 w/2，上边是撒的 y，下边 y + h。
                x0: x - half_w as f32,
                x1: x + half_w as f32,
                top: y0 as f32,
                bottom: y1 as f32,
                rise: 2.,
                vy,
                alpha,
                rgb: [r, g, b],
                alive: true,
                done: false,
                shown: false,
            }
        }

        /// 模式 12 的一帧（`FUN_004b5d20`）。返回这一帧画不画：活着、或者是淡没
        /// 了（不是升到顶）的那一帧还画一次（alpha 已经是 0）。
        pub fn step(&mut self) -> bool {
            match self.dir {
                0 => {
                    self.x += DX;
                    if self.x > self.x1 {
                        self.dir = 1;
                    }
                    if self.x < self.x0 {
                        self.dir = 2;
                    }
                }
                1 => {
                    self.x -= DX;
                    if self.x <= self.x0 {
                        self.dir = 2;
                    }
                }
                _ => {
                    self.x += DX;
                    if self.x >= self.x1 {
                        self.dir = 1;
                    }
                }
            }
            self.y = self.bottom - self.rise;
            if self.y <= self.top + HALF {
                self.done = true;
                self.alive = false;
            }
            self.rise += self.vy * SCALE;
            let a = self.alpha as f32 - FADE;
            if a <= 0. {
                self.alive = false;
                self.done = false;
                self.alpha = 0;
            } else {
                self.alpha = a as u8;
            }
            self.alive || !self.done
        }
    }
}

/// `Par_Effect\spark.tga` 是黑底、没有 alpha 的 RGB 图，原版用另一套渲染状态画
/// （`FUN_005407a0` 传 `DAT_0189505c`，普通界面贴图是 `DAT_0189504c`），看样子是叠加
/// [推断]。imgui 只有 alpha 混合，这里把亮度当 alpha、颜色填白，再乘顶点色，近似叠加。
fn spark_texture(am: &crate::openpal3::asset_manager::AssetManager) -> Tex {
    let mut image = am.read_picture("/basedata/basedata/ui/GameMainUI/Par_Effect/spark.tga")?;
    for p in image.pixels_mut() {
        let a = p[0].max(p[1]).max(p[2]);
        *p = image::Rgba([255, 255, 255, a]);
    }
    Some(am.sprite_from_image(image))
}

#[derive(Clone, Default)]
struct Skin {
    flag_t: Tex,
    flag_m: Tex,
    flag_b: Tex,
    rawbtn: [Tex; 3],
    line: Tex,
    frame_bar: Tex,
    flag: Tex,
    info: Tex,
    word: Tex,
    list_frame: Tex,
    grid: Tex,
    selected: Tex,
    yes: [Tex; 3],
    no: [Tex; 3],
    exit: [Tex; 3],
    arrow: [Tex; 2],
    dlg_bg: Tex,
    spark: Tex,
}

impl Skin {
    fn load(state: &SceState) -> Self {
        let am = state.asset_mgr().clone();
        let spark = spark_texture(&am);
        let mut t = UiTextures::new(&am);
        let mut three = |t: &mut UiTextures, base: &str| -> [Tex; 3] {
            std::array::from_fn(|k| t.get(&am, &format!("{base}{k}.tga")))
        };
        let rawbtn = three(&mut t, "SubGame\\HockShop\\Control\\rawbtn");
        let yes = three(&mut t, "UI\\GameMainUI\\MUIDialog\\Normal\\Yes");
        let no = three(&mut t, "UI\\GameMainUI\\MUIDialog\\Normal\\No");
        let exit = three(&mut t, "UI\\GameMainUI\\Communal\\StpCtl\\Exit");
        let mut one = |n: &str| t.get(&am, n);
        Self {
            flag_t: one("UI\\Assistant\\Deal\\FlagT.tga"),
            flag_m: one("UI\\Assistant\\Deal\\FlagM.tga"),
            flag_b: one("UI\\Assistant\\Deal\\FlagB.tga"),
            rawbtn,
            line: one("UI\\GameMainUI\\Communal\\smt_Line.tga"),
            frame_bar: one("UI\\Flex\\framebar.tga"),
            flag: one("UI\\Assistant\\Smelting\\smt_Flag.tga"),
            info: one("UI\\GameMainUI\\Communal\\Info.tga"),
            word: one("UI\\GameMainUI\\Communal\\Smelt_word.tga"),
            list_frame: one("UI\\GameMainUI\\Magic\\ListFrame.tga"),
            grid: one("UI\\Assistant\\Smelting\\List_Grid.tga"),
            selected: one("UI\\assistant\\Communal\\ListBoxSelPic.tga"),
            yes,
            no,
            exit,
            arrow: [one("ui\\scene\\page0.tga"), one("ui\\scene\\page1.tga")],
            dlg_bg: one("UI\\GameMainUI\\MUIDialog\\Normal\\DlgBackground.tga"),
            spark,
        }
    }
}

/// 菜单三个钮（`FUN_0047be20` 的摆法）。
const MENU: [(usize, &str); 3] = [
    (lang::SMELT, "冶 炼"),
    (lang::CHAT, "聊 天"),
    (lang::LEAVE, "退 出"),
];

#[derive(Clone)]
pub struct SceCommandSmeltingMenu {
    /// 点「聊天」时跑的过程（原版 `DAT_00dba240`）。
    end_proc: i32,
    rows: Vec<Row>,
    cursor: Option<usize>,
    top: usize,
    screen: Screen,
    menu_cursor: usize,
    talk: Talk,
    firing: Option<Firing>,
    /// 出货框里的字，`Some` 时框开着。
    result: Option<String>,
    lang: Vec<String>,
    skin: Skin,
    anim: f32,
    sparks: Vec<spark::Spark>,
    /// 出货后黑框里的产物模型（原版 `+0x2a650`，有模型标记 `+0x2a654`）。
    preview: Option<Rc<ItemPreview>>,
    /// 攒到下一帧开头放的音效（`update` 开头、`finish` 里放）。
    sfx: Option<&'static str>,
    /// 攒不满一帧的时间。
    spark_clock: f32,
    rng: ShopRng,
    last_mouse: (f32, f32),
    chat: bool,
    loaded: bool,
}

impl std::fmt::Debug for SceCommandSmeltingMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandSmeltingMenu")
            .field("end_proc", &self.end_proc)
            .field("screen", &self.screen)
            .field("talk", &self.talk)
            .finish()
    }
}

impl SceCommandSmeltingMenu {
    pub fn new(end_proc: i32) -> Self {
        Self {
            end_proc,
            rows: Vec::new(),
            cursor: None,
            top: 0,
            screen: Screen::Menu,
            menu_cursor: 0,
            talk: Talk::None,
            firing: None,
            result: None,
            lang: vec![],
            skin: Skin::default(),
            anim: 0.,
            sparks: vec![],
            preview: None,
            sfx: None,
            spark_clock: 0.,
            rng: ShopRng::new(0x2a77c),
            last_mouse: (-1., -1.),
            chat: false,
            loaded: false,
        }
    }

    fn text(&self, slot: usize, fallback: &str) -> String {
        self.lang
            .get(slot)
            .filter(|s| !s.is_empty())
            .map(|s| assis_lang::unescape(s))
            .unwrap_or_else(|| fallback.to_string())
    }

    fn rebuild(&mut self, state: &SceState) {
        let Some(db) = state.global_state().game_db().cloned() else {
            self.rows.clear();
            return;
        };
        let p_state = state.global_state().persistent_state();
        let name = |id: u32| {
            db.item(id)
                .and_then(|i| i.name.as_str().ok())
                .unwrap_or_else(|| format!("道具#{id}"))
        };
        let desc = |id: u32| {
            db.item(id)
                .and_then(|i| i.description.as_str().ok())
                .unwrap_or_default()
        };
        self.rows = recipes(&db, p_state.inventory())
            .into_iter()
            .map(|recipe| Row {
                corpse_name: name(recipe.corpse),
                ore_name: name(recipe.ore),
                product_desc: desc(recipe.product),
                recipe,
            })
            .collect();
        if self.cursor.is_some_and(|c| c >= self.rows.len()) {
            self.cursor = None;
        }
    }

    /// 进冶炼页（`FUN_0047c2d0(1)` 给页面发 1000）：没材料说 362，否则说 356。
    fn enter_page(&mut self, state: &SceState) {
        self.rebuild(state);
        self.screen = Screen::Page;
        self.cursor = None;
        self.preview = None;
        self.top = 0;
        self.talk = if self.rows.is_empty() {
            Talk::Nothing
        } else {
            Talk::Intro
        };
    }

    fn talk_text(&self) -> Option<String> {
        let slot = match self.talk {
            Talk::None => return None,
            Talk::Intro => lang::INTRO,
            Talk::Sure => lang::SURE,
            Talk::Think => lang::THINK,
            Talk::Done => lang::DONE,
            Talk::Bye => lang::BYE,
            Talk::Nothing => lang::NOTHING,
        };
        Some(self.text(slot, ""))
    }

    /// 「是」（0x1114）/ 双击行：当场扣材料，问「你确定吗？」（`FUN_0047aab0`）。
    fn take(&mut self, state: &mut SceState) {
        // `0x47b8e0` 起：放上了 wa003，放不上 wa004 [推断：几种放不上的情况都归 wa004]。
        self.sfx = Some(ui_sound::DENIED);
        if self.firing.is_some() || self.talk == Talk::Sure {
            return;
        }
        let Some(row) = self.cursor.and_then(|c| self.rows.get(c)).cloned() else {
            return;
        };
        {
            let mut p = state.global_state_mut().persistent_state_mut();
            if p.item_count(row.recipe.corpse) == 0 || p.item_count(row.recipe.ore) == 0 {
                return;
            }
            p.remove_item(row.recipe.corpse, 1);
            p.remove_item(row.recipe.ore, 1);
        }
        // 原版在这一行上直接减数（`FUN_0047aab0`），不重排列表。
        self.bump(-1);
        self.talk = Talk::Sure;
        self.sfx = Some(ui_sound::NOTE);
    }

    /// 「否」（0x1115）：把刚扣的还回去（`FUN_0047adf0`），说 358。
    fn give_back(&mut self, state: &mut SceState) {
        if self.firing.is_some() {
            return;
        }
        // `0x47b767`：还回去之后放 wa001。
        self.sfx = Some(ui_sound::CLICK);
        if self.talk == Talk::Sure {
            if let Some(row) = self.cursor.and_then(|c| self.rows.get(c)).cloned() {
                let mut p = state.global_state_mut().persistent_state_mut();
                p.add_item(row.recipe.corpse, 1);
                p.add_item(row.recipe.ore, 1);
            }
            self.bump(1);
        }
        self.talk = Talk::Think;
    }

    /// 选中那一行两边的数量一起加减（同一块尸块 / 矿石出现在别的行里也跟着变）。
    fn bump(&mut self, delta: i32) {
        let Some(row) = self.cursor.and_then(|c| self.rows.get(c)).cloned() else {
            return;
        };
        let adj = |n: &mut u32| *n = (*n as i32 + delta).max(0) as u32;
        for r in &mut self.rows {
            if r.recipe.corpse == row.recipe.corpse {
                adj(&mut r.recipe.corpse_count);
            }
            if r.recipe.ore == row.recipe.ore {
                adj(&mut r.recipe.ore_count);
            }
        }
    }

    /// 点那段话（0x3f1 / 0x1116）。
    fn tap_talk(&mut self) {
        match self.talk {
            Talk::Intro | Talk::Think | Talk::Done | Talk::Nothing => self.talk = Talk::None,
            Talk::Sure => {
                // 开炉：`FUN_0047a400` 撒火花、`+0x2a774 = 1`。
                if let Some(row) = self.cursor.and_then(|c| self.rows.get(c)).cloned() {
                    self.firing = Some(Firing { row, elapsed: 0. });
                    self.sfx = Some(ui_sound::FORGE);
                    let rng = &mut self.rng;
                    self.sparks = (0..spark::COUNT)
                        .map(|_| spark::Spark::spawn(&mut || rng.rand15()))
                        .collect();
                    self.spark_clock = 0.;
                }
                self.talk = Talk::None;
            }
            Talk::Bye => self.screen = Screen::Menu,
            Talk::None => {}
        }
    }

    /// 离开钮（0x1102）：先告别，再点一次回菜单。
    fn leave(&mut self) {
        if self.firing.is_some() {
            return;
        }
        if self.talk == Talk::Bye {
            self.screen = Screen::Menu;
        } else {
            self.talk = Talk::Bye;
        }
    }

    /// 火花按帧推（见 [`spark`]）；停炉就清掉。
    fn step_sparks(&mut self, dt: f32) {
        if self.firing.is_none() {
            self.sparks.clear();
            return;
        }
        self.spark_clock += dt;
        while self.spark_clock >= spark::FRAME {
            self.spark_clock -= spark::FRAME;
            let rng = &mut self.rng;
            for s in &mut self.sparks {
                if s.alive {
                    s.shown = s.step();
                } else {
                    *s = spark::Spark::spawn(&mut || rng.rand15());
                }
            }
        }
    }

    /// 满 5 秒出一件（`0x47b3b0`）。
    fn step_firing(&mut self, state: &mut SceState, dt: f32) {
        let Some(f) = self.firing.as_mut() else {
            return;
        };
        f.elapsed += dt;
        if f.elapsed <= SMELT_SECONDS {
            return;
        }
        let done = self.firing.take().expect("刚才还在");
        state
            .global_state_mut()
            .persistent_state_mut()
            .add_item(done.row.recipe.product, 1);
        let product = state
            .global_state()
            .game_db()
            .and_then(|db| db.item(done.row.recipe.product))
            .and_then(|i| i.name.as_str().ok())
            .unwrap_or_default();
        // `"\n        %s: %s\n        %s: %s %s"`，得到 / 失去。
        self.result = Some(format!(
            "\n        {}: {}\n        {}: {} {}",
            self.text(lang::GAIN, "得到"),
            product,
            self.text(lang::LOSE, "失去"),
            done.row.corpse_name,
            done.row.ore_name
        ));
        self.talk = Talk::Done;
        self.cursor = None;
        self.rebuild(state);
        // 出货那一支（`0x47b44a` 起）载产物模型：相机是页面初始化时定死的
        // `FUN_0043a070(210)` + `FUN_00439930(198, 264, 600)`。
        self.preview =
            ItemPreview::build(state, done.row.recipe.product, PREVIEW_CAMERA, PREVIEW_AREA);
    }

    fn select(&mut self, row: usize) {
        // 列表选行那一支（`0x47baaf`）先把有模型标记清掉 [推断：按分支位置]。
        self.preview = None;
        if self.rows.is_empty() || self.firing.is_some() || self.talk == Talk::Sure {
            return;
        }
        let row = row.min(self.rows.len() - 1);
        self.cursor = Some(row);
        if row < self.top {
            self.top = row;
        } else if row >= self.top + rect::ROWS {
            self.top = row + 1 - rect::ROWS;
        }
        // 选行时把莫子牙的话清掉（0x3f1 / 0x1107）。
        self.talk = Talk::None;
    }

    fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        match self.screen {
            Screen::Menu => (0..MENU.len())
                .find(|&k| inside(menu_button_rect(k), x, y))
                .map(Hit::Menu),
            Screen::Page => {
                if self.result.is_some() {
                    return inside(result_yes(), x, y).then_some(Hit::ResultYes);
                }
                for (r, h) in [
                    (rect::EXIT, Hit::Exit),
                    (rect::YES, Hit::Yes),
                    (rect::NO, Hit::No),
                ] {
                    if inside(r, x, y) {
                        return Some(h);
                    }
                }
                if self.talk != Talk::None && inside(rect::TALK, x, y) {
                    return Some(Hit::Talk);
                }
                (0..rect::ROWS)
                    .find(|&k| inside(grid_rect(k), x, y) && self.top + k < self.rows.len())
                    .map(Hit::Row)
            }
        }
    }

    /// 返回 true = 关掉冶炼。
    fn act(&mut self, state: &mut SceState, hit: Hit) -> bool {
        // 菜单三个钮（`FUN_0047c430`）都放 wa001。
        if let Hit::Menu(_) = hit {
            self.sfx = Some(ui_sound::CLICK);
        }
        match hit {
            Hit::Menu(0) => self.enter_page(state),
            Hit::Menu(1) => {
                if self.end_proc > 0 {
                    self.chat = true;
                    return true;
                }
            }
            Hit::Menu(_) => return true,
            Hit::Row(k) => self.select(self.top + k),
            Hit::Yes => self.take(state),
            Hit::No => self.give_back(state),
            Hit::Exit => self.leave(),
            Hit::Talk => self.tap_talk(),
            Hit::ResultYes => self.result = None,
        }
        false
    }

    /// 返回 true = 关掉冶炼。
    fn handle_keys(&mut self, state: &mut SceState) -> bool {
        let (up, down, confirm, cancel) = {
            let input = state.input();
            let p = |k: Key| input.get_key_state(k).pressed();
            (
                p(Key::Up) || p(Key::GamePadDPadUp),
                p(Key::Down) || p(Key::GamePadDPadDown),
                p(Key::Space) || p(Key::GamePadEast),
                p(Key::Escape) || p(Key::GamePadSouth),
            )
        };
        match self.screen {
            Screen::Menu => {
                let n = MENU.len();
                if up {
                    self.menu_cursor = (self.menu_cursor + n - 1) % n;
                } else if down {
                    self.menu_cursor = (self.menu_cursor + 1) % n;
                } else if confirm {
                    return self.act(state, Hit::Menu(self.menu_cursor));
                } else if cancel {
                    return true;
                }
            }
            Screen::Page => {
                if self.result.is_some() {
                    if confirm || cancel {
                        self.result = None;
                    }
                    return false;
                }
                let n = self.rows.len();
                if up && n > 0 {
                    self.select(self.cursor.map_or(0, |c| (c + n - 1) % n));
                } else if down && n > 0 {
                    self.select(self.cursor.map_or(0, |c| (c + 1) % n));
                } else if confirm {
                    if self.talk != Talk::None {
                        self.tap_talk();
                    } else {
                        self.take(state);
                    }
                } else if cancel {
                    if self.talk == Talk::Sure {
                        self.give_back(state);
                    } else {
                        self.leave();
                    }
                }
            }
        }
        false
    }

    fn draw(&self, ui: &Ui, hover: Option<Hit>) {
        overlay(ui, "Smelting", |ui, at, scale| {
            let list = ui.get_window_draw_list();
            let _font = game_font(GameFontSize::SMALL).map(|f| ui.push_font(f));
            let sk = &self.skin;
            match self.screen {
                Screen::Menu => {
                    let n = MENU.len();
                    let t = flag_top_rect();
                    blit(&list, &sk.flag_t, t.0, t.1, at);
                    let b = flag_bottom_rect(n);
                    blit(&list, &sk.flag_b, b.0, b.1, at);
                    for k in 0..n {
                        let m = flag_mid_rect(k);
                        blit(&list, &sk.flag_m, m.0, m.1, at);
                    }
                    for (k, (slot, fallback)) in MENU.iter().enumerate() {
                        let r = menu_button_rect(k);
                        let lit = k == self.menu_cursor || hover == Some(Hit::Menu(k));
                        button(&list, &sk.rawbtn, r, lit, at);
                        let s = self.text(*slot, fallback);
                        let w = super::dealer::text_width(&s);
                        list.add_text(
                            at(
                                r.0 + (r.2 - r.0 - w) / 2,
                                r.1 + (r.3 - r.1 - lay::GLYPH) / 2,
                            ),
                            INK,
                            &s,
                        );
                    }
                }
                Screen::Page => self.draw_page(&list, at, scale, hover),
            }
        });
    }

    fn draw_page(
        &self,
        list: &imgui::DrawListMut<'_>,
        at: &dyn Fn(i32, i32) -> [f32; 2],
        scale: f32,
        hover: Option<Hit>,
    ) {
        let sk = &self.skin;
        blit(list, &sk.line, rect::LINE_L.0, rect::LINE_L.1, at);
        blit(list, &sk.line, rect::LINE_R.0, rect::LINE_R.1, at);
        bar(list, &sk.frame_bar, rect::FRAME_BAR, 30, at);
        blit(list, &sk.flag, rect::FLAG.0, rect::FLAG.1, at);
        blit(list, &sk.info, rect::INFO.0, rect::INFO.1, at);
        if let Some(p) = &self.preview {
            p.draw(list, at);
        }
        blit(list, &sk.word, rect::WORD.0, rect::WORD.1, at);
        blit(
            list,
            &sk.list_frame,
            rect::LIST_FRAME.0,
            rect::LIST_FRAME.1,
            at,
        );
        let headers = [
            self.text(lang::CORPSE, "尸块"),
            self.text(lang::COUNT, "数量"),
            self.text(lang::ORE, "矿石"),
            self.text(lang::COUNT, "数量"),
        ];
        for (r, h) in rect::HEADERS.iter().zip(&headers) {
            label(list, *r, h, at);
        }

        let row_dy = (rect::ROW_H - lay::GLYPH) as f32 * 0.5 * scale;
        for k in 0..rect::ROWS {
            let g = grid_rect(k);
            blit(list, &sk.grid, g.0, g.1, at);
            let i = self.top + k;
            let Some(row) = self.rows.get(i) else {
                continue;
            };
            let chosen = self.cursor == Some(i);
            if chosen {
                blit(list, &sk.selected, g.0, g.1, at);
            }
            let ink = if chosen { [1.0, 0.98, 0.98, 1.0] } else { INK };
            let cells = [
                row.corpse_name.clone(),
                row.recipe.corpse_count.to_string(),
                row.ore_name.clone(),
                row.recipe.ore_count.to_string(),
            ];
            let mut x = rect::LIST.0;
            for (c, text) in cells.iter().enumerate() {
                let [px, py] = at(x + lay::TEXT_INSET, g.1);
                list.add_text([px, py + row_dy], ink, text);
                x += rect::COLUMNS[c];
            }
        }

        // 说明面板：选中行的产物（`FUN_0047b140`）。
        if let Some(row) = self.cursor.and_then(|c| self.rows.get(c)) {
            let (desc, stat) = row
                .product_desc
                .split_once('$')
                .unwrap_or((row.product_desc.as_str(), ""));
            paragraph(list, rect::DESC, desc, at);
            let stat = stat.trim_end_matches(['\n', '\r', ' ', '\u{3000}']);
            if !stat.is_empty() {
                label(list, rect::STAT, stat, at);
            }
        }

        if let Some(text) = self.talk_text() {
            paragraph(list, rect::TALK, &text, at);
            let frame = (self.anim * 2.) as usize % 2;
            blit(list, &sk.arrow[frame], rect::ARROW.0, rect::ARROW.1, at);
        }
        if let Some(tex) = sk.spark.as_deref() {
            for s in self.sparks.iter().filter(|s| s.shown) {
                let (x, y, h) = (s.x as i32, s.y as i32, spark::HALF as i32);
                let [r, g, b] = s.rgb.map(|c| c as f32 / 255.);
                list.add_image(tex.imgui_texture_id(), at(x - h, y - h), at(x + h, y + h))
                    .col([r, g, b, s.alpha as f32 / 255.])
                    .build();
            }
        }

        button(list, &sk.yes, rect::YES, hover == Some(Hit::Yes), at);
        button(list, &sk.no, rect::NO, hover == Some(Hit::No), at);
        button(list, &sk.exit, rect::EXIT, hover == Some(Hit::Exit), at);

        if let Some(text) = &self.result {
            let r = rect::RESULT;
            draw_sprite(list, sk.dlg_bg.as_deref(), r, at);
            let ink = [1.0, 0xfb as f32 / 255., 0x1f as f32 / 255., 1.0];
            for (k, line) in text.split('\n').enumerate() {
                list.add_text(
                    at(
                        r.0 + 6 + 8,
                        r.1 + 6 + 8 + lay::GLYPH * k as i32 - lay::GLYPH,
                    ),
                    ink,
                    line,
                );
            }
            button(
                list,
                &sk.yes,
                result_yes(),
                hover == Some(Hit::ResultYes),
                at,
            );
        }
    }
}

impl SceCommand for SceCommandSmeltingMenu {
    fn initialize(&mut self, _scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        self.skin = Skin::load(state);
    }

    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if !self.loaded {
            self.loaded = true;
            self.lang = read_gbk(state.asset_mgr().vfs(), LANG_PATH)
                .map(|t| assis_lang::parse_section(&t, "sec1"))
                .unwrap_or_default();
            self.rebuild(state);
        }
        if state.fast_forward() {
            return true;
        }

        self.anim += delta_sec;
        if let Some(name) = self.sfx.take() {
            ui_sound::play(state, name);
        }
        self.step_firing(state, delta_sec);
        self.step_sparks(delta_sec);
        if let Some(p) = &self.preview {
            p.render(delta_sec);
        }

        let (mx, my) = virtual_mouse(ui);
        let hover = self.hit(mx, my);
        if (mx, my) != self.last_mouse {
            self.last_mouse = (mx, my);
            if let Some(Hit::Menu(k)) = hover {
                self.menu_cursor = k;
            }
        }
        self.draw(ui, hover);

        let busy = self.firing.is_some();
        if ui.is_mouse_clicked(MouseButton::Left) && !busy {
            if let Some(h) = hover {
                if self.act(state, h) {
                    return self.finish(state);
                }
            }
        }
        if !busy && self.handle_keys(state) {
            return self.finish(state);
        }
        false
    }
}

impl SceCommandSmeltingMenu {
    /// 关掉冶炼：点的是「聊天」就跑参数里的过程（`FUN_0047c430` 的 0x403）。
    fn finish(&mut self, state: &mut SceState) -> bool {
        if let Some(name) = self.sfx.take() {
            ui_sound::play(state, name);
        }
        if self.chat && self.end_proc > 0 {
            state.call_proc(self.end_proc as u32);
        }
        true
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
