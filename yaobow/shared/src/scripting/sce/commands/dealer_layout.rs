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
    /// 买装备页的「装备 / 不装备」（`bEquipYes` 0x129 / `bEquipNo` 0x130，84×50），和商人的话
    /// 那块重叠：亮出来时商人的话清空（`0x46bf0f` 起那段）。
    pub const EQUIP_YES: Rect = (511, 492, 595, 542);
    pub const EQUIP_NO: Rect = (635, 492, 719, 542);
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
    /// 右边那一列的滚动条。
    pub scroll: Scrollbar,
}

/// 列表右边的滚动条。列表控件 `FUN_0043e3b0(基名, rect, 38, 27, 0)` 建它，页面再用
/// `FUN_00441380`（整条的 rect）、`FUN_00442170`（上下钮）、`FUN_00442230(27, 28)`
/// （滑块宽高）摆好；画在 `0x4414f0`，命中在 `FUN_00441e30`。
///
/// 贴图 `Communal\Scroll\scrollbar0..9.tga`（`FUN_004413c0` 按 `%s%d.tga` 载十张）：
/// 0/1/2 滑块（常态/悬停/按住），3 是 16×128 的槽——画的函数里没用到，
/// 4/5/6 上钮，7/8/9 下钮。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scrollbar {
    pub rect: Rect,
    pub up: Rect,
    pub down: Rect,
    pub thumb_w: i32,
    pub thumb_h: i32,
}

/// 滚动条上的哪一块（`FUN_00441e30` 的返回值 0..4）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollPart {
    Up,
    Down,
    Thumb,
    /// 滑块上面那段槽。
    PageUp,
    /// 滑块下面那段槽。
    PageDown,
}

/// 两个买页（`0x45bd1d` / `0x46e274` 起）。
pub const BUY_SCROLL: Scrollbar = Scrollbar {
    rect: (731, 80, 769, 381),
    up: (738, 94, 760, 120),
    down: (738, 347, 760, 373),
    thumb_w: 27,
    thumb_h: 28,
};

/// 卖页（`0x471b5e` 起，每个页签一个列表，参数都一样）。
pub const SELL_SCROLL: Scrollbar = Scrollbar {
    rect: (732, 107, 770, 385),
    up: (739, 120, 761, 146),
    down: (739, 352, 761, 378),
    thumb_w: 27,
    thumb_h: 28,
};

impl Scrollbar {
    /// 滑块能走的那一段有多长：`FUN_00442000` 的 `down.top − up.bottom − 高/2`。
    fn span(&self) -> i32 {
        self.down.1 - self.up.3 - self.thumb_h / 2
    }

    /// 滑块左上角。x = rect.x + (up.x − rect.x)（`+0x150`）；
    /// y = span × pos / range + up.bottom（`FUN_00442000`），画的时候再夹到
    /// [up.bottom, down.top − 高]（`0x441631` 起）。`range` 为 0 时不画滑块。
    pub fn thumb_pos(&self, pos: usize, range: usize) -> Option<(i32, i32)> {
        if range == 0 {
            return None;
        }
        let y = self.span() * pos.min(range) as i32 / range as i32 + self.up.3;
        Some((self.up.0, y.clamp(self.up.3, self.down.1 - self.thumb_h)))
    }

    /// `FUN_00441e30`：先上下钮，再滑块、滑块上面、滑块下面（这三样只在能滚时有）。
    pub fn hit(&self, x: f32, y: f32, pos: usize, range: usize) -> Option<ScrollPart> {
        if !inside(self.rect, x, y) {
            return None;
        }
        if inside(self.up, x, y) {
            return Some(ScrollPart::Up);
        }
        if inside(self.down, x, y) {
            return Some(ScrollPart::Down);
        }
        let (_, ty) = self.thumb_pos(pos, range)?;
        let (x0, x1) = (self.rect.0, self.rect.2);
        if inside((x0, ty, x1, ty + self.thumb_h), x, y) {
            Some(ScrollPart::Thumb)
        } else if inside((x0, self.up.3, x1, ty), x, y) {
            Some(ScrollPart::PageUp)
        } else if inside((x0, ty, x1, self.down.1), x, y) {
            Some(ScrollPart::PageDown)
        } else {
            None
        }
    }

    /// 拖滑块：滑块上边在 `top` 时对应第几行，四舍五入 [推断：拖动那段
    /// （`0x441772` 起）没逐条读]。
    pub fn pos_at(&self, top: f32, range: usize) -> usize {
        let t = (top - self.up.3 as f32) / self.span() as f32;
        (t * range as f32).round().clamp(0., range as f32) as usize
    }
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
    scroll: BUY_SCROLL,
};

/// 卖页：`(472,106)` 起，高 278，12 行；列宽 131 / 52 / 36 / 36。
pub const SELL_LIST: ListGeom = ListGeom {
    rect: (472, 106, 770, 384),
    header: (466, 82, 774, 114),
    grid_rows: 12,
    columns: &[131, 52, 36, 36],
    scroll: SELL_SCROLL,
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

// ------------------------------------------------------------ 五人装备对比

/// 买装备页左边的对比栏：队里每人一张头像框；选中一件装备时，在能穿的人旁边
/// 亮出这件装备的大图标和武/速/防/运四个升降箭头。控件在 `FUN_0045b990` 里建，
/// 头像由 `FUN_0045b200..680` 排、其余由 `FUN_0045e9f0` 排，比较在
/// `FUN_004623a0`，箭头跳动在 `FUN_0046b040`。
pub mod compare {
    use super::Rect;

    /// `FUN_0045a7b0`：`+0x66dc` = 68，`+0x66e0` = 屏高 / 4 = 150，
    /// `+0x66e4/+0x66e8` = 65×86（`AttFrameHead*.tga` 的尺寸）。
    pub const HEAD_X: i32 = 68;
    pub const HEAD_STEP: i32 = 150;
    pub const HEAD_W: i32 = 65;
    pub const HEAD_H: i32 = 86;
    /// 最多排四个人（`FUN_00459fa0`：已经有四个再加就不显示）。
    pub const MAX_HEADS: usize = 4;

    /// 第 k 个显示的人（按角色号从小到大数）的头像框：
    /// y = (150 − 86) / 2 + 150k。
    pub fn head_rect(k: usize) -> Rect {
        let y = (HEAD_STEP - HEAD_H) / 2 + HEAD_STEP * k as i32;
        (HEAD_X, y, HEAD_X + HEAD_W, y + HEAD_H)
    }

    /// 衣服、武器、饰品一在左边，帽子、鞋、饰品二在右边。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Side {
        Left,
        Right,
    }

    /// 大图标：左 (x − 24, y + 36)，右 (x2 + 18, y + 36)。
    pub fn big_icon_pos(head: Rect, side: Side) -> (i32, i32) {
        match side {
            Side::Left => (head.0 - 24, head.1 + 36),
            Side::Right => (head.2 + 18, head.1 + 36),
        }
    }

    /// 头像下面一排六个小图标，x − 6 起每格 18，贴着头像底边。
    /// 顺序：衣、帽、鞋、武器、饰品、饰品。
    pub fn small_icon_pos(head: Rect, k: usize) -> (i32, i32) {
        (head.0 - 6 + 18 * k as i32, head.3)
    }

    /// 箭头的四个格：0 左上、1 右上、2 左下、3 右下。左边一组 x − 30 / x − 2，
    /// 右边一组 x2 + 4 / x2 + 32；上一行 y + 12，下一行 y2 − 38。
    pub fn arrow_pos(head: Rect, side: Side, cell: usize) -> (i32, i32) {
        let x0 = match side {
            Side::Left => head.0 - 30,
            Side::Right => head.2 + 4,
        };
        let x = x0 + if cell % 2 == 1 { 28 } else { 0 };
        let y = if cell < 2 { head.1 + 12 } else { head.3 - 38 };
        (x, y)
    }

    /// 箭头每 36 ms 往上挪 1 像素，挪满 12 再挪回来（`FUN_0046b040` 给每个
    /// 箭头的周期写 0x24，`FUN_00478880` 的往返模式）。返回往上偏了几像素。
    pub const BOB_PX: i32 = 12;
    pub const BOB_TICK: f32 = 0.036;

    pub fn bob(t: f32) -> i32 {
        let step = (t.max(0.) / BOB_TICK) as i32 % (2 * BOB_PX);
        if step <= BOB_PX {
            step
        } else {
            2 * BOB_PX - step
        }
    }

    /// 四项，也是 `W/S/F/Y_up.tga` 的顺序：武、速、防、运。
    pub const STATS: usize = 4;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Arrow {
        pub stat: usize,
        pub up: bool,
        pub cell: usize,
    }

    /// 新装备减去身上那件（没穿就当 0），按武速防运。大了亮向上箭头（上一行），
    /// 小了亮向下箭头（下一行），相等不亮。武、速偏左格，防、运偏右格：速只在
    /// 武没占左格时去左格，运只在防没占右格时去右格（`FUN_004623a0` 的
    /// `+0x10474..` 四个占位标记；速和运不记占位，所以四项全涨时速和防叠在右上，
    /// 照原样）。
    pub fn arrows(delta: [i32; STATS]) -> Vec<Arrow> {
        let mut out = Vec::new();
        let (mut tl, mut tr, mut bl, mut br) = (false, false, false, false);
        let mut put = |stat: usize, d: i32, up_cell: usize, down_cell: usize| {
            if d > 0 {
                out.push(Arrow {
                    stat,
                    up: true,
                    cell: up_cell,
                });
            } else if d < 0 {
                out.push(Arrow {
                    stat,
                    up: false,
                    cell: down_cell,
                });
            }
        };
        put(0, delta[0], 0, 2);
        tl |= delta[0] > 0;
        bl |= delta[0] < 0;
        put(1, delta[1], if tl { 1 } else { 0 }, if bl { 3 } else { 2 });
        put(2, delta[2], 1, 3);
        tr |= delta[2] > 0;
        br |= delta[2] < 0;
        put(3, delta[3], if tr { 0 } else { 1 }, if br { 2 } else { 3 });
        out
    }
}

// ------------------------------------------------------------ 物品 3D 预览

/// 说明面板上方那块黑框里的物品模型。页面上有一个 3D 视图控件（买装备页
/// `+0x16db0`、卖页 `+0x5328`，构造 `FUN_00439030`），选中一行时按物品换相机
/// 参数（`0x46c42b` / `0x4703f0` 一带）、载入 `Item\<模型>\<模型>.pol`、
/// 把贴图换成 `<模型><图标>.tga`（`gbGeomNode::ReplaceTexture`）。模型不转。
pub mod preview {
    use super::super::item_preview::PreviewCamera;
    use super::Rect;

    /// 黑框范围（量的截图，`Info.tga` 在包里没找到散文件）。黑框实际到 411，
    /// 但右上角压着「身体」页签（x ≥ 400、y < 188），预览只贴到 400，右边那
    /// 一条面板本身也是黑的。
    pub const BOX: Rect = (256, 159, 400, 340);

    /// 这件物品的相机（`FUN_0043a070(size)` + `FUN_00439930(330, y, 600)`）：
    /// `weapon_type` 5 → (228, 280)；武器（类型 11）→ (228, 272)；
    /// 8114 隐身符、8132 阎王令 → (180, 252)；其余 (180, 268)。
    pub fn camera(id: u32, item_type: u8, weapon_type: u8) -> PreviewCamera {
        let (size, y) = if weapon_type == 5 {
            (228., 280)
        } else if item_type == 11 {
            (228., 272)
        } else if id == 0x1fb2 || id == 0x1fc4 {
            (180., 252)
        } else {
            (180., 268)
        };
        PreviewCamera {
            size,
            x: 330,
            y,
            tilt: Some(600.),
        }
    }
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

    #[test]
    fn heads_stack_every_150() {
        assert_eq!(compare::head_rect(0), (68, 32, 133, 118));
        assert_eq!(compare::head_rect(3), (68, 482, 133, 568));
        let h = compare::head_rect(0);
        // 数值和 layout_trace 跑 FUN_0045e9f0 得到的一致。
        assert_eq!(compare::big_icon_pos(h, compare::Side::Left), (44, 68));
        assert_eq!(compare::big_icon_pos(h, compare::Side::Right), (151, 68));
        assert_eq!(compare::small_icon_pos(h, 0), (62, 118));
        assert_eq!(compare::small_icon_pos(h, 5), (152, 118));
        assert_eq!(compare::arrow_pos(h, compare::Side::Left, 0), (38, 44));
        assert_eq!(compare::arrow_pos(h, compare::Side::Left, 3), (66, 80));
        assert_eq!(compare::arrow_pos(h, compare::Side::Right, 1), (165, 44));
        assert_eq!(compare::arrow_pos(h, compare::Side::Right, 2), (137, 80));
    }

    #[test]
    fn arrows_pack_toward_their_side() {
        use compare::{Arrow, arrows};
        let a = |stat, up, cell| Arrow { stat, up, cell };
        // 只涨速：左格空着，速去左上。
        assert_eq!(arrows([0, 3, 0, 0]), [a(1, true, 0)]);
        // 武涨速涨：速让到右上。
        assert_eq!(arrows([1, 1, 0, 0]), [a(0, true, 0), a(1, true, 1)]);
        // 只降运：右下。防也降就让到左下。
        assert_eq!(arrows([0, 0, 0, -2]), [a(3, false, 3)]);
        assert_eq!(arrows([0, 0, -1, -2]), [a(2, false, 3), a(3, false, 2)]);
        // 相等不亮。
        assert!(arrows([0; 4]).is_empty());
    }

    #[test]
    fn arrows_bob_up_twelve_and_back() {
        assert_eq!(compare::bob(0.), 0);
        assert_eq!(compare::bob(0.036 * 12. + 0.001), 12);
        assert_eq!(compare::bob(0.036 * 18. + 0.001), 6);
        assert_eq!(compare::bob(0.036 * 24. + 0.001), 0);
    }

    #[test]
    fn the_preview_camera_follows_the_item() {
        let c = |id, t, w| {
            let p = preview::camera(id, t, w);
            (p.size, p.y)
        };
        assert_eq!(c(6601, 11, 1), (228., 272));
        assert_eq!(c(1, 11, 5), (228., 280));
        assert_eq!(c(8114, 10, 0), (180., 252));
        assert_eq!(c(8101, 10, 0), (180., 268));
        let (eye, target) = preview::camera(8101, 10, 0).eye_target();
        // 横向偏 (400 − 330) / 400 × 180 = 31.5。
        assert!((eye[0] - 31.5).abs() < 1e-4 && (target[0] - 31.5).abs() < 1e-4);
        assert!((eye[2] - 1600.).abs() < 1e-4 && target[2] == 0.);
        assert!((eye[1] - target[1] - 600.).abs() < 1e-3);
        // 视角：半宽 180、距离 √(600² + 1600²)。
        let f = preview::camera(8101, 10, 0).fov43();
        assert!(((f / 2.).tan() * 1708.8 - 180.).abs() < 0.5);
    }

    #[test]
    fn the_thumb_runs_between_the_buttons() {
        let sb = BUY_SCROLL;
        assert_eq!(sb.thumb_pos(0, 0), None, "滚不动就没有滑块");
        assert_eq!(sb.thumb_pos(0, 10), Some((738, 120)));
        // 满格按公式是 120 + 213 = 333，夹到 347 − 28。
        assert_eq!(sb.thumb_pos(10, 10), Some((738, 319)));
        assert_eq!(sb.thumb_pos(5, 10), Some((738, 226)));
        assert_eq!(sb.hit(749., 100., 0, 10), Some(ScrollPart::Up));
        assert_eq!(sb.hit(749., 360., 0, 10), Some(ScrollPart::Down));
        assert_eq!(sb.hit(749., 130., 0, 10), Some(ScrollPart::Thumb));
        assert_eq!(sb.hit(749., 200., 0, 10), Some(ScrollPart::PageDown));
        assert_eq!(sb.hit(749., 200., 10, 10), Some(ScrollPart::PageUp));
        assert_eq!(sb.hit(749., 200., 0, 0), None);
        assert_eq!(sb.pos_at(120., 10), 0);
        assert_eq!(sb.pos_at(226.5, 10), 5);
        assert_eq!(sb.pos_at(400., 10), 10);
    }
}
