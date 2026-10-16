//! 141 设置全局屏幕特效 — `LAB_004388c0`（源码 `script/scriptcmd.cpp` 的 `cmd_ScrEft`）。
//!
//! 先把两个效果都关掉，再按参数开一个：
//!
//! | 参数 | 效果 |
//! |---|---|
//! | 0 | `UnderWater`（`effect/UnderWater.cpp`）：场景先画进离屏目标，再按一张凹凸贴图把画面波纹状扭曲着贴回屏幕——水下 |
//! | 1 | `GrayScale`（`effect/GrayScale.cpp`）：画场景时关掉蓝通道（`D3DRS_COLORWRITEENABLE = R|G`，exe `FUN_004b6df0` 写 3），画完再用 `shadow.gbf`（`DESTCOLOR, ZERO`）把 `EffectScn\Frame.Tga` 乘上去——偏黄的回忆色调加暗角 |
//! | 其它（脚本里都是 -1） | 两个都关 |
//!
//! 先前把 0 当成「上一帧和这一帧混合的拖影」，1 只贴了暗角，都不对。
//!
//! 命令不阻塞：只改一个标志，每帧由 [`crate::scripting::sce::SceState::render_screen_effect`] 画，
//! 直到下一个 141 关掉。

use std::cell::RefCell;
use std::rc::Rc;

use crate::openpal3::asset_manager::AssetManager;
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::{IScene, ISceneManager};
use radiance::rendering::{RenderTarget, RenderingEngine, Sprite};

/// `GrayScale::Create` 读的那张图。
pub const FRAME_TEXTURE: &str = "Frame.Tga";

/// 现在开着的屏幕效果。
#[derive(Clone, Default)]
pub enum ScreenEffect {
    #[default]
    None,
    /// 参数 0。
    UnderWater,
    /// 参数 1，带 `Frame.Tga`（读不到就只去蓝）。
    Memory(Option<Rc<Sprite>>),
}

/// 141 ScrEft.
#[derive(Debug, Clone)]
pub struct SceCommandScreenEffect {
    effect: i32,
}

impl SceCommand for SceCommandScreenEffect {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        let effect = match self.effect {
            0 => ScreenEffect::UnderWater,
            1 => {
                let asset_mgr = state.asset_mgr();
                let frame = asset_mgr.load_sprite(asset_mgr.effect_scn_path(FRAME_TEXTURE));
                if frame.is_none() {
                    log::warn!("ScrEft: 读不到 EffectScn\\{FRAME_TEXTURE}");
                }
                ScreenEffect::Memory(frame)
            }
            _ => ScreenEffect::None,
        };
        state.set_screen_effect(effect);
        true
    }
}

impl SceCommandScreenEffect {
    pub fn new(effect: i32) -> Self {
        Self { effect }
    }
}

/// 参数 1 的画法。原版场景清屏是黑的（`pal3.cpp:395` `clearobj.color = (0,0,0)`），蓝通道不写就一直是 0，
/// 等于场景画完乘 (1, 1, 0)；再把 `Frame.Tga` 乘上去（`shadow.gbf`：`DESTCOLOR, ZERO`，颜色取贴图）。
/// 两步都用乘法混合，画进背景绘制列表。
pub fn draw_memory_tint(ui: &Ui, frame: Option<&Sprite>) {
    let size = ui.io().display_size;
    let list = ui.get_background_draw_list();
    radiance::imgui::push_background_blend_multiply();
    list.add_rect([0., 0.], size, [1., 1., 0., 1.])
        .filled(true)
        .build();
    if let Some(frame) = frame {
        // `GrayScale::End`：NDC −1..1 铺满，uv (0,0)-(1,1)。
        list.add_image(frame.imgui_texture_id(), [0., 0.], size)
            .build();
    }
    radiance::imgui::push_background_blend_normal();
}

/// 凹凸贴图的格数：原版是逐像素的偏移，这里在这么多格的网格顶点上算同一个偏移、格子里线性插值
/// [推断：近似，看不出差别的密度]。
const GRID_X: usize = 48;
const GRID_Y: usize = 36;

/// `rand() % 16` 的均值：原版每个凹凸像素加一个 0..15 的随机数，这里取它的均值。
const NOISE_MEAN: f32 = 7.5;

/// 一个点的凹凸偏移（`UnderWater::Create` 生成 V8U8 凹凸贴图的式子 + `D3DTOP_BUMPENVMAP`，
/// `BUMPENVMAT` 是 0.01 的对角阵）。`fx`、`fy` 是凹凸贴图上的位置减 0.5。
pub fn bump_offset(fx: f32, fy: f32) -> (f32, f32) {
    let a = 4. * std::f32::consts::PI * (fx + fy);
    let u = 48. * a.cos() + NOISE_MEAN;
    let v = 48. * a.sin() + NOISE_MEAN;
    // `(char)` 截向零，V8U8 的有符号字节按 /127 归一。
    let du = ((u + v) * 0.5).trunc() / 127.;
    let dv = (u - v).trunc() / 127.;
    (0.01 * du, 0.01 * dv)
}

/// 屏幕上 (`sx`, `sy`)（0..1，y 朝下）这一点该取离屏画面的哪里。凹凸贴图在屏幕上铺一遍（u 0..1），竖着
/// 随时间往上滚：`tv1 = sy + 0.5·t`（`UnderWater::Update`：t 每秒加 0.4，超过 2 减 2）。
pub fn under_water_uv(sx: f32, sy: f32, time: f32) -> [f32; 2] {
    let fx = sx.rem_euclid(1.) - 0.5;
    let fy = (sy + 0.5 * time).rem_euclid(1.) - 0.5;
    let (du, dv) = bump_offset(fx, fy);
    // 离屏画面那一级是 `CLAMP`。
    [(sx + du).clamp(0., 1.), (sy + dv).clamp(0., 1.)]
}

/// 水下：离屏目标和时钟。
#[derive(Default)]
pub struct UnderWater {
    target: RefCell<Option<Box<dyn RenderTarget>>>,
    time: f32,
}

impl UnderWater {
    /// `UnderWater::Update`：`m_fTime += dt × 0.4`，到 2 减 2。
    pub fn update(&mut self, delta_sec: f32) {
        self.time += delta_sec * 0.4;
        if self.time > 2. {
            self.time -= 2.;
        }
    }

    /// 把场景画进离屏目标（`UnderWater::Begin`），再按凹凸偏移整屏贴回（`End`，不混合）。
    pub fn render(
        &self,
        ui: &Ui,
        asset_mgr: &AssetManager,
        engine: &Rc<RefCell<dyn RenderingEngine>>,
        scene: ComRc<IScene>,
    ) {
        let size = ui.io().display_size;
        let scale = ui.io().display_framebuffer_scale;
        let (w, h) = (
            (size[0] * scale[0]).round().max(1.) as u32,
            (size[1] * scale[1]).round().max(1.) as u32,
        );
        let mut slot = self.target.borrow_mut();
        let target =
            slot.get_or_insert_with(|| asset_mgr.component_factory().create_render_target(w, h));
        if target.extent() != (w, h) {
            target.resize(w, h);
        }
        engine
            .borrow_mut()
            .render_scene_to_target(scene, target.as_mut());
        let tex = imgui::TextureId::new(target.imgui_texture_id() as usize);

        let list = ui.get_background_draw_list();
        let uv = |i: usize, j: usize| {
            let (sx, sy) = (i as f32 / GRID_X as f32, j as f32 / GRID_Y as f32);
            let [u, v] = under_water_uv(sx, sy, self.time);
            let v = if cfg!(switchgl) { 1. - v } else { v };
            [u, v]
        };
        let at = |i: usize, j: usize| {
            [
                size[0] * i as f32 / GRID_X as f32,
                size[1] * j as f32 / GRID_Y as f32,
            ]
        };
        for j in 0..GRID_Y {
            for i in 0..GRID_X {
                list.add_image_quad(tex, at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j + 1))
                    .uv(uv(i, j), uv(i + 1, j), uv(i + 1, j + 1), uv(i, j + 1))
                    .build();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ripple_moves_a_few_thousandths_of_the_screen() {
        // 48·cos、48·sin 加噪声，最多 0.0055 左右——800 宽的画面上四五个像素。
        let mut max = (0f32, 0f32);
        for k in 0..200 {
            let (du, dv) = bump_offset(k as f32 / 200. - 0.5, 0.);
            max = (max.0.max(du.abs()), max.1.max(dv.abs()));
        }
        assert!(max.0 > 0.002 && max.0 < 0.004, "{max:?}");
        assert!(max.1 > 0.004 && max.1 < 0.007, "{max:?}");
    }

    #[test]
    fn the_ripple_scrolls_with_time_and_repeats_after_two() {
        let a = under_water_uv(0.3, 0.4, 0.);
        let b = under_water_uv(0.3, 0.4, 0.5);
        let c = under_water_uv(0.3, 0.4, 2.);
        assert_ne!(a, b);
        assert!((a[0] - c[0]).abs() < 1e-6 && (a[1] - c[1]).abs() < 1e-6);
    }
}
