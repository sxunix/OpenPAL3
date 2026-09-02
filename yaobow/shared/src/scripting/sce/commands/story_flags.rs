//! Opcodes that only move story state around: variable arithmetic, the
//! menu pages the plot unlocks, and the two characters whose appearance
//! or behaviour the script switches.

use crate::openpal3::directors::SceneManagerExtensions;
use crate::openpal3::states::persistent_state::Feature;
use crate::scripting::sce::commands::vars;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::{IEntityExt, ISceneManager};

/// 130 变量 A 减去 B，结果取绝对值 — `LAB_004363e0`:
/// `sub` at 0x4363fb, then `jns` / `neg` at 0x436402.
#[derive(Debug, Clone)]
pub struct SceCommandVarDistract {
    lhs: i16,
    rhs: i16,
}

impl SceCommand for SceCommandVarDistract {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let difference = vars::read(state, self.lhs).wrapping_sub(vars::read(state, self.rhs));
        vars::write(state, self.lhs, difference.wrapping_abs());
        true
    }
}

impl SceCommandVarDistract {
    pub fn new(lhs: i16, rhs: i16) -> Self {
        Self { lhs, rhs }
    }
}

/// 145 把角色等级设成队伍平均等级加一个增量 — `FUN_00438840`.
///
/// The average covers every *other* active party member (0x43886d–
/// 0x43888c skips the target and tests the in-team byte), the division
/// is integer, and nothing happens when nobody else is in the party.
/// The result never lowers a character: `max(current, average + delta)`
/// at 0x4388a1.
#[derive(Debug, Clone)]
pub struct SceCommandAverageLevel {
    actor_id: i32,
    delta: i32,
}

impl SceCommand for SceCommandAverageLevel {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let actor_id = self.actor_id as u32;
        let (sum, count, current) = {
            let p_state = state.global_state().persistent_state();
            let others: Vec<i32> = p_state
                .team()
                .iter()
                .filter(|id| **id != actor_id)
                .filter_map(|id| p_state.role(*id))
                .map(|role| role.level)
                .collect();
            let current = p_state.role(actor_id).map(|r| r.level).unwrap_or(0);
            (others.iter().sum::<i32>(), others.len() as i32, current)
        };

        if count == 0 {
            return true;
        }
        let level = current.max(sum / count + self.delta);
        state
            .global_state_mut()
            .persistent_state_mut()
            .set_level(actor_id, level);
        true
    }
}

impl SceCommandAverageLevel {
    pub fn new(actor_id: i32, delta: i32) -> Self {
        Self { actor_id, delta }
    }
}

/// The three 龙葵 dresses the script looks for, in the order
/// `LAB_00438a30` searches them (0x438a4b onwards). The index of the one
/// found is what lands in the variable.
const LONGKUI_DRESSES: [u32; 3] = [6308, 6309, 6310];

/// 149 把背包里的服装给龙葵 — `LAB_00438a30`.
///
/// Finding nothing leaves the variable **untouched** (0x438a93 returns
/// early), which is a real difference from every other "assign to
/// variable" opcode. The dress is consumed.
#[derive(Debug, Clone)]
pub struct SceCommandGiveCloth {
    var: i16,
}

impl SceCommand for SceCommandGiveCloth {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let found = {
            let p_state = state.global_state().persistent_state();
            LONGKUI_DRESSES
                .iter()
                .position(|item| p_state.item_count(*item) > 0)
        };

        let Some(index) = found else {
            return true;
        };

        let item = LONGKUI_DRESSES[index];
        let count = state.global_state().persistent_state().item_count(item);
        state
            .global_state_mut()
            .persistent_state_mut()
            .remove_item(item, count);
        vars::write(state, self.var, index as i32);
        true
    }
}

impl SceCommandGiveCloth {
    pub fn new(var: i16) -> Self {
        Self { var }
    }
}

/// 109 魔剑养成 · 110 降妖谱 · 138 魔剑技 — `LAB_00436fb0` /
/// `LAB_00436fe0` / `LAB_00437010`, each a `setne` into one of three
/// adjacent bytes that the save file carries.
#[derive(Debug, Clone)]
pub struct SceCommandEnableFeature {
    feature: Feature,
    enable: i32,
}

impl SceCommand for SceCommandEnableFeature {
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
            .set_feature(self.feature, self.enable != 0);
        true
    }
}

impl SceCommandEnableFeature {
    pub fn sword_purifying(enable: i32) -> Self {
        Self {
            feature: Feature::SwordPurifying,
            enable,
        }
    }

    pub fn justice_book(enable: i32) -> Self {
        Self {
            feature: Feature::JusticeBook,
            enable,
        }
    }

    pub fn sword_skill(enable: i32) -> Self {
        Self {
            feature: Feature::SwordSkill,
            enable,
        }
    }
}

/// 111 学会合成术 — `LAB_00437040`.
///
/// The original opens a modal picker (`FUN_0047e030`) with one row per
/// party member and blocks until it closes. There is no compose system
/// here yet, so the recipe is recorded and the script carries on; the
/// picker is what is missing, not the recipe.
#[derive(Debug, Clone)]
pub struct SceCommandLearnCompose {
    recipe_id: i32,
}

impl SceCommand for SceCommandLearnCompose {
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
            .learn_compose(self.recipe_id as u32);
        true
    }
}

impl SceCommandLearnCompose {
    pub fn new(recipe_id: i32) -> Self {
        Self { recipe_id }
    }
}

/// 42 设置龙葵形象 — `FUN_00436b40`: model `"105"` for blue (0x436b80),
/// `"106"` for red (0x436b63), then the current action is re-applied so
/// the swap shows at once.
#[derive(Debug, Clone)]
pub struct SceCommandLongKuiMode {
    mode: i32,
}

impl SceCommand for SceCommandLongKuiMode {
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
            .set_longkui_mode(self.mode);
        // Swapping the loaded model needs a reload of ROLE_2 that the
        // scene does not offer yet; the mode is recorded so the right
        // model can be chosen the next time the scene builds her.
        log::warn!("LongKuiMode: 记下形象 {}，模型热替换未实现", self.mode);
        true
    }
}

impl SceCommandLongKuiMode {
    pub fn new(mode: i32) -> Self {
        Self { mode }
    }
}

/// 88 设置花盈的行为模式 — `LAB_00437b90` → `FUN_0040f060`: 0 隐藏,
/// 1 跟随雪见, 2 单飞. Only the visibility half is reproducible here;
/// there is no follower logic to switch.
#[derive(Debug, Clone)]
pub struct SceCommandHuaYingMode {
    mode: i32,
}

/// 花盈 is `ROLE_5` (`ScnScene::map_role_id` maps 5 → model 109).
const HUAYING_ROLE_ID: i32 = 5;

impl SceCommand for SceCommandHuaYingMode {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        state
            .global_state_mut()
            .persistent_state_mut()
            .set_huaying_mode(self.mode);

        let visible = self.mode != 0;
        scene_manager
            .resolve_role_mut_do(state, HUAYING_ROLE_ID, |_e, role| role.set_active(visible));
        true
    }
}

impl SceCommandHuaYingMode {
    pub fn new(mode: i32) -> Self {
        Self { mode }
    }
}

#[cfg(test)]
mod tests {
    use super::LONGKUI_DRESSES;

    #[test]
    fn the_dress_order_is_the_one_the_binary_searches() {
        // 0x18a4 / 0x18a5 / 0x18a6, in that order — the index of the
        // first match is what the script reads back.
        assert_eq!(LONGKUI_DRESSES, [0x18a4, 0x18a5, 0x18a6]);
    }
}

/// 205 RoleOverlap — `LAB_00437ae0`, whose whole body is one store into
/// `role+0x5b0` (0x437b0c). No wait, no other effect.
#[derive(Debug, Clone)]
pub struct SceCommandRoleOverlap {
    role_id: i32,
    value: i32,
}

impl SceCommand for SceCommandRoleOverlap {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let value = self.value;
        scene_manager.resolve_role_mut_do(state, self.role_id, |_e, role| role.set_overlap(value));
        true
    }
}

impl SceCommandRoleOverlap {
    pub fn new(role_id: i32, value: i32) -> Self {
        Self { role_id, value }
    }
}

/// 206 设置角色缩放 — `LAB_00434e90` → `FUN_00419030`.
///
/// The original does not block: it hands the role's action slot a
/// two-second cosine ease (`role+0x4cc = 9`, per-frame case 9) and lets
/// the script run on. yaobow has no per-role tween slot, so the target
/// scale is applied at once. Script timing is preserved — which is what
/// the script can observe — and the ease is what is missing.
#[derive(Debug, Clone)]
pub struct SceCommandRoleScale {
    role_id: i32,
    scale: f32,
    applied: f32,
}

impl SceCommand for SceCommandRoleScale {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        // `Transform::scale` multiplies, so undo whatever this command
        // applied before scaling to the new target.
        let factor = self.scale / self.applied;
        self.applied = self.scale;
        if let Some(entity) = scene_manager.get_resolved_role(state, self.role_id) {
            entity
                .transform()
                .borrow_mut()
                .scale(&radiance::math::Vec3::new(factor, factor, factor));
        }
        true
    }
}

impl SceCommandRoleScale {
    pub fn new(role_id: i32, scale: f32) -> Self {
        Self {
            role_id,
            scale,
            applied: 1.,
        }
    }
}

/// 150 预加载角色动作 — `LAB_00438b00` → `FUN_0041b9f0`, which asks the
/// model for the action's duration and throws the answer away. The call
/// exists only to pull the `.mv3` into the resource cache, so there is no
/// game state to reproduce.
#[derive(Debug, Clone)]
pub struct SceCommandLoadAction {
    role_id: i32,
    action: String,
}

impl SceCommand for SceCommandLoadAction {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        _state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        log::debug!(
            "LoadAction: 角色 {} 动作 {} — 原版只做资源预热",
            self.role_id,
            self.action
        );
        true
    }
}

impl SceCommandLoadAction {
    pub fn new(role_id: i32, action: String) -> Self {
        Self { role_id, action }
    }
}
