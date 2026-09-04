//! 68 提示框 and 86 全屏字幕.

use std::rc::Rc;

use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{Condition, Ui};
use radiance::comdef::ISceneManager;
use radiance::input::Key;
use radiance::rendering::Sprite;

/// 68 显示提示框信息 — `LAB_00437170` → `FUN_00450bc0`.
///
/// Does not block: the note is handed to a widget that fades itself out
/// while the script runs on, so the text goes to [`SceState`] and the vm
/// draws it. `FUN_00450bc0` sizes the box `max(0x40, (strlen + 5) * 10)`
/// wide by `0x30` high and centres it on screen; the dwell time comes
/// from the widget's animation state (`+0xac = 2`), which was not read,
/// so [`NOTE_SECONDS`](crate::scripting::sce::NOTE_SECONDS) follows
/// Pal3.Unity's 2.5 s plus a 1 s fade instead. `[推断]`
#[derive(Debug, Clone)]
pub struct SceCommandNote {
    text: String,
}

impl SceCommand for SceCommandNote {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        state.show_note(self.text.clone());
        true
    }
}

impl SceCommandNote {
    pub fn new(text: String) -> Self {
        Self { text }
    }
}

/// 86 显示全屏字幕 — `LAB_00437b20`.
///
/// Blocks until the player dismisses it (`FUN_00434c40` at 0x437b76).
/// The picture is `caption\<name>.tga`, built from the format string at
/// 0x596234. `numberOfLines` reaches the widget as `+0x14` alongside a
/// hard-coded `+0x15 = 42`; what the pair does was not read, and
/// Pal3.Unity discards the parameter, so it is carried but unused here.
#[derive(Clone)]
pub struct SceCommandCaption {
    name: String,
    lines: i32,
    sprite: Option<Rc<Sprite>>,
    loaded: bool,
}

impl std::fmt::Debug for SceCommandCaption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandCaption")
            .field("name", &self.name)
            .field("lines", &self.lines)
            .field("loaded", &self.loaded)
            .finish()
    }
}

impl SceCommand for SceCommandCaption {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        if !self.loaded {
            self.loaded = true;
            self.sprite = load_caption(state, &self.name);
        }

        let Some(sprite) = self.sprite.clone() else {
            // No picture, nothing to wait for — better than blocking the
            // script on something the player cannot see.
            log::warn!("Caption: 读不到 caption\\{}.tga", self.name);
            return true;
        };

        let [screen_width, screen_height] = ui.io().display_size;
        let scale = (screen_width / sprite.width() as f32)
            .min(screen_height / sprite.height() as f32)
            .min(1.);
        let size = [
            sprite.width() as f32 * scale,
            sprite.height() as f32 * scale,
        ];

        ui.window("Caption")
            .collapsible(false)
            .title_bar(false)
            .resizable(false)
            .movable(false)
            .scroll_bar(false)
            .always_auto_resize(true)
            .position_pivot([0.5, 0.5])
            .position([screen_width / 2., screen_height / 2.], Condition::Always)
            .build(|| {
                imgui::Image::new(sprite.imgui_texture_id(), size).build(ui);
            });

        if state.fast_forward() {
            return true;
        }

        let input = state.input();
        [Key::Space, Key::Escape, Key::GamePadEast, Key::GamePadSouth]
            .iter()
            .any(|key| input.get_key_state(*key).pressed())
    }
}

fn load_caption(state: &SceState, name: &str) -> Option<Rc<Sprite>> {
    let asset_mgr = state.asset_mgr();
    let path = asset_mgr.caption_path(name);
    asset_mgr.load_sprite(path)
}

impl SceCommandCaption {
    pub fn new(name: String, lines: i32) -> Self {
        Self {
            name,
            lines,
            sprite: None,
            loaded: false,
        }
    }
}
