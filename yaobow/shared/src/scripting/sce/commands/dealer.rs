//! 100 买卖系统 — `LAB_00436c00`。
//!
//! 规则在 `openpal3::states::shop`（照 `FUN_00457490` / `FUN_00461c50` /
//! `FUN_00460a90` 写的，见 `switch/re/notes/shop.md`）；这里是把它接到脚本上：
//! 命令在买卖期间阻塞脚本（原版是 `FUN_0044f610(2)` 进模式 2 再挂一个等待
//! 任务），玩家关掉店面脚本才继续。
//!
//! **界面不是原版那一套**。原版的五个列表框布局在 `FUN_004593c0` /
//! `FUN_00459970` 里，是按控件基类算出来的相对矩形，还没逐字段读通；这里先
//! 照战斗那样画一个文字浮层，把买卖跑通。换成像素级的界面是纯渲染改动。
//!
//! **卖**那一半没有：卖价算式还没在 exe 里定位（见 shop.md §5）。

use crate::openpal3::states::shop::{
    DealScript, Haggle, ShopRng, TIERS, buyable, discounted, haggle, price_to_pay,
};
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{Condition, Ui};
use radiance::comdef::ISceneManager;
use radiance::input::Key;
use std::path::PathBuf;

/// 店里的一行。
#[derive(Debug, Clone)]
struct Good {
    id: u32,
    name: String,
    price: i32,
    /// 背包里已经有几个——每种最多买到 99 个（`FUN_0045b7a0`）。
    owned: u32,
}

/// 光标在哪一层。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// 挑东西、调数量。
    Browse,
    /// 挑折扣档；`TIERS` 那一格是「照原价买」。
    Haggle { index: usize },
}

/// 按键，和战斗菜单同一套。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Input {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Cancel,
}

const TIER_LABELS: [&str; TIERS] = ["五折", "六折", "七折", "八折", "九折"];

#[derive(Debug, Clone)]
pub struct SceCommandDealerMenu {
    shop_file: String,
    shop: DealScript,
    goods: Vec<Good>,
    cursor: usize,
    /// 这一单要买几个。
    quantity: i32,
    mode: Mode,
    /// 这家店已经拒绝过几次；到了忍耐度就强制成交（原版 `+0x19d98`）。
    refusals: i32,
    /// 被严词拒绝过之后，再买东西说的是 `HarshTalkbuy`。
    harsh: bool,
    /// 商人现在说的话。
    talk: String,
    rng: ShopRng,
    loaded: bool,
}

impl SceCommandDealerMenu {
    pub fn new(shop: String) -> Self {
        Self {
            shop_file: shop,
            shop: DealScript::default(),
            goods: Vec::new(),
            cursor: 0,
            quantity: 1,
            mode: Mode::Browse,
            refusals: 0,
            harsh: false,
            talk: String::new(),
            rng: ShopRng::new(0x1234_5678),
            loaded: false,
        }
    }

    /// `DealScript\anyShop\q01any.txt` 这样的脚本参数转成 vfs 路径。包里的
    /// 名字是大小写混着的（脚本写 `anyShop`、包里是 `AnyShop`），cpk 的查找
    /// 本来就按小写哈希，所以直接拼就行。
    fn vfs_path(shop_file: &str) -> PathBuf {
        PathBuf::from(format!(
            "/basedata/basedata/{}",
            shop_file.replace('\\', "/")
        ))
    }

    fn load(&mut self, state: &mut SceState) {
        self.loaded = true;
        let path = Self::vfs_path(&self.shop_file);
        let Some(text) = read_gbk(state.asset_mgr().vfs(), &path) else {
            log::error!("DealerMenu: 读不到 {}", path.display());
            return;
        };
        self.shop = DealScript::parse(&text);

        let db = state.global_state().game_db().cloned();
        let p_state = state.global_state().persistent_state();
        self.goods = self
            .shop
            .goods()
            .into_iter()
            .filter_map(|id| {
                let row = db.as_ref()?.item(id)?;
                Some(Good {
                    id,
                    name: row
                        .name
                        .as_str()
                        .map(|n| n.to_string())
                        .unwrap_or_else(|_| format!("道具#{id}")),
                    price: row.price,
                    owned: p_state.item_count(id),
                })
            })
            .collect();

        // 开场白，随后进买的页面（原版分别是 BeginTalk 和 EntryBuy）。
        self.talk = if self.shop.entry_buy.is_empty() {
            self.shop.begin_talk.clone()
        } else {
            format!("{}\n{}", self.shop.begin_talk, self.shop.entry_buy)
        };
        // 掷骰的种子跟着这一单走，跑起来才不是每次一样。
        self.rng =
            ShopRng::new(state.global_state().persistent_state().money() as u32 ^ 0x5bd1_e995);
    }

    fn current(&self) -> Option<&Good> {
        self.goods.get(self.cursor)
    }

    /// 这一件还能买几个。
    fn buyable_now(&self) -> i32 {
        self.current().map(|g| buyable(g.owned)).unwrap_or(0)
    }

    fn total(&self) -> i32 {
        self.current()
            .map(|g| g.price * self.quantity.max(0))
            .unwrap_or(0)
    }

    fn clamp_quantity(&mut self) {
        let max = self.buyable_now();
        if max <= 0 {
            self.quantity = 0;
        } else {
            self.quantity = self.quantity.clamp(1, max);
        }
    }

    /// 两句里随机挑一句（原版 `(rand & 1) + tier*2` 那个下标）。
    fn pick(&mut self, lines: &[String]) -> String {
        if lines.is_empty() {
            return String::new();
        }
        let i = (self.rng.rand15() as usize) % lines.len();
        lines[i].clone()
    }

    /// 付钱、进货。返回 false 表示钱不够。
    fn pay(&mut self, state: &mut SceState, price: i32) -> bool {
        let Some(good) = self.current().cloned() else {
            return false;
        };
        let mut p_state = state.global_state_mut().persistent_state_mut();
        if p_state.money() < price {
            return false;
        }
        p_state.add_money(-price);
        p_state.add_item(good.id, self.quantity.max(0) as u32);
        let owned = p_state.item_count(good.id);
        drop(p_state);
        if let Some(g) = self.goods.get_mut(self.cursor) {
            g.owned = owned;
        }
        self.clamp_quantity();
        true
    }

    /// 不还价，照原价买（`DK == 0` 的店，或者还价页里选「照原价买」）。
    fn buy_at_full_price(&mut self, state: &mut SceState) {
        let total = self.total();
        if self.quantity < 1 {
            return;
        }
        if self.pay(state, total) {
            self.talk = if self.harsh && !self.shop.harsh_talk_buy.is_empty() {
                self.shop.harsh_talk_buy.clone()
            } else {
                self.shop.normal_accept_talk.clone()
            };
        } else {
            self.talk = self.shop.no_enough_money.clone();
        }
        self.mode = Mode::Browse;
    }

    /// 还一次价。
    fn haggle_at(&mut self, state: &mut SceState, tier: usize) {
        if self.quantity < 1 {
            return;
        }
        let total = self.total();
        let money = state.global_state().persistent_state().money();
        let outcome = haggle(
            &self.shop,
            tier,
            total,
            money,
            &mut self.refusals,
            &mut self.rng,
        );

        match outcome {
            Haggle::NotEnoughMoney => {
                self.talk = self.shop.no_enough_money.clone();
                self.mode = Mode::Browse;
            }
            Haggle::Refused => {
                let lines = self.shop.tiers[tier].refuse_talk.clone();
                self.talk = self.pick(&lines);
            }
            Haggle::Harsh => {
                let lines = self.shop.tiers[tier].harsh_talk.clone();
                self.talk = self.pick(&lines);
                self.harsh = true;
                self.mode = Mode::Browse;
            }
            Haggle::Accepted | Haggle::ForceAccepted => {
                let price = price_to_pay(outcome, total, tier).unwrap_or(total);
                let lines = if outcome == Haggle::Accepted {
                    self.shop.tiers[tier].accept_talk.clone()
                } else {
                    self.shop.tiers[tier].force_accept_talk.clone()
                };
                let mut line = self.pick(&lines);
                if outcome == Haggle::ForceAccepted
                    && !self.shop.tiers[tier].force_accept_reply.is_empty()
                {
                    line = format!("{}\n{}", line, self.shop.tiers[tier].force_accept_reply);
                }
                if self.pay(state, price) {
                    self.refusals = 0;
                    self.talk = line;
                } else {
                    self.talk = self.shop.no_enough_money.clone();
                }
                self.mode = Mode::Browse;
            }
        }
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

    /// 返回 true = 关店，脚本继续。
    fn handle(&mut self, state: &mut SceState, input: Input) -> bool {
        match self.mode {
            Mode::Browse => match input {
                Input::Up => {
                    if !self.goods.is_empty() {
                        self.cursor = (self.cursor + self.goods.len() - 1) % self.goods.len();
                        self.quantity = 1;
                        self.clamp_quantity();
                    }
                }
                Input::Down => {
                    if !self.goods.is_empty() {
                        self.cursor = (self.cursor + 1) % self.goods.len();
                        self.quantity = 1;
                        self.clamp_quantity();
                    }
                }
                Input::Left => {
                    self.quantity -= 1;
                    self.clamp_quantity();
                }
                Input::Right => {
                    self.quantity += 1;
                    self.clamp_quantity();
                }
                Input::Confirm => {
                    if self.buyable_now() < 1 {
                        return false;
                    }
                    if self.shop.haggling {
                        self.mode = Mode::Haggle { index: TIERS };
                    } else {
                        self.buy_at_full_price(state);
                    }
                }
                Input::Cancel => {
                    self.talk = self.shop.leave_talk.clone();
                    return true;
                }
            },
            Mode::Haggle { index } => {
                // 五档 + 「照原价买」，一共 TIERS + 1 项。
                let n = TIERS + 1;
                match input {
                    Input::Up | Input::Left => {
                        self.mode = Mode::Haggle {
                            index: (index + n - 1) % n,
                        }
                    }
                    Input::Down | Input::Right => {
                        self.mode = Mode::Haggle {
                            index: (index + 1) % n,
                        }
                    }
                    Input::Confirm => {
                        if index >= TIERS {
                            self.buy_at_full_price(state);
                        } else {
                            self.haggle_at(state, index);
                        }
                    }
                    Input::Cancel => {
                        self.talk = self.shop.normal_cancel_talk.clone();
                        self.mode = Mode::Browse;
                    }
                }
            }
        }
        false
    }

    fn draw(&self, ui: &Ui, money: i32) {
        let [window_width, _] = ui.io().display_size;
        ui.window("Dealer")
            .collapsible(false)
            .title_bar(false)
            .resizable(false)
            .always_auto_resize(true)
            .position([16., 16.], Condition::Always)
            .size_constraints([window_width * 0.4, 0.], [window_width * 0.9, f32::MAX])
            .build(|| {
                let _font = radiance::imgui::game_font(radiance::imgui::GameFontSize::LARGE)
                    .map(|f| ui.push_font(f));

                ui.text(format!("{}　　身上 {} 文", self.shop.merchant_name, money));
                if !self.talk.is_empty() {
                    ui.separator();
                    for line in self.talk.lines() {
                        ui.text(line);
                    }
                }

                ui.separator();
                for (i, g) in self.goods.iter().enumerate() {
                    let marker = if i == self.cursor { "▶" } else { "  " };
                    let line = format!("{marker}{}　{} 文　已有 {}", g.name, g.price, g.owned);
                    if buyable(g.owned) > 0 {
                        ui.text(line);
                    } else {
                        ui.text_disabled(line);
                    }
                }

                ui.separator();
                ui.text(format!(
                    "数量 {}（左右调，最多 {}）　总价 {} 文",
                    self.quantity,
                    self.buyable_now(),
                    self.total()
                ));

                match self.mode {
                    Mode::Browse => {
                        if self.shop.haggling {
                            ui.text("确定：还价　取消：离开");
                        } else {
                            ui.text("确定：买下　取消：离开");
                        }
                    }
                    Mode::Haggle { index } => {
                        ui.separator();
                        ui.text("还价：");
                        for t in 0..TIERS {
                            let marker = if t == index { "▶" } else { "  " };
                            ui.text(format!(
                                "{marker}{}　{} 文",
                                TIER_LABELS[t],
                                discounted(self.total(), t)
                            ));
                        }
                        let marker = if index >= TIERS { "▶" } else { "  " };
                        ui.text(format!("{marker}照原价买　{} 文", self.total()));
                    }
                }
            });
    }
}

impl SceCommand for SceCommandDealerMenu {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        if !self.loaded {
            self.load(state);
            if self.goods.is_empty() {
                log::warn!("DealerMenu: {} 没有商品，直接跳过", self.shop_file);
                return true;
            }
            self.clamp_quantity();
        }

        // 快进（脚本跳过）时不开店。
        if state.fast_forward() {
            return true;
        }

        let money = state.global_state().persistent_state().money();
        self.draw(ui, money);

        if let Some(input) = Self::poll_input(state) {
            if self.handle(state, input) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_path_becomes_a_vfs_path() {
        assert_eq!(
            SceCommandDealerMenu::vfs_path("DealScript\\anyShop\\q01any.txt"),
            PathBuf::from("/basedata/basedata/DealScript/anyShop/q01any.txt")
        );
    }
}
