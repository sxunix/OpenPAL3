//! 146 冶炼 — `LAB_00436a10`。
//!
//! 配方在 `openpal3::states::smelting`（照 `FUN_0047a580` 写的，见
//! `switch/re/notes/smelting.md`）。命令在冶炼期间阻塞脚本（原版是
//! `FUN_0044f610(3)` 进模式 3 再挂一个等待任务），关掉之后跑参数里的那个
//! 过程——参数是**过程号**，原版存进 `DAT_00dba240`，确认键按下时
//! `FUN_00432170(DAT_00dba240, 0)`。
//!
//! 界面是文字浮层，不是原版那一套。和原版的差别只在操作层：原版分「加入
//! 队列」（当场扣材料）和「确认」（等 5 秒出一件）两步，这里并成一步——
//! 确定就扣材料、等满 5 秒把产物给出来，扣什么发什么、数量和顺序都一样。

use crate::openpal3::states::smelting::{Recipe, SMELT_SECONDS, recipes};
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{Condition, Ui};
use radiance::comdef::ISceneManager;
use radiance::input::Key;

/// 一行配方外加两边的名字，界面直接用。
#[derive(Debug, Clone)]
struct Row {
    recipe: Recipe,
    corpse_name: String,
    ore_name: String,
    product_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Input {
    Up,
    Down,
    Confirm,
    Cancel,
}

/// 正在炼的那一件：材料已经扣了，等满 [`SMELT_SECONDS`] 把产物给出来。
#[derive(Debug, Clone)]
struct Firing {
    product: u32,
    product_name: String,
    elapsed: f32,
}

#[derive(Debug, Clone)]
pub struct SceCommandSmeltingMenu {
    /// 关掉冶炼之后要跑的过程（原版 `DAT_00dba240`）。
    end_proc: i32,
    rows: Vec<Row>,
    cursor: usize,
    /// 炉子里那一件，`None` 表示没在炼。
    firing: Option<Firing>,
    /// 这一趟炼成了几件，退出时报一句。
    smelted: u32,
    notice: String,
    loaded: bool,
}

impl SceCommandSmeltingMenu {
    pub fn new(end_proc: i32) -> Self {
        Self {
            end_proc,
            rows: Vec::new(),
            cursor: 0,
            firing: None,
            smelted: 0,
            notice: String::new(),
            loaded: false,
        }
    }

    fn rebuild(&mut self, state: &SceState) {
        let Some(db) = state.global_state().game_db().cloned() else {
            self.rows.clear();
            return;
        };
        let p_state = state.global_state().persistent_state();
        let name = |id: u32| {
            db.item(id)
                .and_then(|i| i.name.as_str().ok())
                .unwrap_or_else(|| format!("道具#{id}"))
        };
        self.rows = recipes(&db, p_state.inventory())
            .into_iter()
            .map(|recipe| Row {
                corpse_name: name(recipe.corpse),
                ore_name: name(recipe.ore),
                product_name: name(recipe.product),
                recipe,
            })
            .collect();
        if self.cursor >= self.rows.len() {
            self.cursor = self.rows.len().saturating_sub(1);
        }
    }

    /// 开一炉：当场扣一块尸块、一份矿石（`FUN_0047aab0` 的「加入队列」就是
    /// 当场扣的），产物要等 [`SMELT_SECONDS`] 秒。
    fn start_firing(&mut self, state: &mut SceState) {
        if self.firing.is_some() {
            return;
        }
        let Some(row) = self.rows.get(self.cursor).cloned() else {
            return;
        };
        if !row.recipe.available() {
            return;
        }
        {
            let mut p_state = state.global_state_mut().persistent_state_mut();
            if p_state.item_count(row.recipe.corpse) == 0 || p_state.item_count(row.recipe.ore) == 0
            {
                return;
            }
            p_state.remove_item(row.recipe.corpse, 1);
            p_state.remove_item(row.recipe.ore, 1);
        }
        self.firing = Some(Firing {
            product: row.recipe.product,
            product_name: row.product_name.clone(),
            elapsed: 0.,
        });
        self.notice = format!("正在炼 {}……", row.product_name);
        self.rebuild(state);
    }

    /// 炉子走一帧。满 5 秒把产物放进背包——`FUN_0047b3b0` 就是这么发货的
    /// （`cmp ecx, 0x1388` 之后 `FUN_0040ccf0`），一炉一件。
    fn step_firing(&mut self, state: &mut SceState, delta_sec: f32) {
        let Some(firing) = self.firing.as_mut() else {
            return;
        };
        firing.elapsed += delta_sec;
        if firing.elapsed < SMELT_SECONDS {
            return;
        }
        let done = self.firing.take().expect("刚才还在");
        state
            .global_state_mut()
            .persistent_state_mut()
            .add_item(done.product, 1);
        self.smelted += 1;
        self.notice = format!("炼出了 {}", done.product_name);
        self.rebuild(state);
    }

    fn poll_input(state: &SceState) -> Option<Input> {
        let input = state.input();
        let pressed = |k: Key| input.get_key_state(k).pressed();
        if pressed(Key::Up) || pressed(Key::GamePadDPadUp) {
            Some(Input::Up)
        } else if pressed(Key::Down) || pressed(Key::GamePadDPadDown) {
            Some(Input::Down)
        } else if pressed(Key::Space) || pressed(Key::GamePadEast) {
            Some(Input::Confirm)
        } else if pressed(Key::Escape) || pressed(Key::GamePadSouth) {
            Some(Input::Cancel)
        } else {
            None
        }
    }

    fn draw(&self, ui: &Ui) {
        let [window_width, _] = ui.io().display_size;
        ui.window("Smelting")
            .collapsible(false)
            .title_bar(false)
            .resizable(false)
            .always_auto_resize(true)
            .position([16., 16.], Condition::Always)
            .size_constraints([window_width * 0.4, 0.], [window_width * 0.9, f32::MAX])
            .build(|| {
                let _font = radiance::imgui::game_font(radiance::imgui::GameFontSize::LARGE)
                    .map(|f| ui.push_font(f));

                ui.text("冶炼");
                if !self.notice.is_empty() {
                    ui.same_line();
                    ui.text_colored([1.0, 0.8, 0.3, 1.0], format!("  {}", self.notice));
                }
                ui.separator();

                if self.rows.is_empty() {
                    ui.text("没有能配上的尸块和矿石。");
                } else {
                    for (i, row) in self.rows.iter().enumerate() {
                        let marker = if i == self.cursor { "▶" } else { "  " };
                        let line = format!(
                            "{marker}{}（{}）＋ {}（{}）→ {}",
                            row.corpse_name,
                            row.recipe.corpse_count,
                            row.ore_name,
                            row.recipe.ore_count,
                            row.product_name
                        );
                        if row.recipe.available() {
                            ui.text(line);
                        } else {
                            ui.text_disabled(line);
                        }
                    }
                }

                ui.separator();
                match self.firing.as_ref() {
                    Some(f) => ui.text(format!(
                        "冶炼中……{:.0}/{:.0} 秒",
                        f.elapsed.min(SMELT_SECONDS),
                        SMELT_SECONDS
                    )),
                    None => ui.text("确定：炼一件　取消：离开"),
                }
            });
    }
}

impl SceCommand for SceCommandSmeltingMenu {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if !self.loaded {
            self.loaded = true;
            self.rebuild(state);
        }

        if state.fast_forward() {
            self.finish(state);
            return true;
        }

        self.step_firing(state, delta_sec);
        self.draw(ui);

        match Self::poll_input(state) {
            Some(Input::Up) => {
                if !self.rows.is_empty() {
                    self.cursor = (self.cursor + self.rows.len() - 1) % self.rows.len();
                }
            }
            Some(Input::Down) => {
                if !self.rows.is_empty() {
                    self.cursor = (self.cursor + 1) % self.rows.len();
                }
            }
            Some(Input::Confirm) => self.start_firing(state),
            Some(Input::Cancel) => {
                // 炉子还在烧就先等着——原版这段时间界面也不放人。
                if self.firing.is_none() {
                    self.finish(state);
                    return true;
                }
            }
            None => {}
        }
        false
    }
}

impl SceCommandSmeltingMenu {
    /// 关掉界面：原版在确认键上跑参数里的过程（`FUN_00432170`）。
    fn finish(&mut self, state: &mut SceState) {
        log::info!("SmeltingMenu: 炼了 {} 件", self.smelted);
        if self.end_proc > 0 {
            state.call_proc(self.end_proc as u32);
        }
    }
}
