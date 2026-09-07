//! 119 客栈住店 — `UIShowHotelMenu`，`FUN_004369c0` → `FUN_0044b630`。
//!
//! 规则在 `openpal3::states::inn`（照客栈界面对象的每帧函数 `0x44b0f0` 写
//! 的，见 `switch/re/notes/inn.md`）。配置的解析是
//! `openpal3::states::shop::RestScript`。
//!
//! 三个过程号的用法照原版：钱够跑 `enough_money_proc`、钱不够跑
//! `not_enough_money_proc`、住完跑 `after_rest_proc`。

use std::path::PathBuf;

use crate::openpal3::states::game_db::PLAYER_ACTOR_IDS;
use crate::openpal3::states::inn::{Answer, G_STORY, RestOutcome, Vitals, rest};
use crate::openpal3::states::shop::RestScript;
use crate::scripting::sce::commands::appraise::overlay;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;
use radiance::input::Key;

const TEXT: [f32; 4] = [1., 1., 1., 1.];
const DIM: [f32; 4] = [0.7, 0.74, 0.8, 1.];
const PICKED: [f32; 4] = [1., 0.86, 0.4, 1.];

/// 结算之后停留几秒让玩家看一眼。
const RESULT_SECONDS: f32 = 1.6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// 问「要不要住店」。
    Asking,
    /// 答完了，给一句回话。
    Answered,
}

#[derive(Debug, Clone)]
pub struct SceCommandShowChatRest {
    config_file: String,
    enough_money_proc: u32,
    not_enough_money_proc: u32,
    after_rest_proc: u32,

    inn: RestScript,
    loaded: bool,
    phase: Phase,
    /// 是/否两项里选中哪个。
    cursor: usize,
    outcome: Option<RestOutcome>,
    shown: f32,
}

impl SceCommandShowChatRest {
    pub fn new(
        config_file: String,
        enough_money_proc: u32,
        not_enough_money_proc: u32,
        after_rest_proc: u32,
    ) -> Self {
        Self {
            config_file,
            enough_money_proc,
            not_enough_money_proc,
            after_rest_proc,
            inn: RestScript::default(),
            loaded: false,
            phase: Phase::Asking,
            cursor: 0,
            outcome: None,
            shown: 0.,
        }
    }

    /// `DealScript\Rest\q01rest.txt` 这样的脚本参数转成 vfs 路径，和店铺
    /// 那边同一套。
    fn load(&mut self, state: &mut SceState) {
        self.loaded = true;
        let path = PathBuf::from(format!(
            "/basedata/basedata/{}",
            self.config_file.replace('\\', "/")
        ));
        let Some(text) = read_gbk(state.asset_mgr().vfs(), &path) else {
            log::error!("ShowChatRest: 读不到 {}", path.display());
            return;
        };
        self.inn = RestScript::parse(&text);
    }

    /// 把存档里在队的人搬进 [`Vitals`]，结算完再写回去。
    fn settle(&mut self, answer: Answer, state: &mut SceState) -> RestOutcome {
        let mut p_state = state.global_state_mut().persistent_state_mut();

        let party: Vec<(u32, bool, Vitals)> = PLAYER_ACTOR_IDS
            .iter()
            .filter_map(|&id| {
                let r = p_state.role(id)?;
                Some((
                    id,
                    p_state.is_in_team(id),
                    Vitals {
                        hp: r.hp,
                        max_hp: r.max_hp,
                        sp: r.sp,
                        max_sp: r.max_sp,
                        mp: r.mp,
                        max_mp: r.max_mp,
                    },
                ))
            })
            .collect();

        let mut money = p_state.money();
        let mut story = p_state.get_global(G_STORY).unwrap_or(0);
        let mut slots: Vec<(bool, Vitals)> = party.iter().map(|(_, ok, v)| (*ok, *v)).collect();

        let outcome = rest(answer, &mut money, self.inn.price, &mut slots, &mut story);

        if let RestOutcome::Rested { paid } = outcome {
            for ((id, _, _), (_, v)) in party.iter().zip(slots.iter()) {
                if let Some(r) = p_state.role_mut(*id) {
                    r.hp = v.hp;
                    r.sp = v.sp;
                    r.mp = v.mp;
                }
            }
            p_state.add_money(-paid);
            p_state.set_global(G_STORY, story);
            log::info!("Inn: 住了一晚，付 {paid} 文，g_story = {story}");
        }
        outcome
    }

    fn draw(&self, ui: &Ui, money: i32) {
        let price = self.inn.price;
        let asking = self.phase == Phase::Asking;
        let cursor = self.cursor;
        let ask = if self.inn.ask.is_empty() {
            "要不要住店？".to_string()
        } else {
            self.inn.ask.clone()
        };
        let reply = match self.outcome {
            Some(RestOutcome::Rested { .. }) => self.inn.enough.clone(),
            Some(RestOutcome::TooPoor) => self.inn.not_enough.clone(),
            _ => String::new(),
        };
        let keeper = self.inn.keeper_name.clone();

        overlay(ui, "Inn", |ui, at, _scale| {
            let list = ui.get_window_draw_list();
            list.add_rect(at(60, 380), at(740, 560), [0.06, 0.07, 0.11, 0.86])
                .filled(true)
                .build();

            if !keeper.is_empty() {
                list.add_text(at(84, 396), DIM, &keeper);
            }
            let head = format!("住一晚 {price} 文　身上 {money} 文");
            list.add_text(at(560, 396), DIM, head);

            if asking {
                for (i, line) in ask.split('\n').enumerate() {
                    list.add_text(at(84, 424 + i as i32 * 22), TEXT, line);
                }
                for (i, label) in ["住下", "不住"].iter().enumerate() {
                    let y = 500 + i as i32 * 24;
                    let picked = i == cursor;
                    list.add_text(
                        at(84, y),
                        if picked { PICKED } else { DIM },
                        format!("{} {label}", if picked { "▶" } else { "  " }),
                    );
                }
                list.add_text(at(560, 524), DIM, "↑↓ 选　回车 确定　Esc 离开");
            } else {
                for (i, line) in reply.split('\n').enumerate() {
                    list.add_text(at(84, 424 + i as i32 * 22), TEXT, line);
                }
            }
        });
    }
}

impl SceCommand for SceCommandShowChatRest {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if !self.loaded {
            self.load(state);
        }

        // 快进（脚本测试）：当作选了「住下」，钱够不够由 `rest` 判。
        if state.fast_forward() {
            let outcome = self.settle(Answer::Rest, state);
            self.queue_procs(outcome, state);
            return true;
        }

        if self.phase == Phase::Asking {
            let (up, down, confirm, cancel) = {
                let input = state.input();
                (
                    input.get_key_state(Key::Up).pressed()
                        || input.get_key_state(Key::GamePadDPadUp).pressed(),
                    input.get_key_state(Key::Down).pressed()
                        || input.get_key_state(Key::GamePadDPadDown).pressed(),
                    input.get_key_state(Key::Space).pressed()
                        || input.get_key_state(Key::GamePadSouth).pressed()
                        || ui.is_key_pressed(imgui::Key::Enter),
                    input.get_key_state(Key::Escape).pressed()
                        || input.get_key_state(Key::GamePadEast).pressed(),
                )
            };

            if up || down {
                self.cursor = 1 - self.cursor;
            }
            if cancel {
                self.cursor = 1;
            }
            if confirm || cancel {
                let answer = if self.cursor == 0 {
                    Answer::Rest
                } else {
                    Answer::Leave
                };
                let outcome = self.settle(answer, state);
                self.outcome = Some(outcome);
                if outcome == RestOutcome::Left {
                    // 选了不住：原版只关界面，一个过程都不跑。
                    return true;
                }
                self.phase = Phase::Answered;
            }
        } else {
            self.shown += delta_sec;
        }

        let money = state.global_state().persistent_state().money();
        self.draw(ui, money);

        if self.phase == Phase::Answered && self.shown >= RESULT_SECONDS {
            let outcome = self.outcome.unwrap_or(RestOutcome::Left);
            self.queue_procs(outcome, state);
            return true;
        }
        false
    }
}

impl SceCommandShowChatRest {
    /// 原版的顺序是「钱够的对白 → 淡出、回血扣钱 → 淡入 → 住完的对白」。
    /// 移植端没有那一秒黑屏，回血在对白之前就做掉了（玩家看不出来），
    /// 两段对白的先后和原版一致——`call_proc` 是压栈，所以后压的先跑。
    fn queue_procs(&self, outcome: RestOutcome, state: &mut SceState) {
        match outcome {
            RestOutcome::Rested { .. } => {
                if self.after_rest_proc > 0 {
                    state.call_proc(self.after_rest_proc);
                }
                if self.enough_money_proc > 0 {
                    state.call_proc(self.enough_money_proc);
                }
            }
            RestOutcome::TooPoor => {
                if self.not_enough_money_proc > 0 {
                    state.call_proc(self.not_enough_money_proc);
                }
            }
            RestOutcome::Left => {}
        }
    }
}

fn read_gbk(vfs: &mini_fs::MiniFs, path: &std::path::Path) -> Option<String> {
    use common::store_ext::StoreExt2;
    use encoding::Encoding;
    let bytes = vfs.read_to_end(path).ok()?;
    encoding::all::GBK
        .decode(&bytes, encoding::DecoderTrap::Ignore)
        .ok()
}
