//! 御剑页（源码 `gamemainui/swordui.cpp`，exe 页面 init `FUN_004ab6b0`）：主界面和净化界面。
//! 规则在 [`shared::openpal3::states::sword_purge`]，3D 视图在 [`super::sword_views`]。
//!
//! 主界面：魔剑说明栏（打开页面时若景天拿着魔剑，从剑框底下每秒 220 像素向左滑出到 x=58）、剑框、
//! 当前属性图标和武器技名（按属性着色）、龙精石和数量、御剑飞行、五灵控制盘（每块阴、阳两个按钮；
//! 净化过的那面换成 `_over` 图，两面都净化过换成 `_perfect`）、阴/阳翻面按钮、3D 剑和龙精石。
//! 鼠标停在这些东西上，说明栏换成对应的说明（`RemarkTextID`），移开回到总说明（206）。
//!
//! 点某块的阴（阳）按钮进净化界面——要入场转完、拿着魔剑、有龙精石、剑正朝着这一面、上一次翻面
//! 转完、这一面没净化过；点的这一块同时成了魔剑的当前属性（就算之后取消也不改回来，照原样）。
//! 净化界面左边是背包里五类装备（页签切换，可按价格 / 古值排序），右边是净化列表；双击左边一件
//! 放进去、双击右边一件拿回来；「确定」弹预算框，「是」就净化、扣一颗龙精石。
//!
//! 原版放进净化列表时就从背包里扣、关页面或取消时再还回去；移植端改成确定时才扣，列表上的数量
//! 照样减去已放进去的件数 [移植：页面关闭没有可靠的回调，免得中途存档丢东西]。
//!
//! 没做的：剑身周围的火光粒子、净化后的粒子（`SetSwordEffect` / `SetPurgeEffect`）、净化字在剑上
//! 渐显（原版 `UpdateMergeData` 每帧逼近 5，这里一次换好）、左右方向键切页签。

use std::time::Instant;

use crosscom::ComRc;
use radiance::comdef::IUiHost;
use shared::openpal3::asset_manager::AssetManager;
use shared::openpal3::states::game_db::GameDb;
use shared::openpal3::states::persistent_state::PersistentState;
use shared::openpal3::states::sword_purge::{
    self as sp, BLOCKS, Basket, DEMON_SWORD, DRAGON_STONE, Refusal, WeaponSkill,
};

use super::sword_views::{Canvas, SwordViews};
use super::ui_draw::{Painter, Pics, Rect, pad_bytes};

const SWORD_DIR: &str = "UI\\GameMainUI\\Sword\\";
const BTN_DIR: &str = "UI\\GameMainUI\\Sword\\WuLingBtn\\";
/// 页面上的块序（水火雷风土）。
const EL_BASE: [&str; BLOCKS] = ["shui", "huo", "lei", "feng", "tu"];
const ATT_ICON: [&str; BLOCKS] = ["Shui", "Huo", "Lei", "Feng", "Tu"];
/// 武器技名的颜色（`SWD_SKILL_TEXT_COLOR_S/H/L/F/T`）。
const SKILL_INK: [[u8; 3]; BLOCKS] = [
    [154, 222, 249],
    [255, 185, 184],
    [202, 183, 255],
    [183, 255, 208],
    [255, 236, 183],
];
/// 属性名（AssisLang 9 水 10 火 12 雷 11 风 13 土）。
const ATT_LANG: [usize; BLOCKS] = [9, 10, 12, 11, 13];
const REMARK_INK: [u8; 3] = [143, 244, 255];
const NUMBER_INK: [u8; 3] = [31, 251, 255];
const LIST_INK: [u8; 3] = [12, 31, 0];
const TITLE_INK: [u8; 3] = [0, 31, 12];
const DIALOG_INK: [u8; 3] = [31, 251, 255];

// ---- 主界面 ----
const REMARK_IN_X: f32 = 264.;
const REMARK_OUT_X: f32 = 58.;
const REMARK_SPEED: f32 = 220.;
const FRAME: (i32, i32) = (260, 2);
const ATT_RECT: Rect = (470, 10, 502, 42);
const NAME_RECT: Rect = (342, 16, 470, 48);
const LJS_BG: (i32, i32) = (668, 48);
const LJS_NUM: Rect = (684, 48, 780, 80);
const YJFX: Rect = (506, 28, 621, 103);
const WULING: (i32, i32) = (690, 131);
const BTN_W: i32 = 43;
const BTN_H: i32 = 51;
const LUNAR_X: i32 = 694;
const POSITIVE_X: i32 = 730;
const ROW_Y: [i32; BLOCKS] = [147, 220, 294, 368, 443];
const PERFECT_X: i32 = 692;
const PERFECT_Y: [i32; BLOCKS] = [140, 212, 285, 358, 432];
const MODE: Rect = (291, 466, 491, 526);
/// 3D 剑和龙精石的「鼠标在上面」：原版拿模型的包围盒测，这里用矩形 [近似]。
const SWORD_HIT: Rect = (370, 62, 450, 430);
const LJS_HIT: Rect = (668, 48, 780, 160);

// ---- 净化界面 ----
const LIST_X: [i32; 2] = [52, 394];
const LIST_Y: i32 = 108;
const GRID_DX: i32 = 18;
const GRID_Y0: i32 = 186;
const ROWS: usize = 12;
const ROW_H: i32 = 23;
const COL_X: [i32; 4] = [0, 130, 168, 206];
const TAB_X: [i32; 5] = [68, 134, 200, 266, 332];
const TAB_BASE: [&str; 5] = ["Weapon", "Cloth", "Hat", "Shoes", "Adorn"];
/// 五个页签对应的装备类型（`IT_WEAPON` / `IT_CLOTH` …）。
const TAB_TYPE: [u8; 5] = [11, 7, 8, 9, 10];
const SORT_RECTS: [Rect; 4] = [
    (124, 474, 220, 500),
    (226, 474, 322, 500),
    (467, 474, 563, 500),
    (569, 474, 665, 500),
];
const OK_RECT: Rect = (226, 502, 322, 528);
const CANCEL_RECT: Rect = (472, 502, 568, 528);
const GU_RECT: Rect = (552, 116, 680, 148);

// ---- 对话框 ----
const MSG_RECT: Rect = (288, 200, 488, 312);
const YESNO_RECT: Rect = (241, 200, 549, 309);
const DLG_INSET: i32 = 6 + 8;
const OPEN_SECS: f32 = 0.5;
const LOCK_SECS: f32 = 0.6;

const REOPEN_SECS: f32 = 0.25;

/// `RemarkTextID[k]`。
fn remark_lang(k: usize) -> usize {
    match k {
        30 => 236,
        31 => 265,
        _ => 206 + k,
    }
}

/// 这一帧的鼠标（屏幕坐标）。
pub struct Input {
    pub mx: f32,
    pub my: f32,
    pub down: bool,
    pub click: bool,
    pub rclick: bool,
    pub wheel: f32,
}

/// 页面要的东西。
pub struct Ctx<'a> {
    pub state: &'a mut PersistentState,
    pub db: &'a GameDb,
    pub lang: &'a dyn Fn(usize) -> String,
    pub sound: &'a dyn Fn(&str),
    /// 御剑飞行解锁了没有（opcode 138）。
    pub yjfx: bool,
    /// 点御剑飞行：0 去大地图，否则是拒绝的 AssisLang 号（放音效也在里面）。
    pub fly: &'a dyn Fn() -> i32,
}

/// 页面告诉脚本的。
pub struct Out {
    /// 净化界面或对话框开着：工具栏点了放 wa004（原版 `m_CanCtrlBtn`）。
    pub locked: bool,
    /// 去大地图，菜单关掉。
    pub close: bool,
}

#[derive(Debug, Clone, PartialEq)]
enum Dialog {
    Msg(String),
    Confirm(String),
}

/// 净化列表那个列表控件里显示的一行：名字取自 `name_id`，数量、古、价格取自 `val_id`。原版放进
/// 一件已有的东西时按 `m_Distribute` 的下标去改那一行的数，排过序以后行对不上，名字和数会错开——
/// 照原样。
#[derive(Debug, Clone, Copy, PartialEq)]
struct StuffRow {
    name_id: u32,
    num: i32,
    val_id: u32,
}

#[derive(Debug, Clone, Default)]
struct Purge {
    page: usize,
    scroll: [i32; 2],
    /// 左边选中的行（按显示顺序）。
    sel: Option<usize>,
    stuff_sel: Option<usize>,
    basket: Basket,
    rows: Vec<StuffRow>,
}

pub struct SwordPage {
    views: SwordViews,
    pics: Pics,
    last: Option<Instant>,
    remark_x: f32,
    remark_out: bool,
    purge: Option<Purge>,
    dialog: Option<(Dialog, f32)>,
    /// 左边列表的排序（`m_SortState`：0 价格、1 古；`None` 不排），关页面不清。
    sort: Option<u8>,
    skills: Option<Vec<WeaponSkill>>,
    sword_texture: Option<Option<String>>,
}

impl Default for SwordPage {
    fn default() -> Self {
        Self {
            views: SwordViews::default(),
            pics: Pics::default(),
            last: None,
            remark_x: REMARK_IN_X,
            remark_out: false,
            purge: None,
            dialog: None,
            sort: None,
            skills: None,
            sword_texture: None,
        }
    }
}

fn item_name(db: &GameDb, id: u32) -> String {
    db.item(id)
        .and_then(|i| i.name.as_str().ok())
        .unwrap_or_default()
}

impl SwordPage {
    fn weapon_skills(&mut self, db: &GameDb) -> Vec<WeaponSkill> {
        self.skills
            .get_or_insert_with(|| {
                db.skills()
                    .iter()
                    .filter(|s| s.skill_type == 4)
                    .map(|s| WeaponSkill {
                        id: s.id,
                        elements: std::array::from_fn(|k| s.element_attributes[k] & 0xff != 0),
                    })
                    .collect()
            })
            .clone()
    }

    fn msg(&mut self, text: String, ctx: &Ctx) {
        self.dialog = Some((Dialog::Msg(text), 0.));
        (ctx.sound)("wa003");
    }

    /// 背包里这一页的装备（扣掉已放进净化列表的），按 `m_SortState` 排好。
    fn equip_rows(&self, ctx: &Ctx, page: usize) -> Vec<(u32, u32)> {
        let basket = self.purge.as_ref().map(|p| &p.basket);
        let mut rows: Vec<(u32, u32)> = ctx
            .state
            .inventory()
            .iter()
            .filter_map(|(&id, &n)| {
                let item = ctx.db.item(id)?;
                if item.item_type != TAB_TYPE[page] {
                    return None;
                }
                let used = basket
                    .map(|b| {
                        b.entries
                            .iter()
                            .filter(|e| e.id == id)
                            .map(|e| e.num)
                            .sum::<i32>()
                    })
                    .unwrap_or(0);
                let left = n as i32 - used;
                (left > 0).then_some((id, left as u32))
            })
            .collect();
        // `TypeArraySort`：std::sort 不稳定，同值的先后和原版不一定一样 [近似]。
        if let Some(by) = self.sort {
            rows.sort_by_key(|&(id, _)| {
                let it = ctx.db.item(id);
                match by {
                    0 => it.map_or(0, |i| i.price),
                    _ => it.map_or(0, |i| i.ancient_value),
                }
            });
        }
        rows
    }

    /// 页面又打开了：入场重来、说明栏回到剑框底下、净化界面和对话框都关掉（`UIWM_ACTIVE`）。
    fn reopen(&mut self, has_weapon: bool) {
        self.remark_x = REMARK_IN_X;
        self.remark_out = has_weapon;
        self.purge = None;
        self.dialog = None;
    }

    /// 按净化状态准备剑的贴图（`CheckWSkill`）；一块都没净化时用模型自带的。
    fn sword_texture(&mut self, am: &AssetManager, ctx: &Ctx) -> Option<String> {
        let sword = ctx.state.magic_sword();
        let any = (0..BLOCKS).any(|b| sword.face_purged(b, true) || sword.face_purged(b, false));
        if !any {
            return None;
        }
        let key = sp::purge_texture_key(sword);
        let dir = "/basedata/basedata/ui/GameMainUI/Sword/Data";
        let src = am.read_picture(format!("{dir}/J131Src.tga"))?;
        let dst = am.read_picture(format!("{dir}/J131Dst.tga"))?;
        // `read_picture` 给的是正向（自上而下）的图，材质载 `.tga` 时拿的是文件里自下而上的行序，
        // 拼好后整张翻过来再交给 `TextureStore`。
        let img = image::imageops::flip_vertical(&sp::purge_texture(&src, &dst, sword));
        radiance::rendering::TextureStore::get_or_update(&format!("{dir}/{key}"), || Some(img));
        Some(key)
    }

    pub fn paint(
        &mut self,
        ui: &ComRc<IUiHost>,
        am: &std::rc::Rc<AssetManager>,
        register: &dyn Fn(i64, imgui::TextureId),
        canvas: &Canvas,
        input: &Input,
        ctx: &mut Ctx,
    ) -> Out {
        let now = Instant::now();
        let prev = self.last.replace(now);
        let dt = prev.map_or(0., |t| now.duration_since(t).as_secs_f32());
        let has_weapon = ctx
            .state
            .role(0)
            .is_some_and(|r| r.equipment.get(5).copied() == Some(DEMON_SWORD));
        if prev.is_none_or(|_| dt > REOPEN_SECS) {
            self.reopen(has_weapon);
        }
        let dt = dt.min(0.1);
        let skills = self.weapon_skills(ctx.db);

        let tex = self.sword_texture(am, ctx);
        if self.sword_texture.as_ref() != Some(&tex) {
            self.views.set_sword_texture(tex.clone());
            self.sword_texture = Some(tex);
        }

        let p = Painter {
            ui,
            register,
            ox: canvas.ox,
            oy: canvas.oy,
            scale: canvas.scale,
        };
        let mut out = Out {
            locked: self.purge.is_some() || self.dialog.is_some(),
            close: false,
        };

        // ---- 对话框先收鼠标（模态） ----
        let mut click = input.click;
        let mut rclick = input.rclick;
        if let Some((dialog, t)) = self.dialog.as_mut() {
            *t += dt;
            let (dialog, t) = (dialog.clone(), *t);
            click = false;
            rclick = false;
            if t >= LOCK_SECS && input.click {
                match dialog {
                    Dialog::Msg(_) => self.dialog = None,
                    Dialog::Confirm(_) => {
                        if p.over(input.mx, input.my, yes_rect()) {
                            (ctx.sound)("wa001");
                            self.dialog = None;
                            self.purge_confirmed(ctx, &skills);
                        } else if p.over(input.mx, input.my, no_rect()) {
                            (ctx.sound)("wa001");
                            self.dialog = None;
                            sp::cancel(ctx.state.magic_sword_mut());
                        }
                    }
                }
            }
        }

        // ---- 说明栏的滑动 ----
        let (target, sign) = if self.remark_out {
            (REMARK_OUT_X, -1.)
        } else {
            (REMARK_IN_X, 1.)
        };
        self.remark_x += sign * REMARK_SPEED * dt;
        if (sign < 0. && self.remark_x < target) || (sign > 0. && self.remark_x > target) {
            self.remark_x = target;
        }

        if self.purge.is_some() {
            self.paint_purge(&p, am, input, click, ctx);
            let _ = self.views.paint(
                ui, am, register, canvas, false, has_weapon, false, ctx.sound,
            );
        } else {
            out.close = self.paint_main(
                &p, am, input, click, rclick, has_weapon, ctx, ui, register, canvas,
            );
        }
        self.paint_dialog(&p, am);
        out
    }

    /// 御剑飞行被拒（迷宫里 373、鬼界外围 108）：弹提示框（`IDB_SWORD_YJFX` 那一支的排版）。
    pub fn refuse_flight(&mut self, index: i32, lang: &dyn Fn(usize) -> String) {
        let pad = if index == 373 { "    " } else { "       " };
        self.dialog = Some((
            Dialog::Msg(format!("\n\n{pad}{}", lang(index as usize))),
            0.,
        ));
    }

    /// 「是」：净化（`Purged` 加 `DLG_BT_YES` 那一支）。
    fn purge_confirmed(&mut self, ctx: &mut Ctx, skills: &[WeaponSkill]) {
        let Some(purge) = self.purge.take() else {
            return;
        };
        let lunar = self.views.lunar();
        sp::confirm(ctx.state.magic_sword_mut(), &purge.basket, skills, lunar);
        for e in &purge.basket.entries {
            ctx.state.remove_item(e.id, e.num.max(0) as u32);
        }
        ctx.state.remove_item(DRAGON_STONE, 1);
        (ctx.sound)("wa027");
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_main(
        &mut self,
        p: &Painter,
        am: &std::rc::Rc<AssetManager>,
        input: &Input,
        click: bool,
        rclick: bool,
        has_weapon: bool,
        ctx: &mut Ctx,
        ui: &ComRc<IUiHost>,
        register: &dyn Fn(i64, imgui::TextureId),
        canvas: &Canvas,
    ) -> bool {
        let sword = ctx.state.magic_sword().clone();
        let lang = ctx.lang;
        let over = |r: Rect| p.over(input.mx, input.my, r);
        let btn = |k: usize, lunar: bool| -> Rect {
            let x = if lunar { LUNAR_X } else { POSITIVE_X };
            (x, ROW_Y[k], x + BTN_W, ROW_Y[k] + BTN_H)
        };
        let perfect =
            |k: usize| -> Rect { (PERFECT_X, PERFECT_Y[k], PERFECT_X + 88, PERFECT_Y[k] + 64) };

        // ---- 说明栏的字：鼠标停在哪 ----
        let mut remark = if has_weapon {
            lang(remark_lang(0))
        } else {
            String::new()
        };
        if has_weapon {
            let mut hit: Option<String> = None;
            for k in 0..BLOCKS {
                let r = |id: usize| lang(remark_lang(id + 5 * k));
                if sword.all_purged[k] {
                    if over(perfect(k)) {
                        hit = Some(r(6));
                    }
                    continue;
                }
                if over(btn(k, true)) {
                    hit = Some(if sword.purged[0][k] { r(5) } else { r(4) });
                }
                if over(btn(k, false)) {
                    hit = Some(if sword.purged[1][k] { r(3) } else { r(2) });
                }
            }
            if over(MODE) {
                hit = Some(lang(remark_lang(30)));
            }
            if over(ATT_RECT) {
                hit = Some(lang(remark_lang(27)));
            }
            if over(NAME_RECT) {
                let desc = ctx
                    .db
                    .skill(sword.weapon_skill)
                    .and_then(|s| s.description.as_str().ok())
                    .unwrap_or_default();
                hit = Some(format!(
                    "{desc}\n\n{}: {}\n{}:   {}",
                    lang(182),
                    sword.clean_cent,
                    lang(96),
                    lang(ATT_LANG[sword.block.min(4)])
                ));
            }
            if ctx.yjfx && over(YJFX) {
                hit = Some(lang(remark_lang(31)));
            }
            if over(SWORD_HIT) {
                hit = Some(lang(remark_lang(29)));
            }
            if over(LJS_HIT) {
                hit = Some(lang(remark_lang(1)));
            }
            if let Some(h) = hit {
                remark = h;
            }
        }

        // ---- 画 ----
        let names = self.main_names(&sword);
        self.pics.preload(am, &names);
        let pic = |n: &str| self.pics.peek(n);
        let rx = self.remark_x.round() as i32;
        p.blit(pic(&format!("{SWORD_DIR}SwordRemark.tga")), rx, 2);
        p.paragraph((rx + 16, 38, rx + 212, 364), REMARK_INK, &remark);
        p.blit(pic(&format!("{SWORD_DIR}SwordFrame.tga")), FRAME.0, FRAME.1);
        let b = sword.block.min(4);
        p.blit(
            pic(&format!("{SWORD_DIR}Sword_{}.tga", ATT_ICON[b])),
            ATT_RECT.0,
            ATT_RECT.1,
        );
        let skill_name = ctx
            .db
            .skill(sword.weapon_skill)
            .and_then(|s| s.name.as_str().ok())
            .unwrap_or_else(|| lang(193));
        p.paragraph(NAME_RECT, SKILL_INK[b], &skill_name);
        p.blit(
            pic(&format!("{SWORD_DIR}Longjingshi.tga")),
            LJS_BG.0,
            LJS_BG.1,
        );
        if ctx.yjfx {
            let v = if over(YJFX) {
                if input.down { 2 } else { 1 }
            } else {
                0
            };
            p.blit(pic(&format!("{SWORD_DIR}yjfx{v}.tga")), YJFX.0, YJFX.1);
        }
        p.blit(
            pic(&format!("{SWORD_DIR}wulingControl.tga")),
            WULING.0,
            WULING.1,
        );
        for k in 0..BLOCKS {
            if sword.all_purged[k] {
                let r = perfect(k);
                p.blit(
                    pic(&format!("{BTN_DIR}{}_perfect.tga", EL_BASE[k])),
                    r.0,
                    r.1,
                );
                continue;
            }
            for (lunar, face) in [(true, "lunar"), (false, "Positive")] {
                let r = btn(k, lunar);
                let side = if lunar { 0 } else { 1 };
                let name = if sword.purged[side][k] {
                    format!("{BTN_DIR}{}_{}_over.tga", EL_BASE[k], face)
                } else {
                    let v = if over(r) {
                        if input.down { 2 } else { 1 }
                    } else {
                        0
                    };
                    format!("{BTN_DIR}{}_{}{}.tga", EL_BASE[k], face, v)
                };
                p.blit(pic(&name), r.0, r.1);
            }
        }
        let mode = if self.views.lunar() {
            "Lunar"
        } else {
            "Positive"
        };
        let v = if over(MODE) {
            if input.down { 2 } else { 1 }
        } else {
            0
        };
        p.blit(pic(&format!("{SWORD_DIR}{mode}{v}.tga")), MODE.0, MODE.1);

        // ---- 点 ----
        let flip = click && over(MODE);
        let _ = self
            .views
            .paint(ui, am, register, canvas, flip, has_weapon, true, ctx.sound);
        let ljs = ctx.state.item_count(DRAGON_STONE);
        p.paragraph(LJS_NUM, NUMBER_INK, &format!("{} {}", lang(97), ljs));

        if click && over(SWORD_HIT) {
            // [移植] 原版双击 3D 剑滑出说明栏、右键收回；这里单击滑出。
            self.remark_out = true;
        }
        if rclick && over(SWORD_HIT) {
            self.remark_out = false;
        }
        if ctx.yjfx && click && over(YJFX) {
            // 去不去得了由服务那边判（`fly_sword`，要再借存档），拒了再回来调 [`Self::refuse_flight`]。
            let _ = (ctx.fly)();
            return true;
        }
        if click {
            for k in 0..BLOCKS {
                if sword.all_purged[k] {
                    continue;
                }
                for lunar in [true, false] {
                    let side = if lunar { 0 } else { 1 };
                    if sword.purged[side][k] || !over(btn(k, lunar)) {
                        continue;
                    }
                    self.wuling_clicked(k, lunar, has_weapon, ctx);
                }
            }
        }
        false
    }

    /// 点了第 `k` 块的阴（阳）按钮（`IDB_SWORD_LUNAR[k]` / `IDB_SWORD_POSITIVE[k]`）。
    fn wuling_clicked(&mut self, k: usize, lunar: bool, has_weapon: bool, ctx: &mut Ctx) {
        if !self.views.ready() || !has_weapon {
            (ctx.sound)("wa004");
            return;
        }
        let lang = ctx.lang;
        if ctx.state.item_count(DRAGON_STONE) == 0 {
            let t = format!("\n     {}\n     {}", lang(242), lang(272));
            self.msg(t, ctx);
            return;
        }
        if self.views.lunar() != lunar {
            let t = format!("\n\n      {}", lang(266));
            self.msg(t, ctx);
            return;
        }
        if !self.views.turned() {
            (ctx.sound)("wa004");
            return;
        }
        let sword = ctx.state.magic_sword_mut();
        sword.block = k;
        let side = if lunar { 0 } else { 1 };
        if sword.all_purged[k] || sword.purged[side][k] {
            (ctx.sound)("wa004");
            return;
        }
        self.purge = Some(Purge::default());
        (ctx.sound)("wa001");
    }

    fn main_names(&self, sword: &sp::MagicSword) -> Vec<String> {
        let mut v: Vec<String> = [
            "SwordRemark.tga",
            "SwordFrame.tga",
            "Longjingshi.tga",
            "wulingControl.tga",
        ]
        .iter()
        .map(|n| format!("{SWORD_DIR}{n}"))
        .collect();
        v.push(format!(
            "{SWORD_DIR}Sword_{}.tga",
            ATT_ICON[sword.block.min(4)]
        ));
        for i in 0..3 {
            v.push(format!("{SWORD_DIR}yjfx{i}.tga"));
            v.push(format!("{SWORD_DIR}Lunar{i}.tga"));
            v.push(format!("{SWORD_DIR}Positive{i}.tga"));
        }
        for el in EL_BASE {
            v.push(format!("{BTN_DIR}{el}_perfect.tga"));
            for face in ["lunar", "Positive"] {
                v.push(format!("{BTN_DIR}{el}_{face}_over.tga"));
                for i in 0..3 {
                    v.push(format!("{BTN_DIR}{el}_{face}{i}.tga"));
                }
            }
        }
        v
    }

    fn purge_names() -> Vec<String> {
        let mut v: Vec<String> = vec![
            "UI\\GameMainUI\\Equip\\ListFrame.tga".into(),
            "UI\\Flex\\Bar.tga".into(),
            "UI\\Flex\\SmallBar.tga".into(),
            "UI\\Flex\\BigBar.tga".into(),
            format!("{SWORD_DIR}PurgeTitle.tga"),
            format!("{SWORD_DIR}List_Grid.tga"),
            format!("{SWORD_DIR}EquipFlag.tga"),
            format!("{SWORD_DIR}PurgeFlag.tga"),
            "UI\\assistant\\Communal\\ListBoxSelPic.tga".into(),
        ];
        for t in TAB_BASE {
            for i in 0..3 {
                v.push(format!("UI\\GameMainUI\\Equip\\{t}{i}.tga"));
            }
        }
        for i in 0..10 {
            v.push(format!(
                "UI\\GameMainUI\\Communal\\Scroll\\scrollbar{i}.tga"
            ));
        }
        for i in 0..3 {
            v.push(format!("UI\\GameMainUI\\Communal\\SortPrice{i}.tga"));
            v.push(format!("{SWORD_DIR}SortGu{i}.tga"));
            v.push(format!("UI\\GameMainUI\\MUIDialog\\Normal\\Yes{i}.tga"));
            v.push(format!("UI\\GameMainUI\\MUIDialog\\Normal\\No{i}.tga"));
        }
        v
    }

    fn paint_purge(
        &mut self,
        p: &Painter,
        am: &AssetManager,
        input: &Input,
        click: bool,
        ctx: &mut Ctx,
    ) {
        let names = Self::purge_names();
        self.pics.preload(am, &names);
        let lang = ctx.lang;
        let over = |r: Rect| p.over(input.mx, input.my, r);
        let state3 = |r: Rect| -> usize {
            if over(r) {
                if input.down { 2 } else { 1 }
            } else {
                0
            }
        };

        let Some(purge) = self.purge.as_ref() else {
            return;
        };
        let page = purge.page;
        let equip = self.equip_rows(ctx, page);
        let purge = self.purge.as_mut().unwrap();

        // ---- 鼠标 ----
        let mut action: Option<&str> = None;
        if click {
            for (b, &x) in TAB_X.iter().enumerate() {
                if over((x, LIST_Y, x + 40, LIST_Y + 50)) {
                    if b == page {
                        (ctx.sound)("wa004");
                    } else {
                        purge.page = b;
                        purge.scroll[0] = 0;
                        purge.sel = None;
                        (ctx.sound)("wa001");
                    }
                }
            }
            if over(SORT_RECTS[0]) {
                action = Some("sort_price");
            } else if over(SORT_RECTS[1]) {
                action = Some("sort_gu");
            } else if over(SORT_RECTS[2]) {
                action = Some("stuff_price");
            } else if over(SORT_RECTS[3]) {
                action = Some("stuff_gu");
            } else if over(OK_RECT) {
                action = Some("ok");
            } else if over(CANCEL_RECT) {
                action = Some("cancel");
            }
        }
        // 两个列表的行、滚动条。
        let lists = [equip.len(), purge.rows.len()];
        let mut row_click: [Option<usize>; 2] = [None, None];
        for (l, &x) in LIST_X.iter().enumerate() {
            let gx = x + GRID_DX;
            let max_scroll = (lists[l] as i32 - ROWS as i32).max(0);
            let list_rect = (gx, GRID_Y0, gx + 298, GRID_Y0 + 288);
            if over(list_rect) {
                if input.wheel > 0. {
                    purge.scroll[l] -= 1;
                }
                if input.wheel < 0. {
                    purge.scroll[l] += 1;
                }
            }
            let (up, dn) = scroll_buttons(gx);
            if click && over(up) {
                purge.scroll[l] -= 1;
            }
            if click && over(dn) {
                purge.scroll[l] += 1;
            }
            purge.scroll[l] = purge.scroll[l].clamp(0, max_scroll);
            for k in 0..ROWS {
                let i = purge.scroll[l] as usize + k;
                let y = GRID_Y0 + ROW_H * k as i32;
                if i < lists[l] && click && over((gx, y, gx + 256, y + ROW_H)) {
                    row_click[l] = Some(i);
                }
            }
        }
        if let Some(i) = row_click[0] {
            if purge.sel == Some(i) {
                // 双击：放进净化列表（`DistributeListData`）。
                let id = equip[i].0;
                if let Some(item) = ctx.db.item(id) {
                    match purge.basket.add(item, page) {
                        Ok(()) => {
                            let existing = purge.basket.entries.iter().position(|e| e.id == id);
                            match existing {
                                Some(a) if a < purge.rows.len() => {
                                    // 按 `m_Distribute` 的下标改那一行。
                                    purge.rows[a].num = purge.basket.entries[a].num;
                                    purge.rows[a].val_id = id;
                                }
                                _ => purge.rows.push(StuffRow {
                                    name_id: id,
                                    num: 1,
                                    val_id: id,
                                }),
                            }
                            (ctx.sound)("wa002");
                        }
                        Err(Refusal::NoGu) => {
                            let t = format!("\n   {}\n   {}", lang(183), lang(271));
                            self.dialog = Some((Dialog::Msg(t), 0.));
                            (ctx.sound)("wa003");
                        }
                        Err(Refusal::TooMuch) => {
                            let t = format!("\n\n       {}", lang(94));
                            self.dialog = Some((Dialog::Msg(t), 0.));
                            (ctx.sound)("wa003");
                        }
                    }
                }
            } else {
                purge.sel = Some(i);
            }
        }
        let purge = self.purge.as_mut().unwrap();
        if let Some(i) = row_click[1] {
            if purge.stuff_sel == Some(i) {
                // 双击：拿回来（`UnDistributeListData`，按 `m_Distribute` 的下标）。
                let id = purge.basket.entries.get(i).map(|e| e.id);
                if let Some(item) = id.and_then(|id| ctx.db.item(id)) {
                    purge.basket.remove(i, item);
                    // 整个列表照 `m_Distribute` 重排。
                    purge.rows = purge
                        .basket
                        .entries
                        .iter()
                        .map(|e| StuffRow {
                            name_id: e.id,
                            num: e.num,
                            val_id: e.id,
                        })
                        .collect();
                    (ctx.sound)("wa002");
                } else {
                    (ctx.sound)("wa004");
                }
            } else {
                purge.stuff_sel = Some(i);
            }
        }
        match action {
            Some("sort_price") | Some("sort_gu") => {
                self.sort = Some(if action == Some("sort_price") { 0 } else { 1 });
                (ctx.sound)("wa001");
            }
            Some("stuff_price") | Some("stuff_gu") => {
                // `SortStuffList`：只重排显示，`m_Distribute` 不动。
                let by_price = action == Some("stuff_price");
                let purge = self.purge.as_mut().unwrap();
                let mut rows: Vec<StuffRow> = purge
                    .basket
                    .entries
                    .iter()
                    .map(|e| StuffRow {
                        name_id: e.id,
                        num: e.num,
                        val_id: e.id,
                    })
                    .collect();
                rows.sort_by_key(|r| {
                    let it = ctx.db.item(r.name_id);
                    if by_price {
                        it.map_or(0, |i| i.price)
                    } else {
                        it.map_or(0, |i| i.ancient_value)
                    }
                });
                purge.rows = rows;
                (ctx.sound)("wa001");
            }
            Some("ok") => {
                // `CheckPurge`：古值是 0 就什么也不做，也不放声。
                let skills = self.skills.clone().unwrap_or_default();
                let purge = self.purge.as_ref().unwrap();
                if let Some(pv) = sp::preview(ctx.state.magic_sword_mut(), &purge.basket, &skills) {
                    let block = ctx.state.magic_sword().block.min(4);
                    let skill = ctx
                        .db
                        .skill(pv.weapon_skill)
                        .and_then(|s| s.name.as_str().ok())
                        .unwrap_or_else(|| lang(193));
                    let line1 = pad_bytes(
                        &format!("      {}   {}", lang(96), lang(ATT_LANG[block])),
                        22,
                    );
                    let text = format!(
                        "{line1}{} {}\n      {}     {}\n      {} {}",
                        lang(182),
                        pv.cent,
                        lang(203),
                        pv.weapon_wu,
                        lang(95),
                        skill
                    );
                    self.dialog = Some((Dialog::Confirm(text), 0.));
                    (ctx.sound)("wa003");
                }
            }
            Some("cancel") => {
                self.purge = None;
                (ctx.sound)("wa001");
                return;
            }
            _ => {}
        }

        // ---- 画 ----
        let pic = |n: &str| self.pics.peek(n);
        let Some(purge) = self.purge.as_ref() else {
            return;
        };
        let equip = self.equip_rows(ctx, purge.page);
        // 左边。
        p.blit(
            pic("UI\\GameMainUI\\Equip\\ListFrame.tga"),
            LIST_X[0],
            LIST_Y,
        );
        let head = format!(
            " {}            {} {}  {}",
            lang(2),
            lang(4),
            lang(196),
            lang(3)
        );
        p.paragraph(
            (LIST_X[0] + 10, LIST_Y + 52, LIST_X[0] + 336, LIST_Y + 84),
            TITLE_INK,
            &head,
        );
        p.bar(
            pic("UI\\Flex\\Bar.tga"),
            (LIST_X[0], LIST_Y + 2, LIST_X[0] + 336, LIST_Y + 34),
            32,
        );
        let rows0: Vec<(u32, i32, u32)> = equip.iter().map(|&(id, n)| (id, n as i32, id)).collect();
        self.paint_list(p, 0, &rows0, purge.scroll[0], purge.sel, input, ctx);
        for (b, &x) in TAB_X.iter().enumerate() {
            let base = format!("UI\\GameMainUI\\Equip\\{}", TAB_BASE[b]);
            if b == purge.page {
                // 当前页签是禁用状态：先画普通图，再盖禁用图（`Weapon2.tga`）。
                p.blit(pic(&format!("{base}0.tga")), x, LIST_Y);
                p.blit(pic(&format!("{base}2.tga")), x, LIST_Y);
            } else {
                let v = state3((x, LIST_Y, x + 40, LIST_Y + 50));
                p.blit(pic(&format!("{base}{v}.tga")), x, LIST_Y);
            }
        }
        // 右边。
        p.blit(
            pic("UI\\GameMainUI\\Equip\\ListFrame.tga"),
            LIST_X[1],
            LIST_Y,
        );
        p.paragraph(
            (LIST_X[1] + 10, LIST_Y + 52, LIST_X[1] + 336, LIST_Y + 84),
            TITLE_INK,
            &head,
        );
        p.blit(
            pic(&format!("{SWORD_DIR}PurgeTitle.tga")),
            LIST_X[1] + 7,
            LIST_Y,
        );
        let rows1: Vec<(u32, i32, u32)> = purge
            .rows
            .iter()
            .map(|r| (r.name_id, r.num, r.val_id))
            .collect();
        self.paint_list(p, 1, &rows1, purge.scroll[1], purge.stuff_sel, input, ctx);
        // 排序、装饰条、标志、按钮、古值。
        let sort_art = [
            "UI\\GameMainUI\\Communal\\SortPrice".to_string(),
            format!("{SWORD_DIR}SortGu"),
            "UI\\GameMainUI\\Communal\\SortPrice".to_string(),
            format!("{SWORD_DIR}SortGu"),
        ];
        for (r, base) in SORT_RECTS.iter().zip(sort_art.iter()) {
            p.blit(pic(&format!("{base}{}.tga", state3(*r))), r.0, r.1);
        }
        p.bar(pic("UI\\Flex\\SmallBar.tga"), (43, 93, 731, 111), 40);
        p.bar(pic("UI\\Flex\\BigBar.tga"), (42, 496, 742, 531), 40);
        p.blit(pic(&format!("{SWORD_DIR}EquipFlag.tga")), 162, 76);
        p.blit(pic(&format!("{SWORD_DIR}PurgeFlag.tga")), 502, 76);
        p.blit(
            pic(&format!(
                "UI\\GameMainUI\\MUIDialog\\Normal\\Yes{}.tga",
                state3(OK_RECT)
            )),
            OK_RECT.0,
            OK_RECT.1,
        );
        p.blit(
            pic(&format!(
                "UI\\GameMainUI\\MUIDialog\\Normal\\No{}.tga",
                state3(CANCEL_RECT)
            )),
            CANCEL_RECT.0,
            CANCEL_RECT.1,
        );
        p.paragraph(
            GU_RECT,
            REMARK_INK,
            &format!("{}/{}", purge.basket.gu, sp::GU_LIMIT),
        );
    }

    /// 一个列表：12 行底格、内容（名称 / 数量 / 古 / 半价）、选中条、滚动条。`rows` 是
    /// (名字取自, 数量, 数值取自)。
    #[allow(clippy::too_many_arguments)]
    fn paint_list(
        &self,
        p: &Painter,
        l: usize,
        rows: &[(u32, i32, u32)],
        scroll: i32,
        sel: Option<usize>,
        input: &Input,
        ctx: &Ctx,
    ) {
        let pic = |n: &str| self.pics.peek(n);
        let gx = LIST_X[l] + GRID_DX;
        for k in 0..ROWS {
            let y = GRID_Y0 + ROW_H * k as i32;
            p.blit(pic(&format!("{SWORD_DIR}List_Grid.tga")), gx, y);
            let i = scroll as usize + k;
            let Some(&(name_id, num, val_id)) = rows.get(i) else {
                continue;
            };
            if sel == Some(i) {
                p.additive(pic("UI\\assistant\\Communal\\ListBoxSelPic.tga"), gx, y);
            }
            let ty = y as f32 + 3.5;
            let it = ctx.db.item(val_id);
            let cols = [
                item_name(ctx.db, name_id),
                num.to_string(),
                it.map_or(0, |i| i.ancient_value).to_string(),
                it.map_or(0, |i| i.price / 2).to_string(),
            ];
            for (c, s) in cols.iter().enumerate() {
                p.text((gx + COL_X[c] + 8) as f32, ty, LIST_INK, s);
            }
        }
        // 滚动条（`MU_SCROLL_PIC`：底 3、上 4/5/6、下 7/8/9、滑块 0）。
        let sb = "UI\\GameMainUI\\Communal\\Scroll\\scrollbar";
        let (up, dn) = scroll_buttons(gx);
        p.blit(pic(&format!("{sb}3.tga")), gx + 298 - 38, GRID_Y0 - 1);
        let v = |r: Rect, base: i32| -> i32 {
            if p.over(input.mx, input.my, r) {
                base + if input.down { 2 } else { 1 }
            } else {
                base
            }
        };
        p.blit(pic(&format!("{sb}{}.tga", v(up, 4))), up.0, up.1);
        p.blit(pic(&format!("{sb}{}.tga", v(dn, 7))), dn.0, dn.1);
        let max_scroll = (rows.len() as i32 - ROWS as i32).max(0);
        let frac = if max_scroll > 0 {
            scroll as f32 / max_scroll as f32
        } else {
            0.
        };
        let (top, bottom) = (up.3 + 4, dn.1 - 4);
        let thumb_y = top as f32 + frac * (bottom - top - 28) as f32;
        p.blit(pic(&format!("{sb}0.tga")), up.0 - 2, thumb_y.round() as i32);
    }

    fn paint_dialog(&mut self, p: &Painter, am: &AssetManager) {
        let Some((dialog, t)) = self.dialog.clone() else {
            return;
        };
        let mut names: Vec<String> = (0..9).map(|i| format!("UI\\Flex\\Note{i}.tga")).collect();
        names.push("UI\\GameMainUI\\MUIDialog\\Normal\\DlgBackground.tga".into());
        for i in 0..3 {
            names.push(format!("UI\\GameMainUI\\MUIDialog\\Normal\\Yes{i}.tga"));
            names.push(format!("UI\\GameMainUI\\MUIDialog\\Normal\\No{i}.tga"));
        }
        self.pics.preload(am, &names);
        let pic = |n: &str| self.pics.peek(n);
        let text = |r: Rect, s: &str| {
            for (k, line) in s.split('\n').enumerate() {
                p.text(
                    (r.0 + DLG_INSET) as f32,
                    (r.1 + DLG_INSET + 16 * k as i32) as f32,
                    DIALOG_INK,
                    line,
                );
            }
        };
        match dialog {
            Dialog::Msg(s) => {
                // `DLG_MSGBOX`：Note 九宫格 0.5 秒从中心展开，展开完出字。
                let x = (t / OPEN_SECS).min(1.);
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
                let tile = pic("UI\\Flex\\Note0.tga").map_or(32, |p| p.width());
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
                        p.stretch(pic(&format!("UI\\Flex\\Note{i}.tga")), *q);
                    }
                }
                if x >= 1. {
                    text(m, &s);
                }
            }
            Dialog::Confirm(s) => {
                p.blit_rect(
                    pic("UI\\GameMainUI\\MUIDialog\\Normal\\DlgBackground.tga"),
                    YESNO_RECT,
                );
                text(YESNO_RECT, &s);
                for (r, base) in [(yes_rect(), "Yes"), (no_rect(), "No")] {
                    p.blit(
                        pic(&format!("UI\\GameMainUI\\MUIDialog\\Normal\\{base}0.tga")),
                        r.0,
                        r.1,
                    );
                }
            }
        }
    }
}

fn yes_rect() -> Rect {
    let r = YESNO_RECT;
    (r.0 + 26, r.3 - 32, r.0 + 116, r.3 - 6)
}

fn no_rect() -> Rect {
    let r = YESNO_RECT;
    (r.2 - 116, r.3 - 32, r.2 - 26, r.3 - 6)
}

/// 列表滚动条的上下键：列表左边 `gx`，上键 (右−31, 上+15) 22×26，下键再往下 206。
fn scroll_buttons(gx: i32) -> (Rect, Rect) {
    let right = gx + 298;
    let up = (right - 31, GRID_Y0 - 2 + 15, right - 9, GRID_Y0 - 2 + 41);
    let dn = (up.0, up.3 + 206, up.2, up.3 + 232);
    (up, dn)
}
