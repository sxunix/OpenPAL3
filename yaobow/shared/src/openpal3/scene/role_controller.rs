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
    /// 走、跑用哪个动作，第一次走/跑时才找（`move_anim_name`）：每个角色建出来就把
    /// 两套动作读进来，一个场景要多读几十个 `.mv3`，Switch 上换场景要多等一秒。
    walking_anim_name: RefCell<Option<String>>,
    running_anim_name: RefCell<Option<String>>,
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
    /// 这个角色的动作网格用自己的材质，不和同模型的别人共用（战斗舞台上要单独染色）。
    unique_materials: Cell<bool>,
    /// How tall the model stands, in world units — the Y extent of the
    /// idle animation's first frame. Anything that has to sit above an
    /// actor's head (the 72 emoji bubble) needs it, and it differs a lot
    /// between characters: 景天 is 76, 龙葵 63, 花盈 28.
    model_height: Cell<f32>,
    model_radius: Cell<f32>,
    /// 每个动作的命名标记（`.mv3` 的 `Mv3ActionDesc`，秒 + 名字）。原版把它们当动画消息推给角色节点。
    anim_events: RefCell<HashMap<String, Vec<(f32, String)>>>,
    /// 这一遍播放里已经取走的标记（原版 `gbVisNode::PopMessage` 取一条少一条）。
    anim_events_taken: RefCell<Vec<String>>,
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

        let (model_height, model_radius) = mv3_model_extent(&asset_mgr, role_name, idle_anim_name);
        let model_height = model_height.unwrap_or(DEFAULT_MODEL_HEIGHT);
        let model_radius = model_radius.unwrap_or(DEFAULT_MODEL_RADIUS);
        // `role.cpp:357`：半径不到 6 的（小鸡那类）直接当成可重叠，不参与碰撞。
        let overlap = i32::from(model_radius < 6.);

        Self {
            entity,
            model_name: RefCell::new(role_name.to_string()),
            asset_mgr: asset_mgr.clone(),
            animations,
            active_anim_name: RefCell::new(idle_anim_name.to_string()),
            idle_anim_name: idle_anim_name.to_string(),
            walking_anim_name: RefCell::new(None),
            running_anim_name: RefCell::new(None),
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
            overlap: RefCell::new(overlap),
            texture_override: RefCell::new(None),
            unique_materials: Cell::new(false),
            model_height: Cell::new(model_height),
            model_radius: Cell::new(model_radius),
            anim_events: RefCell::new(HashMap::new()),
            anim_events_taken: RefCell::new(vec![]),
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
            mv3_model_extent(&self.asset_mgr, model_name, &self.idle_anim_name)
                .0
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
                        self.unique_materials.get(),
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
                        self.asset_mgr.load_role_anim_with_texture(
                            self.entity.clone(),
                            &self.model_name.borrow(),
                            &anim_name,
                            None,
                            self.unique_materials.get(),
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
        if !self.anim_events.borrow().contains_key(&anim_name) {
            let events = self
                .asset_mgr
                .mv3_events(&self.model_name.borrow(), &anim_name);
            self.anim_events
                .borrow_mut()
                .insert(anim_name.clone(), events);
        }
        self.anim_events_taken.borrow_mut().clear();
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

    /// 走（`run == false`）或跑的动作名，头一回用时按角色类型的候选表找、顺手读进来。
    /// 走和跑同理——怪物有自己的 z4，NPC 的「跑」原版就是拿走路顶的；都没有就站着。
    fn move_anim_name(&self, run: bool) -> String {
        let cached = if run {
            self.running_anim_name.borrow().clone()
        } else {
            self.walking_anim_name.borrow().clone()
        };
        if let Some(name) = cached {
            return name;
        }
        let role_name = self.model_name.borrow().clone();
        let set = ActionSet::of(&role_name);
        let mode = if run { MoveMode::Run } else { MoveMode::Walk };
        let found = self
            .asset_mgr
            .load_role_anim_first(self.entity.clone(), &role_name, mode.action_names(set))
            .map(|(name, anim)| {
                self.animations.insert(name.to_string(), anim);
                name.to_string()
            });
        let name = match found {
            Some(name) => name,
            None if run => self.move_anim_name(false),
            None => self.idle_anim_name.clone(),
        };
        if run {
            *self.running_anim_name.borrow_mut() = Some(name.clone());
        } else {
            *self.walking_anim_name.borrow_mut() = Some(name.clone());
        }
        name
    }

    pub fn run(&self) {
        if *self.state.borrow() != RoleState::Running {
            let name = self.move_anim_name(true);
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
            let name = self.move_anim_name(false);
            self.play_anim(&name, RoleAnimationRepeatMode::Loop);
            *self.state.borrow_mut() = RoleState::Walking;
        }
    }

    pub fn set_auto_play_idle(&self, auto_play_idle: bool) {
        *self.auto_play_idle.borrow_mut() = auto_play_idle;
    }

    /// 以后载的动作网格都用自己的材质（清掉已缓存的动作，下次 `play_anim` 重载）。
    pub fn set_unique_materials(&self) {
        if !self.unique_materials.replace(true) {
            self.animations.clear();
        }
    }

    /// 正在放的动作名（`play_anim` 最后一次设的）。
    pub fn active_anim_name(&self) -> String {
        self.active_anim_name.borrow().clone()
    }

    /// 现在这个动作播到了第几秒（循环的动作每圈从 0 起）。
    pub fn anim_time(&self) -> f32 {
        self.anim_now()
            .map_or(0., |a| a.inner::<AnimatedMeshComponent>().morph_time())
    }

    /// 动作的时间倍率（原版 `gbVisNode::SetTimeScale`，跳跃分 4 / 1 / 0 / 2 四档）。
    pub fn set_anim_time_scale(&self, scale: f32) {
        if let Some(anim) = self.anim_now() {
            anim.inner::<AnimatedMeshComponent>().set_time_scale(scale);
        }
    }

    /// `.mv3` 的「hold」标记要不要让动画自己停住。跳跃自己按标记调倍率，不用它。
    pub fn set_anim_hold_enabled(&self, enabled: bool) {
        if let Some(anim) = self.anim_now() {
            anim.inner::<AnimatedMeshComponent>()
                .set_hold_enabled(enabled);
        }
    }

    /// 取一条动画标记（`FUN_00537070`）：播到这个标记了就返回 `true`，同一遍播放里只返回一次。
    pub fn take_anim_event(&self, name: &str) -> bool {
        if self.anim_events_taken.borrow().iter().any(|n| n == name) {
            return false;
        }
        let Some(anim) = self.anim_now() else {
            return false;
        };
        let now = anim.inner::<AnimatedMeshComponent>().morph_time();
        let active = self.active_anim_name.borrow().clone();
        let reached = self
            .anim_events
            .borrow()
            .get(&active)
            .is_some_and(|events| events.iter().any(|(t, n)| n == name && *t <= now));
        if reached {
            self.anim_events_taken.borrow_mut().push(name.to_string());
        }
        reached
    }

    /// 现在在播的那个动作组件（没有就返回 `None`，和 [`Self::active_anim`] 不同，它不 unwrap）。
    fn anim_now(&self) -> Option<ComRc<IAnimatedMeshComponent>> {
        self.animations
            .get(&*self.active_anim_name.borrow())
            .map(|a| a.value().clone())
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

    /// 碰撞半径（角色 `+0x10`，`FUN_004125d0`），野外怪物撞主角时两人相加。
    pub fn model_radius(&self) -> f32 {
        self.model_radius.get()
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
    create_animated_mesh_from_mv3_with_texture(entity, component_factory, vfs, path, None, false)
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
    unique: bool,
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
        // `unique`：每个角色自己一份材质（要单独染色的，见 `set_unique_materials`）。
        let material = if unique {
            material.make_unique()
        } else {
            material
        };
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

/// 读不到模型时的碰撞半径。原版没有这条退路（`Role::Create` 总有模型），
/// 这里借游戏自己的 `ROLE_RADIUS`——`gamebase/roledef.h:327` 和
/// `navigationinfo.h:15` 都写着 `const float ROLE_RADIUS = 12.0f;//角色半径`，
/// 对话距离、跳跃距离、找路让位全按它算。
pub const DEFAULT_MODEL_RADIUS: f32 = 12.0;

/// 碰撞半径：`Role::Create`（`gamebase/role.cpp:355`，exe 里 `FUN_004125d0`）
///
/// ```cpp
/// gbOBBox box;  m_actor.GetCollideOBB(box);
/// radius = 0.5f*sqrtf(box.xlen*box.xlen + box.zlen*box.zlen);
/// radius *= 0.8f;
/// if (radius < 6) m_overlap = TRUE;   // 关闭小鸡等的碰撞
/// ```
///
/// `xlen` 是**边长**不是半边长：`gbOBBox` 的声明旁边注着
/// `center(0,0,0), width = xlen`（`engine/physics/gbcollide.h:41`），
/// `gbMVActorDef::GetCollideOBB` 也是直接写 `aabb.max.x − aabb.min.x`
/// （`engine/actor/gbmorphactor.cpp:131`）。
///
/// 一处还差着：原版取的是**整个动作所有帧**的包围盒
/// （`m_pCurAction->GetBoundBox()`），这里只量了第 0 帧。
fn mv3_model_extent(
    asset_mgr: &Rc<AssetManager>,
    role_name: &str,
    action: &str,
) -> (Option<f32>, Option<f32>) {
    let Some(frame) = asset_mgr
        .vfs()
        .read_to_end(asset_mgr.mv3_path(role_name, action))
        .ok()
        .and_then(|data| read_mv3(&mut Cursor::new(data)).ok())
        .and_then(|mv3| mv3.models.into_iter().next())
        .and_then(|m| m.frames.into_iter().next())
    else {
        return (None, None);
    };
    let (mut lx, mut hx, mut lz, mut hz) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
    let (mut ly, mut hy) = (f32::MAX, f32::MIN);
    for v in &frame.vertices {
        lx = lx.min(v.x as f32);
        hx = hx.max(v.x as f32);
        ly = ly.min(v.y as f32);
        hy = hy.max(v.y as f32);
        lz = lz.min(v.z as f32);
        hz = hz.max(v.z as f32);
    }
    // 高：站立动作第 0 帧的 Y 跨度。
    let height = (hy > ly).then(|| (hy - ly) * MV3_SCALE);
    let radius = (hx > lx && hz > lz).then(|| {
        let (dx, dz) = ((hx - lx) * MV3_SCALE, (hz - lz) * MV3_SCALE);
        0.4 * (dx * dx + dz * dz).sqrt()
    });
    (height, radius)
}
