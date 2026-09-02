//! Inventory and money commands.
//!
//! Opcode meanings follow the original script command set:
//! 46 增加物品(物品ID, 个数) · 47 拿走一个物品(物品ID) · 48 增减金钱(增减值) ·
//! 49 取出金钱赋值给变量 · 78 检查玩家是否拥有某个物品.

use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

#[derive(Debug, Clone)]
pub struct SceCommandAddItem {
    item_id: i32,
    count: i32,
}

impl SceCommand for SceCommandAddItem {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        if self.count > 0 {
            state
                .global_state_mut()
                .persistent_state_mut()
                .add_item(self.item_id as u32, self.count as u32);
        }
        true
    }
}

impl SceCommandAddItem {
    pub fn new(item_id: i32, count: i32) -> Self {
        Self { item_id, count }
    }
}

/// Takes exactly one away — the original command has no count parameter.
#[derive(Debug, Clone)]
pub struct SceCommandRemoveItem {
    item_id: i32,
}

impl SceCommand for SceCommandRemoveItem {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        state
            .global_state_mut()
            .persistent_state_mut()
            .remove_item(self.item_id as u32, 1);
        true
    }
}

impl SceCommandRemoveItem {
    pub fn new(item_id: i32) -> Self {
        Self { item_id }
    }
}

#[derive(Debug, Clone)]
pub struct SceCommandAddMoney {
    delta: i32,
}

impl SceCommand for SceCommandAddMoney {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        state
            .global_state_mut()
            .persistent_state_mut()
            .add_money(self.delta);
        true
    }
}

impl SceCommandAddMoney {
    pub fn new(delta: i32) -> Self {
        Self { delta }
    }
}
