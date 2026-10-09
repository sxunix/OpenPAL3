//! 65 对话选择框（`LAB_00436940`）。
//!
//! 原版的选择框是场景界面里一个常驻对象（0xdba260），在场景界面初始化里由
//! `FUN_00451980(0x3f3, (0, 0, 414, 125), 父窗口, "ui\dlg\selbk.tga")` 建出：外框是
//! `ui\flex\note0..8.tga` 九宫格（FUN_004450a0 按 `"%s%d.tga"` 拼），里面一个名叫
//! `_seldlg` 的列表控件，行高 20（`FUN_0043f640(0x14)`），高亮条 `ui\scene\SelPic.tga`
//! （128×16），文字色 0xfffafa，接近白。
//!
//! * 65 先 `FUN_00451ad0` 清空列表、把 `+0x54`（最长一项的字节数）置 −1，再对每一项
//!   `FUN_00451af0(文本, 序号)` 加进列表并更新 `+0x54`，最后调虚表 0 号槽 0x451b40 显示。
//! * 0x451b40 排版：
//!   - 框高 = (项数 + 2) × 20，框宽 = (最长字节数 + 6) × 10（GBK，一个汉字两字节）；
//!   - 外框和框同尺寸；列表是框宽、框高各减 20，`FUN_00445db0` 居中在框里，也就是四边各内缩 10；
//!   - 框本身 `FUN_00445db0(0)` 居中在 800×600 屏幕上。
//! * 选中时列表发 0x3ef 事件，6 号槽 0x451aa0 把序号存进 `+0x50` 并收起框；66 取的就是它。
//!
//! 九宫格的排法和夹取规则照 `FUN_00445500` 来，见 [`frame_pieces`]。
//!
//! 行内文字的位置和高亮条的矩形都照列表控件画行那段（`0x43e8e0` 起）来，
//! 见 [`row_rect`] / [`highlight_rect`]。
//!
//! 和原版有意不同的一处（不是没查清）：原版开场不高亮任何一项、靠鼠标悬停，
//! 移植端开场先高亮第一项，手柄才能直接选。
//!
//! 操作：十字键 / 方向键上下移动，A / 空格确认；鼠标（触屏）悬停高亮、点击选中；
//! 数字键 1..9、0 直接选（旧行为保留）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::scripting::sce::commands::appraise::{overlay, virtual_mouse};
use crate::scripting::sce::commands::encamp::draw_sprite;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{MouseButton, Ui};
use radiance::imgui::{GameFontSize, game_font};
use radiance::rendering::Sprite;
use radiance::{comdef::ISceneManager, input::Key};

lazy_static::lazy_static! {
    pub static ref KEY_NUM_MAP: HashMap<Key, i32> = create_key_num_hashmap();
}

type Rect = (i32, i32, i32, i32);

const SCREEN_W: i32 = 800;
const SCREEN_H: i32 = 600;
/// 列表的行高（`FUN_0043f640(0x14)`）。
pub const ROW_HEIGHT: i32 = 20;
/// 列表相对框四边的内缩（框宽、框高各减 20 后居中）。
pub const LIST_INSET: i32 = 10;
/// 行内文字相对列表左边的缩进。列表控件画行时
/// （`0x43e95a` / `0x43e91e`）文字原点是
/// `(左 + [+0x49c] + 8, 上 + [+0x4a0] + 8 + 行号·行高)`，
/// 两个偏移字段在这个框里是 0，所以就是 8。
pub const TEXT_INSET: i32 = 8;
/// 列表上边到第一行的内边距，和 [`TEXT_INSET`] 是同一个 8。
pub const LIST_PAD: i32 = 8;
/// 九宫格每块的边长（`note0..8.tga` 都是 32×32）。
pub const FRAME_PIECE: i32 = 32;
/// 高亮条贴图 `SelPic.tga` 的高（128×16）。**画出来不是这个高**——
/// 原版把它拉到 `行高 − 2`，见 [`highlight_rect`]。
pub const HIGHLIGHT_TEX_HEIGHT: i32 = 16;
/// 0xfffafa。
const TEXT_COLOR: [f32; 4] = [1.0, 0.98, 0.98, 1.0];

/// 一项在 GBK 下的字节数（`FUN_00451af0` 拿 strlen 更新 `+0x54`）。
pub fn gbk_len(text: &str) -> usize {
    use encoding::Encoding;
    encoding::all::GBK
        .encode(text, encoding::EncoderTrap::Replace)
        .map(|b| b.len())
        .unwrap_or_else(|_| text.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum())
}

/// 0x451b40：框的 rect（800×600 坐标）。
pub fn box_rect(items: &[String]) -> Rect {
    let longest = items.iter().map(|s| gbk_len(s)).max().unwrap_or(0) as i32;
    let w = (longest + 6) * 10;
    let h = (items.len() as i32 + 2) * ROW_HEIGHT;
    let x0 = (SCREEN_W - w) / 2;
    let y0 = (SCREEN_H - h) / 2;
    (x0, y0, x0 + w, y0 + h)
}

/// 列表区：框四边各内缩 10。
pub fn list_rect(b: Rect) -> Rect {
    (
        b.0 + LIST_INSET,
        b.1 + LIST_INSET,
        b.2 - LIST_INSET,
        b.3 - LIST_INSET,
    )
}

/// 第 `k` 行。第一行从列表上边再下 [`LIST_PAD`] 开始（`0x43e95a`
/// 的 `lea edi, [上 + [+0x4a0] + 8]`），之后逐行加行高。
pub fn row_rect(area: Rect, k: usize) -> Rect {
    let y = area.1 + LIST_PAD + k as i32 * ROW_HEIGHT;
    (area.0, y, area.2, y + ROW_HEIGHT)
}

/// 高亮条（`0x43eab4` 起算的那个矩形）：左边留 4、右边留 2（原版还要再减
/// 滚动条宽 `[+0x494]`，这个框没有滚动条），纵向**和行顶齐平**、高
/// `行高 − 2`，底再夹到列表下边。贴图会被拉到这个高度。
pub fn highlight_rect(row: Rect, area: Rect) -> Rect {
    let y1 = (row.1 + ROW_HEIGHT - 2).min(area.3);
    (row.0 + 4, row.1, row.2 - 2, y1)
}

/// 虚拟坐标 `(x, y)` 落在第几行。
pub fn row_at(items: usize, b: Rect, x: f32, y: f32) -> Option<usize> {
    let area = list_rect(b);
    if x < area.0 as f32 || x >= area.2 as f32 || y < area.1 as f32 {
        return None;
    }
    let top = (area.1 + LIST_PAD) as f32;
    if y < top {
        return None;
    }
    let k = ((y - top) / ROW_HEIGHT as f32) as usize;
    (k < items).then_some(k)
}

/// 九宫格九块的 rect。排法照 `FUN_00445500` 逐块算出来的矩形：
/// **0 左上、1 左下、2 右下、3 右上、4 左、5 下、6 右、7 上、8 中**。
///
/// 块尺寸只有一个值（控件 `+0xb8`，取贴图 0 的 `+0x18`，这里九张都是
/// 32×32），宽高**各自再夹到矩形的一半**：
/// `pw = min(块, (右−左)/2)`、`ph = min(块, (下−上)/2)`。框不够大时原版是
/// 把角块画小、让两边在中线相接，不是让它们重叠——一项的菜单框正好只有
/// 60 px 高（`(1+2)×20`）会走到这一支。中间那块只在还有剩余空间时才画
/// （`0x4456f2` 的 `if (x0 < x1 && y0 < y1)`）。
pub fn frame_pieces(b: Rect) -> [Rect; 9] {
    let (x0, y0, x1, y1) = b;
    let pw = FRAME_PIECE.min((x1 - x0) / 2);
    let ph = FRAME_PIECE.min((y1 - y0) / 2);
    let (ix0, iy0) = (x0 + pw, y0 + ph);
    let (ix1, iy1) = (x1 - pw, y1 - ph);
    [
        (x0, y0, ix0, iy0),
        (x0, iy1, ix0, y1),
        (ix1, iy1, x1, y1),
        (ix1, y0, x1, iy0),
        (x0, iy0, ix0, iy1),
        (ix0, iy1, ix1, y1),
        (ix1, iy0, x1, iy1),
        (ix0, y0, ix1, iy0),
        if ix0 < ix1 && iy0 < iy1 {
            (ix0, iy0, ix1, iy1)
        } else {
            (ix0, iy0, ix0, iy0)
        },
    ]
}

#[derive(Clone)]
pub struct SceCommandDlgSel {
    list: Vec<String>,
    cursor: usize,
    last_mouse: (f32, f32),
    frame: Vec<Option<Rc<Sprite>>>,
    select: Option<Rc<Sprite>>,
}

impl std::fmt::Debug for SceCommandDlgSel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandDlgSel")
            .field("list", &self.list)
            .field("cursor", &self.cursor)
            .finish()
    }
}

impl SceCommand for SceCommandDlgSel {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        if state.fast_forward() {
            // PAL3 has no dialog-choice buffering yet (`/v1/dialog/choose`
            // is not_implemented). Under agent fast-forward, pick the first
            // option so automated runs don't stall waiting on input.
            return self.choose(state, 0);
        }

        let n = self.list.len();
        let b = box_rect(&self.list);
        self.draw(ui, b);
        if n == 0 {
            state.set_dialog_choices(self.list.clone());
            return false;
        }

        let (number, up, down, confirm) = {
            let input = state.input();
            let pressed = |k: Key| input.get_key_state(k).pressed();
            let number = KEY_NUM_MAP
                .iter()
                .find(|(k, _)| pressed(**k))
                .map(|(_, v)| *v as usize);
            (
                number,
                pressed(Key::Up) || pressed(Key::GamePadDPadUp),
                pressed(Key::Down) || pressed(Key::GamePadDPadDown),
                pressed(Key::Space) || pressed(Key::GamePadEast),
            )
        };
        if let Some(sel) = number.filter(|s| *s < n) {
            return self.choose(state, sel);
        }
        if up {
            self.cursor = (self.cursor + n - 1) % n;
        }
        if down {
            self.cursor = (self.cursor + 1) % n;
        }
        // 鼠标只在动过之后才接管高亮，免得停在框上时盖掉十字键的选择。
        let (mx, my) = virtual_mouse(ui);
        if (mx, my) != self.last_mouse {
            self.last_mouse = (mx, my);
            if let Some(k) = row_at(n, b, mx, my) {
                self.cursor = k;
            }
        }
        if ui.is_mouse_clicked(MouseButton::Left) {
            if let Some(k) = row_at(n, b, mx, my) {
                return self.choose(state, k);
            }
        }
        if confirm {
            return self.choose(state, self.cursor.min(n - 1));
        }

        state.set_dialog_choices(self.list.clone());
        false
    }

    fn initialize(&mut self, _scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        let asset_mgr = state.asset_mgr();
        let tex = |rel: &str| asset_mgr.load_sprite(PathBuf::from("/basedata/basedata").join(rel));
        self.frame = (0..9)
            .map(|i| tex(&format!("ui/flex/note{i}.tga")))
            .collect();
        self.select = tex("ui/scene/SelPic.tga");
    }
}

impl SceCommandDlgSel {
    pub fn new(mut list: Vec<String>) -> Self {
        list.reverse();
        Self {
            list,
            cursor: 0,
            last_mouse: (-1., -1.),
            frame: vec![],
            select: None,
        }
    }

    fn choose(&mut self, state: &mut SceState, sel: usize) -> bool {
        state
            .context_mut()
            .current_proc_context_mut()
            .set_dlgsel(sel as i32);
        state.close_dialog();
        true
    }

    fn draw(&self, ui: &Ui, b: Rect) {
        overlay(ui, "DlgSel", |ui, at, scale| {
            let list = ui.get_window_draw_list();
            for (k, r) in frame_pieces(b).iter().enumerate() {
                draw_sprite(&list, self.frame.get(k).and_then(|t| t.as_deref()), *r, at);
            }
            let area = list_rect(b);
            let _font = game_font(GameFontSize::SMALL).map(|f| ui.push_font(f));
            for (k, text) in self.list.iter().enumerate() {
                let row = row_rect(area, k);
                if k == self.cursor {
                    draw_sprite(&list, self.select.as_deref(), highlight_rect(row, area), at);
                }
                // 文字顶就在行顶（`FUN_00540fd0` 拿 y 当上边、y + 字号当下边），
                // 不另做竖直居中。
                let [sx, sy] = at(row.0 + TEXT_INSET, row.1);
                list.add_text([sx, sy], TEXT_COLOR, text);
            }
        });
    }
}

fn create_key_num_hashmap() -> HashMap<Key, i32> {
    let mut map = HashMap::new();
    map.insert(Key::Num1, 0);
    map.insert(Key::Num2, 1);
    map.insert(Key::Num3, 2);
    map.insert(Key::Num4, 3);
    map.insert(Key::Num5, 4);
    map.insert(Key::Num6, 5);
    map.insert(Key::Num7, 6);
    map.insert(Key::Num8, 7);
    map.insert(Key::Num9, 8);
    map.insert(Key::Num0, 9);
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn frame_pieces_shrink_instead_of_overlapping() {
        // 一项的框是 (1+2)×20 = 60 px 高，比两块 32 还矮；原版
        // （`FUN_00445500`）把块夹到一半，上下在中线相接。
        let b = box_rect(&items(&["好"]));
        assert_eq!(b.3 - b.1, 60);
        let p = frame_pieces(b);
        let (top_left, bottom_left) = (p[0], p[1]);
        assert_eq!(top_left.3 - top_left.1, 30, "角块高度该夹到一半");
        assert_eq!(top_left.3, bottom_left.1, "上下该在中线相接，不重叠");
        // 中间一行没地方了，中块退化成空矩形。
        assert_eq!(p[8].0, p[8].2);
        assert_eq!(p[8].1, p[8].3);

        // 够大的框照常：角块就是整块 32。
        let big = box_rect(&items(&["一", "二", "三", "四"]));
        let q = frame_pieces(big);
        assert_eq!(q[0].2 - q[0].0, FRAME_PIECE);
        assert_eq!(q[0].3 - q[0].1, FRAME_PIECE);
        assert!(q[8].0 < q[8].2 && q[8].1 < q[8].3);
    }

    #[test]
    fn the_box_is_sized_by_the_longest_item_and_centred() {
        // 两项、最长 2 字节（一个汉字）：宽 (2+6)×10 = 80，高 (2+2)×20 = 80。
        let b = box_rect(&items(&["是", "否"]));
        assert_eq!(b, (360, 260, 440, 340));
        // 最长一项 6 个汉字 = 12 字节：宽 180；三项：高 100。
        let b = box_rect(&items(&["好的", "我再想想看", "不要不要不要"]));
        assert_eq!((b.2 - b.0, b.3 - b.1), (180, 100));
        assert_eq!(b.0 + b.2, 800, "水平居中");
    }

    #[test]
    fn gbk_counts_two_bytes_per_hanzi() {
        assert_eq!(gbk_len("是"), 2);
        assert_eq!(gbk_len("ab是"), 4);
    }

    #[test]
    fn rows_start_ten_pixels_inside_and_step_twenty() {
        let b = box_rect(&items(&["是", "否"]));
        let area = list_rect(b);
        assert_eq!(area, (370, 270, 430, 330));
        // 第一行从列表上边再下 8 开始（原版 0x43e95a）。
        assert_eq!(row_rect(area, 0), (370, 278, 430, 298));
        assert_eq!(row_rect(area, 1), (370, 298, 430, 318));
        // 高亮条左留 4、右留 2，和行顶齐平、高 行高−2。
        assert_eq!(
            highlight_rect(row_rect(area, 0), area),
            (374, 278, 428, 296)
        );
        assert_eq!(row_at(2, b, 380., 303.), Some(1));
        assert_eq!(row_at(2, b, 380., 275.), None, "第一行上面那 8 像素不算");
        assert_eq!(row_at(2, b, 380., 323.), None, "第三行没有选项");
        assert_eq!(row_at(2, b, 365., 283.), None, "框边上不算");
    }

    #[test]
    fn the_frame_pieces_tile_the_box() {
        let b = (100, 100, 300, 200);
        let p = frame_pieces(b);
        assert_eq!(p[0], (100, 100, 132, 132));
        assert_eq!(p[2], (268, 168, 300, 200));
        assert_eq!(p[8], (132, 132, 268, 168));
    }
}
