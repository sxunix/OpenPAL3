//! 107 滑雪 — `LAB_00436f40`（模式 11）。
//!
//! 规则在 `openpal3::states::ski`（照 `FUN_00531260` / `FUN_0052ed30` /
//! `FUN_00532410` 写的，见 `switch/re/notes/ski.md`）。这里负责把实机的
//! 数据喂进去、跑一局、结算，最后跑参数指定的收尾过程。
//!
//! 操作照原版：**按住鼠标左键加速，点右键跳**，没有左右操控。Esc 弃权。
//!
//! 画面照原版：景天踩着剑滑下 `m22-6.pol` 的 3D 雪道（[`crate::openpal3::ski_view`]，命令开始时
//! 把这个场景压到场景栈顶、结束时弹掉），右上角是计时牌（`Skee_ui`），雪球追上来时右边闪警告。

use std::path::PathBuf;

use std::rc::Rc;

use crate::openpal3::ski_view::SkiView;
use crate::openpal3::states::ski::{
    Aabb, OBSTACLE_COUNT, Outcome, Shapes, SkiInput, SkiRun, TIME_LIMIT_SECS, TREASURE_COUNT,
};
use crate::openpal3::states::ski_path::SkiPath;
use crate::scripting::sce::commands::appraise::overlay;
use crate::scripting::sce::commands::encamp::draw_sprite;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{MouseButton, Ui};
use radiance::comdef::ISceneManager;
use radiance::input::Key;
use radiance::rendering::Sprite;

/// exe 里写死的路径（`s_basedata_Skee_path_ase_005f4d34`）；根目录和
/// `SkeeScene/Scene/` 下各有一份，内容一样。
const PATH_CANDIDATES: [&str; 2] = [
    "/basedata/basedata/Skee_path.ASE",
    "/basedata/basedata/SubGame/Skee/SkeeScene/Scene/Skee_path.ASE",
];

const BALL_POL: &str = "/basedata/basedata/SubGame/Skee/SkeeScene/Ball/Ball.POL";
const OBSTACLE_DIR: &str = "/basedata/basedata/SubGame/Skee/SkeeScene/Obstruct";
/// 人物（角色 101）的动作文件：开局量身高用 c01，滑 h02、跳 h05 的盒子做碰撞。
const MAN_DIR: &str = "/basedata/basedata/ROLE/101";

/// 三件能捡的古董。**写死在 exe 里**（`FUN_0052f6c0` 开头把
/// `0x1783 / 0x178d / 0x1781` 填进 `+0x8a4`），不是从 `skee_sav.sv` 读的
/// ——那个文件内容一样，但代码没读它。
///
/// 6019 鹿纹瓦当 / 6029 刻辞整甲 / 6017 盘龙古砚，都是古董（type 4）。
pub const TREASURE_IDS: [u32; 3] = [6019, 6029, 6017];

/// 滑完之后让玩家看几秒结算。
const RESULT_SECONDS: f32 = 3.0;

const TEXT: [f32; 4] = [1., 1., 1., 1.];

/// `Skee_ui::Create` 的界面（800×600）：计时牌、两位数字、雪球来了的警告。
const TIME_BK_RECT: (i32, i32, i32, i32) = (681, 0, 800, 92);
const TIME_TEN_RECT: (i32, i32, i32, i32) = (729, 16, 761, 40);
const TIME_ONE_RECT: (i32, i32, i32, i32) = (729, 40, 761, 64);
const WARNING_RECT: (i32, i32, i32, i32) = (644, 200, 700, 264);
/// 警告两张图每 160 毫秒换一次（`Skee_ui::Update`）。
const WARNING_FLIP_SECS: f32 = 0.16;
const TIME_BK: &str = "/basedata/basedata/SubGame/Skee/SkeeTimeBk.tga";
const WARNING_A: &str = "/basedata/basedata/SubGame/Skee/sb0.tga";
const WARNING_B: &str = "/basedata/basedata/SubGame/Skee/sb1.tga";
/// 数字借鉴定小游戏的（`SubGame\Appraise\Number\%d.tga`）。
const DIGIT_DIR: &str = "/basedata/basedata/SubGame/Appraise/Number";

/// 界面用到的图。
struct Hud {
    time_bk: Option<Rc<Sprite>>,
    digits: Vec<Option<Rc<Sprite>>>,
    warning: [Option<Rc<Sprite>>; 2],
}

/// 滑雪的场景和界面；不进 `Clone`，命令复制出去时不带（命令只在虚拟机里跑一份）。
#[derive(Default)]
struct Live {
    view: Option<SkiView>,
    pushed: bool,
    hud: Option<Hud>,
    warning_clock: f32,
}

impl Clone for Live {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for Live {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Live")
            .field("pushed", &self.pushed)
            .finish()
    }
}

#[derive(Debug, Clone)]
pub struct SceCommandMiniGameSki {
    end_game_proc: i32,
    run: Option<SkiRun>,
    /// 数据没读到就退回原来那个「跳过」的走法。
    unavailable: bool,
    result_shown: f32,
    settled: bool,
    live: Live,
}

impl SceCommandMiniGameSki {
    pub fn new(end_game_proc: i32) -> Self {
        Self {
            end_game_proc,
            run: None,
            unavailable: false,
            result_shown: 0.,
            settled: false,
            live: Live::default(),
        }
    }

    fn start(&mut self, state: &mut SceState) {
        let vfs = state.asset_mgr().vfs();

        let Some(path) = PATH_CANDIDATES.iter().find_map(|p| {
            let bytes = read_bytes(vfs, p)?;
            let text = encoding::Encoding::decode(
                encoding::all::GBK,
                &bytes,
                encoding::DecoderTrap::Ignore,
            )
            .ok()?;
            let path = SkiPath::parse(&text);
            (!path.points.is_empty()).then_some(path)
        }) else {
            log::warn!("Ski: 读不到 Skee_path.ASE，这一段跳过");
            self.unavailable = true;
            return;
        };

        // 古董用物品自己的模型（`FUN_0052f8e0`：`Item\%s\%s.pol`，`%s` 是物品记录 +0x24 的模型名）。
        let models: Vec<Option<String>> = TREASURE_IDS
            .iter()
            .map(|&id| {
                state.global_state().game_db().and_then(|db| {
                    db.item(id)
                        .and_then(|i| i.model_name.as_str().ok())
                        .filter(|m| !m.is_empty())
                })
            })
            .collect();

        // 碰撞盒都从模型文件量；读不到的用实机量好的兜底。
        let fallback = Shapes::shipped();
        let man = |act: &str| mv3_box(vfs, &format!("{MAN_DIR}/{act}.MV3"));
        let shapes = Shapes {
            man_height: man("C01").map_or(fallback.man_height, |b| b.size[1]),
            man_slide: man("H02").unwrap_or(fallback.man_slide),
            man_jump: man("H05").unwrap_or(fallback.man_jump),
            obstacles: std::array::from_fn::<_, OBSTACLE_COUNT, _>(|i| {
                pol_box(vfs, &format!("{OBSTACLE_DIR}/obstruct{i}.POL"))
                    .unwrap_or(fallback.obstacles[i])
            }),
            treasures: std::array::from_fn::<_, TREASURE_COUNT, _>(|i| {
                models
                    .get(i)
                    .and_then(|m| m.as_ref())
                    .and_then(|m| pol_box(vfs, &format!("/basedata/basedata/item/{m}/{m}.pol")))
                    .unwrap_or(fallback.treasures[i])
            }),
            ball: pol_box(vfs, BALL_POL).unwrap_or(fallback.ball),
        };

        // 原版每件先查一次背包，已经有的就不摆（`FUN_0052ece0`）。
        let owned: Vec<bool> = {
            let p_state = state.global_state().persistent_state();
            TREASURE_IDS
                .iter()
                .map(|&id| p_state.item_count(id) > 0)
                .collect()
        };

        // 摆放点是 `rand15() % 点数 + 100`。
        let mut rng = crate::openpal3::states::shop::ShopRng::new(
            state.global_state().persistent_state().money() as u32 ^ 0x5EE5_0107,
        );
        let rand: Vec<u32> = (0..TREASURE_IDS.len())
            .map(|_| rng.rand15() as u32)
            .collect();

        let run = SkiRun::new(path, shapes, &TREASURE_IDS, &owned, &rand);
        let asset_mgr = state.asset_mgr().clone();
        self.live.view = SkiView::new(&asset_mgr, &run, &models, rng.rand15() as u32);
        if self.live.view.is_none() {
            log::warn!("Ski: 3D 场景建不起来，只画界面");
        }
        let sprite = |p: &str| asset_mgr.load_sprite(PathBuf::from(p));
        self.live.hud = Some(Hud {
            time_bk: sprite(TIME_BK),
            digits: (0..10)
                .map(|i| sprite(&format!("{DIGIT_DIR}/{i}.tga")))
                .collect(),
            warning: [sprite(WARNING_A), sprite(WARNING_B)],
        });
        self.run = Some(run);
    }

    /// `FUN_00532410`：滑完且还剩至少一秒才把收集品塞进背包。
    fn settle(&mut self, state: &mut SceState) {
        if self.settled {
            return;
        }
        self.settled = true;

        let Some(run) = self.run.as_ref() else {
            return;
        };
        let score = run.score();
        if run.awards() {
            let mut p_state = state.global_state_mut().persistent_state_mut();
            for &id in run.picked() {
                p_state.add_item(id, 1);
            }
            log::info!(
                "Ski: 滑完了，得 {score} 分，拿到 {:?}，剩 {} 秒",
                run.picked(),
                run.seconds_left
            );
        } else {
            log::info!(
                "Ski: 没滑完（{:?}），得 {score} 分，一件也不发",
                run.outcome()
            );
        }
    }

    /// 界面（`Skee_ui`）：右上角计时牌和剩余秒数的十位、个位；雪球追上来后（`GiveSomeShow`）右边两张
    /// 警告图轮换；滑完写结果。
    fn draw(&mut self, ui: &Ui, delta_sec: f32) {
        let Some(run) = self.run.as_ref() else {
            return;
        };
        let Some(hud) = self.live.hud.as_ref() else {
            return;
        };
        let warning = run.chasing() && run.outcome().is_none();
        if warning {
            self.live.warning_clock += delta_sec;
        }
        let flip = ((self.live.warning_clock / WARNING_FLIP_SECS) as i32 % 2) as usize;
        let secs = run.seconds_left.clamp(0, 99) as usize;
        let result = run.outcome().map(|o| match o {
            Outcome::Cleared => format!(
                "滑完了　得 {} 分（{} 件 × 100 ＋ {} 秒 × 30）",
                run.score(),
                run.picked().len(),
                run.seconds_left
            ),
            Outcome::Caught => "被后面的雪球追上了　一件也没拿到".to_string(),
            Outcome::TimedOut => "时间到　一件也没拿到".to_string(),
        });
        overlay(ui, "Ski", |ui, at, _scale| {
            let list = ui.get_window_draw_list();
            draw_sprite(&list, hud.time_bk.as_deref(), TIME_BK_RECT, &at);
            let digit = |n: usize| hud.digits.get(n).and_then(|d| d.as_deref());
            draw_sprite(&list, digit(secs / 10), TIME_TEN_RECT, &at);
            draw_sprite(&list, digit(secs % 10), TIME_ONE_RECT, &at);
            if warning {
                draw_sprite(&list, hud.warning[flip].as_deref(), WARNING_RECT, &at);
            }
            if let Some(text) = &result {
                list.add_text(at(200, 300), TEXT, text);
            }
        });
    }

    /// 结束：把滑雪场景从场景栈上弹掉。
    fn leave(&mut self, scene_manager: &ComRc<ISceneManager>) {
        if self.live.pushed {
            scene_manager.pop_scene();
            self.live.pushed = false;
        }
        self.live.view = None;
    }
}

impl SceCommand for SceCommandMiniGameSki {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if self.run.is_none() && !self.unavailable {
            self.start(state);
        }
        if !self.live.pushed && !state.fast_forward() {
            if let Some(view) = &self.live.view {
                scene_manager.push_scene(view.scene.clone());
                self.live.pushed = true;
            }
        }

        // 数据缺了或者快进：照旧只跑收尾过程。
        if self.unavailable || state.fast_forward() {
            self.leave(&scene_manager);
            self.settle(state);
            if self.end_game_proc > 0 {
                state.call_proc(self.end_game_proc as u32);
            }
            return true;
        }

        // [移植] 手柄键照全代码的约定：`GamePadEast`（Switch 的 A）是确定、
        // `GamePadSouth`（B）是取消——其他界面都是空格配 East、Esc 配 South。
        // 先前这里反着：按 A 直接弃权、按住 B 才加速，主线里这一段（M22「滑雪
        // 之前」）一按 A 就丢了。改成 B 弃权、按住 A 加速（对应原版按住左键）。
        let quit = state.input().get_key_state(Key::Escape).pressed()
            || state.input().get_key_state(Key::GamePadSouth).pressed();

        let input = SkiInput {
            // 原版是 `DAT_00bfc5f4 == 1`，左键**按住**。
            boost: ui.is_mouse_down(MouseButton::Left)
                || state.input().get_key_state(Key::GamePadEast).is_down(),
            // 原版是 `(DAT_00bfc5f8 & 0xf) == 1`，右键**按下那一帧**。
            jump: ui.is_mouse_clicked(MouseButton::Right)
                || state.input().get_key_state(Key::Space).pressed()
                || state.input().get_key_state(Key::GamePadWest).pressed(),
        };

        let done = if let Some(run) = self.run.as_mut() {
            run.update(delta_sec, input);
            if let Some(view) = self.live.view.as_mut() {
                view.update(run, delta_sec);
            }
            run.outcome().is_some()
        } else {
            true
        };
        self.draw(ui, delta_sec);

        if done {
            self.result_shown += delta_sec;
        }
        if quit || (done && self.result_shown >= RESULT_SECONDS) {
            self.leave(&scene_manager);
            self.settle(state);
            // 原版滑完回冒险模式时 `FUN_00432170(DAT_00dba254, 0)`。
            if self.end_game_proc > 0 {
                state.call_proc(self.end_game_proc as u32);
            }
            return true;
        }
        false
    }
}

fn read_bytes(vfs: &mini_fs::MiniFs, path: &str) -> Option<Vec<u8>> {
    use common::store_ext::StoreExt2;
    vfs.read_to_end(PathBuf::from(path)).ok()
}

/// `.POL` 的包围盒（`gbGeomNode::GetOBBox`）：所有网格 AABB 的并。
fn pol_box(vfs: &mini_fs::MiniFs, path: &str) -> Option<Aabb> {
    let bytes = read_bytes(vfs, path)?;
    let pol = fileformats::pol::read_pol(&mut std::io::Cursor::new(bytes)).ok()?;
    let mut min = [f32::MAX; 3];
    let mut max = [f32::MIN; 3];
    for m in &pol.meshes {
        let (lo, hi) = (&m.aabb_min, &m.aabb_max);
        for (k, (l, h)) in [(lo.x, hi.x), (lo.y, hi.y), (lo.z, hi.z)]
            .into_iter()
            .enumerate()
        {
            min[k] = min[k].min(l);
            max[k] = max[k].max(h);
        }
    }
    (min[0] <= max[0]).then(|| Aabb::from_min_max(min, max))
}

/// 一个动作文件的包围盒（`gbMVActorDef::GetCollideOBB`：动作的 `GetBoundBox`，`.mv3` 头里那个）。
fn mv3_box(vfs: &mini_fs::MiniFs, path: &str) -> Option<Aabb> {
    let bytes = read_bytes(vfs, path)?;
    let mv3 = fileformats::mv3::read_mv3(&mut std::io::Cursor::new(bytes)).ok()?;
    let m = mv3.models.first()?;
    Some(Aabb::from_min_max(m.aabb_min, m.aabb_max))
}

/// 一局的时长上限，给别处引用。
pub const RUN_SECONDS: i32 = TIME_LIMIT_SECS;
