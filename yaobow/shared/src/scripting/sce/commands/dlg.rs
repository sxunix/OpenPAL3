use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{MouseButton, Ui};
use radiance::{comdef::ISceneManager, input::Key};

#[derive(Debug, Clone)]
pub struct SceCommandDlg {
    text: String,
    dlg_end: bool,
    adv_input_enabled: bool,
    shown: f32,
}

/// `auto_advance_dialog` from the config (testing aid), read once.
fn auto_advance_after() -> f32 {
    static AFTER: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *AFTER.get_or_init(|| {
        crate::config::YaobowConfig::load().auto_advance_dialog_for(crate::GameType::PAL3)
    })
}

impl SceCommand for SceCommandDlg {
    fn initialize(&mut self, _scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        self.adv_input_enabled = state.global_state_mut().adv_input_enabled();
        state.global_state_mut().set_adv_input_enabled(false);
    }

    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if self.dlg_end {
            // state.global_state_mut().set_adv_input_enabled(self.adv_input_enabled);
            state.clear_dialog_avatar();
            state.close_dialog();

            return true;
        }

        state.render_dialog(self.text.as_ref(), delta_sec);
        state.set_dialog_text(self.text.as_ref());

        // delay set_adv_input to the next frame so that the last kay pressed
        // won't trigger the sce proc again.
        //
        // Under agent fast-forward we don't wait for the player to advance
        // the box — end it immediately so automated runs don't stall on
        // dialog. (Still resolves on the next frame, mirroring the keypress
        // path, so adv-input re-enable stays correctly deferred.)
        //
        // A press first turns the page when the text runs past the box
        // (`UIRoleDialog::Update`: `IfEmpty` ? close : `NextPage`).
        self.shown += delta_sec;
        let after = auto_advance_after();
        let auto = after > 0. && self.shown >= after;
        if auto {
            self.shown = 0.;
        }
        let clicked = auto
            || state.input().get_key_state(Key::Space).pressed()
            || state.input().get_key_state(Key::GamePadEast).pressed()
            || state.input().get_key_state(Key::GamePadSouth).pressed()
            || ui.is_mouse_released(MouseButton::Left);
        self.dlg_end = state.fast_forward() || (clicked && !state.next_dialog_page());

        false
    }
}

impl SceCommandDlg {
    pub fn new(text: String) -> Self {
        Self {
            text: text.replace("\\n", "\n"),
            dlg_end: false,
            adv_input_enabled: false,
            shown: 0.,
        }
    }
}
