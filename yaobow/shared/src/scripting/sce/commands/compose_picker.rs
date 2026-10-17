//! 111 学会合成术（`LAB_00437040` → `ChoseCompose::SpecifyCompose`，源码
//! `gamemainui/ChoseCompose.cpp`）。参数是成品的物品号。
//!
//! 弹一个 `UI\Flex\Note` 框：(310, 170) 宽 158，高 160 + 队里每人 20，0.5 秒从中心展开
//! （`UIStaticFlex`）；展开完写「请选择由谁来学习<成品>合成术」（框内 (+10, +10) 起 136×128，
//! 小字，色 (31,251,255)），下面是队里每人一个 `HockShop\Control\rawbtn` 按钮
//! （90×26，从 (+36, +98) 往下排，字是 AssisLang 64..68 的名字）。点谁谁学会，消息栏报
//! `lang.txt` 的 `AddSkill`（「%s习得%s」）；那人已经学满 6 个就弹提示框「技能已满，无法增加」
//! （(300,200,500,300)，色 (18,255,28)），关掉后整个框一起关、谁也没学。脚本等到框关掉。

use std::path::PathBuf;
use std::rc::Rc;

use crate::openpal3::states::compose;
use crate::openpal3::ui_tex::UiTextures;
use crate::scripting::sce::commands::appraise::overlay;
use crate::scripting::sce::commands::dealer::{Tex, blit, button, text_width};
use crate::scripting::sce::commands::dealer_layout::{self as lay, INK, inside};
use crate::scripting::sce::commands::killflyer::draw_flex;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::{MouseButton, Ui};
use radiance::comdef::ISceneManager;
use radiance::input::Key;
use radiance::rendering::Sprite;

const LANG_PATH: &str = "/basedata/basedata/AssisLang.lang";
/// 「请选择由谁来学习」「合成术」。
const LANG_ASK: usize = 204;
const LANG_SKILL: usize = 205;
/// 「技能已满，无法增加」。
const LANG_FULL: usize = 237;
/// 景天..长卿。
const LANG_ROLE0: usize = 64;
/// `lang.txt` 里的角色名键（`SetSpecifyCompose` 的 switch）。
const ROLE_KEYS: [&str; compose::ROLE_COUNT] =
    ["Jingtian", "Xuejian", "Longkui", "Zixuan", "Changqing"];

const BOX_X: i32 = 310;
const BOX_Y: i32 = 170;
const BOX_W: i32 = 158;
const BOX_H: i32 = 160;
const BOX_PER_ROLE: i32 = 20;
const BTN_W: i32 = 90;
const BTN_H: i32 = 26;
/// 提示框（`m_Msg`，`DLG_MSGBOX`）。
const MSG_RECT: (i32, i32, i32, i32) = (300, 200, 500, 300);
const INFO_INK: [f32; 4] = [31. / 255., 251. / 255., 1., 1.];
const MSG_INK: [f32; 4] = [18. / 255., 1., 28. / 255., 200. / 255.];
const LINE: i32 = 18;
/// `UIStaticFlex` 展开时长。
const OPEN_SECS: f32 = 0.5;
/// `MUIDialog` 弹出后 600 毫秒内关不掉。
const MSG_LOCK_SECS: f32 = 0.6;

struct Art {
    note: Vec<Option<Rc<Sprite>>>,
    rawbtn: [Tex; 3],
    lang: Vec<String>,
    product_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Choosing,
    /// 那人学满了，提示框开着。
    Full,
}

pub struct SceCommandLearnCompose {
    product: u32,
    art: Option<Art>,
    stage: Stage,
    /// 当前阶段过了多久。
    time: f32,
    /// 按下时落在哪个按钮上（`UIButton` 松开在同一个上才算）。
    pressed: Option<usize>,
    /// [移植] 手柄 / 方向键选中的那一个；`None` 时没用过键盘，不画（原版只有鼠标）。
    cursor: Option<usize>,
}

impl std::fmt::Debug for SceCommandLearnCompose {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandLearnCompose")
            .field("product", &self.product)
            .finish()
    }
}

impl Clone for SceCommandLearnCompose {
    fn clone(&self) -> Self {
        Self::new(self.product as i32)
    }
}

impl SceCommandLearnCompose {
    pub fn new(product: i32) -> Self {
        Self {
            product: product.max(0) as u32,
            art: None,
            stage: Stage::Choosing,
            time: 0.,
            pressed: None,
            cursor: None,
        }
    }

    fn load(&mut self, state: &SceState) {
        let am = state.asset_mgr().clone();
        let mut t = UiTextures::new(&am);
        let rawbtn = std::array::from_fn(|i| {
            t.get(&am, &format!("SubGame\\HockShop\\Control\\rawbtn{i}.tga"))
        });
        let note = (0..9)
            .map(|i| {
                am.load_sprite(PathBuf::from(format!(
                    "/basedata/basedata/ui/flex/note{i}.tga"
                )))
            })
            .collect();
        let lang = {
            use common::store_ext::StoreExt2;
            am.vfs()
                .read_to_end_from_gbk(PathBuf::from(LANG_PATH))
                .map(|t| crate::openpal3::assis_lang::parse_section(&t, "sec1"))
                .unwrap_or_default()
        };
        let product_name = state
            .global_state()
            .game_db()
            .and_then(|db| db.item(self.product).and_then(|i| i.name.as_str().ok()))
            .unwrap_or_default();
        self.art = Some(Art {
            note,
            rawbtn,
            lang,
            product_name,
        });
    }

    /// 队里有哪些主角，按主角号的顺序（按钮就这么排）。
    fn roles_in_team(state: &SceState) -> Vec<usize> {
        let p = state.global_state().persistent_state();
        (0..compose::ROLE_COUNT)
            .filter(|&r| p.is_in_team(r as u32))
            .collect()
    }

    fn box_rect(n: usize) -> (i32, i32, i32, i32) {
        (
            BOX_X,
            BOX_Y,
            BOX_X + BOX_W,
            BOX_Y + BOX_H + BOX_PER_ROLE * n as i32,
        )
    }

    fn button_rect(k: usize) -> (i32, i32, i32, i32) {
        let x = BOX_X + 36;
        let y = BOX_Y + 98 + BTN_H * k as i32;
        (x, y, x + BTN_W, y + BTN_H)
    }

    fn lang(&self, n: usize) -> String {
        self.art
            .as_ref()
            .and_then(|a| a.lang.get(n))
            .map(|s| crate::openpal3::assis_lang::unescape(s))
            .unwrap_or_default()
    }

    /// 点了第 `role` 号主角。学会了返回 `true`（框关掉）；学满了转去提示框。
    fn choose(&mut self, state: &mut SceState, role: usize) -> bool {
        let Some(recipe) = compose::recipe_for_product(self.product) else {
            log::warn!("SCE 111: {} 不是合成术的成品", self.product);
            return true;
        };
        let learnt = state
            .global_state_mut()
            .persistent_state_mut()
            .learn_compose(role, recipe);
        if !learnt {
            self.stage = Stage::Full;
            self.time = 0.;
            return false;
        }
        // `_snprintf(buf, g_lang["AddSkill"], g_lang[roleName], pItem->name)`。
        let text = {
            use common::store_ext::StoreExt2;
            state
                .asset_mgr()
                .vfs()
                .read_to_end_from_gbk(crate::openpal3::combat::result::LANG_PATH)
                .ok()
        };
        let value = |key: &str| {
            text.as_deref()
                .and_then(|t| crate::openpal3::combat::result::lang_value(t, key))
                .unwrap_or_default()
        };
        let name = self
            .art
            .as_ref()
            .map(|a| a.product_name.clone())
            .unwrap_or_default();
        let note = value("AddSkill")
            .replacen("%s", &value(ROLE_KEYS[role]), 1)
            .replacen("%s", &name, 1);
        state.push_note(note);
        true
    }

    fn draw(&self, ui: &Ui, roles: &[usize], mouse: [f32; 2]) {
        let Some(art) = self.art.as_ref() else { return };
        let full = Self::box_rect(roles.len());
        let x = (self.time / OPEN_SECS).min(1.);
        let open = self.stage == Stage::Full || x >= 1.;
        overlay(ui, "LearnCompose", |ui, at, _scale| {
            let list = ui.get_window_draw_list();
            let _font = radiance::imgui::game_font(radiance::imgui::GameFontSize::SMALL)
                .map(|f| ui.push_font(f));
            if self.stage == Stage::Choosing {
                // `UIStaticFlex`：从中心 32×32 余弦缓动展开。
                let k = (1. - (x * std::f32::consts::PI).cos()) * 0.5;
                let (cx, cy) = ((full.0 + full.2) / 2, (full.1 + full.3) / 2);
                let lerp = |a: i32, b: i32| (a as f32 * (1. - k) + b as f32 * k) as i32;
                let r = (
                    lerp(cx - 16, full.0),
                    lerp(cy - 16, full.1),
                    lerp(cx + 16, full.2),
                    lerp(cy + 16, full.3),
                );
                draw_flex(&list, &art.note, r, &at);
            } else {
                draw_flex(&list, &art.note, full, &at);
            }
            if open && self.stage == Stage::Choosing {
                let ask = format!(
                    "{}{}{}",
                    self.lang(LANG_ASK),
                    art.product_name,
                    self.lang(LANG_SKILL)
                );
                for (k, line) in lay::wrap(&ask, 136).iter().enumerate() {
                    list.add_text(at(BOX_X + 10, BOX_Y + 10 + LINE * k as i32), INFO_INK, line);
                }
                for (k, &role) in roles.iter().enumerate() {
                    let r = Self::button_rect(k);
                    let hover = inside_px(r, mouse);
                    let down = self.pressed == Some(k) && hover;
                    if down {
                        blit(&list, &art.rawbtn[2], r.0, r.1, &at);
                    } else {
                        button(&list, &art.rawbtn, r, hover || self.cursor == Some(k), &at);
                    }
                    // `SetCaptionOffset(1, 3)`、居中。
                    let name = self.lang(LANG_ROLE0 + role);
                    let w = text_width(&name);
                    list.add_text(
                        at(
                            r.0 + 1 + (BTN_W - w) / 2,
                            r.1 + 3 + (BTN_H - lay::GLYPH) / 2,
                        ),
                        INK,
                        &name,
                    );
                }
            }
            if self.stage == Stage::Full {
                // `m_Msg`：同样的 Note 框，0.5 秒展开完才出字。
                let m = MSG_RECT;
                let mx = (self.time / OPEN_SECS).min(1.);
                let k = (1. - (mx * std::f32::consts::PI).cos()) * 0.5;
                let (cx, cy) = ((m.0 + m.2) / 2, (m.1 + m.3) / 2);
                let lerp = |a: i32, b: i32| (a as f32 * (1. - k) + b as f32 * k) as i32;
                let r = (
                    lerp(cx - 16, m.0),
                    lerp(cy - 16, m.1),
                    lerp(cx + 16, m.2),
                    lerp(cy + 16, m.3),
                );
                draw_flex(&list, &art.note, r, &at);
                if mx >= 1. {
                    // "\n  技能已满，无法增加"。
                    let text = format!("\n  {}", self.lang(LANG_FULL));
                    for (k, line) in text.split('\n').enumerate() {
                        list.add_text(at(m.0 + 6, m.1 + 6 + LINE * k as i32), MSG_INK, line);
                    }
                }
            }
        });
    }
}

/// 鼠标（800×600 画布坐标）在不在 `r` 里。
fn inside_px(r: (i32, i32, i32, i32), p: [f32; 2]) -> bool {
    inside(r, p[0], p[1])
}

/// 屏幕坐标换成 800×600 画布坐标（和 [`overlay`] 一样的缩放）。
fn to_canvas(ui: &Ui) -> [f32; 2] {
    let [sw, sh] = ui.io().display_size;
    let scale = (sw / 800.).min(sh / 600.);
    let ox = (sw - 800. * scale) * 0.5;
    let oy = (sh - 600. * scale) * 0.5;
    let [mx, my] = ui.io().mouse_pos;
    [(mx - ox) / scale, (my - oy) / scale]
}

impl SceCommand for SceCommandLearnCompose {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if compose::recipe_for_product(self.product).is_none() {
            log::warn!("SCE 111: {} 不是合成术的成品，跳过", self.product);
            return true;
        }
        let roles = Self::roles_in_team(state);
        if roles.is_empty() || state.fast_forward() {
            // 快进时没人能点：照原样谁也不学，脚本继续。
            return true;
        }
        if self.art.is_none() {
            self.load(state);
        }
        self.time += delta_sec;
        self.cursor = self.cursor.map(|c| c.min(roles.len() - 1));
        let mouse = to_canvas(ui);
        let pressed = {
            let input = state.input();
            let down: Vec<Key> = [
                Key::Up,
                Key::Down,
                Key::GamePadDPadUp,
                Key::GamePadDPadDown,
                Key::GamePadEast,
            ]
            .into_iter()
            .filter(|&k| input.get_key_state(k).pressed())
            .collect();
            move |k: Key| down.contains(&k)
        };

        match self.stage {
            Stage::Choosing if self.time >= OPEN_SECS => {
                let hit = (0..roles.len()).find(|&k| inside_px(Self::button_rect(k), mouse));
                if ui.is_mouse_clicked(MouseButton::Left) {
                    self.pressed = hit;
                }
                let mut chosen = None;
                if ui.is_mouse_released(MouseButton::Left) {
                    if let (Some(p), Some(h)) = (self.pressed, hit) {
                        if p == h {
                            chosen = Some(h);
                        }
                    }
                    self.pressed = None;
                }
                // [移植] 方向键 / 手柄上下挑、A（`GamePadEast`）或回车确定。
                if pressed(Key::Up) || pressed(Key::GamePadDPadUp) {
                    self.cursor = Some(self.cursor.map_or(0, |c| c.saturating_sub(1)));
                }
                if pressed(Key::Down) || pressed(Key::GamePadDPadDown) {
                    self.cursor = Some(self.cursor.map_or(0, |c| (c + 1).min(roles.len() - 1)));
                }
                if ui.is_key_pressed(imgui::Key::Enter) || pressed(Key::GamePadEast) {
                    chosen = Some(self.cursor.unwrap_or(0));
                }
                if let Some(k) = chosen {
                    if self.choose(state, roles[k]) {
                        return true;
                    }
                }
            }
            Stage::Full if self.time >= MSG_LOCK_SECS => {
                let input = state.input();
                let any = ui.is_key_pressed(imgui::Key::Enter)
                    || [
                        Key::Escape,
                        Key::Space,
                        Key::Left,
                        Key::Right,
                        Key::Up,
                        Key::Down,
                        Key::Tab,
                        Key::GamePadEast,
                        Key::GamePadSouth,
                    ]
                    .into_iter()
                    .any(|k| input.get_key_state(k).pressed());
                if ui.is_mouse_clicked(MouseButton::Left) || any {
                    return true;
                }
            }
            _ => {}
        }
        self.draw(ui, &roles, mouse);
        false
    }
}
