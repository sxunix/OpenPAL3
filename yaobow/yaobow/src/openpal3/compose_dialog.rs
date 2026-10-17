//! 仙术页「合」页签的合成流程和它的三个对话框（源码 `gamemainui/magicui.cpp`）。
//!
//! 规则在 [`shared::openpal3::states::compose`]；这里是页面上那一串：
//!
//! 1. 双击一条配方（移植端：再点一下已选中的那条）：材料不够弹提示框 `m_Dlg`「合成元素不够」
//!    （`CheckCom`）；够就弹「是/否」框 `m_DlgEx`「您确定吗？！」，放 wa002（`SND_LIST_OK`）。
//! 2. 「是」：`ComboFinal`——熟练度 +0.01、掷骰、成功得成品，成败都扣材料，放 wa025
//!    （`SND_SYS_COMBO`）；然后列表锁住等 5 秒（`m_comTime.Wait(5000)`）。「否」：什么也不做。
//!    两个按钮都放 wa001（`MUIDialog` 的 `SND_BUTTON_OK`）。
//! 3. 5 秒后弹提示框「合成成功！/合成失败！」加得失清单，放 wa006 / wa007。
//!
//! 原版还给成败各撒一把粒子（`SetComParEffect`），但那 98 个粒子整份源码里没有一处画它们，
//! 所以什么也看不见，这里不做。
//!
//! 对话框（`MUIDialog`，源码 `gamemainui/muidialog.cpp`）：
//! - 提示框 `DLG_MSGBOX`：(300,200,556,356) 的 `UI\Flex\Note` 九宫格，0.5 秒从中心 32×32 余弦缓动
//!   展开，展开完才出字；弹出 600 毫秒后点一下就关。
//! - 「是/否」框 `DLG_YESNO`：右下改写成左上 + 308×109，1:1 贴 `MUIDialog\Normal\DlgBackground.tga`，
//!   「是」「否」各 90×26，离左右边 26、离下边 6；600 毫秒内点不动。
//!
//! 字色都是 (31,251,255)，正文从框内 (6, 6) 起、再缩 8（界面字的边距），一行 16。

use std::rc::Rc;
use std::time::Instant;

use crosscom::ComRc;
use radiance::comdef::IUiHost;
use radiance::rendering::Sprite;
use shared::openpal3::asset_manager::AssetManager;
use shared::openpal3::states::compose::{self, RECIPES};
use shared::openpal3::states::game_db::GameDb;
use shared::openpal3::states::persistent_state::PersistentState;
use shared::openpal3::ui_tex::UiTextures;

type Rect = (i32, i32, i32, i32);

/// `m_Dlg` / `m_DlgEx` 建在 (300, 200)。
const MSG_RECT: Rect = (300, 200, 556, 356);
const YESNO_RECT: Rect = (300, 200, 608, 309);
const BTN_W: i32 = 90;
const BTN_H: i32 = 26;
const OPEN_SECS: f32 = 0.5;
const LOCK_SECS: f32 = 0.6;
/// `m_comTime.Wait(5000)`。
const WORK_SECS: f32 = 5.;
const INK: [f32; 3] = [31. / 255., 251. / 255., 1.];
const GLYPH: f32 = 16.;
const INSET: i32 = 6 + 8;

const LANG_QTY: usize = 4;
const LANG_SHORT: usize = 87;
const LANG_OK: usize = 88;
const LANG_FAIL: usize = 89;
const LANG_SURE: usize = 90;
const LANG_GOT: usize = 157;
const LANG_LOST: usize = 158;

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    Idle,
    /// 提示框开着（材料不够，或合成结果）。
    Message,
    /// 「是/否」框开着。
    Confirm,
    /// 点了「是」，等 5 秒出结果。
    Working,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Btn {
    Yes,
    No,
}

/// 一张贴图和它在界面纹理表里的 id。
struct Pic {
    com_id: i64,
    sprite: Rc<Sprite>,
}

struct Art {
    note: Vec<Option<Pic>>,
    background: Option<Pic>,
    yes: [Option<Pic>; 3],
    no: [Option<Pic>; 3],
}

/// 这一帧要的东西：页面画布的原点和缩放、鼠标（屏幕坐标）。
pub struct Frame {
    pub ox: f32,
    pub oy: f32,
    pub scale: f32,
    pub mx: f32,
    pub my: f32,
    pub down: bool,
    pub click: bool,
}

impl Frame {
    fn at(&self, x: i32, y: i32) -> (f32, f32) {
        (
            self.ox + x as f32 * self.scale,
            self.oy + y as f32 * self.scale,
        )
    }

    fn over(&self, r: Rect) -> bool {
        let (x0, y0) = self.at(r.0, r.1);
        let (x1, y1) = self.at(r.2, r.3);
        self.mx >= x0 && self.mx < x1 && self.my >= y0 && self.my < y1
    }
}

/// 合成要动的那些东西。
pub struct Ctx<'a> {
    pub state: &'a mut PersistentState,
    pub db: &'a GameDb,
    pub lang: &'a dyn Fn(usize) -> String,
    pub sound: &'a dyn Fn(&str),
}

pub struct ComposeDialogs {
    stage: Stage,
    time: f32,
    last: Instant,
    role: usize,
    recipe: usize,
    text: String,
    /// 上一次合成成没成（5 秒后放哪个音效）。
    success: bool,
    /// 「是」框里按下时落在哪个按钮上。
    pressed: Option<Btn>,
    art: Option<Art>,
    rng: shared::openpal3::combat::formulas::CombatRng,
}

impl Default for ComposeDialogs {
    fn default() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u32)
            .unwrap_or(1);
        Self {
            stage: Stage::Idle,
            time: 0.,
            last: Instant::now(),
            role: 0,
            recipe: 0,
            text: String::new(),
            success: false,
            pressed: None,
            art: None,
            rng: shared::openpal3::combat::formulas::CombatRng::new(seed),
        }
    }
}

fn item_name(db: &GameDb, id: u32) -> String {
    db.item(id)
        .and_then(|i| i.name.as_str().ok())
        .unwrap_or_default()
}

impl ComposeDialogs {
    /// 有对话框开着、或者在等结果——页面这时不收点击（`CtlList(false)`）。
    pub fn busy(&self) -> bool {
        self.stage != Stage::Idle
    }

    /// 点了「是」、正等 5 秒出结果（`bIsMotion`）——合成粒子这时在冒。
    pub fn working(&self) -> bool {
        self.stage == Stage::Working
    }

    fn set(&mut self, stage: Stage) {
        self.stage = stage;
        self.time = 0.;
        self.pressed = None;
    }

    /// 双击了 `role` 的配方 `recipe`（`UIWM_LB_DBCLICK` 的 `IDL_MAGIC_COMLIST` 一支）。
    pub fn start(&mut self, ctx: &mut Ctx, role: usize, recipe: usize) {
        if self.busy() || recipe >= compose::RECIPE_COUNT {
            return;
        }
        self.role = role;
        self.recipe = recipe;
        let r = &RECIPES[recipe];
        if compose::check_materials(r, |id| ctx.state.item_count(id)).is_none() {
            // `CheckCom`：「\n\n\n        合成元素不够」。
            self.text = format!("\n\n\n        {}", (ctx.lang)(LANG_SHORT));
            self.set(Stage::Message);
            return;
        }
        self.text = format!("\n            {}", (ctx.lang)(LANG_SURE));
        self.set(Stage::Confirm);
        (ctx.sound)("wa002");
    }

    /// `ComboFinal`。材料在这一刻又不够了（`CheckCom` 失败）就直接弹「不够」，不进 5 秒等待。
    fn combine(&mut self, ctx: &mut Ctx) {
        let (role, recipe) = (self.role, self.recipe);
        let r = &RECIPES[recipe];
        let rate = compose::raise_rate(ctx.state.compose_rate(role, recipe));
        ctx.state.set_compose_rate(role, recipe, rate);
        let success = compose::rolls(compose::rate_percent(rate), self.rng.rand15());
        let qty = (ctx.lang)(LANG_QTY);
        let mut lines = String::new();
        match compose::check_materials(r, |id| ctx.state.item_count(id)) {
            Some(com_num) => {
                if success {
                    // `Inventory::AddItem`：同一件最多 99 个。
                    if ctx.state.item_count(r.product) < 99 {
                        ctx.state.add_item(r.product, 1);
                    }
                    lines += &format!(
                        "    {}{} [{}1]\n",
                        (ctx.lang)(LANG_GOT),
                        item_name(ctx.db, r.product),
                        qty
                    );
                }
                // 背包按拿到的先后走（原版 `m_items` 的顺序），跳格跳到哪一格才和原版一样。
                let order: Vec<(u32, u32)> = ctx
                    .state
                    .inventory_ids()
                    .into_iter()
                    .map(|k| (k, ctx.state.item_count(k)))
                    .collect();
                let n = compose::lost_count_label(r, com_num);
                for (id, take) in compose::consumption(r, com_num, &order) {
                    ctx.state.remove_item(id, take);
                    lines += &format!(
                        "    {}{} [{}{}]\n",
                        (ctx.lang)(LANG_LOST),
                        item_name(ctx.db, id),
                        qty,
                        n
                    );
                }
                let title = (ctx.lang)(if success { LANG_OK } else { LANG_FAIL });
                self.text = format!("\n    {title}\n{lines}");
                self.success = success;
                self.set(Stage::Working);
            }
            None => {
                self.text = format!("\n\n\n        {}", (ctx.lang)(LANG_SHORT));
                self.set(Stage::Message);
            }
        }
        (ctx.sound)("wa025");
    }

    fn load(&mut self, am: &AssetManager) {
        let mut t = UiTextures::new(am);
        let mut pic = |name: &str| {
            t.get(am, name).map(|sprite| Pic {
                com_id: super::service::next_ui_texture_com_id(),
                sprite,
            })
        };
        let note = (0..9)
            .map(|i| pic(&format!("UI\\Flex\\Note{i}.tga")))
            .collect();
        let background = pic("UI\\GameMainUI\\MUIDialog\\Normal\\DlgBackground.tga");
        let yes =
            std::array::from_fn(|i| pic(&format!("UI\\GameMainUI\\MUIDialog\\Normal\\Yes{i}.tga")));
        let no =
            std::array::from_fn(|i| pic(&format!("UI\\GameMainUI\\MUIDialog\\Normal\\No{i}.tga")));
        self.art = Some(Art {
            note,
            background,
            yes,
            no,
        });
    }

    /// 每帧：推进状态、收点击、画当前的对话框。`register` 把贴图登记进界面纹理表。
    pub fn paint(
        &mut self,
        ui: &ComRc<IUiHost>,
        am: &AssetManager,
        register: &dyn Fn(i64, imgui::TextureId),
        frame: &Frame,
        ctx: &mut Ctx,
    ) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
        self.last = now;
        if self.stage == Stage::Idle {
            return;
        }
        self.time += dt;
        if self.art.is_none() {
            self.load(am);
        }

        match self.stage {
            Stage::Message => {
                if self.time >= LOCK_SECS && frame.click {
                    self.set(Stage::Idle);
                    return;
                }
            }
            Stage::Confirm => {
                let yes = yes_rect();
                let no = no_rect();
                let hit = if frame.over(yes) {
                    Some(Btn::Yes)
                } else if frame.over(no) {
                    Some(Btn::No)
                } else {
                    None
                };
                if frame.click {
                    self.pressed = hit;
                }
                // `UIButton`：按下后在同一个按钮上松开才算；600 毫秒内点不动。
                if !frame.down && self.pressed.is_some() {
                    let fired = (self.pressed == hit).then_some(hit).flatten();
                    self.pressed = None;
                    if self.time >= LOCK_SECS {
                        match fired {
                            Some(Btn::Yes) => {
                                (ctx.sound)("wa001");
                                self.combine(ctx);
                            }
                            Some(Btn::No) => {
                                (ctx.sound)("wa001");
                                self.set(Stage::Idle);
                                return;
                            }
                            None => {}
                        }
                    }
                }
            }
            Stage::Working => {
                if self.time >= WORK_SECS {
                    (ctx.sound)(if self.success { "wa006" } else { "wa007" });
                    self.set(Stage::Message);
                }
            }
            Stage::Idle => {}
        }
        self.draw(ui, register, frame);
    }

    fn draw(&self, ui: &ComRc<IUiHost>, register: &dyn Fn(i64, imgui::TextureId), frame: &Frame) {
        let Some(art) = self.art.as_ref() else { return };
        let blit = |pic: &Option<Pic>, r: Rect| {
            let Some(p) = pic else { return };
            register(p.com_id, p.sprite.imgui_texture_id());
            // 界面图 1:1，取贴图左下角那一块。
            let (tw, th) = (
                p.sprite.width().max(1) as f32,
                p.sprite.height().max(1) as f32,
            );
            let (w, h) = ((r.2 - r.0) as f32, (r.3 - r.1) as f32);
            let u1 = (w / tw).min(1.);
            let v0 = (1. - h / th).max(0.);
            let (x0, y0) = frame.at(r.0, r.1);
            let (x1, y1) = frame.at(r.2, r.3);
            ui.image_rect(p.com_id as i32, x0, y0, x1, y1, 0., v0, u1, 1.);
        };
        // 九宫格的每一块整张贴图拉伸（`UIStaticFlex`，和 `killflyer::draw_flex` 一样）。
        let stretch = |pic: &Option<Pic>, r: Rect| {
            let Some(p) = pic else { return };
            register(p.com_id, p.sprite.imgui_texture_id());
            let (x0, y0) = frame.at(r.0, r.1);
            let (x1, y1) = frame.at(r.2, r.3);
            ui.image_rect(p.com_id as i32, x0, y0, x1, y1, 0., 0., 1., 1.);
        };
        let text = |r: Rect| {
            let size = GLYPH * frame.scale;
            for (k, line) in self.text.split('\n').enumerate() {
                let (x, y) = frame.at(r.0 + INSET, r.1 + INSET + (GLYPH as i32) * k as i32);
                ui.text_at_sized(x, y, size, INK[0], INK[1], INK[2], 1., line);
            }
        };
        match self.stage {
            Stage::Message => {
                let x = (self.time / OPEN_SECS).min(1.);
                let k = (1. - (x * std::f32::consts::PI).cos()) * 0.5;
                let m = MSG_RECT;
                let (cx, cy) = ((m.0 + m.2) / 2, (m.1 + m.3) / 2);
                let lerp = |a: i32, b: i32| (a as f32 * (1. - k) + b as f32 * k) as i32;
                let r = (
                    lerp(cx - 16, m.0),
                    lerp(cy - 16, m.1),
                    lerp(cx + 16, m.2),
                    lerp(cy + 16, m.3),
                );
                flex(&art.note, r, &stretch);
                if x >= 1. {
                    text(m);
                }
            }
            Stage::Confirm => {
                blit(&art.background, YESNO_RECT);
                text(YESNO_RECT);
                let state = |b: Btn, r: Rect| -> usize {
                    if self.pressed == Some(b) && frame.over(r) {
                        2
                    } else if frame.over(r) {
                        1
                    } else {
                        0
                    }
                };
                blit(&art.yes[state(Btn::Yes, yes_rect())], yes_rect());
                blit(&art.no[state(Btn::No, no_rect())], no_rect());
            }
            _ => {}
        }
    }
}

fn yes_rect() -> Rect {
    let r = YESNO_RECT;
    (r.0 + 26, r.3 - (BTN_H + 6), r.0 + 26 + BTN_W, r.3 - 6)
}

fn no_rect() -> Rect {
    let r = YESNO_RECT;
    (r.2 - (26 + BTN_W), r.3 - (BTN_H + 6), r.2 - 26, r.3 - 6)
}

/// 九宫格（`UIStaticFlex::Render`）：角块边长取 min(贴图边长, 半宽, 半高)；0..8 是左上、左下、
/// 右下、右上、左、下、右、上、中。
fn flex(pieces: &[Option<Pic>], r: Rect, blit: &dyn Fn(&Option<Pic>, Rect)) {
    let tile = pieces
        .first()
        .and_then(|p| p.as_ref())
        .map_or(32, |p| p.sprite.width() as i32);
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
    for (k, q) in rects.iter().enumerate() {
        if q.0 < q.2 && q.1 < q.3 {
            if let Some(p) = pieces.get(k) {
                blit(p, *q);
            }
        }
    }
}
