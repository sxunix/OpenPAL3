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

    // 下面这些矩形全部直接从两段初始化代码的指令里读出来：展示页
    // `FUN_00528650`、Do 页 `FUN_005257b0`。原版建控件的调用长这样
    //
    // ```text
    // lea  ecx, [esp+R]            ; R 是矩形在栈上的基址
    // push owner / push &rect / push 名字
    // mov  dword ptr [esp+R'+0], 左
    // mov  dword ptr [esp+R'+4], 上
    // mov  dword ptr [esp+R'+8], 右
    // mov  dword ptr [esp+R'+c], 下
    // call dword ptr [vtbl+0x10]   ; SetRect（按钮走 +0x1c，多带贴图名）
    // ```
    //
    // 四个 dword 依次是左、上、右、下。（`switch/re/notes/appraise_minigame.md`
    // 里早先记的几个矩形把左和上写反了，已经改过来。）
    //
    // **矩形的宽高就是图本身的大小**：包里这些 `.tga` 都是左下角为原点
    // （image descriptor 的第 5 位是 0），画面内容贴在画布的**左下角**，右边
    // 和上边用透明像素补到 2 的幂。逐张量出来的内容范围和控件矩形能对上，
    // 多处分毫不差：`TimeBarbk` 1024×128 里是 772×92，控件就是 772×92；
    // `word_type` 128×128 里满幅 128×128，控件 128×128；`Result0` 128×32 里
    // 是 94×20，控件 94×20；`ExhibitRemark` 512×128 里是 467×105，两页的控件
    // 都是 468×104。所以画的时候要取贴图**左下角**那一块，u 取
    // `0..宽/贴图宽`、v（自上而下）取 `1−高/贴图高..1`。

    // ---- 展示页（`FUN_00528650`）----

    /// 背景 `ExhibitBk.tga`（控件 `+0x2b8`，0x5287f1 一带）。
    pub const EXHIBIT_BK: Rect = (0x56, 0x8a, 0x2c8, 0x25e);
    /// 放大视图背景 `ExhibitZoom.tga`（控件 `+0x648`，0x5289c9 一带）。
    pub const EXHIBIT_ZOOM_BK: Rect = (0x56, 0x88, 0x2ca, 0x25d);
    /// 标题图 `word_type.tga`（控件 `+0x480`，0x52879e 一带）。
    pub const WORD_TYPE: Rect = (0x20, 0x0a, 0xa0, 0x8a);
    /// 屏幕两侧的竖线 `UI\GameMainUI\Communal\smt_Line.tga`，两页都有
    /// （展示页 `+0xa1c` / `+0xab4`，Do 页 `+0x16e8` / `+0x1780`）。
    pub const SIDE_LINE_L: Rect = (0, 0, 0xe, 600);
    pub const SIDE_LINE_R: Rect = (0x312, 0, 0x320, 600);
    /// 五件真品的点击格。
    pub const EXHIBIT_SLOTS: [Rect; 5] = [
        (199, 179, 337, 348),
        (464, 179, 602, 348),
        (128, 406, 266, 575),
        (334, 406, 472, 575),
        (540, 406, 678, 575),
    ];
    /// 「鉴定」按钮 0xaa0b01（`Exhibit\Do{0,1,2}.tga`）。0x528c40 的四个
    /// 立即数，走按钮的 `vtbl+0x1c`。`Do0.tga` 是 128×32 的画布，里头的图
    /// 正好 117×26，和这个矩形的 116×26 对得上。
    pub const EXHIBIT_DO_BUTTON: Rect = (0x152, 0x22c, 0x1c6, 0x246);
    /// 展示页的说明牌 `ExhibitRemark.tga`（控件 `+0x518`，0x5286bf 一带）。
    pub const REMARK: Rect = (0xa5, 0x22, 0x279, 0x8a);
    /// Do 页的说明牌（控件 `+0x1358`，0x525808 一带），比展示页的挪了 3 px。
    pub const DO_REMARK: Rect = (0xa8, 0x20, 0x27c, 0x88);
    /// 牌子上那块文字区：两页都是从牌子的矩形算出来的，
    /// 展示页 `左+0x16, 上+6` 起 428×158（0x52870b 一带），
    /// Do 页 `左+0x18, 上+6` 起同样大小（0x52585c 一带）。
    /// **比牌子本身高**——牌子只有 104 px，原版就这么写的，文字多了会漏到
    /// 牌子外面去。
    pub const REMARK_TEXT: Rect = (
        REMARK.0 + 0x16,
        REMARK.1 + 6,
        REMARK.0 + 0x16 + 0x1ac,
        REMARK.1 + 6 + 0x9e,
    );
    pub const DO_REMARK_TEXT: Rect = (
        DO_REMARK.0 + 0x18,
        DO_REMARK.1 + 6,
        DO_REMARK.0 + 0x18 + 0x1ac,
        DO_REMARK.1 + 6 + 0x9e,
    );
    /// 文物名字 0xaa0c21（Do 页控件 `+0x1488`，0x525ba1 一带）。
    pub const REMARK_NAME: Rect = (0x146, 0xaa, 0x1e2, 0xca);
    /// 鉴别要点 0xaa0c19（Do 页控件 `+0x788`，0x525fd8 一带；展示页的同名
    /// 控件 `+0x6e0` 在 0x528a74 一带，位置只差 1 px）。两个控件在屏幕上
    /// 几乎叠着，名字那条压在要点上面 6 px。
    pub const REMARK_DIFFENT: Rect = (0x146, 0xa4, 0x1e2, 0xc4);

    // ---- Do 页（`FUN_005257b0`）----

    /// Do 页背景 `doingBk.tga`（控件 `+0x658`，0x525ab4 一带）。
    pub const DO_BK: Rect = (0x5a, 0x88, 0x2ca, 0x256);
    /// Do 页放大视图背景 `Do\ExhibitZoom.tga`（控件 `+0x6f0`，0x525f09 一带）。
    pub const DO_ZOOM_BK: Rect = (0x5a, 0x88, 0x2cc, 0x25b);
    /// 时间条 `TimeBarbk.tga` / `TimeBarFo.tga`（控件 `+0x1840`，0x525984
    /// 一带）。控件字段 `+0x1914=1`、`+0x1930=300`、`+0x1928=10.0`、
    /// `+0x191c=0xff`、`+0x18b0=620.0`，前景宽度上限按
    /// [`super::Timer::bar_width`]。
    pub const TIME_BAR: Rect = (0x10, 0, 0x314, 0x5c);
    /// 剩余秒数的两位数字（控件 `+0x1520` 十位 / `+0x15b8` 个位，0x525a51
    /// 一带）。初始化先给十位 (0x40,0x12)-(0x60,0x2a)，再
    /// `下 → 上、下 += 0x18` 得个位——**两位数字是上下叠着的**，不是并排，
    /// 各 32×24，整块落在时间条左端。画数字的是
    /// `FUN_005269b0(秒, 十位控件, 个位控件, 目录, flag)`（0x5268a5 的入栈
    /// 顺序）。
    pub const SECONDS: Rect = (0x40, 0x12, 0x60, 0x2a);
    /// 个位相对十位往**下**挪这么多。
    pub const SECONDS_STEP: i32 = 0x18;
    /// 队列按钮 0xaa0c01..05：第一只 (0x78,0xa2)-(0xa0,0xd0)，往后
    /// 「左 = 上一只的右、右 = 左 + 0x28」。
    pub const fn queue_button(i: i32) -> Rect {
        (0x78 + 0x28 * i, 0xa2, 0x78 + 0x28 * i + 0x28, 0xd0)
    }
    /// 放大镜 0xaa0c28..30：第一只 (0x228,0x92)-(0x255,0xd2)，往后
    /// 「左 = 上一只的右 + 2、右 = 左 + 0x2d」。
    pub const fn magnifier(i: i32) -> Rect {
        (0x228 + 0x2f * i, 0x92, 0x255 + 0x2f * i, 0xd2)
    }
    /// 拖起来跟着鼠标走的那只放大镜（控件 `+0x1650`，0x52605e 一带，
    /// 贴 `Magnifier0.tga`）。原版放在这个固定位置，移植让它跟手。
    pub const MAGNIFIER_HELD: Rect = (0x218, 0x9e, 0x245, 0xd0);
    /// 左物 0xaa0c25 / 右物 0xaa0c26：`doingBk` 两块暗格（纹素 33..295 /
    /// 327..589 × 124..448）加背景原点。
    pub const DO_LEFT: Rect = (123, 222, 385, 546);
    pub const DO_RIGHT: Rect = (417, 222, 679, 546);
    /// Result0 / Result1 勾选框（`Result0.tga` 未选 / `Result1.tga` 已选）。
    /// 0x525ce3 给第一只，第二只是「左 = 第一只的右 + 0xc8、右 = 左 + 0x5e」
    /// （0x525d4b 一带），不是整体平移。
    pub const RESULT0: Rect = (0xd5, 0x224, 0x133, 0x238);
    pub const RESULT1: Rect = (
        RESULT0.2 + 0xc8,
        RESULT0.1,
        RESULT0.2 + 0xc8 + 0x5e,
        RESULT0.3,
    );
    /// 确定 0xaa0c18（`MUIDialog\Normal\Yes`，0x525d9a 一带）。
    pub const CONFIRM: Rect = (0x166, 0x23d, 0x1c0, 0x256);
    /// 结算 / 提示框 0xaa0c22、0xaa0c23（`UI\Flex\Note`，0x5261de 一带）。
    pub const NOTE: Rect = (0x116, 0x78, 0x216, 0x1a4);
    /// 答对 / 答错那两行字（0xabe00b、0xabc00c，0x5260c5 一带，通用控件
    /// 的 mode 3，也就是 `UI\Flex\Note` 底加 6 px 内缩的文字）。
    pub const DO_TOAST: Rect = (0x12c, 0xc8, 0x1f4, 0x12c);

    // ---- 开头的询问框 ----

    /// 阶段 0 的「要玩鉴定游戏吗？」。`FUN_00527a80` 用
    /// `FUN_004a16c0(名字, (250,200,550,300), this, 2, flag)` 建通用对话框，
    /// **mode 2 会把传进去的右下角丢掉**，改成 `右 = 左 + 0x134`、
    /// `下 = 上 + 0x6d`（0x4a1740 一带），所以真正的框是这个。
    pub const ASK_BOX: Rect = (250, 200, 250 + 0x134, 200 + 0x6d);
    /// 框里的文字区：`左+6, 上+6, 右−6, 下−0x26`。
    pub const ASK_TEXT: Rect = (
        ASK_BOX.0 + 6,
        ASK_BOX.1 + 6,
        ASK_BOX.2 - 6,
        ASK_BOX.3 - 0x26,
    );
    /// 「是」`MUIDialog\Normal\Yes` / 「否」`Normal\No`：0x4a1790 一带算的，
    /// 「是」的左 = 框左 + 0x1a，「否」的右 = 框右 − 0x1a，两个的下都是
    /// 框下 − 6，宽高取按钮贴图自己的（控件 `+0x260` / `+0x264`）。
    ///
    /// 贴图在包里取不到（`UI\GameMainUI` 不在 `basedata.cpk` 里），宽高借
    /// [`CONFIRM`] 来定：那是同一张 `MUIDialog\Normal\Yes`，同一个屏，
    /// 矩形写死 90×25；当铺的确定钮（`dealer_layout.md`）也是 90×25。
    const ASK_BTN: (i32, i32) = (CONFIRM.2 - CONFIRM.0, CONFIRM.3 - CONFIRM.1);
    pub const ASK_YES: Rect = (
        ASK_BOX.0 + 0x1a,
        ASK_BOX.3 - 6 - ASK_BTN.1,
        ASK_BOX.0 + 0x1a + ASK_BTN.0,
        ASK_BOX.3 - 6,
    );
    pub const ASK_NO: Rect = (
        ASK_BOX.2 - 0x1a - ASK_BTN.0,
        ASK_BOX.3 - 6 - ASK_BTN.1,
        ASK_BOX.2 - 0x1a,
        ASK_BOX.3 - 6,
    );

    /// 放大视图里那块暗格（`ExhibitZoom` 纹素 56..567 × 89..441 加背景
    /// 原点）。只拿来判「点在格子外面就收起放大视图」。
    pub const ZOOM_PANE: Rect = (
        EXHIBIT_ZOOM_BK.0 + 56,
        EXHIBIT_ZOOM_BK.1 + 89 - 45,
        EXHIBIT_ZOOM_BK.0 + 567,
        EXHIBIT_ZOOM_BK.1 + 441 - 45,
    );

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

    pub fn contains(r: Rect, x: f32, y: f32) -> bool {
        x >= r.0 as f32 && x < r.2 as f32 && y >= r.1 as f32 && y < r.3 as f32
    }

    pub fn center(r: Rect) -> (f32, f32) {
        ((r.0 + r.2) as f32 * 0.5, (r.1 + r.3) as f32 * 0.5)
    }

    /// 一张贴图里真正有画面的那块：左下角起 `宽 × 高`，返回自上而下的
    /// 像素范围 `(x, y, 宽, 高)`，用来从 2 的幂画布里裁出来。
    pub const fn art_in_canvas(canvas_h: u32, rect: Rect) -> (u32, u32, u32, u32) {
        let w = (rect.2 - rect.0) as u32;
        let h = (rect.3 - rect.1) as u32;
        (0, canvas_h.saturating_sub(h), w, h)
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
