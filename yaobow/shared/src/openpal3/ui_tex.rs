//! 给 SCE 命令的原生界面取贴图：先查 `ui\UILib\UI_opt.tli` 图集，查不到再读散文件。
//!
//! 原版 `FUN_004426b0(path, useAtlas)` 在 useAtlas 非 0 时按名字查 `UI_opt.tli`
//! （`FUN_0053fcf0` / `FUN_0053fdd0`），取那一页上的子矩形，按原生尺寸贴在控件
//! rect 左上角（`switch/re/notes/state_page_layout.md` §0.1）。`ui\Assistant\*`、
//! `GameMainUI\Communal\Info.tga` 这类贴图只在图集里，包里没有散文件；
//! `ui\flex\framebar.tga` 这类图集里没有、只有散文件。两条路都走，名字照原版写。
//!
//! p7 那边的菜单走 `Pal3UiAtlas`（直接在图集页上取 uv）；这里给 Rust 侧的
//! 浮层用，裁成独立的 `Sprite`，好和 `encamp::draw_sprite` 那套一起画。

use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use common::store_ext::StoreExt2;
use radiance::rendering::Sprite;

use super::asset_manager::AssetManager;
use super::loaders::tli::TliDict;

const TLI_PATH: &str = "/basedata/basedata/ui/UILib/UI_opt.tli";
const UILIB_DIR: &str = "/basedata/basedata/ui/UILib";
const BASEDATA: &str = "/basedata/basedata";

/// 按名字取贴图，取过的缓存起来。图集页整张解一次，按需裁。
#[derive(Default)]
pub struct UiTextures {
    dict: TliDict,
    pages: HashMap<String, Option<image::RgbaImage>>,
    cache: HashMap<String, Option<Rc<Sprite>>>,
}

impl std::fmt::Debug for UiTextures {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UiTextures")
            .field("entries", &self.dict.len())
            .field("cached", &self.cache.len())
            .finish()
    }
}

impl UiTextures {
    pub fn new(asset_mgr: &AssetManager) -> Self {
        let dict = asset_mgr
            .vfs()
            .read_to_end(TLI_PATH)
            .map(|bytes| TliDict::parse(&bytes))
            .unwrap_or_else(|e| {
                log::warn!("UiTextures: 读不到 {TLI_PATH}: {e}");
                TliDict::default()
            });
        Self {
            dict,
            ..Default::default()
        }
    }

    /// `name` 照原版写（`UI\Assistant\Deal\FlagT.tga`），大小写和斜杠方向都不管。
    pub fn get(&mut self, asset_mgr: &AssetManager, name: &str) -> Option<Rc<Sprite>> {
        let key = name.to_ascii_lowercase().replace('\\', "/");
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let sprite = self
            .image(asset_mgr, name)
            .map(|img| asset_mgr.sprite_from_image(img));
        if sprite.is_none() {
            log::warn!("UiTextures: 图集和包里都没有 {name}");
        }
        self.cache.insert(key, sprite.clone());
        sprite
    }

    /// 同 [`Self::get`]，但贴图是**加法混合**上去的，取回来之前先按
    /// [`bake_additive`] 折成普通 alpha 混合能画的样子。`tint` 是原版画它时
    /// 乘的那个颜色，一并烘进去，画的时候不要再乘。
    pub fn get_additive(
        &mut self,
        asset_mgr: &AssetManager,
        name: &str,
        tint: [f32; 4],
    ) -> Option<Rc<Sprite>> {
        let key = format!("{}#add", name.to_ascii_lowercase().replace('\\', "/"));
        if let Some(hit) = self.cache.get(&key) {
            return hit.clone();
        }
        let sprite = self.image(asset_mgr, name).map(|mut img| {
            bake_additive(&mut img, tint);
            asset_mgr.sprite_from_image(img)
        });
        if sprite.is_none() {
            log::warn!("UiTextures: 图集和包里都没有 {name}");
        }
        self.cache.insert(key, sprite.clone());
        sprite
    }

    /// 取这张贴图的像素：先查图集，查不到再读散文件。
    fn image(&mut self, asset_mgr: &AssetManager, name: &str) -> Option<image::RgbaImage> {
        self.from_atlas(asset_mgr, name).or_else(|| {
            asset_mgr.read_picture(PathBuf::from(BASEDATA).join(name.replace('\\', "/")))
        })
    }

    fn from_atlas(&mut self, asset_mgr: &AssetManager, name: &str) -> Option<image::RgbaImage> {
        let e = self.dict.get(name)?;
        let (lib, x, y, w, h) = (e.lib.clone(), e.orix, e.oriy, e.w, e.h);
        let page = self
            .pages
            .entry(lib.to_ascii_lowercase())
            .or_insert_with(|| load_page(asset_mgr, &lib))
            .as_ref()?;
        if w == 0 || h == 0 || x + w > page.width() || y + h > page.height() {
            log::warn!("UiTextures: {name} 的子矩形越出图集页 {lib}");
            return None;
        }
        Some(image::imageops::crop_imm(page, x, y, w, h).to_image())
    }
}

/// 把一张原版**加法混合**画上去的贴图，折成能用普通 alpha 混合近似画的样子。
///
/// 原版画 UI 四边形的那几个渲染效果建在 `0x53fec0`，列表选中条走的是
/// `DAT_01895058` = `gfxscript\ui_tex_color.gbf`（`0x43eb66` →
/// `FUN_00540c60` → `RenderUIQuad(…, DAT_01895058, 贴图)`，全程只有这一个
/// 调用点）。那张 gbf 里写的是：
///
/// ```text
/// SrcBlend = SRCALPHA;  DestBlend = ONE;
/// ColorOp[0] = MODULATE; ColorArg1 = TEXTURE; ColorArg2 = DIFFUSE;
/// AlphaOp[0] = MODULATE; AlphaArg1 = TEXTURE; AlphaArg2 = DIFFUSE;
/// ```
///
/// 也就是 `dst += 贴图.rgb·色.rgb · 贴图.a·色.a`——**只加不盖**，所以条压在
/// 行文字上面也不会把字遮住，只是把那一行提亮。相比之下同一套里的
/// `ui_TL.gbf`（`DAT_0189504c`）才是 `SRCALPHA/INVSRCALPHA` 的普通混合。
///
/// 移植端的 imgui 后端整帧只有一种混合状态（普通 alpha），没法逐条命令换，
/// 于是把该加上去的那一份 `c = 贴图.rgb·色.rgb·贴图.a·色.a` 拆成
/// `A = max(c)`、`RGB = c / A`，再按 alpha 混合画：
///
/// ```text
/// dst·(1 − A) + (c/A)·A = dst + c − dst·A
/// ```
///
/// 只差 `dst·A` 这一项。列表那两张条按 (0xa8,0xa8,0xa8,0xc8) 烘出来，A 最大
/// 分别是 45/255（`ListBoxSelPic`，256×32 的绿色横向光带，上下两头收到全黑）
/// 和 99/255（`SelPic`，128×16 的橙色光带，自带 alpha）。底下又都是暗色，
/// 差得有限。
pub fn bake_additive(img: &mut image::RgbaImage, tint: [f32; 4]) {
    for p in img.pixels_mut() {
        let a = p[3] as f32 / 255. * tint[3];
        let c = [0, 1, 2].map(|i| p[i] as f32 / 255. * tint[i] * a);
        let m = c[0].max(c[1]).max(c[2]);
        if m <= 0. {
            *p = image::Rgba([0, 0, 0, 0]);
            continue;
        }
        *p = image::Rgba([
            (c[0] / m * 255.).round() as u8,
            (c[1] / m * 255.).round() as u8,
            (c[2] / m * 255.).round() as u8,
            (m * 255.).round() as u8,
        ]);
    }
}

/// 读一张图集页。清单里写的是 `N.tga`，零售版多数只带 `N.dds`：先找 `.dds`，
/// 找到了要**竖直翻转**——D3D9 的 `.dds` 是自下而上存的，TLI 坐标按正立的页算
/// （p7 那边 `SpriteService::upload` 同样翻转）。`.tga` 由解码器按文件头的原点
/// 摆正，不用管。
fn load_page(asset_mgr: &AssetManager, lib: &str) -> Option<image::RgbaImage> {
    let path = PathBuf::from(UILIB_DIR).join(lib);
    let dds = path.with_extension("dds");
    if let Ok(bytes) = asset_mgr.vfs().read_to_end(&dds) {
        match image::load_from_memory_with_format(&bytes, image::ImageFormat::Dds) {
            Ok(img) => return Some(image::imageops::flip_vertical(&img.to_rgba8())),
            Err(e) => log::warn!("UiTextures: 解不开 {}: {e}", dds.display()),
        }
    }
    asset_mgr.read_picture(&path)
}

#[cfg(test)]
mod tests {
    use super::bake_additive;

    /// 烘完的像素按普通 alpha 混合画上去，和原版加法混合的差应当正好是
    /// `dst·A`——底色全黑时分毫不差。
    #[test]
    fn baking_reproduces_additive_blending_over_black() {
        let tint = [
            0xa8 as f32 / 255.,
            0xa8 as f32 / 255.,
            0xa8 as f32 / 255.,
            0xc8 as f32 / 255.,
        ];
        let src = image::Rgba([38u8, 88, 75, 255]);
        let mut img = image::RgbaImage::from_pixel(1, 1, src);
        bake_additive(&mut img, tint);
        let baked = img.get_pixel(0, 0).0;
        let a = baked[3] as f32 / 255.;
        for i in 0..3 {
            let want = src[i] as f32 / 255. * tint[i] * (src[3] as f32 / 255. * tint[3]);
            let got = baked[i] as f32 / 255. * a; // dst = 0
            assert!(
                (want - got).abs() < 1. / 255.,
                "通道 {i}: 想要 {want}，得到 {got}"
            );
        }
        // 这张贴图最亮的一点，加上去的那一份不到 1/5。
        assert_eq!(baked[3], 45);
    }

    /// 全透明的像素烘完还是全透明，不会因为除以 0 冒出脏色。
    #[test]
    fn fully_transparent_pixels_stay_transparent() {
        let mut img = image::RgbaImage::from_pixel(1, 1, image::Rgba([200, 200, 200, 0]));
        bake_additive(&mut img, [1., 1., 1., 1.]);
        assert_eq!(img.get_pixel(0, 0).0, [0, 0, 0, 0]);
    }
}
