use std::{
    cell::{Ref, RefCell, RefMut},
    collections::HashMap,
    path::PathBuf,
    rc::Rc,
};

use crate::openpal3::asset_manager::AssetManager;

use super::game_db::GameDb;
use super::persistent_state::PersistentState;
use common::store_ext::StoreExt2;
use radiance::{
    audio::{AudioEngine, AudioMemorySource, AudioSource, AudioSourceState, Codec as AudioCodec},
    rendering::VideoPlayer,
    video::Codec as VideoCodec,
};
use regex::Regex;

pub struct GlobalState {
    audio_engine: Rc<dyn AudioEngine>,
    persistent_state: Rc<RefCell<PersistentState>>,
    fop_state: FopState,
    adv_input_enabled: bool,
    role_controlled: i32,

    asset_mgr: Rc<AssetManager>,
    bgm_source: Box<dyn AudioMemorySource>,
    sound_sources: Vec<Rc<RefCell<Box<dyn AudioMemorySource>>>>,
    default_scene_bgm: HashMap<String, String>,
    /// `music.txt` `[combat]`: city → battle theme.
    combat_bgm: HashMap<String, String>,
    video_player: Box<VideoPlayer>,

    /// Items, skills, combat actors and the experience curve. `None` when
    /// the shipped data is missing or unreadable — callers fall back to
    /// placeholder behaviour rather than refusing to run.
    game_db: Option<Rc<GameDb>>,

    pass_through_wall: bool,
    divine_tree_flower_initialized: bool,
}

impl GlobalState {
    pub fn new(
        asset_mgr: Rc<AssetManager>,
        audio_engine: Rc<dyn AudioEngine>,
        persistent_state: Rc<RefCell<PersistentState>>,
    ) -> Self {
        let bgm_source = audio_engine.create_source();
        let video_player = asset_mgr.component_factory().create_video_player();
        let sound_sources = vec![];
        let music_path = "/basedata/basedata/datascript/music.txt";
        let music_txt = asset_mgr.vfs().read_to_end_from_gbk(music_path).unwrap();
        let default_scene_bgm = parse_music_mapping(&music_txt, "scene");
        let combat_bgm = parse_music_mapping(&music_txt, "combat");

        let game_db = GameDb::load(asset_mgr.vfs(), &PathBuf::from("/basedata/basedata"));
        if let Some(db) = game_db.as_ref() {
            persistent_state.borrow_mut().seed_from_db(db);
        }

        Self {
            persistent_state,
            audio_engine,
            fop_state: FopState::new(),
            adv_input_enabled: true,
            role_controlled: 0,
            asset_mgr,
            bgm_source,
            sound_sources,
            default_scene_bgm,
            combat_bgm,
            video_player,
            game_db,
            pass_through_wall: false,
            divine_tree_flower_initialized: false,
        }
    }

    /// The shipped item/skill/actor tables and experience curve.
    pub fn game_db(&self) -> Option<&Rc<GameDb>> {
        self.game_db.as_ref()
    }

    pub fn adv_input_enabled(&self) -> bool {
        self.adv_input_enabled
    }

    pub fn set_adv_input_enabled(&mut self, adv_input_enabled: bool) {
        self.adv_input_enabled = adv_input_enabled
    }

    pub fn role_controlled(&self) -> i32 {
        self.role_controlled
    }

    pub fn set_role_controlled(&mut self, role_controlled: i32) {
        self.role_controlled = role_controlled
    }

    pub fn play_bgm(&mut self, name: &str) {
        let data = self.asset_mgr.load_music_data(name);
        self.bgm_source.set_data(data, AudioCodec::Mp3);
        self.bgm_source.play(true);
    }

    pub fn play_default_bgm(&mut self) {
        if self.bgm_source.state() != AudioSourceState::Stopped {
            return;
        }

        let (scene_name, sub_scene_name) = {
            let p_state = self.persistent_state.borrow();
            (
                p_state.scene_name().unwrap().to_lowercase(),
                p_state.sub_scene_name().unwrap().to_lowercase(),
            )
        };

        if let Some(name) = self
            .default_scene_bgm
            .get(&format!("{}_{}", scene_name, sub_scene_name))
            .or(self.default_scene_bgm.get(&scene_name))
        {
            let name = name.to_string();
            if name != "NONE" {
                self.play_bgm(&name);
            } else {
                self.bgm_source.stop();
            }
        }
    }

    /// The battle theme for the current city: `music.txt`'s `[combat]`
    /// section keyed by scene name, as `FUN_0053e680` reads it. Returns
    /// false, and leaves the BGM alone, when the city has no entry — the
    /// original does the same.
    pub fn play_combat_bgm(&mut self) -> bool {
        let scene_name = self
            .persistent_state
            .borrow()
            .scene_name()
            .map(|s| s.to_lowercase());
        let Some(name) = scene_name.and_then(|s| self.combat_bgm.get(&s).cloned()) else {
            return false;
        };
        self.play_bgm(&name);
        true
    }

    pub fn bgm_source(&mut self) -> &mut dyn AudioSource {
        self.bgm_source.as_mut()
    }

    pub fn video_player(&mut self) -> &mut VideoPlayer {
        self.video_player.as_mut()
    }

    pub fn play_movie(&mut self, name: &str) -> Option<(u32, u32)> {
        let reader = self.asset_mgr.load_movie_data(name);
        let factory = self.asset_mgr.component_factory();
        self.video_player.play(
            factory,
            self.audio_engine.clone(),
            reader,
            VideoCodec::Bik,
            false,
        )
    }

    pub fn asset_mgr(&self) -> Rc<AssetManager> {
        self.asset_mgr.clone()
    }

    pub fn persistent_state(&self) -> Ref<'_, PersistentState> {
        self.persistent_state.borrow()
    }

    pub fn persistent_state_mut(&mut self) -> RefMut<'_, PersistentState> {
        self.persistent_state.borrow_mut()
    }

    /// A handle to the save state that outlives borrows of the director.
    /// The UI service holds one so status screens can read party stats
    /// while painting from inside the director's own update.
    pub fn persistent_state_rc(&self) -> Rc<RefCell<PersistentState>> {
        self.persistent_state.clone()
    }

    pub fn add_sound_source(&mut self, source: Rc<RefCell<Box<dyn AudioMemorySource>>>) {
        self.sound_sources.push(source);
    }

    pub fn remove_sound_source(&mut self, source: Rc<RefCell<Box<dyn AudioMemorySource>>>) {
        self.sound_sources
            .iter()
            .position(|s| Rc::ptr_eq(s, &source))
            .map(|p| self.sound_sources.remove(p));
    }

    pub fn fop_state_mut(&mut self) -> &mut FopState {
        &mut self.fop_state
    }

    pub fn update(&mut self, delta_sec: f32) {
        // The status page's clock is play time, so it advances only while
        // an adventure is running (this is called from `SceVm::update`).
        self.persistent_state
            .borrow_mut()
            .advance_play_time(delta_sec);

        if self.bgm_source.state() == AudioSourceState::Playing {
            self.bgm_source.update();
        }

        self.remove_stopped_sound_sources();
        for source in &mut self.sound_sources {
            if source.borrow().state() == AudioSourceState::Playing {
                source.borrow_mut().update();
            }
        }
    }

    fn remove_stopped_sound_sources(&mut self) {
        self.sound_sources
            .retain(|s| s.borrow().state() != AudioSourceState::Stopped);
    }

    /// 159 InitFlower's one-shot latch — the original's `DAT_00d8bc38`,
    /// a plain global that is not part of the save file, so it resets
    /// when the game restarts.
    pub fn divine_tree_flower_initialized(&self) -> bool {
        self.divine_tree_flower_initialized
    }

    pub fn set_divine_tree_flower_initialized(&mut self, done: bool) {
        self.divine_tree_flower_initialized = done;
    }

    pub fn pass_through_wall(&self) -> bool {
        self.pass_through_wall
    }

    pub fn pass_through_wall_mut(&mut self) -> &mut bool {
        &mut self.pass_through_wall
    }
}

pub enum Fop {
    And,
    Or,
}

pub struct FopState {
    lhs: Option<bool>,
    op: Option<Fop>,
}

impl FopState {
    pub fn new() -> Self {
        Self {
            lhs: None,
            op: None,
        }
    }

    pub fn push_value(&mut self, value: bool) {
        self.lhs = match (&self.lhs, &self.op) {
            (Some(lhs), Some(Fop::And)) => Some(*lhs && value),
            (Some(lhs), Some(Fop::Or)) => Some(*lhs || value),
            (_, _) => Some(value),
        }
    }

    pub fn set_op(&mut self, op: Fop) {
        self.op = Some(op);
    }

    pub fn reset(&mut self) {
        self.lhs = None;
        self.op = None;
    }

    pub fn value(&self) -> Option<bool> {
        self.lhs
    }
}

fn parse_music_mapping(mapping: &str, section: &str) -> HashMap<String, String> {
    let mut scene_mapping = HashMap::new();
    let scene_begin_regex = Regex::new(&format!(r"BEGIN,\s*{}\b", section)).unwrap();
    let scene_end_regex = Regex::new(r"END").unwrap();
    let music_mapping_regex = Regex::new(r"(.+?)\s*\$(.+?)&").unwrap();
    let mut scene_mapping_state = 0;
    for line in mapping.split('\n') {
        match scene_mapping_state {
            0 => {
                if scene_begin_regex.is_match(line) {
                    scene_mapping_state = 1;
                }
            }
            1 => {
                if scene_end_regex.is_match(line) {
                    scene_mapping_state = 2;
                } else {
                    let capture = music_mapping_regex.captures_iter(line).next();
                    if let Some(c) = capture {
                        scene_mapping.insert(c[1].to_lowercase(), c[2].to_string());
                    }
                }
            }
            _ => {}
        }
    }

    scene_mapping
}
