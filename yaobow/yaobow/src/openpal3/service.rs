//! PAL3 launch service — replaces the placeholder imgui main-menu
//! with a script-built sprite-atlas start menu.
//!
//! `IPal3Service` interface lives in `shared::openpal3::comdef` (from
//! `crosscom/idl/openpal3.idl`) so it can be exposed symmetrically via
//! `IYaobowHostContext.pal3()`. The conforming `class Pal3Service` is
//! declared in `yaobow_services.idl` so the `ComObject_Pal3Service!`
//! macro generates here in `yaobow_lib` — keeping
//! `OpenPal3DebugLayer` + `sce_proc_hooks` (the surrounding
//! director-construction glue) co-located with the service.
//!
//! The PAL3 start menu itself is script-built: `create_director` QIs
//! `IPal3ScriptFactory` off the host script app and calls
//! `make_pal3_start_menu(asset_path)`. The script menu's button
//! handlers call back into this service to mint the adventure
//! director (`create_adventure_director` / `load_adventure_director`)
//! or to exit the app (`exit_app`).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use agent_server::{AgentCommand, AgentError, AgentResponse};
use crosscom::ComRc;
use fileformats::pal3::gdb::GameItemInfo;
use packfs::init_virtual_fs;
use radiance::audio::Codec as AudioCodec;
use radiance::comdef::{
    IApplication, IApplicationExt, IDirector, ISceneManager, IUiHost, IUiLayer,
};
use radiance::input::{InputEngine, SyntheticInputBridge};
use radiance::radiance::{UiLayerBand, UiLayerHandle};
use radiance::video::Codec as VideoCodec;
use radiance_scripting::comdef::services::IAudioSource;
use radiance_scripting::comdef::services::ISpriteService;
use radiance_scripting::comdef::services::IVideoHandle;
use radiance_scripting::services::AudioSource;
use radiance_scripting::services::ImguiTextureCache;
use radiance_scripting::services::SpriteService;
use shared::GameType;
use shared::agent_common::AgentBridge;
use shared::loaders::video_handle::VideoHandle;
use shared::openpal3::agent::{Pal3DispatchCtx, dispatch_pal3_command};
use shared::openpal3::asset_manager::AssetManager;
use shared::openpal3::comdef::{
    IAdventureDirector, IPal3DialogRenderer, IPal3ScriptFactory, IPal3Service, IPal3ServiceImpl,
    IPal3StatusRenderer, IPal3UiAtlas,
};
use shared::openpal3::directors::{AdventureDirector, SceneManagerExtensions};
use shared::openpal3::scene::ScnScene;
use shared::openpal3::states::game_db::GameDb;
use shared::openpal3::states::item_use::{
    self, EARTH_PEARL_PROC, TargetPick, UseContext, UseOutcome,
};
use shared::openpal3::states::persistent_state::{Feature, PAL3_APP_NAME, PersistentState};
use shared::openpal3::ui_atlas::{AtlasManifest, Pal3UiAtlas};
use shared::scripting::sce::vm::SceExecutionOptions;
use shared::ydirs;

use crate::openpal3::debug_layer::OpenPal3DebugLayer;
use crate::openpal3::sce_proc_hooks::SceRestHooks;

/// Number of save slots surfaced by the save/load overlays. The
/// original PC release supports 36 saves (9 pages of 4 scroll rows in
/// its System page); slot files are `1.json`..`36.json`.
const SAVE_SLOT_COUNT: i32 = 36;

/// `IPal3Service::role_stat` selectors, mirrored by the status scripts.
const STAT_LEVEL: i32 = 0;
const STAT_EXP: i32 = 1;
const STAT_EXP_TO_NEXT: i32 = 2;
const STAT_HP: i32 = 3;
const STAT_MAX_HP: i32 = 4;
const STAT_SP: i32 = 5;
const STAT_MAX_SP: i32 = 6;
const STAT_MP: i32 = 7;
const STAT_MAX_MP: i32 = 8;
/// 水火风雷土 occupy 9..=13, in the game database's element order.
const STAT_WATER: i32 = 9;
const STAT_EARTH: i32 = 13;
const STAT_ATTACK: i32 = 14;
const STAT_DEFENSE: i32 = 15;
const STAT_SPEED: i32 = 16;
const STAT_LUCK: i32 = 17;

/// `IPal3Service::skill_info` selectors.
const SKILL_FIELD_TYPE: i32 = 0;
const SKILL_FIELD_LEVEL: i32 = 1;
const SKILL_FIELD_SP_COST: i32 = 2;
const SKILL_FIELD_MP_COST: i32 = 3;
const SKILL_FIELD_SUCCESS_RATE: i32 = 4;
const SKILL_FIELD_ELEMENT_MASK: i32 = 5;
const SKILL_FIELD_REQUIRED_LEVEL: i32 = 6;
const SKILL_FIELD_OUTSIDE_COMBAT: i32 = 7;
const SKILL_FIELD_NEXT_LEVEL_ID: i32 = 8;
const SKILL_FIELD_TARGET_RANGE: i32 = 9;
const SKILL_FIELD_MAGIC_LEVEL: i32 = 10;
const SKILL_FIELD_USE_TIMES: i32 = 11;

/// `IPal3Service::item_info` selectors.
const ITEM_FIELD_TYPE: i32 = 0;
const ITEM_FIELD_WEAPON_TYPE: i32 = 1;
const ITEM_FIELD_PRICE: i32 = 2;
const ITEM_FIELD_ANCIENT_VALUE: i32 = 3;
const ITEM_FIELD_PLACE_OF_USE: i32 = 4;
const ITEM_FIELD_TARGET_RANGE: i32 = 5;
const ITEM_FIELD_SPECIAL_TYPE: i32 = 6;
const ITEM_FIELD_ORE_ID: i32 = 7;
const ITEM_FIELD_PRODUCT_ID: i32 = 8;
const ITEM_FIELD_PRODUCT_PRICE: i32 = 9;
const ITEM_FIELD_SYNTH_MATERIAL_1: i32 = 10;
const ITEM_FIELD_SYNTH_MATERIAL_2: i32 = 11;
const ITEM_FIELD_SYNTH_PRODUCT: i32 = 12;
const ITEM_FIELD_COMBO_COUNT: i32 = 13;

/// `IPal3Service::combat_actor_info` selectors.
const ACTOR_FIELD_TYPE: i32 = 0;
const ACTOR_FIELD_LEVEL: i32 = 1;
/// 精气神武防速运 occupy 2..=8, in the database's attribute order.
const ACTOR_FIELD_HP: i32 = 2;
const ACTOR_FIELD_LUCK: i32 = 8;
/// 五灵 水火风雷土 occupy 9..=13.
const ACTOR_FIELD_WATER: i32 = 9;
const ACTOR_FIELD_EARTH: i32 = 13;
const ACTOR_FIELD_EXP: i32 = 14;
const ACTOR_FIELD_MONEY_WHEN_KILLED: i32 = 15;
const ACTOR_FIELD_LOOT_ID: i32 = 16;
const ACTOR_FIELD_LOOT_COUNT: i32 = 17;
const ACTOR_FIELD_STEALABLE_ITEM: i32 = 18;
const ACTOR_FIELD_STEALABLE_MONEY: i32 = 19;
const ACTOR_FIELD_CORPSE_ID: i32 = 20;

/// Lay out `text` the way an original text widget does: a fixed cell of
/// `cell` logical pixels, ASCII advancing half a cell and CJK a whole
/// one, wrapping at `wrap_px`. `\n` breaks a line; so does the `$` the
/// item descriptions use to separate flavour text from the stat line.
fn wrap_widget_text(text: &str, cell: f32, wrap_px: f32) -> Vec<String> {
    let half = cell / 2.;
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut width = 0.;

    for ch in text.chars() {
        if ch == '\n' || ch == '$' {
            lines.push(std::mem::take(&mut line));
            width = 0.;
            continue;
        }
        let advance = if ch.is_ascii() { half } else { cell };
        if wrap_px > 0. && width + advance > wrap_px && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
            width = 0.;
        }
        line.push(ch);
        width += advance;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Map a script/registry game ordinal to the PAL3-family `GameType`.
/// PAL3 and PAL3A share `Pal3Service`; anything else defaults to PAL3.
fn game_from_ordinal(ordinal: i32) -> GameType {
    match radiance_scripting::services::game_registry::ordinal_to_config_key(ordinal) {
        Some("pal3a") => GameType::PAL3A,
        _ => GameType::PAL3,
    }
}

pub struct Pal3Service {
    app: ComRc<IApplication>,
    /// App-lifetime `AssetManager` cache, keyed by asset path. The
    /// start menu and the adventure director both go through the same
    /// instance so a `New Game` after browsing the menu doesn't
    /// re-mount the vfs or re-decode assets.
    asset_managers: RefCell<HashMap<String, Rc<AssetManager>>>,
    /// Imgui texture cache shared by the generic sprite/atlas loaders
    /// (`create_sprite_service` / `create_ui_atlas`) and the dialog
    /// renderer to upload + frame-gate UI textures. Installed once at
    /// boot by `YaobowApplicationLoader::on_loading`; the loaders return
    /// `None` when this slot is empty.
    texture_cache: RefCell<Option<Rc<RefCell<ImguiTextureCache>>>>,
    /// 纹理缓存的两条队列（外部纹理登记 / 释放），`draw_model_view` 在界面帧里用——
    /// 那时缓存本身被借着，只能往队列里推。
    texture_sinks: RefCell<
        Option<(
            Rc<RefCell<Vec<(i64, imgui::TextureId)>>>,
            Rc<RefCell<Vec<i64>>>,
        )>,
    >,
    /// `draw_model_view` 的视图，按脚本给的 key 存。
    model_views: RefCell<HashMap<i32, ModelViewSlot>>,
    lineup_disc: RefCell<Option<LineupDiscSlot>>,
    /// Script-side factory (`IPal3ScriptFactory`) QI'd from the
    /// reverse-wrapped app root. Used by `create_director` to mint
    /// the scripted start menu. Cleared on app unload to break the
    /// service ↔ script-app reference cycle.
    script_factory: RefCell<Option<ComRc<IPal3ScriptFactory>>>,
    /// Set once on `create_director` so `request_exit` doesn't need
    /// to re-look-up the install path. Not strictly required; useful
    /// for diagnostics.
    last_asset_path: RefCell<Option<String>>,
    /// Game variant for the active launch, set on `create_director`.
    /// PAL3 and PAL3A share this service; this selects per-game assets
    /// such as the start-menu BGM track. Defaults to PAL3.
    last_game: Cell<GameType>,
    /// True when `create_director` has been called at least once and
    /// the debug layer install is pending. Drained by `pump_pre_update`
    /// before the next engine tick (i.e. outside any `engine.borrow()`
    /// scope) so we can register the debug-overlay UI layer on the
    /// `UiManager` without aliasing the title director's update path.
    pending_debug_install: Cell<bool>,
    debug_layer_installed: Cell<bool>,
    /// RAII handle for the registered PAL3 debug overlay UI layer. Held
    /// for the service lifetime so the layer stays registered; dropping
    /// it unregisters the overlay.
    debug_layer_handle: RefCell<Option<UiLayerHandle>>,
    /// Deferred system-menu actions. The menu paints from inside the
    /// AdventureDirector's update (its props RefCell mutably borrowed), so
    /// saving/loading there re-borrows and aborts; the requests queue here
    /// and `pump_pre_update` performs them outside the director borrow.
    pending_save_slot: Cell<i32>,
    pending_load_slot: Cell<i32>,
    /// 按场景算的「能存」（`DAT_005cefa9`：进场景时 = 不是迷宫，m02 再清成 0）。
    can_save_here: Cell<bool>,
    /// 存盘点打开的存档页还开着（这时迷宫里也能存）。
    at_save_point: Cell<bool>,
    /// 存盘点要打开存档页，等状态界面取走。
    save_point_request: Cell<bool>,
    /// The active adventure's save state. Held as its own handle (not
    /// reached through the director) so the status screens can read party
    /// stats while painting from inside the director's update.
    active_persistent_state: RefCell<Option<Rc<RefCell<PersistentState>>>>,
    /// The active adventure's static game data (experience curve, item
    /// and skill tables). Captured alongside the save state.
    active_game_db: RefCell<Option<Rc<GameDb>>>,
    /// Lazily-parsed `datascript/SceneName.txt` — the original's own
    /// scene-code → display-name table (per-cpk sections with a
    /// `default` row). Keyed by (section, code), both lowercase.
    /// Backs the save-slot labels in the system menu.
    scene_name_table: RefCell<Option<HashMap<(String, String), String>>>,

    /// Once-per-process latch for the LOGO.bik intro. Set to true on
    /// the first successful `play_intro_movie` call; subsequent calls
    /// return `None` so re-entry into the start menu (e.g. exit-to-menu
    /// from a future Adventure flow) skips the intro.
    intro_played: Cell<bool>,
    /// The scene-side facts 道具页 needs to judge a use, refreshed once
    /// a frame by [`Self::pump_pre_update`]. The menu paints from inside
    /// the director's update and can't reach the scene from there.
    item_use_ctx: Cell<UseContext>,
    /// 右上角头像：受控主角、现在中的毒（每帧在 `refresh_item_use_context` 里抄）。
    hud_role: Cell<i32>,
    hud_poison: Cell<i32>,
    /// The SCE procedure a just-used 引路蜂 / 土灵珠 asks for. Queued
    /// rather than run on the spot for the same reason saves are: the
    /// menu is painting inside the director's own update.
    pending_item_proc: RefCell<Option<PendingItemProc>>,
    /// 御剑页点了「御剑飞行」：菜单关掉以后进大地图（原版是主菜单切到页 8，页 8 每帧
    /// `FUN_0044f610(0xd)`）。和道具一样排着，等菜单画完再做。
    pending_bigmap: Cell<bool>,
    /// 菜单里放的界面音效（放完前要留着）。
    ui_sounds: RefCell<Vec<Box<dyn radiance::audio::AudioMemorySource>>>,
    /// `AssisLang.lang` 的 `sec1`，第一次用到时读。
    assis_lang: RefCell<Option<Vec<String>>>,
    /// Agent-server bridge. `None` when no `--agent-port` flag was
    /// passed; `Some(_)` enables [`Self::pump_agent`] and makes
    /// every fresh `AdventureDirector` honor pause/step + see
    /// synthetic input.
    agent_bridge: RefCell<Option<Rc<AgentBridge>>>,
}

/// What a used 仙宝 asks the SCE VM to run next.
enum PendingItemProc {
    /// 引路蜂 —— `_ylf_<城市>_<场景>`.
    ByName(String),
    /// 土灵珠 —— proc 2101.
    Id(u32),
    /// 搜宝鼠 —— 不是过程，交给导演（`Scene::UseSeekingMouse`）。
    TreasureMouse,
}

ComObject_Pal3Service!(super::Pal3Service);

impl Pal3Service {
    pub fn create(app: ComRc<IApplication>) -> ComRc<IPal3Service> {
        ComRc::from_object(Self {
            app,
            asset_managers: RefCell::new(HashMap::new()),
            texture_cache: RefCell::new(None),
            texture_sinks: RefCell::new(None),
            model_views: RefCell::new(HashMap::new()),
            lineup_disc: RefCell::new(None),
            script_factory: RefCell::new(None),
            last_asset_path: RefCell::new(None),
            last_game: Cell::new(GameType::PAL3),
            pending_debug_install: Cell::new(false),
            pending_save_slot: Cell::new(-1),
            pending_load_slot: Cell::new(-1),
            can_save_here: Cell::new(true),
            at_save_point: Cell::new(false),
            save_point_request: Cell::new(false),
            active_persistent_state: RefCell::new(None),
            active_game_db: RefCell::new(None),
            scene_name_table: RefCell::new(None),
            debug_layer_installed: Cell::new(false),
            debug_layer_handle: RefCell::new(None),
            intro_played: Cell::new(false),
            item_use_ctx: Cell::new(UseContext::default()),
            hud_role: Cell::new(0),
            hud_poison: Cell::new(-1),
            pending_item_proc: RefCell::new(None),
            pending_bigmap: Cell::new(false),
            ui_sounds: RefCell::new(Vec::new()),
            assis_lang: RefCell::new(None),
            agent_bridge: RefCell::new(None),
        })
    }

    /// Install the imgui texture cache so the sprite/atlas loaders and
    /// dialog renderer can upload UI textures. Called once at boot.
    pub fn set_texture_cache(&self, cache: Rc<RefCell<ImguiTextureCache>>) {
        *self.texture_sinks.borrow_mut() = Some({
            let c = cache.borrow();
            (c.pending_external_updates_sink(), c.pending_forgets_sink())
        });
        *self.texture_cache.borrow_mut() = Some(cache);
    }

    /// Install the script app's PAL3 factory.
    pub fn set_script_factory(&self, factory: ComRc<IPal3ScriptFactory>) {
        *self.script_factory.borrow_mut() = Some(factory);
    }

    /// Drop the held script factory, breaking the
    /// service ↔ host-context ↔ script-CCW reference cycle at teardown.
    pub fn clear_script_factory(&self) {
        *self.script_factory.borrow_mut() = None;
    }

    /// Build the scripted-dialog-box dependencies threaded into a fresh
    /// adventure director: a generic, game-vfs-bound `ISpriteService`
    /// (over this asset path's PAL3 vfs + the shared texture cache) and
    /// the self-contained p7 `IPal3DialogRenderer` minted by the script
    /// factory from it. The renderer owns its sprites/avatar/curtain
    /// state in p7; the host only forwards SCE events + draw calls. Both
    /// the texture cache and the script factory must already be
    /// installed (they are, by the time the player leaves the start menu).
    fn build_dialog_renderer(&self, asset_mgr: Rc<AssetManager>) -> ComRc<IPal3DialogRenderer> {
        let texture_cache = self.texture_cache.borrow().clone().expect(
            "Pal3Service::build_dialog_renderer called before the texture cache was installed. \
             YaobowApplicationLoader must call Pal3Service::set_texture_cache at boot.",
        );
        let factory = self.script_factory.borrow().clone().expect(
            "Pal3Service::build_dialog_renderer called before the script factory was installed \
             (or after it was cleared). YaobowApplicationLoader must call \
             Pal3Service::set_script_factory after installing the script root.",
        );

        // The dialog renderer's sprites still upload through the engine
        // texture cache (consumed above); the per-frame dialog draws now
        // resolve through the same cache via the engine-owned resolver on
        // `UiManager` (see `SceState::render_dialog`), so the renderer is
        // threaded straight through to the SCE VM (no wrapper struct).
        let sprites = SpriteService::create(asset_mgr.vfs_rc(), texture_cache);
        factory.make_pal3_dialog_renderer(sprites)
    }

    fn build_status_renderer(&self, asset_mgr: Rc<AssetManager>) -> ComRc<IPal3StatusRenderer> {
        let texture_cache = self.texture_cache.borrow().clone().expect(
            "Pal3Service::build_status_renderer called before the texture cache was installed. \
             YaobowApplicationLoader must call Pal3Service::set_texture_cache at boot.",
        );
        let factory = self.script_factory.borrow().clone().expect(
            "Pal3Service::build_status_renderer called before the script factory was installed \
             (or after it was cleared). YaobowApplicationLoader must call \
             Pal3Service::set_script_factory after installing the script root.",
        );
        let sprites = SpriteService::create(asset_mgr.vfs_rc(), texture_cache);
        let atlas = self
            .build_ui_atlas(asset_mgr)
            .expect("Pal3Service::build_status_renderer: failed to build UI atlas");
        factory.make_pal3_status_renderer(sprites, atlas)
    }

    /// Build the PAL3 `UI_opt.tli` atlas adapter for a given adventure
    /// asset_mgr (shared with `create_ui_atlas`, but threaded directly
    /// into the status renderer so the in-game menu screen can resolve
    /// `ui/GameMainUI/*` sprites). Requires the texture cache.
    fn build_ui_atlas(&self, asset_mgr: Rc<AssetManager>) -> Option<ComRc<IPal3UiAtlas>> {
        let cache = self.texture_cache.borrow().clone()?;
        let sprites = SpriteService::create(asset_mgr.vfs_rc(), cache);
        let kind = match self.last_game.get() {
            GameType::PAL3A => AtlasManifest::Plug,
            _ => AtlasManifest::Tli,
        };
        let manifest = common::store_ext::StoreExt2::read_to_end(
            asset_mgr.vfs(),
            Pal3UiAtlas::manifest_path(kind),
        )
        .ok()?;
        Some(Pal3UiAtlas::create(sprites, &manifest, kind))
    }

    /// Returns the cached `AssetManager` for `asset_path`, mounting
    /// the vfs + building one on first use.
    /// Resolve a save's (scene cpk, sub-scene) codes to the original's
    /// Chinese display name via `datascript/SceneName.txt` (GBK,
    /// per-cpk `BEGIN,<CPK>` sections of `code\t$name&` rows with a
    /// `default` fallback — the same table the original save screen
    /// draws its slot names from). Parsed once and cached; `None` when
    /// no launch happened yet, the table is missing, or has no row.
    fn scene_display_name(&self, scene: &str, sub: &str) -> Option<String> {
        if self.scene_name_table.borrow().is_none() {
            let asset_path = self.last_asset_path.borrow().clone()?;
            let vfs = self.asset_manager_for(&asset_path).vfs_rc();
            let text = common::store_ext::StoreExt2::read_to_end_from_gbk(
                vfs.as_ref(),
                "/basedata/basedata/datascript/SceneName.txt",
            )
            .ok()?;
            let mut map: HashMap<(String, String), String> = HashMap::new();
            let mut section = String::new();
            for line in text.lines() {
                let line = line.split(';').next().unwrap_or("").trim();
                if line.is_empty() {
                    continue;
                }
                if let Some(rest) = line.strip_prefix("BEGIN,") {
                    section = rest.trim().to_lowercase();
                    continue;
                }
                let (Some(dollar), Some(amp)) = (line.find('$'), line.rfind('&')) else {
                    continue;
                };
                if dollar >= amp {
                    continue;
                }
                let code = line[..dollar].trim().to_lowercase();
                let name = line[dollar + 1..amp].trim().to_string();
                if !section.is_empty() && !code.is_empty() && !name.is_empty() {
                    map.insert((section.clone(), code), name);
                }
            }
            *self.scene_name_table.borrow_mut() = Some(map);
        }

        let table = self.scene_name_table.borrow();
        let map = table.as_ref()?;
        let scene = scene.to_lowercase();
        map.get(&(scene.clone(), sub.to_lowercase()))
            .or_else(|| map.get(&(scene, "default".to_string())))
            .cloned()
    }

    /// 阵法页换位 / 挪位的音效（`shared::openpal3::lineup_disc::SWAP_SOUND`），放一遍。
    fn play_lineup_sound(
        &self,
        asset_path: &str,
        sounds: &mut Vec<Box<dyn radiance::audio::AudioMemorySource>>,
    ) {
        let asset_mgr = self.asset_manager_for(asset_path);
        let data = match asset_mgr.load_snd_data(shared::openpal3::lineup_disc::SWAP_SOUND) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("阵法页音效读不出来: {e:?}");
                return;
            }
        };
        let engine_rc = self.app.engine();
        let audio_engine = engine_rc.borrow().audio_engine();
        drop(engine_rc);
        let mut source = audio_engine.create_source();
        source.set_data(data, AudioCodec::Wav);
        source.play(false);
        sounds.push(source);
    }

    /// 菜单里的界面音效（`FUN_0053df70(名字, 1)`），放一遍。
    fn play_ui_sound(&self, name: &str) {
        let Some(asset_path) = self.last_asset_path.borrow().clone() else {
            return;
        };
        let asset_mgr = self.asset_manager_for(&asset_path);
        let data = match asset_mgr.load_snd_data(name) {
            Ok(d) => d,
            Err(e) => {
                log::warn!("界面音效 {name} 读不出来: {e:?}");
                return;
            }
        };
        let engine_rc = self.app.engine();
        let audio_engine = engine_rc.borrow().audio_engine();
        drop(engine_rc);
        let mut source = audio_engine.create_source();
        source.set_data(data, AudioCodec::Wav);
        source.play(false);
        let mut sounds = self.ui_sounds.borrow_mut();
        // 只留最近几个，放完的早就该放掉了。
        if sounds.len() > 8 {
            sounds.remove(0);
        }
        sounds.push(source);
    }

    /// `AssisLang.lang` 第 `index` 条（`sec1`，按出现顺序编号）。
    fn assis_lang_text(&self, index: usize) -> Option<String> {
        if self.assis_lang.borrow().is_none() {
            let asset_path = self.last_asset_path.borrow().clone()?;
            let asset_mgr = self.asset_manager_for(&asset_path);
            use common::store_ext::StoreExt2;
            use encoding::Encoding;
            let bytes = asset_mgr
                .vfs()
                .read_to_end(PathBuf::from("/basedata/basedata/AssisLang.lang"))
                .ok()?;
            let text = encoding::all::GBK
                .decode(&bytes, encoding::DecoderTrap::Ignore)
                .ok()?;
            *self.assis_lang.borrow_mut() =
                Some(shared::openpal3::assis_lang::parse_section(&text, "sec1"));
        }
        self.assis_lang
            .borrow()
            .as_ref()
            .and_then(|t| t.get(index).cloned())
    }

    fn asset_manager_for(&self, asset_path: &str) -> Rc<AssetManager> {
        if let Some(am) = self.asset_managers.borrow().get(asset_path) {
            return am.clone();
        }

        let engine_rc = self.app.engine();
        let engine = engine_rc.borrow();
        let component_factory = engine.rendering_component_factory();
        let rendering_engine = engine.rendering_engine();
        drop(engine);

        let vfs = init_virtual_fs(&PathBuf::from(asset_path), None);
        let am = Rc::new(AssetManager::new_for_game(
            component_factory,
            Rc::new(vfs),
            self.last_game.get(),
        ));
        am.set_rendering_engine(rendering_engine);
        self.asset_managers
            .borrow_mut()
            .insert(asset_path.to_string(), am.clone());
        am
    }

    fn install_debug_layer_once(&self) {
        // Called by `pump_pre_update` *outside* any outstanding engine
        // borrow. Calling `borrow_mut()` from inside `create_director`
        // would alias the title director's update path
        // (`engine.borrow().update() -> scene_manager.update() ->
        // title_director.update() -> host.pal3().create_director`)
        // and panic at runtime. The pending-flag indirection makes the
        // install deterministic AND safe.
        if self.debug_layer_installed.get() {
            return;
        }
        let engine_rc = self.app.engine();
        let engine = engine_rc.borrow();
        let input_engine = engine.input_engine();
        let scene_manager = engine.scene_manager().clone();
        let ui = engine.ui_manager();
        let layer: ComRc<IUiLayer> = ComRc::from_object(OpenPal3DebugLayer::new(
            input_engine,
            scene_manager,
            ui.clone(),
        ));
        // Register in the DebugOverlay band so the overlay draws on top
        // of game UI. Keep the handle alive for the service lifetime;
        // dropping it would unregister the layer.
        let handle = ui.register_ui_layer(UiLayerBand::DebugOverlay, layer);
        *self.debug_layer_handle.borrow_mut() = Some(handle);
        drop(engine);
        self.debug_layer_installed.set(true);
    }

    /// App-lifetime pre-update hook driven by
    /// `YaobowApplicationLoader::on_updating` (which runs *before*
    /// `engine.borrow().update()` each frame). Installs the PAL3 debug
    /// layer when a previous `create_director` queued it. No-op
    /// otherwise.
    pub fn pump_pre_update(&self) {
        if self.pending_debug_install.get() {
            self.pending_debug_install.set(false);
            self.install_debug_layer_once();
        }

        let save_slot = self.pending_save_slot.replace(-1);
        if save_slot > 0 {
            if let Some(adv) = self.active_adventure_director_owned() {
                adv.inner::<AdventureDirector>()
                    .sce_vm()
                    .global_state()
                    .persistent_state()
                    .save(save_slot);
            }
        }

        self.refresh_item_use_context();

        if self.pending_bigmap.replace(false) {
            if let Some(adv) = self.active_adventure_director_owned() {
                adv.inner::<AdventureDirector>().start_debug_bigmap();
            }
        }

        let pending_proc = self.pending_item_proc.borrow_mut().take();
        if let Some(proc) = pending_proc {
            if let Some(adv) = self.active_adventure_director_owned() {
                let director = adv.inner::<AdventureDirector>();
                let mut vm = director.sce_vm_mut();
                match proc {
                    PendingItemProc::ByName(name) => vm.state_mut().try_call_proc_by_name(&name),
                    PendingItemProc::Id(id) => vm.call_proc(id),
                    PendingItemProc::TreasureMouse => {
                        drop(vm);
                        director.use_treasure_mouse();
                    }
                }
            }
        }

        let load_slot = self.pending_load_slot.replace(-1);
        if load_slot > 0 {
            let asset_path = self.last_asset_path.borrow().clone();
            if let Some(asset_path) = asset_path {
                if let Some(director) = self.load_adventure_director(&asset_path, load_slot) {
                    let sm = self.app.engine().borrow().scene_manager().clone();
                    sm.set_director(director);
                    log::info!("system menu: resumed from slot {load_slot}");
                }
            }
        }
    }

    /// Install the agent bridge so the next `create_adventure_director`
    /// / `load_adventure_director` plumbs synthetic input + pause/step
    /// gating into the new director. Called once at boot by
    /// `YaobowApplicationLoader` when `--pal3 --agent-port` is in
    /// effect.
    pub fn set_agent_bridge(&self, bridge: Rc<AgentBridge>) {
        // Forward to any director already installed so a bridge
        // attached mid-session takes effect immediately.
        if let Some(adv) = self.active_adventure_director_owned() {
            adv.inner::<AdventureDirector>()
                .set_agent_bridge(bridge.clone());
        }
        *self.agent_bridge.borrow_mut() = Some(bridge);
    }

    /// Retire the previous frame's synthetic-input edges, drain the
    /// agent-server command queue, dispatch each command against PAL3
    /// state, then publish frame telemetry. Called once per frame by
    /// `YaobowApplicationLoader::on_updating` (before the engine
    /// tick), so commands land before the active director runs.
    pub fn pump_agent(&self, delta_sec: f32) {
        let bridge = match self.agent_bridge.borrow().as_ref() {
            Some(b) => b.clone(),
            None => return,
        };

        // Retire the *previous* frame's synthetic-input edges before
        // injecting this frame's. `AdventureDirector::update` runs
        // during `engine.update`, i.e. *after* this hook, so an edge
        // injected below must survive until the next pump to be
        // observable at all. Clearing at the end of this function
        // instead would wipe every tap before the director polls it.
        bridge.input_bridge.borrow().end_frame();

        // Lazily wire the rendering engine so `/v1/screenshot` works
        // before any director has set it on the bridge.
        if bridge.rendering_engine.borrow().is_none() {
            let engine = self.app.engine().borrow().rendering_engine();
            bridge.set_rendering_engine(engine);
        }

        // Drain the queue once (single drainer for this game).
        let mut envelopes = Vec::new();
        if let Some(consumer) = bridge.consumer.borrow().as_ref() {
            consumer.drain(|env| envelopes.push(env));
        }

        let scene_manager = self.app.engine().borrow().scene_manager().clone();

        for env in envelopes {
            // Mode-control commands install the next director on the
            // scene manager; routed here because the dispatcher (not
            // any director) owns the mode graph.
            if Self::is_mode_control_command(&env.command) {
                let response = self.dispatch_mode_control(&scene_manager, env.command.clone());
                env.reply(response);
                continue;
            }

            let active = Self::active_adventure_director(&scene_manager);
            let director_ref = active.as_ref().map(|c| c.inner::<AdventureDirector>());
            let ctx = Pal3DispatchCtx {
                bridge: &bridge,
                director: director_ref.as_deref(),
                scene_manager: scene_manager.clone(),
            };
            let response = dispatch_pal3_command(&ctx, env.command.clone());
            env.reply(response);
        }

        // Telemetry: always advance frame counter + publish dt/fps.
        bridge.publish_frame_telemetry(delta_sec);
    }

    fn is_mode_control_command(command: &AgentCommand) -> bool {
        matches!(
            command,
            AgentCommand::EnterNewGame
                | AgentCommand::EnterLoadGame(_)
                | AgentCommand::LoadSlot(_)
                | AgentCommand::ExitGame
        )
    }

    fn dispatch_mode_control(
        &self,
        scene_manager: &ComRc<ISceneManager>,
        command: AgentCommand,
    ) -> AgentResponse {
        if let AgentCommand::ExitGame = command {
            self.app.request_exit();
            return AgentResponse::Ok;
        }

        let asset_path = match self.last_asset_path.borrow().clone() {
            Some(p) => p,
            None => {
                return AgentResponse::err(AgentError::internal(
                    "PAL3 launch asset path not set yet (no playthrough has been mounted)",
                ));
            }
        };

        let director = match command {
            AgentCommand::EnterNewGame => self.create_adventure_director(&asset_path),
            AgentCommand::EnterLoadGame(params) | AgentCommand::LoadSlot(params) => {
                // PAL3 has no in-place state reload (PAL4-style); both
                // EnterLoadGame and LoadSlot rebuild the adventure
                // director from the slot file, mirroring what the menu
                // does when the player picks a save.
                match self.load_adventure_director(&asset_path, params.slot) {
                    Some(d) => d,
                    None => {
                        return AgentResponse::err(AgentError::conflict(format!(
                            "save slot {} is missing or could not be loaded",
                            params.slot
                        )));
                    }
                }
            }
            _ => {
                return AgentResponse::err(AgentError::internal(
                    "dispatch_mode_control called with a non-mode-control command",
                ));
            }
        };
        scene_manager.set_director(director);
        AgentResponse::Ok
    }

    /// Whether the running script asked to go back to the title screen
    /// (147 Switch2Menu, or a lost battle that ends the game). Polled by
    /// `YaobowApplicationLoader::on_updating`, which owns the script
    /// factory that can build the title director.
    pub fn take_title_request(&self) -> bool {
        self.active_adventure_director_owned()
            .map(|adv| adv.inner::<AdventureDirector>().take_title_request())
            .unwrap_or(false)
    }

    fn active_adventure_director(
        scene_manager: &ComRc<ISceneManager>,
    ) -> Option<ComRc<IAdventureDirector>> {
        scene_manager
            .director()
            .and_then(|d| d.query_interface::<IAdventureDirector>())
    }

    fn active_adventure_director_owned(&self) -> Option<ComRc<IAdventureDirector>> {
        let sm = self.app.engine().borrow().scene_manager().clone();
        Self::active_adventure_director(&sm)
    }

    /// Re-read the four scene-side facts 道具页 judges a use against
    /// (`states::item_use::UseContext`). Runs before the engine tick, so
    /// the director and the scene are both free to borrow here; the menu
    /// itself only ever reads the cached copy.
    fn refresh_item_use_context(&self) {
        let sm = self.app.engine().borrow().scene_manager().clone();
        let Some(adv) = Self::active_adventure_director(&sm) else {
            self.item_use_ctx.set(UseContext::default());
            return;
        };

        let mut ctx = UseContext::default();
        if let Some(scene) = sm.scn_scene() {
            let scene = scene.inner::<ScnScene>();
            // 进场景时（`FUN_00423xxx`，0x424039 起）：`DAT_005cefa9 = 场景类型 != 2`，当前城是 `m02`
            // （`DAT_0058de68`）再清成 0。
            self.can_save_here
                .set(scene.scene_type() != 2 && !scene.name().eq_ignore_ascii_case("m02"));
            // `DAT_00d8bbec == 2`——场景类型 2 是迷宫。
            ctx.in_maze = scene.scene_type() == 2;
            ctx.city_is_m22 = scene.name().eq_ignore_ascii_case("M22");
            ctx.city_is_q14 = scene.name().eq_ignore_ascii_case("Q14");
            ctx.treasure_present = scene.treasure_object_present();
        }
        if adv
            .inner::<AdventureDirector>()
            .sce_vm_mut()
            .state_mut()
            .take_save_menu_request()
        {
            self.at_save_point.set(true);
            self.save_point_request.set(true);
        }
        ctx.proc_2101_exists = adv
            .inner::<AdventureDirector>()
            .sce_vm()
            .state()
            .has_proc(EARTH_PEARL_PROC);
        {
            let director = adv.inner::<AdventureDirector>();
            let vm = director.sce_vm();
            let gs = vm.global_state();
            self.hud_role.set(gs.role_controlled().clamp(0, 4));
            let p = gs.poison_kind();
            self.hud_poison
                .set(if (0..=4).contains(&p) { p } else { -1 });
        }
        self.item_use_ctx.set(ctx);
    }

    /// Remember the new adventure's save state so the status screens can
    /// read it without borrowing the director. Called while `adv` is
    /// still a plain object, before it is wrapped as a `ComRc`.
    fn track_persistent_state(&self, adv: &AdventureDirector) {
        let (state, db) = {
            let vm = adv.sce_vm();
            let global = vm.global_state();
            (global.persistent_state_rc(), global.game_db().cloned())
        };
        self.active_persistent_state.replace(Some(state));
        self.active_game_db.replace(db);
    }

    /// Item stacks matching `type_mask` (bit N = `ItemType` N; -1 for
    /// everything), ascending by item id. Items the database doesn't know
    /// are dropped rather than shown as blanks.
    fn matching_stacks(&self, type_mask: i32) -> Vec<u32> {
        let db = self.active_game_db.borrow();
        let Some(db) = db.as_ref() else {
            return Vec::new();
        };
        self.with_persistent_state(Vec::new(), |s| {
            s.inventory()
                .keys()
                .copied()
                .filter(|id| {
                    db.item(*id)
                        .map(|item| type_mask & (1 << item.item_type as i32) != 0)
                        .unwrap_or(false)
                })
                .collect()
        })
    }

    /// Pull one text field off an item's database row.
    fn item_text(
        &self,
        item_id: i32,
        f: impl FnOnce(&fileformats::pal3::gdb::GameItemInfo) -> Option<String>,
    ) -> Option<String> {
        if item_id < 0 {
            return None;
        }
        let db = self.active_game_db.borrow();
        f(db.as_ref()?.item(item_id as u32)?)
    }

    /// Pull one text field off a skill's database row.
    fn skill_text(
        &self,
        skill_id: i32,
        f: impl FnOnce(&fileformats::pal3::gdb::SkillInfo) -> Option<String>,
    ) -> Option<String> {
        if skill_id < 0 {
            return None;
        }
        let db = self.active_game_db.borrow();
        f(db.as_ref()?.skill(skill_id as u32)?)
    }

    /// Pull one text field off a combat actor's database row.
    /// `draw_model_view` 的模型串：`.pol` / `.cvd` 是 basedata 下的路径，`actor:<id>`
    /// 是战斗角色的模型（播待机，怪物那套的 50 号），`item:<id>` 是物品模型
    /// （`Item\<模型>\<模型>.pol`，贴图换成 `<模型><图标>.tga`，和买卖页一样），其余当角色名。
    fn model_source(&self, model: &str) -> Option<shared::openpal3::model_view::ModelSource> {
        use shared::openpal3::combat::motion::{Motion, MotionSet};
        use shared::openpal3::model_view::ModelSource;
        let idle = MotionSet::Monster
            .name(Motion::Idle)
            .unwrap_or("z1")
            .to_string();
        if let Some(id) = model.strip_prefix("actor:") {
            let id: i32 = id.trim().parse().ok()?;
            let name = self.actor_text(id, |a| a.model_id.as_str().ok())?;
            let name = name.trim().to_string();
            if name.is_empty() || name.eq_ignore_ascii_case("null") {
                return None;
            }
            return Some(ModelSource::Role { name, action: idle });
        }
        if let Some(id) = model.strip_prefix("item:") {
            let id: u32 = id.trim().parse().ok()?;
            let db = self.active_game_db.borrow();
            let row = db.as_ref()?.item(id)?;
            let name = row
                .model_name
                .as_str()
                .unwrap_or_default()
                .trim()
                .to_string();
            // 原版拿模型名和一个全局串比，相等就写 "NONE"、什么也不载 [推断：空名]。
            if name.is_empty() || name.eq_ignore_ascii_case("NONE") {
                return None;
            }
            let icon = row.icon_name.as_str().unwrap_or_default();
            return Some(ModelSource::Pol {
                path: PathBuf::from(format!("/basedata/basedata/item/{name}/{name}.pol")),
                texture: Some(format!("{name}{icon}.tga")),
            });
        }
        let lower = model.to_ascii_lowercase();
        let path = || PathBuf::from(format!("/basedata/basedata/{}", model.replace('\\', "/")));
        if lower.ends_with(".cvd") {
            Some(ModelSource::Cvd(path()))
        } else if lower.ends_with(".pol") {
            Some(ModelSource::Pol {
                path: path(),
                texture: None,
            })
        } else {
            Some(ModelSource::Role {
                name: model.to_string(),
                action: idle,
            })
        }
    }

    fn actor_text(
        &self,
        actor_id: i32,
        f: impl FnOnce(&fileformats::pal3::gdb::CombatActorInfo) -> Option<String>,
    ) -> Option<String> {
        if actor_id < 0 {
            return None;
        }
        let db = self.active_game_db.borrow();
        f(db.as_ref()?.combat_actor(actor_id as u32)?)
    }

    /// Run `f` against the active adventure's save state, or return
    /// `default` when no adventure is running. Uses `try_borrow` because
    /// this is called mid-paint and must never abort the frame.
    fn with_persistent_state<T>(&self, default: T, f: impl FnOnce(&PersistentState) -> T) -> T {
        let handle = self.active_persistent_state.borrow();
        let Some(state) = handle.as_ref() else {
            return default;
        };
        match state.try_borrow() {
            Ok(state) => f(&state),
            Err(_) => default,
        }
    }

    /// Run `f` against an item row, the save state and the database at
    /// once — what every read-only 用道具 query needs. `default` when no
    /// adventure is running, the item is unknown, or the state is
    /// already borrowed (this is called mid-paint).
    fn with_item_and_state<T>(
        &self,
        item_id: i32,
        default: T,
        f: impl FnOnce(&GameItemInfo, &PersistentState, &GameDb) -> T,
    ) -> T {
        let db = self.active_game_db.borrow().clone();
        let Some(db) = db else { return default };
        let Some(item) = db.item(item_id as u32) else {
            return default;
        };
        let handle = self.active_persistent_state.borrow();
        let Some(state) = handle.as_ref() else {
            return default;
        };
        match state.try_borrow() {
            Ok(state) => f(item, &state, &db),
            Err(_) => default,
        }
    }

    /// Resolve the `Rc<RefCell<dyn InputEngine>>` we should hand to
    /// the next `AdventureDirector`: the agent's synthetic-input
    /// bridge when present (so `/v1/input/*` reaches the director),
    /// otherwise the real engine input.
    fn input_engine_for_director(&self) -> Rc<RefCell<dyn InputEngine>> {
        if let Some(bridge) = self.agent_bridge.borrow().as_ref() {
            // Rc<RefCell<SyntheticInputBridge>> coerces to
            // Rc<RefCell<dyn InputEngine>> at the binding site.
            let synth: Rc<RefCell<SyntheticInputBridge>> = bridge.input_bridge.clone();
            return synth;
        }
        self.app.engine().borrow().input_engine()
    }

    fn sce_options() -> SceExecutionOptions {
        SceExecutionOptions {
            proc_hooks: vec![Box::new(SceRestHooks::new())],
        }
    }
}

impl IPal3ServiceImpl for Pal3Service {
    fn create_director(&self, asset_path: &str, game_ordinal: i32) -> ComRc<IDirector> {
        // The title script silently routes to the settings page when the
        // configured asset path is empty, so a launch that "does nothing" and
        // a launch that got this far looked identical in the log.
        log::info!(
            "PAL3: launching game {} from asset path {:?}",
            game_ordinal,
            asset_path
        );
        *self.last_asset_path.borrow_mut() = Some(asset_path.to_string());
        let game = game_from_ordinal(game_ordinal);
        self.last_game.set(game);

        // Switch in-game text to the game-shipped font (simsun). No-op if
        // the file is missing; the editor/title selector keep the bundled
        // font. Registered once here; the engine rebuilds the atlas next
        // frame.
        if let Some(bytes) = shared::load_game_font(shared::GameType::PAL3, asset_path) {
            self.app
                .engine()
                .borrow()
                .ui_manager()
                .add_game_font(&bytes, shared::GameType::PAL3.ui_font_scale());
        } else {
            // PAL3 shipped no font file (it used the Windows system font), so
            // a bare install registers the bundled CJK face through the same
            // path -- the dialog metrics are tuned against the game-font
            // sizes, and without this the text fell back to the small base
            // face and rendered cramped. A user-supplied simsun.ttc in the
            // asset directory still wins via the branch above.
            //
            // Switch only: there the glyph ranges are already narrowed, so
            // the two extra faces fit the atlas. On desktop the base faces
            // alone fill a 4096x16384 atlas (full CJK, 3x oversampling) --
            // adding game faces pushes past every GPU's texture limit and
            // MoltenVK aborts. Desktop keeps the upstream behavior.
            #[cfg(target_os = "horizon")]
            {
                log::info!("PAL3: no game font in the install; using the bundled CJK face");
                self.app.engine().borrow().ui_manager().add_game_font(
                    radiance::imgui::bundled_cjk_font(),
                    shared::GameType::PAL3.ui_font_scale(),
                );
            }
        }

        // Warm the AssetManager up front so the menu + adventure
        // director see a consistent VFS. The debug layer install needs
        // an exclusive engine borrow, which is not safe to take from
        // inside the title director's update path that called us; we
        // queue it for the next `pump_pre_update`.
        let _ = self.asset_manager_for(asset_path);
        self.pending_debug_install.set(true);

        let factory = self.script_factory.borrow().clone().expect(
            "Pal3Service::create_director called before the script factory was installed \
             (or after it was cleared). YaobowApplicationLoader must call \
             Pal3Service::set_script_factory after installing the script root.",
        );

        let cfg = shared::config::YaobowConfig::load();
        if cfg.auto_new_game_for(game) {
            let slot = cfg.auto_load_slot_for(game);
            // slot_exists first: PersistentState::load unwraps on a missing
            // file, and the first boot after enabling auto_load_slot has no
            // save yet -- that unwrap took the whole game down at startup.
            if slot > 0
                && shared::openpal3::states::persistent_state::PersistentState::slot_exists(
                    PAL3_APP_NAME,
                    slot,
                )
            {
                if let Some(d) = self.load_adventure_director(asset_path, slot) {
                    log::info!("PAL3: resumed from save slot {}", slot);
                    return d;
                }
                log::info!("PAL3: save slot {} missing, starting a new game", slot);
            }
            log::info!("PAL3: auto_new_game set, skipping the start menu");
            return self.create_adventure_director(asset_path);
        }

        let menu = factory.make_pal3_start_menu(asset_path);
        match menu.query_interface::<IDirector>() {
            Some(director) => director,
            None => {
                log::warn!(
                    "Pal3Service::create_director: scripted start menu did not expose IDirector \
                     for {}; falling back to a fresh adventure director",
                    asset_path
                );
                self.create_adventure_director(asset_path)
            }
        }
    }

    fn create_adventure_director(&self, asset_path: &str) -> ComRc<IDirector> {
        let asset_mgr = self.asset_manager_for(asset_path);
        let dialog_renderer = self.build_dialog_renderer(asset_mgr.clone());
        let status_renderer = self.build_status_renderer(asset_mgr.clone());
        let engine_rc = self.app.engine();
        let engine = engine_rc.borrow();
        let audio_engine = engine.audio_engine();
        let scene_manager = engine.scene_manager().clone();
        let ui = engine.ui_manager();
        drop(engine);
        let input_engine = self.input_engine_for_director();

        let adv = AdventureDirector::new(
            PAL3_APP_NAME,
            asset_mgr,
            audio_engine,
            input_engine,
            ui,
            scene_manager,
            Some(Self::sce_options()),
            dialog_renderer,
            status_renderer,
        );
        if let Some(bridge) = self.agent_bridge.borrow().as_ref() {
            adv.set_agent_bridge(bridge.clone());
        }
        self.track_persistent_state(&adv);
        ComRc::<IDirector>::from_object(adv)
    }

    fn load_adventure_director(&self, asset_path: &str, slot: i32) -> Option<ComRc<IDirector>> {
        // 档不存在就别往下走：`PersistentState::load` 打开文件直接 `unwrap`，读一个
        // 空档位整个游戏就崩了。读档菜单只列存在的档，agent 的 `/v1/load` 不经过它。
        if !shared::openpal3::states::persistent_state::PersistentState::slot_exists(
            PAL3_APP_NAME,
            slot,
        ) {
            log::warn!("load: 存档 {slot} 不存在");
            return None;
        }
        let asset_mgr = self.asset_manager_for(asset_path);
        let dialog_renderer = self.build_dialog_renderer(asset_mgr.clone());
        let status_renderer = self.build_status_renderer(asset_mgr.clone());
        let engine_rc = self.app.engine();
        let engine = engine_rc.borrow();
        let audio_engine = engine.audio_engine();
        let scene_manager = engine.scene_manager().clone();
        let ui = engine.ui_manager();
        drop(engine);
        let input_engine = self.input_engine_for_director();

        let adv = AdventureDirector::load(
            PAL3_APP_NAME,
            asset_mgr,
            audio_engine,
            input_engine,
            ui,
            scene_manager,
            Some(Self::sce_options()),
            slot,
            dialog_renderer,
            status_renderer,
        )?;
        if let Some(bridge) = self.agent_bridge.borrow().as_ref() {
            adv.set_agent_bridge(bridge.clone());
        }
        self.track_persistent_state(&adv);
        Some(ComRc::<IDirector>::from_object(adv))
    }

    fn save_slot_exists(&self, slot: i32) -> bool {
        if slot < 1 || slot > SAVE_SLOT_COUNT {
            return false;
        }
        ydirs::save_dir()
            .join(PAL3_APP_NAME)
            .join("Save")
            .join(format!("{}.json", slot))
            .is_file()
    }

    fn save_current(&self, slot: i32) -> bool {
        // Deferred to `pump_pre_update`: this is called from a script painted
        // inside the AdventureDirector's update, where touching the director
        // would re-borrow its RefCell and abort.
        if slot < 1 || slot > SAVE_SLOT_COUNT {
            return false;
        }
        self.pending_save_slot.set(slot);
        true
    }

    fn request_load_slot(&self, slot: i32) -> bool {
        // Deferred like `save_current`; swapping the director mid-update
        // would additionally pull the rug from under the running script.
        if !self.save_slot_exists(slot) {
            return false;
        }
        self.pending_load_slot.set(slot);
        true
    }

    fn can_save(&self) -> bool {
        self.can_save_here.get() || self.at_save_point.get()
    }

    fn take_save_point_request(&self) -> bool {
        self.save_point_request.replace(false)
    }

    fn leave_save_point(&self) {
        self.at_save_point.set(false);
    }

    fn save_slot_count(&self) -> i32 {
        SAVE_SLOT_COUNT
    }

    fn inventory_size(&self, type_mask: i32) -> i32 {
        self.matching_stacks(type_mask).len() as i32
    }

    fn inventory_item_id(&self, type_mask: i32, index: i32) -> i32 {
        if index < 0 {
            return -1;
        }
        self.matching_stacks(type_mask)
            .get(index as usize)
            .map(|id| *id as i32)
            .unwrap_or(-1)
    }

    fn item_count(&self, item_id: i32) -> i32 {
        if item_id < 0 {
            return 0;
        }
        self.with_persistent_state(0, |s| s.item_count(item_id as u32) as i32)
    }

    fn item_info(&self, item_id: i32, field: i32) -> i32 {
        let db = self.active_game_db.borrow();
        let Some(item) = db.as_ref().and_then(|db| db.item(item_id.max(0) as u32)) else {
            return 0;
        };
        match field {
            ITEM_FIELD_TYPE => item.item_type as i32,
            ITEM_FIELD_WEAPON_TYPE => item.weapon_type as i32,
            ITEM_FIELD_PRICE => item.price,
            ITEM_FIELD_ANCIENT_VALUE => item.ancient_value,
            ITEM_FIELD_PLACE_OF_USE => item.place_of_use_type as i32,
            ITEM_FIELD_TARGET_RANGE => item.target_range_type as i32,
            ITEM_FIELD_SPECIAL_TYPE => item.item_special_type as i32,
            ITEM_FIELD_ORE_ID => item.ore_id as i32,
            ITEM_FIELD_PRODUCT_ID => item.product_id as i32,
            ITEM_FIELD_PRODUCT_PRICE => item.product_price,
            ITEM_FIELD_SYNTH_MATERIAL_1 => item.synthesis_material_ids[0] as i32,
            ITEM_FIELD_SYNTH_MATERIAL_2 => item.synthesis_material_ids[1] as i32,
            ITEM_FIELD_SYNTH_PRODUCT => item.synthesis_product_id as i32,
            ITEM_FIELD_COMBO_COUNT => item.combo_count,
            _ => 0,
        }
    }

    fn draw_item_name(
        &self,
        ui: ComRc<IUiHost>,
        item_id: i32,
        x: f32,
        y: f32,
        size: f32,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    ) {
        let Some(name) = self.item_text(item_id, |item| item.name.as_str().ok()) else {
            return;
        };
        ui.text_at_sized(x, y, size, r, g, b, a, &name);
    }

    fn draw_item_description(
        &self,
        ui: ComRc<IUiHost>,
        item_id: i32,
        x: f32,
        y: f32,
        size: f32,
        wrap_px: f32,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    ) {
        let Some(desc) = self.item_text(item_id, |item| item.description.as_str().ok()) else {
            return;
        };
        for (line_no, line) in wrap_widget_text(&desc, size, wrap_px).iter().enumerate() {
            ui.text_at_sized(x, y + line_no as f32 * size, size, r, g, b, a, line);
        }
    }

    fn draw_item_stat_line(
        &self,
        ui: ComRc<IUiHost>,
        item_id: i32,
        x: f32,
        y: f32,
        size: f32,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    ) {
        let Some(desc) = self.item_text(item_id, |item| item.description.as_str().ok()) else {
            return;
        };
        let Some((_, stats)) = desc.split_once('$') else {
            return;
        };
        let stats = stats.trim_end_matches(['\n', '\r', ' ', '\u{3000}']);
        if stats.is_empty() {
            return;
        }
        ui.text_at_sized(x, y, size, r, g, b, a, stats);
    }

    fn item_usable(&self, item_id: i32, actor_id: i32) -> bool {
        if item_id < 0 || actor_id < 0 {
            return false;
        }
        let ctx = self.item_use_ctx.get();
        self.with_item_and_state(item_id, false, |item, state, db| {
            item_use::usable(item, state, actor_id as u32, db, &ctx)
        })
    }

    fn item_use_target(&self, item_id: i32) -> i32 {
        if item_id < 0 {
            return -3;
        }
        let ctx = self.item_use_ctx.get();
        self.with_item_and_state(
            item_id,
            -3,
            |item, state, db| match item_use::default_target(item, state, db, &ctx) {
                TargetPick::Single { actor_id, .. } => actor_id as i32,
                TargetPick::All { .. } => -1,
                TargetPick::None => -2,
                TargetPick::Invalid => -3,
            },
        )
    }

    fn item_use_arrow(&self, item_id: i32, actor_id: i32) -> bool {
        if item_id < 0 || actor_id < 0 {
            return false;
        }
        let ctx = self.item_use_ctx.get();
        self.with_item_and_state(item_id, false, |item, state, db| {
            // range 1 只在默认目标头上画箭头，range 2 全队都画——两支的
            // 条件都是「被查的那个人可用」（`FUN_0049c660`）。
            match item_use::default_target(item, state, db, &ctx) {
                TargetPick::Single {
                    actor_id: target,
                    arrow,
                } => arrow && target == actor_id as u32,
                TargetPick::All { arrow, .. } => arrow && state.is_in_team(actor_id as u32),
                _ => false,
            }
        })
    }

    fn use_item(&self, item_id: i32, actor_id: i32) -> i32 {
        if item_id < 0 || actor_id < 0 {
            return 0;
        }
        let ctx = self.item_use_ctx.get();
        let db = self.active_game_db.borrow().clone();
        let Some(db) = db else { return 0 };
        let Some(item) = db.item(item_id as u32) else {
            return 0;
        };

        let handle = self.active_persistent_state.borrow();
        let Some(state) = handle.as_ref() else {
            return 0;
        };
        let Ok(mut state) = state.try_borrow_mut() else {
            return 0;
        };

        match item_use::apply(item, &mut state, actor_id as u32, &db, &ctx) {
            UseOutcome::Rejected => 0,
            UseOutcome::Applied => 1,
            UseOutcome::RunProcByName(name) => {
                *self.pending_item_proc.borrow_mut() = Some(PendingItemProc::ByName(name));
                2
            }
            UseOutcome::RunProc(id) => {
                *self.pending_item_proc.borrow_mut() = Some(PendingItemProc::Id(id));
                2
            }
            // 关菜单，下一帧交给导演（`Scene::UseSeekingMouse`）：有藏着的东西放搜宝鼠，没有放 "XMouse"。
            UseOutcome::TreasureMouse { .. } => {
                *self.pending_item_proc.borrow_mut() = Some(PendingItemProc::TreasureMouse);
                2
            }
            UseOutcome::LearnedSkill(_) => 3,
            UseOutcome::LevelUp => 4,
        }
    }

    fn item_use_can_continue(&self, item_id: i32) -> bool {
        if item_id < 0 {
            return false;
        }
        let ctx = self.item_use_ctx.get();
        self.with_item_and_state(item_id, false, |item, state, db| {
            item_use::can_continue(item, state, db, &ctx)
        })
    }

    fn equipped_item(&self, actor_id: i32, slot: i32) -> i32 {
        if actor_id < 0 || slot < 0 {
            return -1;
        }
        self.with_persistent_state(-1, |s| {
            s.equipped(actor_id as u32, slot as usize)
                .map(|id| id as i32)
                .unwrap_or(-1)
        })
    }

    fn equip_item(&self, actor_id: i32, item_id: i32) -> bool {
        if actor_id < 0 || item_id < 0 {
            return false;
        }
        let Some(item_type) = ({
            let db = self.active_game_db.borrow();
            db.as_ref()
                .and_then(|db| db.item(item_id as u32))
                .map(|item| item.item_type)
        }) else {
            return false;
        };

        let handle = self.active_persistent_state.borrow();
        let Some(state) = handle.as_ref() else {
            return false;
        };
        let Ok(mut state) = state.try_borrow_mut() else {
            return false;
        };
        // `equip` reports the displaced piece, not success — a first-time
        // fit displaces nothing, so check the slot instead.
        state.equip(actor_id as u32, item_id as u32, item_type);
        state
            .equipped_slot_of(actor_id as u32, item_id as u32)
            .is_some()
    }

    fn unequip_slot(&self, actor_id: i32, slot: i32) -> bool {
        if actor_id < 0 || slot < 0 {
            return false;
        }
        let handle = self.active_persistent_state.borrow();
        let Some(state) = handle.as_ref() else {
            return false;
        };
        let Ok(mut state) = state.try_borrow_mut() else {
            return false;
        };
        state
            .unequip_to_inventory(actor_id as u32, slot as usize)
            .is_some()
    }

    fn skill_count(&self, actor_id: i32) -> i32 {
        if actor_id < 0 {
            return 0;
        }
        self.with_persistent_state(0, |s| {
            s.role(actor_id as u32)
                .map(|r| r.skills.len() as i32)
                .unwrap_or(0)
        })
    }

    fn skill_id(&self, actor_id: i32, index: i32) -> i32 {
        if actor_id < 0 || index < 0 {
            return -1;
        }
        self.with_persistent_state(-1, |s| {
            s.role(actor_id as u32)
                .and_then(|r| r.skills.get(index as usize))
                .map(|s| s.id as i32)
                .unwrap_or(-1)
        })
    }

    fn skill_info(&self, skill_id: i32, field: i32) -> i32 {
        if skill_id < 0 {
            return 0;
        }
        let db = self.active_game_db.borrow();
        let Some(skill) = db.as_ref().and_then(|db| db.skill(skill_id as u32)) else {
            return 0;
        };
        match field {
            SKILL_FIELD_TYPE => skill.skill_type as i32,
            SKILL_FIELD_LEVEL => skill.level as i32,
            SKILL_FIELD_SP_COST => skill.sp_consume_value,
            SKILL_FIELD_MP_COST => skill.mp_consume_value,
            SKILL_FIELD_SUCCESS_RATE => skill.success_rate_level as i32,
            // The database stores one i32 per element, of which only the
            // low byte matters; fold them into bit 0 水 … bit 4 土.
            SKILL_FIELD_ELEMENT_MASK => skill
                .element_attributes
                .iter()
                .enumerate()
                .filter(|(_, v)| **v != 0)
                .fold(0, |mask, (i, _)| mask | (1 << i)),
            SKILL_FIELD_REQUIRED_LEVEL => skill.required_actor_level as i32,
            SKILL_FIELD_OUTSIDE_COMBAT => skill.is_usable_outside_combat as i32,
            SKILL_FIELD_NEXT_LEVEL_ID => skill.next_level_skill_id as i32,
            SKILL_FIELD_TARGET_RANGE => skill.target_range_type as i32,
            SKILL_FIELD_MAGIC_LEVEL => skill.magic_level as i32,
            // Uses still needed before this level advances. `level` is
            // 1-based; the table has one entry per level below the cap.
            SKILL_FIELD_USE_TIMES => skill
                .times_before_level_up
                .get(skill.level.saturating_sub(1) as usize)
                .map(|v| *v as i32)
                .unwrap_or(0),
            _ => 0,
        }
    }

    fn draw_skill_name(
        &self,
        ui: ComRc<IUiHost>,
        skill_id: i32,
        x: f32,
        y: f32,
        size: f32,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    ) {
        let Some(name) = self.skill_text(skill_id, |s| s.name.as_str().ok()) else {
            return;
        };
        ui.text_at_sized(x, y, size, r, g, b, a, &name);
    }

    fn draw_skill_description(
        &self,
        ui: ComRc<IUiHost>,
        skill_id: i32,
        x: f32,
        y: f32,
        size: f32,
        wrap_px: f32,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    ) {
        let Some(desc) = self.skill_text(skill_id, |s| s.description.as_str().ok()) else {
            return;
        };
        for (line_no, line) in wrap_widget_text(&desc, size, wrap_px).iter().enumerate() {
            ui.text_at_sized(x, y + line_no as f32 * size, size, r, g, b, a, line);
        }
    }

    fn combat_actor_count(&self) -> i32 {
        let db = self.active_game_db.borrow();
        db.as_ref()
            .map(|db| db.combat_actors().len() as i32)
            .unwrap_or(0)
    }

    fn combat_actor_id_at(&self, index: i32) -> i32 {
        if index < 0 {
            return -1;
        }
        let db = self.active_game_db.borrow();
        db.as_ref()
            .and_then(|db| db.combat_actors().get(index as usize))
            .map(|a| a.id as i32)
            .unwrap_or(-1)
    }

    fn combat_actor_info(&self, actor_id: i32, field: i32) -> i32 {
        if actor_id < 0 {
            return 0;
        }
        let db = self.active_game_db.borrow();
        let Some(a) = db.as_ref().and_then(|db| db.combat_actor(actor_id as u32)) else {
            return 0;
        };
        match field {
            ACTOR_FIELD_TYPE => a.actor_type as i32,
            ACTOR_FIELD_LEVEL => a.level,
            ACTOR_FIELD_HP..=ACTOR_FIELD_LUCK => {
                a.attribute_values[(field - ACTOR_FIELD_HP) as usize]
            }
            ACTOR_FIELD_WATER..=ACTOR_FIELD_EARTH => {
                a.element_attribute_values[(field - ACTOR_FIELD_WATER) as usize]
            }
            ACTOR_FIELD_EXP => a.experience,
            ACTOR_FIELD_MONEY_WHEN_KILLED => a.money_when_killed as i32,
            ACTOR_FIELD_LOOT_ID => a.normal_loot as i32,
            ACTOR_FIELD_LOOT_COUNT => a.normal_loot_count as i32,
            ACTOR_FIELD_STEALABLE_ITEM => a.stealable_item_id as i32,
            ACTOR_FIELD_STEALABLE_MONEY => a.stealable_money_amount as i32,
            ACTOR_FIELD_CORPSE_ID => a.corpse_id as i32,
            _ => 0,
        }
    }

    fn draw_model_view(
        &self,
        ui: ComRc<IUiHost>,
        key: i32,
        model: &str,
        size: f32,
        look_x: f32,
        look_y: f32,
        yaw: f32,
        frontal: bool,
        mode: i32,
        speed: f32,
        serial: i32,
        ox: f32,
        oy: f32,
        scale: f32,
    ) -> bool {
        use shared::openpal3::model_view::{ModelView, ViewCamera, v_range};
        use shared::openpal3::scene::CvdPlayMode;
        let Some(asset_path) = self.last_asset_path.borrow().clone() else {
            return false;
        };
        let Some(sinks) = self.texture_sinks.borrow().clone() else {
            return false;
        };
        let mut views = self.model_views.borrow_mut();
        if views.get(&key).is_none_or(|v| v.model != model) {
            let mut old_spin = None;
            if let Some(old) = views.remove(&key) {
                if old.view.spin() != 0. {
                    old_spin = Some((old.view.angle(), old.view.spin()));
                }
                sinks.1.borrow_mut().push(old.com_id);
            }
            let am = self.asset_manager_for(&asset_path);
            let camera = ViewCamera {
                size,
                x: look_x as i32,
                y: look_y as i32,
                tilt: (!frontal).then_some(600.),
            };
            let view = self
                .model_source(model)
                .and_then(|src| ModelView::build(&am, &src, camera, yaw));
            let Some(view) = view else {
                return false;
            };
            // 换模型不动转角（`FUN_00439230` 不写 `+0xf8`），自转也接着转。
            if let Some(old) = &old_spin {
                view.set_angle(old.0);
                view.set_spin(old.1);
            }
            views.insert(
                key,
                ModelViewSlot {
                    model: model.to_string(),
                    view,
                    com_id: next_model_view_com_id(),
                    serial: i32::MIN,
                    last: std::time::Instant::now(),
                },
            );
        }
        let Some(slot) = views.get_mut(&key) else {
            return false;
        };
        if slot.serial != serial {
            slot.serial = serial;
            let mode = match mode {
                1 => CvdPlayMode::Once,
                2 => CvdPlayMode::Loop,
                _ => CvdPlayMode::Hold,
            };
            slot.view.play(mode, speed);
        }
        let now = std::time::Instant::now();
        let dt = now.duration_since(slot.last).as_secs_f32().min(0.1);
        slot.last = now;
        slot.view.render(dt);
        if let Some(tex) = slot.view.texture_id() {
            sinks
                .0
                .borrow_mut()
                .push((slot.com_id, imgui::TextureId::new(tex as usize)));
            let (u0, u1) = (0., 1.);
            let (v0, v1) = v_range(0., 1.);
            ui.image_rect(
                slot.com_id as i32,
                ox,
                oy,
                ox + 800. * scale,
                oy + 600. * scale,
                u0,
                v0,
                u1,
                v1,
            );
        }
        slot.view.finished()
    }

    fn ui_sound(&self, name: &str) -> () {
        self.play_ui_sound(name);
    }

    fn spin_model_view(&self, key: i32, deg_per_sec: f32) -> () {
        if let Some(slot) = self.model_views.borrow().get(&key) {
            slot.view.set_spin(deg_per_sec);
        }
    }

    fn draw_lineup_disc(
        &self,
        ui: ComRc<IUiHost>,
        mx: f32,
        my: f32,
        down: bool,
        ox: f32,
        oy: f32,
        scale: f32,
    ) -> () {
        use shared::openpal3::lineup_disc::LineupDisc;
        use shared::openpal3::model_view::v_range;
        let Some(asset_path) = self.last_asset_path.borrow().clone() else {
            return;
        };
        let Some(sinks) = self.texture_sinks.borrow().clone() else {
            return;
        };
        let team: Vec<u32> =
            self.with_persistent_state(vec![], |s| s.team().iter().map(|id| *id as u32).collect());
        let lineup: Vec<(u32, Option<u8>)> = self.with_persistent_state(vec![], |s| {
            team.iter()
                .map(|id| (*id, s.lineup_position(*id)))
                .collect()
        });

        let longkui = self.with_persistent_state(0, |s| s.longkui_mode());
        let mut slot = self.lineup_disc.borrow_mut();
        if slot
            .as_ref()
            .is_none_or(|d| d.disc.team() != team || d.disc.longkui_mode() != longkui)
        {
            let pitch = slot.as_ref().map(|d| d.disc.pitch());
            if let Some(old) = slot.take() {
                sinks.1.borrow_mut().push(old.com_id);
            }
            let am = self.asset_manager_for(&asset_path);
            let Some(mut disc) = LineupDisc::build(&am, &team, longkui) else {
                return;
            };
            if let Some(p) = pitch {
                disc.set_pitch(p);
            }
            *slot = Some(LineupDiscSlot {
                disc,
                com_id: next_model_view_com_id(),
                last: std::time::Instant::now(),
                last_y: None,
                prev_down: false,
                sounds: vec![],
            });
        }
        let Some(slot) = slot.as_mut() else {
            return;
        };

        let now = std::time::Instant::now();
        let dt = now.duration_since(slot.last).as_secs_f32().min(0.1);
        slot.last = now;

        // 按住在盘上时，鼠标纵向位移（按 800×600 画布算）加到俯角上（`0x485230`）。
        let (x, y) = ((mx - ox) / scale, (my - oy) / scale);
        if down && slot.disc.hits_disc(x, y) {
            if let Some(last) = slot.last_y {
                let pitch = slot.disc.pitch() + (y - last);
                slot.disc.set_pitch(pitch);
            }
        }
        slot.last_y = down.then_some(y);

        // 手柄右摇杆上下也调俯角（移植端加的，原版只有鼠标）；方向和往下拖一致。
        let stick = self
            .input_engine_for_director()
            .borrow()
            .get_axis_state(radiance::input::Axis::RightStickY)
            .value();
        if stick.abs() > 0.2 {
            let pitch =
                slot.disc.pitch() + stick * shared::openpal3::lineup_disc::PAD_PITCH_SPEED * dt;
            slot.disc.set_pitch(pitch);
        }

        // 点盘上的人 / 空站位（`0x485352` 起）：选中、换位、挪位，站位写回存档。
        let debug_click = shared::openpal3::lineup_disc::take_debug_click();
        let click = (down && !slot.prev_down) || debug_click.is_some();
        let (cx, cy) = debug_click.unwrap_or((x, y));
        slot.prev_down = down;
        slot.disc.step(dt);

        // 手柄：方向键左右挪光标、A 键等于点光标那一格（移植端加的，原版只有鼠标）。
        let (pad_left, pad_right, pad_a) = {
            use radiance::input::Key;
            let input = self.input_engine_for_director();
            let input = input.borrow();
            (
                input.get_key_state(Key::GamePadDPadLeft).pressed(),
                input.get_key_state(Key::GamePadDPadRight).pressed(),
                input.get_key_state(Key::GamePadEast).pressed(),
            )
        };
        if pad_left {
            slot.disc.pad_move(-1);
        }
        if pad_right {
            slot.disc.pad_move(1);
        }

        let mut lineup = lineup;
        let pos = |id: u32, l: &[(u32, Option<u8>)]| {
            l.iter().find(|(i, _)| *i == id).and_then(|(_, p)| *p)
        };
        let action = if click {
            slot.disc.click(cx, cy)
        } else if pad_a {
            slot.disc.pad_activate(|id| pos(id, &lineup))
        } else {
            shared::openpal3::lineup_disc::DiscAction::None
        };
        {
            use shared::openpal3::lineup_disc::DiscAction;
            let changes: Vec<(u32, u8)> = match action {
                DiscAction::Swap(a, b) => match (pos(a, &lineup), pos(b, &lineup)) {
                    (Some(pa), Some(pb)) => vec![(a, pb), (b, pa)],
                    (None, Some(pb)) => vec![(a, pb)],
                    (Some(pa), None) => vec![(b, pa)],
                    (None, None) => vec![],
                },
                DiscAction::Move(a, k) => vec![(a, k)],
                _ => vec![],
            };
            if !changes.is_empty() {
                self.play_lineup_sound(&asset_path, &mut slot.sounds);
            }
            for (id, p) in changes {
                if self.set_lineup_position(id as i32, p as i32) {
                    for e in lineup.iter_mut() {
                        if e.0 == id {
                            e.1 = Some(p);
                        } else if e.1 == Some(p) {
                            e.1 = None;
                        }
                    }
                }
            }
        }

        {
            use radiance::audio::AudioSourceState;
            slot.sounds
                .retain(|s| s.state() != AudioSourceState::Stopped);
            for s in slot.sounds.iter_mut() {
                s.update();
            }
        }

        slot.disc
            .place(|id| lineup.iter().find(|(i, _)| *i == id).and_then(|(_, p)| *p));
        slot.disc.render(dt);
        if let Some(tex) = slot.disc.texture_id() {
            sinks
                .0
                .borrow_mut()
                .push((slot.com_id, imgui::TextureId::new(tex as usize)));
            let (v0, v1) = v_range(0., 1.);
            ui.image_rect(
                slot.com_id as i32,
                ox,
                oy,
                ox + 800. * scale,
                oy + 600. * scale,
                0.,
                v0,
                1.,
                v1,
            );
        }
    }

    fn draw_combat_actor_name(
        &self,
        ui: ComRc<IUiHost>,
        actor_id: i32,
        x: f32,
        y: f32,
        size: f32,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    ) {
        let Some(name) = self.actor_text(actor_id, |a| a.name.as_str().ok()) else {
            return;
        };
        ui.text_at_sized(x, y, size, r, g, b, a, &name);
    }

    fn draw_combat_actor_description(
        &self,
        ui: ComRc<IUiHost>,
        actor_id: i32,
        x: f32,
        y: f32,
        size: f32,
        wrap_px: f32,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    ) {
        let Some(desc) = self.actor_text(actor_id, |a| a.description.as_str().ok()) else {
            return;
        };
        for (line_no, line) in wrap_widget_text(&desc, size, wrap_px).iter().enumerate() {
            ui.text_at_sized(x, y + line_no as f32 * size, size, r, g, b, a, line);
        }
    }

    fn lineup_position(&self, actor_id: i32) -> i32 {
        if actor_id < 0 {
            return -1;
        }
        self.with_persistent_state(-1, |s| {
            s.lineup_position(actor_id as u32)
                .map(|p| p as i32)
                .unwrap_or(-1)
        })
    }

    fn lineup_occupant(&self, position: i32) -> i32 {
        if position < 0 {
            return -1;
        }
        self.with_persistent_state(-1, |s| {
            s.lineup_occupant(position as u8)
                .map(|id| id as i32)
                .unwrap_or(-1)
        })
    }

    fn set_lineup_position(&self, actor_id: i32, position: i32) -> bool {
        if actor_id < 0 || position < 0 {
            return false;
        }
        let handle = self.active_persistent_state.borrow();
        let Some(state) = handle.as_ref() else {
            return false;
        };
        let Ok(mut state) = state.try_borrow_mut() else {
            return false;
        };
        state.set_lineup_position(actor_id as u32, position as u8)
    }

    fn sword_flight_unlocked(&self) -> bool {
        self.with_persistent_state(false, |state| state.feature_enabled(Feature::SwordSkill))
    }

    fn fly_sword(&self) -> i32 {
        if !self.sword_flight_unlocked() {
            return 0;
        }
        let ctx = self.item_use_ctx.get();
        match item_use::sword_flight_refusal(&ctx) {
            Some(index) => {
                self.play_ui_sound("wa003");
                index as i32
            }
            None => {
                self.play_ui_sound("wa001");
                self.pending_bigmap.set(true);
                0
            }
        }
    }

    fn draw_lang_text(
        &self,
        ui: ComRc<IUiHost>,
        index: i32,
        x: f32,
        y: f32,
        size: f32,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    ) {
        if index < 0 {
            return;
        }
        let Some(text) = self.assis_lang_text(index as usize) else {
            return;
        };
        ui.text_at_sized(
            x,
            y,
            size,
            r,
            g,
            b,
            a,
            &shared::openpal3::assis_lang::unescape(&text),
        );
    }

    fn lineup_page_opened(&self) -> () {
        let handle = self.active_persistent_state.borrow();
        let Some(state) = handle.as_ref() else {
            return;
        };
        if let Ok(mut state) = state.try_borrow_mut() {
            state.reread_lineup_for_page();
        }
    }

    fn monster_seen(&self, actor_id: i32) -> bool {
        if actor_id < 0 {
            return false;
        }
        self.with_persistent_state(false, |s| s.monster_seen(actor_id as u32))
    }

    fn monsters_seen_count(&self) -> i32 {
        self.with_persistent_state(0, |s| s.monsters_seen_count() as i32)
    }

    fn party_size(&self) -> i32 {
        self.with_persistent_state(0, |s| s.team().len() as i32)
    }

    fn party_actor_id(&self, index: i32) -> i32 {
        self.with_persistent_state(-1, |s| {
            if index < 0 {
                return -1;
            }
            s.team()
                .get(index as usize)
                .map(|id| *id as i32)
                .unwrap_or(-1)
        })
    }

    fn role_stat(&self, actor_id: i32, stat: i32) -> i32 {
        if actor_id < 0 {
            return 0;
        }
        self.with_persistent_state(0, |s| {
            let Some(role) = s.role(actor_id as u32) else {
                return 0;
            };
            match stat {
                STAT_LEVEL => role.level,
                STAT_EXP => role.exp,
                // -1 at level 99, where the original prints a dash
                // instead of a number.
                STAT_EXP_TO_NEXT => self
                    .active_game_db
                    .borrow()
                    .as_ref()
                    .and_then(|db| db.exp_to_next_level(role.level, role.exp))
                    .unwrap_or(-1),
                STAT_HP => role.hp,
                STAT_MAX_HP => role.max_hp,
                STAT_SP => role.sp,
                STAT_MAX_SP => role.max_sp,
                STAT_MP => role.mp,
                STAT_MAX_MP => role.max_mp,
                STAT_WATER..=STAT_EARTH => role.elements[(stat - STAT_WATER) as usize],
                STAT_ATTACK => role.attack,
                STAT_DEFENSE => role.defense,
                STAT_SPEED => role.speed,
                STAT_LUCK => role.luck,
                _ => 0,
            }
        })
    }

    fn controlled_role(&self) -> i32 {
        self.hud_role.get()
    }

    fn poison_kind(&self) -> i32 {
        self.hud_poison.get()
    }

    fn leader_field_states(&self) -> i32 {
        self.with_persistent_state(0, |s| {
            (0..4)
                .filter(|&i| s.leader_maze_state(27 + i))
                .fold(0, |m, i| m | (1 << i))
        })
    }

    fn field_state_time(&self) -> f32 {
        self.with_persistent_state(0., |s| s.maze_effect_time())
    }

    fn money(&self) -> i32 {
        self.with_persistent_state(0, |s| s.money())
    }

    fn play_time(&self) -> i32 {
        self.with_persistent_state(0, |s| s.play_time() as i32)
    }

    fn draw_slot_label(
        &self,
        ui: ComRc<IUiHost>,
        slot: i32,
        x: f32,
        y: f32,
        r: f32,
        g: f32,
        b: f32,
        a: f32,
    ) {
        // Slot metadata is composed here (not returned) because the
        // script bridge cannot marshal string returns. v1 shows the
        // scene/sub-scene codes from the slot JSON; the original PC
        // release shows a display name ("新仙界贰层") + play time +
        // a 64x64 screenshot, which need data we don't record yet.
        if !self.save_slot_exists(slot) {
            return;
        }
        let path = ydirs::save_dir()
            .join(PAL3_APP_NAME)
            .join("Save")
            .join(format!("{}.json", slot));
        let Ok(content) = std::fs::read_to_string(&path) else {
            return;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) else {
            return;
        };
        let scene = v.get("scene").and_then(|s| s.as_str()).unwrap_or("");
        let sub = v.get("sub_scene").and_then(|s| s.as_str()).unwrap_or("");
        let label = match self.scene_display_name(scene, sub) {
            Some(name) => name,
            None if scene.is_empty() => "已存档".to_string(),
            None => format!("{} · {}", scene, sub),
        };
        ui.text_at_small(x, y, r, g, b, a, &label);
    }

    fn load_menu_bgm(&self, asset_path: &str) -> Option<ComRc<IAudioSource>> {
        let asset_mgr = self.asset_manager_for(asset_path);
        let engine_rc = self.app.engine();
        let audio_engine = engine_rc.borrow().audio_engine();
        drop(engine_rc);
        // PAL3A's menu BGM track differs from PAL3's. Both share this
        // service; pick by the game variant captured in create_director.
        let track = match self.last_game.get() {
            GameType::PAL3A => "P01",
            _ => "PI01",
        };
        // `load_music_data` panics on a missing track; isolate it so a
        // partial asset set still yields a (silent) menu.
        let data = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            asset_mgr.load_music_data(track)
        }))
        .ok()?;
        let mut source = audio_engine.create_source();
        source.set_data(data, AudioCodec::Mp3);
        Some(AudioSource::create(source))
    }

    fn create_sprite_service(&self, asset_path: &str) -> Option<ComRc<ISpriteService>> {
        let cache = self.texture_cache.borrow().clone()?;
        let asset_mgr = self.asset_manager_for(asset_path);
        Some(SpriteService::create(asset_mgr.vfs_rc(), cache))
    }

    fn create_ui_atlas(&self, asset_path: &str) -> Option<ComRc<IPal3UiAtlas>> {
        let sprites = self.create_sprite_service(asset_path)?;
        let asset_mgr = self.asset_manager_for(asset_path);
        // PAL3A replaces PAL3's text UI_opt.tli with the UIArtist.plug
        // manifest. Pick the format by the active game variant.
        let kind = match self.last_game.get() {
            GameType::PAL3A => AtlasManifest::Plug,
            _ => AtlasManifest::Tli,
        };
        let manifest = common::store_ext::StoreExt2::read_to_end(
            asset_mgr.vfs(),
            Pal3UiAtlas::manifest_path(kind),
        )
        .ok()?;
        Some(Pal3UiAtlas::create(sprites, &manifest, kind))
    }

    fn exit_app(&self) {
        self.app.request_exit();
    }

    fn play_intro_movie(&self, asset_path: &str) -> Option<ComRc<IVideoHandle>> {
        // PAL3A ships no intro movie; skip straight to the menu.
        if self.last_game.get() == GameType::PAL3A {
            return None;
        }
        // Once-per-process: re-entry into the menu must skip the intro.
        if self.intro_played.get() {
            return None;
        }
        let cache = self.texture_cache.borrow().clone()?;
        let asset_mgr = self.asset_manager_for(asset_path);
        let engine_rc = self.app.engine();
        let engine = engine_rc.borrow();
        let component_factory = engine.rendering_component_factory();
        let audio_engine = engine.audio_engine();
        drop(engine);

        let reader = asset_mgr.load_movie_data("LOGO");
        let mut player = component_factory.create_video_player();
        let size = player.play(
            component_factory.clone(),
            audio_engine,
            reader,
            VideoCodec::Bik,
            false,
        )?;
        self.intro_played.set(true);
        Some(VideoHandle::create(cache, player, size.0, size.1))
    }
}

#[cfg(test)]
mod tests {
    use super::wrap_widget_text;

    #[test]
    fn wraps_on_cell_widths_and_separators() {
        // 16px cell: a CJK char is 16px wide, ASCII 8px. A 64px area
        // therefore fits four CJK characters per line.
        let lines = wrap_widget_text("青竹加乌钢丝编成的斗笠", 16., 64.);
        assert_eq!(lines, vec!["青竹加乌", "钢丝编成", "的斗笠"]);

        // `$` separates flavour text from the stat line, `\n` ends it.
        let lines = wrap_widget_text("耐用。$防+28 速-1\n", 16., 0.);
        assert_eq!(lines, vec!["耐用。", "防+28 速-1"]);

        // ASCII packs two per cell.
        let lines = wrap_widget_text("abcdefgh", 16., 32.);
        assert_eq!(lines, vec!["abcd", "efgh"]);
    }
}

/// `draw_model_view` 的一个视图。
struct ModelViewSlot {
    model: String,
    view: shared::openpal3::model_view::ModelView,
    /// 在纹理缓存里的 id。和 `next_handle_com_id` 的负数区段错开。
    com_id: i64,
    serial: i32,
    last: std::time::Instant,
}

/// 阵法页的五灵盘。
struct LineupDiscSlot {
    disc: shared::openpal3::lineup_disc::LineupDisc,
    com_id: i64,
    last: std::time::Instant,
    /// 上一帧按着时鼠标的纵坐标（画布坐标）；松开就是 `None`。
    last_y: Option<f32>,
    /// 上一帧按着没有，用来认出点下的那一帧。
    prev_down: bool,
    /// 还在放的音效（换位 / 挪位）。
    sounds: Vec<Box<dyn radiance::audio::AudioMemorySource>>,
}

fn next_model_view_com_id() -> i64 {
    use std::sync::atomic::{AtomicI64, Ordering};
    static NEXT: AtomicI64 = AtomicI64::new(0);
    -1_000_000_000 - NEXT.fetch_add(1, Ordering::Relaxed)
}
