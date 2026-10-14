mod adv_director;

pub use adv_director::AdventureDirector;
pub(crate) use adv_director::load_par_shape;
use crosscom::ComRc;
use radiance::comdef::{IEntity, ISceneManager};

use crate::scripting::sce::SceState;

use super::{comdef::IScnSceneComponent, scene::RoleController};

pub trait SceneManagerExtensions {
    fn scn_scene(&self) -> Option<ComRc<IScnSceneComponent>>;

    fn get_resolved_role(&self, state: &SceState, role_id: i32) -> Option<ComRc<IEntity>> {
        let resolved = resolve_role_id(state, role_id);
        // 战斗舞台压在栈顶时没有 ScnScene；这时找不到人，而不是崩。
        self.scn_scene()?
            .inner::<crate::openpal3::scene::ScnScene>()
            .get_role_entity(resolved)
    }

    fn resolve_role_do<T, F: Fn(ComRc<IEntity>, &RoleController) -> T>(
        &self,
        state: &SceState,
        role_id: i32,
        action: F,
    ) -> Option<T> {
        let role = self.get_resolved_role(state, role_id);
        if let Some(r) = role {
            let role_model = RoleController::get_role_controller(r.clone()).unwrap();
            Some(action(r, role_model.inner::<RoleController>()))
        } else {
            log::error!("Cannot find role {}", role_id);
            None
        }
    }

    fn resolve_role_mut_do<T, F: Fn(ComRc<IEntity>, &RoleController) -> T>(
        &self,
        state: &SceState,
        role_id: i32,
        action: F,
    ) -> Option<T> {
        let role = self.get_resolved_role(state, role_id);
        if let Some(r) = role {
            let role_model = RoleController::get_role_controller(r.clone()).unwrap();
            Some(action(r, role_model.inner::<RoleController>()))
        } else {
            log::error!("Cannot find role {}", role_id);
            None
        }
    }
}

impl SceneManagerExtensions for ComRc<ISceneManager> {
    fn scn_scene(&self) -> Option<ComRc<IScnSceneComponent>> {
        self.scene()?
            .get_component(IScnSceneComponent::uuid())?
            .query_interface::<IScnSceneComponent>()
    }
}

fn resolve_role_id(state: &SceState, role_id: i32) -> i32 {
    if role_id == -1 {
        state.global_state().role_controlled()
    } else {
        role_id
    }
}
