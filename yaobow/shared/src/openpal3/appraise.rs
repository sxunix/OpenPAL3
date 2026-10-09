//! 鉴定小游戏的规则（SCE 104 `APPREntry` / 108 `GetAppr`）——五对文物、
//! 计时、计分、结算文本、版面常量。画面和输入在
//! [`crate::scripting::sce::commands::appraise`]。
//!
//! 逆向笔记：`switch/re/notes/appraise_minigame.md`。

/// 一对文物：(真品 id, 赝品 id)，`FUN_005294c0` 里写死的五对。
pub const PAIRS: [(u32, u32); 5] = [
    (6025, 6301),
    (6021, 6302),
    (6023, 6303),
    (6010, 6304),
    (6001, 6305),
];

pub const ITEM_COUNT: usize = PAIRS.len();

/// 模型目录：`SubGame\Appraise\Exhibit\Data\%s.pol`，`%s` 是 gdb 的 `model_name`。
pub const MODEL_DIR: &str = "SubGame/Appraise/Exhibit/Data";
pub const EXHIBIT_DIR: &str = "SubGame/Appraise/Exhibit";
pub const DO_DIR: &str = "SubGame/Appraise/Do";
pub const NUMBER_DIR: &str = "SubGame/Appraise/Number";
/// 鉴别要点：`Appraise.dft` 的 `Diffent0..4`。
pub const DFT_PATH: &str = "SubGame/Appraise/Exhibit/Appraise.dft";
pub const DFT_SECTION: &str = "sec1";
pub const ASSIS_LANG_PATH: &str = "AssisLang.lang";
pub const ASSIS_LANG_SECTION: &str = "sec1";
/// 阶段 1 起播放的音乐。
pub const MUSIC: &str = "PI23";

/// `AssisLang.lang` 里用到的条目（按出现顺序编号）。
pub mod lang {
    pub const NONE: usize = 74;
    pub const ASK: usize = 81;
    pub const TIME_LEFT: usize = 82;
    pub const MAGNIFIERS_LEFT: usize = 83;
    pub const SUCCESS: usize = 84;
    pub const TOTAL_SCORE: usize = 85;
    pub const ITEMS_GAINED: usize = 86;
    pub const FAILED: usize = 195;
}

/// 音效。
pub mod sfx {
    /// 拖起放大镜。
    pub const MAGNIFIER: &str = "wa001";
    /// 开头的询问框。
    pub const ASK: &str = "wa003";
    /// 点了已经做过的题。
    pub const ALREADY_DONE: &str = "wa004";
    /// 答错。
    pub const WRONG: &str = "wa034";
    /// 五件全对。
    pub const ALL_CORRECT: &str = "wa036";
}

/// 放大镜数（`+0x3aac`）。
pub const MAGNIFIERS: i32 = 3;
/// 每只没用掉的放大镜折 40 分。
pub const MAGNIFIER_SCORE: i32 = 40;

/// 计时器（`FUN_00526740`）：从 1:00 起，每秒 `sec--`，`sec < 0` 时借位，
/// `min < 0` 就超时。画面只显示秒。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timer {
    pub min: i32,
    pub sec: i32,
}

impl Timer {
    pub const START: Timer = Timer { min: 1, sec: 0 };

    /// 过了一秒。返回 true 表示超时。
    pub fn tick(&mut self) -> bool {
        self.sec -= 1;
        if self.sec < 0 {
            self.sec = 59;
            self.min -= 1;
        }
        self.min < 0
    }

    /// 时间条宽：`(sec + 2) * 10`，上限 620。
    pub fn bar_width(&self) -> i32 {
        ((self.sec + 2) * 10).min(620)
    }
}

/// 一局的进度（Do 页 `+0x3aa0..+0x3abe` 那几个字段）。
#[derive(Debug, Clone)]
pub struct Round {
    /// 当前题。
    pub current: usize,
    /// `+0x3aa5[5]`
    pub done: [bool; ITEM_COUNT],
    /// `+0x3ab0[5]`
    pub correct: [bool; ITEM_COUNT],
    /// `+0x3ab8`：真品在左 (0) 还是右 (1)。
    pub side: usize,
    pub magnifiers: i32,
    pub timer: Timer,
    /// `+0x3abe`：五件全对才是 1。
    pub result: bool,
}

impl Round {
    pub fn new(first_side: usize) -> Self {
        Self {
            current: 0,
            done: [false; ITEM_COUNT],
            correct: [false; ITEM_COUNT],
            side: first_side & 1,
            magnifiers: MAGNIFIERS,
            timer: Timer::START,
            result: false,
        }
    }

    /// `FUN_005270c0(i)` 的判定：还有没做的题、并且第 i 题没做过才能选。
    pub fn can_pick(&self, i: usize) -> Result<(), PickError> {
        if self.done.iter().all(|d| *d) {
            return Err(PickError::NothingLeft);
        }
        if self.done[i] {
            return Err(PickError::AlreadyDone);
        }
        Ok(())
    }

    /// 换到第 i 题，真品重新随机左右（`FUN_00527130`）。
    pub fn pick(&mut self, i: usize, side: usize) -> Result<(), PickError> {
        self.can_pick(i)?;
        self.current = i;
        self.side = side & 1;
        Ok(())
    }

    /// `FUN_005273d0`：选了左边 (`chose_left`) 且真品在左，或选了右边且真品在右。
    pub fn is_correct(&self, chose_left: bool) -> bool {
        (chose_left && self.side == 0) || (!chose_left && self.side == 1)
    }

    /// 答对了：记下，跳到下一道没做的题（`FUN_00527510`）。返回是否五件全对。
    pub fn mark_correct(&mut self) -> bool {
        self.done[self.current] = true;
        self.correct[self.current] = true;
        if self.done.iter().all(|d| *d) {
            self.result = true;
            return true;
        }
        if let Some(next) = (0..ITEM_COUNT).find(|i| !self.done[*i]) {
            self.current = next;
        }
        false
    }

    /// 答错：记下、结果清零，游戏结束。
    pub fn mark_wrong(&mut self) {
        self.done[self.current] = true;
        self.correct[self.current] = false;
        self.result = false;
    }

    pub fn use_magnifier(&mut self) -> bool {
        if self.magnifiers <= 0 {
            return false;
        }
        self.magnifiers -= 1;
        true
    }

    /// 答对的件数。
    pub fn correct_count(&self) -> usize {
        (0..ITEM_COUNT)
            .filter(|i| self.done[*i] && self.correct[*i])
            .count()
    }

    /// `FUN_00527610` 的计分：`score = 0x50; 每件对的 score = (score + i*10) * 2`，
    /// 再加剩余放大镜 × 40。
    pub fn score(&self) -> i32 {
        let mut score = 0x50;
        for i in 0..ITEM_COUNT {
            if self.done[i] && self.correct[i] {
                score = (score + i as i32 * 10) * 2;
            }
        }
        score + self.magnifiers * MAGNIFIER_SCORE
    }

    /// 答对的那几对的赝品 id——结算后各加一件进背包。
    pub fn rewards(&self) -> Vec<u32> {
        (0..ITEM_COUNT)
            .filter(|i| self.done[*i] && self.correct[*i])
            .map(|i| PAIRS[i].1)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickError {
    /// 一道没做的都不剩了。
    NothingLeft,
    /// 这道已经做过（放 `wa004`）。
    AlreadyDone,
}

/// 结算文本（`FUN_00527610`）。`lang` 是 AssisLang 表，`fake_names` 是五件赝品名，
/// `from_adventure` 对应 `+0x6cc`——从冒险模式进来才列奖品。
///
/// 格式串（0x5e90cc / 0x5e90b4 / 0x5e909c / 0x5e9090 / 0x5d0f44）里的换行和
/// AssisLang 一样是 `\n` 两个字符，交给提示框时再
/// [`unescape`](crate::openpal3::assis_lang::unescape)。
pub fn result_text(
    round: &Round,
    lang: &[String],
    fake_names: &[String; ITEM_COUNT],
    from_adventure: bool,
) -> String {
    let l = |i: usize| lang.get(i).map(String::as_str).unwrap_or("");
    let mut text = format!(
        "\\n\\n        {}:   {}\\n        {}: {}\\n        {}:   {}\\n        {}:     {}\\n",
        l(lang::TIME_LEFT),
        round.timer.sec,
        l(lang::MAGNIFIERS_LEFT),
        round.magnifiers,
        l(lang::SUCCESS),
        round.correct_count(),
        l(lang::TOTAL_SCORE),
        round.score()
    );
    if from_adventure {
        if round.correct_count() < 1 {
            text.push_str(&format!(
                "\\n\\n        {}:   {}",
                l(lang::ITEMS_GAINED),
                l(lang::NONE)
            ));
        } else {
            text.push_str(&format!("\\n\\n        {}: \\n\\n", l(lang::ITEMS_GAINED)));
            for i in 0..ITEM_COUNT {
                if round.done[i] && round.correct[i] {
                    text.push_str("        ");
                    text.push_str(&fake_names[i]);
                    text.push_str("\\n");
                }
            }
        }
    }
    text
}

/// 版面（800×600）。
pub mod layout {
    /// 一个 (x0, y0, x1, y1) 矩形。
    pub type Rect = (i32, i32, i32, i32);

    pub const SCREEN_W: i32 = 800;
    pub const SCREEN_H: i32 = 600;

    /// Exhibit 页背景 `ExhibitBk.tga`。
    pub const EXHIBIT_BK: Rect = (0x88, 0x56, 0x2ca, 0x25d);
    /// 五件真品的点击格。
    pub const EXHIBIT_SLOTS: [Rect; 5] = [
        (199, 179, 337, 348),
        (464, 179, 602, 348),
        (128, 406, 266, 575),
        (334, 406, 472, 575),
        (540, 406, 678, 575),
    ];
    /// 三张 1024×512 背景（`ExhibitBk` / `doingBk` / `ExhibitZoom`）按 1:1 贴到
    /// 屏幕时的原点：把五个点击格和 `ExhibitBk` 里五块暗格的纹素范围对上，
    /// 左边缘全部吻合、右边缘差 ≤4 px，得出 (90, 98)。三张图的有内容范围
    /// 都是 x 0..626、y 45..511，Do 页沿用同一原点是 `[推断]`（Result0 正好
    /// 落在左格下方、Normal_Y 正好在两格中线上，和这个原点自洽）。
    pub const BK_ORIGIN: (i32, i32) = (90, 98);
    /// 「鉴定」按钮 0xaa0b01（`Do0.tga` 128×32）`[推断]` 位置——原版由布局
    /// 脚本给，这里放在背景右下角外侧。
    pub const EXHIBIT_DO_BUTTON: Rect = (600, 560, 728, 592);
    /// 说明牌 `ExhibitRemark.tga`（512×128）在 Do 页的控件矩形
    /// (0xa8,0x20)-(0x27c,0x88)；Exhibit 页沿用 `[推断]`。
    pub const REMARK: Rect = (0xa8, 0x20, 0x27c, 0x88);
    /// 说明牌上的文字区（控件 0xaa0c21 (0x146,0xaa)-(0x1e2,0xca) 是名字，
    /// 0xaa0c19 (0xa4,0xc4)-(0x1e2,0x146) 是说明）`[推断]` 归到牌子里。
    pub const REMARK_NAME: Rect = (0xa8 + 16, 0x20 + 12, 0x27c - 16, 0x20 + 36);
    pub const REMARK_TEXT: Rect = (0xa8 + 16, 0x20 + 40, 0x27c - 16, 0x88 - 8);
    /// 放大视图里的暗格（`ExhibitZoom` 纹素 56..567 × 89..441 加原点）。
    pub const ZOOM_PANE: Rect = (146, 187, 657, 539);

    /// 原版 3D 控件的一台正交相机（`FUN_00439740` 建、`FUN_0043a070` 给
    /// 尺度、`FUN_00439850` 给位置）。
    ///
    /// `gbCamera` 的 FOV 为 0 就是正交（`SetAsCurrent` 0x10021a8e 的分支），
    /// 这时半宽 = `OrthoSize`、半高 = 半宽 × 视口高/宽（`GetViewSizeOnNearPlane`
    /// 0x10021a53）。所以横竖的像素/世界单位是同一个值 `SCREEN_W / (2·ortho)`。
    ///
    /// `FUN_00439850(a, b)` 把相机摆到世界 (cam_x, cam_y, 1600) 看向
    /// (cam_x, cam_y, 0)、up = +Y：
    ///
    /// ```text
    /// cam_x = −(2a/宽 − 1) · ortho
    /// cam_y = −(1 − 2b/高) · ortho
    /// ```
    ///
    /// 注意纵向这一行用的是 `ortho` 而不是半高，原版就是这么算的，照抄。
    #[derive(Debug, Clone, Copy)]
    pub struct View3d {
        pub ortho: f32,
        a: f32,
        b: f32,
    }

    impl View3d {
        const fn new(ortho: f32, a: f32, b: f32) -> Self {
            Self { ortho, a, b }
        }

        /// 相机在世界里的 x/y。
        pub fn cam(&self) -> (f32, f32) {
            (
                -(2. * self.a / SCREEN_W as f32 - 1.) * self.ortho,
                -(1. - 2. * self.b / SCREEN_H as f32) * self.ortho,
            )
        }

        /// 一个世界单位占几个虚拟像素。
        pub fn scale(&self) -> f32 {
            SCREEN_W as f32 / (2. * self.ortho)
        }

        /// 世界 (wx, wy) 落在虚拟屏幕的哪个像素。
        pub fn to_screen(&self, wx: f32, wy: f32) -> (f32, f32) {
            let k = self.scale();
            let (cx, cy) = self.cam();
            (
                SCREEN_W as f32 * 0.5 + (wx - cx) * k,
                SCREEN_H as f32 * 0.5 - (wy - cy) * k,
            )
        }
    }

    /// 展示页五件（控件 0xaa0b08）。
    pub const VIEW_EXHIBIT: View3d = View3d::new(1180., 400., 300.);
    /// 展示页的放大视图（控件 0xaa0b09）。
    pub const VIEW_EXHIBIT_ZOOM: View3d = View3d::new(560., 400., 360.);
    /// Do 页左右两件（控件 0xaa0c25 / 0xaa0c26）。
    pub const VIEW_DO: View3d = View3d::new(600., 400., 320.);
    /// Do 页的放大视图（控件 0xaa0c27）。
    pub const VIEW_DO_ZOOM: View3d = View3d::new(580., 400., 346.);

    /// 展示页五件的世界坐标（状态对象 `+0x8ac` 起的五个 vec3）。
    pub const EXHIBIT_POS: [(f32, f32); 5] = [
        (-380., 136.),
        (380., 136.),
        (-620., -520.),
        (0., -520.),
        (620., -520.),
    ];
    /// Do 页左右两件的世界坐标（状态对象 `+0x1340` 起的两个 vec3）。
    /// 真品放第 `side` 项，赝品放另一项。
    pub const DO_POS: [(f32, f32); 2] = [(-220., -80.), (220., -80.)];

    /// 自转速度，度/秒。3D 控件 `+0xec` 为 1 就是持续自转，速度取 `+0xf4`
    /// 的整数值（0x439b0e 那支：`角度 += (int)[+0xf4] · dt`，满 360 归零）；
    /// 鉴定的三处都写 0x3a = 58（0x525e62 / 0x525ecf / 0x528966）。
    /// 两个放大视图没调过 `FUN_0043a090`，`+0xec` 是 0，不自转。
    pub const SPIN_DEG_PER_SEC: f32 = 58.;

    /// Do 页背景 `doingBk.tga`。
    pub const DO_BK: Rect = (0x88, 0x5a, 0x2ca, 0x256);
    /// 3D 视图控件 0xaa0c27。
    pub const DO_VIEW: Rect = (0x88, 0x5a, 0x2cc, 0x25b);
    /// 队列按钮 0xaa0c01..05：x = 0x78 + 0x28·i，40×46。
    pub const fn queue_button(i: i32) -> Rect {
        (0x78 + 0x28 * i, 0xa2, 0x78 + 0x28 * i + 0x28, 0xd0)
    }
    /// 放大镜 0xaa0c28..30：第一只 (0x228,0x92)-(0x255,0xd2)，往后每只
    /// x 加 0x2f、中间空 2 px。
    pub const fn magnifier(i: i32) -> Rect {
        (0x228 + 0x2f * i, 0x92, 0x255 + 0x2f * i, 0xd2)
    }
    /// 左物 0xaa0c25 / 右物 0xaa0c26：`doingBk` 两块暗格（纹素 33..295 /
    /// 327..589 × 124..448）加原点。
    pub const DO_LEFT: Rect = (123, 222, 385, 546);
    pub const DO_RIGHT: Rect = (417, 222, 679, 546);
    /// Result0 / Result1 勾选框（`Result0.tga` 未选 / `Result1.tga` 已选）。
    pub const RESULT0: Rect = (0xd5, 0x224, 0x133, 0x238);
    pub const RESULT1: Rect = (0xd5 + 200, 0x224, 0x133 + 200, 0x238);
    /// 确定 Normal_Y。
    pub const CONFIRM: Rect = (0x166, 0x23d, 0x1c0, 0x256);
    /// 秒数两位数字：初始化里的矩形 (0x40,0x12)-(0x60,0x2a) 和之后的
    /// `x += 0x18`，按「十位、个位各 24 px 宽」理解 `[推断]`。
    pub const SECONDS: Rect = (0x40, 0x12, 0x60, 0x2a);
    pub const SECONDS_STEP: i32 = 0x18;
    /// 时间条 `[推断]`：接在数字右边，宽度按 [`super::Timer::bar_width`]。
    pub const TIME_BAR: Rect = (0x78, 0x14, 0x78 + 620, 0x28);
    /// 结算 / 提示框 `UI\Flex\Note`。
    pub const NOTE: Rect = (0x116, 0x78, 0x216, 0x1a4);
    /// 开头的询问框和「是 / 否」`[推断]`：原版是通用对话框控件，位置没查，
    /// 这里居中。
    pub const ASK_BOX: Rect = (200, 230, 600, 370);
    pub const ASK_YES: Rect = (290, 310, 370, 350);
    pub const ASK_NO: Rect = (430, 310, 510, 350);

    pub fn contains(r: Rect, x: f32, y: f32) -> bool {
        x >= r.0 as f32 && x < r.2 as f32 && y >= r.1 as f32 && y < r.3 as f32
    }

    pub fn center(r: Rect) -> (f32, f32) {
        ((r.0 + r.2) as f32 * 0.5, (r.1 + r.3) as f32 * 0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_gives_sixty_one_seconds() {
        let mut t = Timer::START;
        let mut ticks = 0;
        while !t.tick() {
            ticks += 1;
            assert!(ticks < 200);
        }
        assert_eq!(ticks, 60);
        assert_eq!((t.min, t.sec), (-1, 59));
        // 第一秒之后画面显示 59
        let mut t = Timer::START;
        t.tick();
        assert_eq!((t.min, t.sec), (0, 59));
        assert_eq!(t.bar_width(), 610);
        assert_eq!(Timer { min: 0, sec: 60 }.bar_width(), 620);
    }

    #[test]
    fn score_formula() {
        // 一件都没对：0x50 + 3 × 40
        let r = Round::new(0);
        assert_eq!(r.score(), 80 + 120);

        // 五件全对、放大镜没用：((((80·2+10)·2+20)·2+30)·2+40)·2
        let mut r = Round::new(0);
        for _ in 0..ITEM_COUNT {
            r.mark_correct();
        }
        let mut s = 0x50;
        for i in 0..5 {
            s = (s + i * 10) * 2;
        }
        assert_eq!(s, 3080);
        assert_eq!(r.score(), 3080 + 120);
        assert!(r.result);
        assert_eq!(r.rewards(), vec![6301, 6302, 6303, 6304, 6305]);
    }

    #[test]
    fn one_wrong_ends_it() {
        let mut r = Round::new(1);
        assert!(!r.is_correct(true));
        assert!(r.is_correct(false));
        assert!(!r.mark_correct());
        assert_eq!(r.current, 1);
        r.pick(3, 0).unwrap();
        assert_eq!(r.current, 3);
        r.mark_wrong();
        assert!(!r.result);
        assert_eq!(r.correct_count(), 1);
        assert_eq!(r.rewards(), vec![6301]);
        assert_eq!(r.can_pick(3), Err(PickError::AlreadyDone));
        assert_eq!(r.can_pick(0), Err(PickError::AlreadyDone));
        assert_eq!(r.can_pick(2), Ok(()));
    }

    #[test]
    fn result_text_shape() {
        let mut lang: Vec<String> = vec![String::new(); 200];
        lang[lang::TIME_LEFT] = "剩余时间".into();
        lang[lang::MAGNIFIERS_LEFT] = "剩余放大镜".into();
        lang[lang::SUCCESS] = "鉴定成功".into();
        lang[lang::TOTAL_SCORE] = "总分数".into();
        lang[lang::ITEMS_GAINED] = "获得物品".into();
        lang[lang::NONE] = "无".into();
        let names = std::array::from_fn(|i| format!("J{}", i + 1));

        let r = Round::new(0);
        assert_eq!(
            result_text(&r, &lang, &names, true),
            "\\n\\n        剩余时间:   0\\n        剩余放大镜: 3\\n        鉴定成功:   0\\n        总分数:     200\\n\\n\\n        获得物品:   无"
        );

        let mut r = Round::new(0);
        r.mark_correct();
        r.use_magnifier();
        let t = result_text(&r, &lang, &names, true);
        assert!(
            t.ends_with("        获得物品: \\n\\n        J1\\n"),
            "{t:?}"
        );
        assert!(t.contains("剩余放大镜: 2"));
        assert!(!result_text(&r, &lang, &names, false).contains("获得物品"));
    }
}
