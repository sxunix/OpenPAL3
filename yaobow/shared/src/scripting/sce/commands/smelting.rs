//! 146 冶炼 — `LAB_00436a10`。
//!
//! 配方在 `openpal3::states::smelting`（照 `FUN_0047a580` 写的，见
//! `switch/re/notes/smelting.md`）。命令在冶炼期间阻塞脚本（原版是
//! `FUN_0044f610(3)` 进模式 3 再挂一个等待任务），关掉之后跑参数里的那个
//! 过程——参数是**过程号**，原版存进 `DAT_00dba240`，确认键按下时
//! `FUN_00432170(DAT_00dba240, 0)`。
//!
//! 界面是文字浮层，不是原版那一套。

use crate::openpal3::states::smelting::{Recipe, recipes};
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

#[derive(Debug, Clone)]
pub struct SceCommandSmeltingMenu {
    /// 关掉冶炼之后要跑的过程（原版 `DAT_00dba240`）。
    end_proc: i32,
    rows: Vec<Row>,
    cursor: usize,
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

    /// 炼一件：扣一块尸块、一份矿石，给一件产物。
    ///
    /// 扣材料是 `FUN_0047aab0` 逐字抄的；发产物那一步 `[推断]`——原版发货
    /// 的位置没找到，见 `smelting.md` §4。
    fn smelt(&mut self, state: &mut SceState) {
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
            p_state.add_item(row.recipe.product, 1);
        }
        self.smelted += 1;
        self.notice = format!("炼出了 {}", row.product_name);
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
                ui.text("确定：炼一件　取消：离开");
            });
    }
}

impl SceCommand for SceCommandSmeltingMenu {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        if !self.loaded {
            self.loaded = true;
            self.rebuild(state);
        }

        if state.fast_forward() {
            self.finish(state);
            return true;
        }

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
            Some(Input::Confirm) => self.smelt(state),
            Some(Input::Cancel) => {
                self.finish(state);
                return true;
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
