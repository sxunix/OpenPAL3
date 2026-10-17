//! p7 菜单页共用的确认框（参考源码 `gamemainui/muidialog.cpp` 的 `DLG_YES` / `DLG_YESNO`；
//! 提示框 `DLG_MSGBOX` 见 [`super::menu_message`]）。
//!
//! - 建框时的 rect 只取左上角，框恒为 308×109，底图 `MUIDialog\Normal\DlgBackground.tga`；
//! - 按钮 90×26（`Yes0/1/2`、`No0/1/2`：常态 / 悬停 / 按下），底边上 6：`DLG_YESNO` 左右各缩 26，
//!   `DLG_YES` 居中；
//! - 字从框内 (6, 6) 再缩 8 起，16 像素一行；开头的空格按一个 8 像素算（原版按字节排版）；
//! - 弹出 600 毫秒内按钮不收（`_m_canEndDlg`）；之后回车 = 是，`DLG_YESNO` 的 Esc = 否；
//! - 点了按钮放 wa001，除非 `DoModel(false)`。

use std::time::Instant;

use crosscom::ComRc;
use radiance::comdef::IUiHost;
use shared::openpal3::asset_manager::AssetManager;

use super::ui_draw::{Painter, Pics, Rect, key_pressed};

const W: i32 = 308;
const H: i32 = 109;
const BTN_W: i32 = 90;
const BTN_H: i32 = 26;
const LOCK_SECS: f32 = 0.6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Yes,
    YesNo,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Answer {
    Yes,
    No,
}

struct Open {
    kind: Kind,
    rect: Rect,
    text: String,
    ink: [u8; 3],
    sound: bool,
    t: f32,
}

#[derive(Default)]
pub struct MuiDialog {
    open: Option<Open>,
    last: Option<Instant>,
    pics: Pics,
}

pub struct Input {
    pub mx: f32,
    pub my: f32,
    pub down: bool,
    pub click: bool,
}

impl MuiDialog {
    /// `DoModel`：在 (left, top) 弹框，`text` 用 `\n` 分行，`sound` 是点按钮时放不放 wa001。
    pub fn show(
        &mut self,
        kind: Kind,
        origin: (i32, i32),
        text: String,
        ink: [u8; 3],
        sound: bool,
    ) {
        self.open = Some(Open {
            kind,
            rect: (origin.0, origin.1, origin.0 + W, origin.1 + H),
            text,
            ink,
            sound,
            t: 0.,
        });
    }

    pub fn busy(&self) -> bool {
        self.open.is_some()
    }

    fn buttons(o: &Open) -> Vec<(Rect, &'static str, Answer)> {
        let r = o.rect;
        let top = r.3 - (BTN_H + 6);
        match o.kind {
            Kind::Yes => {
                let left = r.0 + W / 2 - BTN_W / 2;
                vec![((left, top, left + BTN_W, top + BTN_H), "Yes", Answer::Yes)]
            }
            Kind::YesNo => vec![
                (
                    (r.0 + 26, top, r.0 + 26 + BTN_W, top + BTN_H),
                    "Yes",
                    Answer::Yes,
                ),
                (
                    (r.2 - 26 - BTN_W, top, r.2 - 26, top + BTN_H),
                    "No",
                    Answer::No,
                ),
            ],
        }
    }

    /// 每帧：推进、收输入、画。返回这一帧按下的按钮。
    pub fn paint(
        &mut self,
        ui: &ComRc<IUiHost>,
        am: &AssetManager,
        register: &dyn Fn(i64, imgui::TextureId),
        canvas: (f32, f32, f32),
        input: &Input,
        sound: &dyn Fn(&str),
    ) -> Option<Answer> {
        let now = Instant::now();
        let dt = self
            .last
            .replace(now)
            .map_or(0., |t| now.duration_since(t).as_secs_f32().min(0.1));
        let o = self.open.as_mut()?;
        o.t += dt;
        let p = Painter {
            ui,
            register,
            ox: canvas.0,
            oy: canvas.1,
            scale: canvas.2,
        };
        let live = o.t >= LOCK_SECS;
        let buttons = Self::buttons(o);
        let mut answer = None;
        if live {
            for (r, _, a) in &buttons {
                if input.click && p.over(input.mx, input.my, *r) {
                    answer = Some(*a);
                }
            }
            if answer.is_none() && key_pressed(imgui::Key::Enter) {
                answer = Some(Answer::Yes);
            }
            if answer.is_none() && o.kind == Kind::YesNo && key_pressed(imgui::Key::Escape) {
                answer = Some(Answer::No);
            }
        }
        if let Some(a) = answer {
            if o.sound {
                sound("wa001");
            }
            self.open = None;
            return Some(a);
        }

        let mut names = vec!["UI\\GameMainUI\\MUIDialog\\Normal\\DlgBackground.tga".to_string()];
        for base in ["Yes", "No"] {
            for i in 0..3 {
                names.push(format!("UI\\GameMainUI\\MUIDialog\\Normal\\{base}{i}.tga"));
            }
        }
        self.pics.preload(am, &names);
        p.blit_rect(self.pics.peek(&names[0]), o.rect);
        for (k, line) in o.text.split('\n').enumerate() {
            let pad = line.chars().take_while(|c| *c == ' ').count() as i32;
            p.text(
                (o.rect.0 + 6 + 8 + 8 * pad) as f32,
                (o.rect.1 + 6 + 8 + 16 * k as i32) as f32,
                o.ink,
                line.trim_start_matches(' '),
            );
        }
        for (r, base, _) in &buttons {
            // 锁着的时候（`SetCtrl(false)`）也会亮悬停图，只是不收点击。
            let hover = p.over(input.mx, input.my, *r);
            let state = if hover && input.down && live {
                2
            } else if hover {
                1
            } else {
                0
            };
            p.blit_rect(
                self.pics.peek(&format!(
                    "UI\\GameMainUI\\MUIDialog\\Normal\\{base}{state}.tga"
                )),
                *r,
            );
        }
        None
    }
}
