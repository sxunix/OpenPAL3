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
        let sprite = self.from_atlas(asset_mgr, name).or_else(|| {
            asset_mgr.load_sprite(PathBuf::from(BASEDATA).join(name.replace('\\', "/")))
        });
        if sprite.is_none() {
            log::warn!("UiTextures: 图集和包里都没有 {name}");
        }
        self.cache.insert(key, sprite.clone());
        sprite
    }

    fn from_atlas(&mut self, asset_mgr: &AssetManager, name: &str) -> Option<Rc<Sprite>> {
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
        let sub = image::imageops::crop_imm(page, x, y, w, h).to_image();
        Some(asset_mgr.sprite_from_image(sub))
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
