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
                player_slide_dir: None,
                object_walk: None,
                last_nav_trigger: None,
                player_jump: None,
                jump_charge: None,
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
                flexeff_explosive: vec![],
                flexeff_grid: vec![],
                flexeff_model: vec![],
                flexeff_shock: vec![],
                flexeff_cam: vec![],
                flexeff_timed: vec![],
                flexeff_meteor: vec![],
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
                player_slide_dir: None,
                object_walk: None,
                last_nav_trigger: None,
                player_jump: None,
                jump_charge: None,
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
                flexeff_explosive: vec![],
                flexeff_grid: vec![],
                flexeff_model: vec![],
                flexeff_shock: vec![],
                flexeff_cam: vec![],
                flexeff_timed: vec![],
                flexeff_meteor: vec![],
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
    /// Pop a 68 note, for looking at the note box.
    pub fn show_note(&self, text: String) {
        self.props.borrow_mut().sce_vm.state_mut().show_note(text);
    }

    pub fn call_sce_proc(&self, proc_id: u32) {
        self.props.borrow_mut().sce_vm.call_proc(proc_id);
    }

    /// Start a battle straight away, the way opcode 73/80 would.
    ///
    /// A debug poke for the agent server: every scripted battle sits
    /// behind a cutscene, so verifying the combat stage otherwise means
    /// playing through one. `boss` picks the 80 (fixed line-up) path
    /// over 73 (random encounter).
    /// Push a 65 dialog-choice box with `items` (in display order), so a
    /// driver can see and operate it without playing up to a scripted one.
    pub fn start_debug_dlg_sel(&self, mut items: Vec<String>) {
        let mut p = self.props.borrow_mut();
        let scene_manager = p.scene_manager.clone();
        // SCE stores the parameters last-first and `SceCommandDlgSel::new`
        // reverses them back.
        items.reverse();
        let cmd: Box<dyn crate::scripting::sce::SceCommand> = Box::new(
            crate::scripting::sce::commands::SceCommandDlgSel::new(items),
        );
        p.sce_vm.push_debug_command(cmd, scene_manager);
    }

    /// Push a 100 dealer command for `shop` (a `DealScript\\…` path as
    /// the scripts write it), so the shop UI can be looked at directly.
    pub fn start_debug_dealer(&self, shop: String) {
        let mut p = self.props.borrow_mut();
        let scene_manager = p.scene_manager.clone();
        let cmd: Box<dyn crate::scripting::sce::SceCommand> = Box::new(
            crate::scripting::sce::commands::SceCommandDealerMenu::new(shop),
        );
        p.sce_vm.push_debug_command(cmd, scene_manager);
    }

    /// Open the big map (102 13) right here, so it can be looked at without
    /// playing up to a scripted exit.
    pub fn start_debug_bigmap(&self) {
        let mut p = self.props.borrow_mut();
        let scene_manager = p.scene_manager.clone();
        let cmd: Box<dyn crate::scripting::sce::SceCommand> = Box::new(
            crate::scripting::sce::commands::SceCommandSwitchMode::new(13),
        );
        p.sce_vm.push_debug_command(cmd, scene_manager);
    }

    /// Set big-map destination states the way 125 does, one per index.
    pub fn set_bigmap_elements(&self, states: &[i32]) {
        let mut p = self.props.borrow_mut();
        let mut ps = p.sce_vm.global_state_mut().persistent_state_mut();
        for (id, option) in states.iter().enumerate() {
            ps.set_bigmap_element(id as i32, *option);
        }
    }

    /// Push any SCE command as if a script had issued it.
    pub fn start_debug_command(&self, cmd: Box<dyn crate::scripting::sce::SceCommand>) {
        let mut p = self.props.borrow_mut();
        let scene_manager = p.scene_manager.clone();
        p.sce_vm.push_debug_command(cmd, scene_manager);
    }

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

    /// 物件本身和子节点（先序，最多 8 个）的局部矩阵——调试用，看类型 17 的
    /// 浮动 / 旋转 / 重播有没有在走。
    pub fn object_transforms(&self, object_id: i32) -> Option<Vec<[[f32; 4]; 4]>> {
        fn walk(e: &ComRc<radiance::comdef::IEntity>, out: &mut Vec<[[f32; 4]; 4]>) {
            if out.len() >= 8 {
                return;
            }
            out.push(*e.transform().borrow().matrix().floats());
            for c in e.children() {
                walk(&c, out);
            }
        }
        let props = self.props.borrow();
        let scene = props.scene_manager.scn_scene()?;
        let entity = scene
            .inner::<crate::openpal3::scene::ScnScene>()
            .get_root_object(object_id)?;
        let mut out = vec![];
        walk(&entity, &mut out);
        Some(out)
    }

    /// Put items in the bag — a test aid for the smelting / shop UIs.
    pub fn add_item(&self, id: u32, count: u32) {
        self.props
            .borrow_mut()
            .sce_vm
            .global_state_mut()
            .persistent_state_mut()
            .add_item(id, count);
    }

    /// Put characters into the party — a test aid for the shop's compare panel.
    pub fn add_to_team(&self, actor_id: u32) {
        self.props
            .borrow_mut()
            .sce_vm
            .global_state_mut()
            .persistent_state_mut()
            .add_to_team(actor_id);
    }

    /// Ask the running battle to fire a 合击技 on the next party turn — a test aid.
    pub fn request_combat_combo(&self, id: u32) {
        self.props
            .borrow_mut()
            .sce_vm
            .state_mut()
            .request_combat_combo(id);
    }

    /// Put a character on a formation position — a test aid for the 五灵 disc.
    pub fn set_lineup_position(&self, actor_id: u32, position: u8) {
        self.props
            .borrow_mut()
            .sce_vm
            .global_state_mut()
            .persistent_state_mut()
            .set_lineup_position(actor_id, position);
    }

    /// 把某人升到 `level − 1` 级、经验差 1 点到 `level`——测升级学会特技用。
    pub fn prime_level(&self, actor_id: u32, level: i32) -> bool {
        let mut props = self.props.borrow_mut();
        let gs = props.sce_vm.global_state_mut();
        let Some(db) = gs.game_db().cloned() else {
            return false;
        };
        let mut ps = gs.persistent_state_mut();
        ps.gain_level(actor_id, level - 1);
        match ps.role_mut(actor_id) {
            Some(r) => {
                r.exp = db.exp_for_level(level) - 1;
                true
            }
            None => false,
        }
    }

    /// 让某人学会一条技能（opcode 51 的同一条路）——测特技 / 仙术用。
    pub fn learn_skill(&self, actor_id: u32, skill_id: u32) -> bool {
        self.props
            .borrow_mut()
            .sce_vm
            .global_state_mut()
            .persistent_state_mut()
            .learn_skill(actor_id, skill_id)
    }

    /// Give (or take, if negative) money — a test aid for the shop UI.
    pub fn add_money(&self, amount: i32) {
        self.props
            .borrow_mut()
            .sce_vm
            .global_state_mut()
            .persistent_state_mut()
            .add_money(amount);
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
    /// 主角上一次贴墙滑的方向（角色 `+0x534`，`FUN_0041bad0` 初始化成朝向）。
    player_slide_dir: Option<Vec3>,
    /// 场景物件的动作 2 正牵着主角走（`FUN_00412b60` 设的移动方式 4）。
    object_walk: Option<ObjectWalk>,
    /// 上一帧主角站着的导航触发区的过程号。原版类型 0 触发点（`FUN_00424c20` case 0）只在「上一帧不在同一个物件上」
    /// 时跑过程，站着不动不会每帧重跑。
    last_nav_trigger: Option<u32>,
    /// 主角正在跳（`FUN_00414a30` → 行为 4、动作 0）；第二项是跳的动作放出来没有。
    player_jump: Option<(crate::openpal3::scene::player_jump::Jump, bool)>,
    /// [移植] 跳键按住了多久（蓄力）。原版是右键点地面直接给落点，手柄上没有这一下。
    jump_charge: Option<f32>,
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
    /// 143 播出去的战斗特效：`quad` 那一支的模型加它画成的实体（别的类各有自己的列表）。
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
    /// `explosive` 实例：模型、碎片实体。
    flexeff_explosive: Vec<(
        crate::openpal3::scene::flexeff_explosive::ExplosiveEffect,
        ComRc<radiance::comdef::IEntity>,
    )>,
    /// `grid` 实例：模型、面片实体。
    flexeff_grid: Vec<(
        crate::openpal3::scene::flexeff_grid::GridEffect,
        ComRc<radiance::comdef::IEntity>,
    )>,
    /// `model` 实例：模型、`.cvd` / `.pol` 实体。
    flexeff_model: Vec<(
        crate::openpal3::scene::flexeff_model::ModelEffect,
        ComRc<radiance::comdef::IEntity>,
    )>,
    /// `shock` 实例：模型、面片实体。
    flexeff_shock: Vec<(
        crate::openpal3::scene::flexeff_shock::ShockEffect,
        ComRc<radiance::comdef::IEntity>,
    )>,
    /// `camera` 实例：`(flexcam 序列号，≤ 0 = 只停；离起播还剩几秒)`。
    flexeff_cam: Vec<(i32, f32)>,
    /// `quake` / `bink` 实例：到点才把请求交给战斗命令。`(离到点还剩几秒, 请求)`。
    flexeff_timed: Vec<(f32, FlexTimed)>,
    /// `meteor` 实例：模型、头部面片实体（定义不画头部时没有）、拖尾实体。
    flexeff_meteor: Vec<(
        crate::openpal3::scene::flexeff_meteor::MeteorEffect,
        Option<ComRc<radiance::comdef::IEntity>>,
        ComRc<radiance::comdef::IEntity>,
    )>,
    /// `audio` 那一支——没有画面，只是到点放几个 wav。
    flexeff_audio: Vec<crate::openpal3::scene::flexeff_audio::AudioEffect>,
    /// `callback` 排下来、还在等延迟的特效组：`(组号, 位置, 还差几秒, 深度)`。
    flexeff_pending: Vec<(i32, Vec3, f32, u8)>,
    /// 实体名的流水号，免得同名。
    flexeff_seq: usize,
}

/// 到点才交给战斗命令的特效请求。
#[derive(Debug, Clone)]
enum FlexTimed {
    /// `quake`：`[幅度, 每秒衰减, 频率]`。
    Quake([f32; 3]),
    /// `bink`：`movie/2DEFFECT` 下的名字。
    Movie(String),
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
        // SetAxisAngle 乘 π/180 × 0.5 进四元数，所以角度是度。
        //
        // 转向是**右手 Rodrigues**，已核：`gbQuaternion::GetMatrix`
        // （gbengine `0x10026b60`）按倍积填出 `m00 = 1−yy−zz`、
        // `m01 = xy+wz`、`m10 = xy−wz`，这正是标准列向量旋转矩阵的**转置**；
        // 而引擎用行向量（`gbMatrix4::Transform` `0x100252c0` 算
        // `out.x = x·m00 + y·m10 + z·m20 + m30`），`v·Mᵀ` 就等于把标准右手
        // 旋转作用在 v 上。所以正角 = 绕轴右手定则。
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
            self.water_magic = Some((
                ParticleEffect::with_shapes(&effect, |n| load_par_shape(&asset_mgr, n)),
                entities,
            ));
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

        // `action` 类的提示先攒着：循环里 `asset_mgr` 还借着 `sce_vm`。
        let mut action_cues = vec![];
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
                        let effect =
                            ParticleEffect::with_shapes(&par, |n| load_par_shape(&asset_mgr, n));
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
                // `camera`（FUN_004c39a0）：让战斗镜头播 flexcam 序列。子块 +0 是序列号
                // （实机 54..83），+76 只是这一步的时长，到点并不停镜头。延迟按 +72 算
                // [推断：和其他类同一套时间线，实机全是 0]。
                if class == Some("camera") {
                    for sidx in 0..GROUP_SUBS {
                        if !group.sub_enabled(k, sidx) {
                            continue;
                        }
                        let sub = group.sub(k, sidx).unwrap();
                        let f = |o: usize| f32::from_le_bytes(sub[o..o + 4].try_into().unwrap());
                        let seq = if f(0) > 0. {
                            f(0).round_ties_even() as i32
                        } else {
                            -1
                        };
                        self.flexeff_cam.push((seq, f(72).max(0.)));
                    }
                    continue;
                }
                // `quake`（0x4d35e0）：这一步第一次进来就把 幅度 `+0`、衰减 `+4`、频率 `+8`
                // 写进战斗镜头；`bink`（FUN_004c24b0）：这一步开始后过 `+4` 秒，播
                // `"%s%03d.bik"`（资源前缀 `2DEFFECT\t` + `+0`）；`time`（FUN_004d53e0）改战斗
                // 速度，移植端没有这个量，先不接。
                if matches!(class, Some("quake") | Some("bink") | Some("time")) {
                    if class == Some("time") {
                        log::info!("CEft: time 类（改战斗速度）没接，跳过");
                        continue;
                    }
                    let prefix = def.asset().rsplit('\\').next().unwrap_or("").to_string();
                    for sidx in 0..GROUP_SUBS {
                        if !group.sub_enabled(k, sidx) {
                            continue;
                        }
                        let sub = group.sub(k, sidx).unwrap();
                        let f = |o: usize| f32::from_le_bytes(sub[o..o + 4].try_into().unwrap());
                        if class == Some("quake") {
                            self.flexeff_timed
                                .push((f(72).max(0.), FlexTimed::Quake([f(0), f(4), f(8)])));
                        } else if f(0) != 0. {
                            let name = format!("{prefix}{:03}", f(0).round_ties_even() as i32);
                            self.flexeff_timed
                                .push((f(72).max(0.) + f(4).max(0.), FlexTimed::Movie(name)));
                        }
                    }
                    continue;
                }
                // `action`（FUN_004c0c10）：让舞台上的人做动作、走位、转身，交给战斗命令。
                if class == Some("action") {
                    use crate::openpal3::combat::stage_action::ActionCue;
                    let mut cues = vec![];
                    for sidx in 0..GROUP_SUBS {
                        if group.sub_enabled(k, sidx) {
                            cues.push(ActionCue::from_sub(group.sub(k, sidx).unwrap()));
                        }
                    }
                    action_cues.extend(cues);
                    continue;
                }
                // `model`：摆一个 `.cvd` / `.pol` 模型（scene::flexeff_model）。
                if class == Some("model") {
                    use crate::openpal3::loaders::pol::create_entity_from_pol_model;
                    use crate::openpal3::scene::create_entity_from_cvd_model;
                    use crate::openpal3::scene::flexeff_model::{ModelDef, ModelEffect};
                    use mini_fs::StoreExt;
                    let path = asset_mgr
                        .basedata_path()
                        .join(def.asset().replace('\\', "/"));
                    if asset_mgr.vfs().open(&path).is_err() {
                        log::warn!("CEft: 读不到特效模型 {}", path.display());
                        continue;
                    }
                    let is_pol = path
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("pol"));
                    let md = ModelDef::from_def(&def);
                    let origin = [position.x, position.y, position.z];
                    for sidx in 0..GROUP_SUBS {
                        if !group.sub_enabled(k, sidx) {
                            continue;
                        }
                        let sub = group.sub(k, sidx).unwrap();
                        self.flexeff_seq += 1;
                        let name = format!("FLEXEFF_MODEL_{}", self.flexeff_seq);
                        let entity = if is_pol {
                            create_entity_from_pol_model(
                                &asset_mgr.component_factory(),
                                asset_mgr.vfs(),
                                &path,
                                name,
                                false,
                            )
                        } else {
                            create_entity_from_cvd_model(
                                asset_mgr.component_factory(),
                                asset_mgr.vfs(),
                                &path,
                                name,
                                false,
                            )
                        };
                        scene.add_entity(entity.clone());
                        log::info!("CEft: 模型 {}", def.asset());
                        self.flexeff_model
                            .push((ModelEffect::new(md, sub, origin), entity));
                    }
                    continue;
                }
                // `explosive`：一张图切块炸开（scene::flexeff_explosive）。
                if class == Some("explosive") {
                    use crate::openpal3::scene::flexeff_explosive::{
                        ExplosiveDef, ExplosiveEffect,
                    };
                    use crate::openpal3::scene::flexeff_meteor::build_meteor_entity;
                    let texture_path = asset_mgr
                        .basedata_path()
                        .join(def.asset().replace('\\', "/"));
                    let Some(image) = asset_mgr.read_picture(&texture_path) else {
                        log::warn!("CEft: 读不到碎片贴图 {}", texture_path.display());
                        continue;
                    };
                    let xd = ExplosiveDef::from_def(&def);
                    let origin = [position.x, position.y, position.z];
                    let name = texture_path.to_str().unwrap_or("explosive").to_string();
                    let factory = asset_mgr.component_factory();
                    for sidx in 0..GROUP_SUBS {
                        if !group.sub_enabled(k, sidx) {
                            continue;
                        }
                        let sub = group.sub(k, sidx).unwrap();
                        self.flexeff_seq += 1;
                        let seq = self.flexeff_seq;
                        let effect = ExplosiveEffect::new(xd, sub, origin, seq as u32);
                        let entity = build_meteor_entity(
                            &factory,
                            format!("FLEXEFF_EXPLOSIVE_{seq}"),
                            &name,
                            image.clone(),
                            xd.quad.additive,
                            effect.capacity(),
                        );
                        scene.add_entity(entity.clone());
                        log::info!("CEft: 碎片 {} {}×{}", def.asset(), xd.cols, xd.rows);
                        self.flexeff_explosive.push((effect, entity));
                    }
                    continue;
                }
                // `grid`：扭动起伏的方格面（scene::flexeff_grid）。
                if class == Some("grid") {
                    use crate::openpal3::scene::flexeff_grid::{GridDef, GridEffect};
                    use crate::openpal3::scene::flexeff_meteor::build_meteor_entity;
                    let texture_path = asset_mgr
                        .basedata_path()
                        .join(def.asset().replace('\\', "/"));
                    let Some(image) = asset_mgr.read_picture(&texture_path) else {
                        log::warn!("CEft: 读不到方格贴图 {}", texture_path.display());
                        continue;
                    };
                    let gd = GridDef::from_def(&def);
                    let origin = [position.x, position.y, position.z];
                    let name = texture_path.to_str().unwrap_or("grid").to_string();
                    let factory = asset_mgr.component_factory();
                    for sidx in 0..GROUP_SUBS {
                        if !group.sub_enabled(k, sidx) {
                            continue;
                        }
                        let sub = group.sub(k, sidx).unwrap();
                        self.flexeff_seq += 1;
                        let effect = GridEffect::new(gd, sub, origin);
                        let entity = build_meteor_entity(
                            &factory,
                            format!("FLEXEFF_GRID_{}", self.flexeff_seq),
                            &name,
                            image.clone(),
                            gd.quad.additive,
                            effect.capacity(),
                        );
                        scene.add_entity(entity.clone());
                        log::info!("CEft: 方格 {} {}×{}", def.asset(), gd.cols, gd.rows);
                        self.flexeff_grid.push((effect, entity));
                    }
                    continue;
                }
                // `shock`：一道闪电（scene::flexeff_shock）。
                if class == Some("shock") {
                    use crate::openpal3::scene::flexeff_meteor::build_meteor_entity;
                    use crate::openpal3::scene::flexeff_shock::{ShockDef, ShockEffect};
                    let texture_path = asset_mgr
                        .basedata_path()
                        .join(def.asset().replace('\\', "/"));
                    let Some(image) = asset_mgr.read_picture(&texture_path) else {
                        log::warn!("CEft: 读不到闪电贴图 {}", texture_path.display());
                        continue;
                    };
                    let sd = ShockDef::from_def(&def);
                    let origin = [position.x, position.y, position.z];
                    let name = texture_path.to_str().unwrap_or("shock").to_string();
                    let factory = asset_mgr.component_factory();
                    for sidx in 0..GROUP_SUBS {
                        if !group.sub_enabled(k, sidx) {
                            continue;
                        }
                        let sub = group.sub(k, sidx).unwrap();
                        self.flexeff_seq += 1;
                        let seq = self.flexeff_seq;
                        let effect = ShockEffect::new(sd, sub, origin, seq as u32);
                        let entity = build_meteor_entity(
                            &factory,
                            format!("FLEXEFF_SHOCK_{seq}"),
                            &name,
                            image.clone(),
                            sd.quad.additive,
                            effect.capacity(),
                        );
                        scene.add_entity(entity.clone());
                        self.flexeff_shock.push((effect, entity));
                    }
                    continue;
                }
                // `meteor`：追着目标飞的面片加拖尾（scene::flexeff_meteor）。
                if class == Some("meteor") {
                    use crate::openpal3::scene::flexeff_meteor::{
                        MeteorDef, MeteorEffect, build_meteor_entity,
                    };
                    let texture_path = asset_mgr
                        .basedata_path()
                        .join(def.asset().replace('\\', "/"));
                    let Some(image) = asset_mgr.read_picture(&texture_path) else {
                        log::warn!("CEft: 读不到陨石贴图 {}", texture_path.display());
                        continue;
                    };
                    let md = MeteorDef::from_def(&def);
                    let origin = [position.x, position.y, position.z];
                    let name = texture_path.to_str().unwrap_or("meteor").to_string();
                    let factory = asset_mgr.component_factory();
                    for sidx in 0..GROUP_SUBS {
                        if !group.sub_enabled(k, sidx) {
                            continue;
                        }
                        let sub = group.sub(k, sidx).unwrap();
                        let effect = MeteorEffect::new(md, sub, origin);
                        self.flexeff_seq += 1;
                        let seq = self.flexeff_seq;
                        let heads = md.has_heads().then(|| {
                            let e = build_meteor_entity(
                                &factory,
                                format!("FLEXEFF_METEOR_HEAD_{seq}"),
                                &name,
                                image.clone(),
                                md.quad.additive,
                                effect.head_capacity(),
                            );
                            scene.add_entity(e.clone());
                            e
                        });
                        // 有头部面片时拖尾不贴图（FUN_004cd0d0 的第三个参数），拿一张白图代替。
                        let (trail_name, trail_image) = if md.has_heads() {
                            (
                                "FLEXEFF_METEOR_WHITE".to_string(),
                                image::RgbaImage::from_pixel(
                                    1,
                                    1,
                                    image::Rgba([255, 255, 255, 255]),
                                ),
                            )
                        } else {
                            (name.clone(), image.clone())
                        };
                        let trails = build_meteor_entity(
                            &factory,
                            format!("FLEXEFF_METEOR_TRAIL_{seq}"),
                            &trail_name,
                            trail_image,
                            true,
                            effect.trail_capacity(),
                        );
                        scene.add_entity(trails.clone());
                        log::info!(
                            "CEft: 陨石 {} ×{} 延迟 {} 持续 {} 秒",
                            def.asset(),
                            md.meteors(),
                            f32::from_le_bytes(sub[72..76].try_into().unwrap()),
                            f32::from_le_bytes(sub[76..80].try_into().unwrap())
                        );
                        self.flexeff_meteor.push((effect, heads, trails));
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

        for cue in action_cues {
            log::info!("CEft: 动作 位 {} {:?}", cue.slot, cue.motion_name());
            self.sce_vm.state_mut().request_combat_action(cue);
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

        if !self.flexeff_timed.is_empty() {
            let mut due = vec![];
            self.flexeff_timed.retain_mut(|(left, cue)| {
                *left -= delta_sec;
                if *left <= 0. {
                    due.push(cue.clone());
                    false
                } else {
                    true
                }
            });
            for cue in due {
                match cue {
                    FlexTimed::Quake([amp, decay, freq]) => {
                        log::info!("CEft: 震动 幅度 {amp} 衰减 {decay} 频率 {freq}");
                        self.sce_vm
                            .state_mut()
                            .request_combat_shake(amp, decay, freq);
                    }
                    FlexTimed::Movie(name) => {
                        log::info!("CEft: 2D 动画 {name}");
                        self.sce_vm.state_mut().request_combat_movie(name);
                    }
                }
            }
        }

        if !self.flexeff_cam.is_empty() {
            let mut due = vec![];
            self.flexeff_cam.retain_mut(|(seq, left)| {
                *left -= delta_sec;
                if *left <= 0. {
                    due.push(*seq);
                    false
                } else {
                    true
                }
            });
            for seq in due {
                log::info!("CEft: 镜头 flexcam 序列 {seq}");
                self.sce_vm.state_mut().request_combat_camera(seq);
            }
        }

        if !self.flexeff_model.is_empty() {
            use crate::openpal3::scene::flexeff_model::{place, play_tree, set_visible_tree};
            for (effect, entity) in self.flexeff_model.iter_mut() {
                if effect.step(delta_sec) {
                    set_visible_tree(entity, true);
                    play_tree(entity);
                }
                if effect.visible() {
                    place(entity, effect);
                }
            }
            self.flexeff_model.retain(|(effect, entity)| {
                if effect.finished() {
                    set_visible_tree(entity, false);
                    false
                } else {
                    true
                }
            });
        }

        if !self.flexeff_explosive.is_empty() {
            use crate::openpal3::scene::flexeff_explosive::write_explosive;
            for (effect, entity) in self.flexeff_explosive.iter_mut() {
                effect.step(delta_sec);
                write_explosive(entity, effect);
            }
            self.flexeff_explosive.retain(|(effect, entity)| {
                if effect.finished() {
                    entity.set_visible(false);
                    false
                } else {
                    true
                }
            });
        }

        if !self.flexeff_grid.is_empty() {
            use crate::openpal3::scene::flexeff_grid::write_grid;
            for (effect, entity) in self.flexeff_grid.iter_mut() {
                effect.step(delta_sec);
                write_grid(entity, effect);
            }
            self.flexeff_grid.retain(|(effect, entity)| {
                if effect.finished() {
                    entity.set_visible(false);
                    false
                } else {
                    true
                }
            });
        }

        if !self.flexeff_shock.is_empty() {
            use crate::openpal3::scene::flexeff_shock::write_shock;
            for (effect, entity) in self.flexeff_shock.iter_mut() {
                effect.step(delta_sec);
                write_shock(entity, effect);
            }
            self.flexeff_shock.retain(|(effect, entity)| {
                if effect.finished() {
                    entity.set_visible(false);
                    false
                } else {
                    true
                }
            });
        }

        if !self.flexeff_meteor.is_empty() {
            use crate::openpal3::scene::flexeff_meteor::{write_meteor_heads, write_meteor_trails};
            let (right, up, eye) = {
                let camera = scene.camera();
                let f = camera.transform().matrix().floats();
                (
                    Vec3::new(f[0][0], f[1][0], f[2][0]),
                    Vec3::new(f[0][1], f[1][1], f[2][1]),
                    camera.transform().position(),
                )
            };
            for (effect, heads, trails) in self.flexeff_meteor.iter_mut() {
                effect.step(delta_sec);
                if let Some(h) = heads {
                    write_meteor_heads(h, effect, &right, &up);
                }
                write_meteor_trails(trails, effect, &eye);
            }
            self.flexeff_meteor.retain(|(effect, heads, trails)| {
                if !effect.finished() {
                    return true;
                }
                if let Some(h) = heads {
                    h.set_visible(false);
                }
                trails.set_visible(false);
                false
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

    /// 主角走不通时贴墙滑（`FUN_00414350` 行为 3 那一支，源码是
    /// `Role::UpdateKeyMove` 末尾那段「自动修正位置」，`gamebase/role.cpp:1137`）。
    ///
    /// 四个探测点在**角色自己的坐标系**里写死：
    /// `(±ROLE_RADIUS/2, 0, ROLE_RADIUS/2)` 和 `(±ROLE_RADIUS, 0, 0)`，
    /// `ROLE_RADIUS = 12`，也就是右前、左前各 (6, 6)、正右、正左各 12。
    /// 逐个 `GlobalizePosition` 成世界点再看走不走得通，**不 break**，所以
    /// 最后一个点说了算。走得通时 `newdir = 那个点 − position`（偏移），
    /// 走不通时 `newdir` 留着**那个点的世界坐标**没减——原版就这样，照抄。
    ///
    /// 然后 `newdir.Normalize(); newdir.y = 0;`（先归一化再抹 y），乘上这一步
    /// 原本要走的距离，落点那一格走得通才真挪。
    fn slide_player(
        &mut self,
        scene_manager: &ComRc<ISceneManager>,
        layer: usize,
        role: &ComRc<radiance::comdef::IEntity>,
        pos: &Vec3,
        target: &Vec3,
    ) -> Option<Vec3> {
        let scn = scene_manager.scn_scene()?;
        let scene = scn.inner::<crate::openpal3::scene::ScnScene>();
        let facing = crate::openpal3::scene::ScnScene::facing_of(role);
        // 右 = 前 × 上（`FUN_004122c0`）。
        let right = Vec3::new(-facing.z, 0., facing.x);
        let mut v = Vec3::new(0., 0., 0.);
        let mut found = false;
        for (r, f) in [(6., 6.), (-6., 6.), (12., 0.), (-12., 0.)] {
            let off = Vec3::add(&Vec3::scalar_mul(r, &right), &Vec3::scalar_mul(f, &facing));
            let p = Vec3::add(pos, &off);
            if scene.walkable_at(layer, &p) {
                v = off;
                found = true;
            } else {
                // 角色 +8 是离地相对高度（站地上是 0），这组值的 y 也就是 0。
                v = Vec3::new(p.x, 0., p.z);
            }
        }
        if !found {
            return None;
        }
        let (dx, dz) = (target.x - pos.x, target.z - pos.z);
        let len = (dx * dx + dz * dz).sqrt();
        let n = Vec3::normalized(&v);
        let mut d = Vec3::new(n.x, 0., n.z);
        // 原版这个「上一次滑的方向」是角色自己的成员 `m_lastautodir`，
        // 在 `Role` 初始化时置成 `forward`（`gamebase/role.cpp:3924`），
        // 之后每次贴墙滑都更新（`:1179`）。判定也一模一样：
        // `if (newdir.DotProduct(&m_lastautodir) < -0.1f) newdir = m_lastautodir;`
        // （`:1173`）。这里改成第一次滑的时候才取当时的朝向，差别只在
        // 「角色建出来到第一次贴墙之间转过向」这种情形。
        let prev = self.player_slide_dir.get_or_insert(facing);
        if d.x * prev.x + d.z * prev.z < -0.1 {
            d = *prev;
        }
        *prev = d;
        let mut to = Vec3::add(pos, &Vec3::scalar_mul(len, &d));
        if !scene.walkable_at(layer, &to) {
            return None;
        }
        to.y = scene.get_height(layer, scene.scene_coord_to_nav_coord(layer, &to));
        Some(to)
    }

    /// 场景物件的动作队列和状态（`FUN_00426eb0` 里 `FUN_0042f460` 和状态机那部分）。
    fn step_object_actions(&mut self, dt: f32) {
        let Some(scene_rc) = self.scene_manager.scn_scene() else {
            return;
        };
        let player = self
            .scene_manager
            .get_resolved_role(self.sce_vm.state(), -1);
        let scene = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
        let mut w = ObjWorld {
            scene,
            player,
            state: self.sce_vm.state_mut(),
            walk: &mut self.object_walk,
            procs: vec![],
        };
        scene.step_object_actions(dt, &mut w);
        let procs = std::mem::take(&mut w.procs);
        self.run_object_procs(procs);
    }

    /// 物件动作 4 要跑的过程（`FUN_00432170(id, 0)`）：编号 ≥ 100 在当前场景的 `.sce` 里，
    /// < 100 在 `init.sce` 里（两张表 `DAT_00d8c308` / `DAT_00d8c2e8`，`FUN_00431fb0` 装的）。
    fn run_object_procs(&mut self, procs: Vec<i32>) {
        for id in procs {
            log::debug!("场景物件：跑过程 {id}");
            if id >= 100 {
                self.sce_vm.call_proc(id as u32);
            } else {
                let init = std::rc::Rc::new(self.sce_vm.state().asset_mgr().load_init_sce());
                if !self
                    .sce_vm
                    .state_mut()
                    .call_proc_in(init, "init", id as u32)
                {
                    log::warn!("场景物件：init.sce 里没有 proc {id}");
                }
            }
        }
    }

    /// 能不能跳（`FUN_0041ee60`）：迷宫里（场景类型 2，原版 `DAT_00d8bbec == 2`）、主角脚下那格是跳跃区
    /// （0x2c）或神树传送点（0x26）。返回主角和它所在的层。
    fn jump_spot(&self) -> Option<(ComRc<radiance::comdef::IEntity>, usize)> {
        let scene_rc = self.scene_manager.scn_scene()?;
        let player = self
            .scene_manager
            .get_resolved_role(self.sce_vm.state(), -1)?;
        let rc = RoleController::get_role_controller(player.clone())?;
        let layer = rc.inner::<RoleController>().nav_layer();
        let scene = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
        let pos = player.transform().borrow().position();
        (scene.scene_type() == 2 && scene.jump_zone(layer, &pos)).then_some((player, layer))
    }

    /// 「跳」的图标该不该出、左上角在哪（`FUN_0041d9a0` 0x41dc6d 起）：站在跳跃区上时，主角头顶
    /// （`FUN_00419a80`）再高 10 投到屏幕（归一化坐标，y 朝上）。
    ///
    /// 头顶怎么算已核：`FUN_00419a80` 取角色世界位置 `+0xa0/+0xa4/+0xa8`，
    /// 再把几何节点虚表 `+0x1c` 写出的 gbOBBox 的 `+0x44` 加到 y 上
    /// （`0x419abf` 读的就是那块缓冲区的 +68）。gbOBBox 的三条边长在
    /// `+0x40..+0x48`、`GetRadius` 要乘 0.5，所以 `+0x44` 是**整条 Y 边长，
    /// 即模型高**（和 `lineup_page_layout.md` §177 查到的是同一个字段）——
    /// 移植端用角色模型高是对的。
    fn jump_icon_ndc(&self) -> Option<[f32; 2]> {
        let (player, _) = self.jump_spot()?;
        let rc = RoleController::get_role_controller(player.clone())?;
        let pos = player.transform().borrow().position();
        let head = [
            pos.x,
            pos.y + rc.inner::<RoleController>().model_height() + 10.,
            pos.z,
            1.,
        ];
        let world = self.scene_manager.scene()?;
        let cam = world.camera();
        let m = radiance::math::Mat44::multiplied(cam.projection_matrix(), &cam.view_matrix());
        let r = m.floats();
        let clip = |i: usize| (0..4).map(|k| r[i][k] * head[k]).sum::<f32>();
        let w = clip(3);
        (w > 1e-4).then(|| [clip(0) / w, clip(1) / w])
    }

    /// [移植] 蓄力跳：站在跳跃区上按住 J / 手柄 Y（北）蓄力，松手起跳，按多久对应跳多远
    /// （[`charge_distance`](crate::openpal3::scene::player_jump::charge_distance)：两格到最远 168）。
    /// 原版是右键点地面直接给落点（`FUN_0041d9a0` → `FUN_00414a30`），手柄上没有这一下。
    fn update_jump_charge(&mut self, dt: f32) {
        let held = {
            let input = self.input_engine.borrow();
            input.get_key_state(Key::J).is_down()
                || input.get_key_state(Key::GamePadNorth).is_down()
        };
        if !held {
            if let Some(t) = self.jump_charge.take() {
                self.launch_jump(t);
            }
            return;
        }
        if self.jump_spot().is_none() {
            self.jump_charge = None;
            return;
        }
        self.jump_charge = Some(self.jump_charge.unwrap_or(0.) + dt);
    }

    /// 松手起跳：按住的秒数换成距离，方向取摇杆（没推就朝面向），落点还是原版那一套
    /// （`FUN_00414a30`：拉回 168、取连物件的地面高度、格子直线不通就落在最后一个能走的格子）。
    fn launch_jump(&mut self, held: f32) {
        use crate::openpal3::scene::player_jump::{ANIM, Jump, charge_distance};
        let Some((player, layer)) = self.jump_spot() else {
            return;
        };
        let Some(rc) = RoleController::get_role_controller(player.clone()) else {
            return;
        };
        let Some(scene_rc) = self.scene_manager.scn_scene() else {
            return;
        };
        let scene = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
        let pos = player.transform().borrow().position();
        let (stick, _) = get_moving_direction_and_throw(
            self.input_engine.clone(),
            self.scene_manager.scene().unwrap(),
        );
        let dir = if stick.norm() > 0.5 {
            stick
        } else {
            crate::openpal3::scene::ScnScene::facing_of(&player)
        };
        let n = (dir.x * dir.x + dir.z * dir.z).sqrt();
        if n < 1e-4 {
            return;
        }
        let dist = charge_distance(held);
        let aim = Vec3::new(pos.x + dir.x / n * dist, 0., pos.z + dir.z / n * dist);
        let target = scene.jump_target(layer, &pos, &aim);
        log::debug!(
            "主角起跳：按住 {held:.2} 秒 = {dist:.0}，({:.0}, {:.0}, {:.0}) → ({:.0}, {:.0}, {:.0})",
            pos.x,
            pos.y,
            pos.z,
            target.x,
            target.y,
            target.z
        );
        player
            .transform()
            .borrow_mut()
            .look_at(&Vec3::new(target.x, pos.y, target.z));
        let rc = rc.inner::<RoleController>();
        let anim = rc.play_anim(ANIM, crate::openpal3::scene::RoleAnimationRepeatMode::Hold);
        // 起跳那一下按 4 倍速放（`FUN_00536d00(4.0)`）；`.mv3` 的 hold 标记这里不让动画自己停，
        // 由跳跃按标记调倍率（原版就是这么干的）。
        rc.set_anim_hold_enabled(false);
        rc.set_anim_time_scale(4.);
        self.player_jump = Some((Jump::new(pos, target), anim));
    }

    /// 跳的每一帧（`FUN_004150c0` case 0）；落地后 `FUN_00420b40(主角, dt, 1)`。
    fn step_player_jump(&mut self, dt: f32) {
        let Some((mut jump, anim)) = self.player_jump.take() else {
            return;
        };
        let Some(player) = self
            .scene_manager
            .get_resolved_role(self.sce_vm.state(), -1)
        else {
            return;
        };
        let rc = RoleController::get_role_controller(player.clone());
        let anim_done = !anim
            || rc.as_ref().is_none_or(|rc| {
                rc.inner::<RoleController>().state()
                    != crate::openpal3::scene::RoleState::PlayingAnimation
            });
        // 动作标记：`holds` 起跳做完（换 1 倍速往上飞）、`hold` 空中定格（倍率 0）。
        let (holds, hold) = match rc.as_ref() {
            Some(rc) if anim => {
                let rc = rc.inner::<RoleController>();
                (rc.take_anim_event("holds"), rc.take_anim_event("hold"))
            }
            _ => (false, false),
        };
        let before = jump.phase();
        let (pos, done) = jump.step(dt, holds, anim_done);
        if let Some(rc) = rc.as_ref().filter(|_| anim) {
            let rc = rc.inner::<RoleController>();
            let after = jump.phase();
            if after != before {
                // 起跳做完换 1 倍速，落地那一下 2 倍速（`FUN_004150c0` case 0 的 `FUN_00536d00`）。
                match after {
                    crate::openpal3::scene::player_jump::Phase::Rise => rc.set_anim_time_scale(1.),
                    crate::openpal3::scene::player_jump::Phase::Land => rc.set_anim_time_scale(2.),
                    _ => {}
                }
            }
            // `hold` 只在空中管用（原版是阶段 1 / 2 收这条消息）：落地那一段要按 2 倍速把动作放完。
            if hold
                && matches!(
                    after,
                    crate::openpal3::scene::player_jump::Phase::Rise
                        | crate::openpal3::scene::player_jump::Phase::Fall
                )
            {
                rc.set_anim_time_scale(0.);
            }
        }
        player.transform().borrow_mut().set_position(&pos);
        self.sce_vm
            .global_state_mut()
            .persistent_state_mut()
            .set_position(pos);
        if !done {
            self.player_jump = Some((jump, anim));
            return;
        }
        log::debug!("主角落地：({:.0}, {:.0}, {:.0})", pos.x, pos.y, pos.z);
        let layer = rc
            .as_ref()
            .map_or(0, |rc| rc.inner::<RoleController>().nav_layer());
        if let Some(rc) = &rc {
            rc.inner::<RoleController>().idle();
        }
        let Some(scene_rc) = self.scene_manager.scn_scene() else {
            return;
        };
        let scene = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
        let mut w = ObjWorld {
            scene,
            player: Some(player),
            state: self.sce_vm.state_mut(),
            walk: &mut self.object_walk,
            procs: vec![],
        };
        scene.player_landed(layer, &pos, dt, &mut w);
        let procs = std::mem::take(&mut w.procs);
        self.run_object_procs(procs);
    }

    /// `FUN_00420b40`：主角脚下那格的物件（被动作牵着走、移动方式 4 的时候不查）。
    fn player_on_object(&mut self, dt: f32) {
        if self.object_walk.is_some() {
            return;
        }
        let Some(scene_rc) = self.scene_manager.scn_scene() else {
            return;
        };
        let Some(player) = self
            .scene_manager
            .get_resolved_role(self.sce_vm.state(), -1)
        else {
            return;
        };
        let layer = RoleController::get_role_controller(player.clone())
            .map_or(0, |rc| rc.inner::<RoleController>().nav_layer());
        let pos = player.transform().borrow().position();
        let scene = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
        let mut w = ObjWorld {
            scene,
            player: Some(player),
            state: self.sce_vm.state_mut(),
            walk: &mut self.object_walk,
            procs: vec![],
        };
        scene.player_on_object(layer, &pos, dt, &mut w);
        let procs = std::mem::take(&mut w.procs);
        self.run_object_procs(procs);
    }

    fn move_role(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        delta_sec: f32,
        moving_direction: &Vec3,
        throw: f32,
    ) {
        // 栈顶不是冒险场景（比如战斗舞台压上来了）时找不到主角，这一帧就不挪。
        let Some(role) = scene_manager.get_resolved_role(self.sce_vm.state(), -1) else {
            return;
        };
        let Some(role_controller) = RoleController::get_role_controller(role.clone()) else {
            return;
        };
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
        let (target_nav_coord, height, walkable) = {
            let scn = scene_manager.scn_scene().unwrap();
            let scene = scn.inner::<crate::openpal3::scene::ScnScene>();
            let tnc = scene.scene_coord_to_nav_coord(nav_layer, &target_position);
            // 地面高度照 `FUN_00412230` → `FUN_00421230`：格子上有吊桥、乌龟这类物件时站在物件上。
            let h = scene.floor_height(nav_layer, &target_position);
            // 能不能走照原版 `FUN_00421020`：格子记录 +4 字节 > 1，再按格子上的场景物件改判。
            let d = scene.walkable_at(nav_layer, &target_position);
            (tnc, h, d)
        };
        let _ = target_nav_coord;
        target_position.y = height;

        let role = scene_manager
            .get_resolved_role(self.sce_vm.state(), -1)
            .unwrap();
        let free = self.sce_vm.global_state().pass_through_wall() || walkable;
        let slid = if moving_direction.norm() > 0.5 && !free {
            self.slide_player(
                &scene_manager,
                nav_layer,
                &role,
                &position,
                &target_position,
            )
        } else {
            None
        };
        if moving_direction.norm() > 0.5 && (free || slid.is_some()) {
            match mode {
                MoveMode::Run => role_controller.inner::<RoleController>().run(),
                _ => role_controller.inner::<RoleController>().walk(),
            }
            // 朝向跟速度（`FUN_004122c0`）——滑的时候也是输入的方向。
            let look_at = Vec3::new(target_position.x, position.y, target_position.z);
            let dest = slid.unwrap_or(target_position);
            role.transform()
                .borrow_mut()
                .look_at(&look_at)
                .set_position(&dest);

            self.sce_vm
                .global_state_mut()
                .persistent_state_mut()
                .set_position(dest);

            position = dest
        } else if self.object_walk.is_none() && self.player_jump.is_none() {
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

    /// 野外遇敌：主角停下（`FUN_004181e0` 调在主角身上），按撞上的怪开一场野外战斗。战斗命令一初始化就关掉
    /// 冒险输入，所以不会一帧里连撞几次。
    fn check_field_encounter(&mut self) -> bool {
        use crate::openpal3::scene::RoleController;
        let Some(scene_rc) = self.scene_manager.scn_scene() else {
            return false;
        };
        let Some(player) = self
            .scene_manager
            .get_resolved_role(self.sce_vm.state(), -1)
        else {
            return false;
        };
        let radius = RoleController::get_role_controller(player.clone())
            .map(|rc| rc.inner::<RoleController>().model_radius())
            .unwrap_or(crate::openpal3::scene::DEFAULT_MODEL_RADIUS);
        let Some(hit) = scene_rc
            .inner::<crate::openpal3::scene::ScnScene>()
            .touching_field_monster(&player, radius)
        else {
            return false;
        };
        if let Some(rc) = RoleController::get_role_controller(player.clone()) {
            rc.inner::<RoleController>().idle();
        }
        log::info!(
            "Field: 撞上 ROLE_{}（{:?} × {}），朝向判定 {}",
            hit.role_index,
            hit.ids,
            hit.count,
            hit.sneak
        );
        let cmd = Box::new(
            crate::scripting::sce::commands::SceCommandCombat::new_field(
                hit.count as i32,
                hit.ids,
                hit.sneak,
                hit.role_index,
            ),
        );
        self.sce_vm
            .push_debug_command(cmd, self.scene_manager.clone());
        true
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
        // 类型 17 物件的待机重播 / 浮动 / 旋转同样不受过场影响（FUN_00426eb0）。
        if let Some(scene_rc) = self.scene_manager.scn_scene() {
            let scn = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
            scn.step_object_moves(delta_sec);
            scn.step_scene_objects(delta_sec);
        }
        // 场景物件的动作队列和状态（`FUN_00426eb0`）同样每帧推，过场里也推。
        self.step_object_actions(delta_sec);

        // Weather is drawn over the whole frame in the original, after
        // the scene graph ends, so it keeps running through cutscenes.
        self.step_snow(delta_sec);
        self.step_water_magic(delta_sec);
        self.step_flexeff(delta_sec);

        if !adv_enabled {
            // [移植] 过场里把「跳」的图标收起（原版这时不更新它）。
            self.sce_vm.state_mut().set_jump_icon(None);
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

        // 冻结计数小于 1（场景物件的动作 8）：主角不听输入（`FUN_0041d9a0`）、野怪停住、不撞怪、不查脚下物件。
        let frozen = self.sce_vm.global_state().frozen();
        // 跳的时候主角是行为 4：不听输入、不查脚下物件和触发区（`FUN_0041d9a0` / `FUN_00422e40`）。
        if self.player_jump.is_some() {
            self.step_player_jump(delta_sec);
        } else if !frozen {
            self.update_jump_charge(delta_sec);
        }
        // 蓄力时主角站着不动（原版瞄落点的时候也不走），但头上的图标照出。
        let frozen = frozen || self.player_jump.is_some() || self.jump_charge.is_some();
        // 头上「跳」的图标跟着 `FUN_0041d9a0` 每帧更新；冻着、跳着的时候那个函数不跑，图标保持原样。
        if !frozen {
            let icon = self.jump_icon_ndc();
            self.sce_vm.state_mut().set_jump_icon(icon);
        }
        let (moving_direction, throw) = if frozen {
            (Vec3::new(0., 0., 0.), 0.)
        } else {
            get_moving_direction_and_throw(
                self.input_engine.clone(),
                self.scene_manager.scene().unwrap(),
            )
        };
        self.move_role(
            self.scene_manager.clone(),
            delta_sec,
            &moving_direction,
            throw,
        );
        // 主角脚下的场景物件（`FUN_00422e40` → `FUN_00420b40`）。
        if !frozen {
            self.player_on_object(delta_sec);
        }

        // Advance ambient NPC patrols (non-scripted townsfolk walking their
        // authored loop). Scripted roles are untouched.
        {
            let scene_rc = self.scene_manager.scn_scene().unwrap();
            scene_rc
                .inner::<crate::openpal3::scene::ScnScene>()
                .step_patrols(delta_sec);
        }

        // 野外怪物 AI（`FUN_00417420`）。
        {
            let gs = self.sce_vm.global_state();
            if let (Some(db), Some(player)) = (
                gs.game_db().cloned(),
                self.scene_manager
                    .get_resolved_role(self.sce_vm.state(), -1),
            ) {
                let layer = RoleController::get_role_controller(player.clone())
                    .map(|rc| rc.inner::<RoleController>().nav_layer())
                    .unwrap_or(0);
                let monster = *gs.monster_constants();
                let role = *gs.role_constants();
                if let Some(scene_rc) = self.scene_manager.scn_scene() {
                    scene_rc
                        .inner::<crate::openpal3::scene::ScnScene>()
                        .step_field_monsters(
                            delta_sec, &player, layer, &db, &monster, &role, frozen,
                        );
                }
            }
        }
        // 野外怪物撞上主角就开战（`FUN_00417420` 末尾 → `FUN_00407560`）。战斗命令一初始化就把舞台
        // 压到场景栈顶，这一帧后面按冒险场景取主角的代码就不能再跑了。
        if !frozen && self.check_field_encounter() {
            return None;
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
        let proc_id_opt = proc_id_opt.filter(|_| !frozen);
        let entered = proc_id_opt.filter(|id| self.last_nav_trigger != Some(*id));
        self.last_nav_trigger = proc_id_opt;
        if let Some(proc_id) = entered {
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
        if !frozen
            && (input.get_key_state(Key::F).pressed()
                || input.get_key_state(Key::Space).pressed()
                || input.get_key_state(Key::GamePadEast).pressed())
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

/// 场景物件的动作 2 牵着主角走（`FUN_00412b60` 设的移动方式 4：起点 +0x554、终点 +0x540、按走的速度）。
#[derive(Debug, Clone, Copy)]
struct ObjectWalk {
    start: Vec3,
    target: Vec3,
    speed: f32,
}

/// 场景物件的动作要碰的主角、模型、音效、冻结计数。
struct ObjWorld<'a> {
    scene: &'a crate::openpal3::scene::ScnScene,
    player: Option<ComRc<radiance::comdef::IEntity>>,
    state: &'a mut crate::scripting::sce::SceState,
    walk: &'a mut Option<ObjectWalk>,
    /// 动作 4 要跑的过程，推完一帧再交给 SCE 虚拟机。
    procs: Vec<i32>,
}

impl ObjWorld<'_> {
    /// 记一条物件状态进存档（换场景、读档回来照它建）。
    fn save_override(
        &mut self,
        i: usize,
        f: impl FnOnce(&mut crate::openpal3::states::persistent_state::PersistentState, &str, &str, i32),
    ) {
        let (city, scene) = (
            self.scene.name().to_string(),
            self.scene.sub_name().to_string(),
        );
        let Some(id) = self.scene.object_id_at(i) else {
            return;
        };
        let mut p = self.state.global_state_mut().persistent_state_mut();
        f(&mut p, &city, &scene, id);
    }

    fn player_layer(&self) -> usize {
        self.player
            .as_ref()
            .and_then(|p| RoleController::get_role_controller(p.clone()))
            .map_or(0, |rc| rc.inner::<RoleController>().nav_layer())
    }

    fn set_player(&mut self, p: &Vec3) {
        if let Some(e) = &self.player {
            e.transform().borrow_mut().set_position(p);
        }
        self.state
            .global_state_mut()
            .persistent_state_mut()
            .set_position(*p);
    }

    fn idle(&self) {
        if let Some(rc) = self
            .player
            .as_ref()
            .and_then(|p| RoleController::get_role_controller(p.clone()))
        {
            rc.inner::<RoleController>().idle();
        }
    }

    fn cvds(&self, i: usize) -> Vec<ComRc<crate::openpal3::comdef::ICvdModel>> {
        let mut v = vec![];
        if let Some(e) = self.scene.object_entity_at(i) {
            crate::openpal3::scene::cvd_components(&e, &mut v);
        }
        v
    }
}

impl crate::openpal3::scene::object_action::World for ObjWorld<'_> {
    fn active(&self, i: usize) -> bool {
        self.scene.object_active_at(i)
    }

    fn object_pos(&self, i: usize) -> Vec3 {
        self.scene
            .object_entity_at(i)
            .map_or(Vec3::new(0., 0., 0.), |e| e.transform().borrow().position())
    }

    fn set_object_pos(&mut self, i: usize, p: &Vec3) {
        if let Some(e) = self.scene.object_entity_at(i) {
            e.transform().borrow_mut().set_position(p);
        }
        let p = *p;
        self.save_override(i, |s, city, scene, id| {
            s.set_scene_object_position(city, scene, id, p)
        });
    }

    fn player_pos(&self) -> Vec3 {
        self.player
            .as_ref()
            .map_or(Vec3::new(0., 0., 0.), |e| e.transform().borrow().position())
    }

    /// `FUN_00416490`：放到这一点，再按脚下重算高度（`FUN_00412230`）。
    fn carry_player(&mut self, p: &Vec3) {
        let mut q = *p;
        q.y = self.scene.floor_height(self.player_layer(), &q);
        self.set_player(&q);
    }

    fn fix_player_height(&mut self) {
        let mut p = self.player_pos();
        p.y = self.scene.floor_height(self.player_layer(), &p);
        self.set_player(&p);
    }

    fn play_anim(&mut self, i: usize, speed: f32) {
        log::debug!("场景物件：#{i} 播动画");
        for c in self.cvds(i) {
            c.inner::<crate::openpal3::scene::CvdModelComponent>()
                .play(crate::openpal3::scene::CvdPlayMode::Once, speed);
        }
    }

    fn anim_done(&self, i: usize) -> bool {
        self.cvds(i).iter().all(|c| {
            c.inner::<crate::openpal3::scene::CvdModelComponent>()
                .finished()
        })
    }

    /// `FUN_0041e720`：冻住时主角停下。
    fn freeze(&mut self, release: bool) {
        self.state.global_state_mut().add_object_freeze(release);
        log::debug!(
            "场景物件：冻结计数 {}",
            self.state.global_state().object_freeze()
        );
        if self.state.global_state().frozen() && self.walk.is_none() {
            self.idle();
        }
    }

    fn set_passable(&mut self, i: usize, passable: bool) {
        log::debug!("场景物件：#{i} 可通过 {passable}");
        self.scene.set_object_passable(i, passable);
    }

    /// `FUN_00416550`：朝 (x, z) 方向转，离得不到 0.005 不转。
    fn face_player(&mut self, p: &Vec3) {
        let Some(e) = &self.player else {
            return;
        };
        let pos = e.transform().borrow().position();
        let (dx, dz) = (p.x - pos.x, p.z - pos.z);
        if (dx * dx + dz * dz).sqrt() >= 0.005 {
            e.transform()
                .borrow_mut()
                .look_at(&Vec3::new(p.x, pos.y, p.z));
        }
    }

    /// `FUN_00412b60(to, 1)`：离目标（x、z）不到 12 就不走（移动方式 0）。
    fn walk_begin(&mut self, to: &Vec3) {
        let pos = self.player_pos();
        let (dx, dz) = (to.x - pos.x, to.z - pos.z);
        if (dx * dx + dz * dz).sqrt() < 12. {
            *self.walk = None;
            return;
        }
        let speed = self
            .state
            .global_state()
            .role_constants()
            .speed(MoveMode::Walk, true, 0.);
        log::debug!(
            "场景物件：主角从 ({:.0}, {:.0}) 走到 ({:.0}, {:.0})，速度 {speed}",
            pos.x,
            pos.z,
            to.x,
            to.z
        );
        *self.walk = Some(ObjectWalk {
            start: pos,
            target: *to,
            speed,
        });
        if let Some(rc) = self
            .player
            .as_ref()
            .and_then(|p| RoleController::get_role_controller(p.clone()))
        {
            rc.inner::<RoleController>().walk();
        }
    }

    /// `FUN_0042f460` case 2：先 `FUN_00413cd0` case 4（在起点→终点上的投影到了终点前 0.1 以内就停、放到终点，
    /// `FUN_00414350` + `FUN_00412230`），没到就按 归一化(终点 − 位置) × 走的速度 挪一步；`check` 时落点不能走
    /// 就停在原地，`fix` 时按脚下重算高度、否则保持原高度。
    fn walk_step(&mut self, dt: f32, check: bool, fix: bool) -> bool {
        let Some(wk) = *self.walk else {
            return true;
        };
        let pos = self.player_pos();
        let layer = self.player_layer();
        let (sx, sz) = (wk.target.x - wk.start.x, wk.target.z - wk.start.z);
        let len = (sx * sx + sz * sz).sqrt();
        let (rx, rz) = (pos.x - wk.start.x, pos.z - wk.start.z);
        let arrived = len < 5e-5 || {
            let proj = (rx * sx + rz * sz) / len;
            proj >= 0. && proj - len > -0.1
        };
        if arrived {
            let mut p = Vec3::new(wk.target.x, pos.y, wk.target.z);
            if self.scene.walkable_at(layer, &p) {
                p.y = self.scene.floor_height(layer, &p);
                self.set_player(&p);
            }
            log::debug!("场景物件：主角走到了 ({:.0}, {:.1}, {:.0})", p.x, p.y, p.z);
            *self.walk = None;
            self.idle();
            return true;
        }
        let (dx, dz) = (wk.target.x - pos.x, wk.target.z - pos.z);
        let n = (dx * dx + dz * dz).sqrt();
        let (vx, vz) = if n > 0. {
            (dx / n * wk.speed, dz / n * wk.speed)
        } else {
            (0., 0.)
        };
        let mut next = Vec3::new(pos.x + vx * dt, pos.y, pos.z + vz * dt);
        if check && !self.scene.walkable_at(layer, &next) {
            log::debug!("场景物件：主角走到 ({:.0}, {:.0}) 被挡住", pos.x, pos.z);
            *self.walk = None;
            self.idle();
            return true;
        }
        if fix {
            next.y = self.scene.floor_height(layer, &next);
        }
        if let Some(e) = &self.player {
            e.transform()
                .borrow_mut()
                .look_at(&Vec3::new(next.x + vx, next.y, next.z + vz));
        }
        self.set_player(&next);
        false
    }

    fn sound(&mut self, name: &str) {
        log::debug!("场景物件：音效 {name}");
        crate::scripting::sce::commands::play_sfx(self.state, name);
    }

    fn call_proc(&mut self, id: i32) {
        self.procs.push(id);
    }

    fn proc_running(&self) -> bool {
        self.state.context().is_running()
    }

    fn save_state(&mut self, i: usize, state: u8) {
        self.save_override(i, |s, city, scene, id| {
            s.set_scene_object_state(city, scene, id, state as i32)
        });
    }

    fn object_tiles(&mut self, i: usize, clear: bool, layer: usize) {
        log::debug!(
            "场景物件：#{i} {}格子（层 {layer}）",
            if clear { "清掉" } else { "重新登记" }
        );
        self.scene.set_object_tiles(i, clear, layer);
        if !clear {
            self.save_override(i, |s, city, scene, id| {
                s.set_scene_object_layer(city, scene, id, layer as u8)
            });
        }
    }

    fn set_player_layer(&mut self, layer: usize) {
        log::debug!("场景物件：主角换到层 {layer}");
        if let Some(rc) = self
            .player
            .as_ref()
            .and_then(|p| RoleController::get_role_controller(p.clone()))
        {
            rc.inner::<RoleController>().set_nav_layer(layer);
        }
    }

    fn object_model_height(&self, i: usize) -> f32 {
        self.scene.object_model_height(i)
    }

    fn tile_point(&self, layer: usize, x: i32, z: i32) -> (Vec3, f32) {
        self.scene.tile_point(layer, x, z)
    }

    fn party_size(&self) -> usize {
        self.state.global_state().persistent_state().team().len()
    }

    /// `FUN_00430570(key)` 取 `lang.txt` 的文字、`FUN_00450bc0` 弹提示框（和脚本 68 同一个框）。
    fn hint(&mut self, key: &str) {
        let text = common::store_ext::StoreExt2::read_to_end_from_gbk(
            self.state.asset_mgr().vfs(),
            crate::openpal3::combat::result::LANG_PATH,
        )
        .ok()
        .and_then(|t| crate::openpal3::combat::result::lang_value(&t, key));
        match text {
            Some(t) => {
                log::debug!("场景物件：提示「{key}」= {t}");
                self.state.show_note(t);
            }
            None => log::warn!("场景物件：lang.txt 里没有 {key}"),
        }
    }
}

/// 读 `.par` 里形状 2 的发射器指的那份 `.csf` 点云。名字是相对
/// `effect\gbParticle` 目录的反斜杠写法。
fn load_par_shape(
    asset_mgr: &std::rc::Rc<crate::openpal3::asset_manager::AssetManager>,
    name: &str,
) -> Option<Vec<[f32; 3]>> {
    use fileformats::pal3::csf::read_csf;
    let path = asset_mgr.particle_root().join(name.replace('\\', "/"));
    let data = common::store_ext::StoreExt2::read_to_end(asset_mgr.vfs(), &path).ok()?;
    match read_csf(&mut data.as_slice()) {
        Ok(shape) => Some(shape.points),
        Err(e) => {
            log::warn!("读不开 {}: {e}", path.display());
            None
        }
    }
}
