//! 125 大地图打开或关闭某个区域 — `LAB_00436bd0`.
//!
//! 处理函数只有两行：
//!
//! ```text
//! [0xdba014 + id*4] = option;        // i32 状态[38]
//! [0xdba0ac + id]   = (option > 0);  // u8  新点亮[38]
//! ```
//!
//! 状态进大地图时由 `FUN_004499a0` 解释：0 不显示、**1 显示但不能去**（点了弹
//! 「不能去“地名”」）、**2 能去**。u8 那张是「新点亮」，进图闪两轮后清零，
//! 不进存档（存读档 0x537d90 / 0x538ae8 只存到 0xdba0ac 之前）。——先前以为
//! 1 和 2 没区别，是把闪烁循环错当成了绘制循环，见 `bigmap.md` 的更正。
//!
//! 这里按原样把 `option` 存进存档，[`PersistentState::bigmap_element_visible`] /
//! `bigmap_element_enterable` 按上面的规则判；「新点亮」另记一份不存档的。
//! 目的地表和 `id` 的含义见 [`crate::openpal3::bigmap`]。
//!
//! [`PersistentState::bigmap_element_visible`]:
//!     crate::openpal3::states::PersistentState::bigmap_element_visible

use crate::openpal3::bigmap::DESTINATIONS;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

#[derive(Debug, Clone)]
pub struct SceCommandSetBigMapElement {
    id: i32,
    option: i32,
}

impl SceCommand for SceCommandSetBigMapElement {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let name = DESTINATIONS
            .get(self.id as usize)
            .map(|d| d.name)
            .unwrap_or("?");
        log::debug!(
            "SetBigMapElement: 目的地 {} 「{name}」 -> {}",
            self.id,
            self.option
        );
        state
            .global_state_mut()
            .persistent_state_mut()
            .set_bigmap_element(self.id, self.option);
        true
    }
}

impl SceCommandSetBigMapElement {
    pub fn new(id: i32, option: i32) -> Self {
        Self { id, option }
    }
}
