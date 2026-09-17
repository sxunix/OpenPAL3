use crate::{
    openpal3::directors::SceneManagerExtensions,
    scripting::sce::{SceCommand, SceState},
};

use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;

#[derive(Debug, Clone)]
pub struct SceCommandObjectActive {
    object_id: i32,
    active: i32,
}

impl SceCommand for SceCommandObjectActive {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let scn = scene_manager.scn_scene().unwrap();
        let s = scn.inner::<crate::openpal3::scene::ScnScene>();
        if let Some(e) = s.get_root_object(self.object_id) {
            e.set_visible(self.active != 0);
        }
        s.set_object_active(self.object_id, self.active != 0);
        // 原版这条命令还把物件的「被改过」标记 +0x240 置 1，存档里就带上它的在场状态
        // （`LAB_004377f0` 0x43782e）。换场景回来、读档回来都照这个建。
        let (city, scene) = (s.name().to_string(), s.sub_name().to_string());
        state
            .global_state_mut()
            .persistent_state_mut()
            .set_scene_object_active(&city, &scene, self.object_id, self.active != 0);

        true
    }
}

impl SceCommandObjectActive {
    pub fn new(object_id: i32, active: i32) -> Self {
        Self { object_id, active }
    }
}
