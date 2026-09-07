//! 105 打苍蝇 — `LAB_004370e0`（模式 9）。
//!
//! 规则在 `openpal3::states::killflyer`（照 `FUN_00529870` /
//! `FUN_00529e40` / `FUN_0052aa50` 写的，见 `switch/re/notes/killflyer.md`）。
//! 命令在游戏期间阻塞脚本，鼠标在场地里点苍蝇；打完一波结算：钱加进身上、
//! 花费从景天的精里扣（扣到 1 为止）。
//!
//! 画面是 800×600 虚拟坐标上的方块，不是原版的 `SubGame\KillFlyer\*.tga`
//! ——贴图通路还没接。

use crate::openpal3::states::killflyer::{FLY_SIZE, FlyGame, FlyState, cost_from_hp};
use crate::scripting::sce::commands::appraise::{overlay, virtual_mouse};
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{MouseButton, Ui};
use radiance::comdef::ISceneManager;
use radiance::input::Key;

/// 打完之后还让玩家看几秒结果。
const RESULT_SECONDS: f32 = 2.5;

const FLY_COLOR: [f32; 4] = [0.15, 0.12, 0.1, 1.];
const GOLD_COLOR: [f32; 4] = [1., 0.84, 0.2, 1.];
const FALLING_COLOR: [f32; 4] = [0.55, 0.2, 0.2, 1.];
const TEXT_COLOR: [f32; 4] = [1., 1., 1., 1.];

#[derive(Debug, Clone)]
pub struct SceCommandMiniGameSwatAFly {
    game: Option<FlyGame>,
    /// 开局时按当时的精算出来的花费（`ftol(精 × 0.01)`，至少 1）。
    cost: i32,
    /// 打完之后停留的时间。
    result_shown: f32,
    settled: bool,
}

impl SceCommandMiniGameSwatAFly {
    pub fn new() -> Self {
        Self {
            game: None,
            cost: 0,
            result_shown: 0.,
            settled: false,
        }
    }

    fn start(&mut self, state: &mut SceState) {
        let hp = state
            .global_state()
            .persistent_state()
            .role(0)
            .map(|r| r.hp)
            .unwrap_or(0);
        self.cost = cost_from_hp(hp);
        let seed = state.global_state().persistent_state().money() as u32
            ^ (hp as u32).rotate_left(13)
            ^ 0x1234_5678;
        self.game = Some(FlyGame::new(seed));
    }

    /// `FUN_0052aa50`：钱加上这一局挣的，精减掉花费（下限 1）。
    fn settle(&mut self, state: &mut SceState) {
        if self.settled {
            return;
        }
        self.settled = true;
        let earned = self.game.as_ref().map(|g| g.earned).unwrap_or(0);
        let mut p_state = state.global_state_mut().persistent_state_mut();
        p_state.add_money(earned);
        if let Some(role) = p_state.role_mut(0) {
            role.hp = (role.hp - self.cost).max(1);
        }
        log::info!("KillFlyer: 挣了 {earned} 文，花了 {} 精", self.cost);
    }

    fn draw(&self, ui: &Ui) {
        let Some(game) = self.game.as_ref() else {
            return;
        };
        let done = game.finished();
        overlay(ui, "KillFlyer", |ui, at, scale| {
            let list = ui.get_window_draw_list();
            // 场地。
            list.add_rect(at(0, 0), at(800, 600), [0., 0., 0., 0.45])
                .filled(true)
                .build();

            for fly in &game.flies {
                if fly.state == FlyState::Done && fly.fall == 0. {
                    continue;
                }
                let color = match (fly.state, fly.gold) {
                    (FlyState::Falling, _) => FALLING_COLOR,
                    (_, true) => GOLD_COLOR,
                    _ => FLY_COLOR,
                };
                let half = FLY_SIZE * 0.5;
                list.add_rect(
                    at((fly.x - half) as i32, (fly.y - half) as i32),
                    at((fly.x + half) as i32, (fly.y + half) as i32),
                    color,
                )
                .filled(true)
                .build();
            }

            let _ = scale;
            let line = if done {
                format!("打死 {} 只，得 {} 文", game.killed, game.earned)
            } else {
                format!("打苍蝇　已得 {} 文　（点苍蝇，Esc 离开）", game.earned)
            };
            list.add_text(at(24, 24), TEXT_COLOR, line);
        });
    }
}

impl SceCommand for SceCommandMiniGameSwatAFly {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if self.game.is_none() {
            self.start(state);
        }
        if state.fast_forward() {
            self.settle(state);
            return true;
        }

        let quit = state.input().get_key_state(Key::Escape).pressed()
            || state.input().get_key_state(Key::GamePadSouth).pressed();

        let mouse = virtual_mouse(ui);
        let clicked = ui.is_mouse_clicked(MouseButton::Left);
        if let Some(game) = self.game.as_mut() {
            game.tick(delta_sec);
            if clicked {
                game.swat(mouse.0, mouse.1);
            }
        }
        self.draw(ui);

        let done = self.game.as_ref().map(|g| g.finished()).unwrap_or(true);
        if done {
            self.result_shown += delta_sec;
        }
        if quit || (done && self.result_shown >= RESULT_SECONDS) {
            self.settle(state);
            return true;
        }
        false
    }
}
