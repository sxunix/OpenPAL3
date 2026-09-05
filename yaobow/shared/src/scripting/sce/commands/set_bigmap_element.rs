//! 125 大地图打开或关闭某个区域 — `LAB_00436bd0`.
//!
//! 处理函数只有两行：
//!
//! ```text
//! [0xdba014 + id*4] = option;        // i32 状态[38]
//! [0xdba0ac + id]   = (option > 0);  // u8  可见[38]
//! ```
//!
//! 大地图的绘制循环（0x44a366）逐个跳过 `可见[i] == 0` 的元件，只看后者；
//! 前面那份 i32 只被存读档碰（0x537d90 / 0x538ae8）。所以 **`option` 取 1
//! 还是 2 在游戏里没有区别，起作用的只有"是否 > 0"**——`bigmap.sce` 的
//! 2000 号过程「大地图开启测试」连着 38 条这个指令，值就在 0/1/2 之间。
//!
//! 这里按原样把 `option` 存进存档，可见性由
//! [`PersistentState::bigmap_element_visible`] 判。目的地表和 `id` 的含义
//! 见 [`crate::openpal3::bigmap`]。
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
