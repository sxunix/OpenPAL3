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
use packfs::init_virtual_fs;
use radiance::audio::Codec as AudioCodec;
use radiance::comdef::{
    IApplication, IApplicationExt, IDirector, ISceneManager, IUiHost, IUiLayer,
};
use radiance::input::{InputEngine, SyntheticInputBridge};
use radiance::radiance::{UiLayerBand, UiLayerHandle};
use radiance::video::Codec as VideoCodec;
use radiance_scripting::comdef::services::IAudioSource;
use radiance_scripting::comdef::services::ISprite;
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
use shared::openpal3::directors::AdventureDirector;
use shared::openpal3::states::game_db::GameDb;
use shared::openpal3::states::persistent_state::{PAL3_APP_NAME, PersistentState};
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

/// `IPal3Service::item_info` selectors.
const ITEM_FIELD_TYPE: i32 = 0;
const ITEM_FIELD_WEAPON_TYPE: i32 = 1;
const ITEM_FIELD_PRICE: i32 = 2;
const ITEM_FIELD_ANCIENT_VALUE: i32 = 3;
const ITEM_FIELD_PLACE_OF_USE: i32 = 4;
const ITEM_FIELD_TARGET_RANGE: i32 = 5;
const ITEM_FIELD_SPECIAL_TYPE: i32 = 6;

/// Where the item icons live in the UI atlas. The database stores only
/// the numeric suffix (`"1"`, `"6"`, …).
const ITEM_ICON_PREFIX: &str = "ui/GameMainUI/Prop/Icon/";

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
    /// Agent-server bridge. `None` when no `--agent-port` flag was
    /// passed; `Some(_)` enables [`Self::pump_agent`] and makes
    /// every fresh `AdventureDirector` honor pause/step + see
    /// synthetic input.
    agent_bridge: RefCell<Option<Rc<AgentBridge>>>,
}

ComObject_Pal3Service!(super::Pal3Service);

impl Pal3Service {
    pub fn create(app: ComRc<IApplication>) -> ComRc<IPal3Service> {
        ComRc::from_object(Self {
            app,
            asset_managers: RefCell::new(HashMap::new()),
            texture_cache: RefCell::new(None),
            script_factory: RefCell::new(None),
            last_asset_path: RefCell::new(None),
            last_game: Cell::new(GameType::PAL3),
            pending_debug_install: Cell::new(false),
            pending_save_slot: Cell::new(-1),
            pending_load_slot: Cell::new(-1),
            active_persistent_state: RefCell::new(None),
            active_game_db: RefCell::new(None),
            scene_name_table: RefCell::new(None),
            debug_layer_installed: Cell::new(false),
            debug_layer_handle: RefCell::new(None),
            intro_played: Cell::new(false),
            agent_bridge: RefCell::new(None),
        })
    }

    /// Install the imgui texture cache so the sprite/atlas loaders and
    /// dialog renderer can upload UI textures. Called once at boot.
    pub fn set_texture_cache(&self, cache: Rc<RefCell<ImguiTextureCache>>) {
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

    fn asset_manager_for(&self, asset_path: &str) -> Rc<AssetManager> {
        if let Some(am) = self.asset_managers.borrow().get(asset_path) {
            return am.clone();
        }

        let engine_rc = self.app.engine();
        let engine = engine_rc.borrow();
        let component_factory = engine.rendering_component_factory();
        drop(engine);

        let vfs = init_virtual_fs(&PathBuf::from(asset_path), None);
        let am = Rc::new(AssetManager::new_for_game(
            component_factory,
            Rc::new(vfs),
            self.last_game.get(),
        ));
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

    fn item_icon(&self, atlas: ComRc<IPal3UiAtlas>, item_id: i32) -> Option<ComRc<ISprite>> {
        let icon = self.item_text(item_id, |item| item.icon_name.as_str().ok())?;
        if icon.is_empty() {
            return None;
        }
        let name = format!("{}{}.tga", ITEM_ICON_PREFIX, icon);
        if !atlas.has_sprite(&name) {
            return None;
        }
        atlas.sprite(&name)
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
