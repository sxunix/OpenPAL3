use std::{
    cell::{Ref, RefCell, RefMut},
    rc::Rc,
};

use crate::{
    openpal3::states::footstep,
    openpal3::states::role_move::{MoveMode, player_mode},
    openpal3::{
        asset_manager::AssetManager,
        directors::SceneManagerExtensions,
        scene::{LadderTestResult, RoleController},
        states::{global_state::GlobalState, persistent_state::PersistentState},
    },
    scripting::sce::vm::{SceExecutionOptions, SceVm},
    utils::{get_camera_rotation, get_moving_direction_and_throw},
};

use crosscom::ComRc;
use log::debug;
use radiance::{
    audio::AudioEngine,
    comdef::{IDirector, IDirectorImpl, IEntityExt, ISceneExt, ISceneManager},
    input::{InputEngine, Key},
    math::Vec3,
    radiance::UiManager,
};

use crate::agent_common::AgentBridge;

pub struct AdventureDirector {
    props: RefCell<AdventureDirectorProps>,
}

ComObject_AdventureDirector!(super::AdventureDirector);

impl AdventureDirector {
    pub fn new(
        app_name: &str,
        asset_mgr: Rc<AssetManager>,
        audio_engine: Rc<dyn AudioEngine>,
        input_engine: Rc<RefCell<dyn InputEngine>>,
        ui: Rc<UiManager>,
        scene_manager: ComRc<ISceneManager>,
        sce_vm_options: Option<SceExecutionOptions>,
        dialog_renderer: ComRc<crate::openpal3::comdef::IPal3DialogRenderer>,
        status_renderer: ComRc<crate::openpal3::comdef::IPal3StatusRenderer>,
    ) -> Self {
        let p_state = Rc::new(RefCell::new(PersistentState::new(app_name.to_string())));
        let global_state = GlobalState::new(asset_mgr.clone(), audio_engine.clone(), p_state);
        let mut sce_vm = SceVm::new(
            audio_engine.clone(),
            input_engine.clone(),
            ui,
            scene_manager.clone(),
            asset_mgr.load_init_sce(),
            "init".to_string(),
            asset_mgr.clone(),
            global_state,
            sce_vm_options,
            dialog_renderer,
            status_renderer,
        );
        sce_vm.call_proc(51);
        log::info!("AdventureDirector: init.sce loaded, proc 51 queued");

        Self {
            props: RefCell::new(AdventureDirectorProps {
                input_engine,
                scene_manager,
                sce_vm,
                camera_rotation: 0.,
                scripted_camera_seeded: false,
                layer_switch_triggered: false,
                agent_bridge: None,
                snow: crate::openpal3::scene::Snow::new(),
                snow_entity: None,
                snow_failed: false,
                footstep: None,
                cave_wolf: None,
                water_magic: None,
                water_magic_failed: false,
                flexeff: vec![],
                flexeff_cyl: vec![],
                flexeff_par: vec![],
                flexeff_audio: vec![],
                flexeff_pending: vec![],
                flexeff_seq: 0,
            }),
        }
    }

    fn props_mut(&self) -> RefMut<'_, AdventureDirectorProps> {
        self.props.borrow_mut()
    }

    pub fn load(
        app_name: &str,
        asset_mgr: Rc<AssetManager>,
        audio_engine: Rc<dyn AudioEngine>,
        input_engine: Rc<RefCell<dyn InputEngine>>,
        ui: Rc<UiManager>,
        scene_manager: ComRc<ISceneManager>,
        sce_vm_options: Option<SceExecutionOptions>,
        slot: i32,
        dialog_renderer: ComRc<crate::openpal3::comdef::IPal3DialogRenderer>,
        status_renderer: ComRc<crate::openpal3::comdef::IPal3StatusRenderer>,
    ) -> Option<Self> {
        let p_state = PersistentState::load(app_name, slot);
        let scene_name = p_state.scene_name();
        let sub_scene_name = p_state.sub_scene_name();
        if scene_name.is_none() || sub_scene_name.is_none() {
            log::error!("Cannot load save {}: scene or sub_scene is empty", slot);
            return None;
        }

        let scene = asset_mgr.load_scn(
            scene_name.as_ref().unwrap(),
            sub_scene_name.as_ref().unwrap(),
        );

        scene_manager.push_scene(scene);

        // 存档里记着红龙葵的话，场景刚建好的蓝龙葵要换回来。
        crate::scripting::sce::commands::apply_longkui_model(
            &scene_manager,
            p_state.longkui_mode(),
        );

        let mut global_state = GlobalState::new(
            asset_mgr.clone(),
            audio_engine.clone(),
            Rc::new(RefCell::new(p_state)),
        );

        // The role id should be saved in persistant state
        let role_entity = scene_manager
            .scn_scene()
            .unwrap()
            .inner::<crate::openpal3::scene::ScnScene>()
            .get_role_entity(0)
            .unwrap();

        let role = RoleController::get_role_controller(role_entity.clone()).unwrap();
        role.inner::<RoleController>().set_active(true);
        role_entity
            .transform()
            .borrow_mut()
            .set_position(&global_state.persistent_state_mut().position());

        global_state.play_default_bgm();

        let mut sce_vm = SceVm::new(
            audio_engine.clone(),
            input_engine.clone(),
            ui,
            scene_manager.clone(),
            asset_mgr.load_sce(scene_name.as_ref().unwrap()),
            scene_name.as_ref().unwrap().clone(),
            asset_mgr.clone(),
            global_state,
            sce_vm_options,
            dialog_renderer,
            status_renderer,
        );

        // don't draw curtain when loading a save
        sce_vm.state_mut().set_curtain(0.);
        sce_vm.state_mut().try_call_proc_by_name(&format!(
            "_{}_{}",
            scene_name.as_ref().unwrap(),
            sub_scene_name.as_ref().unwrap()
        ));

        Some(Self {
            props: RefCell::new(AdventureDirectorProps {
                input_engine,
                scene_manager: scene_manager.clone(),
                sce_vm,
                camera_rotation: 0.,
                scripted_camera_seeded: false,
                layer_switch_triggered: false,
                agent_bridge: None,
                snow: crate::openpal3::scene::Snow::new(),
                snow_entity: None,
                snow_failed: false,
                footstep: None,
                cave_wolf: None,
                water_magic: None,
                water_magic_failed: false,
                flexeff: vec![],
                flexeff_cyl: vec![],
                flexeff_par: vec![],
                flexeff_audio: vec![],
                flexeff_pending: vec![],
                flexeff_seq: 0,
            }),
        })
    }

    pub fn sce_vm(&self) -> Ref<'_, SceVm> {
        Ref::map(self.props.borrow(), |p| &p.sce_vm)
    }

    pub fn sce_vm_mut(&self) -> RefMut<'_, SceVm> {
        RefMut::map(self.props.borrow_mut(), |p| &mut p.sce_vm)
    }

    /// Install the agent bridge so this director honors pause/step
    /// and exposes its tick rate to `/v1/state`. Called by
    /// `Pal3Service::set_agent_bridge` after the agent server is
    /// booted (so menus exit and a fresh `AdventureDirector` gets the
    /// bridge before its first tick).
    pub fn set_agent_bridge(&self, bridge: Rc<AgentBridge>) {
        self.props.borrow_mut().agent_bridge = Some(bridge);
    }

    /// Currently-installed bridge, if any. Used by `Pal3Service`'s
    /// agent dispatcher to inspect/replace it when a new director is
    /// pushed onto the scene manager.
    pub fn agent_bridge(&self) -> Option<Rc<AgentBridge>> {
        self.props.borrow().agent_bridge.clone()
    }

    /// Resolve the currently-controlled role entity (player slot
    /// returned by `GlobalState::role_controlled`). `None` when no
    /// scene is mounted or the role cannot be resolved.
    pub fn controlled_role_position(&self) -> Option<Vec3> {
        let p = self.props.borrow();
        if p.scene_manager.scene().is_none() {
            return None;
        }
        let role = p.scene_manager.get_resolved_role(p.sce_vm.state(), -1)?;
        Some(role.transform().borrow().position())
    }

    /// Run an SCE proc by id, as the debug overlay's "Execute" button
    /// does. Exposed so an automation driver can reach a scripted beat
    /// without playing up to it — the only way to exercise commands
    /// whose effect is on screen.
    pub fn call_sce_proc(&self, proc_id: u32) {
        self.props.borrow_mut().sce_vm.call_proc(proc_id);
    }

    /// Start a battle straight away, the way opcode 73/80 would.
    ///
    /// A debug poke for the agent server: every scripted battle sits
    /// behind a cutscene, so verifying the combat stage otherwise means
    /// playing through one. `boss` picks the 80 (fixed line-up) path
    /// over 73 (random encounter).
    pub fn start_debug_combat(&self, ids: [i32; 6], boss: bool) {
        let mut p = self.props.borrow_mut();
        let scene_manager = p.scene_manager.clone();
        let cmd: Box<dyn crate::scripting::sce::SceCommand> = if boss {
            Box::new(crate::scripting::sce::commands::SceCommandCombat::new_boss(
                ids[0], ids[1], ids[2], ids[3], ids[4], ids[5],
            ))
        } else {
            Box::new(crate::scripting::sce::commands::SceCommandCombat::new(
                ids[0], ids[1], ids[2], ids[3],
            ))
        };
        p.sce_vm.push_debug_command(cmd, scene_manager);
    }

    /// Point the camera at `target` from `eye` and hold it there.
    ///
    /// Parks the polar camera in "a script has it" mode — following off,
    /// focus left on the player — which is the one combination neither
    /// the follow rig nor `apply_scripted_camera` writes to, so the pose
    /// survives until something else claims the camera.
    /// Read and clear "147 asked for the title screen". The host polls
    /// this once a frame from outside the director's own update.
    pub fn take_title_request(&self) -> bool {
        self.props
            .borrow_mut()
            .sce_vm
            .state_mut()
            .take_title_request()
    }

    pub fn set_camera_pose(&self, eye: Vec3, target: Vec3) -> bool {
        let p = self.props.borrow();
        let Some(scene) = p.scene_manager.scene() else {
            return false;
        };
        scene
            .camera_mut()
            .transform_mut()
            .set_position(&eye)
            .look_at(&target);
        drop(p);

        let mut p = self.props.borrow_mut();
        let camera = p.sce_vm.global_state_mut().camera_mut();
        camera.set_focus(crate::openpal3::states::camera::Focus::Player);
        camera.set_follow_player(false);
        let (yaw, pitch, distance) =
            crate::openpal3::states::camera::polar_of(&Vec3::sub(&eye, &target));
        camera.set_yaw(yaw);
        camera.set_pitch(pitch);
        camera.set_distance(distance);
        camera.set_look_at(target);
        true
    }

    /// Swap to `scene`/`sub_scene` immediately, the way the debug
    /// overlay's "Load" button does. Exposed for the same reason as
    /// [`Self::call_sce_proc`]: a scripted beat in another scene cannot
    /// be reached any other way without playing to it.
    pub fn load_scene(&self, scene: &str, sub_scene: &str) -> bool {
        {
            // A debug jump must not be able to kill the game: the scene
            // loader unwraps its file open, so a name that does not
            // exist would panic rather than come back as an error.
            let p = self.props.borrow();
            let path = p.sce_vm.state().asset_mgr().scn_path(scene, sub_scene);
            if mini_fs::StoreExt::open(p.sce_vm.state().asset_mgr().vfs(), &path).is_err() {
                log::warn!("load_scene: 没有 {}", path.display());
                return false;
            }
        }
        let mut p = self.props.borrow_mut();
        let scene_manager = p.scene_manager.clone();
        let command = crate::scripting::sce::commands::SceCommandLoadScene::new(
            scene.to_string(),
            sub_scene.to_string(),
        );
        p.sce_vm.abort_active_commands();
        command.load_for_debug(scene_manager, p.sce_vm.state_mut());
        true
    }

    /// Set a script global, the way the debug overlay's "Main Story"
    /// box does. Negative indices are globals; see `commands::vars`.
    /// Flip 152's water effect, for checking it without playing the
    /// cutscene that turns it on.
    /// Queue flexeff group `group` at `position`, the way 143 and combat
    /// do, so a driver can time screenshots against a known start.
    pub fn play_effect(&self, group: i32, position: Vec3) {
        self.props
            .borrow_mut()
            .sce_vm
            .state_mut()
            .play_effect(group, position);
    }

    pub fn set_water_magic(&self, on: bool) {
        self.props
            .borrow_mut()
            .sce_vm
            .state_mut()
            .set_water_magic(on);
    }

    /// Write an actor's tint alpha, for checking the 135/136 fades.
    /// Returns how many render objects it reached, or `None` if the scene
    /// has no such role.
    pub fn set_role_alpha(&self, role_id: i32, alpha: f32) -> Option<usize> {
        let props = self.props.borrow();
        let scene = props.scene_manager.scn_scene()?;
        // -1 is "whoever the player is driving", as everywhere else in
        // the script layer.
        let role_id = if role_id == -1 {
            props.sce_vm.global_state().role_controlled()
        } else {
            role_id
        };
        let entity = scene
            .inner::<crate::openpal3::scene::ScnScene>()
            .get_role_entity(role_id)?;
        Some(crate::scripting::sce::commands::role_fade::set_alpha(
            &entity, alpha,
        ))
    }

    pub fn set_script_global(&self, var: i16, value: i32) {
        self.props
            .borrow_mut()
            .sce_vm
            .global_state_mut()
            .persistent_state_mut()
            .set_global(var, value);
    }

    /// Teleport the leader (the role returned by `role_controlled`)
    /// to `pos`. Mirrors the agent server's `/v1/player/teleport`
    /// semantics. Returns `false` when no scene/role is available.
    pub fn teleport_controlled_role(&self, pos: Vec3) -> bool {
        let p = self.props.borrow();
        if p.scene_manager.scene().is_none() {
            return false;
        }
        let role = match p.scene_manager.get_resolved_role(p.sce_vm.state(), -1) {
            Some(r) => r,
            None => return false,
        };
        role.transform().borrow_mut().set_position(&pos);
        drop(p);
        // Mirror to persistent state so a subsequent save records the
        // teleported position.
        let mut pm = self.props.borrow_mut();
        pm.sce_vm
            .global_state_mut()
            .persistent_state_mut()
            .set_position(pos);
        true
    }
}

impl IDirectorImpl for AdventureDirector {
    fn activate(&self) {
        log::info!("AdventureDirector activated");
    }

    fn update(&self, delta_sec: f32) -> Option<ComRc<IDirector>> {
        // Honor `/v1/time/pause` + `/v1/time/step`: when an agent
        // bridge is installed and we're paused with no pending steps,
        // skip the entire SCE tick so movement, animations, and the
        // VM all freeze in lockstep. Stepped frames use the
        // bridge-provided fixed `dt`.
        //
        // The same borrow also reads `/v1/time/fast_forward`, which we
        // push onto the SCE state so commands can skip input-blocked
        // waits (dialog / movie) and collapse timed tweens this frame.
        let (effective_dt, fast_forward) = match self.props.borrow().agent_bridge.as_ref() {
            Some(bridge) => match bridge.effective_dt(delta_sec) {
                (true, dt) => (dt, bridge.fast_forward.get()),
                (false, _) => return None,
            },
            None => (delta_sec, false),
        };
        let mut props = self.props_mut();
        props.sce_vm.state_mut().set_fast_forward(fast_forward);
        props.do_update(effective_dt)
    }

    fn deactivate(&self) {}
}

struct AdventureDirectorProps {
    input_engine: Rc<RefCell<dyn InputEngine>>,
    scene_manager: ComRc<ISceneManager>,
    sce_vm: SceVm,
    camera_rotation: f32,
    /// Whether the polar camera's angles have been taken from the live
    /// camera since a script last focused it somewhere. Reset when the
    /// camera goes back to following the player.
    scripted_camera_seeded: bool,
    layer_switch_triggered: bool,
    /// Agent-server bridge. `None` when the embedded HTTP listener
    /// is not running; `Some(_)` gates pause/step + provides the
    /// synthetic-input overlay backing `/v1/input/*`.
    agent_bridge: Option<Rc<AgentBridge>>,
    /// 140 下雪. The simulation always exists; the entity is built the
    /// first time a script turns the weather on, and hidden rather than
    /// removed when it turns it off.
    snow: crate::openpal3::scene::Snow,
    snow_entity: Option<ComRc<radiance::comdef::IEntity>>,
    snow_failed: bool,
    /// 脚步声：当前循环播着的那一条，和它的文件名。原版一次只有一条
    /// （`DAT_00c116f4`），换了就 Stop 旧的 Play 新的。
    footstep: Option<(
        String,
        Rc<RefCell<Box<dyn radiance::audio::AudioMemorySource>>>,
    )>,
    /// 112 山洞之狼：正在进行的那一局，没在玩就是 `None`。
    cave_wolf: Option<crate::openpal3::states::cave_wolf::CaveWolf>,
    /// 152 水特效：`.par` 跑起来的模拟和它每个 group 的网格。
    water_magic: Option<(
        crate::openpal3::scene::ParticleEffect,
        Vec<ComRc<radiance::comdef::IEntity>>,
    )>,
    water_magic_failed: bool,
    /// 143 播出去的战斗特效：`quad` 那一支的模型加它画成的实体。
    /// 其余类（meteor / audio / callback …）还没做。
    flexeff: Vec<(
        crate::openpal3::scene::flexeff_quad::QuadEffect,
        ComRc<radiance::comdef::IEntity>,
    )>,
    /// `cylinder` 那一支。
    flexeff_cyl: Vec<(
        crate::openpal3::scene::flexeff_cylinder::CylinderEffect,
        ComRc<radiance::comdef::IEntity>,
    )>,
    /// flexeff 的 `particle` 类：`.par` 粒子，挂在特效位置上。
    flexeff_par: Vec<FlexParticle>,
    /// `audio` 那一支——没有画面，只是到点放几个 wav。
    flexeff_audio: Vec<crate::openpal3::scene::flexeff_audio::AudioEffect>,
    /// `callback` 排下来、还在等延迟的特效组：`(组号, 位置, 还差几秒, 深度)`。
    flexeff_pending: Vec<(i32, Vec3, f32, u8)>,
    /// 实体名的流水号，免得同名。
    flexeff_seq: usize,
}

/// `callback` 链的展开深度上限。实机最深 3 层且无环，留一点余量。
const FLEXEFF_MAX_DEPTH: u8 = 5;

/// 一份 flexeff `particle` 实例（战斗特效里的 `.par`）。
///
/// 原版的实例时间线（FUN_004c8e90 排、FUN_004c90c0 走）：延迟 `+72` 后
/// 打开发射，这一步持续 `+76` 秒；每帧按子块移动、绕圈、缩放。步走完、
/// 实例没了，FUN_004c8bf0 就 `FUN_004bd7a0(0)` 关发射，已有粒子照常走完。
struct FlexParticle {
    effect: crate::openpal3::scene::ParticleEffect,
    entities: Vec<ComRc<radiance::comdef::IEntity>>,
    /// 播放位置加定义记录 `+0xb8..+0xc0` 的偏移（FUN_004c8b80）。
    origin: Vec3,
    /// 子块 +72：起播延迟。
    delay: f32,
    /// 子块 +76：发射这一步的时长。
    life: f32,
    t: f32,
    /// 定义记录 `+200` 起步，子块 +0 每秒加多少（下限 0.001）。
    scale: f32,
    grow: f32,
    /// 子块 +4..+12：每秒平移。
    velocity: Vec3,
    /// 子块 +20..+28 转轴（零向量按 `gbVec3D::Normalize` 当 +y）、
    /// +32 半径、+36 每秒几度。
    axis: Vec3,
    radius: f32,
    spin: f32,
    /// 平移和绕圈算出来的当前偏移。
    moved: Vec3,
    orbit: Vec3,
}

impl FlexParticle {
    /// FUN_004c90c0：发射那一步里每帧做的事。
    fn drive(&mut self, delta_sec: f32) {
        if self.grow != 0. {
            self.scale = (self.scale + self.grow * delta_sec).max(0.001);
            self.effect.set_scale(self.scale);
        }
        self.moved = Vec3::add(&self.moved, &Vec3::scalar_mul(delta_sec, &self.velocity));
        // `SetAxisAngle(轴, (t − 起点) × +36)` 后转 `(+32, 0, 0)`；gbengine 的
        // SetAxisAngle 乘 π/180 × 0.5 进四元数，所以角度是度。转向按
        // 右手 Rodrigues **[推断]**（Rotate 是行向量乘法，未逐项核对正负）。
        let k = if self.axis.norm() < 1e-6 {
            Vec3::new(0., 1., 0.)
        } else {
            Vec3::normalized(&self.axis)
        };
        let (sin, cos) = ((self.t - self.delay) * self.spin).to_radians().sin_cos();
        let v = Vec3::new(self.radius, 0., 0.);
        let cross = Vec3::cross(&k, &v);
        let dot = Vec3::dot(&k, &v);
        self.orbit = Vec3::add(
            &Vec3::add(&Vec3::scalar_mul(cos, &v), &Vec3::scalar_mul(sin, &cross)),
            &Vec3::scalar_mul(dot * (1. - cos), &k),
        );
    }
}

impl AdventureDirectorProps {
    /// 152 的水特效：跟着玩家走的一组 `.par` 粒子。
    fn step_water_magic(&mut self, delta_sec: f32) {
        use crate::openpal3::scene::{ParticleEffect, build_particle_entity, write_particle_mesh};
        use fileformats::pal3::par::{ParEffect, read_par};
        use radiance::comdef::ISceneExt;

        let wanted = self.sce_vm.state().water_magic();
        let Some(scene) = self.scene_manager.scene() else {
            return;
        };

        if wanted && self.water_magic.is_none() && !self.water_magic_failed {
            self.water_magic_failed = true;
            let asset_mgr = self.sce_vm.state().asset_mgr();
            let path = asset_mgr
                .particle_root()
                .join(crate::scripting::sce::commands::WATER_MAGIC_PAR);
            let Some(effect) = common::store_ext::StoreExt2::read_to_end(asset_mgr.vfs(), &path)
                .ok()
                .and_then(|data| read_par(&mut data.as_slice()).ok())
                .map(|file| ParEffect::decode(&file))
            else {
                log::warn!("WaterMagic: 读不到 {}", path.display());
                return;
            };

            let mut entities = vec![];
            for (i, group) in effect.groups.iter().enumerate() {
                // `.par` 里的贴图路径是相对 effect 目录的反斜杠写法。
                // 贴图路径是相对 gbParticle 目录的反斜杠写法。
                let texture_path = asset_mgr
                    .particle_root()
                    .join(group.param.texture.replace('\\', "/"));
                let Some(image) = asset_mgr.read_picture(&texture_path) else {
                    log::warn!("WaterMagic: 读不到贴图 {}", texture_path.display());
                    continue;
                };
                let entity = build_particle_entity(
                    &asset_mgr.component_factory(),
                    i,
                    texture_path.to_str().unwrap_or("particle"),
                    image,
                );
                scene.add_entity(entity.clone());
                entities.push(entity);
            }
            if entities.is_empty() {
                return;
            }
            self.water_magic_failed = false;
            self.water_magic = Some((ParticleEffect::new(&effect), entities));
        }

        let Some((effect, entities)) = self.water_magic.as_ref() else {
            return;
        };
        for entity in entities {
            entity.set_visible(wanted);
        }
        if !wanted {
            return;
        }

        // 特效跟着玩家：原版把它挂在角色身上。
        let anchor = crate::openpal3::directors::SceneManagerExtensions::get_resolved_role(
            &self.scene_manager,
            self.sce_vm.state(),
            -1,
        )
        .map(|role| role.transform().borrow().position())
        .unwrap_or(Vec3::new(0., 0., 0.));

        let (right, up) = {
            let camera = scene.camera();
            let f = camera.transform().matrix().floats();
            (
                Vec3::new(f[0][0], f[1][0], f[2][0]),
                Vec3::new(f[0][1], f[1][1], f[2][1]),
            )
        };

        effect.step(delta_sec);
        for (entity, group) in entities.iter().zip(effect.groups()) {
            let particles: Vec<_> = group
                .particles()
                .into_iter()
                .map(|mut p| {
                    p.position = Vec3::add(&anchor, &p.position);
                    p
                })
                .collect();
            write_particle_mesh(entity, &particles, group.def().param.frames, &right, &up);
        }
    }

    /// 放一个特效音，和表情音效同一条路：读 `/snd/<名>.wav`，起一个一次性
    /// 音源挂在全局状态上。
    fn play_effect_sound(&mut self, name: &str) {
        match self.sce_vm.state().asset_mgr().load_snd_data(name) {
            Ok(data) => {
                let mut source = self.sce_vm.state().audio_engine().create_source();
                source.set_data(data, radiance::audio::Codec::Wav);
                source.play(false);
                self.sce_vm
                    .state_mut()
                    .global_state_mut()
                    .add_sound_source(std::rc::Rc::new(std::cell::RefCell::new(source)));
            }
            // 实机里有一个号码是 14200000 的垃圾值，原版同样找不到文件。
            Err(e) => log::warn!("CEft: 特效音 {name} 读不到: {e:?}"),
        }
    }

    /// 143 CEft 排下的战斗特效。
    ///
    /// 每帧先把脚本排的请求接进来：查 `flexeff` 的特效组，对其中 `quad` 类
    /// 的条目按每个启用的子块建一个 [`QuadEffect`]（逆向见
    /// `switch/re/notes/flexeff_format.md`），再逐个推进、写网格、收掉跑完
    /// 的。别的类还没做，`combat_effect.rs` 那边会把它们记进日志。
    fn step_flexeff(&mut self, delta_sec: f32) {
        use crate::openpal3::scene::flexeff_audio::{AudioEffect, sound_names};
        use crate::openpal3::scene::flexeff_cylinder::{
            CylinderDef, CylinderEffect, build_cylinder_entity, write_cylinder_mesh,
        };
        use crate::openpal3::scene::flexeff_quad::{
            MAX_QUADS, QuadDef, QuadEffect, build_quad_entity, write_quad_mesh,
        };
        use fileformats::pal3::flexeff::GROUP_SUBS;
        use radiance::comdef::ISceneExt;

        let mut requests: Vec<(i32, Vec3, u8)> = self
            .sce_vm
            .state_mut()
            .take_effect_requests()
            .into_iter()
            .map(|(g, p)| (g, p, 0u8))
            .collect();
        // `callback` 排下来的，延迟到点了就展开。
        for entry in self.flexeff_pending.iter_mut() {
            entry.2 -= delta_sec;
        }
        let mut still = vec![];
        for (g, p, left, depth) in std::mem::take(&mut self.flexeff_pending) {
            if left <= 0. {
                requests.push((g, p, depth));
            } else {
                still.push((g, p, left, depth));
            }
        }
        self.flexeff_pending = still;

        let Some(scene) = self.scene_manager.scene() else {
            return;
        };

        for (group_id, position, depth) in requests {
            let asset_mgr = self.sce_vm.state().asset_mgr();
            let Some(eff) = asset_mgr.flexeff() else {
                continue;
            };
            let Some(group) = eff.group(group_id.max(0) as usize) else {
                continue;
            };
            if !group.is_valid() {
                continue;
            }
            for (k, id) in group.ids().iter().enumerate() {
                let Some(def) = eff.find(*id).and_then(|i| eff.definition(i)) else {
                    continue;
                };
                let class = def.class();
                // `callback` 是「再播另一个特效组」：子块 +0 和 +32 是组号，
                // 预载那一支（0x4c2ba0）就是拿它们去调 FUN_004ce460——和指令
                // 148 同一个函数。实机 492 次引用全部有效，链最深 3 层、无环。
                if class == Some("callback") {
                    if depth < FLEXEFF_MAX_DEPTH {
                        for sidx in 0..GROUP_SUBS {
                            if !group.sub_enabled(k, sidx) {
                                continue;
                            }
                            let sub = group.sub(k, sidx).unwrap();
                            let delay = f32::from_le_bytes(sub[72..76].try_into().unwrap()).max(0.);
                            for o in [0usize, 32] {
                                let v =
                                    f32::from_le_bytes(sub[o..o + 4].try_into().unwrap()) as i32;
                                if v > 0 {
                                    self.flexeff_pending.push((v, position, delay, depth + 1));
                                }
                            }
                        }
                    } else {
                        log::warn!("CEft: 特效组 {group_id} 的 callback 链太深，停在这里");
                    }
                    continue;
                }
                // `audio` 没有贴图也没有网格，先单独处理掉。
                if class == Some("audio") {
                    for sidx in 0..GROUP_SUBS {
                        if !group.sub_enabled(k, sidx) {
                            continue;
                        }
                        let sub = group.sub(k, sidx).unwrap();
                        let names = sound_names(&def, sub);
                        if !names.is_empty() {
                            self.flexeff_audio.push(AudioEffect::new(names, sub));
                        }
                    }
                    continue;
                }
                // `particle`：`.par` 粒子（运行时和 152 水特效共用）。
                if class == Some("particle") {
                    use crate::openpal3::scene::{ParticleEffect, build_particle_entity};
                    use fileformats::pal3::par::{ParEffect, read_par};
                    let par_path = asset_mgr
                        .basedata_path()
                        .join(def.asset().replace('\\', "/"));
                    let Some(par) =
                        common::store_ext::StoreExt2::read_to_end(asset_mgr.vfs(), &par_path)
                            .ok()
                            .and_then(|data| read_par(&mut data.as_slice()).ok())
                            .map(|file| ParEffect::decode(&file))
                    else {
                        log::warn!("CEft: 读不到粒子 {}", par_path.display());
                        continue;
                    };
                    // 定义记录里的两项（FUN_004c8b80）：`+200` 是整个粒子
                    // 系统的缩放，`+0xb8..+0xc0` 加在播放位置上。
                    let raw = {
                        use fileformats::pal3::flexeff::DEF_SIZE;
                        &eff.definitions_block()[def.index * DEF_SIZE..(def.index + 1) * DEF_SIZE]
                    };
                    let rf = |o: usize| f32::from_le_bytes(raw[o..o + 4].try_into().unwrap());
                    let scale = if rf(200) > 0. { rf(200) } else { 1. };
                    let offset = Vec3::new(rf(0xb8), rf(0xbc), rf(0xc0));
                    for sidx in 0..GROUP_SUBS {
                        if !group.sub_enabled(k, sidx) {
                            continue;
                        }
                        let sub = group.sub(k, sidx).unwrap();
                        let f = |o: usize| f32::from_le_bytes(sub[o..o + 4].try_into().unwrap());
                        let mut entities = vec![];
                        for (i, g) in par.groups.iter().enumerate() {
                            let texture_path = asset_mgr
                                .particle_root()
                                .join(g.param.texture.replace('\\', "/"));
                            let Some(image) = asset_mgr.read_picture(&texture_path) else {
                                log::warn!("CEft: 读不到粒子贴图 {}", texture_path.display());
                                continue;
                            };
                            let entity = build_particle_entity(
                                &asset_mgr.component_factory(),
                                i,
                                texture_path.to_str().unwrap_or("particle"),
                                image,
                            );
                            entity.set_visible(false);
                            scene.add_entity(entity.clone());
                            entities.push(entity);
                        }
                        if entities.is_empty() {
                            continue;
                        }
                        let effect = ParticleEffect::new(&par);
                        effect.set_scale(scale);
                        log::info!(
                            "CEft: 粒子 {} 缩放 {scale} 偏移 {offset:?} 延迟 {} 发射 {} 秒",
                            def.asset(),
                            f(72),
                            f(76)
                        );
                        self.flexeff_par.push(FlexParticle {
                            effect,
                            entities,
                            origin: Vec3::add(&position, &offset),
                            delay: f(72).max(0.),
                            life: f(76).max(0.),
                            t: 0.,
                            scale,
                            grow: f(0),
                            velocity: Vec3::new(f(4), f(8), f(12)),
                            axis: Vec3::new(f(20), f(24), f(28)),
                            radius: f(32),
                            spin: f(36),
                            moved: Vec3::new(0., 0., 0.),
                            orbit: Vec3::new(0., 0., 0.),
                        });
                    }
                    continue;
                }
                if class != Some("quad") && class != Some("cylinder") {
                    continue;
                }
                // 资源名是 `effect\xxx.tga` 这样的反斜杠相对路径。
                let texture_path = asset_mgr
                    .basedata_path()
                    .join(def.asset().replace('\\', "/"));
                let Some(image) = asset_mgr.read_picture(&texture_path) else {
                    log::warn!("CEft: 读不到特效贴图 {}", texture_path.display());
                    continue;
                };
                let name = texture_path.to_str().unwrap_or("flexeff");
                let origin = [position.x, position.y, position.z];
                for sidx in 0..GROUP_SUBS {
                    if !group.sub_enabled(k, sidx) {
                        continue;
                    }
                    let sub = group.sub(k, sidx).unwrap();
                    self.flexeff_seq += 1;
                    let seq = self.flexeff_seq;
                    let factory = asset_mgr.component_factory();
                    if class == Some("quad") {
                        let qd = QuadDef::from_def(&def);
                        let entity =
                            build_quad_entity(&factory, seq, name, image.clone(), qd.additive);
                        scene.add_entity(entity.clone());
                        self.flexeff
                            .push((QuadEffect::new(qd, sub, origin), entity));
                    } else {
                        let cd = CylinderDef::from_def(&def);
                        let entity =
                            build_cylinder_entity(&factory, seq, name, image.clone(), cd.additive);
                        scene.add_entity(entity.clone());
                        self.flexeff_cyl
                            .push((CylinderEffect::new(cd, sub, origin), entity));
                    }
                }
            }
        }

        // 声音先处理，它不依赖场景也不依赖相机。
        if !self.flexeff_audio.is_empty() {
            let mut due: Vec<String> = vec![];
            for a in self.flexeff_audio.iter_mut() {
                if let Some(names) = a.step(delta_sec) {
                    due.extend(names.iter().cloned());
                }
            }
            self.flexeff_audio.retain(|a| !a.finished());
            for name in due {
                self.play_effect_sound(&name);
            }
        }

        if !self.flexeff_par.is_empty() {
            use crate::openpal3::scene::write_particle_mesh;
            let (right, up) = {
                let camera = scene.camera();
                let f = camera.transform().matrix().floats();
                (
                    Vec3::new(f[0][0], f[1][0], f[2][0]),
                    Vec3::new(f[0][1], f[1][1], f[2][1]),
                )
            };
            for p in self.flexeff_par.iter_mut() {
                p.t += delta_sec;
                if p.t < p.delay {
                    continue;
                }
                if p.t >= p.delay + p.life {
                    p.effect.stop();
                } else {
                    p.drive(delta_sec);
                }
                p.effect.step(delta_sec);
                let at = Vec3::add(&p.origin, &Vec3::add(&p.moved, &p.orbit));
                for (entity, group) in p.entities.iter().zip(p.effect.groups()) {
                    entity.set_visible(true);
                    let particles: Vec<_> = group
                        .particles()
                        .into_iter()
                        .map(|mut q| {
                            q.position = Vec3::add(&at, &q.position);
                            q
                        })
                        .collect();
                    write_particle_mesh(entity, &particles, group.def().param.frames, &right, &up);
                }
            }
            // 发射停了、粒子也走完了（或者拖太久）就收掉。
            self.flexeff_par.retain(|p| {
                let over = p.t >= p.delay + p.life
                    && (p.effect.alive_count() == 0 || p.t >= p.delay + p.life + 10.);
                if over {
                    for e in &p.entities {
                        e.set_visible(false);
                    }
                }
                !over
            });
        }

        if self.flexeff.is_empty() && self.flexeff_cyl.is_empty() {
            return;
        }
        let (right, up) = {
            let camera = scene.camera();
            let f = camera.transform().matrix().floats();
            (
                Vec3::new(f[0][0], f[1][0], f[2][0]),
                Vec3::new(f[0][1], f[1][1], f[2][1]),
            )
        };
        for (effect, entity) in self.flexeff.iter_mut() {
            effect.step(delta_sec);
            if effect.quads().len() > MAX_QUADS {
                log::warn!(
                    "CEft: 一个特效画了 {} 个面片，超出上限",
                    effect.quads().len()
                );
            }
            write_quad_mesh(entity, effect, &right, &up);
        }
        // 跑完的收掉。
        let mut done = vec![];
        self.flexeff.retain(|(effect, entity)| {
            if effect.finished() {
                done.push(entity.clone());
                false
            } else {
                true
            }
        });
        for entity in done {
            entity.set_visible(false);
        }

        for (effect, entity) in self.flexeff_cyl.iter_mut() {
            effect.step(delta_sec);
            write_cylinder_mesh(entity, effect);
        }
        let mut done = vec![];
        self.flexeff_cyl.retain(|(effect, entity)| {
            if effect.finished() {
                done.push(entity.clone());
                false
            } else {
                true
            }
        });
        for entity in done {
            entity.set_visible(false);
        }
    }

    /// Step the weather and lay its quads out for this frame.
    ///
    /// `gbSnow` keeps its drops in camera space and `gbSnow::Draw`
    /// billboards them, so each flake is a quad on the camera's right/up
    /// axes centred at the drop's camera-space offset. `SIZE` is the
    /// full edge; `Draw` halves it.
    fn step_snow(&mut self, delta_sec: f32) {
        use crate::openpal3::scene::{Snow, build_snow_entity};
        use radiance::comdef::ISceneExt;
        use radiance::rendering::VertexComponents;

        let wanted = self.sce_vm.state().snow();
        let Some(scene) = self.scene_manager.scene() else {
            return;
        };

        if wanted && self.snow_entity.is_none() && !self.snow_failed {
            let asset_mgr = self.sce_vm.state().asset_mgr();
            match build_snow_entity(&asset_mgr) {
                Some(entity) => {
                    scene.add_entity(entity.clone());
                    self.snow_entity = Some(entity);
                }
                None => {
                    // Once is enough; this runs every frame.
                    self.snow_failed = true;
                    log::warn!("Snow: 读不到 effect\\{}", Snow::TEXTURE_NAME);
                }
            }
        }

        let Some(entity) = self.snow_entity.clone() else {
            return;
        };
        entity.set_visible(wanted);
        if !wanted {
            return;
        }

        let (camera_position, right, up) = {
            let camera = scene.camera();
            let m = *camera.transform().matrix();
            let f = m.floats();
            (
                Vec3::new(f[0][3], f[1][3], f[2][3]),
                Vec3::new(f[0][0], f[1][0], f[2][0]),
                Vec3::new(f[0][1], f[1][1], f[2][1]),
            )
        };
        let forward = Vec3::cross(&right, &up);

        self.snow.step(delta_sec);
        let drops = self.snow.positions();
        let half = Snow::SIZE / 2.;

        let Some(rc) = entity.get_rendering_component() else {
            return;
        };
        let offset = |c: &Vec3, a: f32, b: f32| {
            Vec3::add(
                c,
                &Vec3::add(&Vec3::scalar_mul(a, &right), &Vec3::scalar_mul(b, &up)),
            )
        };
        for object in rc.render_objects() {
            object.as_dyn().update_vertices(&|mut buffer| {
                for i in 0..Snow::MAX {
                    // Camera space to world: a drop's z runs into the
                    // screen, which is -forward.
                    let corners = match drops.get(i) {
                        // Unused quads collapse onto a point.
                        None => [Vec3::new(0., 0., 0.); 4],
                        Some(p) => {
                            let centre = Vec3::add(
                                &camera_position,
                                &Vec3::add(
                                    &Vec3::add(
                                        &Vec3::scalar_mul(p.x, &right),
                                        &Vec3::scalar_mul(p.y, &up),
                                    ),
                                    &Vec3::scalar_mul(-p.z, &forward),
                                ),
                            );
                            [
                                offset(&centre, -half, -half),
                                offset(&centre, half, -half),
                                offset(&centre, half, half),
                                offset(&centre, -half, half),
                            ]
                        }
                    };
                    for (k, corner) in corners.iter().enumerate() {
                        buffer.set_component(
                            i * 4 + k,
                            VertexComponents::POSITION,
                            |p: &mut Vec3| *p = *corner,
                        );
                    }
                }
            });
        }
    }
}

impl AdventureDirectorProps {
    fn test_save(&self) {
        let input = self.input_engine.borrow_mut();
        let save_slot = if input.get_key_state(Key::Num1).pressed() {
            1
        } else if input.get_key_state(Key::Num2).pressed() {
            2
        } else if input.get_key_state(Key::Num3).pressed() {
            3
        } else if input.get_key_state(Key::Num4).pressed() {
            4
        } else {
            -1
        };

        // Switch has no keyboard, so the Num1-4 debug saves are unreachable
        // there. West (Y) is unused in the adventure layer; make it the
        // quick-save until a real save menu is wired up.
        #[cfg(target_os = "horizon")]
        let save_slot = if input.get_key_state(Key::GamePadWest).pressed() {
            1
        } else {
            save_slot
        };

        self.sce_vm
            .global_state()
            .persistent_state()
            .save(save_slot);
    }

    /// 山洞之狼（SCE 112）。原版把它做成主循环的游戏状态 7
    /// （`DAT_00bfda6c`，`0x405690` 起）：每帧加时间，满 60 秒算时间到，
    /// 否则扫一遍角色表，活着的角色离主角不到 24 就算抓到。规则和常数在
    /// `states::cave_wolf`。
    fn step_cave_wolf(&mut self, delta_sec: f32) {
        use crate::openpal3::states::cave_wolf::{self, CaveWolf, Outcome};
        use crate::scripting::sce::commands::take_cave_wolf_start;

        if take_cave_wolf_start(self.sce_vm.state_mut()) {
            self.cave_wolf = Some(CaveWolf::new());
        }
        if self.cave_wolf.is_none() {
            return;
        }

        // 主角到最近一个「活着的」角色有多远。原版看的是角色记录的
        // `+0x278`，那一位是从 `.scn` 角色记录的 `+0x54` 抄来的，也就是
        // 移植版 `RoleController::is_active()`。
        let nearest = self.nearest_active_role_distance();

        let outcome = self
            .cave_wolf
            .as_mut()
            .and_then(|g| g.tick(delta_sec, nearest));
        let Some(outcome) = outcome else {
            return;
        };
        self.cave_wolf = None;

        let proc = match outcome {
            Outcome::Caught => {
                let wins = {
                    let mut p = self.sce_vm.global_state_mut().persistent_state_mut();
                    let wins = p.cave_wolf_wins();
                    p.record_cave_wolf_win();
                    // 前四次各发一件奖品（`DAT_00bfda44 < 4`）。
                    if let Some(item) = cave_wolf::reward_for(wins) {
                        p.add_item(item, 1);
                    }
                    wins
                };
                log::info!("CaveWolf: 抓到了，这是第 {} 次", wins + 1);
                cave_wolf::WIN_PROC
            }
            Outcome::TimedOut => {
                log::info!("CaveWolf: 时间到");
                cave_wolf::LOSE_PROC
            }
        };

        let init = Rc::new(self.sce_vm.state().asset_mgr().load_init_sce());
        if !self.sce_vm.state_mut().call_proc_in(init, "init", proc) {
            log::warn!("CaveWolf: init.sce 里没有 proc {proc}");
        }
    }

    /// 场上活着的角色里离主角最近的那个有多远。没有别人就是 `None`。
    fn nearest_active_role_distance(&self) -> Option<f32> {
        let player = self
            .scene_manager
            .get_resolved_role(self.sce_vm.state(), -1)?;
        let here = player.transform().borrow().position();
        let scene = self.scene_manager.scene()?;

        scene
            .entities()
            .iter()
            .filter(|e| e.name().starts_with("ROLE_") && e.name() != player.name())
            .filter(|e| {
                RoleController::get_role_controller((*e).clone())
                    .map(|rc| rc.inner::<RoleController>().is_active())
                    .unwrap_or(false)
            })
            .map(|e| {
                let p = e.transform().borrow().position();
                Vec3::sub(&p, &here).norm()
            })
            .fold(None, |acc: Option<f32>, d| {
                Some(acc.map_or(d, |best| best.min(d)))
            })
    }

    /// 脚步声。原版 `FUN_0041eb80`：按主角的移动模式挑走或跑那一套，
    /// 再按脚下 nav 格的地面类型（第 5 字节）挑 a–d 里的一个，循环播；
    /// 和上一帧同一条就不管，换了就停旧的播新的，停下来就停掉。
    fn step_footstep(&mut self, moving_direction: &Vec3, throw: f32) {
        // 只看有没有输入。原版挑的是角色的移动模式，撞墙走不动时模式
        // 也还在「走」上，声音不停。
        let mode = (moving_direction.norm() > 0.5).then(|| player_mode(throw));
        let wanted = footstep::sound_name(mode, self.player_floor_type());

        match (&self.footstep, &wanted) {
            // 没换，继续循环播着。
            (Some((playing, _)), Some(name)) if playing == name => return,
            _ => {}
        }
        if let Some((_, source)) = self.footstep.take() {
            source.borrow_mut().stop();
            self.sce_vm.global_state_mut().remove_sound_source(source);
        }
        let Some(name) = wanted else { return };
        let Ok(data) = self.sce_vm.state().asset_mgr().load_snd_data(&name) else {
            // 缺这个音效就不再重试——每帧读一次盘不值当。
            log::debug!("脚步声 {name} 读不到");
            return;
        };
        let mut source = self.sce_vm.state().audio_engine().create_source();
        source.set_data(data, radiance::audio::Codec::Wav);
        source.play(true);
        let source = Rc::new(RefCell::new(source));
        self.sce_vm
            .global_state_mut()
            .add_sound_source(source.clone());
        self.footstep = Some((name, source));
    }

    /// 主角脚下 nav 格的地面类型。越界原版填 2（`FUN_0041eb80` 开头）。
    fn player_floor_type(&self) -> i32 {
        const OUT_OF_RANGE: i32 = 2;
        let Some(role) = self
            .scene_manager
            .get_resolved_role(self.sce_vm.state(), -1)
        else {
            return OUT_OF_RANGE;
        };
        let Some(scn) = self.scene_manager.scn_scene() else {
            return OUT_OF_RANGE;
        };
        let Some(controller) = RoleController::get_role_controller(role.clone()) else {
            return OUT_OF_RANGE;
        };
        let layer = controller.inner::<RoleController>().nav_layer();
        let position = role.transform().borrow().position();
        let s = scn.inner::<crate::openpal3::scene::ScnScene>();
        let (nx, nz) = s.scene_coord_to_nav_coord(layer, &position);
        s.nav()
            .get(layer, nx.trunc() as i32, nz.trunc() as i32)
            .map(|tile| ((tile.distance_to_border >> 8) & 0xff) as i32)
            .unwrap_or(OUT_OF_RANGE)
    }

    fn move_role(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        delta_sec: f32,
        moving_direction: &Vec3,
        throw: f32,
    ) {
        let role = scene_manager
            .get_resolved_role(self.sce_vm.state(), -1)
            .unwrap();
        let role_controller = RoleController::get_role_controller(role.clone()).unwrap();
        let mut position = role.transform().borrow().position();

        let nav_layer = role_controller.inner::<RoleController>().nav_layer();

        // 推力分走跑，速度取 `scene.txt` 的 PWalkSpeed / PRunSpeed
        // （见 `switch/re/notes/role_move.md`）。原先这里无条件跑、
        // 速度写死 175——那不是原版的任何一个值。
        let mode = player_mode(throw);
        let speed = self
            .sce_vm
            .global_state()
            .role_constants()
            .speed(mode, true, 0.);
        let mut target_position = Vec3::add(
            &position,
            &Vec3::scalar_mul(speed * delta_sec, &moving_direction),
        );
        let (target_nav_coord, height, distance_to_border) = {
            let scn = scene_manager.scn_scene().unwrap();
            let scene = scn.inner::<crate::openpal3::scene::ScnScene>();
            let tnc = scene.scene_coord_to_nav_coord(nav_layer, &target_position);
            let h = scene.get_height(nav_layer, tnc);
            let d = scene.get_distance_to_border_by_scene_coord(nav_layer, &target_position);
            (tnc, h, d)
        };
        let _ = target_nav_coord;
        target_position.y = height;

        let role = scene_manager
            .get_resolved_role(self.sce_vm.state(), -1)
            .unwrap();
        if moving_direction.norm() > 0.5
            && (self.sce_vm.global_state().pass_through_wall()
                || distance_to_border > std::f32::EPSILON)
        {
            match mode {
                MoveMode::Run => role_controller.inner::<RoleController>().run(),
                _ => role_controller.inner::<RoleController>().walk(),
            }
            let look_at = Vec3::new(target_position.x, position.y, target_position.z);
            role.transform()
                .borrow_mut()
                .look_at(&look_at)
                .set_position(&target_position);

            self.sce_vm
                .global_state_mut()
                .persistent_state_mut()
                .set_position(target_position);

            position = target_position
        } else {
            role_controller.inner::<RoleController>().idle();
        }

        // The default follow rig, left exactly as it was so nothing moves
        // on screen while no script has taken the camera over. A script
        // that focuses the camera elsewhere, or turns following off with
        // CameraFollowPlayer(0), is served by `apply_scripted_camera`.
        let script_has_camera = !matches!(
            self.sce_vm.global_state().camera().focus(),
            crate::openpal3::states::camera::Focus::Player
        ) || !self.sce_vm.global_state().camera().follow_player();
        if !script_has_camera {
            let scene = scene_manager.scene().unwrap();
            scene
                .camera_mut()
                .transform_mut()
                .set_position(&Vec3::new(400., 400., 400.))
                .rotate_axis_angle(&Vec3::UP, self.camera_rotation)
                .translate(&position)
                .look_at(&position);
        }
    }

    /// Point the camera at whatever the script has focused it on, using
    /// the polar model in [`Pal3Camera`](crate::openpal3::states::camera::Pal3Camera).
    ///
    /// The angles and distance are seeded from wherever the camera
    /// already is the first time a script takes it over, so a focus
    /// command swings the view onto its target without also changing the
    /// framing — which is what `FUN_0041c170` and friends do: they write
    /// the target pointer and nothing else.
    fn apply_scripted_camera(&mut self) {
        use crate::openpal3::states::camera::{Focus, polar_of};

        // 栈顶是小游戏压上来的普通场景（鉴定 104）时没有角色可解析，
        // 也没有它的相机可管。
        if self.scene_manager.scn_scene().is_none() {
            return;
        }

        let focus = self.sce_vm.global_state().camera().focus().clone();
        let target = match &focus {
            Focus::Player => {
                self.scripted_camera_seeded = false;
                return;
            }
            Focus::Point(p) => Some(*p),
            Focus::Role(id) => self
                .scene_manager
                .get_resolved_role(self.sce_vm.state(), *id)
                .map(|e| e.transform().borrow().position()),
            Focus::Object(id) => self
                .scene_manager
                .scn_scene()
                .and_then(|s| {
                    s.inner::<crate::openpal3::scene::ScnScene>()
                        .get_root_object(*id)
                })
                .map(|e| e.transform().borrow().position()),
        };

        let Some(target) = target else {
            return;
        };

        let Some(scene) = self.scene_manager.scene() else {
            return;
        };

        // Seed from the live camera the first time round: offset from the
        // new target to where the camera is now gives the angles and
        // distance to keep.
        let current = scene.camera().transform().position();
        let camera = self.sce_vm.global_state_mut().camera_mut();
        if !self.scripted_camera_seeded {
            let (yaw, pitch, distance) = polar_of(&Vec3::sub(&current, &target));
            if distance > f32::EPSILON {
                camera.set_yaw(yaw);
                camera.set_pitch(pitch);
                camera.set_distance(distance);
            }
            self.scripted_camera_seeded = true;
        }

        camera.look_at_target(target);
        let position = camera.position();
        scene
            .camera_mut()
            .transform_mut()
            .set_position(&position)
            .look_at(&target);
    }

    fn do_update(&mut self, delta_sec: f32) -> Option<ComRc<IDirector>> {
        self.sce_vm.update(delta_sec);

        // Freeze the whole game world while the full-screen status (状态)
        // menu is open, but only when the player actually has control —
        // never mid-cutscene, where the SCE VM is driving animations.
        // `IScene::set_active(false)` makes the scene's per-frame entity
        // update a no-op, so NPC patrols, role animations and world
        // transforms all hold their pose; we also skip the world sim
        // below so movement/triggers stop too.
        let adv_enabled = self.sce_vm.global_state().adv_input_enabled();
        let menu_open = self.sce_vm.state().status_renderer().is_menu_open();
        let freeze = adv_enabled && menu_open;
        if let Some(scene) = self.scene_manager.scene() {
            scene.set_active(!freeze);
        }

        // Has to run before the input guard: during a cutscene the ADV
        // director stops updating the world, but the script's camera
        // still needs to be applied every frame.
        self.apply_scripted_camera();

        // Scenery slid by opcode 90 keeps moving during a cutscene, when
        // the world sim below is not running.
        if let Some(scene_rc) = self.scene_manager.scn_scene() {
            scene_rc
                .inner::<crate::openpal3::scene::ScnScene>()
                .step_object_moves(delta_sec);
        }

        // Weather is drawn over the whole frame in the original, after
        // the scene graph ends, so it keeps running through cutscenes.
        self.step_snow(delta_sec);
        self.step_water_magic(delta_sec);
        self.step_flexeff(delta_sec);

        if !adv_enabled {
            return None;
        }
        if freeze {
            return None;
        }

        if self.scene_manager.scene().is_none() {
            return None;
        }

        self.step_cave_wolf(delta_sec);
        self.test_save();

        let (moving_direction, throw) = get_moving_direction_and_throw(
            self.input_engine.clone(),
            self.scene_manager.scene().unwrap(),
        );
        self.move_role(
            self.scene_manager.clone(),
            delta_sec,
            &moving_direction,
            throw,
        );

        // Advance ambient NPC patrols (non-scripted townsfolk walking their
        // authored loop). Scripted roles are untouched.
        {
            let scene_rc = self.scene_manager.scn_scene().unwrap();
            scene_rc
                .inner::<crate::openpal3::scene::ScnScene>()
                .step_patrols(delta_sec);
        }

        self.camera_rotation =
            get_camera_rotation(self.input_engine.clone(), self.camera_rotation, delta_sec);

        let (position, nav_layer) = {
            let role = self
                .scene_manager
                .get_resolved_role(self.sce_vm.state(), -1)
                .unwrap();
            let r = RoleController::get_role_controller(role.clone()).unwrap();
            (
                role.transform().borrow().position(),
                r.inner::<RoleController>().nav_layer(),
            )
        };

        let scene_rc = self.scene_manager.scn_scene().unwrap();
        let proc_id_opt = {
            let s = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
            s.test_nav_trigger(nav_layer, &position)
        };
        if let Some(proc_id) = proc_id_opt {
            debug!("New proc triggerd by nav: {}", proc_id);
            self.sce_vm.call_proc(proc_id);
        }

        let nav_layer_triggered = {
            let s = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
            s.test_nav_layer_trigger(nav_layer, &position)
        };
        if nav_layer_triggered {
            if !self.layer_switch_triggered {
                let layer = {
                    let e = self
                        .scene_manager
                        .get_resolved_role(self.sce_vm.state(), -1)
                        .unwrap();
                    let r = RoleController::get_role_controller(e).unwrap();
                    r.inner::<RoleController>().nav_layer()
                };
                let new_layer = (layer + 1) % 2;

                let mut test_coord = position;
                let mut d = 0.0;
                for _i in 0..50 {
                    d = {
                        let s = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
                        s.get_distance_to_border_by_scene_coord(new_layer, &test_coord)
                    };
                    if d > 0.0 {
                        break;
                    }

                    test_coord = Vec3::add(&test_coord, &moving_direction);
                }

                if d > 0.0 {
                    let e = self
                        .scene_manager
                        .get_resolved_role(self.sce_vm.state(), -1)
                        .unwrap();
                    let r = RoleController::get_role_controller(e).unwrap();
                    r.inner::<RoleController>().switch_nav_layer();
                    self.scene_manager
                        .get_resolved_role(self.sce_vm.state(), -1)
                        .unwrap()
                        .transform()
                        .borrow_mut()
                        .set_position(&test_coord);
                    self.layer_switch_triggered = true;
                }
            }
        } else {
            self.layer_switch_triggered = false;
        }

        let input = self.input_engine.borrow_mut();
        // Space matches the original PC binding (空格 = 对话/调查/确定);
        // F is the upstream OpenPAL3 binding, GamePadEast the pad one.
        if input.get_key_state(Key::F).pressed()
            || input.get_key_state(Key::Space).pressed()
            || input.get_key_state(Key::GamePadEast).pressed()
        {
            let trigger_proc_id = {
                let scene = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
                scene
                    .test_aabb_trigger(&position)
                    .or_else(|| scene.test_item_trigger(&position))
                    .or_else(|| {
                        scene.test_role_trigger(
                            &position,
                            self.sce_vm.global_state().role_controlled(),
                        )
                    })
            };
            if let Some(proc_id) = trigger_proc_id {
                debug!("New proc triggerd: {}", proc_id);
                self.sce_vm.call_proc(proc_id);
            }

            let result = {
                let scene = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
                scene.test_ladder(nav_layer, &position)
            };
            match result {
                Some(LadderTestResult::NewPosition((new_layer, new_position))) => {
                    debug!(
                        "Ladder detected, new_layer: {:?} new position: {:?}",
                        &new_layer, &new_position
                    );

                    if new_layer {
                        let e = self
                            .scene_manager
                            .get_resolved_role(self.sce_vm.state(), -1)
                            .unwrap();
                        let r = RoleController::get_role_controller(e).unwrap();
                        r.inner::<RoleController>().switch_nav_layer();
                    }

                    self.scene_manager
                        .get_resolved_role(self.sce_vm.state(), -1)
                        .unwrap()
                        .transform()
                        .borrow_mut()
                        .set_position(&new_position);
                }
                Some(LadderTestResult::SceProc(proc_id)) => {
                    debug!("Ladder detected, proc_id {}", &proc_id);
                    self.sce_vm.call_proc(proc_id);
                }
                None => {}
            }
        }

        None
    }
}
