//! 买卖界面（SCE 100）的像素布局，800×600 原生坐标。
//!
//! 逆向记录在 `switch/re/notes/dealer_layout.md`。贴图一律按原生尺寸贴在
//! rect 左上角（`state_page_layout.md` §0.1），所以这里多数只给放置点。

pub type Rect = (i32, i32, i32, i32);

/// 文本控件的内边距（`+0x44` = 8）和字格（16px 档）。
pub const TEXT_INSET: i32 = 8;
pub const GLYPH: i32 = 16;

/// 列表、表头、说明文字的颜色 RGB(0x0c,0x1f,0x00)（`ColorQuadFmt` 两种写法
/// 都落到这个色）；按钮上的字也是这个色（`FUN_004593c0` 里 `0xff0c1f00`）。
pub const INK: [f32; 4] = [0x0c as f32 / 255., 0x1f as f32 / 255., 0., 1.];

// ---------------------------------------------------------------- 前台菜单

/// 菜单锚点：`FUN_004590d0` 写 `+0x88 = 360`、`+0x8c = 136`，`+0x80/+0x84`
/// = 行宽 90、行高 24。
const MENU_ANCHOR_X: i32 = 360;
const MENU_ANCHOR_Y: i32 = 136;
const MENU_ROW_W: i32 = 90;
pub const MENU_ROW_H: i32 = 24;

/// 前台菜单的五个按钮。按钮 id 4..8，字是 AssisLang 的槽号。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuButton {
    BuyEquip,
    BuyProp,
    Sell,
    Chat,
    Leave,
}

impl MenuButton {
    /// `FUN_004593c0` 里 `FUN_0043afa0(&Lang[n])` 的 n。
    pub fn lang_slot(self) -> usize {
        match self {
            MenuButton::BuyEquip => 262,
            MenuButton::BuyProp => 263,
            MenuButton::Sell => 264,
            MenuButton::Chat => 163,
            MenuButton::Leave => 166,
        }
    }

    pub fn fallback_label(self) -> &'static str {
        match self {
            MenuButton::BuyEquip => "买装备",
            MenuButton::BuyProp => "买道具",
            MenuButton::Sell => "卖东西",
            MenuButton::Chat => "聊 天",
            MenuButton::Leave => "退 出",
        }
    }
}

/// 按店铺类型摆哪几个按钮（`FUN_00459970`，跳转表 0x459f1c =
/// `[0x4599c9, 0x459ce3, 0x459b56, 0x459ce3]`）。类型越界时原版直接返回、
/// 一个按钮都不摆；这里留一个「退出」，免得卡死。
pub fn menu_buttons(shop_type: u8) -> Vec<MenuButton> {
    use MenuButton::*;
    match shop_type {
        0 => vec![BuyProp, Sell, Chat, Leave],
        2 => vec![BuyEquip, Sell, Chat, Leave],
        1 | 3 => vec![BuyEquip, BuyProp, Sell, Chat, Leave],
        _ => vec![Leave],
    }
}

/// 第 k 个按钮：左 = 锚点 x − 10，顶 = 锚点 y + 32，逐行下移 24。
pub fn menu_button_rect(k: usize) -> Rect {
    let x = MENU_ANCHOR_X - 10;
    let y = MENU_ANCHOR_Y + 32 + MENU_ROW_H * k as i32;
    (x, y, x + MENU_ROW_W, y + MENU_ROW_H)
}

/// 按钮后面那一段竖幡（`FlagM`）：按钮 rect 四边各缩 1、宽 86。
pub fn flag_mid_rect(k: usize) -> Rect {
    let b = menu_button_rect(k);
    (b.0 + 1, b.1 + 1, b.0 + 1 + 0x56, b.3 - 1)
}

/// 幡头 `FlagT`：`(+0x88, +0x8c, +0x88 + 0x44, +0x8c + 0x20)`。
pub fn flag_top_rect() -> Rect {
    (
        MENU_ANCHOR_X,
        MENU_ANCHOR_Y,
        MENU_ANCHOR_X + 0x44,
        MENU_ANCHOR_Y + 0x20,
    )
}

/// 幡尾 `FlagB`：接在最后一个按钮底下，高 11（`0x459eeb` 起）。
pub fn flag_bottom_rect(buttons: usize) -> Rect {
    let top = menu_button_rect(buttons.max(1) - 1).3;
    (MENU_ANCHOR_X, top, MENU_ANCHOR_X + 0x44, top + 0xb)
}

// ---------------------------------------------------------------- 三个子页

/// 页面共用的几块（买装备 `FUN_0045b990`、买道具 `FUN_0046df40`、卖
/// `FUN_00471590` 三处立即数相同）。
pub mod page {
    use super::Rect;

    /// `BuyFlag.tga` / `SaleFlag.tga` 的 rect。
    pub const FLAG: Rect = (266, 22, 482, 150);
    /// 底栏 `UI\Flex\framebar.tga`，横向三段、端头宽 30、高 35。
    pub const FRAME_BAR: Rect = (0, 564, 800, 599);
    pub const FRAME_BAR_CAP: i32 = 30;
    /// 左边的 `Assistant\Communal\Frame.tga`。
    pub const FRAME: Rect = (0, 0, 206, 600);
    /// `GameMainUI\Equip\ListFrame.tga`。
    pub const LIST_FRAME: Rect = (454, 32, 790, 425);
    /// `GameMainUI\Communal\Info.tga` 说明面板。
    pub const INFO: Rect = (218, 142, 450, 556);
    /// 面板上的竖字 `Equip_word` / `Prop_word`。
    pub const WORD: Rect = (404, 158, 420, 190);
    /// 说明文字，和道具页一样多行自动换行；`$` 后面那一段是属性行。
    pub const DESC: Rect = (232, 388, 444, 556);
    pub const STAT: Rect = (232, 520, 444, 552);
    /// 「持有 N」（身上的钱）。
    pub const MONEY: Rect = (478, 392, 624, 424);
    /// 「累计 N」（这一单的钱）。
    pub const TOTAL: Rect = (638, 392, 764, 424);
    /// 商人的话（买装备 `+0x17604` id 0x109，买道具 `+0x56cc`，卖 `+0x19894`）。
    pub const TALK: Rect = (489, 447, 745, 547);
    /// 底栏上的三个按钮，贴图 89×25。
    pub const YES: Rect = (300, 569, 390, 594);
    pub const DICKER: Rect = (456, 569, 546, 594);
    pub const NO: Rect = (612, 569, 702, 594);
    /// 卖页没有还价钮，「取消」挪到 (522,569)。
    pub const SELL_NO: Rect = (522, 569, 612, 594);
    /// 右下角 `StpCtl\Exit`：`(页宽 − 38, 页高 − 50, 页宽, 页高 − 2)`。
    pub const EXIT: Rect = (762, 550, 800, 598);
}

/// 一张列表的几何。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListGeom {
    /// 列表控件的 rect；右边 38 像素是滚动条列。
    pub rect: Rect,
    /// 表头文本控件。
    pub header: Rect,
    /// 装饰网格一共几行。
    pub grid_rows: usize,
    /// 各列宽（`FUN_0043e560`）。
    pub columns: &'static [i32],
}

/// 行高（`FUN_0043f640(0x17)`）。
pub const ROW_H: i32 = 23;
/// 网格贴图（`Buy_Grid` / `Sale_Grid`）256×24。
pub const GRID_W: i32 = 256;

/// 两个买页：`(476,79)` 起，高 306，13 行网格；列宽 Name 168 / Price 51 /
/// Num1 38。
pub const BUY_LIST: ListGeom = ListGeom {
    rect: (476, 79, 769, 385),
    header: (476, 50, 782, 82),
    grid_rows: 13,
    columns: &[168, 51, 38],
};

/// 卖页：`(472,106)` 起，高 278，12 行；列宽 131 / 52 / 36 / 36。
pub const SELL_LIST: ListGeom = ListGeom {
    rect: (472, 106, 770, 384),
    header: (466, 82, 774, 114),
    grid_rows: 12,
    columns: &[131, 52, 36, 36],
};

impl ListGeom {
    /// 能同时看见几行。
    pub fn visible_rows(&self) -> usize {
        self.grid_rows
    }

    /// 第 k 格网格：`(x, y1 + 2 + 23k)`。
    pub fn grid_rect(&self, k: usize) -> Rect {
        let y = self.rect.1 + 2 + ROW_H * k as i32;
        (self.rect.0, y, self.rect.0 + GRID_W, y + ROW_H)
    }

    /// 选中行的锚点（`FUN_0043ebb0`）：`(x1, y1 + (行 − 顶行) × 23)`。
    pub fn row_anchor(&self, visible_index: usize) -> (i32, i32) {
        (self.rect.0, self.rect.1 + ROW_H * visible_index as i32)
    }

    /// 第 c 列的左边。
    pub fn column_x(&self, c: usize) -> i32 {
        self.rect.0 + self.columns.iter().take(c).sum::<i32>()
    }

    /// 鼠标在第几行可见行上；只认网格那 256 宽。
    pub fn row_at(&self, x: f32, y: f32) -> Option<usize> {
        (0..self.grid_rows).find(|&k| inside(self.grid_rect(k), x, y))
    }
}

/// 卖页顶上六个页签（`0x4722d6` 起）：`(472 + 53i, 30, 514 + 53i, 80)`；
/// 贴图 42×53。
pub fn sell_tab_rect(i: usize) -> Rect {
    let x = 472 + 53 * i as i32;
    (x, 30, x + 42, 80)
}

/// 页签后面那条 `UI\Flex\Bar.tga`（`FUN_0043dc40(0x20, 0x20)`）。
pub const SELL_TAB_BAR: Rect = (461, 32, 781, 64);
pub const SELL_TAB_BAR_CAP: i32 = 32;

/// 选中行右边那个「持有:N」提示（`+0x19aec`）：行锚点右移 90，90×32。
pub fn owned_tip_rect(anchor: (i32, i32)) -> Rect {
    (anchor.0 + 90, anchor.1, anchor.0 + 180, anchor.1 + 32)
}

/// 选中行上的加减钮（`0x46c693` 起，默认排法 `+0x18b24 = 1`）：
/// 「加」在行锚点 +200，「减」再隔 20，都比行顶高 4，24×40。
pub fn spinner_rects(anchor: (i32, i32)) -> (Rect, Rect) {
    let (x, y) = (anchor.0 + 200, anchor.1 - 4);
    let add = (x, y, x + 24, y + 40);
    let sub = (add.2 + 20, y, add.2 + 44, y + 40);
    (add, sub)
}

/// 五个还价档（`bDK5..9`，84×50）：前三个一行，后两个下一行
/// （`0x45e6f0` 起）。
pub fn tier_rect(tier: usize) -> Rect {
    let (col, row) = if tier < 3 { (tier, 0) } else { (tier - 3, 1) };
    let x = 492 + 84 * col as i32;
    let y = if row == 0 { 439 } else { 491 };
    (x, y, x + 84, y + 50)
}

/// 横向三段条（`FUN_0043d940`，`+0x50 == 0` 那一支）：左端头、中段按端头
/// 宽度平铺、右端头；贴图也按端头宽切三列。返回 (目标 rect, 贴图第几列)。
pub fn three_slice(r: Rect, cap: i32) -> Vec<(Rect, usize)> {
    let mut out = vec![((r.0, r.1, r.0 + cap, r.3), 0)];
    let mut x = r.0 + cap;
    while x < r.2 - cap {
        out.push(((x, r.1, x + cap, r.3), 1));
        x += cap;
    }
    out.push(((r.2 - cap, r.1, r.2, r.3), 2));
    out
}

pub fn inside(r: Rect, x: f32, y: f32) -> bool {
    x >= r.0 as f32 && x < r.2 as f32 && y >= r.1 as f32 && y < r.3 as f32
}

/// 原版文本控件的折行：ASCII 半格、汉字一格，`\n` 和 `$` 换行。
pub fn wrap(text: &str, width_px: i32) -> Vec<String> {
    let mut lines = vec![];
    let mut line = String::new();
    let mut w = 0;
    for ch in text.chars() {
        if ch == '\n' || ch == '$' {
            lines.push(std::mem::take(&mut line));
            w = 0;
            continue;
        }
        let adv = if ch.is_ascii() { GLYPH / 2 } else { GLYPH };
        if w + adv > width_px && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            w = 0;
        }
        line.push(ch);
        w += adv;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_hangs_from_the_flag() {
        assert_eq!(flag_top_rect(), (360, 136, 428, 168));
        assert_eq!(menu_button_rect(0), (350, 168, 440, 192));
        assert_eq!(menu_button_rect(4), (350, 264, 440, 288));
        assert_eq!(flag_mid_rect(0), (351, 169, 437, 191));
        // 幡尾接在最后一个按钮底边。
        assert_eq!(flag_bottom_rect(4), (360, 264, 428, 275));
        assert_eq!(flag_bottom_rect(5), (360, 288, 428, 299));
    }

    #[test]
    fn the_shop_type_picks_the_buttons() {
        use MenuButton::*;
        assert_eq!(menu_buttons(0), [BuyProp, Sell, Chat, Leave]);
        assert_eq!(menu_buttons(1), [BuyEquip, BuyProp, Sell, Chat, Leave]);
        assert_eq!(menu_buttons(2), [BuyEquip, Sell, Chat, Leave]);
        assert_eq!(menu_buttons(3), menu_buttons(1));
        assert_eq!(menu_buttons(9), [Leave]);
    }

    #[test]
    fn rows_grids_and_the_spinner() {
        assert_eq!(BUY_LIST.grid_rect(0), (476, 81, 732, 104));
        assert_eq!(BUY_LIST.grid_rect(12), (476, 357, 732, 380));
        assert_eq!(SELL_LIST.grid_rect(11), (472, 361, 728, 384));
        assert_eq!(BUY_LIST.column_x(1), 644);
        assert_eq!(BUY_LIST.column_x(2), 695);
        let a = BUY_LIST.row_anchor(2);
        assert_eq!(a, (476, 125));
        assert_eq!(owned_tip_rect(a), (566, 125, 656, 157));
        assert_eq!(
            spinner_rects(a),
            ((676, 121, 700, 161), (720, 121, 744, 161))
        );
        assert_eq!(BUY_LIST.row_at(480., 82.), Some(0));
        assert_eq!(BUY_LIST.row_at(740., 82.), None, "滚动条列不算");
    }

    #[test]
    fn tier_buttons_sit_in_two_rows() {
        assert_eq!(tier_rect(0), (492, 439, 576, 489));
        assert_eq!(tier_rect(2), (660, 439, 744, 489));
        assert_eq!(tier_rect(3), (492, 491, 576, 541));
        assert_eq!(tier_rect(4), (576, 491, 660, 541));
        assert_eq!(sell_tab_rect(5), (737, 30, 779, 80));
    }

    #[test]
    fn a_bar_is_two_caps_and_tiled_middles() {
        let pieces = three_slice(page::FRAME_BAR, page::FRAME_BAR_CAP);
        assert_eq!(pieces.first(), Some(&((0, 564, 30, 599), 0)));
        assert_eq!(pieces.last(), Some(&((770, 564, 800, 599), 2)));
        // 中段从 30 起每 30 一块，最后一块盖到右端头下面。
        assert!(
            pieces[1..pieces.len() - 1]
                .iter()
                .all(|(r, c)| *c == 1 && r.2 - r.0 == 30)
        );
        assert_eq!(pieces.len(), 2 + 25);
    }

    #[test]
    fn text_wraps_like_the_widget() {
        assert_eq!(wrap("甲乙丙", 32), ["甲乙", "丙"]);
        assert_eq!(wrap("ab$cd", 100), ["ab", "cd"]);
    }
}
