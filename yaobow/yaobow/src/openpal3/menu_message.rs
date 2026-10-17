//! p7 菜单页共用的提示框（`MUIDialog` 的 `DLG_MSGBOX`，源码 `gamemainui/muidialog.cpp`）：
//! `UI\Flex\Note` 九宫格 0.5 秒从中心 32×32 余弦缓动展开，展开完才出字；弹出 600 毫秒后点一下就关。
//! 字色 (31,251,255)，从框内 (6, 6) 起、再缩 8，加上各页自己的 `SetTextOffset`。

use std::time::Instant;

use crosscom::ComRc;
use radiance::comdef::IUiHost;
use shared::openpal3::asset_manager::AssetManager;

use super::ui_draw::{Painter, Pics, Rect};

const OPEN_SECS: f32 = 0.5;
const LOCK_SECS: f32 = 0.6;
const INK: [u8; 3] = [31, 251, 255];

#[derive(Default)]
pub struct MenuMessage {
    open: Option<(Rect, (i32, i32), String, f32)>,
    last: Option<Instant>,
    pics: Pics,
}

impl MenuMessage {
    /// 弹一个框：`rect` 是建框时的位置，`offset` 是 `SetTextOffset`，`text` 用 `\n` 分行。
    pub fn show(&mut self, rect: Rect, offset: (i32, i32), text: String) {
        self.open = Some((rect, offset, text, 0.));
    }

    pub fn busy(&self) -> bool {
        self.open.is_some()
    }

    /// 每帧：推进、收点击、画。返回框是不是还开着（开着时页面不收点击）。
    pub fn paint(
        &mut self,
        ui: &ComRc<IUiHost>,
        am: &AssetManager,
        register: &dyn Fn(i64, imgui::TextureId),
        canvas: (f32, f32, f32),
        click: bool,
    ) -> bool {
        let now = Instant::now();
        let dt = self
            .last
            .replace(now)
            .map_or(0., |t| now.duration_since(t).as_secs_f32().min(0.1));
        let Some((m, offset, text, t)) = self.open.as_mut() else {
            return false;
        };
        *t += dt;
        if *t >= LOCK_SECS && click {
            self.open = None;
            return false;
        }
        let (m, offset, text, t) = (*m, *offset, text.clone(), *t);
        let names: Vec<String> = (0..9).map(|i| format!("UI\\Flex\\Note{i}.tga")).collect();
        self.pics.preload(am, &names);
        let p = Painter {
            ui,
            register,
            ox: canvas.0,
            oy: canvas.1,
            scale: canvas.2,
        };
        let x = (t / OPEN_SECS).min(1.);
        let k = (1. - (x * std::f32::consts::PI).cos()) * 0.5;
        let (cx, cy) = ((m.0 + m.2) / 2, (m.1 + m.3) / 2);
        let lerp = |a: i32, b: i32| (a as f32 * (1. - k) + b as f32 * k) as i32;
        let r = (
            lerp(cx - 16, m.0),
            lerp(cy - 16, m.1),
            lerp(cx + 16, m.2),
            lerp(cy + 16, m.3),
        );
        let tile = self.pics.peek(&names[0]).map_or(32, |p| p.width());
        let w = tile.min((r.2 - r.0) / 2);
        let h = tile.min((r.3 - r.1) / 2);
        let rects = [
            (r.0, r.1, r.0 + w, r.1 + h),
            (r.0, r.3 - h, r.0 + w, r.3),
            (r.2 - w, r.3 - h, r.2, r.3),
            (r.2 - w, r.1, r.2, r.1 + h),
            (r.0, r.1 + h, r.0 + w, r.3 - h),
            (r.0 + w, r.3 - h, r.2 - w, r.3),
            (r.2 - w, r.1 + h, r.2, r.3 - h),
            (r.0 + w, r.1, r.2 - w, r.1 + h),
            (r.0 + w, r.1 + h, r.2 - w, r.3 - h),
        ];
        for (i, q) in rects.iter().enumerate() {
            if q.0 < q.2 && q.1 < q.3 {
                p.stretch(self.pics.peek(&names[i]), *q);
            }
        }
        if x >= 1. {
            for (k, line) in text.split('\n').enumerate() {
                p.text(
                    (m.0 + 14 + offset.0) as f32,
                    (m.1 + 14 + offset.1 + 16 * k as i32) as f32,
                    INK,
                    line,
                );
            }
        }
        true
    }
}
