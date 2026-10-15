use common::store_ext::StoreExt2;
use crosscom::ComRc;
use encoding::{DecoderTrap, types::Encoding};
use ini::Ini;
use mini_fs::MiniFs;
use mini_fs::prelude::*;
use radiance::comdef::{IAnimatedMeshComponent, IEntity, IScene};
use radiance::math::Vec3;
use radiance::rendering::{ComponentFactory, RenderingEngine, Sprite};
use radiance::scene::{CoreScene, ISceneExt, SceneLight, SceneLighting};
use radiance::utils::SeekRead;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::{io, rc::Rc};

use fileformats::pal3::flexeff::FlexEff;

use crate::GameType;

use super::comdef::IScnSceneComponent;
use super::loaders::nav_loader::NavFile;
use super::loaders::nav_loader::nav_load_from_file;
use super::loaders::pol::create_entity_from_pol_model;
use super::loaders::sce_loader::SceFile;
use super::loaders::sce_loader::sce_load_from_file;
use super::loaders::scn_loader::scn_load_from_file;
use super::scene::ScnScene;
use super::scene::create_animated_mesh_from_mv3;
use super::scene::create_entity_from_cvd_model;
use super::scene::create_mv3_entity;

/// Which decoder a picture's extension implies. TGA has no magic
/// number, so `image` cannot sniff it and the format has to be named.
fn format_of(path: &Path) -> image::ImageFormat {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("dds") => image::ImageFormat::Dds,
        Some("jpg") | Some("jpeg") => image::ImageFormat::Jpeg,
        Some("bmp") => image::ImageFormat::Bmp,
        Some("png") => image::ImageFormat::Png,
        _ => image::ImageFormat::Tga,
    }
}

pub struct AssetManager {
    factory: Rc<dyn ComponentFactory>,
    game: GameType,
    scene_path: PathBuf,
    music_path: PathBuf,
    movie_path: PathBuf,
    movie_end_path: PathBuf,
    movie_effect_path: PathBuf,
    snd_path: PathBuf,
    basedata_path: PathBuf,
    vfs: Rc<MiniFs>,
    /// `cbdata\flexeff.data` 的两张表，第一次要用时才解。整个包 9.3 MB，
    /// 解出来也是 9.3 MB，所以只解一次。`None` 表示试过但读不到。
    flexeff: std::cell::RefCell<Option<Option<Rc<FlexEff>>>>,
    /// 渲染引擎，界面里要画离屏小场景（买装备页的物品预览）时用。宿主建好
    /// 资源管理器后塞进来；测试和编辑器里可能没有。
    renderer: std::cell::RefCell<Option<Rc<std::cell::RefCell<dyn RenderingEngine>>>>,
}

impl AssetManager {
    pub fn new(factory: Rc<dyn ComponentFactory>, vfs: Rc<MiniFs>) -> Self {
        Self::new_for_game(factory, vfs, GameType::PAL3)
    }

    /// PAL3 and PAL3A share this loader; the variant selects per-game
    /// scene layout (PAL3A keeps every `.scn`/`.nav` in `scn.cpk` under
    /// `/scene/scn/Scn/<cpk>/`, PAL3 next to each scene cpk).
    pub fn new_for_game(
        factory: Rc<dyn ComponentFactory>,
        vfs: Rc<MiniFs>,
        game: GameType,
    ) -> Self {
        Self {
            factory,
            game,
            basedata_path: PathBuf::from("/basedata/basedata"),
            flexeff: std::cell::RefCell::new(None),
            renderer: std::cell::RefCell::new(None),
            scene_path: PathBuf::from("/scene"),
            music_path: PathBuf::from("/music/music/music"),
            movie_path: PathBuf::from("/movie/movie/movie"),
            movie_end_path: PathBuf::from("/movie/movie_end/movie"),
            movie_effect_path: PathBuf::from("/movie/movie/2deffect"),
            snd_path: PathBuf::from("/snd"),
            vfs,
        }
    }

    pub fn set_rendering_engine(&self, engine: Rc<std::cell::RefCell<dyn RenderingEngine>>) {
        *self.renderer.borrow_mut() = Some(engine);
    }

    /// 界面在 `update` 里调用是安全的：引擎只在渲染阶段借用自己，
    /// 这时候没有被借着（`CoreRadianceEngine::update`）。
    pub fn rendering_engine(&self) -> Option<Rc<std::cell::RefCell<dyn RenderingEngine>>> {
        self.renderer.borrow().clone()
    }

    pub fn vfs(&self) -> &MiniFs {
        &self.vfs
    }

    pub fn vfs_rc(&self) -> Rc<MiniFs> {
        self.vfs.clone()
    }

    pub fn component_factory(&self) -> Rc<dyn ComponentFactory> {
        self.factory.clone()
    }

    pub fn load_scn(self: &Rc<Self>, cpk_name: &str, scn_name: &str) -> ComRc<IScene> {
        let scene_path = self.scn_path(cpk_name, scn_name);
        log::info!(
            "PAL3: loading scene {}/{} ({})",
            cpk_name,
            scn_name,
            scene_path.display()
        );

        let scn_file = scn_load_from_file(&self.vfs, scene_path, self.game);
        let nav_file = self.load_nav(&scn_file.cpk_name, &scn_file.scn_base_name);

        let scene = CoreScene::create();
        scene.set_lighting(self.load_lgt(cpk_name, &scn_file.scn_base_name, scn_file.is_night));
        let component = ScnScene::new(scene.clone(), &self, cpk_name, scn_name, scn_file, nav_file);
        scene.add_component(IScnSceneComponent::uuid(), ComRc::from_object(component));
        log::info!("PAL3: scene {}/{} loaded", cpk_name, scn_name);
        scene
    }

    /// Load the scene's dynamic light table into a [`SceneLighting`] consumed
    /// by the dynamically-lit actor shader. Night scenes ship a separate, warmer
    /// (candlelit) light set under the `1/` subfolder — the same day/night split
    /// the `.pol`/`.cvd` loaders use — so for night scenes that variant is
    /// preferred, falling back to the base `.lgt`. Missing or unreadable files
    /// fall back to an ambient-only environment so actors stay visible.
    fn load_lgt(&self, cpk_name: &str, scn_base_name: &str, is_night: bool) -> SceneLighting {
        // 环境光照原版来：`gbGfxManager` 构造里写死 (0.2, 0.2, 0.2)
        // （gbengine.dll `0x1001d9b9`，字段 `+0x1ac`），而 `SetAmbientLight`
        // 在 exe 的导入表里根本没有，所以全程不变——白天夜里都一样。
        // 着色器里的环境项 = 这个值 × 材质 Ambient + 自发光系数 × 材质 Emissive，
        // PAL3 的材质 Ambient 都是 (1,1,1)、自发光系数全程 0，所以直接等于它。
        let _ = is_night;
        let ambient = crate::openpal3::combat::stage::AMBIENT;

        let base_folder = self.scene_path.join(cpk_name).join(scn_base_name);
        let mut candidate_paths = vec![];
        if is_night {
            // Night variant lives alongside the night `.pol`/`.cvd` in `1/`.
            candidate_paths.push(
                base_folder
                    .join("1")
                    .join(scn_base_name)
                    .with_extension("lgt"),
            );
        }
        candidate_paths.push(base_folder.join(scn_base_name).with_extension("lgt"));

        let (path, lights) = candidate_paths
            .iter()
            .find_map(|path| {
                let mut reader = self.vfs.open(path).ok()?;
                let mut buf = Vec::new();
                if io::Read::read_to_end(&mut reader, &mut buf).is_err() {
                    return None;
                }
                match fileformats::pal3::lgt::read_lgt(&mut io::Cursor::new(buf)) {
                    Ok(lgt) => Some((
                        path.clone(),
                        lgt.lights
                            .iter()
                            .map(|l| {
                                let p = l.position();
                                SceneLight::with_range(
                                    Vec3::new(p[0], p[1], p[2]),
                                    l.color,
                                    l.range,
                                )
                            })
                            .collect::<Vec<_>>(),
                    )),
                    Err(e) => {
                        log::warn!("Failed to parse {}: {}", path.display(), e);
                        None
                    }
                }
            })
            .unwrap_or_else(|| (base_folder.clone(), vec![]));

        let lighting = SceneLighting::new(ambient, lights);
        log::info!(
            "PAL3 .lgt loaded from {}: {} lights, ambient {:?}",
            path.display(),
            lighting.lights.len(),
            lighting.ambient
        );
        lighting
    }
    pub fn load_sce(&self, cpk_name: &str) -> SceFile {
        // PAL3 keeps each scene's `.sce` in its scene cpk
        // (`/scene/<cpk>/<cpk>.sce`); PAL3A collects them in `sce.cpk`
        // as `/scene/sce/Sce/<cpk>.sce`.
        let sce_path = match self.game {
            GameType::PAL3A => self
                .scene_path
                .join("sce")
                .join("Sce")
                .join(cpk_name)
                .with_extension("sce"),
            _ => self
                .scene_path
                .join(cpk_name)
                .join(cpk_name)
                .with_extension("sce"),
        };
        sce_load_from_file(&self.vfs, sce_path)
    }

    /// `cbdata\flexeff.data` —— 战斗特效的定义表和特效组表，143/148 用。
    /// 解析和那个「只读前 0x2c047 字节再 LZO 解压」的坑写在
    /// `fileformats::pal3::flexeff` 和 `switch/re/notes/flexeff_format.md`。
    pub fn flexeff(&self) -> Option<Rc<FlexEff>> {
        let mut slot = self.flexeff.borrow_mut();
        if slot.is_none() {
            let path = self.basedata_path.join("cbdata/flexeff.data");
            *slot = Some(
                common::store_ext::StoreExt2::read_to_end(self.vfs.as_ref(), &path)
                    .ok()
                    .and_then(|raw| match FlexEff::decode(&raw) {
                        Ok(eff) => Some(Rc::new(eff)),
                        Err(e) => {
                            log::warn!("flexeff.data 解不开: {e}");
                            None
                        }
                    }),
            );
        }
        slot.as_ref().and_then(|o| o.clone())
    }

    pub fn load_init_sce(&self) -> SceFile {
        let init_sce = self.basedata_path.join("init.sce");
        sce_load_from_file(&self.vfs, init_sce)
    }

    /// PAL3 stores each scene's `.scn` next to its cpk
    /// (`/scene/<cpk>/<scn>.scn`); PAL3A collects every `.scn` in
    /// `scn.cpk` as `/scene/scn/Scn/<cpk>/<cpk>_<scn>.scn`.
    pub fn scn_path(&self, cpk_name: &str, scn_name: &str) -> PathBuf {
        match self.game {
            GameType::PAL3A => self
                .scene_path
                .join("scn")
                .join("Scn")
                .join(cpk_name)
                .join(format!("{}_{}", cpk_name, scn_name))
                .with_extension("scn"),
            _ => self
                .scene_path
                .join(cpk_name)
                .join(scn_name)
                .with_extension("scn"),
        }
    }

    pub fn load_nav(&self, cpk_name: &str, scn_name: &str) -> NavFile {
        let nav_path = match self.game {
            GameType::PAL3A => self
                .scene_path
                .join("scn")
                .join("Scn")
                .join(cpk_name)
                .join(scn_name)
                .join(scn_name)
                .with_extension("nav"),
            _ => self
                .scene_path
                .join(cpk_name)
                .join(scn_name)
                .join(scn_name)
                .with_extension("nav"),
        };
        nav_load_from_file(&self.vfs, nav_path)
    }

    pub fn load_role(
        self: &Rc<Self>,
        role_name: &str,
        default_action: &str,
        name: String,
        visible: bool,
    ) -> Option<ComRc<IEntity>> {
        create_mv3_entity(self.clone(), role_name, default_action, name, visible).ok()
    }

    /// Build the fixed circle blob shadow child entity (`shadow.tga`) attached
    /// under each role. Returns `None` if the texture is unavailable.
    pub fn build_role_shadow(&self) -> Option<ComRc<IEntity>> {
        super::scene::build_role_shadow(&self.factory, &self.vfs, &self.basedata_path)
    }

    pub fn load_role_anim_config(&self, role_name: &str) -> Ini {
        let path = self
            .basedata_path
            .join("ROLE")
            .join(role_name)
            .join(role_name)
            .with_extension("ini");

        let mv3_ini = encoding::all::GBK
            .decode(&self.vfs.read_to_end(&path).unwrap(), DecoderTrap::Ignore)
            .unwrap();
        Ini::load_from_str(&mv3_ini).unwrap()
    }

    pub fn load_role_anim_first<'a>(
        &self,
        entity: ComRc<IEntity>,
        role_name: &str,
        action_names: &[&'a str],
    ) -> Option<(&'a str, ComRc<IAnimatedMeshComponent>)> {
        for action_name in action_names {
            let anim = self.load_role_anim(entity.clone(), role_name, action_name);
            if anim.is_some() {
                return Some((action_name, anim.unwrap()));
            }
        }

        None
    }

    pub fn load_role_anim(
        &self,
        entity: ComRc<IEntity>,
        role_name: &str,
        action_name: &str,
    ) -> Option<ComRc<IAnimatedMeshComponent>> {
        let dir = self.basedata_path.join("ROLE").join(role_name);
        let path = dir.join(action_name).with_extension("mv3");
        if let Ok(anim) = create_animated_mesh_from_mv3(
            entity.clone(),
            &self.component_factory(),
            &self.vfs,
            &path,
        ) {
            return Some(anim);
        }
        // 动作文件不按动作名起名的角色（搜宝鼠 `DXD` 的 z1、z2 都是 `DXD.MV3`）：查角色目录下
        // `<角色>.ini` 的 `[action_N] name = … / file = …`。
        let file = self.role_action_file(role_name, action_name)?;
        create_animated_mesh_from_mv3(
            entity,
            &self.component_factory(),
            &self.vfs,
            &dir.join(file),
        )
        .ok()
    }

    /// `<角色>.ini` 里动作 `action_name` 对应的文件名；没有这个文件或这一项是 `None`。
    fn role_action_file(&self, role_name: &str, action_name: &str) -> Option<String> {
        let path = self
            .basedata_path
            .join("ROLE")
            .join(role_name)
            .join(role_name)
            .with_extension("ini");
        let bytes = self.vfs.read_to_end(&path).ok()?;
        let text = encoding::all::GBK
            .decode(&bytes, DecoderTrap::Ignore)
            .ok()?;
        let ini = Ini::load_from_str(&text).ok()?;
        ini.iter().find_map(|(_, props)| {
            let name = props.get("name")?.trim();
            name.eq_ignore_ascii_case(action_name)
                .then(|| props.get("file").map(|f| f.trim().to_string()))
                .flatten()
        })
    }

    /// As [`Self::load_role_anim`], with opcode 116's texture override.
    pub fn load_role_anim_with_texture(
        &self,
        entity: ComRc<IEntity>,
        role_name: &str,
        action_name: &str,
        texture: Option<&str>,
        unique: bool,
    ) -> Option<ComRc<IAnimatedMeshComponent>> {
        let path = self
            .basedata_path
            .join("ROLE")
            .join(role_name)
            .join(action_name)
            .with_extension("mv3");

        crate::openpal3::scene::create_animated_mesh_from_mv3_with_texture(
            entity,
            &self.component_factory(),
            &self.vfs,
            &path,
            texture,
            unique,
        )
        .ok()
    }

    pub fn mv3_path(&self, role_name: &str, action_name: &str) -> PathBuf {
        self.basedata_path
            .join("ROLE")
            .join(role_name)
            .join(action_name)
            .with_extension("mv3")
    }

    /// `.mv3` 里的命名标记（`Mv3ActionDesc`：帧号 + 名字，帧号 / 4580 是秒）。原版把它们当动画消息推给
    /// 角色节点，`FUN_00537070(名字)` 取一条；跳跃等的是 `holds`（起跳完）和 `hold`（空中定格）。
    pub fn mv3_events(&self, role_name: &str, action_name: &str) -> Vec<(f32, String)> {
        let path = self.mv3_path(role_name, action_name);
        let Ok(data) = self.vfs.read_to_end(&path) else {
            return vec![];
        };
        let Ok(mv3) = fileformats::mv3::read_mv3(&mut std::io::Cursor::new(data)) else {
            return vec![];
        };
        mv3.action_desc
            .iter()
            .filter_map(|a| {
                let name = a.name.as_str().ok()?.to_string();
                (!name.is_empty()).then_some((a.tick as f32 / 4580., name))
            })
            .collect()
    }

    pub fn load_scn_pol(
        &self,
        cpk_name: &str,
        scn_name: &str,
        pol_name: &str,
        is_night: bool,
        index: u16,
    ) -> Option<ComRc<IEntity>> {
        let folder = self.scene_path.join(cpk_name).join(scn_name);
        let mut paths = vec![];
        if is_night {
            paths.push(folder.join("1").join(pol_name).with_extension("pol"));
        }

        paths.push(folder.join(pol_name).with_extension("pol"));

        for path in &paths {
            if self.vfs.open(path).is_ok() {
                return Some(create_entity_from_pol_model(
                    &self.factory,
                    &self.vfs,
                    path,
                    format!("OBJECT_{}", index),
                    true,
                ));
            }
        }

        None
    }

    pub fn load_scn_cvd(
        &self,
        cpk_name: &str,
        scn_name: &str,
        cvd_name: &str,
        is_night: bool,
        index: u16,
    ) -> Option<ComRc<IEntity>> {
        let folder = self.scene_path.join(cpk_name).join(scn_name);
        let mut paths = vec![];
        if is_night {
            paths.push(folder.join("1").join(cvd_name).with_extension("cvd"));
        }

        paths.push(folder.join(cvd_name).with_extension("cvd"));

        for path in &paths {
            if self.vfs.open(path).is_ok() {
                return Some(create_entity_from_cvd_model(
                    self.factory.clone(),
                    &self.vfs,
                    path,
                    format!("OBJECT_{}", index),
                    true,
                ));
            }
        }

        None
    }

    pub fn load_object_item_pol(
        &self,
        obj_name: &str,
        index: u16,
        visible: bool,
    ) -> Option<ComRc<IEntity>> {
        let path = self.get_object_item_path(obj_name);
        if self.vfs.open(&path).is_ok() {
            Some(create_entity_from_pol_model(
                &self.factory,
                &self.vfs,
                &path,
                format!("OBJECT_{}", index),
                visible,
            ))
        } else {
            None
        }
    }

    /// `basedata\effect` 底下的一个 `.pol` 模型（存盘点的石头 `effect\stone.pol`），实体叫 `name`。
    pub fn load_effect_pol(&self, file: &str, name: String) -> Option<ComRc<IEntity>> {
        let path = self.effect_root().join(file);
        if self.vfs.open(&path).is_err() {
            log::warn!("读不到 {}", path.display());
            return None;
        }
        Some(create_entity_from_pol_model(
            &self.factory,
            &self.vfs,
            &path,
            name,
            true,
        ))
    }

    pub fn load_object_item_cvd(
        &self,
        obj_name: &str,
        index: u16,
        visible: bool,
    ) -> Option<ComRc<IEntity>> {
        let path = self.get_object_item_path(obj_name);
        if self.vfs.open(&path).is_ok() {
            Some(create_entity_from_cvd_model(
                self.factory.clone(),
                &self.vfs,
                &path,
                format!("OBJECT_{}", index),
                visible,
            ))
        } else {
            None
        }
    }

    /// The `/basedata/basedata` root, for callers that build their own
    /// asset paths under it.
    pub fn basedata_path(&self) -> &PathBuf {
        &self.basedata_path
    }

    /// Path to a full-screen caption picture — opcode 86's
    /// `caption\%s.tga` (format string at 0x596234).
    pub fn caption_path(&self, name: &str) -> PathBuf {
        self.basedata_path
            .join("caption")
            .join(format!("{name}.tga"))
    }

    /// Path to the PAL3 scene-effect (`EffectScn`) asset root.
    fn effect_scn_root(&self) -> PathBuf {
        self.basedata_path.join("EffectScn")
    }

    /// `basedata\\effect`, where the weather textures live.
    pub fn effect_root(&self) -> PathBuf {
        self.basedata_path.join("effect")
    }

    /// `basedata\\effect\\gbParticle`, where the `.par` particle effects
    /// and their `partexture\\` textures live.
    pub fn particle_root(&self) -> PathBuf {
        self.effect_root().join("gbParticle")
    }

    /// A file under `EffectScn`, e.g. the 141 screen effect's
    /// `EffectScn\Frame.Tga`.
    pub fn effect_scn_path(&self, name: &str) -> PathBuf {
        self.effect_scn_root().join(name)
    }

    /// Load a 2D picture the way the rest of the port reads textures:
    /// the retail Chinese release ships almost everything as `.dds` even
    /// where the code (and the exe's own strings) names a `.tga`, so try
    /// that first and fall back to the name as written.
    pub fn read_picture<P: AsRef<Path>>(&self, path: P) -> Option<image::RgbaImage> {
        let path = path.as_ref();
        let candidates = [
            (path.with_extension("dds"), image::ImageFormat::Dds),
            (path.to_path_buf(), format_of(path)),
        ];
        for (candidate, format) in candidates {
            let Ok(data) = self.vfs.read_to_end(&candidate) else {
                continue;
            };
            match image::load_from_memory_with_format(&data, format) {
                Ok(image) => return Some(image.to_rgba8()),
                Err(e) => log::warn!("解不开图片 {}: {e}", candidate.display()),
            }
        }
        None
    }

    pub fn load_sprite<P: AsRef<Path>>(&self, path: P) -> Option<Rc<Sprite>> {
        let image = self.read_picture(path)?;
        Some(Rc::new(Sprite::load_from_image(
            image,
            self.factory.as_ref(),
        )))
    }

    /// 给 2D 界面读图：和 [`Self::load_sprite`] 一样先找 `.dds`，但找到的 `.dds`
    /// 要**竖直翻转**——零售版的 `.dds` 是 D3D9 自下而上存的（图集页、
    /// `EffectScn\104s.dds` 都是这样，见 `switch/re/notes/dealer_layout.md` §0）。
    /// `read_picture` 还给粒子、灵效这些 3D 贴图用，那边的朝向另有约定，不在这里改。
    pub fn load_ui_sprite<P: AsRef<Path>>(&self, path: P) -> Option<Rc<Sprite>> {
        let path = path.as_ref();
        let dds = path.with_extension("dds");
        let image = match self.vfs.read_to_end(&dds) {
            Ok(bytes) => match image::load_from_memory_with_format(&bytes, image::ImageFormat::Dds)
            {
                Ok(img) => image::imageops::flip_vertical(&img.to_rgba8()),
                Err(e) => {
                    log::warn!("解不开图片 {}: {e}", dds.display());
                    return None;
                }
            },
            Err(_) => self.read_picture(path)?,
        };
        Some(self.sprite_from_image(image))
    }

    /// 已经解好的图直接做成 `Sprite`——从图集裁出来的小图走这里
    /// （[`super::ui_tex::UiTextures`]）。
    pub fn sprite_from_image(&self, image: image::RgbaImage) -> Rc<Sprite> {
        Rc::new(Sprite::load_from_image(image, self.factory.as_ref()))
    }

    /// Build a PAL3 scene effect (candle / lamp / torch / fire) for the
    /// given effect id (`ScnNode::dw184[3]`). Returns the holder entity
    /// (flame attached) or the bare flame, or `None` for unknown ids.
    pub fn load_scene_effect(&self, effect_id: u32, index: u16) -> Option<ComRc<IEntity>> {
        crate::openpal3::scene::build_effect(
            &self.factory,
            &self.vfs,
            &self.effect_scn_root(),
            effect_id,
            index,
        )
    }

    pub fn load_music_data(&self, music_name: &str) -> Vec<u8> {
        let path = self.music_path.join(music_name).with_extension("mp3");
        self.vfs.read_to_end(path).unwrap()
    }

    pub fn load_movie_data(&self, movie_name: &str) -> Box<dyn SeekRead> {
        let movie = self.movie_path.join(movie_name).with_extension("bik");
        let end_movie = self.movie_end_path.join(movie_name).with_extension("bik");
        let effect_movie = self
            .movie_effect_path
            .join(movie_name)
            .with_extension("bik");
        let file = self.vfs.open(movie).unwrap_or_else(|_| {
            self.vfs
                .open(end_movie)
                .unwrap_or_else(|_| self.vfs.open(effect_movie).unwrap())
        });

        Box::new(BufReader::new(file))
    }

    pub fn load_snd_data(&self, snd_name: &str) -> io::Result<Vec<u8>> {
        let path = self.snd_path.join(snd_name).with_extension("wav");
        self.vfs.read_to_end(path)
    }

    /// 场景物件模型文件在哪（找法和 [`crate::openpal3::scene::ScnScene`] 载入物件时一样：`_` 开头的在场景目录，
    /// 夜里先找 `1\`，先 `.pol` 后 `.cvd`；带扩展名的在 `object\`；其余是 `item\名\名.pol`）。
    pub fn scn_object_model_path(
        &self,
        cpk_name: &str,
        scn_name: &str,
        obj_name: &str,
        is_night: bool,
    ) -> Option<PathBuf> {
        if obj_name.starts_with('_') {
            let folder = self.scene_path.join(cpk_name).join(scn_name);
            for ext in ["pol", "cvd"] {
                let mut paths = vec![];
                if is_night {
                    paths.push(folder.join("1").join(obj_name).with_extension(ext));
                }
                paths.push(folder.join(obj_name).with_extension(ext));
                if let Some(p) = paths.into_iter().find(|p| self.vfs.open(p).is_ok()) {
                    return Some(p);
                }
            }
            return None;
        }
        let path = self.get_object_item_path(obj_name);
        self.vfs.open(&path).is_ok().then_some(path)
    }

    fn get_object_item_path(&self, obj_name: &str) -> PathBuf {
        if obj_name.contains('.') {
            self.basedata_path.join("object").join(&obj_name)
        } else {
            self.basedata_path
                .join("item")
                .join(&obj_name)
                .join(&obj_name)
                .with_extension("pol")
        }
    }
}
