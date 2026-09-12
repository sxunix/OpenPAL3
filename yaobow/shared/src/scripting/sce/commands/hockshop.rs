//! 103 当铺 — `LAB_00436c60`，模式 7。
//!
//! 账目在 `openpal3::states::hockshop`（等级表 `HockShopLevel.hsl`，算式照
//! `FUN_00524e00` / `FUN_0051f130`，死当物照同目录 `source.txt`），逆向记录在
//! `switch/re/notes/hockshop.md`。命令在当铺期间阻塞脚本（原版是模式 7 + 等待任务）。
//!
//! 界面照原版（`FUN_005232b0` 建的模式对象，控件位置用 `switch/re/layout_trace.py`
//! 从布局函数里算出来）：
//!
//! * 进店（`0x523b90`）：脚本变量 `HockShop_Gut`（`init.scs` 第 49 个，即 −32719）
//!   大于 0 就按 `FUN_0051f130` 分一笔红利、清零、弹「您巳得到当铺的分红: N」；
//!   资本为 0 进增资页，否则进经营主页。
//! * 经营主页（`FUN_0051dfd0`）：竖幡上「典当 / 收购 / 经营 / 增资 / 聊天 / 退出」，
//!   按等级开放；「经营」展开四个设定（当期 / 利率 / 拆当率 / 经营重点），每个再展开
//!   它的选项。**只有「聊天」跑参数里的过程**（0xa01b05），「退出」什么都不跑。
//! * 增资页（`FUN_00524230`）：`Invest_Frame` 上五个数、拖动条、确定 / 取消。
//! * 典当 / 收购：原版挂的就是买卖界面的卖页和买道具页，这里借 `dealer` 那两页。
//!
//! 手柄：十字键上下挑、右键进下一列、左键退回，A 确定，B 返回；增资页左右调数目。

use std::path::PathBuf;

use super::appraise::{overlay, virtual_mouse};
use super::dealer::{HostedPage, SceCommandDealerMenu, Tex, blit, button, text_width};
use super::dealer_layout::{self as lay, INK, inside};
use super::encamp::draw_sprite;
use super::ui_sound;
use crate::openpal3::assis_lang;
use crate::openpal3::states::hockshop::{
    HockShopLevels, PawnRng, Proportions, max_investment, pick_dead_pawn_items,
};
use crate::openpal3::states::shop::ShopRng;
use crate::openpal3::ui_tex::UiTextures;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{MouseButton, Ui};
use radiance::comdef::ISceneManager;
use radiance::imgui::{GameFontSize, game_font};
use radiance::input::Key;

const HSL_PATH: &str = "/basedata/basedata/SubGame/HockShop/Data/HockShopLevel.hsl";
const LANG_PATH: &str = "/basedata/basedata/AssisLang.lang";
/// `HockShop_Gut`：`init.scs` 的命名变量表里排第 49 个（`g_story` 是第 0 个 = −32768）
/// [推断：编号按表里的次序排]。
const GUT_VAR: i16 = -32768 + 49;

type Rect = (i32, i32, i32, i32);

mod lang {
    pub const DIVIDEND: usize = 75;
    /// 等级名：E 戊 · D 丁 · C 丙 · B 乙 · A 甲。
    pub const LEVEL_E: usize = 387;
    pub const HEADERS: [usize; 4] = [167, 168, 169, 170];
    pub const OPTIONS: [&[usize]; 4] = [
        &[171, 172, 173],
        &[174, 175, 176],
        &[174, 175, 176],
        &[177, 178, 179, 180],
    ];
}

/// 经营设定四列各对应存档里的哪个选择（0 方针 · 1 当期 · 2 利率 · 3 拆当率）。
const HEADER_CHOICE: [usize; 4] = [1, 2, 3, 0];

/// 主页菜单（`FUN_0051dfd0`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Item {
    Pawn,
    Buy,
    DealIn,
    Invest,
    Chat,
    Leave,
}

const MENU: [(Item, usize, &str); 6] = [
    (Item::Pawn, 159, "典 当"),
    (Item::Buy, 160, "收 购"),
    (Item::DealIn, 161, "经 营"),
    (Item::Invest, 162, "增 资"),
    (Item::Chat, 163, "聊 天"),
    (Item::Leave, 166, "退 出"),
];

mod rect {
    use super::Rect;
    pub const FLAG_T: (i32, i32) = (182, 78);
    pub const FLAG_B: (i32, i32) = (182, 266);
    pub fn menu(k: usize) -> Rect {
        let y = 110 + 26 * k as i32;
        (172, y, 262, y + 26)
    }
    pub fn flag_mid(k: usize) -> (i32, i32) {
        (173, 111 + 26 * k as i32)
    }
    pub fn header(h: usize) -> Rect {
        let y = 162 + 28 * h as i32;
        (264, y, 354, y + 26)
    }
    pub fn option(h: usize, k: usize) -> Rect {
        let y = 162 + 28 * (h + k) as i32;
        (356, y, 446, y + 26)
    }
    // 增资页（`FUN_00524230`）。
    pub const FRAME: (i32, i32) = (112, 84);
    pub const LEVEL: Rect = (400, 84, 528, 116);
    pub const CAP: Rect = (284, 126, 412, 158);
    pub const TOTAL: Rect = (558, 126, 686, 158);
    pub const MONEY: Rect = (284, 166, 412, 198);
    pub const AMOUNT: Rect = (558, 166, 686, 198);
    pub const BAR: Rect = (186, 216, 589, 254);
    pub const YES: Rect = (238, 287, 328, 313);
    pub const NO: Rect = (428, 287, 518, 313);
    /// 进店分红那个框（`FUN_005232b0`：(250,200) 种类 0）。
    pub const NOTE: Rect = (250, 200, 250 + 0x134, 200 + 0x6d);
    pub fn note_yes() -> Rect {
        let x = NOTE.0 + (NOTE.2 - NOTE.0) / 2 - 89 / 2;
        let y = NOTE.3 - 25 - 6;
        (x, y, x + 89, y + 25)
    }
}

/// 增资页数字的颜色：`0xffeab1` / `0xb1eaff` 两种写法都落到 RGB(0xb1,0xea,0xff)。
const INVEST_INK: [f32; 4] = [0xb1 as f32 / 255., 0xea as f32 / 255., 1., 1.];
const NOTE_INK: [f32; 4] = [1.0, 0xfb as f32 / 255., 0x1f as f32 / 255., 1.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Business,
    Invest,
    Hosted,
}

/// 手柄的焦点在哪一列。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Menu,
    Header,
    Option,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    Menu(usize),
    Header(usize),
    Option(usize),
    Bar,
    Yes,
    No,
    NoteYes,
}

#[derive(Clone, Default)]
struct Skin {
    flag_t: Tex,
    flag_m: Tex,
    flag_b: Tex,
    rawbtn: [Tex; 3],
    rawbtn2: Tex,
    unrawbtn: Tex,
    frame: Tex,
    bar: Tex,
    thumb: [Tex; 3],
    yes: [Tex; 3],
    no: [Tex; 3],
    note_bg: Tex,
}

impl Skin {
    fn load(state: &SceState) -> Self {
        let am = state.asset_mgr().clone();
        let mut t = UiTextures::new(&am);
        let three = |t: &mut UiTextures, base: &str| -> [Tex; 3] {
            std::array::from_fn(|k| t.get(&am, &format!("{base}{k}.tga")))
        };
        let rawbtn = three(&mut t, "SubGame\\HockShop\\Control\\rawbtn");
        let thumb = three(&mut t, "UI\\GameMainUI\\Communal\\Scroll\\scrollbar");
        let yes = three(&mut t, "UI\\GameMainUI\\MUIDialog\\Normal\\Yes");
        let no = three(&mut t, "UI\\GameMainUI\\MUIDialog\\Normal\\No");
        let mut one = |n: &str| t.get(&am, n);
        Self {
            flag_t: one("UI\\Assistant\\Deal\\FlagT.tga"),
            flag_m: one("UI\\Assistant\\Deal\\FlagM.tga"),
            flag_b: one("UI\\Assistant\\Deal\\FlagB.tga"),
            rawbtn2: rawbtn[2].clone(),
            rawbtn,
            unrawbtn: one("SubGame\\HockShop\\Control\\unrawbtn.tga"),
            frame: one("SubGame\\HockShop\\Invest\\Invest_Frame.tga"),
            bar: one("SubGame\\HockShop\\Invest\\DragBar_bk.tga"),
            thumb,
            yes,
            no,
            note_bg: one("UI\\GameMainUI\\MUIDialog\\Normal\\DlgBackground.tga"),
        }
    }
}

/// 把 `ShopRng` 接到死当物生成上：概率掷法和还价、逃跑同一套。
struct PawnDice<'a>(&'a mut ShopRng);

impl PawnRng for PawnDice<'_> {
    fn chance(&mut self, p: f32) -> bool {
        crate::openpal3::states::shop::roll(self.0, p)
    }
    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.0.rand15() as usize) % n
    }
}

#[derive(Clone)]
pub struct SceCommandMiniGamePawnshop {
    /// 点「聊天」时跑的过程（原版 `DAT_00dba244`）。
    end_proc: i32,
    table: HockShopLevels,
    lang: Vec<String>,
    skin: Skin,
    screen: Screen,
    hosted: Option<Box<SceCommandDealerMenu>>,
    menu_cursor: usize,
    focus: Focus,
    dealin_open: bool,
    header_cursor: usize,
    header_open: Option<usize>,
    option_cursor: usize,
    /// 增资这一单打算投多少（`+0x148c`）。
    amount: i32,
    dragging: bool,
    /// 这一帧要放的音效。
    sfx: Option<&'static str>,
    /// 分红框里的字。
    note: Option<String>,
    /// 死当物（id, 单价）：进店时按等级和经营重点摸的。
    dead_pawn: Vec<(u32, i32)>,
    rng: ShopRng,
    last_mouse: (f32, f32),
    chat: bool,
    loaded: bool,
}

impl std::fmt::Debug for SceCommandMiniGamePawnshop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandMiniGamePawnshop")
            .field("end_proc", &self.end_proc)
            .field("screen", &self.screen)
            .finish()
    }
}

impl SceCommandMiniGamePawnshop {
    pub fn new(end_proc: i32) -> Self {
        Self {
            end_proc,
            table: HockShopLevels::default(),
            lang: vec![],
            skin: Skin::default(),
            screen: Screen::Business,
            hosted: None,
            menu_cursor: 0,
            focus: Focus::Menu,
            dealin_open: false,
            header_cursor: 0,
            header_open: None,
            option_cursor: 0,
            amount: 0,
            dragging: false,
            sfx: None,
            note: None,
            dead_pawn: vec![],
            rng: ShopRng::new(0x1357_9bdf),
            last_mouse: (-1., -1.),
            chat: false,
            loaded: false,
        }
    }

    fn text(&self, slot: usize, fallback: &str) -> String {
        self.lang
            .get(slot)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
    }

    fn load(&mut self, state: &mut SceState) {
        self.loaded = true;
        let vfs = state.asset_mgr().vfs();
        match read_gbk(vfs, &PathBuf::from(HSL_PATH)) {
            Some(text) => self.table = HockShopLevels::parse(&text),
            None => log::error!("当铺：读不到 {HSL_PATH}"),
        }
        self.lang = read_gbk(vfs, &PathBuf::from(LANG_PATH))
            .map(|t| assis_lang::parse_section(&t, "sec1"))
            .unwrap_or_default();
        let (money, capital) = {
            let p = state.global_state().persistent_state();
            (p.money(), p.hockshop_capital())
        };
        self.rng = ShopRng::new(money as u32 ^ (capital as u32).rotate_left(7) ^ 0x9e37_79b9);
        self.rebuild_dead_pawn(state);

        // 进店分红（`0x523b90`）：`HockShop_Gut > 0` 才分，分到钱才清零。
        let gut = state
            .global_state()
            .persistent_state()
            .get_global(GUT_VAR)
            .unwrap_or(0);
        if gut > 0 {
            let table = self.table.clone();
            let paid = state
                .global_state_mut()
                .persistent_state_mut()
                .hockshop_dividend(&table);
            if paid > 0 {
                state
                    .global_state_mut()
                    .persistent_state_mut()
                    .set_global(GUT_VAR, 0);
                // 进店弹红利框（`FUN_00523820` `0x523c3f`）放 wa003。
                self.sfx = Some(ui_sound::NOTE);
                self.note = Some(format!(
                    "{}: {}",
                    self.text(lang::DIVIDEND, "您巳得到当铺的分红"),
                    paid
                ));
            }
        }
        self.screen = if capital > 0 {
            Screen::Business
        } else {
            Screen::Invest
        };
    }

    fn level(&self, state: &SceState) -> crate::openpal3::states::hockshop::Level {
        *self
            .table
            .level(state.global_state().persistent_state().hockshop_level())
    }

    /// 进店摆出来的死当物：件数在 `DisMinNum..=DisMaxNum` 之间，按当前经营重点
    /// 的比例挑（`source.txt`），单价打这一级的 `BuyDiscount`。
    fn rebuild_dead_pawn(&mut self, state: &SceState) {
        self.dead_pawn.clear();
        let Some(db) = state.global_state().game_db().cloned() else {
            return;
        };
        let mode = state
            .global_state()
            .persistent_state()
            .hockshop_choice(0)
            .max(0) as usize;
        let level = self.level(state);
        if !level.buy || level.display_max <= 0 {
            return;
        }
        let span = (level.display_max - level.display_min).max(0) + 1;
        let count = level.display_min + (self.rng.rand15() % span);
        let proportions = level
            .proportions
            .get(mode)
            .copied()
            .unwrap_or(Proportions::default());
        let table: Vec<(u32, u8)> = db.items().iter().map(|i| (i.id, i.item_type)).collect();
        let ids = {
            let mut dice = PawnDice(&mut self.rng);
            pick_dead_pawn_items(count.max(0) as usize, proportions, &table, &mut dice)
        };
        self.dead_pawn = ids
            .into_iter()
            .filter_map(|id| {
                let row = db.item(id)?;
                Some((id, (row.price as f32 * level.buy_discount) as i32))
            })
            .collect();
    }

    fn enabled(&self, state: &SceState, item: Item) -> bool {
        let lv = self.level(state);
        match item {
            Item::Pawn => lv.sale,
            Item::Buy => lv.buy,
            Item::DealIn => lv.deal_in,
            Item::Invest => lv.invest,
            Item::Chat | Item::Leave => true,
        }
    }

    fn max_amount(state: &SceState) -> i32 {
        let p = state.global_state().persistent_state();
        max_investment(p.hockshop_capital(), p.money())
    }

    /// 返回 true = 关店。
    fn activate(&mut self, state: &mut SceState, item: Item) -> bool {
        // `FUN_0051f1b0`：置灰的钮点了放 wa004，其余 wa001。
        if !self.enabled(state, item) {
            self.sfx = Some(ui_sound::DENIED);
            return false;
        }
        self.sfx = Some(ui_sound::CLICK);
        match item {
            Item::Pawn | Item::Buy => {
                let page = if item == Item::Pawn {
                    HostedPage::Sell
                } else {
                    HostedPage::Buy
                };
                self.hosted = Some(Box::new(SceCommandDealerMenu::hosted(
                    state,
                    page,
                    &self.dead_pawn,
                )));
                self.screen = Screen::Hosted;
            }
            Item::DealIn => {
                self.dealin_open = !self.dealin_open;
                self.header_open = None;
                self.focus = if self.dealin_open {
                    Focus::Header
                } else {
                    Focus::Menu
                };
            }
            Item::Invest => {
                self.amount = 0;
                self.screen = Screen::Invest;
            }
            Item::Chat => {
                self.chat = true;
                return true;
            }
            Item::Leave => return true,
        }
        false
    }

    fn open_header(&mut self, state: &SceState, h: usize) {
        self.sfx = Some(ui_sound::CLICK);
        if self.header_open == Some(h) {
            self.header_open = None;
            self.focus = Focus::Header;
        } else {
            self.header_open = Some(h);
            self.option_cursor = state
                .global_state()
                .persistent_state()
                .hockshop_choice(HEADER_CHOICE[h])
                .max(0) as usize;
            self.focus = Focus::Option;
        }
        self.header_cursor = h;
    }

    fn choose(&mut self, state: &mut SceState, k: usize) {
        self.sfx = Some(ui_sound::CLICK);
        let Some(h) = self.header_open else { return };
        let which = HEADER_CHOICE[h];
        state
            .global_state_mut()
            .persistent_state_mut()
            .set_hockshop_choice(which, k as i32);
        if which == 0 {
            self.rebuild_dead_pawn(state);
        }
        self.header_open = None;
        self.focus = Focus::Header;
    }

    /// 增资确定（`FUN_005249a0` 的 0xa00b07）。
    fn invest(&mut self, state: &mut SceState) {
        self.sfx = Some(ui_sound::CLICK);
        if self.amount > 0 {
            state
                .global_state_mut()
                .persistent_state_mut()
                .hockshop_invest(self.amount);
            self.rebuild_dead_pawn(state);
        }
        self.amount = 0;
        if state.global_state().persistent_state().hockshop_capital() > 0 {
            self.screen = Screen::Business;
        }
    }

    /// 返回 true = 关店（资本还是 0 时点取消就直接出店）。
    fn cancel_invest(&mut self, state: &SceState) -> bool {
        self.sfx = Some(ui_sound::CLICK);
        self.amount = 0;
        if state.global_state().persistent_state().hockshop_capital() > 0 {
            self.screen = Screen::Business;
            false
        } else {
            true
        }
    }

    /// 拖动条动了一格放 wa005（`FUN_005249a0` `0x524ab1`）。
    fn set_amount(&mut self, v: i32) {
        if v != self.amount {
            self.amount = v;
            self.sfx = Some(ui_sound::TICK);
        }
    }

    fn bar_value(&self, state: &SceState, mx: f32) -> i32 {
        let r = rect::BAR;
        let (x0, x1) = ((r.0 + 16 + 13) as f32, (r.2 - 16 - 13) as f32);
        let t = ((mx - x0) / (x1 - x0)).clamp(0., 1.);
        (t * Self::max_amount(state) as f32).round() as i32
    }

    fn thumb_rect(&self, state: &SceState) -> Rect {
        let r = rect::BAR;
        let max = Self::max_amount(state).max(1);
        let t = self.amount as f32 / max as f32;
        let x = r.0 + 16 + ((r.2 - r.0 - 32 - 26) as f32 * t) as i32;
        let y = r.1 + (r.3 - r.1 - 28) / 2;
        (x, y, x + 26, y + 28)
    }

    fn hit(&self, state: &SceState, x: f32, y: f32) -> Option<Hit> {
        if self.note.is_some() {
            return inside(rect::note_yes(), x, y).then_some(Hit::NoteYes);
        }
        match self.screen {
            Screen::Hosted => None,
            Screen::Invest => [
                (rect::YES, Hit::Yes),
                (rect::NO, Hit::No),
                (rect::BAR, Hit::Bar),
            ]
            .into_iter()
            .find(|(r, _)| inside(*r, x, y))
            .map(|(_, h)| h),
            Screen::Business => {
                if let Some(h) = self.header_open {
                    let n = lang::OPTIONS[h].len();
                    if let Some(k) = (0..n).find(|&k| inside(rect::option(h, k), x, y)) {
                        return Some(Hit::Option(k));
                    }
                }
                if self.dealin_open {
                    if let Some(h) = (0..4).find(|&h| inside(rect::header(h), x, y)) {
                        return Some(Hit::Header(h));
                    }
                }
                let _ = state;
                (0..MENU.len())
                    .find(|&k| inside(rect::menu(k), x, y))
                    .map(Hit::Menu)
            }
        }
    }

    /// 返回 true = 关店。
    fn click(&mut self, state: &mut SceState, hit: Hit, mx: f32) -> bool {
        match hit {
            Hit::NoteYes => {
                self.sfx = Some(ui_sound::CLICK);
                self.note = None;
            }
            Hit::Menu(k) => {
                self.menu_cursor = k;
                self.focus = Focus::Menu;
                return self.activate(state, MENU[k].0);
            }
            Hit::Header(h) => self.open_header(state, h),
            Hit::Option(k) => self.choose(state, k),
            Hit::Bar => {
                self.dragging = true;
                let v = self.bar_value(state, mx);
                self.set_amount(v);
            }
            Hit::Yes => self.invest(state),
            Hit::No => return self.cancel_invest(state),
        }
        false
    }

    /// 返回 true = 关店。
    fn keys(&mut self, state: &mut SceState) -> bool {
        let (up, down, left, right, confirm, cancel) = {
            let input = state.input();
            let p = |k: Key| input.get_key_state(k).pressed();
            (
                p(Key::Up) || p(Key::GamePadDPadUp),
                p(Key::Down) || p(Key::GamePadDPadDown),
                p(Key::Left) || p(Key::GamePadDPadLeft),
                p(Key::Right) || p(Key::GamePadDPadRight),
                p(Key::Space) || p(Key::GamePadEast),
                p(Key::Escape) || p(Key::GamePadSouth),
            )
        };
        if self.note.is_some() {
            if confirm || cancel {
                self.note = None;
            }
            return false;
        }
        match self.screen {
            Screen::Hosted => {}
            Screen::Invest => {
                let max = Self::max_amount(state);
                let step = (max / 20).max(1);
                if left || down {
                    self.set_amount((self.amount - step).clamp(0, max));
                } else if right || up {
                    self.set_amount((self.amount + step).clamp(0, max));
                } else if confirm {
                    self.invest(state);
                } else if cancel {
                    return self.cancel_invest(state);
                }
            }
            Screen::Business => match self.focus {
                Focus::Menu => {
                    let n = MENU.len();
                    if up {
                        self.menu_cursor = (self.menu_cursor + n - 1) % n;
                    } else if down {
                        self.menu_cursor = (self.menu_cursor + 1) % n;
                    } else if right && self.dealin_open {
                        self.focus = Focus::Header;
                    } else if confirm {
                        return self.activate(state, MENU[self.menu_cursor].0);
                    } else if cancel {
                        if self.dealin_open {
                            self.dealin_open = false;
                        } else {
                            return true;
                        }
                    }
                }
                Focus::Header => {
                    if up {
                        self.header_cursor = (self.header_cursor + 3) % 4;
                    } else if down {
                        self.header_cursor = (self.header_cursor + 1) % 4;
                    } else if confirm || right {
                        self.open_header(state, self.header_cursor);
                    } else if left || cancel {
                        self.header_open = None;
                        self.focus = Focus::Menu;
                    }
                }
                Focus::Option => {
                    let n = self.header_open.map_or(1, |h| lang::OPTIONS[h].len());
                    if up {
                        self.option_cursor = (self.option_cursor + n - 1) % n;
                    } else if down {
                        self.option_cursor = (self.option_cursor + 1) % n;
                    } else if confirm {
                        self.choose(state, self.option_cursor);
                    } else if left || cancel {
                        self.header_open = None;
                        self.focus = Focus::Header;
                    }
                }
            },
        }
        false
    }

    fn draw(&self, ui: &Ui, state: &SceState, hover: Option<Hit>) {
        overlay(ui, "HockShop", |ui, at, _scale| {
            let list = ui.get_window_draw_list();
            let _font = game_font(GameFontSize::SMALL).map(|f| ui.push_font(f));
            match self.screen {
                Screen::Business => self.draw_business(&list, at, state, hover),
                Screen::Invest => self.draw_invest(&list, at, state, hover),
                Screen::Hosted => {}
            }
            if let Some(text) = &self.note {
                let r = rect::NOTE;
                draw_sprite(&list, self.skin.note_bg.as_deref(), r, at);
                list.add_text(at(r.0 + 6 + 8, r.1 + 6 + 8), NOTE_INK, text);
                button(
                    &list,
                    &self.skin.yes,
                    rect::note_yes(),
                    hover == Some(Hit::NoteYes),
                    at,
                );
            }
        });
    }

    fn centered(
        list: &imgui::DrawListMut<'_>,
        r: Rect,
        s: &str,
        at: &dyn Fn(i32, i32) -> [f32; 2],
    ) {
        let w = text_width(s);
        list.add_text(
            at(
                r.0 + (r.2 - r.0 - w) / 2,
                r.1 + (r.3 - r.1 - lay::GLYPH) / 2,
            ),
            INK,
            s,
        );
    }

    fn rawbutton(
        &self,
        list: &imgui::DrawListMut<'_>,
        r: Rect,
        lit: bool,
        on: bool,
        enabled: bool,
        at: &dyn Fn(i32, i32) -> [f32; 2],
    ) {
        let sk = &self.skin;
        if !enabled {
            blit(list, &sk.unrawbtn, r.0, r.1, at);
        } else if on {
            blit(list, &sk.rawbtn2, r.0, r.1, at);
        } else {
            button(list, &sk.rawbtn, r, lit, at);
        }
    }

    fn draw_business(
        &self,
        list: &imgui::DrawListMut<'_>,
        at: &dyn Fn(i32, i32) -> [f32; 2],
        state: &SceState,
        hover: Option<Hit>,
    ) {
        let sk = &self.skin;
        blit(list, &sk.flag_t, rect::FLAG_T.0, rect::FLAG_T.1, at);
        blit(list, &sk.flag_b, rect::FLAG_B.0, rect::FLAG_B.1, at);
        for k in 0..MENU.len() {
            let (x, y) = rect::flag_mid(k);
            blit(list, &sk.flag_m, x, y, at);
        }
        for (k, (item, slot, fallback)) in MENU.iter().enumerate() {
            let r = rect::menu(k);
            let lit =
                hover == Some(Hit::Menu(k)) || (self.focus == Focus::Menu && k == self.menu_cursor);
            let on = *item == Item::DealIn && self.dealin_open;
            self.rawbutton(list, r, lit, on, self.enabled(state, *item), at);
            Self::centered(list, r, &self.text(*slot, fallback), at);
        }
        if !self.dealin_open {
            return;
        }
        let p = state.global_state().persistent_state();
        for h in 0..4 {
            let r = rect::header(h);
            let lit = hover == Some(Hit::Header(h))
                || (self.focus == Focus::Header && h == self.header_cursor);
            self.rawbutton(list, r, lit, self.header_open == Some(h), true, at);
            Self::centered(list, r, &self.text(lang::HEADERS[h], ""), at);
        }
        if let Some(h) = self.header_open {
            let current = p.hockshop_choice(HEADER_CHOICE[h]).max(0) as usize;
            for (k, slot) in lang::OPTIONS[h].iter().enumerate() {
                let r = rect::option(h, k);
                let lit = hover == Some(Hit::Option(k))
                    || (self.focus == Focus::Option && k == self.option_cursor);
                self.rawbutton(list, r, lit, k == current, true, at);
                Self::centered(list, r, &self.text(*slot, ""), at);
            }
        }
    }

    fn draw_invest(
        &self,
        list: &imgui::DrawListMut<'_>,
        at: &dyn Fn(i32, i32) -> [f32; 2],
        state: &SceState,
        hover: Option<Hit>,
    ) {
        let sk = &self.skin;
        if let Some(frame) = sk.frame.as_deref() {
            let r = (
                rect::FRAME.0,
                rect::FRAME.1,
                rect::FRAME.0 + 543,
                rect::FRAME.1 + 234,
            );
            draw_sprite(list, Some(frame), r, at);
        }
        let (money, capital, level) = {
            let p = state.global_state().persistent_state();
            (p.money(), p.hockshop_capital(), p.hockshop_level())
        };
        let label = |r: Rect, s: String| {
            list.add_text(
                at(r.0 + lay::TEXT_INSET, r.1 + lay::TEXT_INSET),
                INVEST_INK,
                s,
            );
        };
        label(
            rect::LEVEL,
            self.text(lang::LEVEL_E - level.clamp(0, 4) as usize, "戊"),
        );
        label(
            rect::CAP,
            crate::openpal3::states::hockshop::CAPITAL_FOR_TOP.to_string(),
        );
        label(rect::TOTAL, (capital + self.amount).to_string());
        label(rect::MONEY, (money - self.amount).to_string());
        label(rect::AMOUNT, self.amount.to_string());
        draw_sprite(list, sk.bar.as_deref(), rect::BAR, at);
        let t = self.thumb_rect(state);
        let lit = usize::from(self.dragging || hover == Some(Hit::Bar));
        blit(list, &sk.thumb[lit], t.0, t.1, at);
        button(list, &sk.yes, rect::YES, hover == Some(Hit::Yes), at);
        button(list, &sk.no, rect::NO, hover == Some(Hit::No), at);
    }

    fn finish(&mut self, state: &mut SceState) -> bool {
        if self.chat && self.end_proc > 0 {
            state.call_proc(self.end_proc as u32);
        }
        true
    }
}

impl SceCommand for SceCommandMiniGamePawnshop {
    fn initialize(&mut self, _scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        self.skin = Skin::load(state);
    }

    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        if !self.loaded {
            self.load(state);
        }
        if state.fast_forward() {
            return true;
        }
        if self.screen == Screen::Hosted {
            let done = self
                .hosted
                .as_mut()
                .is_none_or(|h| h.hosted_frame(ui, state));
            if done {
                self.hosted = None;
                self.screen = Screen::Business;
            }
            return false;
        }

        let (mx, my) = virtual_mouse(ui);
        let hover = self.hit(state, mx, my);
        if self.dragging {
            if ui.is_mouse_down(MouseButton::Left) {
                let v = self.bar_value(state, mx);
                self.set_amount(v);
            } else {
                self.dragging = false;
            }
        }
        if (mx, my) != self.last_mouse {
            self.last_mouse = (mx, my);
            if let Some(Hit::Menu(k)) = hover {
                self.menu_cursor = k;
            }
        }
        self.draw(ui, state, hover);
        let mut done = false;
        if ui.is_mouse_clicked(MouseButton::Left) {
            if let Some(h) = hover {
                done = self.click(state, h, mx);
            }
        }
        if !done {
            done = self.keys(state);
        }
        if let Some(name) = self.sfx.take() {
            ui_sound::play(state, name);
        }
        if done {
            return self.finish(state);
        }
        false
    }
}

fn read_gbk(vfs: &mini_fs::MiniFs, path: &std::path::Path) -> Option<String> {
    use common::store_ext::StoreExt2;
    use encoding::Encoding;
    let bytes = vfs.read_to_end(path).ok()?;
    encoding::all::GBK
        .decode(&bytes, encoding::DecoderTrap::Ignore)
        .ok()
}
