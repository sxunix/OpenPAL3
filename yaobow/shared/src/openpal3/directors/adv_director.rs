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
                agent_bridge: None,
                player_slide_dir: None,
                object_walk: None,
                player_fly: None,
                hurt_time: 0.,
                ram_fly: None,
                last_nav_trigger: None,
                scene_slots: Default::default(),
                input_lock: false,
                agent_walk: None,
                agent_walk_goal: None,
                agent_interact: false,
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

        // 读档时由存档里记的那个主角受控（`SetCtrl(loc.underctrl, false)`，`Archive.cpp:278`）：
        // 它站到存档位置上、在场，景天（建场景时默认在场的那个）换下。
        let controlled = global_state.persistent_state().controlled_role();
        let scn = scene_manager.scn_scene().unwrap();
        let scn = scn.inner::<crate::openpal3::scene::ScnScene>();
        let role_entity = scn
            .get_role_entity(controlled)
            .or_else(|| scn.get_role_entity(0))
            .unwrap();
        if controlled != 0 {
            if let Some(rc) = scn
                .get_role_entity(0)
                .and_then(RoleController::get_role_controller)
            {
                rc.inner::<RoleController>().set_active(false);
            }
        }
        global_state.set_role_controlled(controlled);

        let role = RoleController::get_role_controller(role_entity.clone()).unwrap();
        role.inner::<RoleController>().set_active(true);
        let layer = global_state.persistent_state().player_layer() as usize;
        role.inner::<RoleController>().set_nav_layer(layer);
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

        // 存档里记的物件状态（拨过的开关、清掉的机关、放下的吊桥、脚本藏起的物件……）套回这个场景，
        // 再跑进场脚本——和脚本换场景（`SceCommandLoadScene::load`）同一个顺序。
        crate::scripting::sce::commands::SceCommandLoadScene::new(
            scene_name.clone().unwrap(),
            sub_scene_name.clone().unwrap(),
        )
        .restore_after_save_load(scene_manager.clone(), sce_vm.state_mut());

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
                agent_bridge: None,
                player_slide_dir: None,
                object_walk: None,
                player_fly: None,
                hurt_time: 0.,
                ram_fly: None,
                last_nav_trigger: None,
                scene_slots: Default::default(),
                input_lock: false,
                agent_walk: None,
                agent_walk_goal: None,
                agent_interact: false,
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
    /// [调试] 主角为什么不听输入：脚本锁、飞行、冻结计数、物件锁（登记着的物件序号、类型、状态）、agent 走路。
    pub fn debug_locks(&self) -> serde_json::Value {
        let p = self.props.borrow();
        let objects = p
            .scene_manager
            .scn_scene()
            .map(|s| {
                s.inner::<crate::openpal3::scene::ScnScene>()
                    .object_lock_report()
            })
            .unwrap_or_default();
        serde_json::json!({
            "input_lock": p.input_lock,
            "script_running": p.sce_vm.state().context().is_running(),
            "active_commands": p.sce_vm.has_active_commands(),
            "adv_input": p.sce_vm.global_state().adv_input_enabled(),
            "fly": p.player_fly.is_some(),
            "object_walk": p.object_walk.is_some(),
            "frozen": p.sce_vm.global_state().frozen(),
            "player_jump": p.player_jump.is_some(),
            "agent_walk": p.agent_walk.as_ref().map(|(w, _)| w.len()),
            "agent_goal": p.agent_walk_goal.as_ref().map(|g| (g.point.x, g.point.z, g.resumes)),
            "objects": objects,
        })
    }

    /// [调试] 全队回满（同指令 `FullTeamAtt`），探路工具连着打仗时用。
    pub fn heal_team(&self) {
        let mut p = self.props.borrow_mut();
        let scene_manager = p.scene_manager.clone();
        let cmd: Box<dyn crate::scripting::sce::SceCommand> =
            Box::new(crate::scripting::sce::commands::SceCommandFullTeamAtt::new());
        p.sce_vm.push_debug_command(cmd, scene_manager);
    }

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

    /// 调试：正在打的战斗直接判胜。
    pub fn request_combat_win(&self) {
        self.props
            .borrow_mut()
            .sce_vm
            .state_mut()
            .request_combat_win();
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
    /// 传送主角并把导航层切到 `layer`。导航格触发（类型 0 等）只认主角**当前层**
    /// 上的格子（原版 `Scene::InterActRole` 查的是 `m_objmap[role->m_onlayer]`），
    /// 只改位置不改层的话，从二楼出来再传到一楼的触发区是不会触发的。
    /// 调试用：让主角从现在的位置一帧一格走到第 `nav_index` 个走格触发点（或 `point` 旁边），
    /// `interact` 到了按一下调查键。返回路上几格，到不了是 `None`。
    pub fn agent_walk_to(
        &self,
        nav_index: Option<usize>,
        point: Vec3,
        interact: bool,
    ) -> Option<usize> {
        let mut p = self.props.borrow_mut();
        let path = p.plan_agent_walk(nav_index, &point)?;
        let n = path.len();
        let goal_proc = nav_index.and_then(|i| {
            p.scene_manager.scn_scene().and_then(|s| {
                s.inner::<crate::openpal3::scene::ScnScene>()
                    .nav_trigger_proc(i)
            })
        });
        p.agent_walk = Some((path.into_iter().collect(), interact));
        p.agent_walk_goal = Some(AgentWalkGoal {
            nav_index,
            point,
            interact,
            goal_proc,
            scene: p.scene_key(),
            resumes: 0,
            free_resume: false,
        });
        Some(n)
    }

    /// 调试用：agent 带着主角走的路还没走完。
    /// 存档前补上存档里要有的东西（见 `AdventureDirectorProps::record_for_save`）。
    pub fn record_for_save(&self) {
        self.props.borrow().record_for_save();
    }

    pub fn agent_walking(&self) -> bool {
        let p = self.props.borrow();
        p.agent_walk.is_some() || p.agent_walk_goal.is_some()
    }

    /// 调试用：场景物件正在动（动作队列没做完，或者登记在 `ObjLockInput` 里还没闲下来——开关、
    /// 特技机关拨到一半）。这时主角不听输入。
    pub fn object_busy(&self) -> bool {
        let p = self.props.borrow();
        p.player_fly.is_some()
            || p.ram_fly.is_some()
            || p.scene_manager.scn_scene().is_some_and(|s| {
                let s = s.inner::<crate::openpal3::scene::ScnScene>();
                s.object_input_locked() || s.object_actions_busy()
            })
    }

    /// 调试用：主角现在这层上能不能走到 `to`（见 [`ScnScene::agent_reachable`]）。
    pub fn agent_reachable(
        &self,
        to: &[Vec3],
    ) -> Option<(Vec<bool>, usize, Vec<String>, Vec<usize>)> {
        let p = self.props.borrow();
        let scene = p.scene_manager.scn_scene()?;
        let role = p.scene_manager.get_resolved_role(p.sce_vm.state(), -1)?;
        let layer = RoleController::get_role_controller(role.clone())
            .map_or(0, |rc| rc.inner::<RoleController>().nav_layer());
        let from = role.transform().borrow().position();
        Some(
            scene
                .inner::<crate::openpal3::scene::ScnScene>()
                .agent_reachable(layer, &from, to),
        )
    }

    pub fn teleport_controlled_role_on_layer(&self, pos: Vec3, layer: usize) -> bool {
        if !self.teleport_controlled_role(pos) {
            return false;
        }
        let p = self.props.borrow();
        if let Some(role) = p.scene_manager.get_resolved_role(p.sce_vm.state(), -1) {
            if let Some(rc) = RoleController::get_role_controller(role) {
                rc.inner::<RoleController>().set_nav_layer(layer);
            }
        }
        true
    }

    /// 主角转向 (x, z)（agent 用：推箱子要从哪一边推，取决于按调查键时朝哪边、站哪边）。
    pub fn face_controlled_role(&self, x: f32, z: f32) -> bool {
        let p = self.props.borrow();
        let Some(role) = p.scene_manager.get_resolved_role(p.sce_vm.state(), -1) else {
            return false;
        };
        let pos = role.transform().borrow().position();
        role.transform()
            .borrow_mut()
            .look_at(&Vec3::new(x, pos.y, z));
        true
    }

    /// 找一个朝向，让调查键正好选中物件 `i`（[`ScnScene::select_examine_object`] 比的是最后一个进视野的
    /// 物件的距离，正对一排箱子时远处那个会把近处这个挤掉），转过去。找不到返回 `false`。
    pub fn face_to_select(&self, i: usize) -> bool {
        let p = self.props.borrow();
        let Some(scene_rc) = p.scene_manager.scn_scene() else {
            return false;
        };
        let Some(role) = p.scene_manager.get_resolved_role(p.sce_vm.state(), -1) else {
            return false;
        };
        let layer = RoleController::get_role_controller(role.clone())
            .map_or(0, |rc| rc.inner::<RoleController>().nav_layer());
        let scene = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
        let pos = role.transform().borrow().position();
        for k in 0..72 {
            let a = (k as f32 * 5.).to_radians();
            let f = Vec3::new(a.sin(), 0., a.cos());
            if scene.select_examine_object(&pos, &f, layer) == Some(i) {
                role.transform()
                    .borrow_mut()
                    .look_at(&Vec3::new(pos.x + f.x, pos.y, pos.z + f.z));
                return true;
            }
        }
        false
    }

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
    /// Agent-server bridge. `None` when the embedded HTTP listener
    /// is not running; `Some(_)` gates pause/step + provides the
    /// synthetic-input overlay backing `/v1/input/*`.
    agent_bridge: Option<Rc<AgentBridge>>,
    /// 主角上一次贴墙滑的方向（角色 `+0x534`，`FUN_0041bad0` 初始化成朝向）。
    player_slide_dir: Option<Vec3>,
    /// 场景物件的动作 2 正牵着主角走（`FUN_00412b60` 设的移动方式 4）。
    object_walk: Option<ObjectWalk>,
    /// 主角在「旋转飞」（`Role::RotateFly`，食人花吞进去、吐出来）。
    player_fly: Option<PlayerFly>,
    /// `SceneMgr::m_fHurtTime`：每帧加，被场景机关伤到时清零（冲撞要它超过 1.2 才撞）。
    hurt_time: f32,
    /// 被冲撞撞飞：（速度，已飞的秒数，落地的层和格）。
    ram_fly: Option<(Vec3, f32, (usize, i32, i32))>,
    /// 上一帧主角站着的导航触发区的过程号。原版类型 0 触发点（`FUN_00424c20` case 0）只在「上一帧不在同一个物件上」
    /// 时跑过程，站着不动不会每帧重跑。
    last_nav_trigger: Option<u32>,
    /// 原版场景按类型分两槽放（`SceneMgr::s_scene`，`SceneIndex`：城镇 0 和战斗关 2 共用一槽、建筑内部 1
    /// 一槽）。换进来的场景和这一槽里放着的是同一个时只 `LoadData(file, TRUE)`，不 `Destroy()`，
    /// 「上一次站的物件」`m_lastobj` 留着（`scenemgr.cpp:134`、`scene.cpp` `LoadData`）——从城镇进房间
    /// 再出来，落脚点在进门的那扇门里也不会立刻又进去。这里每槽记（场景，那时的
    /// [`Self::last_nav_trigger`]），第三项是现在所在的场景和槽。
    scene_slots: (
        [Option<((String, String), Option<u32>)>; 2],
        Option<((String, String), usize)>,
    ),
    /// 主角碰出来的过程还在跑（原版 `ObjLockInput`：物件、对话、调查起的脚本连同它 `Call` 出去的
    /// 都登记进去，`RoleCtrl::Update` 见它非空就不处理主角操作，`SceneMgr::Update` 也不查脚下物件，
    /// `rolectrl.cpp` / `scenemgr.cpp:315`）。移植端的脚本机只有一条调用栈，碰出来的过程跑着时
    /// 再按调查键会把同一个过程又压一层——爬梯子的过程就这样被反复从梯子底下重爬。
    input_lock: bool,
    /// [调试] agent 让主角照 [`ScnScene::agent_path`] 一帧一格走过去（探路工具用，传送会跨过路上的
    /// 走格触发）。第二项：走到了按一下调查键。脚本接手（上锁、冻结）就作罢。
    agent_walk: Option<(std::collections::VecDeque<(usize, Vec3)>, bool)>,
    /// [调试] 正在走的那趟要去哪（走到半路碰出过程会停下，过程跑完、还在同一个场景就从当下的
    /// 位置重新找路接着走——原版玩家按着方向键，过程放开后照样往前走）。
    agent_walk_goal: Option<AgentWalkGoal>,
    /// [调试] 下一帧当调查键按下了（agent 走到地方以后）。
    agent_interact: bool,
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

        if save_slot > 0 {
            self.record_for_save();
        }
        self.sce_vm
            .global_state()
            .persistent_state()
            .save(save_slot);
    }

    /// 存档前把存档里要有、平时不随时写进去的东西补上：主角现在的位置和所在层（原版存的是主角
    /// 当前的位置；移植端有几处挪主角的地方只挪了实体，存档位置会落后）。
    fn record_for_save(&self) {
        let Some(role) = self
            .scene_manager
            .get_resolved_role(self.sce_vm.state(), -1)
        else {
            return;
        };
        let layer = RoleController::get_role_controller(role.clone())
            .map_or(0, |rc| rc.inner::<RoleController>().nav_layer());
        let pos = role.transform().borrow().position();
        let mut p = self
            .sce_vm
            .global_state()
            .persistent_state_shared()
            .borrow_mut();
        p.set_player_layer(layer as u8);
        p.set_position(pos);
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

    /// 调查键没找到人（`SceneMgr::GetSelRole_RoleCtrl`）时选物件（`FUN_00420cd0`）、交给它
    /// （`Role::Check` → `InterActRole(主角, 1, 0, 0.001)`）。只接移植端做了的物件类型（推箱子 0xb、
    /// 特技机关 0xc、开关 0x17），做了返回 `true`；别的类型照旧走后面的老路子（梯子 / 藤等）。
    fn examine_object(&mut self, position: &Vec3, layer: usize) -> bool {
        let Some(scene_rc) = self.scene_manager.scn_scene() else {
            return false;
        };
        let player = self
            .scene_manager
            .get_resolved_role(self.sce_vm.state(), -1);
        let forward = player
            .as_ref()
            .map(crate::openpal3::scene::ScnScene::facing_of)
            .unwrap_or(Vec3::new(0., 0., 1.));
        let scene = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
        let Some(i) = scene.select_examine_object(position, &forward, layer) else {
            return false;
        };
        debug!("调查键选中物件 #{i}（类型 {:?}）", scene.object_kind_at(i));
        let mut w = ObjWorld {
            scene,
            player,
            state: self.sce_vm.state_mut(),
            walk: &mut self.object_walk,
            fly: &mut self.player_fly,
            hurt_time: &mut self.hurt_time,
            procs: vec![],
        };
        let done = scene.examine_object(i, &mut w);
        let procs = std::mem::take(&mut w.procs);
        self.run_object_procs(procs);
        done
    }

    /// 主角「旋转飞」的一帧（特殊动作 0xb）：飞完站住。
    fn step_player_fly(&mut self, dt: f32) {
        let Some(mut fly) = self.player_fly.take() else {
            return;
        };
        fly.t = (fly.t + dt).min(FLY_SECS);
        let k = (1. - (fly.t / FLY_SECS * std::f32::consts::PI).cos()) * 0.5;
        let p = Vec3::new(
            fly.to.x * k + (1. - k) * fly.from.x,
            fly.to.y * k + (1. - k) * fly.from.y,
            fly.to.z * k + (1. - k) * fly.from.z,
        );
        if let Some(e) = self
            .scene_manager
            .get_resolved_role(self.sce_vm.state(), -1)
        {
            e.transform().borrow_mut().set_position(&p);
        }
        self.sce_vm
            .global_state_mut()
            .persistent_state_mut()
            .set_position(p);
        if fly.t < FLY_SECS {
            self.player_fly = Some(fly);
        }
    }

    /// 冲撞（见 [`crate::openpal3::scene::ram`]）：摆动、撞主角、撞飞后落地。
    fn step_rams(&mut self, dt: f32) {
        use crate::openpal3::scene::ram;
        self.hurt_time += dt;
        let Some(scene_rc) = self.scene_manager.scn_scene() else {
            return;
        };
        let scene = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
        scene.step_save_points(dt);
        let player = self
            .scene_manager
            .get_resolved_role(self.sce_vm.state(), -1);
        if let Some((vel, t, target)) = self.ram_fly.as_mut() {
            *t += dt;
            if let Some(e) = &player {
                let p = e.transform().borrow().position();
                let q = Vec3::new(p.x + vel.x * dt, p.y + vel.y * dt, p.z + vel.z * dt);
                e.transform().borrow_mut().set_position(&q);
            }
            if *t >= ram::FLY_SECS {
                let (layer, x, z) = *target;
                let (c, _) = scene.tile_point(layer, x, z);
                if let Some(e) = &player {
                    e.transform().borrow_mut().set_position(&c);
                    if let Some(rc) = RoleController::get_role_controller(e.clone()) {
                        let rc = rc.inner::<RoleController>();
                        rc.set_nav_layer(layer);
                        rc.idle();
                    }
                }
                self.sce_vm
                    .global_state_mut()
                    .persistent_state_mut()
                    .set_position(c);
                log::debug!("冲撞：落到层 {layer} ({x}, {z})");
                self.ram_fly = None;
            }
        }
        let info = player.as_ref().and_then(|e| {
            let rc = RoleController::get_role_controller(e.clone())?;
            let rc = rc.inner::<RoleController>();
            Some((
                e.transform().borrow().position(),
                rc.model_radius(),
                rc.model_height(),
            ))
        });
        let ready = self.ram_fly.is_none() && self.hurt_time > ram::HURT_DELAY;
        if let Some((vel, target)) = scene.step_rams(dt, info, ready) {
            log::debug!(
                "冲撞：撞上主角，飞向层 {} ({}, {})",
                target.0,
                target.1,
                target.2
            );
            self.hurt_time = 0.;
            self.ram_fly = Some((vel, 0., target));
            if let Some(rc) = player.and_then(RoleController::get_role_controller) {
                rc.inner::<RoleController>().idle();
            }
            obj_hurt(self.sce_vm.state_mut(), ram::HURT_PERCENT);
        }
    }

    /// 场景物件的动作队列和状态（`FUN_00426eb0` 里 `FUN_0042f460` 和状态机那部分）。
    fn step_object_actions(&mut self, dt: f32) {
        self.step_player_fly(dt);
        self.step_rams(dt);
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
            fly: &mut self.player_fly,
            hurt_time: &mut self.hurt_time,
            procs: vec![],
        };
        scene.step_object_actions(dt, &mut w);
        let procs = std::mem::take(&mut w.procs);
        self.run_object_procs(procs);
    }

    /// 物件动作 4 要跑的过程（`FUN_00432170(id, 0)`）：编号 ≥ 100 在当前场景的 `.sce` 里，
    /// < 100 在 `init.sce` 里（两张表 `DAT_00d8c308` / `DAT_00d8c2e8`，`FUN_00431fb0` 装的）。
    fn run_object_procs(&mut self, procs: Vec<i32>) {
        // 物件起的脚本同样登记进 `ObjLockInput`（`sceneobj.cpp` 里的 `Add(NULL, runscript)`）。
        if !procs.is_empty() {
            self.input_lock = true;
        }
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

    /// 切换主角（见调用处）。
    fn update_role_switch(&mut self) {
        let pressed = {
            let input = self.input_engine.borrow();
            input.get_key_state(Key::Tab).pressed()
                || input.get_key_state(Key::GamePadLeftShoulder).pressed()
        };
        if !pressed {
            return;
        }
        let maze = self
            .scene_manager
            .scn_scene()
            .is_some_and(|s| s.inner::<crate::openpal3::scene::ScnScene>().scene_type() == 2);
        if !maze {
            return;
        }
        let acting = self
            .scene_manager
            .get_resolved_role(self.sce_vm.state(), -1)
            .and_then(RoleController::get_role_controller)
            .is_some_and(|rc| {
                rc.inner::<RoleController>().state()
                    == crate::openpal3::scene::RoleState::PlayingAnimation
            });
        if acting {
            return;
        }
        let cur = self.sce_vm.global_state().role_controlled();
        let p = self.sce_vm.global_state().persistent_state();
        let mut i = cur + 1;
        if i > 4 {
            i = 0;
        }
        while !p.is_in_team(i as u32) {
            i += 1;
            if i > 4 {
                i = 0;
            }
            if i == cur {
                break;
            }
        }
        drop(p);
        log::debug!("切换主角：{cur} -> {i}");
        let scene_manager = self.scene_manager.clone();
        crate::scripting::sce::commands::set_ctrl(&scene_manager, self.sce_vm.state_mut(), i);
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
            fly: &mut self.player_fly,
            hurt_time: &mut self.hurt_time,
            procs: vec![],
        };
        scene.player_landed(layer, &pos, dt, &mut w);
        let procs = std::mem::take(&mut w.procs);
        self.run_object_procs(procs);
    }

    /// `FUN_00420b40`：主角脚下那格的物件（被动作牵着走、移动方式 4 的时候不查）。
    /// 主角碰出来的过程：跑起来，并且锁住主角操作直到它（连同它叫的）跑完（`ObjLockInput::Add`）。
    fn run_triggered_proc(&mut self, proc_id: u32) {
        self.sce_vm.call_proc(proc_id);
        self.input_lock = true;
        // 原版碰出过程时主角站住（类型 0 的 `InterActRole` 里 `Stand()`）。agent 带着走的路也先停下——
        // 过程常常当帧就换了场景、跑完了，锁一放开，剩下的路点还是旧场景的坐标。去的就是这个触发点的话
        // 这趟算走完了，不然过程跑完再接着走（见 `agent_walk_goal`）。
        self.agent_walk = None;
        if self
            .agent_walk_goal
            .as_ref()
            .is_some_and(|g| g.goal_proc == Some(proc_id))
        {
            self.agent_walk_goal = None;
        }
    }

    /// [调试] 从主角现在的位置找去 agent 目标的路（见 [`ScnScene::agent_path`]）。
    fn plan_agent_walk(
        &self,
        nav_index: Option<usize>,
        point: &Vec3,
    ) -> Option<Vec<(usize, Vec3)>> {
        let scene = self.scene_manager.scn_scene()?;
        let role = self
            .scene_manager
            .get_resolved_role(self.sce_vm.state(), -1)?;
        let layer = RoleController::get_role_controller(role.clone())
            .map_or(0, |rc| rc.inner::<RoleController>().nav_layer());
        let from = role.transform().borrow().position();
        scene
            .inner::<crate::openpal3::scene::ScnScene>()
            .agent_path(layer, &from, nav_index, point)
    }

    /// [调试] 现在是哪个场景（存档里的场景名 + 块名）。
    fn scene_key(&self) -> (String, String) {
        let p = self.sce_vm.global_state().persistent_state();
        (
            p.scene_name().unwrap_or_default(),
            p.sub_scene_name().unwrap_or_default(),
        )
    }

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
            fly: &mut self.player_fly,
            hurt_time: &mut self.hurt_time,
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

        // 下面是冒险模式的世界模拟（主角移动、NPC 巡逻、野怪、触发器），要求栈顶
        // 就是冒险场景。小游戏和战斗会往场景栈上压自己的 `CoreScene`（宿营 106、
        // 战斗舞台……），这时 `scn_scene()` 是 `None`——原版那是 `SwitchMode` 换了
        // 一个模式，冒险世界根本不更新。战斗和多数剧情会先关冒险输入，被上面的
        // `!adv_enabled` 挡掉；宿营不关，先前走到 `scn_scene().unwrap()` 就崩了
        // （回放里经调试入口开宿营时实测）。
        if self.scene_manager.scn_scene().is_none() {
            return None;
        }

        self.step_cave_wolf(delta_sec);
        self.test_save();

        // `ObjLockInput::Update`：登记的过程都跑完了就解锁。
        if self.input_lock
            && !self.sce_vm.state().context().is_running()
            && !self.sce_vm.has_active_commands()
        {
            self.input_lock = false;
        }
        // 物件那半个 `ObjLockInput`（开关、特技机关拨到一半）同样锁。
        let locked = self.input_lock
            || self.player_fly.is_some()
            || self.ram_fly.is_some()
            || self.scene_manager.scn_scene().is_some_and(|s| {
                s.inner::<crate::openpal3::scene::ScnScene>()
                    .object_input_locked()
            });

        // 冻结计数小于 1（场景物件的动作 8）：主角不听输入（`FUN_0041d9a0`）、野怪停住、不撞怪、不查脚下物件。
        let frozen = self.sce_vm.global_state().frozen();
        // 跳的时候主角是行为 4：不听输入、不查脚下物件和触发区（`FUN_0041d9a0` / `FUN_00422e40`）。
        if self.player_jump.is_some() {
            self.step_player_jump(delta_sec);
        } else if !frozen && !locked {
            self.update_jump_charge(delta_sec);
        }
        // 切换主角（`RoleCtrl::Update`，exe 0x4…：`FUN_0040e5f0` 前那段）：迷宫里（场景类型 2）、按下切换键、
        // 没有登记着的锁、不在组队动画里、主角没在做动作时，从下一个编号起找第一个在队里的主角接手。
        // 键盘是原版的 Tab；[移植] 手柄原版没有，用左肩键（十字键在冒险里管走路）。
        if !frozen && !locked && self.player_jump.is_none() {
            self.update_role_switch();
        }
        // 蓄力时主角站着不动（原版瞄落点的时候也不走），但头上的图标照出。
        let frozen = frozen || self.player_jump.is_some() || self.jump_charge.is_some();
        // 头上「跳」的图标跟着 `FUN_0041d9a0` 每帧更新；冻着、跳着的时候那个函数不跑，图标保持原样。
        if !frozen {
            let icon = self.jump_icon_ndc();
            self.sce_vm.state_mut().set_jump_icon(icon);
        }
        let (moving_direction, throw) = if frozen || locked {
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
        if !frozen && !locked {
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

        // [调试] agent 带着主角一格一格走（`agent_walk_to`）。
        if self.agent_walk.is_some() && (locked || frozen) {
            self.agent_walk = None;
            // 只是场景物件在动（食人花吞吐、开关）的那种打断，接着走不算一次重试——古藤林一条路
            // 要过好几朵花。
            // 被冲撞撞飞不算：人已经被挪走了，再走多半又被撞（蜀山故道的「打回起点」）。
            // 把人载走的物件（神树传送点、电梯、上下传送板、移动浮板——浮板会挪，比它路线上的点）本身就是目标的：载到了就是走完了，再接着走
            // 只会又踩上去。路过的（探路把它当成换层的路）照常接着走。
            let goal = self.agent_walk_goal.as_ref().map(|g| g.point);
            let rode = self.scene_manager.scn_scene().is_some_and(|s| {
                let s = s.inner::<crate::openpal3::scene::ScnScene>();
                s.object_lock_report().iter().any(|o| {
                    (o.3 || o.4 > 0)
                        && matches!(o.1, 0x5 | 0x26 | 0x27 | 0x28)
                        && goal.is_some_and(|g| {
                            s.object_ride_points(o.0)
                                .iter()
                                .any(|c| (g.x - c.x).hypot(g.z - c.z) < 60.)
                        })
                })
            });
            if rode {
                self.agent_walk_goal = None;
            } else if !self.input_lock && !frozen && self.ram_fly.is_none() {
                if let Some(g) = self.agent_walk_goal.as_mut() {
                    g.free_resume = true;
                }
            }
        }
        // 换了场景，旧场景的路点和目标都作废（过场里冒险循环不跑，走到这儿时可能已经换过场景了）。
        if self
            .agent_walk_goal
            .as_ref()
            .is_some_and(|g| g.scene != self.scene_key())
        {
            self.agent_walk = None;
            self.agent_walk_goal = None;
        }
        // 半路停下的，过程跑完了：还在同一个场景就重新找路接着走，换了场景或者走不到了就作罢。
        if self.agent_walk.is_none() && !locked && !frozen {
            if let Some(mut goal) = self.agent_walk_goal.take() {
                if goal.scene == self.scene_key() && goal.resumes < 3 {
                    if let Some(path) = self.plan_agent_walk(goal.nav_index, &goal.point) {
                        self.agent_walk = Some((path.into_iter().collect(), goal.interact));
                        if !std::mem::take(&mut goal.free_resume) {
                            goal.resumes += 1;
                        }
                        self.agent_walk_goal = Some(goal);
                    }
                }
            }
        }
        if let Some((path, interact)) = self.agent_walk.as_mut() {
            match path.pop_front() {
                Some((layer, p)) => {
                    if let Some(role) = self
                        .scene_manager
                        .get_resolved_role(self.sce_vm.state(), -1)
                    {
                        role.transform().borrow_mut().set_position(&p);
                        if let Some(rc) = RoleController::get_role_controller(role) {
                            rc.inner::<RoleController>().set_nav_layer(layer);
                        }
                    }
                }
                None => {
                    self.agent_interact = *interact;
                    // [调试] 要按调查键的，先转向目标（调查键挑人、挑物件都看朝向）。目标是场景物件时，
                    // 原版选物件最后比的是最后一个进了视野的物件（见 `select_examine_object`），正对着
                    // 目标、它后面又有序号更大的物件时会落空——这时转一圈找个能选中它的朝向。
                    if *interact {
                        if let (Some(goal), Some(role)) = (
                            self.agent_walk_goal.as_ref(),
                            self.scene_manager
                                .get_resolved_role(self.sce_vm.state(), -1),
                        ) {
                            let layer = RoleController::get_role_controller(role.clone())
                                .map_or(0, |rc| rc.inner::<RoleController>().nav_layer());
                            let scene = self.scene_manager.scn_scene();
                            // 路点是格子中心；玩家能一直走到格子边上。朝目标再挪近（每步 1，落点
                            // 得能走、离目标不到 12 就停），躺在床上的人这样才进得了 24 以内。
                            let mut pos = role.transform().borrow().position();
                            if let Some(s) = scene.as_ref() {
                                let s = s.inner::<crate::openpal3::scene::ScnScene>();
                                for _ in 0..24 {
                                    let (dx, dz) = (goal.point.x - pos.x, goal.point.z - pos.z);
                                    let d = (dx * dx + dz * dz).sqrt();
                                    if d < 12. {
                                        break;
                                    }
                                    let next = Vec3::new(pos.x + dx / d, pos.y, pos.z + dz / d);
                                    if !s.walkable_at(layer, &next) {
                                        break;
                                    }
                                    pos = next;
                                }
                                role.transform().borrow_mut().set_position(&pos);
                            }
                            let (dx, dz) = (goal.point.x - pos.x, goal.point.z - pos.z);
                            let base = dz.atan2(dx);
                            let picks = |a: f32| {
                                let fwd = Vec3::new(a.cos(), 0., a.sin());
                                scene.as_ref().is_some_and(|s| {
                                    let s = s.inner::<crate::openpal3::scene::ScnScene>();
                                    s.select_examine_object(&pos, &fwd, layer)
                                        .and_then(|i| s.object_entity_at(i))
                                        .is_some_and(|e| {
                                            let o = e.transform().borrow().position();
                                            (o.x - goal.point.x).abs() < 1.
                                                && (o.z - goal.point.z).abs() < 1.
                                        })
                                })
                            };
                            let a = (0..16)
                                .map(|k| base + k as f32 * std::f32::consts::PI / 8.)
                                .find(|a| picks(*a))
                                .unwrap_or(base);
                            role.transform().borrow_mut().look_at(&Vec3::new(
                                pos.x + a.cos(),
                                pos.y,
                                pos.z + a.sin(),
                            ));
                        }
                    }
                    self.agent_walk = None;
                    self.agent_walk_goal = None;
                }
            }
        }

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
        // 换了场景：原来那一槽记下离开时的「上一次站的」，新场景要是还是它那一槽里放着的那个，接着用。
        {
            let key = {
                let s = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
                (s.name().to_lowercase(), s.sub_name().to_lowercase())
            };
            let slot = {
                let s = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
                if s.scene_type() == 1 { 1 } else { 0 }
            };
            let (slots, cur) = &mut self.scene_slots;
            if cur.as_ref().is_none_or(|(k, _)| *k != key) {
                if let Some((old, old_slot)) = cur.take() {
                    slots[old_slot] = Some((old, self.last_nav_trigger));
                }
                self.last_nav_trigger = match &slots[slot] {
                    Some((k, last)) if *k == key => *last,
                    _ => None,
                };
                *cur = Some((key, slot));
            }
        }
        // 锁着的时候原版整段 `Scene::InterActRole` 不跑，「上一次站的物件」也不更新。
        if !locked {
            let proc_id_opt = {
                let s = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
                s.test_nav_trigger(nav_layer, &position)
            };
            let proc_id_opt = proc_id_opt.filter(|_| !frozen);
            let entered = proc_id_opt.filter(|id| self.last_nav_trigger != Some(*id));
            self.last_nav_trigger = proc_id_opt;
            if let Some(proc_id) = entered {
                debug!("New proc triggerd by nav: {}", proc_id);
                self.run_triggered_proc(proc_id);
            }
        }

        // 换层（`Role::UpdateLayer(-1)`，`gamebase/role.cpp:243`：键盘操作时每帧）：站进本层的传送口，
        // 另一层同号传送口里最近那格离得够近、又大致在前面，就换过去。见 [`ScnScene::portal_switch`]。
        // 先前这里是移植端自己的做法（踩进换层区沿走的方向找另一层能走的格子），宾化仓库后门那种
        // 只靠传送口连着的地方会走不过去。
        if !frozen && !locked && self.agent_walk.is_none() {
            if let Some(role) = self
                .scene_manager
                .get_resolved_role(self.sce_vm.state(), -1)
            {
                let forward = crate::openpal3::scene::ScnScene::facing_of(&role);
                let switched = scene_rc
                    .inner::<crate::openpal3::scene::ScnScene>()
                    .portal_switch(nav_layer, &position, Some(&forward));
                if let Some((layer, p)) = switched {
                    if let Some(rc) = RoleController::get_role_controller(role.clone()) {
                        rc.inner::<RoleController>().set_nav_layer(layer);
                    }
                    role.transform().borrow_mut().set_position(&p);
                }
            }
        }

        let agent_interact = std::mem::take(&mut self.agent_interact);
        let input_engine = self.input_engine.clone();
        let input = input_engine.borrow_mut();
        // Space matches the original PC binding (空格 = 对话/调查/确定);
        // F is the upstream OpenPAL3 binding, GamePadEast the pad one.
        if !frozen
            && !locked
            && (agent_interact
                || input.get_key_state(Key::F).pressed()
                || input.get_key_state(Key::Space).pressed()
                || input.get_key_state(Key::GamePadEast).pressed())
        {
            // 空格先找人（`RoleCtrl` 键盘那一支：`SceneMgr::GetSelRole_RoleCtrl()`，选不到人才
            // `GetObjSelect_RoleCtrl()` 找物件，`gamebase/rolectrl.cpp:405`）。以前先查虚拟盒子 / 道具，
            // 人站在床边时（蜀山高级弟子房间的龙葵）空格总是触发床。
            let trigger_proc_id = {
                let scene = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
                let forward = self
                    .scene_manager
                    .get_resolved_role(self.sce_vm.state(), -1)
                    .map(|r| crate::openpal3::scene::ScnScene::facing_of(&r))
                    .unwrap_or(Vec3::new(0., 0., 1.));
                scene
                    .test_role_trigger(
                        &position,
                        self.sce_vm.global_state().role_controlled(),
                        &forward,
                        nav_layer,
                    )
                    .or_else(|| scene.test_aabb_trigger(&position))
                    .or_else(|| scene.test_item_trigger(&position))
            };
            if let Some(proc_id) = trigger_proc_id {
                debug!("New proc triggerd: {}", proc_id);
                self.run_triggered_proc(proc_id);
            } else if self.examine_object(&position, nav_layer) {
                return None;
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
                    self.run_triggered_proc(proc_id);
                }
                None => {}
            }
        }

        None
    }
}

/// [调试] agent 带着主角走的那一趟的目标，见 `AdventureDirectorProps::agent_walk_goal`。
struct AgentWalkGoal {
    nav_index: Option<usize>,
    point: Vec3,
    interact: bool,
    /// 去的走格触发点挂的过程：碰出的就是它，这趟算走完。
    goal_proc: Option<u32>,
    scene: (String, String),
    resumes: u8,
    /// 上一次停下是场景物件在动，接着走不算重试。
    free_resume: bool,
}

/// 场景物件的动作 2 牵着主角走（`FUN_00412b60` 设的移动方式 4：起点 +0x554、终点 +0x540、按走的速度）。
#[derive(Debug, Clone, Copy)]
struct ObjectWalk {
    start: Vec3,
    target: Vec3,
    speed: f32,
}

/// `Role::RotateFly`（`FUN_00414e50` 起、`FUN_00415xxx` 特殊动作 0xb 每帧推）：0.5 秒里按
/// `(1 − cos πt) / 2` 从起点挪到终点；`flag` 1 飞进去时缩放 1 → 0.7，2 飞出来时 0.7 → 1。
/// [未做] 同时绕「(起点 − 终点) × Y」那根轴转 0 → 90°（飞出来 90 → 0）、蹲下的动作（0x12）和缩放，
/// 移植端只挪位置。
#[derive(Debug, Clone, Copy)]
struct PlayerFly {
    from: Vec3,
    to: Vec3,
    t: f32,
}

/// 飞一趟的秒数（`+0x4e4 = 0.5`）。
const FLY_SECS: f32 = 0.5;

/// 场景物件的动作要碰的主角、模型、音效、冻结计数。
/// `PlayerMgr::Obj_Hurt(frac)`（`FUN_004101f0(frac, 1, 0)`）：队里每人扣最大精的 `frac`，最少剩 1。
fn obj_hurt(state: &mut crate::scripting::sce::SceState, frac: f32) {
    let mut p = state.global_state_mut().persistent_state_mut();
    let team: Vec<u32> = p.team().to_vec();
    for id in team {
        if let Some(r) = p.role_mut(id) {
            let hp = (r.hp as f32 - r.max_hp as f32 * frac) as i32;
            r.hp = hp.max(1);
        }
    }
}

struct ObjWorld<'a> {
    scene: &'a crate::openpal3::scene::ScnScene,
    player: Option<ComRc<radiance::comdef::IEntity>>,
    state: &'a mut crate::scripting::sce::SceState,
    walk: &'a mut Option<ObjectWalk>,
    fly: &'a mut Option<PlayerFly>,
    /// 离上次被机关打到过了多久（`_DAT_0058de64`，冲撞和剑冢的剑、锤共用）。
    hurt_time: &'a mut f32,
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

    fn nav_layer_of_player(&self) -> usize {
        self.player
            .as_ref()
            .and_then(|p| RoleController::get_role_controller(p.clone()))
            .map_or(0, |rc| rc.inner::<RoleController>().nav_layer())
    }
    fn player_rc(&self) -> Option<ComRc<crate::openpal3::comdef::IRoleController>> {
        self.player
            .as_ref()
            .and_then(|p| RoleController::get_role_controller(p.clone()))
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
        q.y = self.scene.floor_height(self.nav_layer_of_player(), &q);
        self.set_player(&q);
    }

    fn fix_player_height(&mut self) {
        let mut p = self.player_pos();
        p.y = self.scene.floor_height(self.nav_layer_of_player(), &p);
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
        let layer = self.nav_layer_of_player();
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

    fn controlled_role(&self) -> i32 {
        self.state.global_state().role_controlled()
    }

    fn player_layer(&self) -> usize {
        self.nav_layer_of_player()
    }

    fn can_role_goto(&self, i: usize, layer: usize, from: &Vec3, to: &Vec3) -> bool {
        self.scene.object_can_direct_go(i, layer, from, to)
    }

    fn object_extent(&self, i: usize) -> Option<(f32, f32)> {
        self.scene.object_extent(i)
    }

    fn object_animated(&self, i: usize) -> bool {
        !self.cvds(i).is_empty()
    }

    fn stand(&mut self) {
        self.idle();
    }

    /// `FUN_00414e30(act, 1)`：动作名照 `PTR_DAT_0056fdac` 那张表。
    fn show_act(&mut self, act: usize) {
        let Some(name) = crate::openpal3::combat::motion::ACTION_NAMES.get(act) else {
            return;
        };
        if let Some(rc) = self.player_rc() {
            let ok = rc.inner::<RoleController>().play_anim(
                name,
                crate::openpal3::scene::RoleAnimationRepeatMode::Repeat(1),
            );
            log::debug!(
                "场景物件：主角播动作 {name}（{}）",
                if ok { "开始" } else { "没有这个动作" }
            );
        }
    }

    fn player_acting(&self) -> bool {
        self.fly.is_some()
            || self.player_rc().is_some_and(|rc| {
                rc.inner::<RoleController>().state()
                    == crate::openpal3::scene::RoleState::PlayingAnimation
            })
    }

    fn toggle_switch(&mut self, i: usize) -> bool {
        let next = !self.switch_on(i);
        log::debug!("场景物件：#{i} 开关 -> {}", next as i32);
        self.save_override(i, |s, city, scene, id| {
            s.set_scene_object_state(city, scene, id, next as i32)
        });
        next
    }

    /// 存档里记了就用记的，没记就是 `.scn` 里的初值。
    fn switch_on(&self, i: usize) -> bool {
        let Some(id) = self.scene.object_id_at(i) else {
            return false;
        };
        self.state
            .global_state()
            .persistent_state()
            .scene_object_state_saved(self.scene.name(), self.scene.sub_name(), id)
            .map_or(self.scene.object_switch_init(i), |v| v != 0)
    }

    fn deactivate(&mut self, i: usize) {
        log::debug!("场景物件：#{i} 不在场了");
        let Some(id) = self.scene.object_id_at(i) else {
            return;
        };
        self.scene.set_object_active(id, false);
        if let Some(e) = self.scene.object_entity_at(i) {
            e.set_visible(false);
        }
        self.save_override(i, |s, city, scene, id| {
            s.set_scene_object_active(city, scene, id, false)
        });
    }

    fn fly_player(&mut self, to: &Vec3, flag: u8) {
        log::debug!(
            "场景物件：主角飞向 ({:.0}, {:.0}, {:.0})（{flag}）",
            to.x,
            to.y,
            to.z
        );
        self.idle();
        let from = self.player_pos();
        *self.fly = Some(PlayerFly {
            from,
            to: *to,
            t: 0.,
        });
    }

    fn set_player_shown(&mut self, shown: bool) {
        if let Some(e) = &self.player {
            e.set_visible(shown);
        }
    }

    fn place_player(&mut self, layer: usize, x: i32, z: i32, at: &Vec3) {
        log::debug!("场景物件：主角放到层 {layer} ({x}, {z})");
        self.set_player_layer(layer);
        self.set_player(at);
    }

    fn reset_anim(&mut self, i: usize) {
        for c in self.cvds(i) {
            c.inner::<crate::openpal3::scene::CvdModelComponent>()
                .rewind();
        }
    }

    fn save_times(&mut self, i: usize, times: u8) {
        self.save_override(i, |s, city, scene, id| {
            s.set_scene_object_times(city, scene, id, times)
        });
    }

    fn object_rect(&self, i: usize) -> [i32; 4] {
        self.scene.object_rect(i)
    }

    fn set_switch(&mut self, i: usize, on: bool) {
        log::debug!("场景物件：#{i} 开关标志 -> {}", on as i32);
        self.save_override(i, |s, city, scene, id| {
            s.set_scene_object_state(city, scene, id, on as i32)
        });
    }

    fn depend_switch_on(&self, scene: &str, id: i32) -> bool {
        self.state
            .global_state()
            .persistent_state()
            .scene_object_state(self.scene.name(), scene, id)
            != 0
    }

    fn player_on(&self, i: usize) -> bool {
        // 脚下那格在主角这一层登记的是它，就是同一层（不能借物件动作查层：这时正借着）。
        let layer = self.nav_layer_of_player();
        self.scene.object_under(layer, &self.player_pos()) == Some(i)
    }

    fn save_pos(&mut self, i: usize) {
        let p = self.object_pos(i);
        self.save_override(i, |s, city, scene, id| {
            s.set_scene_object_position(city, scene, id, p)
        });
    }

    fn object_yaw(&self, i: usize) -> f32 {
        self.scene.object_yaw(i)
    }

    fn nav_floor(&self, layer: usize, p: &Vec3) -> f32 {
        self.scene.nav_floor(layer, p)
    }

    fn put_player(&mut self, p: &Vec3) {
        self.set_player(p);
    }

    fn walkable_at(&self, layer: usize, p: &Vec3) -> bool {
        self.scene.walkable_at(layer, p)
    }

    fn box_can_go(&self, i: usize, at: &Vec3) -> bool {
        self.scene.object_box_can_go(i, at)
    }

    fn object_owned_rect(&self, i: usize, layer: usize) -> Option<[i32; 4]> {
        self.scene.object_owned_rect(i, layer)
    }

    fn player_tile(&self, layer: usize) -> (i32, i32) {
        self.scene.tile_of(layer, &self.player_pos())
    }

    fn npc_walk_speed(&self) -> f32 {
        self.state.global_state().role_constants().walk_speed
    }

    fn set_act(&mut self, act: usize) {
        let Some(name) = crate::openpal3::combat::motion::ACTION_NAMES.get(act) else {
            return;
        };
        if let Some(rc) = self.player_rc() {
            rc.inner::<RoleController>()
                .play_anim(name, crate::openpal3::scene::RoleAnimationRepeatMode::Loop);
        }
    }

    fn clear_trigger_rect(&mut self, i: usize) {
        self.scene.clear_object_trigger_rect(i);
    }

    fn set_object_yaw(&mut self, i: usize, deg: f32) {
        self.scene.set_object_yaw(i, deg);
    }

    fn save_yaw(&mut self, i: usize) {
        let deg = self.scene.object_yaw(i);
        self.save_override(i, |s, city, scene, id| {
            s.set_scene_object_rotation(city, scene, id, deg)
        });
    }

    fn object_tile_walkable(&self, i: usize, p: &Vec3) -> bool {
        self.scene
            .object_tile_walkable(i, self.nav_layer_of_player(), p)
    }

    fn flower_master_model(&mut self, i: usize) {
        let open = self.state.global_state().divine_tree_flower();
        self.scene.flower_master_model(i, open);
    }

    fn switch_flowers(&mut self, open: bool) {
        log::debug!("场景物件：神树花 {}", if open { "开" } else { "关" });
        self.state.global_state_mut().set_divine_tree_flower(open);
        self.scene.switch_flowers(open);
    }

    fn floor_at(&self, layer: usize, p: &Vec3) -> f32 {
        self.scene.floor_height(layer, p)
    }

    fn set_render_pos(&mut self, i: usize, p: &Vec3) {
        if let Some(e) = self.scene.object_entity_at(i) {
            e.transform().borrow_mut().set_position(p);
        }
    }

    fn set_object_roll(&mut self, i: usize, deg: f32) {
        self.scene.set_object_roll(i, deg);
    }

    fn hurt_ready(&self) -> bool {
        *self.hurt_time > crate::openpal3::scene::ram::HURT_DELAY
    }

    fn player_hit_by(&self, i: usize) -> bool {
        let Some(e) = &self.player else {
            return false;
        };
        let Some(rc) = RoleController::get_role_controller(e.clone()) else {
            return false;
        };
        let rc = rc.inner::<RoleController>();
        let pl = e.transform().borrow().position();
        let at = self.object_pos(i);
        self.scene
            .object_hits_player(i, &at, &pl, rc.model_radius(), rc.model_height())
    }

    fn obj_hurt(&mut self, frac: f32) {
        log::debug!("场景物件：主角被机关打到，全队扣 {frac}");
        obj_hurt(self.state, frac);
        *self.hurt_time = 0.;
    }

    fn give_item(&mut self, id: i32) {
        let name = self.state.global_state().game_db().and_then(|db| {
            db.item(id as u32)
                .map(|it| it.name.as_str().unwrap_or_default())
        });
        let Some(name) = name else {
            log::warn!("场景物件：数据库里没有物品 {id}");
            return;
        };
        log::debug!("场景物件：得到 {id} {name}");
        self.state
            .global_state_mut()
            .persistent_state_mut()
            .add_item(id as u32, 1);
        self.push_msg("getitem", &name);
        crate::scripting::sce::commands::play_sfx(self.state, "wa006");
    }

    fn give_money(&mut self, amount: i32) {
        let before = self.state.global_state().persistent_state().money();
        self.state
            .global_state_mut()
            .persistent_state_mut()
            .add_money(amount);
        let now = self.state.global_state().persistent_state().money();
        let delta = now - before;
        if now == 0 {
            self.push_msg("lossallmoney", "");
        } else if delta < 0 {
            self.push_msg("lossmoney", &(-delta).to_string());
        } else {
            self.push_msg("getmoney", &delta.to_string());
            crate::scripting::sce::commands::play_sfx(self.state, "wa006");
        }
    }

    fn give_exp(&mut self, amount: i32) {
        let ctrl = self.state.global_state().role_controlled().max(0) as u32;
        if let Some(r) = self
            .state
            .global_state_mut()
            .persistent_state_mut()
            .role_mut(ctrl)
        {
            r.exp += amount;
        }
        self.push_msg("getexp", &amount.to_string());
    }

    fn favor_add(&mut self, role: i32, delta: i32) {
        self.state
            .global_state_mut()
            .persistent_state_mut()
            .add_favor(role.max(0) as u32, delta);
        if delta < 0 {
            crate::scripting::sce::commands::play_sfx(self.state, "wa048");
        }
    }

    fn rand(&mut self) -> i32 {
        self.state.global_state_mut().crt_rand()
    }

    fn open_save_menu(&mut self) {
        log::debug!("场景物件：存盘点，打开存档页");
        self.state.request_save_menu();
    }

    fn swap_model(&mut self, i: usize, name: &str) {
        log::debug!("场景物件：#{i} 换模型 {name}");
        self.scene.replace_object_model(i, name);
    }

    fn hold_anim_end(&mut self, i: usize) {
        if let Some(id) = self.scene.object_id_at(i) {
            self.scene.hold_object_anim_end(id);
        }
    }

    fn mesh_fx_state(&self, i: usize) -> Option<i32> {
        self.scene.mesh_fx_state(i)
    }

    fn mesh_fx_clear_off(&mut self, i: usize) {
        log::debug!("场景物件：#{i} 瘴气散开");
        self.scene.mesh_fx_clear_off(i);
    }

    fn object_name(&self, i: usize) -> String {
        self.scene.object_name_at(i).unwrap_or_default()
    }
}

impl ObjWorld<'_> {
    /// `FUN_00410360`（消息栏）：`lang.txt` 里 `key` 那条，`%s` / `%d` 换成 `arg`。
    /// [移植] 原版是一栏叠着放的消息（`DAT_00bfdb80` 那张表，屏幕上方居中，一条一行），这里先排进提示框。
    fn push_msg(&mut self, key: &str, arg: &str) {
        let text = common::store_ext::StoreExt2::read_to_end_from_gbk(
            self.state.asset_mgr().vfs(),
            crate::openpal3::combat::result::LANG_PATH,
        )
        .ok()
        .and_then(|t| crate::openpal3::combat::result::lang_value(&t, key));
        let Some(t) = text else {
            log::warn!("场景物件：lang.txt 里没有 {key}");
            return;
        };
        let t = match (t.find("%s"), t.find("%d")) {
            (Some(k), _) | (None, Some(k)) => format!("{}{}{}", &t[..k], arg, &t[k + 2..]),
            _ => t,
        };
        self.state.push_note(t);
    }
}

/// 读 `.par` 里形状 2 的发射器指的那份 `.csf` 点云。名字是相对
/// `effect\gbParticle` 目录的反斜杠写法。
pub(crate) fn load_par_shape(
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
