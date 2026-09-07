//! 103 当铺经营 — `LAB_00436c60`，模式 7。
//!
//! 账目在 `openpal3::states::hockshop`（等级表来自
//! `SubGame/HockShop/Data/HockShopLevel.hsl`，算式照 `FUN_00524e00` /
//! `FUN_0051f130`，死当物的挑法照同目录 `source.txt` 那段原版源码），见
//! `switch/re/notes/hockshop.md`。
//!
//! 命令在当铺期间阻塞脚本（原版是模式 7 + 等待任务），关掉时跑参数里的那个
//! 过程——参数是过程号，原版存进 `DAT_00dba244`，离开当铺时
//! `FUN_00432170(DAT_00dba244, 0)`。
//!
//! 界面是文字浮层，不是原版那五页；卖东西沿用买卖那边的规则（`price / 2`）。

use std::path::PathBuf;

use crate::openpal3::states::hockshop::{
    HockShopLevels, Level, MODES, PawnRng, Proportions, TIERS, max_investment, pick_dead_pawn_items,
};
use crate::openpal3::states::shop::{ShopRng, sell_price, sellable};
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{Condition, Ui};
use radiance::comdef::ISceneManager;
use radiance::input::Key;

const HSL_PATH: &str = "/basedata/basedata/SubGame/HockShop/Data/HockShopLevel.hsl";
/// 增资拖动条一格多少钱 `[推断: 原版是拖动条，没有格子]`。
const INVEST_STEP: i32 = 1_000;

const LEVEL_NAMES: [&str; 5] = ["E", "D", "C", "B", "A"];
const MODE_NAMES: [&str; MODES] = ["灵丹妙药", "南北杂货", "奇珍异宝", "来者不拒"];
const TIER_NAMES: [&str; TIERS] = ["低", "中", "高"];
const CHOICE_NAMES: [&str; 4] = ["经营方针", "当期", "利率", "拆当率"];

/// 一行可卖 / 可买的东西。
#[derive(Debug, Clone)]
struct Row {
    id: u32,
    name: String,
    price: i32,
    owned: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Menu,
    /// 增资。
    Invest,
    /// 经营：挑四个选择中的第 `which` 个。
    DealIn {
        which: usize,
    },
    /// 买死当物。
    Buy,
    /// 卖东西。
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Input {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Cancel,
}

/// 菜单项，按等级开放。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    Invest,
    Dividend,
    DealIn,
    Buy,
    Sell,
    Leave,
}

impl Entry {
    fn label(self) -> &'static str {
        match self {
            Entry::Invest => "增资",
            Entry::Dividend => "分红",
            Entry::DealIn => "经营",
            Entry::Buy => "买死当物",
            Entry::Sell => "卖东西",
            Entry::Leave => "离开",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SceCommandMiniGamePawnshop {
    /// 关掉当铺之后要跑的过程（原版 `DAT_00dba244`）。
    end_proc: i32,
    table: HockShopLevels,
    page: Page,
    cursor: usize,
    /// 增资这一单打算投多少。
    amount: i32,
    /// 死当物（进店时按等级和经营方针随机摸的）。
    dead_pawn: Vec<Row>,
    /// 背包里能卖的东西。
    stock: Vec<Row>,
    notice: String,
    /// 这一趟分过红没有——原版隔多久能再分还没读出来 `[推断]`。
    paid_dividend: bool,
    rng: ShopRng,
    loaded: bool,
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

impl SceCommandMiniGamePawnshop {
    pub fn new(end_proc: i32) -> Self {
        Self {
            end_proc,
            table: HockShopLevels::default(),
            page: Page::Menu,
            cursor: 0,
            amount: 0,
            dead_pawn: Vec::new(),
            stock: Vec::new(),
            notice: String::new(),
            paid_dividend: false,
            rng: ShopRng::new(0x1357_9bdf),
            loaded: false,
        }
    }

    fn load(&mut self, state: &mut SceState) {
        self.loaded = true;
        if let Some(text) = read_gbk(state.asset_mgr().vfs(), &PathBuf::from(HSL_PATH)) {
            self.table = HockShopLevels::parse(&text);
        } else {
            log::error!("当铺：读不到 {HSL_PATH}");
        }
        let (money, capital) = {
            let p = state.global_state().persistent_state();
            (p.money(), p.hockshop_capital())
        };
        self.rng = ShopRng::new(money as u32 ^ (capital as u32).rotate_left(7) ^ 0x9e37_79b9);
        self.amount = INVEST_STEP.min(max_investment(capital, money));
        self.rebuild_dead_pawn(state);
        self.rebuild_stock(state);
    }

    fn level(&self, state: &SceState) -> &Level {
        self.table
            .level(state.global_state().persistent_state().hockshop_level())
    }

    /// 进店摆出来的死当物：件数在 `DisMinNum..=DisMaxNum` 之间，按当前经营
    /// 方针的比例挑（`source.txt`）。
    fn rebuild_dead_pawn(&mut self, state: &SceState) {
        self.dead_pawn.clear();
        let Some(db) = state.global_state().game_db().cloned() else {
            return;
        };
        let (level_idx, mode) = {
            let p = state.global_state().persistent_state();
            (p.hockshop_level(), p.hockshop_choice(0).max(0) as usize)
        };
        let level = *self.table.level(level_idx);
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

        let p_state = state.global_state().persistent_state();
        self.dead_pawn = ids
            .into_iter()
            .filter_map(|id| {
                let row = db.item(id)?;
                Some(Row {
                    id,
                    name: row
                        .name
                        .as_str()
                        .map(|n| n.to_string())
                        .unwrap_or_else(|_| format!("道具#{id}")),
                    // 买死当物按这一级的折扣（`BuyDiscount`）。
                    price: (row.price as f32 * level.buy_discount) as i32,
                    owned: p_state.item_count(id),
                })
            })
            .collect();
    }

    fn rebuild_stock(&mut self, state: &SceState) {
        self.stock.clear();
        let Some(db) = state.global_state().game_db().cloned() else {
            return;
        };
        let p_state = state.global_state().persistent_state();
        self.stock = p_state
            .inventory()
            .iter()
            .filter(|(_, count)| **count > 0)
            .filter_map(|(id, count)| {
                let row = db.item(*id)?;
                if !sellable(row.item_type) {
                    return None;
                }
                Some(Row {
                    id: *id,
                    name: row
                        .name
                        .as_str()
                        .map(|n| n.to_string())
                        .unwrap_or_else(|_| format!("道具#{id}")),
                    price: sell_price(row.price),
                    owned: *count,
                })
            })
            .collect();
    }

    /// 这一级开放哪些菜单项。
    fn entries(&self, state: &SceState) -> Vec<Entry> {
        let lv = self.level(state);
        let mut out = Vec::new();
        if lv.invest {
            out.push(Entry::Invest);
        }
        if lv.melon_cutting {
            out.push(Entry::Dividend);
        }
        if lv.deal_in {
            out.push(Entry::DealIn);
        }
        if lv.buy {
            out.push(Entry::Buy);
        }
        if lv.sale {
            out.push(Entry::Sell);
        }
        out.push(Entry::Leave);
        out
    }

    fn poll_input(state: &SceState) -> Option<Input> {
        let input = state.input();
        let pressed = |k: Key| input.get_key_state(k).pressed();
        if pressed(Key::Up) || pressed(Key::GamePadDPadUp) {
            Some(Input::Up)
        } else if pressed(Key::Down) || pressed(Key::GamePadDPadDown) {
            Some(Input::Down)
        } else if pressed(Key::Left) || pressed(Key::GamePadDPadLeft) {
            Some(Input::Left)
        } else if pressed(Key::Right) || pressed(Key::GamePadDPadRight) {
            Some(Input::Right)
        } else if pressed(Key::Space) || pressed(Key::GamePadEast) {
            Some(Input::Confirm)
        } else if pressed(Key::Escape) || pressed(Key::GamePadSouth) {
            Some(Input::Cancel)
        } else {
            None
        }
    }

    /// 返回 true = 关店。
    fn handle(&mut self, state: &mut SceState, input: Input) -> bool {
        match self.page {
            Page::Menu => {
                let entries = self.entries(state);
                let n = entries.len().max(1);
                match input {
                    Input::Up => self.cursor = (self.cursor + n - 1) % n,
                    Input::Down => self.cursor = (self.cursor + 1) % n,
                    Input::Confirm => match entries.get(self.cursor).copied() {
                        Some(Entry::Invest) => {
                            let (capital, money) = {
                                let p = state.global_state().persistent_state();
                                (p.hockshop_capital(), p.money())
                            };
                            self.amount = INVEST_STEP.min(max_investment(capital, money));
                            self.page = Page::Invest;
                        }
                        Some(Entry::Dividend) => self.pay_dividend(state),
                        Some(Entry::DealIn) => {
                            self.page = Page::DealIn { which: 0 };
                        }
                        Some(Entry::Buy) => {
                            self.cursor = 0;
                            self.page = Page::Buy;
                        }
                        Some(Entry::Sell) => {
                            self.cursor = 0;
                            self.page = Page::Sell;
                        }
                        _ => return true,
                    },
                    Input::Cancel => return true,
                    _ => {}
                }
            }
            Page::Invest => {
                let (capital, money) = {
                    let p = state.global_state().persistent_state();
                    (p.hockshop_capital(), p.money())
                };
                let cap = max_investment(capital, money);
                match input {
                    Input::Left | Input::Down => {
                        self.amount = (self.amount - INVEST_STEP).clamp(0, cap)
                    }
                    Input::Right | Input::Up => {
                        self.amount = (self.amount + INVEST_STEP).clamp(0, cap)
                    }
                    Input::Confirm => {
                        let spent = state
                            .global_state_mut()
                            .persistent_state_mut()
                            .hockshop_invest(self.amount);
                        let level = state.global_state().persistent_state().hockshop_level();
                        self.notice = format!(
                            "投了 {spent} 文，当铺现在是 {} 级",
                            LEVEL_NAMES[level as usize]
                        );
                        self.rebuild_dead_pawn(state);
                        self.page = Page::Menu;
                    }
                    Input::Cancel => self.page = Page::Menu,
                }
            }
            Page::DealIn { which } => match input {
                Input::Up => {
                    self.page = Page::DealIn {
                        which: (which + 3) % 4,
                    }
                }
                Input::Down => {
                    self.page = Page::DealIn {
                        which: (which + 1) % 4,
                    }
                }
                Input::Left | Input::Right => {
                    let span = if which == 0 { MODES } else { TIERS } as i32;
                    let mut p_state = state.global_state_mut().persistent_state_mut();
                    let cur = p_state.hockshop_choice(which);
                    let next = if input == Input::Left {
                        (cur + span - 1) % span
                    } else {
                        (cur + 1) % span
                    };
                    p_state.set_hockshop_choice(which, next);
                    drop(p_state);
                    if which == 0 {
                        self.rebuild_dead_pawn(state);
                    }
                }
                Input::Confirm | Input::Cancel => self.page = Page::Menu,
            },
            Page::Buy => match input {
                Input::Up => {
                    let n = self.dead_pawn.len().max(1);
                    self.cursor = (self.cursor + n - 1) % n;
                }
                Input::Down => {
                    let n = self.dead_pawn.len().max(1);
                    self.cursor = (self.cursor + 1) % n;
                }
                Input::Confirm => self.buy(state),
                Input::Cancel => {
                    self.cursor = 0;
                    self.page = Page::Menu;
                }
                _ => {}
            },
            Page::Sell => match input {
                Input::Up => {
                    let n = self.stock.len().max(1);
                    self.cursor = (self.cursor + n - 1) % n;
                }
                Input::Down => {
                    let n = self.stock.len().max(1);
                    self.cursor = (self.cursor + 1) % n;
                }
                Input::Confirm => self.sell(state),
                Input::Cancel => {
                    self.cursor = 0;
                    self.page = Page::Menu;
                }
                _ => {}
            },
        }
        false
    }

    fn pay_dividend(&mut self, state: &mut SceState) {
        if self.paid_dividend {
            self.notice = "这一趟已经分过红了".to_string();
            return;
        }
        let table = self.table.clone();
        let paid = state
            .global_state_mut()
            .persistent_state_mut()
            .hockshop_dividend(&table);
        self.paid_dividend = true;
        self.notice = format!("分了 {paid} 文");
    }

    fn buy(&mut self, state: &mut SceState) {
        let Some(row) = self.dead_pawn.get(self.cursor).cloned() else {
            return;
        };
        {
            let mut p_state = state.global_state_mut().persistent_state_mut();
            if p_state.money() < row.price {
                drop(p_state);
                self.notice = "钱不够".to_string();
                return;
            }
            p_state.add_money(-row.price);
            p_state.add_item(row.id, 1);
        }
        self.dead_pawn.remove(self.cursor);
        if self.cursor >= self.dead_pawn.len() {
            self.cursor = self.dead_pawn.len().saturating_sub(1);
        }
        self.notice = format!("买下了 {}，付 {} 文", row.name, row.price);
        self.rebuild_stock(state);
    }

    fn sell(&mut self, state: &mut SceState) {
        let Some(row) = self.stock.get(self.cursor).cloned() else {
            return;
        };
        {
            let mut p_state = state.global_state_mut().persistent_state_mut();
            if p_state.item_count(row.id) == 0 {
                return;
            }
            p_state.remove_item(row.id, 1);
            p_state.add_money(row.price);
        }
        self.notice = format!("卖了 {}，得 {} 文", row.name, row.price);
        self.rebuild_stock(state);
        if self.cursor >= self.stock.len() {
            self.cursor = self.stock.len().saturating_sub(1);
        }
    }

    fn draw(&self, ui: &Ui, state: &SceState) {
        let (money, capital, level_idx) = {
            let p = state.global_state().persistent_state();
            (p.money(), p.hockshop_capital(), p.hockshop_level())
        };
        let lv = *self.table.level(level_idx);
        let choices: Vec<i32> = (0..4)
            .map(|i| state.global_state().persistent_state().hockshop_choice(i))
            .collect();
        let entries = self.entries(state);

        let [window_width, _] = ui.io().display_size;
        ui.window("HockShop")
            .collapsible(false)
            .title_bar(false)
            .resizable(false)
            .always_auto_resize(true)
            .position([16., 16.], Condition::Always)
            .size_constraints([window_width * 0.45, 0.], [window_width * 0.9, f32::MAX])
            .build(|| {
                let _font = radiance::imgui::game_font(radiance::imgui::GameFontSize::LARGE)
                    .map(|f| ui.push_font(f));

                ui.text(format!(
                    "永安当（{} 级）　资本 {} 文　身上 {} 文",
                    LEVEL_NAMES[level_idx as usize], capital, money
                ));
                ui.text(format!(
                    "经营方针 {}　当期 {}　利率 {}　拆当率 {}",
                    MODE_NAMES[choices[0].clamp(0, 3) as usize],
                    TIER_NAMES[choices[1].clamp(0, 2) as usize],
                    TIER_NAMES[choices[2].clamp(0, 2) as usize],
                    TIER_NAMES[choices[3].clamp(0, 2) as usize],
                ));
                if !self.notice.is_empty() {
                    ui.text_colored([1.0, 0.85, 0.35, 1.0], &self.notice);
                }
                ui.separator();

                match self.page {
                    Page::Menu => {
                        for (i, e) in entries.iter().enumerate() {
                            let marker = if i == self.cursor { "▶" } else { "  " };
                            ui.text(format!("{marker}{}", e.label()));
                        }
                    }
                    Page::Invest => {
                        let after = capital + self.amount;
                        ui.text(format!(
                            "投入 {} 文（左右调，最多 {}）",
                            self.amount,
                            max_investment(capital, money)
                        ));
                        ui.text(format!(
                            "投完资本 {} 文 → {} 级（满格 {} 文）",
                            after,
                            LEVEL_NAMES[crate::openpal3::states::hockshop::level_for_capital(after)
                                as usize],
                            crate::openpal3::states::hockshop::CAPITAL_FOR_TOP
                        ));
                        ui.text("确定：投　取消：回去");
                    }
                    Page::DealIn { which } => {
                        for i in 0..4 {
                            let marker = if i == which { "▶" } else { "  " };
                            let value = if i == 0 {
                                MODE_NAMES[choices[0].clamp(0, 3) as usize]
                            } else {
                                TIER_NAMES[choices[i].clamp(0, 2) as usize]
                            };
                            ui.text(format!("{marker}{}　{}", CHOICE_NAMES[i], value));
                        }
                        let rate = lv.dividend_rate(
                            choices[0].max(0) as usize,
                            choices[1].max(0) as usize,
                            choices[2].max(0) as usize,
                            choices[3].max(0) as usize,
                        );
                        ui.text(format!(
                            "这套配法的红利：资本的 {:.1}%（下次分红 {} 文）",
                            rate * 100.,
                            (capital as f32 * rate) as i32
                        ));
                        ui.text("左右换　确定/取消：回去");
                    }
                    Page::Buy => {
                        if self.dead_pawn.is_empty() {
                            ui.text("今天没有死当物。");
                        }
                        for (i, row) in self.dead_pawn.iter().enumerate() {
                            let marker = if i == self.cursor { "▶" } else { "  " };
                            ui.text(format!(
                                "{marker}{}　{} 文　已有 {}",
                                row.name, row.price, row.owned
                            ));
                        }
                        ui.text(format!(
                            "（{} 级打 {:.0} 折）确定：买一件　取消：回去",
                            LEVEL_NAMES[level_idx as usize],
                            lv.buy_discount * 10.
                        ));
                    }
                    Page::Sell => {
                        if self.stock.is_empty() {
                            ui.text("身上没有能卖的东西。");
                        }
                        for (i, row) in self.stock.iter().enumerate() {
                            let marker = if i == self.cursor { "▶" } else { "  " };
                            ui.text(format!(
                                "{marker}{}　{} 文　有 {}",
                                row.name, row.price, row.owned
                            ));
                        }
                        ui.text("确定：卖一件　取消：回去");
                    }
                }
            });
    }

    fn finish(&mut self, state: &mut SceState) {
        if self.end_proc > 0 {
            state.call_proc(self.end_proc as u32);
        }
    }
}

impl SceCommand for SceCommandMiniGamePawnshop {
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
            self.finish(state);
            return true;
        }

        self.draw(ui, state);

        if let Some(input) = Self::poll_input(state) {
            if self.handle(state, input) {
                self.finish(state);
                return true;
            }
        }
        false
    }
}

/// 读一个 GBK 文本文件。
fn read_gbk(vfs: &mini_fs::MiniFs, path: &std::path::Path) -> Option<String> {
    use common::store_ext::StoreExt2;
    use encoding::Encoding;
    let bytes = vfs.read_to_end(path).ok()?;
    encoding::all::GBK
        .decode(&bytes, encoding::DecoderTrap::Ignore)
        .ok()
}
