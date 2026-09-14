//! 战斗结算画面：分出胜负后那一段时间轴（`FUN_004f7910` 排的任务组）。
//!
//! 版面坐标和 [`hud`](super::hud) 一样是 800×600、贴图截左下角。逆向记录在
//! `switch/re/notes/combat_result.md`。

use super::hud::{BarPadding, Rect};
use super::state::Outcome;

// ---------------------------------------------------------------- 版面

/// 胜 / 败大字（`FUN_00516940`：id 800 / 0x330，初始隐藏）。
pub const BIG_WIN: &str = "result/bigwin.tga";
pub const BIG_LOSE: &str = "result/biglose.tga";
pub const BIG_RECT: Rect = (150, -50, 406, 462);

/// 四块人物面板（`FUN_00517d60`：x = 0/200/400/600，256×256）。
pub const PANELS: usize = 4;
pub const PANEL_X: [i32; PANELS] = [0, 200, 400, 600];
pub const PANEL_BACK: &str = "result/back.tga";
pub fn panel_rect(panel: usize) -> Rect {
    let x = PANEL_X[panel];
    (x, 0, x + 256, 256)
}

/// 以下相对面板（`FUN_0051c330`）。
pub const FACE_RECT: Rect = (15, -15, 79, 113);
pub fn face_texture(slot: usize) -> String {
    format!("result/face/{slot}.tga")
}
/// 精 / 气 / 神三条，槽是 `SLOT.tga`。
pub const GAUGE_RECTS: [Rect; 3] = [(83, 46, 168, 58), (86, 71, 171, 83), (84, 97, 169, 109)];
pub const GAUGE_FILLS: [&str; 3] = [
    "result/face/HP.tga",
    "result/face/GP.tga",
    "result/face/MP.tga",
];
pub const GAUGE_SLOT: &str = "result/face/SLOT.tga";
/// `FUN_0043fd90(3)`、`FUN_0043fdb0(-2)`；右 −4、上 0、下 −3 是构造时的默认值。
pub const GAUGE_PADDING: BarPadding = BarPadding {
    left: -2,
    right: -4,
    top: 0,
    bottom: -3,
    common: 3,
};
/// 「精 气 神」三个字的竖条（id 0x220）。
pub const GAUGE_LABEL: &str = "result/face/HP-GP-MP.tga";
pub const GAUGE_LABEL_RECT: Rect = (66, 42, 130, 109);
/// 名字和等级那一行：控件 0x212（`FUN_0051c330` 建在面板 `+0x68`，(0,0)-(187,32)）。
/// `FUN_005184e0` 用 `%s%s %d %s` 拼好后 `FUN_004427c0(字, 1)` 写进 0x212；紧挨着的 0x211
/// （`+0x64`，(0,211)-(187,243)）写的是 `DAT_00bfdaa4`——一个空串，所以那一行是空的。
/// `FUN_004427c0` 第二个参数只写控件 `+0x48`，名字这句是 1，用途还没看。
///
/// 面板底图上还印着五灵「水 火 / 雷 风 / 土 双」六格和「经验值」，格里的数
/// （`FUN_004f39f0`）移植端没有，不画。
pub const NAME_RECT: Rect = (0, 0, 187, 32);

/// 面板往下滑进来：第 k 块晚 0.08 秒起步（`FUN_005184e0` 的 `k * 0.08`），从 y = −256
/// 滑到 0。那个调用里的 3000 是**加速度**：每帧的处理 `LAB_00518fe0` 算
/// `y = 起点 + 0.5 × 3000 × t²`，到终点就停（`__ftol` 截断）。
pub const SLIDE_DELAY: f32 = 0.08;
pub const SLIDE_ACCEL: f32 = 3000.;
pub fn panel_slide_y(panel: usize, elapsed: f32) -> i32 {
    let t = (elapsed - panel as f32 * SLIDE_DELAY).max(0.);
    (-256. + 0.5 * SLIDE_ACCEL * t * t).min(0.) as i32
}

/// 升级的横幅（`FUN_00517df0`，id 0x213）。
pub const LEVELUP_PANEL: &str = "result/panel/levelup.tga";
pub const LEVELUP_RECT: Rect = (284, 268, 516, 332);
/// 仙术升级、学会五灵仙术时的横幅（`FUN_00517df0` id 0x215..0x21b），按仙术在仙术库里
/// 的行：0 水 1 火 2 雷 3 风 4 土 5 双（跳表 0x502c90 → 控件 `+0x448/44c/444/440/450/454`）。
pub const ELEMENT_BANNERS: [&str; 6] = [
    "result/panel/waterlvup.tga",
    "result/panel/firelvup.tga",
    "result/panel/thunderlvup.tga",
    "result/panel/windlvup.tga",
    "result/panel/earthlvup.tga",
    "result/panel/doublelvup.tga",
];
pub const ELEMENT_BANNER_RECT: Rect = (249, 263, 551, 337);

/// 仙术在仙术库（战斗位 `+0x2e8`，6 行 × 8）里的行。库由 `FUN_0040ec20` 排：按 gdb 次序取
/// 本人能用的 `SkillType` 1/2/3，数五灵里值**等于 1** 的个数——一个就进那一行（水 0、火 1、
/// 风 3、雷 2、土 4），两个以上进第 5 行「双」，一个都没有的不进；排完每行按 id 冒泡升序。
/// `elements` 是 `SkillInfo::element_attributes`（水火风雷土）。
pub fn magic_row(elements: &[i32; 5]) -> Option<u8> {
    let on: Vec<usize> = (0..5).filter(|i| elements[*i] == 1).collect();
    match on.as_slice() {
        [] => None,
        // 水 火 风 雷 土 → 行 0 1 3 2 4
        [e] => Some([0, 1, 3, 2, 4][*e]),
        _ => Some(5),
    }
}

/// 面板上六个数（控件 0x202..0x207，白字 16px），按仙术库的行排：数值是那一行 8 格仙术
/// 使用次数的和（`FUN_004f39f0` 加战斗位 `+0x3d8 + 行*8` 的 8 个字节）。
pub const ELEMENT_NUMBER_RECTS: [Rect; 6] = [
    (51, 108, 115, 140),
    (136, 108, 200, 140),
    (51, 134, 115, 166),
    (136, 134, 200, 166),
    (51, 160, 115, 192),
    (136, 160, 200, 192),
];

/// 得到钱 / 物品的横幅（id 0x214）：`FUN_00517df0` 建在 `+0x458`，(221,255)-(579,345)，挂 getitem.tga。
pub const GETITEM_PANEL: &str = "result/panel/getitem.tga";
pub const GETITEM_RECT: Rect = (221, 255, 579, 345);

// ---------------------------------------------------------------- 文字

/// `datascript\lang.txt` 里结算要用的几个词（`FUN_00430570` 按键名取）。
#[derive(Debug, Clone, PartialEq)]
pub struct ResultLang {
    /// `Level` = 级
    pub level: String,
    /// `Levelup` = 升为
    pub levelup: String,
    /// `GetItem` = 得到
    pub get_item: String,
    /// `Money` = 文钱
    pub money: String,
    /// `Mastered` = 学会
    pub mastered: String,
}

pub const LANG_PATH: &str = "/basedata/basedata/datascript/lang.txt";

/// `lang.txt` 里一个键的值（一行 `键<tab…>$值&`，`FUN_00430570`）。
pub fn lang_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        (line.split_whitespace().next() == Some(key))
            .then(|| {
                let (a, b) = (line.find('$')?, line.rfind('&')?);
                (a < b).then(|| line[a + 1..b].to_string())
            })
            .flatten()
    })
}

impl ResultLang {
    /// 解析 `lang.txt`：一行 `键<tab…>$值&`。缺的键用实机的值兜底。
    pub fn parse(text: &str) -> Self {
        let mut map = std::collections::HashMap::new();
        for line in text.lines() {
            let Some(key) = line.split_whitespace().next() else {
                continue;
            };
            let (Some(a), Some(b)) = (line.find('$'), line.rfind('&')) else {
                continue;
            };
            if a < b {
                map.insert(key.to_string(), line[a + 1..b].to_string());
            }
        }
        let get = |k: &str, d: &str| map.get(k).cloned().unwrap_or_else(|| d.to_string());
        Self {
            level: get("Level", "级"),
            levelup: get("Levelup", "升为"),
            get_item: get("GetItem", "得到"),
            money: get("Money", "文钱"),
            mastered: get("Mastered", "学会"),
        }
    }

    /// `%s%s %d %s`（0x5db778）：名字 + 升为 + 等级 + 级。
    pub fn level_up(&self, name: &str, level: i32) -> String {
        format!("{name}{} {level} {}", self.levelup, self.level)
    }
    /// `%s%s %s`（0x5db7b4，`LAB_00502950`）：名字 + 学会 + 技能名。
    pub fn mastered(&self, name: &str, skill: &str) -> String {
        format!("{name}{} {skill}", self.mastered)
    }
    /// `%s %s %s %d %s`（0x5db7c8，`LAB_00502cc0`）：名字 + 仙术名 + 升为 + 等级 + 级。
    pub fn skill_up(&self, name: &str, skill: &str, level: i32) -> String {
        format!("{name} {skill} {} {level} {}", self.levelup, self.level)
    }
    /// `%s %d %s`（0x5db798）：得到 + 数 + 文钱。
    pub fn money(&self, amount: i32) -> String {
        format!("{} {amount} {}", self.get_item, self.money)
    }
    /// 一个用 `%s %s`（0x5b0bcc），多个用 `%s %s %d`（0x5db78c）。
    pub fn item(&self, name: &str, count: i32) -> String {
        if count == 1 {
            format!("{} {name}", self.get_item)
        } else {
            format!("{} {name} {count}", self.get_item)
        }
    }
}

// ---------------------------------------------------------------- 时间轴

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageKind {
    LevelUp,
    /// 升级学会特技（`LAB_00502950`）。
    Mastered,
    /// 仙术升级（`LAB_00502cc0`）。
    SkillUp,
    Money,
    Item,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResultMessage {
    pub kind: MessageKind,
    pub text: String,
    /// 学会 / 仙术升级用哪块五灵横幅（[`ELEMENT_BANNERS`] 的行）；`None` 用升级横幅——
    /// 特技（`FUN_004f69f0` 只认 `SkillType` 1..3）和不在库里的都是这样。
    pub row: Option<u8>,
}

impl ResultMessage {
    /// 登记时的时长：升级 5 秒（0x4fabec）、钱 5 秒、物品 4 秒（`FUN_004fb370`）。
    pub fn seconds(&self) -> f32 {
        match self.kind {
            MessageKind::LevelUp
            | MessageKind::Mastered
            | MessageKind::SkillUp
            | MessageKind::Money => 5.,
            MessageKind::Item => 4.,
        }
    }
    /// 显示时放的音效：升级 `wa014`（`FUN_005025c0`）。物品的 `wa006` 原版在**登记**时
    /// 就放（`FUN_004fb370` 每登记一件放一次），不在这里，见 [`item_sounds`]。钱不出声。
    pub fn sound(&self) -> Option<&'static str> {
        match self.kind {
            MessageKind::LevelUp => Some("wa014"),
            // 学会 / 仙术升级都放 wa015（0x502b55）。
            MessageKind::Mastered | MessageKind::SkillUp => Some("wa015"),
            MessageKind::Item => None,
            MessageKind::Money => None,
        }
    }
    pub fn panel(&self) -> (&'static str, Rect) {
        match self.kind {
            MessageKind::LevelUp => (LEVELUP_PANEL, LEVELUP_RECT),
            MessageKind::Mastered | MessageKind::SkillUp => match self.row {
                Some(r) => (ELEMENT_BANNERS[r as usize], ELEMENT_BANNER_RECT),
                None => (LEVELUP_PANEL, LEVELUP_RECT),
            },
            _ => (GETITEM_PANEL, GETITEM_RECT),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// 什么都不画（`LAB_00502300` 0.5 秒、`LAB_005021c0` 0.2 秒——两处都在调音乐）。
    Pause(f32),
    /// 胜 / 败大字（`FUN_00501e60` / `FUN_00501f30`），同时藏起战斗界面。
    Banner {
        win: bool,
        secs: f32,
    },
    /// 人物面板滑进来（`FUN_00502430`）；之后一直留着。
    Panels(f32),
    Message(ResultMessage),
    /// 停住等（`LAB_004fce00`）。
    Hold(f32),
    /// 渐黑（`LAB_00502000`：`FUN_004dfff0` 拉到 255）。
    Fade(f32),
}

impl Step {
    pub fn seconds(&self) -> f32 {
        match self {
            Step::Pause(s) | Step::Panels(s) | Step::Hold(s) | Step::Fade(s) => *s,
            Step::Banner { secs, .. } => *secs,
            Step::Message(m) => m.seconds(),
        }
    }
    /// 按任意键能不能跳过（这几个步骤函数都查了 `FUN_00402b00`）。
    pub fn skippable(&self) -> bool {
        matches!(
            self,
            Step::Banner { .. } | Step::Panels(_) | Step::Message(_) | Step::Hold(_)
        )
    }
    pub fn sound(&self) -> Option<&'static str> {
        match self {
            Step::Banner { win: true, .. } => Some("wa012"),
            Step::Banner { win: false, .. } => Some("wa013"),
            Step::Message(m) => m.sound(),
            _ => None,
        }
    }
}

/// 排时间轴时当场要放的音效：每件物品一次 `wa006`（`FUN_004fb370`，0x4fb3d5 / 0x4fb42d）。
pub fn item_sounds(messages: &[ResultMessage]) -> usize {
    messages
        .iter()
        .filter(|m| m.kind == MessageKind::Item)
        .count()
}

/// 三种结局各排什么（`FUN_004f7910`）。
pub fn timeline(outcome: Outcome, messages: Vec<ResultMessage>) -> Vec<Step> {
    let msgs = messages.into_iter().map(Step::Message);
    match outcome {
        // 任务组 0x17：0x502300(0.5) → 0x501e60(3) → 0x502430(2) → 升级 → 钱物
        // → 0x4fce00(5) → 0x5021c0(0.2) → 0x502000(1)
        Outcome::Win => [
            Step::Pause(0.5),
            Step::Banner {
                win: true,
                secs: 3.,
            },
            Step::Panels(2.),
        ]
        .into_iter()
        .chain(msgs)
        .chain([Step::Hold(5.), Step::Pause(0.2), Step::Fade(1.)])
        .collect(),
        // phase 4：0x502430(2) → 升级 → 钱物 → 0x4fce00(15)
        Outcome::Escaped => [Step::Panels(2.)]
            .into_iter()
            .chain(msgs)
            .chain([Step::Hold(15.)])
            .collect(),
        // 任务组 0x19：0x502300(0.5) → 0x501f30(5) → 0x502000(1) → 0x5021c0（时长没读，按 0.2）
        Outcome::Lose => vec![
            Step::Pause(0.5),
            Step::Banner {
                win: false,
                secs: 5.,
            },
            Step::Fade(1.),
            Step::Pause(0.2),
        ],
        Outcome::Ongoing => vec![],
    }
}

/// 按顺序播时间轴。
#[derive(Debug, Clone)]
pub struct ResultPlayer {
    steps: Vec<Step>,
    index: usize,
    t: f32,
    panels_since: Option<f32>,
    started: bool,
}

impl ResultPlayer {
    pub fn new(steps: Vec<Step>) -> Self {
        Self {
            steps,
            index: 0,
            t: 0.,
            panels_since: None,
            started: false,
        }
    }

    pub fn finished(&self) -> bool {
        self.index >= self.steps.len()
    }

    pub fn current(&self) -> Option<&Step> {
        self.steps.get(self.index)
    }

    /// 走 `dt` 秒；`skip` 是这一帧按了键。返回这一帧新开始的那一步（给它放声音）。
    pub fn update(&mut self, dt: f32, skip: bool) -> Option<Step> {
        if let Some(p) = self.panels_since.as_mut() {
            *p += dt;
        }
        let mut begun = None;
        if !self.started {
            self.started = true;
            begun = self.begin();
        }
        let Some(step) = self.steps.get(self.index) else {
            return begun;
        };
        self.t += dt;
        if self.t >= step.seconds() || (skip && step.skippable()) {
            self.index += 1;
            self.t = 0.;
            begun = self.begin().or(begun);
        }
        begun
    }

    fn begin(&mut self) -> Option<Step> {
        let step = self.steps.get(self.index)?.clone();
        if matches!(step, Step::Panels(_)) && self.panels_since.is_none() {
            self.panels_since = Some(0.);
        }
        Some(step)
    }

    /// 胜 / 败大字该不该画。
    pub fn banner(&self) -> Option<bool> {
        match self.current() {
            Some(Step::Banner { win, .. }) => Some(*win),
            _ => None,
        }
    }
    /// 人物面板从出现到现在多久；`None` = 还没出来。
    pub fn panels_elapsed(&self) -> Option<f32> {
        self.panels_since
    }
    pub fn message(&self) -> Option<&ResultMessage> {
        match self.current() {
            Some(Step::Message(m)) => Some(m),
            _ => None,
        }
    }
    /// 渐黑到多黑（0..1）；渐黑之后的步骤保持全黑。
    pub fn fade(&self) -> Option<f32> {
        let fade_at = self.steps.iter().position(|s| matches!(s, Step::Fade(_)))?;
        if self.index < fade_at {
            return None;
        }
        if self.index == fade_at {
            let secs = self.steps[fade_at].seconds();
            return Some((self.t / secs).clamp(0., 1.));
        }
        Some(1.)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lang() -> ResultLang {
        ResultLang::parse(
            "Level\t\t\t$级&\nLevelup\t\t\t$升为&\nGetItem\t\t\t$得到&\nMoney\t\t\t$文钱&\n",
        )
    }

    #[test]
    fn messages_read_like_the_original_format_strings() {
        let l = lang();
        assert_eq!(l.level_up("景天", 5), "景天升为 5 级");
        assert_eq!(l.money(120), "得到 120 文钱");
        assert_eq!(l.item("止血草", 1), "得到 止血草");
        assert_eq!(l.item("止血草", 3), "得到 止血草 3");
        // 缺的键用实机值兜底。
        assert_eq!(ResultLang::parse(""), l);
    }

    #[test]
    fn a_win_runs_banner_panels_messages_hold_fade() {
        let msgs = vec![
            ResultMessage {
                kind: MessageKind::LevelUp,
                text: "x".into(),
                row: None,
            },
            ResultMessage {
                kind: MessageKind::Item,
                text: "y".into(),
                row: None,
            },
        ];
        let t = timeline(Outcome::Win, msgs);
        let secs: Vec<f32> = t.iter().map(|s| s.seconds()).collect();
        assert_eq!(secs, vec![0.5, 3., 2., 5., 4., 5., 0.2, 1.]);
        assert_eq!(t[1].sound(), Some("wa012"));
        assert_eq!(t[3].sound(), Some("wa014"));
    }

    #[test]
    fn escape_skips_the_banner_and_holds_longer() {
        let t = timeline(Outcome::Escaped, vec![]);
        assert_eq!(t, vec![Step::Panels(2.), Step::Hold(15.)]);
        let lose = timeline(Outcome::Lose, vec![]);
        assert_eq!(
            lose[1],
            Step::Banner {
                win: false,
                secs: 5.
            }
        );
        assert_eq!(lose[1].sound(), Some("wa013"));
    }

    #[test]
    fn the_player_steps_through_and_a_key_skips_what_can_be_skipped() {
        let mut p = ResultPlayer::new(vec![
            Step::Pause(0.5),
            Step::Banner {
                win: true,
                secs: 3.,
            },
            Step::Hold(5.),
        ]);
        assert_eq!(
            p.update(0.1, false),
            Some(Step::Pause(0.5)),
            "第一帧报第一步"
        );
        assert_eq!(p.update(0.1, true), None, "停顿跳不过");
        assert_eq!(
            p.update(0.4, false),
            Some(Step::Banner {
                win: true,
                secs: 3.
            })
        );
        assert_eq!(p.banner(), Some(true));
        assert_eq!(p.update(0.01, true), Some(Step::Hold(5.)), "大字按键跳过");
        assert_eq!(p.banner(), None);
        p.update(5.1, false);
        assert!(p.finished());
    }

    #[test]
    fn panels_stay_after_their_step_and_slide_in_staggered() {
        let mut p = ResultPlayer::new(vec![Step::Panels(2.), Step::Hold(5.)]);
        p.update(0., false);
        p.update(2.1, false);
        assert!(matches!(p.current(), Some(Step::Hold(_))));
        assert!(p.panels_elapsed().unwrap() > 2.);
        assert_eq!(panel_slide_y(0, 0.), -256);
        assert_eq!(panel_slide_y(0, 1.), 0);
        assert!(
            panel_slide_y(3, 0.24) < panel_slide_y(0, 0.24),
            "后面的晚走"
        );
    }

    #[test]
    fn the_fade_ramps_then_stays_black() {
        let mut p = ResultPlayer::new(vec![Step::Hold(1.), Step::Fade(1.), Step::Pause(0.2)]);
        p.update(0., false);
        assert_eq!(p.fade(), None);
        p.update(1.0, false);
        p.update(0.5, false);
        assert!((p.fade().unwrap() - 0.5).abs() < 1e-5);
        p.update(0.6, false);
        assert_eq!(p.fade(), Some(1.));
    }

    #[test]
    fn the_layout_is_the_one_read_from_the_exe() {
        assert_eq!(panel_rect(3), (600, 0, 856, 256));
        // 三条都是 85×12。
        for r in GAUGE_RECTS {
            assert_eq!((r.2 - r.0, r.3 - r.1), (85, 12));
        }
        // 结算条的填充区满值宽：(x2+4−3) − (x1−2+3) = 85。
        assert_eq!(
            super::super::hud::full_fill_width(GAUGE_RECTS[0], GAUGE_PADDING),
            85
        );
    }

    #[test]
    fn a_skill_sits_in_the_row_of_its_element() {
        assert_eq!(magic_row(&[1, 0, 0, 0, 0]), Some(0), "水");
        assert_eq!(magic_row(&[0, 1, 0, 0, 0]), Some(1), "火");
        assert_eq!(magic_row(&[0, 0, 1, 0, 0]), Some(3), "风");
        assert_eq!(magic_row(&[0, 0, 0, 1, 0]), Some(2), "雷");
        assert_eq!(magic_row(&[0, 0, 0, 0, 1]), Some(4), "土");
        assert_eq!(magic_row(&[1, 0, 0, 0, 1]), Some(5), "双");
        assert_eq!(magic_row(&[0; 5]), None);
    }

    #[test]
    fn mastered_and_skill_up_read_like_the_original() {
        let lang = ResultLang::parse("");
        assert_eq!(lang.mastered("景天", "洒金笺"), "景天学会 洒金笺");
        assert_eq!(lang.skill_up("雪见", "风咒", 2), "雪见 风咒 升为 2 级");
        let m = ResultMessage {
            kind: MessageKind::SkillUp,
            text: String::new(),
            row: Some(3),
        };
        assert_eq!(m.panel().0, "result/panel/windlvup.tga");
        assert_eq!(m.sound(), Some("wa015"));
    }
}
