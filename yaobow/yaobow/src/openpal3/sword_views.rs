//! 御剑页的两个 3D 视图（源码 `gamemainui/swordui.cpp`，exe `FUN_004ab6b0` 建）：
//!
//! - 魔剑 `UI\GameMainUI\Sword\Data\Sword.pol`：`SetOrthosize(100)`、`SetOriginPt(392, 400)`（正看）。
//!   每次打开页面有一段入场：正交半宽从 1000 每秒减 500 到 100（相机的平移是按 100 定的，所以剑从
//!   画面中间附近一边变大一边落到位），同时转角从 −1080° 每秒加 460° 到 0；转回 0 以后用每秒 98° 再
//!   转 180°（`SetRotateMode(2, 180)`）。阴/阳按钮（`IDB_SWORD_CTRLLUNAR` / `…POSITIVE`）各让它再转 180°，
//!   只在入场完了、上一次转完了、景天拿着魔剑（6603）时才算，放 wa001，否则 wa004。
//!   剑的贴图按净化进度从 `J131Src` / `J131Dst` 拼（`PurgeEffect`）；一块都没净化时就是 `J131Src`，
//!   和模型自带的 `J131.tga` 逐像素相同。移植端还没有净化，所以不换贴图。
//! - 龙晶石 `…\ljs.POL`：`SetOrthosize(460)`、`SetOriginPt(730, 152)`（exe `FUN_00439850(0x2da, 0x98)`；
//!   源码写的 726）、每秒 98° 自转。
//!
//! 进净化模式时两个都藏起来（`SwitchPurge`）；移植端没有净化模式，一直画。

use std::time::Instant;

use crosscom::ComRc;
use radiance::comdef::IUiHost;
use shared::openpal3::asset_manager::AssetManager;
use shared::openpal3::model_view::{ModelSource, ModelView, SPIN_DEG_PER_SEC, ViewCamera, v_range};

const SWORD_POL: &str = "/basedata/basedata/ui/GameMainUI/Sword/Data/Sword.pol";
const LJS_POL: &str = "/basedata/basedata/ui/GameMainUI/Sword/Data/ljs.POL";
/// 景天拿着的是魔剑才能翻面（`m_bHasWeapon`）。
pub const MAGIC_SWORD: u32 = 6603;

const SWORD_SIZE: f32 = 100.;
const SWORD_START_SIZE: f32 = 1000.;
const SWORD_ZOOM_PER_SEC: f32 = 500.;
const SWORD_START_ROT: f32 = -1080.;
const SWORD_ROT_PER_SEC: f32 = 460.;
/// `m_angle`：每次转 180°。
const SWORD_TURN: f32 = 180.;
/// 页面隔了这么久没画，就当关掉又打开了（原版在 `UIWM_ACTIVE` 里重置入场）。
const REOPEN_SECS: f32 = 0.25;

struct Slot {
    view: ModelView,
    com_id: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Stage {
    /// 入场：半宽和转角都在走。
    Intro { size: f32, rot: f32 },
    /// 入场完了（`m_CanCtrlBtn`）。
    Ready,
}

pub struct SwordViews {
    sword: Option<Slot>,
    ljs: Option<Slot>,
    stage: Stage,
    /// `m_is3dLunar`：按钮显示「阴」。
    lunar: bool,
    last: Option<Instant>,
}

impl Default for SwordViews {
    fn default() -> Self {
        Self {
            sword: None,
            ljs: None,
            stage: Stage::Intro {
                size: SWORD_START_SIZE,
                rot: SWORD_START_ROT,
            },
            lunar: true,
            last: None,
        }
    }
}

/// 这一帧画在哪：800×600 画布的原点和缩放。
pub struct Canvas {
    pub ox: f32,
    pub oy: f32,
    pub scale: f32,
}

impl SwordViews {
    pub fn lunar(&self) -> bool {
        self.lunar
    }

    fn build(
        am: &std::rc::Rc<AssetManager>,
        path: &str,
        size: f32,
        x: i32,
        y: i32,
    ) -> Option<Slot> {
        let source = ModelSource::Pol {
            path: path.into(),
            texture: None,
        };
        let camera = ViewCamera {
            size,
            x,
            y,
            tilt: None,
        };
        let view = ModelView::build(am, &source, camera, 0.)?;
        Some(Slot {
            view,
            com_id: super::service::next_ui_texture_com_id(),
        })
    }

    /// 每帧：推进入场、处理翻面请求（`flip`），画两个视图。返回翻面请求算不算数；
    /// `sound` 放 wa001 / wa004。
    pub fn paint(
        &mut self,
        ui: &ComRc<IUiHost>,
        am: &std::rc::Rc<AssetManager>,
        register: &dyn Fn(i64, imgui::TextureId),
        canvas: &Canvas,
        flip: bool,
        has_sword: bool,
        sound: &dyn Fn(&str),
    ) -> bool {
        let now = Instant::now();
        let prev = self.last.replace(now);
        let dt = prev.map_or(0., |t| now.duration_since(t).as_secs_f32());
        if prev.is_some() && dt > REOPEN_SECS {
            // 关掉又打开：入场重来，按钮回到「阴」。
            self.stage = Stage::Intro {
                size: SWORD_START_SIZE,
                rot: SWORD_START_ROT,
            };
            self.lunar = true;
        }
        let dt = dt.min(0.1);
        if self.sword.is_none() {
            self.sword = Self::build(am, SWORD_POL, SWORD_SIZE, 392, 400);
        }
        if self.ljs.is_none() {
            self.ljs = Self::build(am, LJS_POL, 460., 730, 152);
            if let Some(l) = &self.ljs {
                l.view.set_spin(SPIN_DEG_PER_SEC);
            }
        }

        let mut accepted = false;
        if let Some(sword) = &self.sword {
            if let Stage::Intro { size, rot } = self.stage {
                // `m_bSwordMotion` / `m_bSwordRot`。
                let size = (size - SWORD_ZOOM_PER_SEC * dt).max(SWORD_SIZE);
                let rot = (rot + SWORD_ROT_PER_SEC * dt).min(0.);
                sword.view.set_ortho(size);
                sword.view.set_angle(rot);
                self.stage = Stage::Intro { size, rot };
                if rot == 0. {
                    sword.view.set_angle(0.);
                    sword.view.turn_by(SWORD_TURN, SPIN_DEG_PER_SEC);
                    sword.view.set_ortho(SWORD_SIZE);
                    self.stage = Stage::Ready;
                }
            }
            if flip {
                if self.stage == Stage::Ready && has_sword && sword.view.turned() {
                    self.lunar = !self.lunar;
                    sword.view.turn_by(SWORD_TURN, SPIN_DEG_PER_SEC);
                    sound("wa001");
                    accepted = true;
                } else {
                    sound("wa004");
                }
            }
        }

        // 龙晶石先建、先画，剑在它上面（两个视图的画面不重叠）。
        for slot in [&self.ljs, &self.sword].into_iter().flatten() {
            slot.view.render(dt);
            let Some(tex) = slot.view.texture_id() else {
                continue;
            };
            register(slot.com_id, imgui::TextureId::new(tex as usize));
            let (v0, v1) = v_range(0., 1.);
            ui.image_rect(
                slot.com_id as i32,
                canvas.ox,
                canvas.oy,
                canvas.ox + 800. * canvas.scale,
                canvas.oy + 600. * canvas.scale,
                0.,
                v0,
                1.,
                v1,
            );
        }
        accepted
    }
}
