//! 72 在角色头顶出现表情符号 — `FUN_00437210`.
//!
//! Blocks: the handler takes a slot from the emoji manager's six
//! (`FUN_0044ff20`, base `mgr+0x38d0`, stride 0x8e0) and waits on
//! `slot+0x44` until the animation has run. Ids outside `0 <= id < 13`
//! show nothing at all in `FUN_0044e8b0`, and there are only eleven
//! shipped sheets, so an unknown id returns immediately here rather than
//! leaving the script waiting on something that will never appear.

use crate::openpal3::directors::SceneManagerExtensions;
use crate::openpal3::scene::emoji::{
    EMOJI_ENTITY_PREFIX, build_emoji, emoji_duration, emoji_entity_name, emoji_sound,
};
use crate::openpal3::scene::{DEFAULT_MODEL_HEIGHT, RoleController};
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::{IEntity, IEntityExt, ISceneManager};

#[derive(Clone)]
pub struct SceCommandRoleEmote {
    role_id: i32,
    emoji_id: i32,
    /// Seconds left of the animation; `None` until `initialize` has run.
    remaining: Option<f32>,
    actor: Option<ComRc<IEntity>>,
}

impl std::fmt::Debug for SceCommandRoleEmote {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandRoleEmote")
            .field("role_id", &self.role_id)
            .field("emoji_id", &self.emoji_id)
            .field("remaining", &self.remaining)
            .finish()
    }
}

/// Hide every bubble already on `actor`; only one shows at a time, the
/// way the original hands out one of its six slots.
fn hide_bubbles(actor: &ComRc<IEntity>) {
    for child in actor.children() {
        if child.name().starts_with(EMOJI_ENTITY_PREFIX) {
            child.set_visible(false);
        }
    }
}

impl SceCommand for SceCommandRoleEmote {
    fn initialize(&mut self, scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        let Some(duration) = emoji_duration(self.emoji_id) else {
            log::warn!("RoleEmote: 表情 {} 没有对应图集", self.emoji_id);
            return;
        };
        let Some(actor) = scene_manager.get_resolved_role(state, self.role_id) else {
            log::warn!("RoleEmote: 找不到角色 {}", self.role_id);
            return;
        };

        hide_bubbles(&actor);

        let name = emoji_entity_name(self.emoji_id);
        let existing = actor.children().into_iter().find(|c| c.name() == name);
        let bubble = match existing {
            Some(bubble) => bubble,
            None => {
                let height = RoleController::get_role_controller(actor.clone())
                    .map(|rc| rc.inner::<RoleController>().model_height())
                    .unwrap_or(DEFAULT_MODEL_HEIGHT);
                let asset_mgr = state.asset_mgr().clone();
                let Some(bubble) = build_emoji(
                    &asset_mgr.component_factory(),
                    asset_mgr.vfs(),
                    asset_mgr.basedata_path(),
                    self.emoji_id,
                    height,
                ) else {
                    log::warn!("RoleEmote: 表情 {} 的气泡建不出来", self.emoji_id);
                    return;
                };
                actor.attach(bubble.clone());
                bubble
            }
        };
        bubble.set_visible(true);

        self.actor = Some(actor);
        self.remaining = Some(duration);

        if let Some(sound) = emoji_sound(self.emoji_id) {
            play(state, sound);
        }
    }

    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        let Some(remaining) = self.remaining.as_mut() else {
            return true;
        };

        *remaining -= delta_sec;
        if *remaining > 0. && !state.fast_forward() {
            return false;
        }

        if let Some(actor) = self.actor.take() {
            hide_bubbles(&actor);
        }
        self.remaining = None;
        true
    }
}

/// Fire and forget, the same way `SceCommandPlaySound` starts a clip —
/// the emoji's sound is not what the script waits on.
fn play(state: &mut SceState, name: &str) {
    match state.asset_mgr().load_snd_data(name) {
        Ok(data) => {
            let mut source = state.audio_engine().create_source();
            source.set_data(data, radiance::audio::Codec::Wav);
            source.play(false);
            state
                .global_state_mut()
                .add_sound_source(std::rc::Rc::new(std::cell::RefCell::new(source)));
        }
        Err(e) => log::error!("表情音效 {name} 读不到: {e:?}"),
    }
}

impl SceCommandRoleEmote {
    pub fn new(role_id: i32, emoji_id: i32) -> Self {
        Self {
            role_id,
            emoji_id,
            remaining: None,
            actor: None,
        }
    }
}
