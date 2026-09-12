//! 买卖页和冶炼页的物品 3D 预览：原版页面上的 3D 视图控件，选中物品时载入
//! `Item\<模型>\<模型>.pol` 并把贴图换成 `<模型><图标>.tga`（`gbGeomNode::ReplaceTexture`）。
//! 离屏渲染在 [`crate::openpal3::model_view`]，这里只管选模型和贴到说明面板的黑框上。
//! 逆向记录在 `switch/re/notes/dealer_layout.md` §6b。

use std::path::PathBuf;
use std::rc::Rc;

use super::dealer_layout::Rect;
use crate::openpal3::model_view::{ModelSource, ModelView, v_range};
use crate::scripting::sce::SceState;

pub use crate::openpal3::model_view::ViewCamera as PreviewCamera;

pub(super) struct ItemPreview {
    view: ModelView,
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
        let am = state.asset_mgr().clone();
        let db = state.global_state().game_db().cloned()?;
        let row = db.item(id)?;
        let model = row.model_name.as_str().unwrap_or_default();
        // 原版拿模型名和一个全局串比，相等就写 "NONE"、什么也不载 [推断：空名]。
        if model.is_empty() || model.eq_ignore_ascii_case("NONE") {
            return None;
        }
        let icon = row.icon_name.as_str().unwrap_or_default();
        let source = ModelSource::Pol {
            path: PathBuf::from(format!("/basedata/basedata/item/{model}/{model}.pol")),
            texture: Some(format!("{model}{icon}.tga")),
        };
        let view = ModelView::build(&am, &source, camera, 0.)?;
        Some(Rc::new(Self { view, area }))
    }

    pub(super) fn render(&self, delta_sec: f32) {
        self.view.render(delta_sec);
    }

    /// 把渲染目标里 `area` 那一块贴到界面同一位置。
    pub(super) fn draw(&self, list: &imgui::DrawListMut<'_>, at: &dyn Fn(i32, i32) -> [f32; 2]) {
        let Some(tex) = self.view.texture_id() else {
            return;
        };
        let (x0, y0, x1, y1) = self.area;
        let (u0, u1) = (x0 as f32 / 800., x1 as f32 / 800.);
        let (v0, v1) = v_range(y0 as f32 / 600., y1 as f32 / 600.);
        list.add_image(imgui::TextureId::new(tex as usize), at(x0, y0), at(x1, y1))
            .uv_min([u0, v0])
            .uv_max([u1, v1])
            .build();
    }
}
