//! 100 买卖系统 — `LAB_00436c00`。
//!
//! 规则在 `openpal3::states::shop`（照 `FUN_00457490` / `FUN_00461c50` /
//! `FUN_00460a90` 写的，见 `switch/re/notes/shop.md`）；这里是把它接到脚本上：
//! 命令在买卖期间阻塞脚本（原版是 `FUN_0044f610(2)` 进模式 2 再挂一个等待
//! 任务），玩家关掉店面脚本才继续。
//!
//! 界面照原版的像素布局画（`dealer_layout`，逆向记录
//! `switch/re/notes/dealer_layout.md`）：
//!
//! - 先是一面竖幡上挂几个按钮（`FUN_004593c0` / `FUN_00459970`），按店铺类型
//!   摆「买装备 / 买道具 / 卖东西 / 聊天 / 退出」；
//! - 点进去是三个全屏子页之一（买装备 `FUN_0045b990`、买道具 `FUN_0046df40`、
//!   卖 `FUN_00471590`）：左边框、右边列表、中间说明面板、底栏三个按钮。
//!   原版一单可以在好几行上各挑数量，「累计」是整单的钱，确定时一起结。
//!
//! 操作：十字键上下选行、左右调数量，A（空格）确定，Y（Q）还价，
//! X（E）换卖页的页签，B（Esc）回上一层。鼠标和原版一样点按钮。
//!
//! 买装备页左边是队里每人一张头像框的对比栏（`dealer_layout::compare`）；
//! 说明面板上方的黑框里是选中物品的 3D 模型（`dealer_layout::preview`）。
//!
//! 列表右边的滚动条照原版（`dealer_layout::Scrollbar`）：上下钮一行一行滚，点槽翻一页，
//! 滑块可以拖。
//!
//! 按钮音效照原版（[`ui_sound`]）：按钮 wa001，加减数量动了 wa005、动不了 wa004。

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use super::appraise::{overlay, virtual_mouse};
use super::dealer_layout::{
    self as lay, BUY_LIST, INK, ListGeom, MenuButton, Rect, SELL_LIST, ScrollPart, compare as cmp,
    inside, page,
};
use super::encamp::draw_sprite;
use super::item_preview::ItemPreview;
use super::ui_sound;
use crate::openpal3::assis_lang;
use crate::openpal3::states::attributes::{SLOT_ATTACK, SLOT_DEFENSE, SLOT_LUCK, SLOT_SPEED};
use crate::openpal3::states::game_db::PLAYER_ACTOR_IDS;
use crate::openpal3::states::persistent_state::{
    SLOT_ADORN1, SLOT_ADORN2, SLOT_CLOTH, SLOT_HAT, SLOT_SHOES, SLOT_WEAPON,
    equipment_slots_for_type,
};
use crate::openpal3::states::shop::{
    DealScript, Haggle, ShopRng, TIERS, buyable, haggle, price_to_pay, sell_price, sellable,
};
use crate::openpal3::ui_tex::UiTextures;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{MouseButton, Ui};
use radiance::comdef::ISceneManager;
use radiance::imgui::{GameFontSize, game_font};
use radiance::input::Key;
use radiance::rendering::Sprite;

const LANG_PATH: &str = "/basedata/basedata/AssisLang.lang";
const LANG_SECTION: &str = "sec1";

/// AssisLang 里用到的几格（`FUN_0045b990` 那几处 `&Lang[n]`）。
mod lang {
    pub const NAME: usize = 2;
    pub const PRICE: usize = 3;
    pub const COUNT: usize = 4;
    pub const HELD: usize = 44;
    pub const TOTAL: usize = 123;
    /// 没人能装备（`FUN_00462120` 里数完五个人都不能穿时显示的
    /// `DAT_00dd4a3c`）。槽位是这么定的：exe 里 `&DAT_00dc733c + 角色号·0x100`
    /// 是五个主角名，而表里第 64–68 条正好是景天/雪见/龙葵/紫萱/长卿，
    /// 所以 `LangN` 在 `0xdc733c + (N−64)·0x100`；代进 `0xdd4a3c` 得 N = 279，
    /// 内容「没有人可以装备」。（这张表共 400 条，下标就是编号。）
    pub const NOBODY_CAN_WEAR: usize = 279;
}

/// 原版菜单背后每帧再贴一张场景截图（`0x459900`）：打开时 `FUN_004bdc40` 把场景按
/// 512×384 渲染进一张渲染目标，之后用白色、alpha 0x3e 拉满全屏叠上去。只有底下的
/// 场景还在画时，这样一张 24% 的叠图才有意义——效果是场景上一层淡淡的虚影，不是
/// 压暗 [推断]。移植端直接让场景照常画，不另加这一层。

/// 选中行的字色（见 `draw_page`）。
const SELECTED_INK: [f32; 4] = [1.0, 0.98, 0.98, 1.0];

pub(super) type Tex = Option<Rc<Sprite>>;

/// 店里或背包里的一行。
#[derive(Debug, Clone)]
struct Good {
    id: u32,
    name: String,
    description: String,
    item_type: u8,
    /// 预览相机按它挑（`weapon_type` 5 另有一档）。
    weapon_type: u8,
    price: i32,
    /// 背包里已经有几个——每种最多买到 99 个（`FUN_0045b7a0`）。
    owned: u32,
    /// 武速防运（对比栏用），见 [`stats_of`]。
    stats: [i32; cmp::STATS],
    /// 五个人谁能穿（gdb `+0x266`）。
    applicable: [bool; 5],
}

/// 物品的武、速、防、运：gdb `attribute_impact_value` 的 3、5、4、6 格
/// （`+0x28a/+0x28e/+0x28c/+0x290`，`FUN_004623a0` 按这个顺序比）。
fn stats_of(row: &fileformats::pal3::gdb::GameItemInfo) -> [i32; cmp::STATS] {
    [SLOT_ATTACK, SLOT_SPEED, SLOT_DEFENSE, SLOT_LUCK].map(|k| row.attribute_impact_value[k] as i32)
}

/// 一类装备在对比栏里占哪几组：（身上哪一格、左右、大图标号）。大图标号是
/// [`Skin::big`] 的下标（衣、武器、饰品、帽、鞋、饰品——`FUN_0045b990` 建控件的
/// 顺序）。饰品有两格，两边各比一次。
fn compare_groups(item_type: u8) -> &'static [(usize, cmp::Side, usize)] {
    use cmp::Side::{Left, Right};
    match item_type {
        7 => &[(SLOT_CLOTH, Left, 0)],
        8 => &[(SLOT_HAT, Right, 3)],
        9 => &[(SLOT_SHOES, Right, 4)],
        10 => &[(SLOT_ADORN1, Left, 2), (SLOT_ADORN2, Right, 5)],
        11 => &[(SLOT_WEAPON, Left, 1)],
        _ => &[],
    }
}

/// 头像下那排小图标对应身上哪一格（衣、帽、鞋、武器、饰品、饰品）。
const SMALL_SLOTS: [usize; 6] = [
    SLOT_CLOTH,
    SLOT_HAT,
    SLOT_SHOES,
    SLOT_WEAPON,
    SLOT_ADORN1,
    SLOT_ADORN2,
];

/// 对比栏里一个人。
/// 买的时候就穿上的那一件（原版 `FUN_00469eb0` 把它从这一行的数量里挪到「已穿」
/// `+0x16d9c`，记下谁的哪一格 `+0x16d24`）。结账时一起付钱、穿上。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Worn {
    id: u32,
    item_type: u8,
    /// 角色号 0..4。
    role: usize,
    /// 身上哪一格（`SLOT_*`）。
    slot: usize,
}

#[derive(Debug, Clone)]
struct Card {
    role: usize,
    head: Rect,
    /// 小图标亮不亮：那一格穿着东西 [推断]。已查清的部分：`FUN_00462280`
    /// 对这一行的装备，把队里能穿它的人（`物品+0x266+人`）那一格小图标
    /// **开闪**（`FUN_00444d10`），`FUN_00462340` 五个一起停闪
    /// （`FUN_00444d20`）——这两个管的是闪，不是亮；预穿的状态记在
    /// `+0x16d00`（开关）、`+0x16d01+人`（每格一字节）、`+0x16d24`（每格的
    /// 物品号）。进页时拿现有装备点亮这件事仍没追到，保持现在的读法。
    small: [bool; 6],
    /// 这件装备能穿时：（左右、大图标号、箭头）。
    groups: Vec<(cmp::Side, usize, Vec<cmp::Arrow>)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Equip,
    Prop,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Screen {
    Menu,
    Page(Page),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browse,
    /// 五个折扣档（`bDK5..9`）亮着，`index` 是手柄挑的那一档。
    Haggle {
        index: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Input {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Cancel,
    Dicker,
    NextTab,
}

/// 鼠标落在什么上面。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hit {
    /// 买装备页「装备」（`bEquipYes`）。
    EquipYes,
    /// 「不装备」（`bEquipNo`）。
    EquipNo,
    /// 对比栏第几张卡的第几个小图标（选穿到谁的哪一格）。
    Slot(usize, usize),
    Menu(usize),
    /// 列表第几个**可见**行。
    Row(usize),
    Yes,
    Dicker,
    No,
    Exit,
    Tier(usize),
    Tab(usize),
    Add,
    Sub,
    Scroll(ScrollPart),
}

/// 卖页六个页签（`FUN_0040d010` 的 0/1/2/3/4/6 号表，`shop.md` §5.2）。
fn sell_tab_of(item_type: u8) -> Option<usize> {
    match item_type {
        1 => Some(0),
        3 => Some(1),
        2 => Some(2),
        5 | 12 => Some(3),
        4 => Some(4),
        7..=11 => Some(5),
        _ => None,
    }
}

const SELL_TAB_TEX: [&str; 6] = [
    "UI\\GameMainUI\\Prop\\Resume",
    "UI\\GameMainUI\\Prop\\Treasure",
    "UI\\GameMainUI\\Prop\\Attack",
    "UI\\GameMainUI\\Prop\\Smelt",
    "UI\\GameMainUI\\Prop\\Ancient",
    "UI\\GameMainUI\\Equip\\Cloth",
];

/// 这一界面用到的贴图。三态的按钮按原版命名 `<基名>0/1/2.tga`：常态、鼠标
/// 悬停、按下（选中）。
#[derive(Clone, Default)]
struct Skin {
    flag_t: Tex,
    flag_m: Tex,
    flag_b: Tex,
    rawbtn: [Tex; 3],
    buy_flag: Tex,
    sale_flag: Tex,
    frame_bar: Tex,
    frame: Tex,
    list_frame: Tex,
    info: Tex,
    equip_word: Tex,
    prop_word: Tex,
    buy_grid: Tex,
    sale_grid: Tex,
    selected: Tex,
    tab_bar: Tex,
    yes: [Tex; 3],
    no: [Tex; 3],
    equip_yes: [Tex; 3],
    equip_no: [Tex; 3],
    dicker: [Tex; 3],
    undicker: Tex,
    exit: [Tex; 3],
    add: [Tex; 3],
    sub: [Tex; 3],
    unadd: Tex,
    unsub: Tex,
    tiers: [[Tex; 3]; TIERS],
    tabs: [[Tex; 3]; 6],
    /// `scrollbar0..9.tga`，见 [`lay::Scrollbar`]。
    scroll: [Tex; 10],
    heads: [Tex; 5],
    big: [Tex; 6],
    small: [Tex; 6],
    up: [Tex; cmp::STATS],
    down: [Tex; cmp::STATS],
}

impl Skin {
    fn load(state: &SceState) -> Self {
        let am = state.asset_mgr().clone();
        let mut t = UiTextures::new(&am);
        let mut one = |name: &str| t.get(&am, name);
        let mut three = |base: &str| -> [Tex; 3] {
            [
                t_get(&mut one, base, 0),
                t_get(&mut one, base, 1),
                t_get(&mut one, base, 2),
            ]
        };
        fn t_get(one: &mut dyn FnMut(&str) -> Tex, base: &str, i: usize) -> Tex {
            one(&format!("{base}{i}.tga"))
        }
        let rawbtn = three("SubGame\\HockShop\\Control\\rawbtn");
        let yes = three("UI\\GameMainUI\\MUIDialog\\Normal\\Yes");
        let no = three("UI\\GameMainUI\\MUIDialog\\Normal\\No");
        let dicker = three("UI\\Assistant\\Communal\\bDicker");
        let exit = three("UI\\GameMainUI\\Communal\\StpCtl\\Exit");
        let add = three("UI\\Assistant\\Communal\\Add");
        let sub = three("UI\\Assistant\\Communal\\Sub");
        let tiers = std::array::from_fn(|k| three(&format!("UI\\Assistant\\Equip\\bDK{}", k + 5)));
        let tabs = std::array::from_fn(|k| three(SELL_TAB_TEX[k]));
        let equip_yes = three("UI\\Assistant\\Equip\\bEquipYes");
        let equip_no = three("UI\\Assistant\\Equip\\bEquipNo");
        drop(three);
        const EQ: &str = "UI\\Assistant\\Equip\\";
        let heads =
            std::array::from_fn(|i| one(&format!("UI\\GameMainUI\\Communal\\AttFrameHead{i}.tga")));
        let big = ["Cloth", "Weapon", "Adorn", "Hat", "Shoes", "Adorn"]
            .map(|n| one(&format!("{EQ}{n}.tga")));
        let small = ["Cloth", "Hat", "Shoes", "Weapon", "Adorn", "Adorn"]
            .map(|n| one(&format!("{EQ}{n}_small.tga")));
        let up = ["W", "S", "F", "Y"].map(|n| one(&format!("{EQ}{n}_up.tga")));
        let down = ["W", "S", "F", "Y"].map(|n| one(&format!("{EQ}{n}_down.tga")));
        let scroll = std::array::from_fn(|i| {
            one(&format!(
                "UI\\GameMainUI\\Communal\\Scroll\\scrollbar{i}.tga"
            ))
        });
        Self {
            equip_yes,
            equip_no,
            scroll,
            heads,
            big,
            small,
            up,
            down,
            flag_t: one("UI\\Assistant\\Deal\\FlagT.tga"),
            flag_m: one("UI\\Assistant\\Deal\\FlagM.tga"),
            flag_b: one("UI\\Assistant\\Deal\\FlagB.tga"),
            rawbtn,
            buy_flag: one("UI\\Assistant\\Communal\\BuyFlag.tga"),
            sale_flag: one("UI\\Assistant\\Communal\\SaleFlag.tga"),
            frame_bar: one("UI\\Flex\\framebar.tga"),
            frame: one("UI\\Assistant\\Communal\\Frame.tga"),
            list_frame: one("UI\\GameMainUI\\Equip\\ListFrame.tga"),
            info: one("UI\\GameMainUI\\Communal\\Info.tga"),
            equip_word: one("UI\\GameMainUI\\Communal\\Equip_word.tga"),
            prop_word: one("UI\\GameMainUI\\Communal\\Prop_word.tga"),
            buy_grid: one("UI\\Assistant\\Communal\\Buy_Grid.tga"),
            sale_grid: one("UI\\Assistant\\Communal\\Sale_Grid.tga"),
            selected: one("UI\\Assistant\\Communal\\ListBoxSelPic.tga"),
            tab_bar: one("UI\\Flex\\Bar.tga"),
            yes,
            no,
            dicker,
            undicker: one("UI\\Assistant\\Communal\\UnDicker.tga"),
            exit,
            add,
            sub,
            unadd: one("UI\\Assistant\\Communal\\UnAdd.tga"),
            unsub: one("UI\\Assistant\\Communal\\UnSub.tga"),
            tiers,
            tabs,
        }
    }
}

#[derive(Clone)]
pub struct SceCommandDealerMenu {
    shop_file: String,
    shop: DealScript,
    loaded: bool,
    screen: Screen,
    buttons: Vec<MenuButton>,
    menu_cursor: usize,
    equip_goods: Vec<Good>,
    prop_goods: Vec<Good>,
    /// 背包里能卖的全部东西，按 id 升序。
    stock: Vec<Good>,
    /// 卖页当前页签下的那几行。
    sell_rows: Vec<Good>,
    sell_tab: usize,
    cursor: usize,
    /// 列表滚到哪一行在最上面。
    top: usize,
    /// 这一单：物品 id → 个数。原版每行各自记数量，确定时一起结。
    cart: HashMap<u32, i32>,
    mode: Mode,
    /// 这家店已经拒绝过几次；到了忍耐度就强制成交（原版 `+0x19d98`）。
    refusals: i32,
    /// 被严词拒绝过之后，再买东西说的是 `HarshTalkbuy`。
    harsh: bool,
    /// 商人现在说的话（原版画在页面 `+0x17604` 那个文本框里）。
    talk: String,
    rng: ShopRng,
    lang: Vec<String>,
    skin: Skin,
    last_mouse: (f32, f32),
    /// 点了「聊天」：关店后跑商人的过程（`FUN_00432170(M_ID)`）。
    chat_proc: Option<u16>,
    /// 买装备页左边的对比栏，每帧从队伍和身上的装备重算。
    cards: Vec<Card>,
    /// 箭头跳动用的钟，换选中行时归零。
    clock: f32,
    /// 预览的是哪件物品、建出来没有（模型缺了就是 `None`，不每帧重试）。
    preview: Option<(u32, Option<Rc<ItemPreview>>)>,
    frame_dt: f32,
    /// 正在拖滑块：鼠标离滑块上边多远。
    drag: Option<f32>,
    /// 鼠标左键按着（滚动条按钮画按下的样子）。
    mouse_down: bool,
    /// 这一帧要放的按钮音效。
    sfx: Option<&'static str>,
    /// 「装备 / 不装备」亮着（`FUN_00462220(1)`）。
    ask_equip: bool,
    /// 点了「装备」，等点对比栏里某人的某一格（`+0x16d00`）。
    wear_mode: bool,
    /// 进「选谁穿」多久了：能穿的人那一格小图标跟着闪（[`flash_alpha`]）。
    wear_clock: f32,
    /// 这一单里买了就穿的。
    worn: Vec<Worn>,
    /// 当铺借用的买页：每行是池子里的一件，最多买 1 个，买下就从列表里去掉。
    single_each: bool,
    /// 这次结账买下的物品 id（当铺拿去把池子里那件标成卖掉）。
    bought: Vec<u32>,
}

impl std::fmt::Debug for SceCommandDealerMenu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandDealerMenu")
            .field("shop_file", &self.shop_file)
            .field("screen", &self.screen)
            .field("mode", &self.mode)
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl SceCommandDealerMenu {
    pub fn new(shop: String) -> Self {
        Self {
            shop_file: shop,
            shop: DealScript::default(),
            loaded: false,
            screen: Screen::Menu,
            buttons: vec![],
            menu_cursor: 0,
            equip_goods: vec![],
            prop_goods: vec![],
            stock: vec![],
            sell_rows: vec![],
            sell_tab: 0,
            cursor: 0,
            top: 0,
            cart: HashMap::new(),
            mode: Mode::Browse,
            refusals: 0,
            harsh: false,
            talk: String::new(),
            rng: ShopRng::new(0x1234_5678),
            lang: vec![],
            skin: Skin::default(),
            last_mouse: (-1., -1.),
            chat_proc: None,
            cards: vec![],
            clock: 0.,
            preview: None,
            frame_dt: 0.,
            drag: None,
            mouse_down: false,
            sfx: None,
            ask_equip: false,
            wear_mode: false,
            wear_clock: 0.,
            worn: vec![],
            single_each: false,
            bought: vec![],
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
        self.lang = read_gbk(state.asset_mgr().vfs(), &PathBuf::from(LANG_PATH))
            .map(|t| assis_lang::parse_section(&t, LANG_SECTION))
            .unwrap_or_default();
        self.buttons = lay::menu_buttons(self.shop.shop_type);

        let shop = self.shop.clone();
        self.equip_goods = self.goods_of(state, &shop.equipment);
        self.prop_goods = self.goods_of(state, &shop.props);
        self.rebuild_stock(state);
        // 掷骰的种子跟着这一单走，跑起来才不是每次一样。
        self.rng =
            ShopRng::new(state.global_state().persistent_state().money() as u32 ^ 0x5bd1_e995);
    }

    fn good(state: &SceState, id: u32, owned: u32) -> Option<Good> {
        let db = state.global_state().game_db().cloned()?;
        let row = db.item(id)?;
        Some(Good {
            id,
            name: row
                .name
                .as_str()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| format!("道具#{id}")),
            description: row
                .description
                .as_str()
                .map(|d| d.to_string())
                .unwrap_or_default(),
            item_type: row.item_type,
            weapon_type: row.weapon_type,
            price: row.price,
            owned,
            stats: stats_of(row),
            applicable: row.applicable_actors.map(|a| a != 0),
        })
    }

    fn goods_of(&self, state: &SceState, ids: &[u32]) -> Vec<Good> {
        let p_state = state.global_state().persistent_state();
        let owned: Vec<u32> = ids.iter().map(|id| p_state.item_count(*id)).collect();
        drop(p_state);
        ids.iter()
            .zip(owned)
            .filter_map(|(id, n)| Self::good(state, *id, n))
            .collect()
    }

    /// 背包里能卖的东西。`FUN_0040d010` 的六张表并起来就是「除剧情之外的
    /// 一切」，`BTreeMap` 的 id 升序也就是原版的列表顺序。
    fn rebuild_stock(&mut self, state: &SceState) {
        let held: Vec<(u32, u32)> = state
            .global_state()
            .persistent_state()
            .inventory()
            .iter()
            .filter(|(_, count)| **count > 0)
            .map(|(id, count)| (*id, *count))
            .collect();
        self.stock = held
            .into_iter()
            .filter_map(|(id, n)| Self::good(state, id, n))
            .filter(|g| sellable(g.item_type))
            .collect();
        self.refresh_sell_rows();
    }

    fn refresh_sell_rows(&mut self) {
        let tab = self.sell_tab;
        self.sell_rows = self
            .stock
            .iter()
            .filter(|g| sell_tab_of(g.item_type) == Some(tab))
            .cloned()
            .collect();
    }

    fn text(&self, slot: usize, fallback: &str) -> String {
        self.lang
            .get(slot)
            .filter(|s| !s.is_empty())
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
    }

    fn page(&self) -> Option<Page> {
        match self.screen {
            Screen::Page(p) => Some(p),
            Screen::Menu => None,
        }
    }

    fn geom(&self) -> &'static ListGeom {
        if self.page() == Some(Page::Sell) {
            &SELL_LIST
        } else {
            &BUY_LIST
        }
    }

    fn rows(&self) -> &[Good] {
        match self.page() {
            Some(Page::Equip) => &self.equip_goods,
            Some(Page::Prop) => &self.prop_goods,
            Some(Page::Sell) => &self.sell_rows,
            None => &[],
        }
    }

    fn current(&self) -> Option<&Good> {
        self.rows().get(self.cursor)
    }

    fn in_cart(&self, id: u32) -> i32 {
        self.cart.get(&id).copied().unwrap_or(0)
    }

    /// 这一行最多能挑几个：买是 `99 − 已持有`，卖是手上有几个。
    fn cap(&self, g: &Good) -> i32 {
        if self.page() == Some(Page::Sell) {
            g.owned as i32
        } else if self.single_each {
            1
        } else {
            buyable(g.owned)
        }
    }

    /// 「累计」：买是原价合计，卖是 `price / 2` 合计。
    fn total(&self) -> i32 {
        let sell = self.page() == Some(Page::Sell);
        let all = self
            .equip_goods
            .iter()
            .chain(&self.prop_goods)
            .chain(&self.stock);
        let mut seen = std::collections::HashSet::new();
        all.filter(|g| seen.insert(g.id))
            .filter_map(|g| {
                let n = self.in_cart(g.id);
                (n > 0).then(|| if sell { sell_price(g.price) } else { g.price } * n)
            })
            .sum::<i32>()
            + if sell {
                0
            } else {
                self.worn
                    .iter()
                    .map(|w| {
                        self.equip_goods
                            .iter()
                            .find(|g| g.id == w.id)
                            .map_or(0, |g| g.price)
                    })
                    .sum()
            }
    }

    /// 选中行或数量变了：这一行在单子里、还没人穿，就亮「装备 / 不装备」，商人的话收起
    /// （选行 0x46c2dc、加一个 0x46bce5 两处）；否则收起。
    fn prompt_equip(&mut self) {
        self.ask_equip = self.page() == Some(Page::Equip)
            && matches!(self.mode, Mode::Browse)
            && self
                .current()
                .is_some_and(|g| self.in_cart(g.id) > 0 && !self.worn.iter().any(|w| w.id == g.id));
    }

    /// 单子作废（取消、换页、离开，原版 `FUN_0046acd0`）：买了就穿的也一起撤掉。
    fn reset_wear(&mut self) {
        self.worn.clear();
        self.ask_equip = false;
        self.wear_mode = false;
    }

    /// 点了「装备」（0x129）：进选人状态。能穿的在队伍里只有一个、又不是饰品（两格要人挑）
    /// 时直接给他穿上；一个都没有就说一句（`FUN_00462120`）。
    fn equip_yes(&mut self, state: &SceState) {
        self.ask_equip = false;
        self.wear_mode = true;
        self.wear_clock = 0.;
        let Some(g) = self.current().cloned() else {
            return;
        };
        let who: Vec<usize> = {
            let p = state.global_state().persistent_state();
            (0..PLAYER_ACTOR_IDS.len())
                .filter(|&r| g.applicable[r] && p.is_in_team(PLAYER_ACTOR_IDS[r]))
                .collect()
        };
        match who.as_slice() {
            [] => {
                self.talk = self
                    .text(lang::NOBODY_CAN_WEAR, "没有人可以装备")
                    .to_string();
                self.wear_mode = false;
            }
            [r] if g.item_type != 10 => {
                if let Some(&slot) = equipment_slots_for_type(g.item_type).first() {
                    self.wear(*r, slot);
                }
            }
            _ => {}
        }
    }

    /// 手柄没有指针，按「装备」后有几个人能穿时给第一个能穿的穿上 [简化]。
    fn wear_first_fit(&mut self, state: &SceState) {
        let Some(g) = self.current().cloned() else {
            return;
        };
        if self.worn.iter().any(|w| w.id == g.id) {
            return;
        }
        let first = {
            let p = state.global_state().persistent_state();
            (0..PLAYER_ACTOR_IDS.len())
                .find(|&r| g.applicable[r] && p.is_in_team(PLAYER_ACTOR_IDS[r]))
        };
        if let (Some(r), Some(&slot)) = (first, equipment_slots_for_type(g.item_type).first()) {
            self.wear(r, slot);
        }
    }

    /// 把选中那一行的一件给 `role` 穿到 `slot`（`FUN_00469eb0`）：从单子里的数量挪一件出来；
    /// 那一格先前已经预订了别的，就把那件放回单子。不能穿、格子不对、单子里没有都不动。
    fn wear(&mut self, role: usize, slot: usize) {
        let Some(g) = self.current().cloned() else {
            return;
        };
        if !g.applicable.get(role).copied().unwrap_or(false)
            || !equipment_slots_for_type(g.item_type).contains(&slot)
            || self.in_cart(g.id) <= 0
        {
            return;
        }
        if let Some(i) = self
            .worn
            .iter()
            .position(|w| w.role == role && w.slot == slot)
        {
            if self.worn[i].id == g.id {
                return;
            }
            let old = self.worn.remove(i);
            *self.cart.entry(old.id).or_insert(0) += 1;
        }
        let n = self.in_cart(g.id) - 1;
        if n <= 0 {
            self.cart.remove(&g.id);
        } else {
            self.cart.insert(g.id, n);
        }
        self.worn.push(Worn {
            id: g.id,
            item_type: g.item_type,
            role,
            slot,
        });
    }

    fn enter(&mut self, page: Page) {
        self.screen = Screen::Page(page);
        self.mode = Mode::Browse;
        self.cursor = 0;
        self.top = 0;
        self.cart.clear();
        self.reset_wear();
        // `EntrySale` 确实是解析出来的字段（exe 里有一张 0x20 步长的字段名表：
        // `MC` / `Patience` / `BeginTalk` / `EntryBuy` / `EntrySale` /
        // `NormalAcptTalk`…），但卖页把它显示出来的地方没在 exe 里找到，
        // 照字段意思用 [推断]。
        self.talk = match page {
            Page::Sell => self.shop.entry_sale.clone(),
            _ => self.shop.entry_buy.clone(),
        };
    }

    fn back_to_menu(&mut self) {
        self.screen = Screen::Menu;
        self.mode = Mode::Browse;
        self.cart.clear();
        self.reset_wear();
    }

    fn select(&mut self, row: usize) {
        let n = self.rows().len();
        if n == 0 {
            return;
        }
        if self.cursor != row.min(n - 1) {
            self.clock = 0.;
        }
        self.cursor = row.min(n - 1);
        let vis = self.geom().visible_rows();
        if self.cursor < self.top {
            self.top = self.cursor;
        } else if self.cursor >= self.top + vis {
            self.top = self.cursor + 1 - vis;
        }
        self.prompt_equip();
    }

    /// 列表能往下滚几行。
    fn scroll_range(&self) -> usize {
        self.rows().len().saturating_sub(self.geom().visible_rows())
    }

    /// 只挪列表、不动选中行 [推断：原版上下钮是不是也挪选中行没追]。
    fn scroll_by(&mut self, delta: isize) {
        let range = self.scroll_range() as isize;
        self.top = (self.top as isize + delta).clamp(0, range) as usize;
    }

    fn step(&mut self, delta: isize) {
        let n = self.rows().len() as isize;
        if n > 0 {
            self.select(((self.cursor as isize + delta).rem_euclid(n)) as usize);
        }
    }

    /// 返回数量变没变。
    fn adjust(&mut self, delta: i32) -> bool {
        let Some(g) = self.current().cloned() else {
            return false;
        };
        let before = self.in_cart(g.id);
        let n = (before + delta).clamp(0, self.cap(&g).max(0));
        if n == 0 {
            self.cart.remove(&g.id);
        } else {
            self.cart.insert(g.id, n);
        }
        n != before
    }

    /// 加减钮 / 左右键：动了放 wa005，到头了放 wa004（`0x46bdc8` / `0x46c04c` 那两对分支）。
    fn nudge(&mut self, delta: i32) {
        self.sfx = Some(if self.adjust(delta) {
            ui_sound::TICK
        } else {
            ui_sound::DENIED
        });
        self.prompt_equip();
    }

    fn set_tab(&mut self, tab: usize) {
        self.sell_tab = tab % SELL_TAB_TEX.len();
        self.cursor = 0;
        self.top = 0;
        self.refresh_sell_rows();
    }

    /// 两句里随机挑一句（原版 `(rand & 1) + tier*2` 那个下标）。
    fn pick(&mut self, lines: &[String]) -> String {
        if lines.is_empty() {
            return String::new();
        }
        let i = (self.rng.rand15() as usize) % lines.len();
        lines[i].clone()
    }

    /// 付钱、把这一单的东西放进背包。返回 false 表示钱不够。
    fn checkout_buy(&mut self, state: &mut SceState, price: i32) -> bool {
        let mut p_state = state.global_state_mut().persistent_state_mut();
        if p_state.money() < price {
            return false;
        }
        p_state.add_money(-price);
        let mut just = vec![];
        for (id, n) in self.cart.drain() {
            p_state.add_item(id, n.max(0) as u32);
            if n > 0 {
                just.push(id);
            }
        }
        // 买了就穿的：先进背包再穿上，换下来的回背包。
        for w in self.worn.drain(..) {
            p_state.add_item(w.id, 1);
            p_state.equip_in_slot(PLAYER_ACTOR_IDS[w.role], w.id, w.item_type, w.slot);
        }
        self.ask_equip = false;
        self.wear_mode = false;
        for g in self
            .equip_goods
            .iter_mut()
            .chain(self.prop_goods.iter_mut())
        {
            g.owned = p_state.item_count(g.id);
        }
        drop(p_state);
        if self.single_each {
            self.prop_goods.retain(|g| !just.contains(&g.id));
            let n = self.rows().len();
            self.cursor = self.cursor.min(n.saturating_sub(1));
            self.top = self.top.min(self.scroll_range());
        }
        self.bought.extend(just);
        true
    }

    /// 当铺取走这几次结账买下的东西。
    pub(super) fn take_bought(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.bought)
    }

    /// 不还价，照原价买（底栏「确定」）。
    fn buy_at_full_price(&mut self, state: &mut SceState) {
        let total = self.total();
        if total <= 0 {
            return;
        }
        self.talk = if self.checkout_buy(state, total) {
            if self.harsh && !self.shop.harsh_talk_buy.is_empty() {
                self.shop.harsh_talk_buy.clone()
            } else {
                self.shop.normal_accept_talk.clone()
            }
        } else {
            self.shop.no_enough_money.clone()
        };
        self.mode = Mode::Browse;
    }

    /// 还一次价。
    fn haggle_at(&mut self, state: &mut SceState, tier: usize) {
        let total = self.total();
        if total <= 0 {
            self.mode = Mode::Browse;
            return;
        }
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
            // 每还一次价原版都先 `FUN_00461fd0(0)` 把五个折扣钮收起，商人的话才露出来；
            // 被拒了要再点「还价」重来。
            Haggle::Refused => {
                let lines = self.shop.tiers[tier].refuse_talk.clone();
                self.talk = self.pick(&lines);
                self.mode = Mode::Browse;
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
                if self.checkout_buy(state, price) {
                    self.refusals = 0;
                    self.talk = line;
                } else {
                    self.talk = self.shop.no_enough_money.clone();
                }
                self.mode = Mode::Browse;
            }
        }
    }

    /// 卖掉这一单：钱 += `price / 2` × 个数，东西从背包扣掉（`FUN_00474c80`
    /// 的六个同构分支）。卖不还价。
    fn sell(&mut self, state: &mut SceState) {
        if self.cart.is_empty() {
            return;
        }
        let price: HashMap<u32, i32> = self.stock.iter().map(|g| (g.id, g.price)).collect();
        {
            let mut p_state = state.global_state_mut().persistent_state_mut();
            for (id, n) in self.cart.drain() {
                let n = n.max(0) as u32;
                if n == 0 || p_state.item_count(id) < n {
                    continue;
                }
                p_state.remove_item(id, n);
                p_state.add_money(sell_price(price.get(&id).copied().unwrap_or(0)) * n as i32);
            }
        }
        self.rebuild_stock(state);
        let n = self.rows().len();
        self.select(self.cursor.min(n.saturating_sub(1)));
        self.talk = self.shop.normal_accept_talk.clone();
    }

    fn confirm(&mut self, state: &mut SceState) {
        match self.page() {
            Some(Page::Sell) => self.sell(state),
            Some(_) => self.buy_at_full_price(state),
            None => {}
        }
    }

    fn start_haggle(&mut self) {
        if self.page().is_some_and(|p| p != Page::Sell) && self.shop.haggling && self.total() > 0 {
            self.mode = Mode::Haggle { index: 0 };
        }
    }

    /// 返回 true = 关店。
    fn activate(&mut self, state: &SceState, button: MenuButton) -> bool {
        match button {
            MenuButton::BuyEquip => self.enter(Page::Equip),
            MenuButton::BuyProp => self.enter(Page::Prop),
            // 刚买的东西也要能卖：进卖页前按背包重排一遍。
            MenuButton::Sell => {
                self.rebuild_stock(state);
                self.enter(Page::Sell);
            }
            // `FUN_0045a600` case 7：`+0xa0` 是 0 就什么都不做。
            MenuButton::Chat => {
                if self.shop.merchant_id > 0 {
                    self.chat_proc = Some(self.shop.merchant_id);
                    return true;
                }
            }
            MenuButton::Leave => return true,
        }
        false
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
        } else if pressed(Key::Q) || pressed(Key::GamePadNorth) {
            Some(Input::Dicker)
        } else if pressed(Key::E) || pressed(Key::GamePadWest) {
            Some(Input::NextTab)
        } else {
            None
        }
    }

    /// 返回 true = 关店，脚本继续。
    fn handle(&mut self, state: &mut SceState, input: Input) -> bool {
        // 键盘 / 手柄按到的也是那几个按钮，音效同鼠标。
        let click = match (self.screen, self.mode, input) {
            (Screen::Menu, _, Input::Confirm) => true,
            (Screen::Page(_), Mode::Browse, Input::Confirm | Input::Dicker | Input::Cancel) => true,
            (Screen::Page(Page::Sell), Mode::Browse, Input::NextTab) => true,
            (
                Screen::Page(_),
                Mode::Haggle { .. },
                Input::Confirm | Input::Cancel | Input::Dicker,
            ) => true,
            _ => false,
        };
        if click {
            self.sfx = Some(ui_sound::CLICK);
        }
        match (self.screen, self.mode) {
            (Screen::Menu, _) => {
                let n = self.buttons.len().max(1);
                match input {
                    Input::Up => self.menu_cursor = (self.menu_cursor + n - 1) % n,
                    Input::Down => self.menu_cursor = (self.menu_cursor + 1) % n,
                    Input::Confirm => {
                        if let Some(b) = self.buttons.get(self.menu_cursor).copied() {
                            return self.activate(state, b);
                        }
                    }
                    Input::Cancel => return true,
                    _ => {}
                }
            }
            (Screen::Page(_), Mode::Browse) => match input {
                Input::Up => self.step(-1),
                Input::Down => self.step(1),
                Input::Left => self.nudge(-1),
                Input::Right => self.nudge(1),
                Input::Confirm => self.confirm(state),
                Input::Dicker => self.start_haggle(),
                Input::NextTab => {
                    if self.page() == Some(Page::Sell) {
                        self.set_tab(self.sell_tab + 1);
                    } else if self.ask_equip {
                        // 手柄：X 当「装备」。
                        self.sfx = Some(ui_sound::CLICK);
                        self.equip_yes(state);
                        if self.wear_mode {
                            self.wear_first_fit(state);
                        }
                    }
                }
                Input::Cancel => self.back_to_menu(),
            },
            (Screen::Page(_), Mode::Haggle { index }) => match input {
                Input::Up | Input::Left => {
                    self.mode = Mode::Haggle {
                        index: (index + TIERS - 1) % TIERS,
                    }
                }
                Input::Down | Input::Right => {
                    self.mode = Mode::Haggle {
                        index: (index + 1) % TIERS,
                    }
                }
                Input::Confirm => self.haggle_at(state, index),
                Input::Cancel | Input::Dicker => {
                    self.talk = self.shop.normal_cancel_talk.clone();
                    self.mode = Mode::Browse;
                }
                Input::NextTab => {}
            },
        }
        false
    }

    fn cursor_anchor(&self) -> Option<(i32, i32)> {
        let vis = self.cursor.checked_sub(self.top)?;
        (vis < self.geom().visible_rows() && self.cursor < self.rows().len())
            .then(|| self.geom().row_anchor(vis))
    }

    fn hit(&self, x: f32, y: f32) -> Option<Hit> {
        let page = match self.screen {
            Screen::Menu => {
                return (0..self.buttons.len())
                    .find(|&k| inside(lay::menu_button_rect(k), x, y))
                    .map(Hit::Menu);
            }
            Screen::Page(p) => p,
        };
        if let Mode::Haggle { .. } = self.mode {
            if let Some(t) = (0..TIERS).find(|&t| inside(lay::tier_rect(t), x, y)) {
                return Some(Hit::Tier(t));
            }
        }
        if page == Page::Equip && self.ask_equip {
            if inside(page::EQUIP_YES, x, y) {
                return Some(Hit::EquipYes);
            }
            if inside(page::EQUIP_NO, x, y) {
                return Some(Hit::EquipNo);
            }
        }
        if page == Page::Equip && self.wear_mode {
            for (c, card) in self.cards.iter().enumerate() {
                for k in 0..SMALL_SLOTS.len() {
                    let (ix, iy) = cmp::small_icon_pos(card.head, k);
                    if inside((ix, iy, ix + 18, iy + 18), x, y) {
                        return Some(Hit::Slot(c, k));
                    }
                }
            }
        }
        let no = if page == Page::Sell {
            page::SELL_NO
        } else {
            page::NO
        };
        for (r, h) in [
            (page::EXIT, Hit::Exit),
            (page::YES, Hit::Yes),
            (no, Hit::No),
        ] {
            if inside(r, x, y) {
                return Some(h);
            }
        }
        if page != Page::Sell && inside(page::DICKER, x, y) {
            return Some(Hit::Dicker);
        }
        if page == Page::Sell {
            if let Some(i) = (0..SELL_TAB_TEX.len()).find(|&i| inside(lay::sell_tab_rect(i), x, y))
            {
                return Some(Hit::Tab(i));
            }
        }
        if let Some(a) = self.cursor_anchor() {
            let (add, sub) = lay::spinner_rects(a);
            if inside(add, x, y) {
                return Some(Hit::Add);
            }
            if inside(sub, x, y) {
                return Some(Hit::Sub);
            }
        }
        if let Some(part) = self.geom().scroll.hit(x, y, self.top, self.scroll_range()) {
            return Some(Hit::Scroll(part));
        }
        self.geom()
            .row_at(x, y)
            .filter(|k| self.top + k < self.rows().len())
            .map(Hit::Row)
    }

    fn follow_hover(&mut self, hit: Hit) {
        match hit {
            Hit::Menu(k) => self.menu_cursor = k,
            Hit::Tier(t) => self.mode = Mode::Haggle { index: t },
            _ => {}
        }
    }

    /// 返回 true = 关店。
    fn click(&mut self, state: &mut SceState, hit: Hit) -> bool {
        // 菜单（`FUN_0045a600`）和子页上的按钮（`0x46b8dc` 起那一串）点下都放 wa001；
        // 列表行和滚动条原版这里不放。
        if matches!(
            hit,
            Hit::Menu(_)
                | Hit::Yes
                | Hit::No
                | Hit::EquipYes
                | Hit::EquipNo
                | Hit::Dicker
                | Hit::Exit
                | Hit::Tier(_)
                | Hit::Tab(_)
        ) {
            self.sfx = Some(ui_sound::CLICK);
        }
        match hit {
            Hit::Menu(k) => {
                if let Some(b) = self.buttons.get(k).copied() {
                    return self.activate(state, b);
                }
            }
            Hit::Row(k) => self.select(self.top + k),
            Hit::Yes => {
                if let Mode::Haggle { .. } = self.mode {
                    self.mode = Mode::Browse;
                }
                self.confirm(state);
            }
            Hit::Dicker => self.start_haggle(),
            Hit::No => {
                if let Mode::Haggle { .. } = self.mode {
                    self.talk = self.shop.normal_cancel_talk.clone();
                    self.mode = Mode::Browse;
                } else {
                    self.cart.clear();
                    self.reset_wear();
                }
            }
            Hit::EquipYes => self.equip_yes(state),
            Hit::EquipNo => {
                self.ask_equip = false;
                self.wear_mode = false;
            }
            Hit::Slot(c, k) => {
                if self.wear_mode {
                    if let Some(role) = self.cards.get(c).map(|card| card.role) {
                        self.wear(role, SMALL_SLOTS[k]);
                    }
                }
            }
            Hit::Exit => self.back_to_menu(),
            Hit::Tier(t) => self.haggle_at(state, t),
            Hit::Tab(i) => self.set_tab(i),
            Hit::Add => self.nudge(1),
            Hit::Sub => self.nudge(-1),
            Hit::Scroll(part) => {
                let page = self.geom().visible_rows() as isize;
                match part {
                    ScrollPart::Up => self.scroll_by(-1),
                    ScrollPart::Down => self.scroll_by(1),
                    ScrollPart::PageUp => self.scroll_by(-page),
                    ScrollPart::PageDown => self.scroll_by(page),
                    ScrollPart::Thumb => {
                        let sb = self.geom().scroll;
                        if let Some((_, ty)) = sb.thumb_pos(self.top, self.scroll_range()) {
                            self.drag = Some(self.last_mouse.1 - ty as f32);
                        }
                    }
                }
            }
        }
        false
    }

    fn finish(&mut self, state: &mut SceState) -> bool {
        if let Some(proc_id) = self.chat_proc.take() {
            state.call_proc(proc_id as u32);
        }
        true
    }
}

// ---------------------------------------------------------------- 画

/// 贴图按原生尺寸贴在 (x, y)。
pub(super) fn blit(
    list: &imgui::DrawListMut<'_>,
    tex: &Tex,
    x: i32,
    y: i32,
    at: &dyn Fn(i32, i32) -> [f32; 2],
) {
    if let Some(s) = tex.as_deref() {
        let r = (x, y, x + s.width() as i32, y + s.height() as i32);
        draw_sprite(list, Some(s), r, at);
    }
}

/// 同 [`blit`]，带透明度。
fn blit_alpha(
    list: &imgui::DrawListMut<'_>,
    tex: &Tex,
    x: i32,
    y: i32,
    alpha: f32,
    at: &dyn Fn(i32, i32) -> [f32; 2],
) {
    if let Some(s) = tex.as_deref() {
        let r = (x, y, x + s.width() as i32, y + s.height() as i32);
        let (uv0, uv1) = super::encamp::sprite_uv(s, r);
        list.add_image(s.imgui_texture_id(), at(r.0, r.1), at(r.2, r.3))
            .uv_min(uv0)
            .uv_max(uv1)
            .col([1., 1., 1., alpha])
            .build();
    }
}

/// 小图标闪的透明度（控件 0x56ae84 的更新 `0x444d30`）：`FUN_00444d10` 开闪后透明度从
/// 上限（控件原本的 255）每帧减 `+0xa8`（1.0）减到下限 `+0xa4`（6），再每帧加回上限，如此往复。
/// 原版按帧走、帧率不固定，这里按每秒 60 帧折成秒 [推断]。
fn flash_alpha(t: f32) -> f32 {
    const MIN: f32 = 6. / 255.;
    const RATE: f32 = 60. / 255.;
    let half = (1. - MIN) / RATE;
    let phase = t.max(0.) % (2. * half);
    if phase < half {
        1. - phase * RATE
    } else {
        MIN + (phase - half) * RATE
    }
}

/// 三态按钮：悬停或手柄选中用 1 号图，常态 0 号。
pub(super) fn button(
    list: &imgui::DrawListMut<'_>,
    tex: &[Tex; 3],
    r: Rect,
    lit: bool,
    at: &dyn Fn(i32, i32) -> [f32; 2],
) {
    blit(list, &tex[usize::from(lit)], r.0, r.1, at);
}

/// 横向三段条：贴图切成宽 `cap` 的三列（`FUN_0043dab0`），竖直方向取贴图
/// 底下那 `h` 行（和 `sprite_uv` 同一个约定）。
pub(super) fn bar(
    list: &imgui::DrawListMut<'_>,
    tex: &Tex,
    r: Rect,
    cap: i32,
    at: &dyn Fn(i32, i32) -> [f32; 2],
) {
    let Some(s) = tex.as_deref() else { return };
    let (tw, th) = (s.width().max(1) as f32, s.height().max(1) as f32);
    let v0 = (1. - (r.3 - r.1) as f32 / th).max(0.);
    for (piece, col) in lay::three_slice(r, cap) {
        let u0 = (col as i32 * cap) as f32 / tw;
        let u1 = (((col as i32 + 1) * cap) as f32 / tw).min(1.);
        list.add_image(
            s.imgui_texture_id(),
            at(piece.0, piece.1),
            at(piece.2, piece.3),
        )
        .uv_min([u0, v0])
        .uv_max([u1, 1.])
        .build();
    }
}

/// 文本控件的第一行：rect 左上各缩 8。
pub(super) fn label(
    list: &imgui::DrawListMut<'_>,
    r: Rect,
    text: &str,
    at: &dyn Fn(i32, i32) -> [f32; 2],
) {
    list.add_text(at(r.0 + lay::TEXT_INSET, r.1 + lay::TEXT_INSET), INK, text);
}

/// 多行：到 rect 右边 − 8 折行，行距 16。
pub(super) fn paragraph(
    list: &imgui::DrawListMut<'_>,
    r: Rect,
    text: &str,
    at: &dyn Fn(i32, i32) -> [f32; 2],
) {
    let width = r.2 - r.0 - 2 * lay::TEXT_INSET;
    for (k, line) in lay::wrap(text, width).iter().enumerate() {
        let y = r.1 + lay::TEXT_INSET + lay::GLYPH * k as i32;
        if y + lay::GLYPH > r.3 {
            break;
        }
        list.add_text(at(r.0 + lay::TEXT_INSET, y), INK, line);
    }
}

pub(super) fn text_width(text: &str) -> i32 {
    text.chars()
        .map(|c| {
            if c.is_ascii() {
                lay::GLYPH / 2
            } else {
                lay::GLYPH
            }
        })
        .sum()
}

impl SceCommandDealerMenu {
    fn draw(&self, ui: &Ui, money: i32, hover: Option<Hit>) {
        overlay(ui, "Dealer", |ui, at, scale| {
            let list = ui.get_window_draw_list();
            let _font = game_font(GameFontSize::SMALL).map(|f| ui.push_font(f));
            match self.screen {
                Screen::Menu => self.draw_menu(&list, at, hover),
                Screen::Page(p) => self.draw_page(&list, at, scale, p, money, hover),
            }
        });
    }

    fn draw_menu(
        &self,
        list: &imgui::DrawListMut<'_>,
        at: &dyn Fn(i32, i32) -> [f32; 2],
        hover: Option<Hit>,
    ) {
        let n = self.buttons.len();
        let sk = &self.skin;
        let t = lay::flag_top_rect();
        blit(list, &sk.flag_t, t.0, t.1, at);
        let b = lay::flag_bottom_rect(n);
        blit(list, &sk.flag_b, b.0, b.1, at);
        for k in 0..n {
            let m = lay::flag_mid_rect(k);
            blit(list, &sk.flag_m, m.0, m.1, at);
        }
        for (k, btn) in self.buttons.iter().enumerate() {
            let r = lay::menu_button_rect(k);
            let lit = k == self.menu_cursor || hover == Some(Hit::Menu(k));
            button(list, &sk.rawbtn, r, lit, at);
            // 字居中 [推断]：按钮控件的对齐方式没有逐字段读。
            let s = self.text(btn.lang_slot(), btn.fallback_label());
            let x = r.0 + (r.2 - r.0 - text_width(&s)) / 2;
            let y = r.1 + (r.3 - r.1 - lay::GLYPH) / 2;
            list.add_text(at(x, y), INK, &s);
        }
    }

    fn draw_page(
        &self,
        list: &imgui::DrawListMut<'_>,
        at: &dyn Fn(i32, i32) -> [f32; 2],
        scale: f32,
        page: Page,
        money: i32,
        hover: Option<Hit>,
    ) {
        let sk = &self.skin;
        let sell = page == Page::Sell;
        let geom = self.geom();

        blit(list, &sk.frame, page::FRAME.0, page::FRAME.1, at);
        let flag = if sell { &sk.sale_flag } else { &sk.buy_flag };
        blit(list, flag, page::FLAG.0, page::FLAG.1, at);
        bar(
            list,
            &sk.frame_bar,
            page::FRAME_BAR,
            page::FRAME_BAR_CAP,
            at,
        );
        blit(
            list,
            &sk.list_frame,
            page::LIST_FRAME.0,
            page::LIST_FRAME.1,
            at,
        );
        blit(list, &sk.info, page::INFO.0, page::INFO.1, at);
        if let Some((_, Some(p))) = &self.preview {
            p.draw(list, at);
        }
        match page {
            Page::Equip => blit(list, &sk.equip_word, page::WORD.0, page::WORD.1, at),
            Page::Prop => blit(list, &sk.prop_word, page::WORD.0, page::WORD.1, at),
            Page::Sell => {}
        }
        if page == Page::Equip {
            self.draw_cards(list, at);
        }

        // 卖页的页签条。
        if sell {
            bar(
                list,
                &sk.tab_bar,
                lay::SELL_TAB_BAR,
                lay::SELL_TAB_BAR_CAP,
                at,
            );
            for i in 0..SELL_TAB_TEX.len() {
                let r = lay::sell_tab_rect(i);
                let state = if i == self.sell_tab {
                    2
                } else {
                    usize::from(hover == Some(Hit::Tab(i)))
                };
                blit(list, &sk.tabs[i][state], r.0, r.1, at);
            }
        }

        // 表头。
        let (name, price, count) = (
            self.text(lang::NAME, "名称"),
            self.text(lang::PRICE, "价格"),
            self.text(lang::COUNT, "数量"),
        );
        let header = if sell {
            let held = self.text(lang::HELD, "持有");
            format!("{name}            {price}   {count} {held}")
        } else {
            format!("{name}                {price}   {count}")
        };
        label(list, geom.header, &header, at);

        // 网格、选中条、各行。
        let grid = if sell { &sk.sale_grid } else { &sk.buy_grid };
        let rows = self.rows();
        let row_dy = (lay::ROW_H - lay::GLYPH) as f32 * 0.5 * scale;
        for k in 0..geom.grid_rows {
            let g = geom.grid_rect(k);
            blit(list, grid, g.0, g.1, at);
            let i = self.top + k;
            let Some(good) = rows.get(i) else { continue };
            if i == self.cursor {
                blit(list, &sk.selected, g.0, g.1, at);
            }
            // 选中条是深绿色不透明的，上面的字换成浅色 [推断]：列表控件画行的那段
            // 没追到（`0x43ff70` 画的是滚动条），颜色取对话选择框的 0xfffafa。
            let ink = if i == self.cursor { SELECTED_INK } else { INK };
            let cell = |c: usize, s: &str| {
                let [x, y] = at(geom.column_x(c) + lay::TEXT_INSET, g.1);
                list.add_text([x, y + row_dy], ink, s);
            };
            cell(0, &good.name);
            let unit = if sell {
                sell_price(good.price)
            } else {
                good.price
            };
            cell(1, &unit.to_string());
            let n = self.in_cart(good.id);
            if n > 0 {
                cell(2, &n.to_string());
            }
            if sell {
                cell(3, &good.owned.to_string());
            }
        }

        // 滚动条：上下钮，能滚时再画滑块（槽那张图原版不画）。
        let sb = geom.scroll;
        let state = |part: ScrollPart, dragging: bool| -> usize {
            if dragging {
                2
            } else if hover == Some(Hit::Scroll(part)) {
                if self.mouse_down { 2 } else { 1 }
            } else {
                0
            }
        };
        blit(
            list,
            &sk.scroll[4 + state(ScrollPart::Up, false)],
            sb.up.0,
            sb.up.1,
            at,
        );
        blit(
            list,
            &sk.scroll[7 + state(ScrollPart::Down, false)],
            sb.down.0,
            sb.down.1,
            at,
        );
        if let Some((tx, ty)) = sb.thumb_pos(self.top, self.scroll_range()) {
            let k = state(ScrollPart::Thumb, self.drag.is_some());
            blit(list, &sk.scroll[k], tx, ty, at);
        }

        // 选中行上的「持有:N」和加减钮。
        if let (Some(a), Some(good)) = (self.cursor_anchor(), self.current()) {
            if !sell {
                let held = self.text(lang::HELD, "持有");
                // 这行字压在选中条上，和选中行一样用浅色 [推断]。
                let r = lay::owned_tip_rect(a);
                list.add_text(
                    at(r.0 + lay::TEXT_INSET, r.1 + lay::TEXT_INSET),
                    SELECTED_INK,
                    format!("{held}:{}", good.owned),
                );
            }
            let n = self.in_cart(good.id);
            let (add, sub) = lay::spinner_rects(a);
            if n < self.cap(good) {
                button(list, &sk.add, add, hover == Some(Hit::Add), at);
            } else {
                blit(list, &sk.unadd, add.0, add.1, at);
            }
            if n > 0 {
                button(list, &sk.sub, sub, hover == Some(Hit::Sub), at);
            } else {
                blit(list, &sk.unsub, sub.0, sub.1, at);
            }

            // 说明面板：`$` 前是描述，后面是属性行。
            let (desc, stat) = good
                .description
                .split_once('$')
                .unwrap_or((good.description.as_str(), ""));
            paragraph(list, page::DESC, desc, at);
            let stat = stat.trim_end_matches(['\n', '\r', ' ', '\u{3000}']);
            if !stat.is_empty() {
                label(list, page::STAT, stat, at);
            }
        }

        let held = self.text(lang::HELD, "持有");
        label(list, page::MONEY, &format!("{held} {money}"), at);
        let total = self.text(lang::TOTAL, "累计");
        label(list, page::TOTAL, &format!("{total} {}", self.total()), at);

        match self.mode {
            Mode::Browse if self.ask_equip => {
                button(
                    list,
                    &sk.equip_yes,
                    page::EQUIP_YES,
                    hover == Some(Hit::EquipYes),
                    at,
                );
                button(
                    list,
                    &sk.equip_no,
                    page::EQUIP_NO,
                    hover == Some(Hit::EquipNo),
                    at,
                );
            }
            Mode::Browse => paragraph(list, page::TALK, &self.talk, at),
            Mode::Haggle { index } => {
                for t in 0..TIERS {
                    let r = lay::tier_rect(t);
                    let lit = t == index || hover == Some(Hit::Tier(t));
                    button(list, &sk.tiers[t], r, lit, at);
                }
            }
        }

        button(list, &sk.yes, page::YES, hover == Some(Hit::Yes), at);
        if sell {
            button(list, &sk.no, page::SELL_NO, hover == Some(Hit::No), at);
        } else {
            button(list, &sk.no, page::NO, hover == Some(Hit::No), at);
            if self.shop.haggling {
                button(
                    list,
                    &sk.dicker,
                    page::DICKER,
                    hover == Some(Hit::Dicker),
                    at,
                );
            } else {
                blit(list, &sk.undicker, page::DICKER.0, page::DICKER.1, at);
            }
        }
        button(list, &sk.exit, page::EXIT, hover == Some(Hit::Exit), at);
    }
}

impl SceCommandDealerMenu {
    /// 对比栏：队里的人按角色号排，最多四个；选中的是这件装备、这个人能穿，
    /// 就拿它和身上那一格比（`FUN_004623a0`）。
    fn build_cards(&mut self, state: &SceState) {
        self.cards.clear();
        if self.page() != Some(Page::Equip) {
            return;
        }
        let Some(db) = state.global_state().game_db().cloned() else {
            return;
        };
        let good = self.current().cloned();
        let p = state.global_state().persistent_state();
        for (role, &id) in PLAYER_ACTOR_IDS.iter().enumerate() {
            if !p.is_in_team(id) || self.cards.len() == cmp::MAX_HEADS {
                continue;
            }
            let mut groups = vec![];
            if let Some(g) = good.as_ref().filter(|g| g.applicable[role]) {
                for &(slot, side, big) in compare_groups(g.item_type) {
                    let worn = p
                        .equipped(id, slot)
                        .and_then(|e| db.item(e))
                        .map(stats_of)
                        .unwrap_or_default();
                    let delta = std::array::from_fn(|k| g.stats[k] - worn[k]);
                    groups.push((side, big, cmp::arrows(delta)));
                }
            }
            self.cards.push(Card {
                role,
                head: cmp::head_rect(self.cards.len()),
                small: SMALL_SLOTS.map(|s| {
                    p.equipped(id, s).is_some()
                        || self.worn.iter().any(|w| w.role == role && w.slot == s)
                }),
                groups,
            });
        }
    }

    /// 选中行换了就换预览；三个子页都有这个视图（`FUN_0045b990` / `FUN_0046df40` /
    /// `FUN_00471590` 都引用 `Item\%s\%s.pol`）。
    fn update_preview(&mut self, state: &SceState) {
        let want = self.page().and_then(|_| self.current()).map(|g| g.id);
        if self.preview.as_ref().map(|(id, _)| *id) != want {
            self.preview = want.map(|id| {
                let camera = self
                    .current()
                    .map(|g| lay::preview::camera(g.id, g.item_type, g.weapon_type))
                    .unwrap_or(lay::preview::camera(id, 0, 0));
                (id, ItemPreview::build(state, id, camera, lay::preview::BOX))
            });
        }
        if let Some((_, Some(p))) = &self.preview {
            p.render(self.frame_dt);
        }
    }

    fn draw_cards(&self, list: &imgui::DrawListMut<'_>, at: &dyn Fn(i32, i32) -> [f32; 2]) {
        let sk = &self.skin;
        let lift = cmp::bob(self.clock);
        // 「选谁穿」时，能穿的人对应那一格的小图标闪（`FUN_00462280` → `FUN_00444d10`）。
        let flashing = self
            .current()
            .filter(|_| self.wear_mode)
            .map(|g| (g.applicable, equipment_slots_for_type(g.item_type)));
        let alpha = flash_alpha(self.wear_clock);
        for c in &self.cards {
            blit(list, &sk.heads[c.role], c.head.0, c.head.1, at);
            for (k, lit) in c.small.iter().enumerate() {
                let (x, y) = cmp::small_icon_pos(c.head, k);
                let flash = flashing
                    .as_ref()
                    .is_some_and(|(who, slots)| who[c.role] && slots.contains(&SMALL_SLOTS[k]));
                if flash {
                    blit_alpha(list, &sk.small[k], x, y, alpha, at);
                } else if *lit {
                    blit(list, &sk.small[k], x, y, at);
                }
            }
            for (side, big, arrows) in &c.groups {
                let (x, y) = cmp::big_icon_pos(c.head, *side);
                blit(list, &sk.big[*big], x, y, at);
                for a in arrows {
                    let (x, y) = cmp::arrow_pos(c.head, *side, a.cell);
                    let tex = if a.up { &sk.up } else { &sk.down };
                    blit(list, &tex[a.stat], x, y - lift, at);
                }
            }
        }
    }
}

impl SceCommand for SceCommandDealerMenu {
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
        self.clock += delta_sec;
        self.frame_dt = delta_sec;
        if self.wear_mode {
            self.wear_clock += delta_sec;
        }
        if !self.loaded {
            self.load(state);
            if self.shop.merchant_name.is_empty() && self.buttons.is_empty() {
                log::warn!("DealerMenu: {} 读不出来，直接跳过", self.shop_file);
                return true;
            }
        }

        // 快进（脚本跳过）时不开店。
        if state.fast_forward() {
            return true;
        }

        self.frame(ui, state)
    }
}

/// 当铺借用的两页：原版当铺的模式对象下挂的就是买卖这两个类（`FUN_005232b0`：
/// `FUN_0046d140` 买道具页、`FUN_00470a60` 卖页），见 `switch/re/notes/hockshop.md`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HostedPage {
    Buy,
    Sell,
}

impl SceCommandDealerMenu {
    /// 直接开在买道具页或卖页上；`goods` 是（物品 id，单价）。不还价、商人不说话。
    pub(super) fn hosted(state: &SceState, page: HostedPage, goods: &[(u32, i32)]) -> Self {
        let mut me = Self::new(String::new());
        me.loaded = true;
        me.skin = Skin::load(state);
        me.lang = read_gbk(state.asset_mgr().vfs(), &PathBuf::from(LANG_PATH))
            .map(|t| assis_lang::parse_section(&t, LANG_SECTION))
            .unwrap_or_default();
        let owned: Vec<u32> = {
            let p = state.global_state().persistent_state();
            goods.iter().map(|(id, _)| p.item_count(*id)).collect()
        };
        me.prop_goods = goods
            .iter()
            .zip(owned)
            .filter_map(|((id, price), n)| {
                let mut g = Self::good(state, *id, n)?;
                g.price = *price;
                Some(g)
            })
            .collect();
        me.rebuild_stock(state);
        me.single_each = page == HostedPage::Buy;
        me.enter(match page {
            HostedPage::Buy => Page::Prop,
            HostedPage::Sell => Page::Sell,
        });
        me.talk.clear();
        me
    }

    /// 画一帧、处理输入；返回 true = 玩家离开了这一页（原版回当铺主页）。
    pub(super) fn hosted_frame(&mut self, ui: &Ui, state: &mut SceState) -> bool {
        self.frame(ui, state);
        self.screen == Screen::Menu
    }

    /// 一帧：鼠标、画、点击、按键。返回 true = 关店。
    fn frame(&mut self, ui: &Ui, state: &mut SceState) -> bool {
        let (mx, my) = virtual_mouse(ui);
        self.mouse_down = ui.is_mouse_down(MouseButton::Left);
        if let Some(grab) = self.drag {
            if self.mouse_down {
                let sb = self.geom().scroll;
                self.top = sb.pos_at(my - grab, self.scroll_range());
            } else {
                self.drag = None;
            }
        }
        let hover = self.hit(mx, my);
        if (mx, my) != self.last_mouse {
            self.last_mouse = (mx, my);
            if let Some(h) = hover {
                self.follow_hover(h);
            }
        }

        self.build_cards(state);
        self.update_preview(state);
        let money = state.global_state().persistent_state().money();
        self.draw(ui, money, hover);

        let mut done = false;
        if ui.is_mouse_clicked(MouseButton::Left) {
            if let Some(h) = hover {
                done = self.click(state, h);
            }
        }
        if !done {
            if let Some(input) = Self::poll_input(state) {
                done = self.handle(state, input);
            }
        }
        // 先放音效再关店，离开钮那一下才听得见。
        if let Some(name) = self.sfx.take() {
            ui_sound::play(state, name);
        }
        if done {
            return self.finish(state);
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

    fn weapon(id: u32, price: i32) -> Good {
        Good {
            id,
            name: String::new(),
            description: String::new(),
            item_type: 11,
            weapon_type: 2,
            price,
            owned: 0,
            stats: [0; cmp::STATS],
            applicable: [true, true, false, false, false],
        }
    }

    #[test]
    fn a_piece_worn_at_the_counter_leaves_the_cart_but_not_the_bill() {
        let mut m = SceCommandDealerMenu::new(String::new());
        m.screen = Screen::Page(Page::Equip);
        m.equip_goods = vec![weapon(10, 100), weapon(11, 30)];
        m.cart.insert(10, 2);
        m.prompt_equip();
        assert!(m.ask_equip, "单子里有、还没人穿：问穿不穿");

        m.wear(0, SLOT_WEAPON);
        assert_eq!(m.in_cart(10), 1);
        assert_eq!(m.worn.len(), 1);
        assert_eq!(m.total(), 200, "穿上的那件照样算钱");
        m.prompt_equip();
        assert!(!m.ask_equip, "有人穿了就不再问");

        // 同一格再穿同一件：不动；格子不对、这人不能穿：不动。
        m.wear(0, SLOT_WEAPON);
        m.wear(0, SLOT_HAT);
        m.wear(3, SLOT_WEAPON);
        assert_eq!((m.in_cart(10), m.worn.len()), (1, 1));

        // 那一格改穿别的：先前那件回到单子里。
        m.cart.insert(11, 1);
        m.cursor = 1;
        m.wear(0, SLOT_WEAPON);
        assert_eq!((m.in_cart(10), m.in_cart(11)), (2, 0));
        assert_eq!(
            m.worn,
            vec![Worn {
                id: 11,
                item_type: 11,
                role: 0,
                slot: SLOT_WEAPON
            }]
        );

        // 取消：一起撤。
        m.cart.clear();
        m.reset_wear();
        assert!(m.worn.is_empty() && !m.wear_mode);
    }

    #[test]
    fn wearable_icons_pulse_down_then_back_up() {
        assert!((flash_alpha(0.) - 1.).abs() < 1e-6, "从最亮开始往下走");
        assert!(flash_alpha(1.) < flash_alpha(0.5));
        let half = (1. - 6. / 255.) / (60. / 255.);
        assert!((flash_alpha(half) - 6. / 255.).abs() < 1e-3, "到下限");
        assert!(flash_alpha(half + 1.) > flash_alpha(half));
        assert!((flash_alpha(2. * half) - 1.).abs() < 1e-3, "一整轮回到最亮");
    }

    #[test]
    fn a_script_path_becomes_a_vfs_path() {
        assert_eq!(
            SceCommandDealerMenu::vfs_path("DealScript\\anyShop\\q01any.txt"),
            PathBuf::from("/basedata/basedata/DealScript/anyShop/q01any.txt")
        );
    }
}
