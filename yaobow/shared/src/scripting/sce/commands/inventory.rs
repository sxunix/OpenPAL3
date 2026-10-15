//! Inventory and money commands.
//!
//! Opcode meanings follow the original script command set:
//! 46 增加物品(物品ID, 个数) · 47 拿走一个物品(物品ID) · 48 增减金钱(增减值) ·
//! 49 取出金钱赋值给变量 · 78 检查玩家是否拥有某个物品.
//!
//! 46、48 在原版里都报消息栏（`FUN_00410360`），见各自的 `update`。

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
        // exe 0x435830：两个参数都要非 0，数据库里要有这件。加完（景天的剑直接装上，见
        // `PersistentState::script_add_item`）重算属性、钳一遍当前值（`FUN_0040be80(0)` + 12 次钳位，
        // 只在换剑那一支），消息栏报「getitem」、放 wa006。
        if self.item_id == 0 || self.count == 0 {
            return true;
        }
        let Some(db) = state.global_state().game_db().cloned() else {
            return true;
        };
        let name = {
            let mut p = state.global_state_mut().persistent_state_mut();
            let name = p.script_add_item(&db, self.item_id as u32, self.count as u32);
            if name.is_some() {
                if let Some(item) = db.item(self.item_id as u32) {
                    if item.item_type == 11 && item.applicable_actors[0] != 0 {
                        crate::openpal3::states::item_use::clamp_current(&mut p, 0, &db);
                    }
                }
            }
            name
        };
        if let Some(name) = name {
            state.push_lang_note("getitem", &name);
            // 0x435bb6：屏幕效果在走就不放音效。
            if !state.is_fading() {
                super::play_sfx(state, "wa006");
            }
        }
        true
    }
}

impl SceCommandAddItem {
    pub fn new(item_id: i32, count: i32) -> Self {
        Self { item_id, count }
    }
}

/// Takes exactly one away — the original command has no count parameter —
/// and strips it off anyone wearing it.
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
        // exe 0x435bf0：数据库里要有这件；装备格里穿着的清掉、背包拿走一件（`script_remove_item`），
        // 消息栏报「loseitem」，屏幕效果没在走就放 wa007。
        let Some(db) = state.global_state().game_db().cloned() else {
            return true;
        };
        let name = state
            .global_state_mut()
            .persistent_state_mut()
            .script_remove_item(&db, self.item_id as u32);
        if let Some(name) = name {
            state.push_lang_note("loseitem", &name);
            if !state.is_fading() {
                super::play_sfx(state, "wa007");
            }
        }
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
        // exe 0x437630：直接 `FUN_0040fc90(增减值)`，带消息栏和 wa006。
        state.add_money_with_note(self.delta);
        true
    }
}

impl SceCommandAddMoney {
    pub fn new(delta: i32) -> Self {
        Self { delta }
    }
}
