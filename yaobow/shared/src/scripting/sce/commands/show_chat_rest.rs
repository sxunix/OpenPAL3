//! 119 客栈 — `UIShowHotelMenu`，`FUN_004369c0` → `FUN_0044b630`。
//!
//! 规则在 `openpal3::states::inn`（照客栈界面对象的每帧函数 `0x44b0f0` 和
//! 消息处理 `FUN_0044b710` 写的，见 `switch/re/notes/inn.md`）。配置的解析
//! 是 `openpal3::states::shop::RestScript`。
//!
//! 流程照原版：
//!
//! ```text
//! 三项菜单 聊天 / 住宿 / 退出
//!   ├ 聊天 → 跑 M_ID 那个对话过程
//!   ├ 退出 → 什么都不跑
//!   └ 住宿 → 播 M_Rest → 「住宿共需 N 文钱」是/否
//!             ├ 否     → 什么都不跑
//!             ├ 钱不够 → 跑 not_enough_money_proc
//!             └ 钱够   → 淡出、等一秒、回血扣钱改剧情、淡入
//!                        → 跑 enough_money_proc 再跑 after_rest_proc
//! ```

use std::path::PathBuf;

use crate::openpal3::assis_lang;
use crate::openpal3::states::game_db::PLAYER_ACTOR_IDS;
use crate::openpal3::states::inn::{
    Answer, BLACKOUT_MILLIS, FADE_SECONDS, G_STORY, InnOutcome, MenuItem, Vitals, can_afford,
    cost_line, lang, rest,
};
use crate::openpal3::states::shop::RestScript;
use crate::scripting::sce::commands::appraise::overlay;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;
use radiance::input::Key;

const LANG_PATH: &str = "/basedata/basedata/AssisLang.lang";
const LANG_SECTION: &str = "sec1";

const TEXT: [f32; 4] = [1., 1., 1., 1.];
const DIM: [f32; 4] = [0.7, 0.74, 0.8, 1.];
const PICKED: [f32; 4] = [1., 0.86, 0.4, 1.];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// 三项菜单。
    Menu,
    /// 播 `M_Rest`。
    Line,
    /// 「住宿共需 N 文钱」是/否。
    Confirm,
    /// 淡出。
    FadingOut,
    /// 全黑，等一秒；结算就在这一段的末尾做。
    Blackout,
    /// 淡入。
    FadingIn,
}

#[derive(Debug, Clone)]
pub struct SceCommandShowChatRest {
    config_file: String,
    enough_money_proc: u32,
    not_enough_money_proc: u32,
    after_rest_proc: u32,

    inn: RestScript,
    /// `AssisLang.lang` 的 346–350 五条。
    labels: [String; 5],
    loaded: bool,

    phase: Phase,
    /// 菜单里选中第几项。
    cursor: usize,
    /// 确认框里选中「住下」还是「不住」。
    answer: usize,
    /// 当前阶段已经走了多久。
    spent: f32,
    settled: bool,
    outcome: Option<InnOutcome>,
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
            labels: Default::default(),
            loaded: false,
            phase: Phase::Menu,
            cursor: 0,
            answer: 0,
            spent: 0.,
            settled: false,
            outcome: None,
        }
    }

    /// `DealScript\Rest\q01rest.txt` 这样的脚本参数转成 vfs 路径，和店铺
    /// 那边同一套。三个菜单项和确认框那句话不在这个配置里，在本地化表里。
    fn load(&mut self, state: &mut SceState) {
        self.loaded = true;
        let vfs = state.asset_mgr().vfs();

        let path = PathBuf::from(format!(
            "/basedata/basedata/{}",
            self.config_file.replace('\\', "/")
        ));
        match read_gbk(vfs, &path) {
            Some(text) => self.inn = RestScript::parse(&text),
            None => log::error!("ShowChatRest: 读不到 {}", path.display()),
        }

        let fallback = ["住宿共需", "文钱", "聊天", "住宿", "退出"];
        let table = read_gbk(vfs, &PathBuf::from(LANG_PATH))
            .map(|text| assis_lang::parse_section(&text, LANG_SECTION))
            .unwrap_or_else(|| {
                log::error!("ShowChatRest: 读不到 {LANG_PATH}");
                vec![]
            });
        for (i, slot) in [
            lang::COST_PREFIX,
            lang::COST_SUFFIX,
            lang::CHAT,
            lang::STAY,
            lang::LEAVE,
        ]
        .into_iter()
        .enumerate()
        {
            self.labels[i] = table
                .get(slot)
                .cloned()
                .unwrap_or_else(|| fallback[i].to_string());
        }
    }

    fn label(&self, item: MenuItem) -> &str {
        match item {
            MenuItem::Chat => &self.labels[2],
            MenuItem::Stay => &self.labels[3],
            MenuItem::Leave => &self.labels[4],
        }
    }

    fn cost_text(&self) -> String {
        cost_line(&self.labels[0], self.inn.price, &self.labels[1])
    }

    /// 扣钱、回血、改剧情号。只在全黑那一段末尾做一次。
    fn settle(&mut self, answer: Answer, state: &mut SceState) -> InnOutcome {
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

        if let InnOutcome::Rested { paid } = outcome {
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

    /// 结束时把该跑的过程压上去。`call_proc` 是压栈，所以后压的先跑——要
    /// 「钱够的对白」在「住完的对白」之前，就得先压后者。
    fn queue_procs(&self, state: &mut SceState) {
        match self.outcome {
            Some(InnOutcome::Chat { keeper_proc }) => {
                if keeper_proc > 0 {
                    state.call_proc(keeper_proc as u32);
                }
            }
            Some(InnOutcome::Rested { .. }) => {
                if self.after_rest_proc > 0 {
                    state.call_proc(self.after_rest_proc);
                }
                if self.enough_money_proc > 0 {
                    state.call_proc(self.enough_money_proc);
                }
            }
            Some(InnOutcome::TooPoor) => {
                if self.not_enough_money_proc > 0 {
                    state.call_proc(self.not_enough_money_proc);
                }
            }
            Some(InnOutcome::Left) | None => {}
        }
    }

    fn finish(&mut self, state: &mut SceState) -> bool {
        state.set_curtain(0.);
        self.queue_procs(state);
        true
    }

    fn draw(&self, ui: &Ui, money: i32) {
        // 全黑那一段只留幕布，界面不画。
        if self.phase == Phase::Blackout {
            return;
        }
        let phase = self.phase;
        let cursor = self.cursor;
        let answer = self.answer;
        let keeper = self.inn.keeper_name.clone();
        let price = self.inn.price;
        let items: Vec<String> = MenuItem::ALL
            .iter()
            .map(|i| self.label(*i).to_string())
            .collect();
        let line = self.inn.ask.clone();
        let cost = self.cost_text();

        overlay(ui, "Inn", |ui, at, _scale| {
            let list = ui.get_window_draw_list();
            list.add_rect(at(60, 372), at(740, 560), [0.06, 0.07, 0.11, 0.86])
                .filled(true)
                .build();

            if !keeper.is_empty() {
                list.add_text(at(84, 388), DIM, &keeper);
            }
            list.add_text(
                at(548, 388),
                DIM,
                format!("住一晚 {price} 文　身上 {money} 文"),
            );

            match phase {
                Phase::Menu => {
                    for (i, label) in items.iter().enumerate() {
                        let picked = i == cursor;
                        list.add_text(
                            at(84, 424 + i as i32 * 26),
                            if picked { PICKED } else { DIM },
                            format!("{} {label}", if picked { "▶" } else { "  " }),
                        );
                    }
                    list.add_text(at(548, 528), DIM, "↑↓ 选　回车 确定　Esc 退出");
                }
                Phase::Line => {
                    for (i, l) in line.split('\n').enumerate() {
                        list.add_text(at(84, 424 + i as i32 * 24), TEXT, l);
                    }
                    list.add_text(at(548, 528), DIM, "回车 继续");
                }
                Phase::Confirm => {
                    // 原版这句是「\n + 十个空格 + 住宿共需 N 文钱」。
                    for (i, l) in cost.trim_start_matches('\n').split('\n').enumerate() {
                        list.add_text(at(84, 424 + i as i32 * 24), TEXT, l);
                    }
                    for (i, label) in ["住下", "不住"].iter().enumerate() {
                        let picked = i == answer;
                        list.add_text(
                            at(84, 480 + i as i32 * 26),
                            if picked { PICKED } else { DIM },
                            format!("{} {label}", if picked { "▶" } else { "  " }),
                        );
                    }
                    list.add_text(at(548, 528), DIM, "↑↓ 选　回车 确定　Esc 不住");
                }
                _ => {}
            }
        });
    }
}

/// 这一帧按了什么。
struct Keys {
    up: bool,
    down: bool,
    confirm: bool,
    cancel: bool,
}

fn keys(ui: &Ui, state: &SceState) -> Keys {
    let input = state.input();
    Keys {
        up: input.get_key_state(Key::Up).pressed()
            || input.get_key_state(Key::GamePadDPadUp).pressed(),
        down: input.get_key_state(Key::Down).pressed()
            || input.get_key_state(Key::GamePadDPadDown).pressed(),
        confirm: input.get_key_state(Key::Space).pressed()
            || input.get_key_state(Key::GamePadSouth).pressed()
            || ui.is_key_pressed(imgui::Key::Enter),
        cancel: input.get_key_state(Key::Escape).pressed()
            || input.get_key_state(Key::GamePadEast).pressed(),
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

        // 快进（脚本测试）：当作选了「退出」，一个过程都不跑，钱和血都不动
        // ——自动跑图不该替玩家花钱。
        if state.fast_forward() {
            self.outcome = Some(InnOutcome::Left);
            return self.finish(state);
        }

        self.spent += delta_sec;

        match self.phase {
            Phase::Menu => {
                let k = keys(ui, state);
                if k.up {
                    self.cursor = (self.cursor + MenuItem::ALL.len() - 1) % MenuItem::ALL.len();
                }
                if k.down {
                    self.cursor = (self.cursor + 1) % MenuItem::ALL.len();
                }
                if k.cancel {
                    self.outcome = Some(InnOutcome::Left);
                    return self.finish(state);
                }
                if k.confirm {
                    match MenuItem::ALL[self.cursor] {
                        MenuItem::Chat => {
                            self.outcome = Some(InnOutcome::Chat {
                                keeper_proc: self.inn.keeper_id,
                            });
                            return self.finish(state);
                        }
                        MenuItem::Leave => {
                            self.outcome = Some(InnOutcome::Left);
                            return self.finish(state);
                        }
                        MenuItem::Stay => {
                            self.phase = Phase::Line;
                            self.spent = 0.;
                        }
                    }
                }
            }

            Phase::Line => {
                // 原版是等对话系统把 `M_Rest` 播完，也就是等玩家按键
                // （0x44b111 那个「对话系统忙不忙」的虚调用）。这里同样只
                // 等按键，不加超时。
                let k = keys(ui, state);
                if k.confirm || k.cancel {
                    self.phase = Phase::Confirm;
                    self.spent = 0.;
                    self.answer = 0;
                }
            }

            Phase::Confirm => {
                let k = keys(ui, state);
                if k.up || k.down {
                    self.answer = 1 - self.answer;
                }
                if k.cancel {
                    self.answer = 1;
                }
                if k.confirm || k.cancel {
                    if self.answer == 1 {
                        self.outcome = Some(InnOutcome::Left);
                        return self.finish(state);
                    }
                    // 钱不够就地退出，不黑屏——原版也是先比钱再淡出。
                    let money = state.global_state().persistent_state().money();
                    if !can_afford(money, self.inn.price) {
                        self.outcome = Some(InnOutcome::TooPoor);
                        return self.finish(state);
                    }
                    self.phase = Phase::FadingOut;
                    self.spent = 0.;
                }
            }

            Phase::FadingOut => {
                let t = (self.spent / FADE_SECONDS).clamp(0., 1.);
                state.set_auto_fade_in(false);
                state.set_curtain(t);
                if t >= 1. {
                    self.phase = Phase::Blackout;
                    self.spent = 0.;
                }
            }

            Phase::Blackout => {
                state.set_curtain(1.);
                if self.spent * 1000. >= BLACKOUT_MILLIS as f32 {
                    if !self.settled {
                        self.settled = true;
                        self.outcome = Some(self.settle(Answer::Rest, state));
                    }
                    self.phase = Phase::FadingIn;
                    self.spent = 0.;
                }
            }

            Phase::FadingIn => {
                let t = (self.spent / FADE_SECONDS).clamp(0., 1.);
                state.set_curtain(1. - t);
                if t >= 1. {
                    return self.finish(state);
                }
            }
        }

        let money = state.global_state().persistent_state().money();
        self.draw(ui, money);
        false
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
