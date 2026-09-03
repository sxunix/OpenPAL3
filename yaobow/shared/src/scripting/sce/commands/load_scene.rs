use crate::{
    openpal3::{directors::SceneManagerExtensions, scene::RoleController},
    scripting::sce::{SceCommand, SceState},
};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct SceCommandLoadScene {
    name: String,
    sub_name: String,
}

impl SceCommand for SceCommandLoadScene {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        // Must stay single-frame: with ScriptRunMode 2 the VM keeps
        // pulling the proc's following commands (RoleSetPos,
        // CameraDefault, ...) while a multi-frame command is pending, so
        // a phased load would let them run against the OLD scene and
        // leave the camera stale over the new one.
        self.load(scene_manager, state);
        true
    }
}

impl SceCommandLoadScene {
    pub fn new(name: String, sub_name: String) -> Self {
        Self { name, sub_name }
    }

    /// Put back every scene object the story has slid with opcode 90.
    ///
    /// The original persists a moved position through the object's
    /// `+0x244` flag and rebuilds the object from saved state on the way
    /// in; opcode 158 is what opts an object out of that. Without this,
    /// a bridge lowered before leaving would be back up on return.
    fn restore_moved_objects(&self, scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        let Some(scene_rc) = scene_manager.scn_scene() else {
            return;
        };
        let scene = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
        let saved: Vec<(i32, radiance::math::Vec3)> = {
            let p_state = state.global_state().persistent_state();
            scene
                .object_ids()
                .into_iter()
                .filter(|id| p_state.object_loads_from_save(*id))
                .filter_map(|id| {
                    p_state
                        .scene_object_position(&self.name, &self.sub_name, id)
                        .map(|p| (id, p))
                })
                .collect()
        };

        for (id, position) in saved {
            if let Some(entity) = scene.get_root_object(id) {
                use radiance::comdef::IEntityExt;
                entity.transform().borrow_mut().set_position(&position);
            }
        }
    }

    /// Perform the actual scene swap immediately (no transition).
    /// Also used by `SceCommandEntryRow`, which needs the load done
    /// within its single update call.
    pub(crate) fn load(&self, scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        let last_scene = scene_manager.scn_scene();
        let cpk_changed = last_scene
            .and_then(|s| Some(s.inner::<crate::openpal3::scene::ScnScene>().name() != &self.name))
            .or(Some(true))
            .unwrap();

        log::info!("sce: LoadScene {}/{}", self.name, self.sub_name);
        scene_manager.pop_scene();
        scene_manager.push_scene(state.asset_mgr().load_scn(&self.name, &self.sub_name));
        let e = scene_manager.get_resolved_role(state, -1).unwrap();
        let r = RoleController::get_role_controller(e.clone()).unwrap();
        r.inner::<RoleController>().set_active(true);

        state
            .global_state_mut()
            .persistent_state_mut()
            .set_scene_name(self.name.clone(), self.sub_name.clone());

        self.restore_moved_objects(scene_manager.clone(), state);
        if cpk_changed {
            let sce = Rc::new(state.asset_mgr().load_sce(&self.name));
            state.context_mut().set_sce(sce, self.name.clone());
            state.global_state_mut().bgm_source().stop();
            state.global_state_mut().play_default_bgm();
        }

        state.try_call_proc_by_name(&format!("_{}_{}", self.name, self.sub_name));

        // The original engine fades around every scene load itself — the
        // door procs carry no Fade commands (verified in Q01). When the
        // script hasn't faded the screen out, snap to black now and let
        // the VM ease back in; a script-managed curtain (already at 1)
        // is left alone.
        if state.curtain() < 1. {
            state.set_curtain(1.);
            state.set_auto_fade_in(true);
        }
    }
}
