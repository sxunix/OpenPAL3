//! 战斗界面的版面（原版 `D:\Project\PAL3\Combat\CBUI.cpp`）。
//!
//! 坐标都是 800×600 原生像素，读自 exe，地址写在各常量上。控件 rect 的
//! 约定同 `switch/re/notes/state_page_layout.md` §0：
//!
//! * **子控件的 rect 相对父控件**——`FUN_00445b90` 有父时调
//!   `FUN_00445e10`，把父的左上角加上去。CBUI 自己的 rect 是整屏
//!   (0,0)-(800,600)（`FUN_005163d0`），所以挂在它下面的就是屏幕坐标。
//! * 贴图按原生尺寸贴，rect 只截贴图**左下角** w×h 那一块（`FUN_00540210`）。
//!
//! 逆向记录在 `switch/re/notes/combat_hud.md`。

use super::formulas::GAUGE_FULL;
use super::session::MenuEntry;

/// (x1, y1, x2, y2)。
pub type Rect = (i32, i32, i32, i32);

/// 战斗界面贴图的目录（`basedata` 下）。
pub const UI_DIR: &str = "ui/combat";

/// 把相对父控件的 rect 换成父控件所在的坐标系（`FUN_00445e10`）。
pub fn offset(r: Rect, parent: Rect) -> Rect {
    (
        r.0 + parent.0,
        r.1 + parent.1,
        r.2 + parent.0,
        r.3 + parent.1,
    )
}

// ---------------------------------------------------------------- 控制盘

/// 控制盘面板。`FUN_00517a50` 在 0x517a83 把这个 rect 传给 `FUN_00519400`。
pub const CONTROL_PANEL: Rect = (10, 225, 266, 481);
/// 盘底，相对面板 (0,0)-(256,256)（`FUN_00519400`）。
pub const CONTROL_BACK: &str = "control/back.tga";
pub const CONTROL_BACK_RECT: Rect = (0, 0, 256, 256);

/// 控制盘上的按钮，`FUN_005196c0` 逐个建。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RingButton {
    /// 擊
    Attack,
    /// 技（特技）
    Skill,
    /// 術（仙术）
    Magic,
    /// 物
    Item,
    /// 防
    Defence,
    /// 陣
    Lineup,
    /// AI
    Ai,
    /// 逃
    Flee,
    /// 援
    Protect,
    /// 魔。建完就 show(0)，平时不出来。
    Mj,
}

impl RingButton {
    /// `FUN_005196c0` 建按钮的顺序。
    pub const ALL: [RingButton; 10] = [
        RingButton::Mj,
        RingButton::Attack,
        RingButton::Skill,
        RingButton::Magic,
        RingButton::Item,
        RingButton::Lineup,
        RingButton::Ai,
        RingButton::Flee,
        RingButton::Protect,
        RingButton::Defence,
    ];

    /// 控件 id（`FUN_00519a10` 的第二个参数）。
    pub fn id(self) -> u32 {
        match self {
            RingButton::Attack => 0x103,
            RingButton::Skill => 0x104,
            RingButton::Magic => 0x105,
            RingButton::Item => 0x106,
            RingButton::Lineup => 0x108,
            RingButton::Defence => 0x109,
            RingButton::Ai => 0x10a,
            RingButton::Flee => 0x10b,
            RingButton::Protect => 0x10c,
            RingButton::Mj => 0x10d,
        }
    }

    /// 贴图名的主干：`ui\combat\control\<主干>` 后面接 `0`/`1`/`2`，
    /// 不可用那张是 `<主干>3.tga`。
    pub fn stem(self) -> &'static str {
        match self {
            RingButton::Attack => "attack",
            RingButton::Skill => "skill",
            RingButton::Magic => "magic",
            RingButton::Item => "item",
            RingButton::Lineup => "lineup",
            RingButton::Defence => "defence",
            RingButton::Ai => "ai",
            RingButton::Flee => "flee",
            RingButton::Protect => "protect",
            RingButton::Mj => "mj",
        }
    }

    /// 相对控制盘面板的 rect，逐条读自 `FUN_005196c0`。
    pub fn rect(self) -> Rect {
        match self {
            RingButton::Mj => (-13, 185, 71, 269),
            RingButton::Attack => (53, 156, 99, 202),
            RingButton::Skill => (57, 97, 103, 143),
            RingButton::Magic => (2, 153, 48, 199),
            RingButton::Item => (56, 208, 102, 254),
            RingButton::Lineup => (19, 192, 65, 238),
            RingButton::Ai => (19, 121, 65, 167),
            RingButton::Flee => (90, 192, 136, 238),
            RingButton::Protect => (90, 121, 136, 167),
            RingButton::Defence => (111, 153, 157, 199),
        }
    }

    /// 建好之后显不显示：「魔」是 show(0)，其余 show(1)。
    pub fn shown_at_start(self) -> bool {
        self != RingButton::Mj
    }

    /// 有没有不可用那张图（`FUN_0043aa00(<主干>3.tga, 0)`）。「魔」没有。
    pub fn has_disabled_look(self) -> bool {
        self != RingButton::Mj
    }

    /// 九宫格里的位置（列, 行）。按 rect 中心排出来：擊居中，技術防物在
    /// 上左右下，AI 援 陣 逃 在四个斜角，和盘底图上的字对得上。
    pub fn cell(self) -> Option<(i32, i32)> {
        Some(match self {
            RingButton::Ai => (0, 0),
            RingButton::Skill => (1, 0),
            RingButton::Protect => (2, 0),
            RingButton::Magic => (0, 1),
            RingButton::Attack => (1, 1),
            RingButton::Defence => (2, 1),
            RingButton::Lineup => (0, 2),
            RingButton::Item => (1, 2),
            RingButton::Flee => (2, 2),
            RingButton::Mj => return None,
        })
    }

    pub fn at_cell(cell: (i32, i32)) -> Option<RingButton> {
        RingButton::ALL.into_iter().find(|b| b.cell() == Some(cell))
    }

    /// 移植端的规则层做了哪一项。陣、AI、援、魔还没有。
    pub fn entry(self) -> Option<MenuEntry> {
        match self {
            RingButton::Attack => Some(MenuEntry::Attack),
            RingButton::Skill => Some(MenuEntry::Special),
            RingButton::Magic => Some(MenuEntry::Skill),
            RingButton::Item => Some(MenuEntry::Item),
            RingButton::Defence => Some(MenuEntry::Defend),
            RingButton::Flee => Some(MenuEntry::Escape),
            RingButton::Ai => Some(MenuEntry::Ai),
            RingButton::Lineup => Some(MenuEntry::Lineup),
            _ => None,
        }
    }

    pub fn of_entry(entry: MenuEntry) -> RingButton {
        match entry {
            MenuEntry::Attack => RingButton::Attack,
            MenuEntry::Special => RingButton::Skill,
            MenuEntry::Skill => RingButton::Magic,
            MenuEntry::Item => RingButton::Item,
            MenuEntry::Defend => RingButton::Defence,
            MenuEntry::Escape => RingButton::Flee,
            MenuEntry::Ai => RingButton::Ai,
            MenuEntry::Lineup => RingButton::Lineup,
        }
    }
}

/// 按钮的四张图。0/1/2 分别是平时、指上去、按下 **[推断]**（按钮基类的
/// 惯例，没有逐条核对）；3 是 `FUN_0043aa00` 另挂的那张，按用途当不可用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonLook {
    Normal = 0,
    Focused = 1,
    Pressed = 2,
    Disabled = 3,
}

pub fn button_texture(button: RingButton, look: ButtonLook) -> String {
    format!("control/{}{}.tga", button.stem(), look as u8)
}

/// 手柄在盘上移动焦点。**原版控制盘只认鼠标**（`FUN_005195e0` 只处理
/// 指上去 0x3e9 和点下 0x3ef），这是移植端加的：朝一个方向走，跳过不可用
/// 的格子，走出格子就原地不动。
pub fn ring_move(
    from: RingButton,
    (dx, dy): (i32, i32),
    enabled: impl Fn(RingButton) -> bool,
) -> RingButton {
    let Some((mut col, mut row)) = from.cell() else {
        return from;
    };
    loop {
        col += dx;
        row += dy;
        match RingButton::at_cell((col, row)) {
            None => return from,
            Some(b) if enabled(b) => return b,
            Some(_) => {}
        }
    }
}

// ---------------------------------------------------------------- 状态面板

/// 底下一排四块（`FUN_00516a30` 的第一个循环）。
pub const PANELS: usize = 4;
/// `DAT_00570178`
pub const PANEL_X: [i32; PANELS] = [0, 207, 403, 600];
/// `DAT_00570188`
pub const PANEL_Y: [i32; PANELS] = [491, 491, 491, 491];
pub const PANEL_W: i32 = 0x9d;
pub const PANEL_H: i32 = 0x6a;

pub fn panel_rect(panel: usize) -> Rect {
    let (x, y) = (PANEL_X[panel], PANEL_Y[panel]);
    (x, y, x + PANEL_W, y + PANEL_H)
}

/// 第一块用 `StatePanel0`（带剑槽），其余 `StatePanel`。
pub fn panel_texture(panel: usize) -> &'static str {
    if panel == 0 {
        "state/StatePanel0.tga"
    } else {
        "state/StatePanel.tga"
    }
}

/// 精 / 气 / 神三条。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gauge {
    Hp,
    Gp,
    Mp,
}

impl Gauge {
    pub const ALL: [Gauge; 3] = [Gauge::Hp, Gauge::Gp, Gauge::Mp];

    /// 相对面板左上角。第一块面板整体右移 12（多了剑槽）。
    /// 读自 `FUN_00515e90` / `00515ee0` / `00515f30` / `00515f80` /
    /// `00515fd0` / `00516020`，它们开机时把结果写进 `DAT_017ace1c` 起的表。
    fn offset(self, panel: usize) -> (i32, i32) {
        let first = panel == 0;
        match self {
            Gauge::Hp => (if first { 0x77 } else { 0x6b }, 0x1c),
            Gauge::Gp => (if first { 0x80 } else { 0x74 }, 0x36),
            Gauge::Mp => (if first { 0x79 } else { 0x6d }, 0x52),
        }
    }

    /// 屏幕上的条，85×12（`FUN_00516a30`：x2 = x+0x55、y2 = y+0xc）。
    pub fn rect(self, panel: usize) -> Rect {
        let (dx, dy) = self.offset(panel);
        let (x, y) = (PANEL_X[panel] + dx, PANEL_Y[panel] + dy);
        (x, y, x + 0x55, y + 0xc)
    }

    pub fn fill_texture(self) -> &'static str {
        match self {
            Gauge::Hp => "state/ProgBar_HP.tga",
            Gauge::Gp => "state/ProgBar_GP.tga",
            Gauge::Mp => "state/ProgBar_MP.tga",
        }
    }
}

/// 三条共用的槽。
pub const GAUGE_BACK: &str = "state/ProgBar.tga";

/// 进度条的内边距（`+0x84..+0x94`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BarPadding {
    pub left: i32,
    pub right: i32,
    pub top: i32,
    pub bottom: i32,
    pub common: i32,
}

/// 精气神三条：`FUN_0043fdb0(-2)` 左、`fdc0(7)` 右、`fdd0(1)` 上、
/// `fde0(1)` 下；通用 4 是构造时的默认值，没改。
pub const GAUGE_PADDING: BarPadding = BarPadding {
    left: -2,
    right: 7,
    top: 1,
    bottom: 1,
    common: 4,
};

/// 横向条的填充区（绘制在 0x4401de）：
///
/// ```text
/// x1 = 左 + 通用 + rect.x1
/// y1 = rect.y1 + 上 + 通用 − 1
/// y2 = rect.y2 − (下 + 通用)
/// 满值宽 = (rect.x2 − 右 − 通用) − x1          （FUN_0043fca0）
/// x2 = x1 + 值 × 满值宽 / 上限                 （FUN_0043fdf0，截断取整）
/// ```
///
/// 空条返回 `None`。
pub fn fill_rect(bar: Rect, pad: BarPadding, value: f32, max: f32) -> Option<Rect> {
    if max <= 0. {
        return None;
    }
    let x1 = bar.0 + pad.left + pad.common;
    let y1 = bar.1 + pad.top + pad.common - 1;
    let y2 = bar.3 - (pad.bottom + pad.common);
    let full = (bar.2 - pad.right - pad.common) - x1;
    let width = (value.clamp(0., max) * full as f32 / max) as i32;
    (width > 0 && y2 > y1).then_some((x1, y1, x1 + width, y2))
}

/// 满值时填充区多宽。
pub fn full_fill_width(bar: Rect, pad: BarPadding) -> i32 {
    (bar.2 - pad.right - pad.common) - (bar.0 + pad.left + pad.common)
}

/// 第一块面板左边的竖条（`FUN_00516a30`，`DAT_017acb8c`/`DAT_017acb88`
/// 由 `FUN_00516070`/`FUN_00516090` 算成面板 + (12, 17)）。上限 50
/// （`FUN_0043fca0(50.0)`），开战清零。**谁往里加值还没找到**，现在一直空。
pub fn sword_rect() -> Rect {
    let (x, y) = (PANEL_X[0] + 12, PANEL_Y[0] + 17);
    (x, y, x + 0x1e, y + 0x56)
}
pub const SWORD_BAR: &str = "state/SwordBar.tga";
pub const SWORD_FILL: &str = "state/Sword.tga";
pub const SWORD_MAX: f32 = 50.;

// ---------------------------------------------------------------- 头像

/// 左上角的 Q 版头像和竖排名字，五个角色各一套、rect 全一样
/// （`FUN_00516a30` 第二个循环，父控件是 CBUI，所以是屏幕坐标）。
/// 同一时刻显示哪一张 **[推断]**：当前在选行动的那个人。
pub const PORTRAIT_RECT: Rect = (6, 12, 70, 94);
pub const NAME_RECT: Rect = (10, 10, 30, 42);

/// 按我方战斗位（0 景天 … 4 长卿，`PTR_…_Jingtian_tga_0057042c` 那张表）。
pub const PORTRAIT_NAMES: [&str; 5] = ["Jingtian", "Xuejian", "Longkui", "Zixuan", "Changqing"];

pub fn portrait_texture(slot: usize) -> Option<String> {
    PORTRAIT_NAMES.get(slot).map(|n| format!("state/{n}.tga"))
}

pub fn name_texture(slot: usize) -> Option<String> {
    PORTRAIT_NAMES
        .get(slot)
        .map(|n| format!("state/{n}Name.tga"))
}

// ---------------------------------------------------------------- 行动条

/// 右上角的行动条底（`FUN_00517650`）。512 宽的图，rect 只有 320 宽。
pub const SEQUENCE_BACK: &str = "attacksequen/back.tga";
pub const SEQUENCE_BACK_RECT: Rect = (0x1e3, 10, 0x323, 0x2a);

/// 每个参战者一个头像，64×64，y 恒为 8。
pub const HEAD_SIZE: i32 = 64;
pub const HEAD_Y: i32 = 8;

/// 头像的 x（`FUN_004dbb20`）：蓄力阶段按蓄力条从 460 走到 657；出手阶段
/// 按出手条从 659 走到 749。`__ftol` 截断。
pub fn head_x(gauge: f32, cast_gauge: f32, executing: bool) -> i32 {
    if executing {
        (cast_gauge * 0.007_826_087 + 659.) as i32
    } else {
        (gauge * 0.017_130_435 + 460.) as i32
    }
}

pub fn head_rect(x: i32) -> Rect {
    (x, HEAD_Y, x + HEAD_SIZE, HEAD_Y + HEAD_SIZE)
}

/// 我方五个战斗位的行动条头像名。包里 `attacksequen` 下只有 101 102 104
/// 106 107——龙葵那张是 **106**（图上就是龙葵），不是舞台用的 105。原版
/// 头像和舞台都取战斗位 `+8` 的同一个模型名，所以舞台上的龙葵也许该用
/// 106，还没核实。
pub const PARTY_HEADS: [&str; 5] = ["101", "104", "106", "107", "102"];

/// `ui\combat\attacksequen\<模型名>.tga`（`FUN_004db9f0`，模型名取自
/// 战斗位 `+8`）。包里全是小写。
pub fn head_texture(model: &str) -> String {
    format!("attacksequen/{}.tga", model.to_lowercase())
}

/// 蓄力条满格，头像停在 460 + 197。
pub const HEAD_READY_X: i32 = (GAUGE_FULL * 0.017_130_435 + 460.) as i32;

// ---------------------------------------------------------------- 列表

/// 仙术列表（`FUN_0051a430`，面板 rect 由 0x517b63 传入）。
pub const MAGIC_PANEL: Rect = (2, 278, 251, 484);
/// 道具列表（`FUN_00519bd0`，0x517af3）。
pub const ITEM_PANEL: Rect = (6, 278, 255, 484);
/// 特技列表（`FUN_0051ad10`，0x517bd3）。
pub const SKILL_PANEL: Rect = (5, 237, 261, 493);

/// 列表底图相对面板的 rect：仙术、道具是 249×206。
pub const LIST_BACK_RECT: Rect = (0, 0, 0xf9, 0xce);
pub const MAGIC_BACK: &str = "control/MagicList/MagicPanel.tga";
pub const ITEM_BACK: &str = "control/ItemList/ItemPanel.tga";
/// 特技列表的底图，256×256。
pub const SKILL_BACK: &str = "control/SkillList/SkillPanel.tga";
pub const SKILL_BACK_RECT: Rect = (0, 0, 0x100, 0x100);
/// 特技列表的行区（`FUN_0051ad10`：相对面板 (0x27,0x6e)-(0xf1,0xe2)），正好六行——
/// 每人最多六条特技（战斗位 `+0x2c8` 六格）。
pub const SKILL_ROWS: Rect = (0x27, 0x6e, 0xf1, 0xe2);
/// 选中条画到哪：行区比底图的框线宽十几像素，两列（0x96 + 0x2b）加起来正好到框线，
/// 选中条按两列的总宽截 [推断：原版选中条怎么裁没读]。
pub const SKILL_SELECT_ROWS: Rect = (
    SKILL_ROWS.0,
    SKILL_ROWS.1,
    SKILL_ROWS.0 + SKILL_COLUMNS[0] + SKILL_COLUMNS[1],
    SKILL_ROWS.3,
);
pub const SKILL_VISIBLE_ROWS: usize = ((SKILL_ROWS.3 - SKILL_ROWS.1) / ROW_HEIGHT) as usize;
/// 特技列表两列：名称 0x96、耗气 0x2b（0x51af2d / 0x51af3e 两次 `FUN_0043e560`）。
pub const SKILL_COLUMNS: [i32; 2] = [0x96, 0x2b];
/// 选中行的底（`FUN_0043e330`，仙术和道具列表都用道具那张）。
pub const ROW_SELECT: &str = "control/ItemList/SelPic.tga";

/// 列表的行区，相对面板（两个列表都是 (0x27,0x4e)-(0xf1,0xaf)）。
pub const LIST_ROWS: Rect = (0x27, 0x4e, 0xf1, 0xaf);
/// 行高（`FUN_0043f640(0x13)`）。
pub const ROW_HEIGHT: i32 = 0x13;
/// 行区装得下几行。
pub const VISIBLE_ROWS: usize = ((LIST_ROWS.3 - LIST_ROWS.1) / ROW_HEIGHT) as usize;

/// 仙术列表三列：名称、耗神、级别/次（`FUN_0043e560` 的宽度）。
pub const MAGIC_COLUMNS: [i32; 3] = [0x60, 0x25, 0x32];
/// 道具列表两列：名称、数量。
pub const ITEM_COLUMNS: [i32; 2] = [0x96, 0x2b];

/// 选中第 `index` 行时，列表从第几行开始画——让选中行留在可见区里。
pub fn first_visible(index: usize, len: usize) -> usize {
    first_visible_in(index, len, VISIBLE_ROWS)
}

/// 同 [`first_visible`]，可见行数另给（特技列表是六行）。
pub fn first_visible_in(index: usize, len: usize, visible: usize) -> usize {
    if len <= visible {
        return 0;
    }
    index.saturating_sub(visible - 1).min(len - visible)
}

// ---------------------------------------------------------------- 横幅

/// 招式名的横幅（`FUN_005177b0`，+0x430 / +0x434 两块，rect 相同）。
pub const TRICK_NAME_RECT: Rect = (300, 30, 461, 87);
pub const TRICK_BACK: &str = "TrickNameBack.tga";
pub const TRICK_BACK2: &str = "TrickNameBack2.tga";
/// 横幅的字号（`FUN_004427a0(1)` → 20px 档）。
pub const TRICK_FONT_PX: i32 = 20;

// ---------------------------------------------------------------- 托管面板

/// 托管面板（控制盘「AI」，`FUN_0051b210`）。
pub const AI_PANEL: Rect = (14, 316, 172, 474);
/// 底图 256×256，面板本身 0x9e×0x9e，只露左上那一块。
pub const AI_BACK: &str = "control/AI/back.tga";
pub const AI_BACK_RECT: Rect = (0, 0, 0x9e, 0x9e);
/// 六个勾选框（控件 0xd6..0xdb），按 [`super::ai::Switch::ALL`] 的顺序。
pub const AI_SWITCH_RECTS: [Rect; 6] = [
    (7, 0x30, 0x50, 0x49),
    (0x4e, 0x30, 0x97, 0x49),
    (7, 0x49, 0x50, 0x62),
    (0x4e, 0x49, 0x97, 0x62),
    (7, 0x62, 0x50, 0x7b),
    (0x4e, 0x62, 0x97, 0x7b),
];
/// 「确定」（0xdc），`ok0/1/2` 三张：平时、指上去、按下。
pub const AI_OK_RECT: Rect = (0x23, 0x7f, 0x7f, 0x99);
/// 面板焦点里「确定」的编号（0..=5 是勾选框）。焦点是移植端给手柄加的，原版只认鼠标。
pub const AI_OK_FOCUS: usize = 6;
/// 人名条（控件 0xd0..0xd4 五张，`PTR_…_AI_jingtian_tga_00570218`），叠在同一格。
pub const AI_NAME_RECT: Rect = (0x39, 0x1f, 0x5f, 0x2f);
pub const AI_NAMES: [&str; 5] = ["jingtian", "xuejian", "longkui", "zixuan", "changqing"];

pub fn ai_switch_texture(stem: &str, on: bool) -> String {
    format!("control/AI/{stem}{}.tga", on as u8)
}

pub fn ai_ok_texture(look: u8) -> String {
    format!("control/AI/ok{look}.tga")
}

pub fn ai_name_texture(slot: usize) -> Option<String> {
    AI_NAMES.get(slot).map(|n| format!("control/AI/{n}.tga"))
}

// ---------------------------------------------------------------- 阵型面板

/// 阵型面板（控制盘「陣」，`FUN_0051b710`）：底图 `lineup\back.tga`，相对面板
/// (0,0)-(0xe6,0xfa)，控件 0xf0。
pub const LINEUP_PANEL: Rect = (9, 232, 239, 482);
pub const LINEUP_BACK: &str = "control/Lineup/back.tga";
pub const LINEUP_BACK_RECT: Rect = (0, 0, 0xe6, 0xfa);
/// 五张头像（控件 0xf1..0xf5）按各自的五灵位摆：`FUN_0051bb50` 取
/// `DAT_0057024c / DAT_00570268[位]`，再加 (9, 0xe8)——正好是面板原点，所以这里
/// 存相对面板的坐标。下标是战斗位 `+0x470`：1 水 2 火 3 风 4 雷 5 土 6 盘心，0 不用。
pub const LINEUP_SPOTS: [(i32, i32); 7] = [
    (0, 0),
    (0x68, 0xbc),
    (0x2e, 0x94),
    (0x88, 0x57),
    (0x41, 0x57),
    (0x9c, 0x94),
    (0x68, 0x83),
];
/// 头像控件 0x28×0x26（贴图 64×64，只露左上）。
pub const LINEUP_FACE: (i32, i32) = (0x28, 0x26);
/// 抓起来的头像放大到 1.4 倍（0x4de489 写控件缩放 0x3fb33333），放下回 1.0。
pub const LINEUP_HELD_SCALE: f32 = 1.4;
pub const LINEUP_NAMES: [&str; 5] = ["jingtian", "xuejian", "longkui", "zixuan", "changqing"];

pub fn lineup_face_rect(spot: u8) -> Option<Rect> {
    if spot == 0 {
        return None;
    }
    let (x, y) = *LINEUP_SPOTS.get(spot as usize)?;
    Some((x, y, x + LINEUP_FACE.0, y + LINEUP_FACE.1))
}

/// 头像：平时 `<名>.tga`，动不了的 `<名>_d.tga`（`FUN_004425d0` 挂的第二张，暗色）。
pub fn lineup_face_texture(slot: usize, dimmed: bool) -> Option<String> {
    LINEUP_NAMES
        .get(slot)
        .map(|n| format!("control/Lineup/{n}{}.tga", if dimmed { "_d" } else { "" }))
}

/// 手柄在六个位上挪光标。**原版只能用鼠标拖**，这是移植端加的：朝那个方向、
/// 按「距离² + 4×偏离²」取最近的位，那个方向上没有就不动。
pub fn lineup_move(from: u8, (dx, dy): (i32, i32)) -> u8 {
    let (fx, fy) = LINEUP_SPOTS[from.clamp(1, 6) as usize];
    let mut best: Option<(u8, i32)> = None;
    for spot in 1..=6u8 {
        if spot == from {
            continue;
        }
        let (x, y) = LINEUP_SPOTS[spot as usize];
        let (vx, vy) = (x - fx, y - fy);
        let along = vx * dx + vy * dy;
        if along <= 0 {
            continue;
        }
        let across = vx * dy - vy * dx;
        let cost = vx * vx + vy * vy + 4 * across * across;
        if best.map(|(_, c)| cost < c).unwrap_or(true) {
            best = Some((spot, cost));
        }
    }
    best.map(|(s, _)| s).unwrap_or(from)
}

/// 全部要用的贴图，给普查测试核对包里都在。
pub fn every_texture() -> Vec<String> {
    let mut all = vec![CONTROL_BACK.to_string()];
    for b in RingButton::ALL {
        for look in [ButtonLook::Normal, ButtonLook::Focused, ButtonLook::Pressed] {
            all.push(button_texture(b, look));
        }
        if b.has_disabled_look() {
            all.push(button_texture(b, ButtonLook::Disabled));
        }
    }
    all.push(panel_texture(0).to_string());
    all.push(panel_texture(1).to_string());
    all.push(GAUGE_BACK.to_string());
    all.extend(Gauge::ALL.iter().map(|g| g.fill_texture().to_string()));
    all.push(SWORD_BAR.to_string());
    all.push(SWORD_FILL.to_string());
    for slot in 0..PORTRAIT_NAMES.len() {
        all.extend(portrait_texture(slot));
        all.extend(name_texture(slot));
    }
    all.push(SEQUENCE_BACK.to_string());
    all.extend([MAGIC_BACK, ITEM_BACK, ROW_SELECT, TRICK_BACK, TRICK_BACK2].map(String::from));
    all.push(AI_BACK.to_string());
    for s in super::ai::Switch::ALL {
        for on in [false, true] {
            all.push(ai_switch_texture(s.stem(), on));
        }
    }
    all.extend((0u8..3).map(ai_ok_texture));
    all.extend((0..AI_NAMES.len()).filter_map(ai_name_texture));
    all.push(LINEUP_BACK.to_string());
    for slot in 0..LINEUP_NAMES.len() {
        all.extend(lineup_face_texture(slot, false));
        all.extend(lineup_face_texture(slot, true));
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pad_walks_the_lineup_disc_by_direction() {
        assert_eq!(lineup_move(1, (0, -1)), 6, "水往上是盘心");
        assert_eq!(lineup_move(6, (-1, 0)), 2, "盘心往左是火");
        assert_eq!(lineup_move(6, (1, 0)), 5, "盘心往右是土");
        assert_eq!(lineup_move(4, (1, 0)), 3, "雷往右是风");
        assert_eq!(lineup_move(1, (0, 1)), 1, "水下面没有位，不动");
    }

    fn center(r: Rect) -> (i32, i32) {
        ((r.0 + r.2) / 2, (r.1 + r.3) / 2)
    }

    #[test]
    fn the_grid_matches_where_the_buttons_actually_sit() {
        // 同一列的按钮中心 x 递增，同一行的 y 递增——九宫格不是拍脑袋排的。
        for a in RingButton::ALL {
            for b in RingButton::ALL {
                let (Some((ac, ar)), Some((bc, br))) = (a.cell(), b.cell()) else {
                    continue;
                };
                let (pa, pb) = (center(a.rect()), center(b.rect()));
                if ac < bc {
                    assert!(pa.0 < pb.0, "{a:?} 应在 {b:?} 左边");
                }
                if ar < br {
                    assert!(pa.1 < pb.1, "{a:?} 应在 {b:?} 上面");
                }
            }
        }
        // 四个正方向到擊的距离差不多，擊就是盘心。
        let c = center(RingButton::Attack.rect());
        for b in [
            RingButton::Skill,
            RingButton::Magic,
            RingButton::Item,
            RingButton::Defence,
        ] {
            let p = center(b.rect());
            let d = (((p.0 - c.0).pow(2) + (p.1 - c.1).pow(2)) as f32).sqrt();
            assert!((50. ..=60.).contains(&d), "{b:?} 离盘心 {d}");
        }
    }

    #[test]
    fn eight_buttons_are_46_square_and_mj_is_the_odd_one() {
        for b in RingButton::ALL {
            let r = b.rect();
            if b == RingButton::Mj {
                assert_eq!((r.2 - r.0, r.3 - r.1), (84, 84));
                assert!(r.0 < 0, "魔探出盘外");
            } else {
                assert_eq!((r.2 - r.0, r.3 - r.1), (46, 46), "{b:?}");
                assert!(r.0 >= 0 && r.2 <= 256 && r.1 >= 0 && r.3 <= 256);
            }
        }
        assert!(!RingButton::Mj.shown_at_start());
        assert_eq!(
            RingButton::ALL
                .iter()
                .filter(|b| b.shown_at_start())
                .count(),
            9
        );
    }

    #[test]
    fn ids_are_the_ones_the_click_handler_accepts() {
        // FUN_005195e0 只对 0x102 < id < 0x10d 播指上去的声音——魔不在内。
        for b in RingButton::ALL {
            let id = b.id();
            assert_eq!(0x102 < id && id < 0x10d, b != RingButton::Mj, "{b:?}");
        }
    }

    #[test]
    fn the_pad_walks_the_grid_and_skips_what_is_not_there() {
        let ported = |b: RingButton| b.entry().is_some();
        let from = RingButton::Attack;
        assert_eq!(ring_move(from, (-1, 0), ported), RingButton::Magic);
        assert_eq!(ring_move(from, (1, 0), ported), RingButton::Defence);
        assert_eq!(ring_move(from, (0, 1), ported), RingButton::Item);
        // 技（特技）做了，往上走到技。
        assert_eq!(ring_move(from, (0, -1), ported), RingButton::Skill);
        // 逃在右下角：从物往右、从防往下都能到。
        assert_eq!(
            ring_move(RingButton::Item, (1, 0), ported),
            RingButton::Flee
        );
        assert_eq!(
            ring_move(RingButton::Defence, (0, 1), ported),
            RingButton::Flee
        );
        // 到边上就停。
        assert_eq!(
            ring_move(RingButton::Flee, (1, 0), ported),
            RingButton::Flee
        );
        // 全都可用时，从擊往左上走一格也不会斜着跳。
        assert_eq!(ring_move(from, (0, -1), |_| true), RingButton::Skill);
    }

    #[test]
    fn every_menu_entry_has_a_button_and_back() {
        for e in MenuEntry::ALL {
            assert_eq!(RingButton::of_entry(e).entry(), Some(e));
        }
    }

    #[test]
    fn the_panels_and_the_visible_bars_fit_the_screen() {
        for p in 0..PANELS {
            let r = panel_rect(p);
            assert!(r.2 <= 800 && r.3 <= 600, "面板 {p} 出界 {r:?}");
            for g in Gauge::ALL {
                // rect 只是命中区；槽图的可见部分只有左边 80 像素
                // （ProgBar.tga 的不透明列是 0..79），这部分不出屏。
                let b = g.rect(p);
                assert!(b.0 + 80 <= 800, "{g:?}@{p} 出界");
            }
        }
        // 气条的 rect 本身是越界的——exe 就是这么写的，别去「修」。
        assert_eq!(Gauge::Gp.rect(0).2, 213, "伸进第二块（x=207）");
        assert_eq!(Gauge::Gp.rect(3).2, 801, "伸出屏幕一个像素");
        // 第一块整体右挪 12。
        assert_eq!(
            Gauge::Hp.rect(0).0 - PANEL_X[0],
            Gauge::Hp.rect(1).0 - PANEL_X[1] + 12
        );
    }

    #[test]
    fn the_fill_follows_the_padding_arithmetic() {
        let bar = Gauge::Hp.rect(1);
        assert_eq!(
            full_fill_width(bar, GAUGE_PADDING),
            72,
            "85 − (−2+4) − (7+4)"
        );
        let full = fill_rect(bar, GAUGE_PADDING, 88., 88.).unwrap();
        assert_eq!(full, (bar.0 + 2, bar.1 + 4, bar.0 + 2 + 72, bar.3 - 5));
        let half = fill_rect(bar, GAUGE_PADDING, 44., 88.).unwrap();
        assert_eq!(half.2 - half.0, 36);
        assert_eq!(fill_rect(bar, GAUGE_PADDING, 0., 88.), None);
        assert_eq!(fill_rect(bar, GAUGE_PADDING, 5., 0.), None);
        // 超上限按上限算。
        assert_eq!(fill_rect(bar, GAUGE_PADDING, 999., 88.), Some(full));
    }

    #[test]
    fn heads_travel_the_bar_in_two_legs() {
        assert_eq!(head_x(0., 0., false), 460);
        assert_eq!(head_x(GAUGE_FULL, 0., false), 657);
        assert_eq!(HEAD_READY_X, 657);
        assert_eq!(head_x(GAUGE_FULL, 0., true), 659, "出手阶段从 659 起");
        assert_eq!(head_x(0., GAUGE_FULL, true), 749);
        // 两段首尾衔接、不回头。
        assert!(head_x(GAUGE_FULL, 0., false) < head_x(0., 0., true));
        assert_eq!(head_texture("B01"), "attacksequen/b01.tga");
    }

    #[test]
    fn the_list_scrolls_to_keep_the_selection_visible() {
        assert_eq!(VISIBLE_ROWS, 5);
        assert_eq!(first_visible(0, 3), 0);
        assert_eq!(first_visible(4, 10), 0);
        assert_eq!(first_visible(5, 10), 1);
        assert_eq!(first_visible(9, 10), 5);
        assert_eq!(first_visible(9, 5), 0);
    }
}
