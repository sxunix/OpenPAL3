//! Party, favour and character-attribute commands.
//!
//! 43 增减好感(角色ID, 增减值) · 50 取出好感赋值给变量(变量, 角色ID) ·
//! 51 学会新特技(主角ID, 特技ID) · 52/53 好感最高/最低的角色赋值给变量 ·
//! 54 设置主角某项属性为满值(角色ID, 属性类型) · 153 回满全队精气神 ·
//! 202 设置主角是否在队伍中(主角ID, 1 出现 0 不出现).

use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

/// Attribute selector used by `FullRoleAtt`; indexes the same table as the
/// game database's `attribute_values`.
const ATTRIBUTE_HP: i32 = 0;
const ATTRIBUTE_SP: i32 = 1;
const ATTRIBUTE_MP: i32 = 2;

fn restore_all(state: &mut SceState, actor_id: u32) {
    if let Some(role) = state
        .global_state_mut()
        .persistent_state_mut()
        .role_mut(actor_id)
    {
        role.hp = role.max_hp;
        role.sp = role.max_sp;
        role.mp = role.max_mp;
    }
}

#[derive(Debug, Clone)]
pub struct SceCommandFavorAdd {
    actor_id: i32,
    delta: i32,
}

impl SceCommand for SceCommandFavorAdd {
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
            .add_favor(self.actor_id as u32, self.delta);
        true
    }
}

impl SceCommandFavorAdd {
    pub fn new(actor_id: i32, delta: i32) -> Self {
        Self { actor_id, delta }
    }
}

#[derive(Debug, Clone)]
pub struct SceCommandGetFavor {
    var: i16,
    actor_id: i32,
}

impl SceCommand for SceCommandGetFavor {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let favor = state
            .global_state()
            .persistent_state()
            .favor(self.actor_id as u32);
        state
            .context_mut()
            .current_proc_context_mut()
            .set_local(self.var, favor);
        true
    }
}

impl SceCommandGetFavor {
    pub fn new(var: i16, actor_id: i32) -> Self {
        Self { var, actor_id }
    }
}

/// 52 (`most = true`) and 53 (`most = false`).
#[derive(Debug, Clone)]
pub struct SceCommandGetFavorite {
    var: i16,
    most: bool,
}

impl SceCommand for SceCommandGetFavorite {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let actor_id = {
            let p_state = state.global_state().persistent_state();
            if self.most {
                p_state.favorite()
            } else {
                p_state.least_favorite()
            }
        };

        state
            .context_mut()
            .current_proc_context_mut()
            .set_local(self.var, actor_id as i32);
        true
    }
}

impl SceCommandGetFavorite {
    pub fn new(var: i16, most: bool) -> Self {
        Self { var, most }
    }
}

#[derive(Debug, Clone)]
pub struct SceCommandAddSkill {
    actor_id: i32,
    skill_id: i32,
}

impl SceCommand for SceCommandAddSkill {
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
            .learn_skill(self.actor_id as u32, self.skill_id as u32);
        true
    }
}

impl SceCommandAddSkill {
    pub fn new(actor_id: i32, skill_id: i32) -> Self {
        Self { actor_id, skill_id }
    }
}

#[derive(Debug, Clone)]
pub struct SceCommandFullRoleAtt {
    actor_id: i32,
    attribute: i32,
}

impl SceCommand for SceCommandFullRoleAtt {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let actor_id = self.actor_id as u32;
        let attribute = self.attribute;
        if let Some(role) = state
            .global_state_mut()
            .persistent_state_mut()
            .role_mut(actor_id)
        {
            match attribute {
                ATTRIBUTE_HP => role.hp = role.max_hp,
                ATTRIBUTE_SP => role.sp = role.max_sp,
                ATTRIBUTE_MP => role.mp = role.max_mp,
                // The remaining attributes have no separate "current"
                // value to top up.
                other => log::warn!("FullRoleAtt: unsupported attribute {}", other),
            }
        }
        true
    }
}

impl SceCommandFullRoleAtt {
    pub fn new(actor_id: i32, attribute: i32) -> Self {
        Self {
            actor_id,
            attribute,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SceCommandFullTeamAtt {}

impl SceCommand for SceCommandFullTeamAtt {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let team = state.global_state().persistent_state().team().to_vec();
        for actor_id in team {
            restore_all(state, actor_id);
        }
        true
    }
}

impl SceCommandFullTeamAtt {
    pub fn new() -> Self {
        Self {}
    }
}

#[derive(Debug, Clone)]
pub struct SceCommandInTeam {
    actor_id: i32,
    is_in: i32,
}

impl SceCommand for SceCommandInTeam {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let mut p_state = state.global_state_mut().persistent_state_mut();
        if self.is_in != 0 {
            p_state.add_to_team(self.actor_id as u32);
        } else {
            p_state.remove_from_team(self.actor_id as u32);
        }
        true
    }
}

impl SceCommandInTeam {
    pub fn new(actor_id: i32, is_in: i32) -> Self {
        Self { actor_id, is_in }
    }
}
