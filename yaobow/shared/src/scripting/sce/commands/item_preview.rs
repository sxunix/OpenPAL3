//! 界面里的物品 3D 预览：原版在页面上放一个 3D 视图控件（构造 `FUN_00439030`，
//! 虚表 0x56ac20），自己一台正交相机，选中物品时载入 `Item\<模型>\<模型>.pol` 并把
//! 贴图换成 `<模型><图标>.tga`（`gbGeomNode::ReplaceTexture`）。买卖页和冶炼页都有。
//!
//! 移植端的界面总在 3D 之上，不能照原版在主画面上直接画：改成一个只放这件物品
//! 的离屏小场景，每帧画进渲染目标，再把说明面板黑框那一块贴到界面上。逆向记录在
//! `switch/re/notes/dealer_layout.md` §6b。

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use super::dealer_layout::Rect;
use crate::scripting::sce::SceState;
use crosscom::ComRc;
use radiance::comdef::IScene;
use radiance::math::Vec3;
use radiance::rendering::{RenderTarget, RenderingEngine};
use radiance::scene::{CoreScene, ISceneExt, SceneLighting};

/// `FUN_00439930(x, y, h)` 的 h 和写死的相机 z。
const HEIGHT: f32 = 600.;
const EYE_Z: f32 = 1600.;

/// 渲染目标的大小：4:3，和 800×600 的界面同比例，只取黑框那一块。
const TARGET: (u32, u32) = (1024, 768);

/// 相机：`FUN_0043a070(size)`（正交半宽）+ `FUN_00439930(x, y, 600)`。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PreviewCamera {
    pub size: f32,
    pub x: i32,
    pub y: i32,
}

impl PreviewCamera {
    /// 眼睛和注视点（`FUN_00439930`）：原点落在屏幕 (x, y) 附近。纵向也乘的是
    /// 半宽，不是半高——照原样。
    pub fn eye_target(&self) -> ([f32; 3], [f32; 3]) {
        let dx = -((2. * self.x as f32 / 800. - 1.) * self.size);
        let dy = (1. - 2. * self.y as f32 / 600.) * self.size;
        ([dx, HEIGHT - dy, EYE_Z], [dx, -dy, 0.])
    }

    /// 原版是正交相机（`gbCamera` `+0` 视角 0、`+4` 正交半宽）。移植端的相机只有
    /// 透视，用很窄的视角从同一个位置看：注视点处的半宽正好是 `size` [近似]。
    pub fn fov43(&self) -> f32 {
        let d = (HEIGHT * HEIGHT + EYE_Z * EYE_Z).sqrt();
        2. * (self.size / d).atan()
    }
}

// 退下来的场景和渲染目标，等下一次 [`ItemPreview::build`] 再真正释放。
//
// Vulkan 的 `render_scene_to_target` 当场提交，把目标自己的「画完」信号量交给
// 引擎，本帧主画面提交时等它。界面在 `update` 里画完预览、同一帧又被关掉的话
// （冶炼页出完货按离开），目标一析构信号量就没了，主画面去等一个已销毁的信号量，
// 进程直接崩。所以析构时先挂在这里，至少隔一帧再放。
thread_local! {
    static RETIRED: RefCell<Vec<(ComRc<IScene>, Box<dyn RenderTarget>)>> =
        const { RefCell::new(Vec::new()) };
}

pub(super) struct ItemPreview {
    scene: ComRc<IScene>,
    target: RefCell<Option<Box<dyn RenderTarget>>>,
    engine: Rc<RefCell<dyn RenderingEngine>>,
    /// 贴到界面上的那一块。
    area: Rect,
}

impl ItemPreview {
    pub(super) fn build(
        state: &SceState,
        id: u32,
        camera: PreviewCamera,
        area: Rect,
    ) -> Option<Rc<Self>> {
        use mini_fs::StoreExt;
        // 上一回退下来的已经隔了至少一帧，这时放掉是安全的。
        RETIRED.with(|r| r.borrow_mut().clear());
        let am = state.asset_mgr().clone();
        let engine = am.rendering_engine()?;
        let db = state.global_state().game_db().cloned()?;
        let row = db.item(id)?;
        let model = row.model_name.as_str().unwrap_or_default();
        // 原版拿模型名和一个全局串比，相等就写 "NONE"、什么也不载 [推断：空名]。
        if model.is_empty() || model.eq_ignore_ascii_case("NONE") {
            return None;
        }
        let icon = row.icon_name.as_str().unwrap_or_default();
        let path = PathBuf::from(format!("/basedata/basedata/item/{model}/{model}.pol"));
        if am.vfs().open(&path).is_err() {
            return None;
        }
        let entity = crate::openpal3::loaders::pol::create_entity_from_pol_model_with_texture(
            &am.component_factory(),
            am.vfs(),
            &path,
            format!("ITEM_PREVIEW_{id}"),
            true,
            Some(&format!("{model}{icon}.tga")),
        );
        let scene = CoreScene::create();
        // 原版给这个视图挂了一盏方向光（绕 X 45°、绕 Y 90°）；这里和鉴宝一样
        // 用满环境光 [推断]。
        scene.set_lighting(SceneLighting::new([1., 1., 1.], vec![]));
        entity.load();
        scene.add_entity(entity);

        let (eye, at) = camera.eye_target();
        {
            let mut cam = scene.camera_mut();
            cam.set_fov43(camera.fov43());
            cam.set_aspect(4. / 3.);
            cam.transform_mut()
                .set_position(&Vec3::new(eye[0], eye[1], eye[2]))
                .look_at(&Vec3::new(at[0], at[1], at[2]));
        }
        let target = am
            .component_factory()
            .create_render_target(TARGET.0, TARGET.1);
        Some(Rc::new(Self {
            scene,
            target: RefCell::new(Some(target)),
            engine,
            area,
        }))
    }

    pub(super) fn render(&self, delta_sec: f32) {
        self.scene.update(delta_sec);
        if let Some(target) = self.target.borrow_mut().as_mut() {
            self.engine
                .borrow_mut()
                .render_scene_to_target(self.scene.clone(), target.as_mut());
        }
    }

    /// 把渲染目标里 `area` 那一块贴到界面同一位置。GL 的离屏纹理是自下而上的，
    /// Switch 上要把 v 翻过来 [推断：还没在机器上看过]。
    pub(super) fn draw(&self, list: &imgui::DrawListMut<'_>, at: &dyn Fn(i32, i32) -> [f32; 2]) {
        let (x0, y0, x1, y1) = self.area;
        let Some(tex) = self.target.borrow().as_ref().map(|t| t.imgui_texture_id()) else {
            return;
        };
        let id = imgui::TextureId::new(tex as usize);
        let (u0, u1) = (x0 as f32 / 800., x1 as f32 / 800.);
        let (mut v0, mut v1) = (y0 as f32 / 600., y1 as f32 / 600.);
        if cfg!(switchgl) {
            (v0, v1) = (1. - v0, 1. - v1);
        }
        list.add_image(id, at(x0, y0), at(x1, y1))
            .uv_min([u0, v0])
            .uv_max([u1, v1])
            .build();
    }
}

impl Drop for ItemPreview {
    fn drop(&mut self) {
        if let Some(target) = self.target.get_mut().take() {
            let scene = self.scene.clone();
            RETIRED.with(|r| r.borrow_mut().push((scene, target)));
        }
    }
}
