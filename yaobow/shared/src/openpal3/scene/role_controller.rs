use crate::openpal3::asset_manager::AssetManager;
use crate::openpal3::comdef::IRoleController;
use crate::openpal3::states::role_move::{ActionSet, MoveMode};
use common::store_ext::StoreExt2;
use crosscom::ComRc;
use dashmap::DashMap;
use dashmap::mapref::one::Ref;
use fileformats::mv3::{Mv3Model, read_mv3};
use mini_fs::{MiniFs, StoreExt};
use radiance::comdef::{
    IAnimatedMeshComponent, IAnimatedMeshComponentExt, IComponent, IComponentImpl, IEntity,
    IEntityExt,
};
use radiance::components::mesh::{
    AnimatedMeshComponent, Geometry, MorphAnimationState, MorphTarget, TexCoord,
};
use radiance::math::Vec3;
use radiance::rendering::{ComponentFactory, MaterialDef, Pal3ActorMaterialDef};
use radiance::scene::CoreEntity;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;
use std::rc::Rc;

use super::error::EntityError;

/// Model units to world units for MV3 actors. 景天's `ROLE/101/c01.mv3`
/// spans 4863 model units vertically, which is 76 world units.
pub const MV3_SCALE: f32 = 0.01562;

#[derive(PartialEq, Copy, Clone, Debug)]
pub enum RoleAnimationRepeatMode {
    Repeat(i32),
    Loop,
    Hold,
}

#[derive(PartialEq, Copy, Clone, Debug)]
pub enum RoleState {
    PlayingAnimation,
    AnimationFinished,
    AnimationHolding,
    Idle,
    Walking,
    Running,
}

pub fn create_mv3_entity(
    asset_mgr: Rc<AssetManager>,
    role_name: &str,
    idle_anim: &str,
    name: String,
    visible: bool,
) -> Result<ComRc<IEntity>, EntityError> {
    let entity = CoreEntity::create(name, visible);

    entity.add_component(
        IRoleController::uuid(),
        crosscom::ComRc::from_object(RoleController::new(
            entity.clone(),
            asset_mgr.clone(),
            role_name,
            idle_anim,
        )?),
    );

    if let Some(shadow) = asset_mgr.build_role_shadow() {
        entity.attach(shadow);
    }

    Ok(entity)
}

pub struct RoleController {
    entity: ComRc<IEntity>,
    /// 42 设置龙葵形象要在场上换模型，所以它是可变的（原版改的是角色记录
    /// 里 +0xae/+0x2a8 那两份模型名）。
    model_name: RefCell<String>,
    asset_mgr: Rc<AssetManager>,
    animations: DashMap<String, ComRc<IAnimatedMeshComponent>>,
    active_anim_name: RefCell<String>,
    idle_anim_name: String,
    walking_anim_name: String,
    running_anim_name: String,
    anim_repeat_mode: RefCell<RoleAnimationRepeatMode>,
    is_active: RefCell<bool>,
    state: RefCell<RoleState>,
    auto_play_idle: RefCell<bool>,
    nav_layer: RefCell<usize>,
    proc_id: RefCell<i32>,
    // Patrol path loaded from the .scn role record (0x84/0x88 path, 0x80 mode,
    // 0x150 speed). Empty when the role has no path. Driven by the ADV director,
    // which has scene/nav access for ground-snapping.
    patrol_path: RefCell<Vec<Vec3>>,
    patrol_index: RefCell<usize>,
    patrol_forward: RefCell<bool>,
    patrol_mode: RefCell<u32>,
    patrol_speed: RefCell<f32>,
    // 205 RoleOverlap (`LAB_00437ae0`) stores its argument at role+0x5b0
    // and does nothing else. Kept here so the collision code can read it
    // once role-vs-role blocking exists.
    overlap: RefCell<i32>,
    // 116 SetRoleTexture. The texture is baked into the material when the
    // mesh is built, so an override means rebuilding: it invalidates the
    // animation cache and the active action is reloaded through it.
    texture_override: RefCell<Option<String>>,
    /// How tall the model stands, in world units — the Y extent of the
    /// idle animation's first frame. Anything that has to sit above an
    /// actor's head (the 72 emoji bubble) needs it, and it differs a lot
    /// between characters: 景天 is 76, 龙葵 63, 花盈 28.
    model_height: Cell<f32>,
}

ComObject_RoleController!(super::RoleController);

impl RoleController {
    pub fn new(
        entity: ComRc<IEntity>,
        asset_mgr: Rc<AssetManager>,
        role_name: &str,
        idle_anim: &str,
    ) -> Result<Self, EntityError> {
        let idle_anim = idle_anim;
        // 站立动画按角色类型排优先级：主角 c01、NPC z1/z2、怪物 z0 打头。
        // 实机 269 个角色里有 z0 的都同时有 z1，所以退路一定走得通。
        let mut candidates = vec![idle_anim];
        candidates.extend_from_slice(ActionSet::of(role_name).idle_names());
        let anim = asset_mgr
            .load_role_anim_first(entity.clone(), role_name, &candidates)
            .ok_or(EntityError::EntityAnimationNotFound)?;

        Ok(Self::new_from_idle_animation(
            entity, asset_mgr, role_name, anim.0, anim.1,
        ))
    }

    pub fn new_from_idle_animation(
        entity: ComRc<IEntity>,
        asset_mgr: Rc<AssetManager>,
        role_name: &str,
        idle_anim_name: &str,
        idle_anim: ComRc<IAnimatedMeshComponent>,
    ) -> Self {
        let animations = DashMap::new();
        if !idle_anim_name.trim().is_empty() {
            animations.insert(idle_anim_name.to_string(), idle_anim);
        }

        // 走和跑同理——怪物有自己的 z4，NPC 的「跑」原版就是拿走路顶的。
        let set = ActionSet::of(role_name);
        let walking_anim = asset_mgr.load_role_anim_first(
            entity.clone(),
            role_name,
            MoveMode::Walk.action_names(set),
        );
        let running_anim = asset_mgr.load_role_anim_first(
            entity.clone(),
            role_name,
            MoveMode::Run.action_names(set),
        );

        let walking_anim_name = walking_anim
            .map(|(name, _)| name)
            .unwrap_or(idle_anim_name)
            .to_string();

        let running_anim_name = running_anim
            .map(|(name, _)| name)
            .unwrap_or(walking_anim_name.as_str())
            .to_string();

        let model_height =
            mv3_model_height(&asset_mgr, role_name, idle_anim_name).unwrap_or(DEFAULT_MODEL_HEIGHT);

        Self {
            entity,
            model_name: RefCell::new(role_name.to_string()),
            asset_mgr: asset_mgr.clone(),
            animations,
            active_anim_name: RefCell::new(idle_anim_name.to_string()),
            idle_anim_name: idle_anim_name.to_string(),
            walking_anim_name,
            running_anim_name,
            anim_repeat_mode: RefCell::new(RoleAnimationRepeatMode::Repeat(1)),
            is_active: RefCell::new(false),
            state: RefCell::new(RoleState::Idle),
            auto_play_idle: RefCell::new(true),
            nav_layer: RefCell::new(0),
            proc_id: RefCell::new(0),
            patrol_path: RefCell::new(vec![]),
            patrol_index: RefCell::new(0),
            patrol_forward: RefCell::new(true),
            patrol_mode: RefCell::new(0),
            patrol_speed: RefCell::new(0.),
            overlap: RefCell::new(0),
            texture_override: RefCell::new(None),
            model_height: Cell::new(model_height),
        }
    }

    pub fn get_role_controller(entity: ComRc<IEntity>) -> Option<ComRc<IRoleController>> {
        entity
            .get_component(IRoleController::uuid())?
            .query_interface::<IRoleController>()
    }

    pub fn is_active(&self) -> bool {
        *self.is_active.borrow()
    }

    pub fn set_active(&self, active: bool) {
        *self.is_active.borrow_mut() = active;
        if active {
            let anim_name = { self.active_anim_name.borrow().clone() };
            let mode = *self.anim_repeat_mode.borrow();
            self.play_anim(&anim_name, mode);
        } else {
            self.entity.set_rendering_component(None);
        }

        self.entity.set_visible(active);
    }

    /// 205 RoleOverlap's argument. Nothing reads it yet — role-vs-role
    /// blocking does not exist — but the script's intent is recorded.
    /// 116 给角色更换纹理贴图 — `LAB_00437ff0` → `FUN_0041a150`, which
    /// builds `role\<model>\<name>.tga` and reloads the model's texture.
    /// The script passes the bare stem; the folder and extension are the
    /// engine's.
    pub fn set_texture_override(&self, texture: Option<String>) {
        if *self.texture_override.borrow() == texture {
            return;
        }
        *self.texture_override.borrow_mut() = texture;
        self.animations.clear();
        if self.is_active() {
            let anim_name = { self.active_anim_name.borrow().clone() };
            let mode = *self.anim_repeat_mode.borrow();
            self.play_anim(&anim_name, mode);
        }
    }

    /// 换掉这个角色用的模型，当场生效——42 设置龙葵形象走的就是这条。
    ///
    /// 原版 `FUN_00436b40` 把角色记录里的两份模型名（`+0xae` 和 `+0x2a8`）
    /// 改成 `"105"`（蓝）或 `"106"`（红），再调 `FUN_00412710` 按当前的动作
    /// 码重新加载一次，所以换装是立刻看得见的。这里照做：清掉动作缓存
    /// （里面的网格是按旧模型建的）、重新量一次身高，还在场上就把当前动作
    /// 重播一遍。
    ///
    /// 动作名不重算——原版也是拿同一个动作码去新模型上找。
    pub fn set_model_name(&self, model_name: &str) {
        if *self.model_name.borrow() == model_name {
            return;
        }
        *self.model_name.borrow_mut() = model_name.to_string();
        self.animations.clear();
        self.model_height.set(
            mv3_model_height(&self.asset_mgr, model_name, &self.idle_anim_name)
                .unwrap_or(DEFAULT_MODEL_HEIGHT),
        );
        if self.is_active() {
            let anim_name = { self.active_anim_name.borrow().clone() };
            let mode = *self.anim_repeat_mode.borrow();
            self.play_anim(&anim_name, mode);
        }
    }

    pub fn overlap(&self) -> i32 {
        *self.overlap.borrow()
    }

    pub fn set_overlap(&self, overlap: i32) {
        *self.overlap.borrow_mut() = overlap;
    }

    pub fn proc_id(&self) -> i32 {
        *self.proc_id.borrow()
    }

    pub fn set_proc_id(&self, proc_id: i32) {
        *self.proc_id.borrow_mut() = proc_id;
    }

    /// Start `anim_name`. Returns whether an animation was actually
    /// started: a name the model has no `.mv3` for leaves the controller
    /// untouched, and a caller that blocks on the action finishing (the
    /// 22 `RoleShowAction` command) has to know, or it waits for a state
    /// change that can never come.
    pub fn play_anim(&self, anim_name: &str, repeat_mode: RoleAnimationRepeatMode) -> bool {
        let anim_name = anim_name.to_lowercase();
        if anim_name.is_empty() {
            return false;
        }
        match self.animations.get(&anim_name) {
            Some(anim) => {
                self.play_anim_mesh_internal(
                    anim.key().to_string(),
                    anim.value().clone(),
                    repeat_mode,
                );
                true
            }
            _ => {
                let override_name = self.texture_override.borrow().clone();
                let anim = self
                    .asset_mgr
                    .load_role_anim_with_texture(
                        self.entity.clone(),
                        &self.model_name.borrow(),
                        &anim_name,
                        override_name.as_deref(),
                    )
                    .or_else(|| {
                        // A texture the script named but the data does
                        // not have would otherwise leave the actor with
                        // no mesh at all; fall back to its own.
                        if let Some(name) = &override_name {
                            log::warn!(
                                "SetRoleTexture: role\\{}\\{}.tga 读不到，用原贴图",
                                self.model_name.borrow(),
                                name
                            );
                        }
                        self.asset_mgr.load_role_anim(
                            self.entity.clone(),
                            &self.model_name.borrow(),
                            &anim_name,
                        )
                    });

                match anim {
                    Some(anim) => {
                        self.animations.insert(anim_name.to_string(), anim.clone());
                        self.play_anim_mesh_internal(anim_name, anim, repeat_mode);
                        true
                    }
                    None => {
                        log::warn!("角色 {} 没有动作 {}", self.model_name.borrow(), anim_name);
                        false
                    }
                }
            }
        }
    }

    pub fn play_anim_mesh(
        &self,
        anim_name: String,
        anim: ComRc<IAnimatedMeshComponent>,
        repeat_mode: RoleAnimationRepeatMode,
    ) {
        self.animations.insert(anim_name.clone(), anim.clone());
        self.play_anim_mesh_internal(anim_name, anim, repeat_mode);
    }

    fn play_anim_mesh_internal(
        &self,
        anim_name: String,
        anim: ComRc<IAnimatedMeshComponent>,
        repeat_mode: RoleAnimationRepeatMode,
    ) {
        *self.active_anim_name.borrow_mut() = anim_name;
        *self.anim_repeat_mode.borrow_mut() = repeat_mode;
        *self.state.borrow_mut() = RoleState::PlayingAnimation;

        // The MV3 "hold" marker is an SCE-driven pause point used only by the
        // `RoleShowAction` hold mode (`-2`), which freezes the actor mid-action
        // until `RoleEndAction` resumes it. Looping/repeating actions (idle,
        // walk, run) must ignore the marker, otherwise they stutter at it on
        // every cycle and never reach their final frames cleanly.
        anim.set_hold_enabled(matches!(repeat_mode, RoleAnimationRepeatMode::Hold));

        // Looping clips (idle/walk/run) wrap rather than end: their final
        // keyframe is a loop-seam frame (often a distinct pose from frame 0 in
        // run cycles), so it must not be presented as a resting frame or the
        // actor flashes a wrong pose each cycle. Only one-shot/hold playback
        // settles on the true final frame.
        anim.set_loop_playback(matches!(repeat_mode, RoleAnimationRepeatMode::Loop));

        self.entity.add_component(
            IAnimatedMeshComponent::uuid(),
            anim.query_interface::<IComponent>().unwrap(),
        );
        self.replay_anim();
    }

    pub fn continue_anim(&self) {
        self.active_anim().value().play(false);
        *self.state.borrow_mut() = RoleState::PlayingAnimation;
    }

    pub fn replay_anim(&self) {
        self.active_anim().value().play(true);
    }

    pub fn run(&self) {
        if *self.state.borrow() != RoleState::Running {
            let name = self.running_anim_name.clone();
            self.play_anim(&name, RoleAnimationRepeatMode::Loop);
            *self.state.borrow_mut() = RoleState::Running;
        }
    }

    pub fn idle(&self) {
        if *self.state.borrow() != RoleState::Idle {
            let name = self.idle_anim_name.clone();
            self.play_anim(&name, RoleAnimationRepeatMode::Loop);
            *self.state.borrow_mut() = RoleState::Idle;
        }
    }

    pub fn walk(&self) {
        if *self.state.borrow() != RoleState::Walking {
            let name = self.walking_anim_name.clone();
            self.play_anim(&name, RoleAnimationRepeatMode::Loop);
            *self.state.borrow_mut() = RoleState::Walking;
        }
    }

    pub fn set_auto_play_idle(&self, auto_play_idle: bool) {
        *self.auto_play_idle.borrow_mut() = auto_play_idle;
    }

    pub fn state(&self) -> RoleState {
        *self.state.borrow()
    }

    pub fn repeat_mode(&self) -> RoleAnimationRepeatMode {
        *self.anim_repeat_mode.borrow()
    }

    pub fn nav_layer(&self) -> usize {
        *self.nav_layer.borrow()
    }

    /// The model's height in world units; see [`Self::model_height`]'s
    /// field docs.
    pub fn model_height(&self) -> f32 {
        self.model_height.get()
    }

    pub fn model_name(&self) -> String {
        self.model_name.borrow().clone()
    }

    pub fn switch_nav_layer(&self) -> usize {
        *self.nav_layer.borrow_mut() = (self.nav_layer() + 1) % 2;
        self.nav_layer()
    }

    pub fn set_nav_layer(&self, layer: usize) {
        *self.nav_layer.borrow_mut() = layer;
    }

    /// Install a patrol path (scene `.scn` role fields). `mode` is the raw 0x80
    /// path-mode value; its low byte selects loop (0) vs ping-pong (non-zero).
    /// A path with fewer than two waypoints is ignored.
    pub fn set_patrol(&self, path: Vec<Vec3>, mode: u32, speed: f32) {
        if path.len() < 2 {
            return;
        }
        *self.patrol_path.borrow_mut() = path;
        *self.patrol_index.borrow_mut() = 0;
        *self.patrol_forward.borrow_mut() = true;
        *self.patrol_mode.borrow_mut() = mode;
        *self.patrol_speed.borrow_mut() = speed;
    }

    pub fn has_patrol(&self) -> bool {
        self.patrol_path.borrow().len() >= 2
    }

    pub fn patrol_speed(&self) -> f32 {
        *self.patrol_speed.borrow()
    }

    /// Current patrol target waypoint, if any.
    pub fn patrol_target(&self) -> Option<Vec3> {
        let path = self.patrol_path.borrow();
        path.get(*self.patrol_index.borrow()).copied()
    }

    /// Whether the patrol loops (`true`) or ping-pongs (`false`). Loop when the
    /// path-mode low byte is zero.
    fn patrol_loops(&self) -> bool {
        (*self.patrol_mode.borrow() & 0xff) == 0
    }

    /// Advance to the next patrol waypoint, honoring loop vs ping-pong.
    pub fn advance_patrol(&self) {
        let len = self.patrol_path.borrow().len();
        if len < 2 {
            return;
        }
        let mut index = self.patrol_index.borrow_mut();
        if self.patrol_loops() {
            *index = (*index + 1) % len;
        } else {
            let mut forward = self.patrol_forward.borrow_mut();
            if *forward {
                if *index + 1 >= len - 1 {
                    *index = len - 1;
                    *forward = false;
                } else {
                    *index += 1;
                }
            } else if *index <= 1 {
                *index = 0;
                *forward = true;
            } else {
                *index -= 1;
            }
        }
    }

    fn active_anim(&self) -> Ref<'_, String, ComRc<IAnimatedMeshComponent>> {
        self.animations
            .get(&*self.active_anim_name.borrow())
            .unwrap()
    }
}

impl IComponentImpl for RoleController {
    fn on_loading(&self) -> crosscom::Void {
        if !self.idle_anim_name.trim().is_empty() && self.is_active() {
            self.idle();
        }
    }

    fn on_unloading(&self) {}

    fn on_updating(&self, _delta_sec: f32) -> crosscom::Void {
        if self.is_active() {
            if self.active_anim().value().morph_animation_state() == MorphAnimationState::Finished {
                self.state.replace(RoleState::AnimationFinished);
                let mode = *self.anim_repeat_mode.borrow();
                match mode {
                    RoleAnimationRepeatMode::Loop => {
                        self.replay_anim();
                    }
                    RoleAnimationRepeatMode::Repeat(count) => {
                        if count - 1 > 0 {
                            *self.anim_repeat_mode.borrow_mut() =
                                RoleAnimationRepeatMode::Repeat(count - 1);
                            self.replay_anim();
                        } else {
                            if *self.auto_play_idle.borrow() {
                                self.idle();
                            }
                        }
                    }
                    RoleAnimationRepeatMode::Hold => {
                        if *self.auto_play_idle.borrow() {
                            self.idle();
                        }
                    }
                }
            } else if self.active_anim().value().morph_animation_state()
                == MorphAnimationState::Holding
            {
                let mode = *self.anim_repeat_mode.borrow();
                match mode {
                    RoleAnimationRepeatMode::Loop | RoleAnimationRepeatMode::Repeat(_) => {
                        self.continue_anim();
                    }
                    RoleAnimationRepeatMode::Hold => {
                        self.state.replace(RoleState::AnimationHolding);
                    }
                }
            }
        }
    }
}

pub fn create_animated_mesh_from_mv3<P: AsRef<Path>>(
    entity: ComRc<IEntity>,
    component_factory: &Rc<dyn ComponentFactory>,
    vfs: &MiniFs,
    path: P,
) -> anyhow::Result<ComRc<IAnimatedMeshComponent>> {
    create_animated_mesh_from_mv3_with_texture(entity, component_factory, vfs, path, None)
}

/// As [`create_animated_mesh_from_mv3`], but with every model's texture
/// replaced by `<mv3 dir>/<texture>.tga` — opcode 116's
/// `role\%s\%s.tga` (format string at 0x5889d4), whose first `%s` is
/// the model directory the `.mv3` already lives in.
pub fn create_animated_mesh_from_mv3_with_texture<P: AsRef<Path>>(
    entity: ComRc<IEntity>,
    component_factory: &Rc<dyn ComponentFactory>,
    vfs: &MiniFs,
    path: P,
    texture: Option<&str>,
) -> anyhow::Result<ComRc<IAnimatedMeshComponent>> {
    let mv3file = read_mv3(&mut Cursor::new(vfs.read_to_end(&path)?))?;
    let mut frames = vec![];

    for model_index in 0..mv3file.models.len() {
        let model = &mv3file.models[model_index];
        let mut texture_path = path.as_ref().to_owned();
        texture_path.pop();
        let texture_index = if model_index < mv3file.texture_count as usize {
            model_index
        } else {
            0
        };

        match texture {
            Some(name) => texture_path.push(format!("{name}.tga")),
            None => texture_path.push(
                mv3file.textures[texture_index].names[0]
                    .to_string()
                    .unwrap(),
            ),
        }

        // MV3 actor textures (PAL3 roles) typically rely on alpha cutout
        // for hair / eye fringes. Keep the default `BlendMode::AlphaTest`.
        // Use the dynamically-lit material so scene lights (`.lgt`) shade the
        // actor per-pixel; the geometry builder supplies per-frame normals.
        let material = Pal3ActorMaterialDef::create(texture_path.to_str().unwrap(), |name| {
            vfs.open(name).ok()
        });
        for mesh_index in 0..model.mesh_count as usize {
            frames.push(create_geometry_frames(model, mesh_index, &material))
        }
    }

    let anim_timestamps: Vec<u32> = mv3file.models[0]
        .frames
        .iter()
        .map(|f| f.timestamp)
        .collect();

    let mut morph_targets: Vec<MorphTarget> = Vec::<MorphTarget>::with_capacity(frames.len());
    for frame_index in 0..mv3file.models[0].frame_count as usize {
        let mut geometries = vec![];
        for mesh_index in 0..frames.len() {
            geometries.push(frames[mesh_index][frame_index].clone());
        }

        morph_targets.push(MorphTarget::new(
            geometries,
            anim_timestamps[frame_index] as f32 / 4580.,
            component_factory.clone(),
        ));
    }

    let mut hold_time = 0.;
    for anim in mv3file.action_desc {
        if anim.name.as_str()?.starts_with("hold") {
            hold_time = anim.tick as f32 / 4580.;
        }
    }

    let animated_mesh = AnimatedMeshComponent::new(entity, component_factory.clone());
    animated_mesh.set_morph_targets(morph_targets, hold_time);

    Ok(ComRc::from_object(animated_mesh))
}

fn create_geometry_frames(
    model: &Mv3Model,
    mesh_index: usize,
    material: &MaterialDef,
) -> Vec<Geometry> {
    let mesh = &model.meshes[mesh_index];
    let hash = |index, texcoord_index| index as u32 * model.texcoord_count + texcoord_index as u32;

    // Smooth normals are computed per frame over the ORIGINAL (un-split) MV3
    // vertices using the original triangle connectivity, then shared across the
    // texcoord-split copies below. Computing them after the split would give
    // each UV-seam copy only its own chart's faces, producing faceted, frame-
    // varying seam normals that make the actor's lighting shimmer/"swim" during
    // animation. MV3 winding is consistent (verified), so geometric smooth
    // normals are reliable. (The MV3's own packed `normal_phi`/`normal_theta`
    // are an alternative, but recomputing avoids that encoding entirely.)
    let frame_normals: Vec<Vec<Vec3>> = (0..model.frame_count as usize)
        .map(|k| compute_mv3_frame_normals(&model.frames[k].vertices, mesh))
        .collect();

    let mut indices: Vec<u32> = Vec::<u32>::with_capacity(model.vertex_per_frame as usize);
    let mut vertices = vec![vec![]; model.frame_count as usize];
    let mut normals = vec![vec![]; model.frame_count as usize];
    let mut texcoord = vec![vec![]; model.frame_count as usize];
    let mut index_map = HashMap::new();

    for t in &mesh.triangles {
        for (&i, &j) in t.indices.iter().zip(&t.texcoord_indices) {
            let h = hash(i, j);
            let index = match index_map.get(&h) {
                None => {
                    let index = index_map.len();
                    for k in 0..model.frame_count as usize {
                        let frame = &model.frames[k];
                        vertices[k].push(Vec3::new(
                            frame.vertices[i as usize].x as f32 * MV3_SCALE,
                            frame.vertices[i as usize].y as f32 * MV3_SCALE,
                            frame.vertices[i as usize].z as f32 * MV3_SCALE,
                        ));

                        // Share the original vertex's smooth normal across every
                        // texcoord-split copy so UV seams stay seamless.
                        normals[k].push(frame_normals[k][i as usize]);

                        if (j as u32) < model.texcoord_count {
                            texcoord[k].push(TexCoord::new(
                                model.texcoords[j as usize].u,
                                -model.texcoords[j as usize].v,
                            ));
                        } else {
                            texcoord[k].push(TexCoord::new(0., 0.));
                        }
                    }
                    index_map.insert(h, index as u32);
                    index as u32
                }
                Some(index) => *index,
            };

            indices.push(index);
        }
    }

    let mut geometries = vec![];
    for i in 0..model.frame_count as usize {
        geometries.push(Geometry::new(
            &vertices[i],
            Some(&normals[i]),
            &vec![texcoord[i].clone()],
            indices.clone(),
            material.clone(),
        ))
    }

    geometries
}

/// Compute per-vertex smooth normals for one MV3 mesh of a single frame, indexed
/// by the **original** MV3 vertex index (the entry for vertex `i` corresponds to
/// `frame_vertices[i]`). Positions use the loader's stored convention (the parser
/// already negates x/z), so the resulting normals live in the same space as the
/// geometry the shader transforms. Area-weighted (un-normalized cross product)
/// accumulation, then normalized; degenerate vertices fall back to +Y.
fn compute_mv3_frame_normals(
    frame_vertices: &[fileformats::mv3::Mv3Vertex],
    mesh: &fileformats::mv3::Mv3Mesh,
) -> Vec<Vec3> {
    let pos = |idx: usize| {
        Vec3::new(
            frame_vertices[idx].x as f32,
            frame_vertices[idx].y as f32,
            frame_vertices[idx].z as f32,
        )
    };

    let mut normals = vec![Vec3::new(0.0, 0.0, 0.0); frame_vertices.len()];
    for t in &mesh.triangles {
        let (a, b, c) = (
            t.indices[0] as usize,
            t.indices[1] as usize,
            t.indices[2] as usize,
        );
        let e1 = Vec3::sub(&pos(b), &pos(a));
        let e2 = Vec3::sub(&pos(c), &pos(a));
        let fnormal = Vec3::cross(&e1, &e2);
        for &idx in &[a, b, c] {
            normals[idx] = Vec3::add(&normals[idx], &fnormal);
        }
    }
    for n in &mut normals {
        if Vec3::dot(n, n) > 1e-12 {
            *n = Vec3::normalized(n);
        } else {
            *n = Vec3::new(0.0, 1.0, 0.0);
        }
    }
    normals
}

/// Fallback when the `.mv3` cannot be measured — roughly a main
/// character's height.
pub const DEFAULT_MODEL_HEIGHT: f32 = 76.0;

/// Measure a role model by the Y extent of its idle animation's first
/// frame. Cheap enough to do once per role; the file is already in the
/// package cache by the time this runs.
fn mv3_model_height(asset_mgr: &Rc<AssetManager>, role_name: &str, action: &str) -> Option<f32> {
    let path = asset_mgr.mv3_path(role_name, action);
    let data = asset_mgr.vfs().read_to_end(&path).ok()?;
    let mv3 = read_mv3(&mut Cursor::new(data)).ok()?;
    let frame = mv3.models.first()?.frames.first()?;
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for v in &frame.vertices {
        lo = lo.min(v.y as f32);
        hi = hi.max(v.y as f32);
    }
    (hi > lo).then(|| (hi - lo) * MV3_SCALE)
}
