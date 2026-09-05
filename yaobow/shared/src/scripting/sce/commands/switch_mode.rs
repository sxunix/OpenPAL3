//! 102 全局游戏模式切换 — `LAB_00436f90`.
//!
//! 指令表里这条记的是「选择当前渲染状态」，处理函数说的不是这回事。它只做
//! 一件事：
//!
//! ```text
//! FUN_0044f610(this = 0xdb9fd0, 取参数());
//! ```
//!
//! `FUN_0044f610` 是全局模式切换：把新模式存进 `[管理器+0x38c8]`、旧的存进
//! `+0x38cc`，再让 `FUN_0044f8d0(模式)` 造出该模式的状态对象放进 `+0x38c4`，
//! 之后每帧的更新、输入、绘制全走那个对象的虚表。战斗笔记里「战败走
//! `SwitchMode(0xf)` 回标题」说的就是它。
//!
//! 脚本里这条指令只出现 16 处，参数只有两种：**13** 在 11 个
//! 「从XX进入大地图」里，**0** 在 5 个 `T_mXX` 里。所以移植只需要认这几个
//! 模式：
//!
//! | 参数 | 原版 | 这里 |
//! |---|---|---|
//! | 0 | 回冒险模式 | 已经在冒险模式，什么都不用做 |
//! | 13 | 大地图 | [`BigMapScreen`] |
//! | 15 | 回标题 | 同 147，`request_title` |
//! | 其余 | 战斗/菜单/小游戏等 | 记一条日志放过 |
//!
//! 大地图的完整逆向在 `switch/re/notes/bigmap.md`，目的地表在
//! [`crate::openpal3::bigmap`]。这里复刻的是它的**行为**：
//!
//! * 画 6×4 张 256×256 的背景，再画 `可见[i] != 0` 的目的地图标
//!   （0x44a366 的绘制循环）；
//! * 选中第 `i` 个之后跑 `ui\bigmap\bigmap.sce` 的 `i + 100` 号过程，
//!   再回冒险模式（`FUN_0044a4f0`）。目的地过程自己做 `LoadScene` 和摆
//!   角色位置。
//!
//! 没有复刻的：原版用 `Spirit_{LU,RU,LD,RD}{0,1}.tga` 四角各两帧做选中框的
//! 呼吸动画，这里用一个描边代替 `[推断]`；原版的鼠标滚轮/边缘卷动也没做，
//! 整张图按屏幕等比缩放一次画完。

use std::rc::Rc;

use crate::openpal3::bigmap::{
    DESTINATIONS, Destination, ELEMENT_DIR, MAP_DIR, MAP_HEIGHT, MAP_WIDTH, SCE_PATH, TILE,
    TILES_X, TILES_Y, proc_of,
};
use crate::openpal3::loaders::sce_loader::sce_load_from_file;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{Condition, Ui};
use radiance::comdef::ISceneManager;
use radiance::rendering::Sprite;

/// 回冒险模式。
pub const MODE_ADVENTURE: i32 = 0;
/// 标题画面——147 走的也是这个（原版 `SwitchMode(4)` 建标题状态对象，
/// 15 是「结束当前游戏回标题」）。
pub const MODE_TITLE: i32 = 15;
/// 大地图。
pub const MODE_BIGMAP: i32 = 13;

/// 大地图那一屏。第一次 update 时把 24 张背景和用得上的图标读进来。
struct BigMapScreen {
    tiles: Vec<Option<Rc<Sprite>>>,
    icons: Vec<Option<Rc<Sprite>>>,
    /// 这次进图时哪些目的地是亮的（指令 125 存下的状态 > 0）。
    visible: Vec<usize>,
    /// 键盘选中的那个，鼠标悬停会盖掉它。
    cursor: usize,
}

impl BigMapScreen {
    fn load(state: &SceState) -> Self {
        let asset_mgr = state.asset_mgr();
        let tiles = (0..TILES_X * TILES_Y)
            .map(|n| asset_mgr.load_sprite(format!("{MAP_DIR}/BigMap{n}.tga")))
            .collect();

        let visible: Vec<usize> = (0..DESTINATIONS.len())
            .filter(|i| {
                state
                    .global_state()
                    .persistent_state()
                    .bigmap_element_visible(*i as i32)
            })
            .collect();

        // 只读亮着的那些图标，暗的画不出来也就不必读。
        let icons = DESTINATIONS
            .iter()
            .enumerate()
            .map(|(i, d)| {
                visible
                    .contains(&i)
                    .then(|| asset_mgr.load_sprite(format!("{ELEMENT_DIR}/{}.tga", d.texture)))
                    .flatten()
            })
            .collect();

        Self {
            tiles,
            icons,
            cursor: 0,
            visible,
        }
    }

    /// 屏幕上画一次，返回玩家这一帧选中的目的地。
    fn draw(&mut self, ui: &Ui) -> Option<usize> {
        let [screen_w, screen_h] = ui.io().display_size;
        let scale = (screen_w / MAP_WIDTH as f32).min(screen_h / MAP_HEIGHT as f32);
        let origin = [
            (screen_w - MAP_WIDTH as f32 * scale) * 0.5,
            (screen_h - MAP_HEIGHT as f32 * scale) * 0.5,
        ];
        let at = |x: i32, y: i32| [origin[0] + x as f32 * scale, origin[1] + y as f32 * scale];

        let mouse = ui.io().mouse_pos;
        let mut hovered = None;
        for &i in &self.visible {
            let d = &DESTINATIONS[i];
            let a = at(d.x, d.y);
            let b = at(d.x + d.width, d.y + d.height);
            if mouse[0] >= a[0] && mouse[0] < b[0] && mouse[1] >= a[1] && mouse[1] < b[1] {
                hovered = Some(i);
            }
        }
        if let Some(i) = hovered {
            self.cursor = i;
        }

        let _pad = ui.push_style_var(imgui::StyleVar::WindowPadding([0., 0.]));
        let _border = ui.push_style_var(imgui::StyleVar::WindowBorderSize(0.));
        ui.window("BigMap")
            .collapsible(false)
            .title_bar(false)
            .resizable(false)
            .movable(false)
            .scroll_bar(false)
            .draw_background(false)
            .no_inputs()
            .position([0., 0.], Condition::Always)
            .size([screen_w, screen_h], Condition::Always)
            .build(|| {
                let list = ui.get_window_draw_list();
                // 背景先铺满，再压图标。
                for (n, tile) in self.tiles.iter().enumerate() {
                    let Some(tile) = tile else { continue };
                    let (x, y) = crate::openpal3::bigmap::tile_origin(n as i32);
                    list.add_image(tile.imgui_texture_id(), at(x, y), at(x + TILE, y + TILE))
                        .build();
                }
                for &i in &self.visible {
                    let d = &DESTINATIONS[i];
                    let a = at(d.x, d.y);
                    let b = at(d.x + d.width, d.y + d.height);
                    if let Some(icon) = &self.icons[i] {
                        list.add_image(icon.imgui_texture_id(), a, b).build();
                    }
                    if i == self.cursor {
                        // `[推断]` 原版是四角 Spirit_* 的两帧动画，这里先用
                        // 一个描边把选中标出来。
                        list.add_rect(a, b, [1., 0.92, 0.45, 1.])
                            .thickness(2.)
                            .build();
                    }
                }
            });

        let confirm = ui.is_mouse_clicked(imgui::MouseButton::Left) && hovered.is_some();
        if confirm {
            return hovered;
        }
        self.keyboard(ui)
    }

    /// 方向键在亮着的目的地之间挑，空格/回车确认。原版是纯鼠标的，这个是
    /// 移植加的，方便手柄和无鼠标的场合 `[推断]`。
    fn keyboard(&mut self, ui: &Ui) -> Option<usize> {
        let step = |v: &Vec<usize>, cur: usize, forward: bool| -> usize {
            let at = v.iter().position(|i| *i == cur).unwrap_or(0);
            let n = v.len();
            let next = if forward {
                (at + 1) % n
            } else {
                (at + n - 1) % n
            };
            v[next]
        };
        if self.visible.is_empty() {
            return None;
        }
        if ui.is_key_pressed(imgui::Key::RightArrow) || ui.is_key_pressed(imgui::Key::DownArrow) {
            self.cursor = step(&self.visible, self.cursor, true);
        }
        if ui.is_key_pressed(imgui::Key::LeftArrow) || ui.is_key_pressed(imgui::Key::UpArrow) {
            self.cursor = step(&self.visible, self.cursor, false);
        }
        (ui.is_key_pressed(imgui::Key::Space) || ui.is_key_pressed(imgui::Key::Enter))
            .then_some(self.cursor)
    }
}

/// 102 SwitchMode.
pub struct SceCommandSwitchMode {
    mode: i32,
    screen: Option<BigMapScreen>,
    /// 读过一次就不再重试，读不到就直接放过，免得卡死脚本。
    tried: bool,
}

impl Clone for SceCommandSwitchMode {
    fn clone(&self) -> Self {
        Self::new(self.mode)
    }
}

impl std::fmt::Debug for SceCommandSwitchMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandSwitchMode")
            .field("mode", &self.mode)
            .finish()
    }
}

impl SceCommand for SceCommandSwitchMode {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        match self.mode {
            MODE_BIGMAP => self.big_map(ui, state),
            MODE_ADVENTURE => true,
            MODE_TITLE => {
                log::info!("SwitchMode: 15 回标题");
                state.request_title();
                true
            }
            other => {
                log::warn!("SwitchMode: 模式 {other} 没有对应实现，跳过");
                true
            }
        }
    }
}

impl SceCommandSwitchMode {
    pub fn new(mode: i32) -> Self {
        Self {
            mode,
            screen: None,
            tried: false,
        }
    }

    fn big_map(&mut self, ui: &Ui, state: &mut SceState) -> bool {
        if !self.tried {
            self.tried = true;
            let screen = BigMapScreen::load(state);
            if screen.visible.is_empty() {
                log::warn!("SwitchMode: 大地图上一个目的地都没亮，直接返回");
                return true;
            }
            log::info!("SwitchMode: 进入大地图，{} 个目的地", screen.visible.len());
            self.screen = Some(screen);
        }
        let Some(screen) = self.screen.as_mut() else {
            return true;
        };

        // 自动化跑图时不能停在这里等人点，取第一个亮着的目的地。
        let picked = if state.fast_forward() {
            screen.visible.first().copied()
        } else {
            screen.draw(ui)
        };
        let Some(index) = picked else {
            return false;
        };

        self.screen = None;
        Self::enter(state, index);
        true
    }

    /// `FUN_0044a4f0`：跑 `bigmap.sce` 的 `index + 100` 号过程，再回冒险
    /// 模式。回模式这一步在移植里是隐式的——目的地过程自己带着一条
    /// `102 0`。
    fn enter(state: &mut SceState, index: usize) {
        let Some(proc_id) = proc_of(index) else {
            return;
        };
        let name = DESTINATIONS
            .get(index)
            .map(|d: &Destination| d.name)
            .unwrap_or("?");
        let sce = Rc::new(sce_load_from_file(
            state.asset_mgr().vfs(),
            std::path::PathBuf::from(SCE_PATH),
        ));
        log::info!("SwitchMode: 大地图选中 {index} 「{name}」，跑 bigmap proc {proc_id}");
        if !state.call_proc_in(sce, "bigmap", proc_id) {
            log::warn!("SwitchMode: bigmap.sce 里没有 proc {proc_id}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_modes_the_scripts_actually_use() {
        // 全部 16 处 102 只传 13 和 0；15 是战斗那边走的回标题。
        assert_eq!((MODE_ADVENTURE, MODE_BIGMAP, MODE_TITLE), (0, 13, 15));
    }

    #[test]
    fn the_destination_procs_line_up_with_the_table() {
        assert_eq!(proc_of(0), Some(100));
        assert_eq!(proc_of(DESTINATIONS.len() - 1), Some(137));
    }
}
