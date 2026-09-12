//! 68 提示框 and 86 全屏字幕.

use std::rc::Rc;

use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;
use radiance::rendering::Sprite;

/// 68 显示提示框信息 — `LAB_00437170` → `FUN_00450bc0`.
///
/// Does not block: the note is handed to a widget that hides itself after
/// [`NOTE_SECONDS`](crate::scripting::sce::NOTE_SECONDS) while the script
/// runs on, so the text goes to [`SceState`] and the vm draws it. Layout
/// and timing: `switch/re/notes/note_box.md`.
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

/// 86 显示全屏字幕 — `LAB_00437b20`。
///
/// 图是 `caption\<name>.tga`（0x596234），交给场景界面里的常驻控件 0xdba0d4
/// （`FUN_0044a970(路径, 行数)`）。那是一张**竖排、从右往左一列一列显出来**的诗句：
///
/// * rect 是 `(0, 50, 760, 600)`（`0x44f3bb`），初始化时下边改成 `50 + 贴图高`；
/// * 第 k 列宽 42（`+0x54`），在 `x2 − 42(k+1) .. x2 − 42k`，贴图 uv 同样从右往左
///   切（`FUN_0044aa20`）；一共画 `行数 + 1` 列；
/// * 每帧（`0x44a820`）计时 += dt，还没出完时每满 2.0 秒推进一列；正在出的那列
///   alpha = 计时 / 2 × 255，出过的是 255（`0x44a870`）；
/// * 全出完后再停 5.0 秒，之后 alpha = 1 − (计时 − 5) / 3，淡没了就把控件藏起来。
///
/// 脚本等的是这个控件的等待任务（0xdba118 = 控件 + 0x44），「完成」就是控件不可见
/// （`0x44ab00` 读 `+0x38`）——**不等按键**，原版也没有跳过。
#[derive(Clone)]
pub struct SceCommandCaption {
    name: String,
    lines: i32,
    sprite: Option<Rc<Sprite>>,
    loaded: bool,
    /// 正在出第几列（`+0x58`）。
    column: i32,
    /// 计时（`+0x5c`）。
    timer: f32,
}

impl std::fmt::Debug for SceCommandCaption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandCaption")
            .field("name", &self.name)
            .field("lines", &self.lines)
            .field("column", &self.column)
            .finish()
    }
}

/// 控件 rect（`0x44f3bb`）的左上和右边。
const CAPTION_TOP: i32 = 50;
const CAPTION_RIGHT: i32 = 760;
/// 一列宽（`+0x54 = 0x2a`）。
const COLUMN_W: i32 = 42;
/// 一列淡入用几秒（0x56afa0）。
const COLUMN_SECONDS: f32 = 2.0;
/// 出完后停几秒（0x56a744）、淡出速度（0x56afcc = 1/3）。
const HOLD_SECONDS: f32 = 5.0;
const FADE_RATE: f32 = 1. / 3.;

impl SceCommandCaption {
    pub fn new(name: String, lines: i32) -> Self {
        Self {
            name,
            lines,
            sprite: None,
            loaded: false,
            column: 0,
            timer: 0.,
        }
    }

    /// `0x44a820`。
    fn step(&mut self, dt: f32) {
        self.timer += dt;
        if self.column <= self.lines && self.timer > COLUMN_SECONDS {
            self.column += 1;
            self.timer = 0.;
        }
    }

    /// 每一列的 alpha（0..=255），`None` 表示已经淡没、该收了（`0x44a870`）。
    fn alphas(&self) -> Option<Vec<u8>> {
        let n = (self.lines.max(0) + 1) as usize;
        if self.column <= self.lines {
            let current = ((self.timer / COLUMN_SECONDS) * 255.) as i32;
            Some(
                (0..n as i32)
                    .map(|k| match k.cmp(&self.column) {
                        std::cmp::Ordering::Less => 255,
                        std::cmp::Ordering::Equal => current.clamp(0, 255) as u8,
                        std::cmp::Ordering::Greater => 0,
                    })
                    .collect(),
            )
        } else {
            let f = (1. - (self.timer - HOLD_SECONDS).max(0.) * FADE_RATE).max(0.);
            let a = (f * 255.) as i32;
            (a > 0).then(|| vec![a as u8; n])
        }
    }
}

impl SceCommand for SceCommandCaption {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if !self.loaded {
            self.loaded = true;
            self.sprite = load_caption(state, &self.name);
        }
        let Some(sprite) = self.sprite.clone() else {
            log::warn!("Caption: 读不到 caption\\{}.tga", self.name);
            return true;
        };
        if state.fast_forward() {
            return true;
        }

        self.step(delta_sec);
        let Some(alphas) = self.alphas() else {
            return true;
        };
        let (tw, th) = (sprite.width().max(1) as i32, sprite.height().max(1) as i32);
        super::appraise::overlay(ui, "Caption", |ui, at, _scale| {
            let list = ui.get_window_draw_list();
            for (k, a) in alphas.iter().enumerate() {
                if *a == 0 {
                    continue;
                }
                let k = k as i32;
                let (x0, x1) = (
                    CAPTION_RIGHT - COLUMN_W * (k + 1),
                    CAPTION_RIGHT - COLUMN_W * k,
                );
                let u1 = 1. - (COLUMN_W * k) as f32 / tw as f32;
                let u0 = 1. - (COLUMN_W * (k + 1)) as f32 / tw as f32;
                list.add_image(
                    sprite.imgui_texture_id(),
                    at(x0, CAPTION_TOP),
                    at(x1, CAPTION_TOP + th),
                )
                .uv_min([u0, 0.])
                .uv_max([u1, 1.])
                .col([1., 1., 1., *a as f32 / 255.])
                .build();
            }
        });
        false
    }
}

fn load_caption(state: &SceState, name: &str) -> Option<Rc<Sprite>> {
    let asset_mgr = state.asset_mgr();
    let path = asset_mgr.caption_path(name);
    asset_mgr.load_sprite(path)
}

#[cfg(test)]
mod caption_tests {
    use super::*;

    #[test]
    fn columns_appear_right_to_left_then_the_whole_thing_fades() {
        let mut c = SceCommandCaption::new("1A".into(), 2);
        assert_eq!(c.alphas(), Some(vec![0, 0, 0]));
        c.step(1.0);
        assert_eq!(c.alphas(), Some(vec![127, 0, 0]), "第一列淡入到一半");
        c.step(1.01);
        assert_eq!(c.alphas(), Some(vec![255, 0, 0]));
        c.step(2.01);
        c.step(2.01);
        assert_eq!(c.column, 3, "三列都出完");
        assert_eq!(c.alphas(), Some(vec![255, 255, 255]));
        c.step(5.0 + 1.5);
        assert_eq!(c.alphas(), Some(vec![127, 127, 127]), "停 5 秒后 3 秒淡出");
        c.step(1.6);
        assert_eq!(c.alphas(), None);
    }
}
