//! 宿营小游戏（SCE 106 `ENCAMP_Entry`）的规则部分：转盘的扇区表、指针和
//! 进度条的步进、结算公式、版面常量。不碰渲染。
//!
//! 逆向笔记：`switch/re/notes/encamp.md`。

pub const DIR: &str = "SubGame/Encampment";
pub const SCENE_POL: &str = "SubGame/Encampment/Encamp_scene/Encamp_Scene.pol";
pub const RMK_PATH: &str = "SubGame/Encampment/Encamp.rmk";
pub const RMK_SECTION: &str = "sec1";
pub const ASSIS_LANG_PATH: &str = "AssisLang.lang";
pub const ASSIS_LANG_SECTION: &str = "sec1";

/// 五个角色槽：队伍序 → 模型目录（`FUN_0052b3b0`）。
pub const ROLE_MODELS: [&str; 5] = ["101", "104", "105", "107", "102"];
/// 宿营里的动作：动作表 `PTR_DAT_00570904` 第 10 项。
pub const ROLE_ACTION: &str = "c09";
/// 五个槽的位置（`FUN_0047d030`，原版世界单位）。
pub const ROLE_POSITIONS: [(f32, f32, f32); 5] = [
    (106., 0., -204.),
    (4., 0., -167.),
    (-89., 0., -196.),
    (-86., 0., -276.),
    (89., 0., -270.),
];
/// 五个槽绕 Y 的朝向（度，actor+0x38）。
pub const ROLE_YAWS: [f32; 5] = [256., 170., 116., 80., 288.];

/// 方位号 → 旗子 pol（`FUN_0052b3b0` 的 switch）；0–3 之外没有旗子。
pub fn flag_pol(site: i32) -> Option<String> {
    let n = match site {
        0 => "WZ07",
        1 => "WZ08",
        2 => "WZ09",
        3 => "WZ10",
        _ => return None,
    };
    Some(format!("{DIR}/EncampFlag/{n}/{n}.pol"))
}

/// 相机（`FUN_0047c640` / `FUN_0047c830(1)`）。
pub mod camera {
    /// gbCamera 的 FOV 整角（度）。
    pub const FOV_DEG: f32 = 50.;
    pub const TARGET: (f32, f32, f32) = (0., 20., 0.);
    pub const DIST: f32 = 700.;
    pub const YAW_DEG: f32 = 603.;
    pub const PITCH_DEG: f32 = -12.;

    /// `dir = (sin yaw·cos pitch, sin pitch, −cos pitch·cos yaw)`，
    /// `eye = target − dist·dir`。
    pub fn eye() -> (f32, f32, f32) {
        let (yaw, pitch) = (YAW_DEG.to_radians(), PITCH_DEG.to_radians());
        let dir = (
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            -pitch.cos() * yaw.cos(),
        );
        (
            TARGET.0 - DIST * dir.0,
            TARGET.1 - DIST * dir.1,
            TARGET.2 - DIST * dir.2,
        )
    }
}

pub mod lang {
    pub const JING: usize = 14;
    /// 64 景天、65 雪见、66 龙葵、67 紫萱、68 长卿。
    pub const NAMES: usize = 64;
    pub const ALL_FULL: usize = 154;
    pub const FAVOR: usize = 155;
    pub const RECOVER: usize = 156;
    pub const ALL: usize = 185;
}

pub mod sfx {
    /// 进场循环。
    pub const AMBIENT: &str = "wa032";
    /// 转盘转动循环。
    pub const SPIN: &str = "wa033";
    /// 景天/长卿大满贯。
    pub const FULL: &str = "wd003";
    /// 女角大满贯（好感 +10，与 43 FavorAdd 的 4–5 档同音）。
    pub const FAVOR: &str = "wa009";
    /// 结算循环里每人一声。
    pub const RECOVER: &str = "wd001";
    /// 对话框「是」。
    pub const OK: &str = "wa001";
}

// ---------- 指针 ----------

/// 指针速度上限（度/帧，`+0x27d4`）。
pub const POINTER_MAX_SPEED: f32 = 5.0;
/// 每秒加减多少速度（`dt·36.6·0.08`）。
pub const POINTER_ACCEL_PER_SEC: f32 = 36.6 * 0.08;

/// 转盘指针：角度按帧减（`FUN_00442ff0` 模式 2 方向 1），速度按时间加减。
#[derive(Debug, Clone, PartialEq)]
pub struct Pointer {
    /// 度，[0, 360)。
    pub angle: f32,
    /// 度/帧。
    pub speed: f32,
    pub accelerating: bool,
}

impl Pointer {
    pub fn new() -> Self {
        Self {
            angle: 0.,
            speed: 0.,
            accelerating: true,
        }
    }

    /// 走一帧。返回这一帧之后是否停住（`+0x5cc`：speed ≤ 0）。
    pub fn step(&mut self, delta_sec: f32) -> bool {
        let dv = delta_sec * POINTER_ACCEL_PER_SEC;
        if self.accelerating {
            self.speed = (self.speed + dv).min(POINTER_MAX_SPEED);
        } else {
            self.speed = (self.speed - dv).max(0.);
        }
        // 控件更新：方向 1，角度减，< 0 归到 359.0
        self.angle -= self.speed;
        if self.angle < 0. {
            self.angle = 359.0;
        }
        self.speed <= 0.
    }

    pub fn start_braking(&mut self) {
        self.accelerating = false;
    }
}

impl Default for Pointer {
    fn default() -> Self {
        Self::new()
    }
}

/// 指针停下的角度 → 恢复百分比（`FUN_0052c480`）。`f = 360 − angle`：
/// f ≤ 180 → 0；≤ 270 → 30；≤ 318.6 → 50；≤ 354.2 → 75；≤ 359 → 100；
/// 再大保持上一次的值。
pub fn sector_percent(angle: f32, previous: f32) -> f32 {
    let f = 360. - angle;
    if f <= 180. {
        0.
    } else if f <= 270. {
        30.
    } else if f <= 318.6 {
        50.
    } else if f <= 354.2 {
        75.
    } else if f <= 359. {
        100.
    } else {
        previous
    }
}

// ---------- 进度条 ----------

/// 76 px 的条扣掉边距后的轨道长（`+0x78`）。
pub const BAR_TRACK_PX: f32 = 70.;
/// 每秒走多少像素（`dt·36.0·speed`，speed = 1.0）。
pub const BAR_PX_PER_SEC: f32 = 36.;

/// 原版进度条控件（`FUN_0043f7f0`）里和数值有关的那部分。
#[derive(Debug, Clone, PartialEq)]
pub struct Bar {
    pub max: i32,
    /// 目标像素。
    target_px: f32,
    /// 当前像素。
    cur_px: f32,
    /// 到达后的那一帧为真（`+0xa0`）。
    pub arrived: bool,
}

impl Bar {
    pub fn new(max: i32) -> Self {
        Self {
            max: max.max(1),
            target_px: 0.,
            cur_px: 0.,
            arrived: false,
        }
    }

    fn scale(&self) -> f32 {
        BAR_TRACK_PX / self.max as f32
    }

    /// `FUN_0043fdf0`：设目标值。
    pub fn set_target(&mut self, value: i32) {
        self.target_px = value as f32 * self.scale();
    }

    /// `FUN_0043fe90`：当前和目标都回到 0。
    pub fn reset(&mut self) {
        self.target_px = 0.;
        self.cur_px = 0.;
        self.arrived = false;
    }

    pub fn step(&mut self, delta_sec: f32) {
        if self.cur_px == self.target_px {
            self.arrived = true;
            return;
        }
        self.arrived = false;
        let d = delta_sec * BAR_PX_PER_SEC;
        if self.cur_px < self.target_px {
            self.cur_px = (self.cur_px + d).min(self.target_px);
        } else {
            self.cur_px = (self.cur_px - d).max(self.target_px);
        }
    }

    /// 已填充的像素数（0..=70）。
    pub fn fill_px(&self) -> f32 {
        self.cur_px
    }

    /// 显示值 `ROUND(cur·max/70)`。
    pub fn display_value(&self) -> i32 {
        (self.cur_px * self.max as f32 / BAR_TRACK_PX).round() as i32
    }
}

// ---------- 结算 ----------

/// 结算要读写的角色数值，全部按队伍序（0 景天 … 4 长卿）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RoleStats {
    pub hp: i32,
    pub max_hp: i32,
    pub sp: i32,
    pub max_sp: i32,
    pub mp: i32,
    pub max_mp: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settlement {
    /// 对话框文字（`\n` 已经是真正的换行）。
    pub text: String,
    /// 大满贯的那个人和对应音效；没有则 `None`。
    pub jackpot: Option<(usize, &'static str)>,
    /// 好感 +10 的对象（女角大满贯）。
    pub favor_plus_ten: Option<usize>,
}

/// `FUN_0052d200(k)`：四个头像槽全是 k 且 k 在队。
pub fn jackpot(heads: [usize; 4], present: &[bool; 5]) -> Option<usize> {
    let k = heads[0];
    (heads.iter().all(|h| *h == k) && present.get(k).copied().unwrap_or(false)).then_some(k)
}

/// 大满贯的处理方式。
pub fn is_full_restore_role(k: usize) -> bool {
    k == 0 || k == 4
}

/// `ftol(pct·0.01·max + cur)`，再按上限截断（`FUN_0052d230`）。
fn restore(cur: i32, max: i32, pct: f32) -> i32 {
    let v = (pct * 0.01 * max as f32 + cur as f32).trunc() as i32;
    clamp_stat(v, max)
}

fn clamp_stat(v: i32, max: i32) -> i32 {
    if v < max { v.max(0) } else { max }
}

/// 结算 `FUN_0052d230`。`stats` 按队伍序，不在队的不动；`lang` 是
/// AssisLang 表。返回文字和音效提示，数值原地改。
pub fn settle(
    stats: &mut [RoleStats; 5],
    present: &[bool; 5],
    heads: [usize; 4],
    pct: f32,
    lang: &[String],
) -> Settlement {
    let s = |i: usize| lang.get(i).map(String::as_str).unwrap_or("");
    let name = |k: usize| s(lang::NAMES + k).to_string();

    let mut text = String::new();
    let mut result = Settlement {
        text: String::new(),
        jackpot: None,
        favor_plus_ten: None,
    };
    if let Some(k) = jackpot(heads, present) {
        if is_full_restore_role(k) {
            let r = &mut stats[k];
            r.hp = r.max_hp;
            r.sp = r.max_sp;
            r.mp = r.max_mp;
            text.push_str(&format!("\n          {}{}", name(k), s(lang::ALL_FULL)));
            result.jackpot = Some((k, sfx::FULL));
        } else {
            let r = &mut stats[k];
            r.hp = restore(r.hp, r.max_hp, pct);
            text.push_str(&format!("\n            {}{}+10", name(k), s(lang::FAVOR)));
            result.jackpot = Some((k, sfx::FAVOR));
            result.favor_plus_ten = Some(k);
        }
    }
    for (r, on) in stats.iter_mut().zip(present) {
        if !*on {
            continue;
        }
        r.hp = restore(r.hp, r.max_hp, pct);
        r.sp = clamp_stat(r.sp, r.max_sp);
        r.mp = clamp_stat(r.mp, r.max_mp);
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&format!(
        "          {}{}{}{}%",
        s(lang::ALL),
        s(lang::RECOVER),
        s(lang::JING),
        pct.trunc() as i32
    ));
    result.text = text;
    result
}

// ---------- 版面（800×600） ----------

pub mod layout {
    pub type Rect = (i32, i32, i32, i32);

    pub const SCREEN_W: i32 = 800;
    pub const SCREEN_H: i32 = 600;

    pub const REMARK: Rect = (125, 172, 675, 428);
    /// 说明文字：说明牌内缩 0x44 / 0x32。
    pub const REMARK_TEXT: Rect = (125 + 0x44, 172 + 0x32, 675 - 0x44, 428 - 0x32);
    pub const TURNTABLE: Rect = (200, 160, 618, 578);
    pub const POINTER: Rect = (383, 272, 431, 464);
    /// 上、左、下、右四个头像槽。
    pub const HEADS: [Rect; 4] = [
        (370, 160, 448, 238),
        (202, 336, 280, 414),
        (370, 496, 448, 574),
        (538, 336, 616, 414),
    ];
    /// 结果对话框（kind 0，带「是」钮）。
    pub const RESULT_DIALOG: Rect = (246, 245, 446, 345);
    /// 说明牌之后的过场框（kind 3，按任意键）。
    pub const WAIT_DIALOG: Rect = (250, 200, 550, 328);

    /// 第 i 块结果面板的 x（`FUN_0052d0f0`）。第五块 x = 808 在屏幕外，原版如此。
    pub const PANEL_X: [i32; 5] = [0, 0xca, 0x194, 0x25e, 0x328];
    pub const PANEL_W: i32 = 0xc0;
    pub const PANEL_H: i32 = 0x100;

    /// 面板里的元件，相对面板左上角。
    pub const PANEL_NAME_FRAME: Rect = (0x2a, 2, 0x98, 0x22);
    pub const PANEL_HEAD: Rect = (0x3a, 0x21, 0x7b, 0x77);
    pub const PANEL_JING_TEXT: Rect = (0x28, 0x84, 0x3e, 0x97);
    pub const PANEL_JING_BAR: Rect = (0x38, 0x8a, 0x84, 0x96);
    pub const PANEL_JING_VALUE: Rect = (0x42, 0x74, 0x92, 0x94);
    pub const PANEL_SHEN_TEXT: Rect = (0x28, 0x9e, 0x3e, 0xb1);
    pub const PANEL_SHEN_BAR: Rect = (0x38, 0xa4, 0x84, 0xb0);
    pub const PANEL_SHEN_VALUE: Rect = (0x42, 0x8e, 0x92, 0xae);

    /// 进度条控件的边距：左 2、右 4（2 + 2）。
    pub const BAR_INSET_L: i32 = 2;
    pub const BAR_INSET_R: i32 = 4;

    pub const fn offset(r: Rect, dx: i32, dy: i32) -> Rect {
        (r.0 + dx, r.1 + dy, r.2 + dx, r.3 + dy)
    }

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
    fn camera_eye_matches_hand_calculation() {
        let (x, y, z) = camera::eye();
        assert!((x - 610.0).abs() < 1.0, "{x}");
        assert!((y - 165.5).abs() < 0.5, "{y}");
        assert!((z + 310.9).abs() < 1.0, "{z}");
    }

    #[test]
    fn flag_only_for_four_sites() {
        assert_eq!(
            flag_pol(1).as_deref(),
            Some("SubGame/Encampment/EncampFlag/WZ08/WZ08.pol")
        );
        assert!(flag_pol(4).is_none());
        assert!(flag_pol(-1).is_none());
    }

    #[test]
    fn sectors() {
        // 12 点左边那道红
        // 边界都是 `f ≤ 阈值`，所以 5.8/41.4/90/180 这几个角度本身落在低一档。
        assert_eq!(sector_percent(3., 0.), 100.);
        assert_eq!(sector_percent(5.7, 0.), 100.);
        assert_eq!(sector_percent(5.9, 0.), 75.);
        assert_eq!(sector_percent(41.3, 0.), 75.);
        assert_eq!(sector_percent(41.5, 0.), 50.);
        assert_eq!(sector_percent(89.9, 0.), 50.);
        assert_eq!(sector_percent(90.1, 0.), 30.);
        assert_eq!(sector_percent(179.9, 0.), 30.);
        assert_eq!(sector_percent(180.1, 0.), 0.);
        assert_eq!(sector_percent(359., 0.), 0.);
        // (0, 1) 保持上一次
        assert_eq!(sector_percent(0.5, 75.), 75.);
        assert_eq!(sector_percent(1., 0.), 100.);
    }

    #[test]
    fn pointer_turns_clockwise_and_wraps_to_359() {
        let mut p = Pointer::new();
        // 60 fps 下加到 5°/帧要 5 / (2.928/60) ≈ 102 帧
        let mut frames = 0;
        while p.speed < POINTER_MAX_SPEED {
            p.step(1. / 60.);
            frames += 1;
        }
        assert!((100..=105).contains(&frames), "{frames}");
        // 角度往下走，过 0 回到 359
        p.angle = 2.;
        p.speed = POINTER_MAX_SPEED;
        assert!(!p.step(1. / 60.));
        assert_eq!(p.angle, 359.0);

        p.start_braking();
        let mut stopped = false;
        for _ in 0..200 {
            if p.step(1. / 60.) {
                stopped = true;
                break;
            }
        }
        assert!(stopped);
        assert_eq!(p.speed, 0.);
    }

    #[test]
    fn bar_moves_36px_per_second_and_reports_arrival_once() {
        let mut b = Bar::new(140);
        b.set_target(70); // 一半 = 35 px
        b.step(0.5); // 18 px
        assert!((b.fill_px() - 18.).abs() < 1e-4);
        assert_eq!(b.display_value(), 36);
        assert!(!b.arrived);
        b.step(0.5); // 到 35 px 封顶
        assert!((b.fill_px() - 35.).abs() < 1e-4);
        assert!(!b.arrived);
        b.step(0.1);
        assert!(b.arrived);
        assert_eq!(b.display_value(), 70);
    }

    fn lang() -> Vec<String> {
        let mut v = vec![String::new(); 200];
        v[lang::JING] = "精".into();
        for (i, n) in ["景天", "雪见", "龙葵", "紫萱", "长卿"].iter().enumerate() {
            v[lang::NAMES + i] = n.to_string();
        }
        v[lang::ALL_FULL] = "精气神全满".into();
        v[lang::FAVOR] = "好感".into();
        v[lang::RECOVER] = "恢复".into();
        v[lang::ALL] = "全体".into();
        v
    }

    fn stats() -> [RoleStats; 5] {
        std::array::from_fn(|i| RoleStats {
            hp: 10 + i as i32,
            max_hp: 100,
            sp: 5,
            max_sp: 50,
            mp: 200,
            max_mp: 60,
        })
    }

    #[test]
    fn plain_settlement_restores_percent_of_max_hp() {
        let mut s = stats();
        let present = [true, true, false, false, true];
        let r = settle(&mut s, &present, [0, 1, 2, 3], 30., &lang());
        assert_eq!(r.jackpot, None);
        assert_eq!(r.text, "          全体恢复精30%");
        assert_eq!(s[0].hp, 40);
        assert_eq!(s[1].hp, 41);
        assert_eq!(s[2].hp, 12, "不在队的不动");
        assert_eq!(s[4].hp, 44);
        // 超上限的当前值被截断
        assert_eq!(s[0].mp, 60);
        assert_eq!(s[2].mp, 200);
    }

    #[test]
    fn jingtian_jackpot_fills_everything() {
        let mut s = stats();
        let present = [true, true, true, true, true];
        let r = settle(&mut s, &present, [0, 0, 0, 0], 75., &lang());
        assert_eq!(r.jackpot, Some((0, "wd003")));
        assert_eq!(r.favor_plus_ten, None);
        assert_eq!(
            r.text,
            "\n          景天精气神全满\n          全体恢复精75%"
        );
        assert_eq!((s[0].hp, s[0].sp, s[0].mp), (100, 50, 60));
        assert_eq!(s[1].hp, 86);
    }

    #[test]
    fn girl_jackpot_gets_hp_twice_and_favor() {
        let mut s = stats();
        let present = [true, true, true, true, true];
        let r = settle(&mut s, &present, [2, 2, 2, 2], 50., &lang());
        assert_eq!(r.jackpot, Some((2, "wa009")));
        assert_eq!(r.favor_plus_ten, Some(2));
        assert_eq!(r.text, "\n            龙葵好感+10\n          全体恢复精50%");
        // 12 + 50 = 62，再 + 50 = 100（封顶）
        assert_eq!(s[2].hp, 100);
        assert_eq!(s[1].hp, 61);
    }

    #[test]
    fn jackpot_needs_presence() {
        assert_eq!(
            jackpot([3, 3, 3, 3], &[true, true, true, false, true]),
            None
        );
        assert_eq!(
            jackpot([3, 3, 3, 3], &[true, true, true, true, true]),
            Some(3)
        );
        assert_eq!(jackpot([3, 3, 1, 3], &[true; 5]), None);
    }
}
