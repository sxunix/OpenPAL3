//! 107 滑雪 — `LAB_00436f40`（模式 11）。
//!
//! 规则在 `openpal3::states::ski`（照 `FUN_00531260` / `FUN_0052ed30` /
//! `FUN_00532410` 写的，见 `switch/re/notes/ski.md`）。这里负责把实机的
//! 数据喂进去、跑一局、结算，最后跑参数指定的收尾过程。
//!
//! 操作照原版：**按住鼠标左键加速，点右键跳**，没有左右操控。Esc 弃权。
//!
//! 画面是 800×600 虚拟坐标上的一条示意雪道（进度条式），不是原版那个 3D
//! 的滑坡——`m22-6.POL` 的场景通路还没接。

use std::path::PathBuf;

use crate::openpal3::states::ski::{
    BOOST_MAX, CRUISE_SPEED, Outcome, Phase, Radii, SkiInput, SkiRun, TIME_LIMIT_SECS,
};
use crate::openpal3::states::ski_path::SkiPath;
use crate::scripting::sce::commands::appraise::overlay;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{MouseButton, Ui};
use radiance::comdef::ISceneManager;
use radiance::input::Key;

/// exe 里写死的路径（`s_basedata_Skee_path_ase_005f4d34`）；根目录和
/// `SkeeScene/Scene/` 下各有一份，内容一样。
const PATH_CANDIDATES: [&str; 2] = [
    "/basedata/basedata/Skee_path.ASE",
    "/basedata/basedata/SubGame/Skee/SkeeScene/Scene/Skee_path.ASE",
];

const BALL_POL: &str = "/basedata/basedata/SubGame/Skee/SkeeScene/Ball/Ball.POL";
const OBSTACLE_POL: &str = "/basedata/basedata/SubGame/Skee/SkeeScene/Obstruct/obstruct0.POL";
const TREASURE_POL: &str = "/basedata/basedata/SubGame/Skee/SkeeScene/Treasure/Treasure0.POL";

/// 三件能捡的古董。**写死在 exe 里**（`FUN_0052f6c0` 开头把
/// `0x1783 / 0x178d / 0x1781` 填进 `+0x8a4`），不是从 `skee_sav.sv` 读的
/// ——那个文件内容一样，但代码没读它。
///
/// 6019 鹿纹瓦当 / 6029 刻辞整甲 / 6017 盘龙古砚，都是古董（type 4）。
pub const TREASURE_IDS: [u32; 3] = [6019, 6029, 6017];

/// 滑完之后让玩家看几秒结算。
const RESULT_SECONDS: f32 = 3.0;

const TEXT: [f32; 4] = [1., 1., 1., 1.];
const DIM: [f32; 4] = [0.65, 0.7, 0.8, 1.];
const SLOPE: [f32; 4] = [0.82, 0.88, 0.95, 1.];
const BALL: [f32; 4] = [1., 1., 1., 1.];
const CHASER: [f32; 4] = [0.75, 0.3, 0.25, 1.];
const OBSTACLE: [f32; 4] = [0.35, 0.25, 0.18, 1.];
const TREASURE: [f32; 4] = [1., 0.84, 0.2, 1.];

/// 雪道画在这一段虚拟坐标里。
const TRACK_LEFT: i32 = 60;
const TRACK_RIGHT: i32 = 740;
const TRACK_Y: i32 = 380;

#[derive(Debug, Clone)]
pub struct SceCommandMiniGameSki {
    end_game_proc: i32,
    run: Option<SkiRun>,
    /// 数据没读到就退回原来那个「跳过」的走法。
    unavailable: bool,
    result_shown: f32,
    settled: bool,
}

impl SceCommandMiniGameSki {
    pub fn new(end_game_proc: i32) -> Self {
        Self {
            end_game_proc,
            run: None,
            unavailable: false,
            result_shown: 0.,
            settled: false,
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

        // 碰撞半径从模型自带的包围盒来。实机 obstruct0..5 和 Treasure0..7
        // 是同一份 `.POL` 的十四份拷贝，所以读一个就够。
        let radii = Radii {
            ball: pol_extent(vfs, BALL_POL).unwrap_or(Radii::SHIPPED.ball),
            obstacle: pol_extent(vfs, OBSTACLE_POL).unwrap_or(Radii::SHIPPED.obstacle),
            treasure: pol_extent(vfs, TREASURE_POL).unwrap_or(Radii::SHIPPED.treasure),
            // 追球那一处原版是 `gbOBBox::GetRadius`。
            chaser: pol_radius(vfs, BALL_POL).unwrap_or(Radii::SHIPPED.chaser),
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

        self.run = Some(SkiRun::new(path, radii, &TREASURE_IDS, &owned, &rand));
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

    fn draw(&self, ui: &Ui) {
        let Some(run) = self.run.as_ref() else {
            return;
        };
        let len = run.path().length().max(1.);
        let span = (TRACK_RIGHT - TRACK_LEFT) as f32;
        let at_x = |d: f32| TRACK_LEFT + (d.clamp(0., len) / len * span) as i32;

        overlay(ui, "Ski", |ui, at, _scale| {
            let list = ui.get_window_draw_list();
            list.add_rect(at(0, 0), at(800, 600), [0.05, 0.08, 0.14, 0.75])
                .filled(true)
                .build();

            // 雪道。
            list.add_rect(at(TRACK_LEFT, TRACK_Y), at(TRACK_RIGHT, TRACK_Y + 6), SLOPE)
                .filled(true)
                .build();

            for o in run.obstacles() {
                let x = at_x(o.distance);
                list.add_rect(at(x - 4, TRACK_Y - 22), at(x + 4, TRACK_Y), OBSTACLE)
                    .filled(true)
                    .build();
            }
            for t in run.treasures() {
                if !t.present {
                    continue;
                }
                let x = at_x(t.distance);
                list.add_rect(at(x - 5, TRACK_Y - 74), at(x + 5, TRACK_Y - 64), TREASURE)
                    .filled(true)
                    .build();
            }

            // 追上来那个雪球，过半程才出现。
            if run.chasing() {
                let x = at_x(run.chaser);
                list.add_circle(at(x, TRACK_Y - 14), 11., CHASER)
                    .filled(true)
                    .build();
            }

            // 玩家。跳起来的时候画高一点。
            let x = at_x(run.travelled);
            let lift = ((run.height() - run.pos.1) * 0.45) as i32;
            list.add_circle(at(x, TRACK_Y - 12 - lift), 9., BALL)
                .filled(true)
                .build();

            let head = format!(
                "滑雪　剩 {:>2} 秒　速度 {:>5.1}　收集 {}/{}",
                run.seconds_left,
                run.speed,
                run.picked().len(),
                run.treasures().len()
            );
            list.add_text(at(TRACK_LEFT, 60), TEXT, head);

            let hint = match run.outcome() {
                Some(Outcome::Cleared) => format!(
                    "滑完了　得 {} 分（{} 件 × 100 ＋ {} 秒 × 30）",
                    run.score(),
                    run.picked().len(),
                    run.seconds_left
                ),
                Some(Outcome::Caught) => "被后面的雪球追上了　一件也没拿到".to_string(),
                Some(Outcome::TimedOut) => "时间到　一件也没拿到".to_string(),
                None if !run.chasing() => {
                    "按住左键加速（过半程才管用）　右键跳　Esc 弃权".to_string()
                }
                None => "后面追上来了　按住左键加速　右键跳过障碍".to_string(),
            };
            list.add_text(at(TRACK_LEFT, 84), DIM, hint);

            if run.phase() == Phase::Hit {
                list.add_text(at(TRACK_LEFT, 108), CHASER, "撞上了，正在减速");
            }

            // 速度条：巡航 10.86 到冲刺上限 28.86。
            let w = ((run.speed / BOOST_MAX).clamp(0., 1.) * 200.) as i32;
            list.add_rect(
                at(TRACK_LEFT, 440),
                at(TRACK_LEFT + 200, 452),
                [0.2, 0.24, 0.3, 1.],
            )
            .filled(true)
            .build();
            list.add_rect(at(TRACK_LEFT, 440), at(TRACK_LEFT + w, 452), SLOPE)
                .filled(true)
                .build();
            let cruise = TRACK_LEFT + (CRUISE_SPEED / BOOST_MAX * 200.) as i32;
            list.add_rect(at(cruise, 436), at(cruise + 1, 456), CHASER)
                .filled(true)
                .build();
        });
    }
}

impl SceCommand for SceCommandMiniGameSki {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if self.run.is_none() && !self.unavailable {
            self.start(state);
        }

        // 数据缺了或者快进：照旧只跑收尾过程。
        if self.unavailable || state.fast_forward() {
            self.settle(state);
            if self.end_game_proc > 0 {
                state.call_proc(self.end_game_proc as u32);
            }
            return true;
        }

        let quit = state.input().get_key_state(Key::Escape).pressed()
            || state.input().get_key_state(Key::GamePadEast).pressed();

        let input = SkiInput {
            // 原版是 `DAT_00bfc5f4 == 1`，左键**按住**。
            boost: ui.is_mouse_down(MouseButton::Left)
                || state.input().get_key_state(Key::GamePadSouth).is_down(),
            // 原版是 `(DAT_00bfc5f8 & 0xf) == 1`，右键**按下那一帧**。
            jump: ui.is_mouse_clicked(MouseButton::Right)
                || state.input().get_key_state(Key::Space).pressed()
                || state.input().get_key_state(Key::GamePadWest).pressed(),
        };

        let done = if let Some(run) = self.run.as_mut() {
            run.update(delta_sec, input);
            run.outcome().is_some()
        } else {
            true
        };
        self.draw(ui);

        if done {
            self.result_shown += delta_sec;
        }
        if quit || (done && self.result_shown >= RESULT_SECONDS) {
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

/// `.POL` 第一个网格的 AABB 三个半轴。
fn pol_half_extents(vfs: &mini_fs::MiniFs, path: &str) -> Option<(f32, f32, f32)> {
    let bytes = read_bytes(vfs, path)?;
    let pol = fileformats::pol::read_pol(&mut std::io::Cursor::new(bytes)).ok()?;
    let mesh = pol.meshes.first()?;
    let (mn, mx) = (&mesh.aabb_min, &mesh.aabb_max);
    Some((
        (mx.x - mn.x) * 0.5,
        (mx.y - mn.y) * 0.5,
        (mx.z - mn.z) * 0.5,
    ))
}

/// 最大的那个半轴。
fn pol_extent(vfs: &mini_fs::MiniFs, path: &str) -> Option<f32> {
    let (x, y, z) = pol_half_extents(vfs, path)?;
    Some(x.max(y).max(z))
}

/// 半对角线——原版 `gbOBBox::GetRadius` 那一处用它，这里拿 AABB 顶替 OBB。
fn pol_radius(vfs: &mini_fs::MiniFs, path: &str) -> Option<f32> {
    let (x, y, z) = pol_half_extents(vfs, path)?;
    Some((x * x + y * y + z * z).sqrt())
}

/// 一局的时长上限，给别处引用。
pub const RUN_SECONDS: i32 = TIME_LIMIT_SECS;
