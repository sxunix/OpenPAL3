use crate::{
    openpal3::{directors::SceneManagerExtensions, scene::RoleController},
    scripting::sce::{SceCommand, SceState, commands::story_flags},
};
use crosscom::ComRc;
use imgui::Ui;
use radiance::comdef::ISceneManager;
use std::rc::Rc;

#[derive(Debug, Clone)]
pub struct SceCommandLoadScene {
    name: String,
    sub_name: String,
}

impl SceCommand for SceCommandLoadScene {
    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        _ui: &Ui,
        state: &mut SceState,
        _delta_sec: f32,
    ) -> bool {
        // Must stay single-frame: with ScriptRunMode 2 the VM keeps
        // pulling the proc's following commands (RoleSetPos,
        // CameraDefault, ...) while a multi-frame command is pending, so
        // a phased load would let them run against the OLD scene and
        // leave the camera stale over the new one.
        self.load(scene_manager, state);
        true
    }
}

impl SceCommandLoadScene {
    pub fn new(name: String, sub_name: String) -> Self {
        Self { name, sub_name }
    }

    /// 把存档里记着的物件状态摆回去（原版存档里每个物件一条记录，建场景时就按它建，
    /// `FUN_00538fa0` 存、`FUN_0042c3a0` 决定哪些要存；脚本 158 是逐物件的例外）。
    ///
    /// 移植端物件是照 `.scn` 建完再改：先摆位置（脚本 90 挪过的、乌龟沉下去的、传送板升上去的），
    /// 再是在不在场（脚本 85）、在第几层（传送板换层），最后是物件自己的状态——放下的吊桥要变成
    /// 可通过、动画停在最后一帧，沉下去的乌龟动画同样停在最后一帧。
    fn restore_scene_objects(&self, scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        use radiance::comdef::IEntityExt;
        let Some(scene_rc) = scene_manager.scn_scene() else {
            return;
        };
        let scene = scene_rc.inner::<crate::openpal3::scene::ScnScene>();
        type Saved = (
            i32,
            Option<radiance::math::Vec3>,
            Option<bool>,
            Option<u8>,
            i32,
            Option<u8>,
        );
        let saved: Vec<Saved> = {
            let p = state.global_state().persistent_state();
            scene
                .object_ids()
                .into_iter()
                .filter(|id| p.object_loads_from_save(*id))
                .map(|id| {
                    (
                        id,
                        p.scene_object_position(&self.name, &self.sub_name, id),
                        p.scene_object_active(&self.name, &self.sub_name, id),
                        p.scene_object_layer(&self.name, &self.sub_name, id),
                        p.scene_object_state(&self.name, &self.sub_name, id),
                        p.scene_object_times(&self.name, &self.sub_name, id),
                    )
                })
                .collect()
        };

        // 158 名单里的物件照 `.scn` 建：旧的存档记录作废（开关标志、次数之类不能再从存档读回来）。
        {
            let skipped: Vec<i32> = {
                let p = state.global_state().persistent_state();
                scene
                    .object_ids()
                    .into_iter()
                    .filter(|id| !p.object_loads_from_save(*id))
                    .collect()
            };
            let mut p = state.global_state_mut().persistent_state_mut();
            for id in skipped {
                p.forget_scene_object(&self.name, &self.sub_name, id);
            }
        }
        // 158 记的名单只管这一次进场景，用完就清（`FUN_00538d60` 末尾把 vector 的 end 置成 begin）。
        state
            .global_state_mut()
            .persistent_state_mut()
            .clear_objects_not_from_save();

        for (id, position, active, layer, object_state, times) in saved {
            if let Some(t) = times {
                scene.set_object_times(id, t);
            }
            if let (Some(p), Some(entity)) = (position, scene.get_root_object(id)) {
                entity.transform().borrow_mut().set_position(&p);
            }
            if let Some(active) = active {
                if let Some(entity) = scene.get_root_object(id) {
                    entity.set_visible(active);
                }
                scene.set_object_active(id, active);
            }
            if let Some(layer) = layer {
                scene.set_object_layer(id, layer as usize);
            }
            if position.is_some() || active.is_some() || layer.is_some() || object_state != 0 {
                log::debug!(
                    "存档物件：{}/{} OBJECT_{id} 位置 {:?} 在场 {:?} 层 {:?} 状态 {object_state}",
                    self.name,
                    self.sub_name,
                    position.map(|p| (p.x as i32, p.y as i32, p.z as i32)),
                    active,
                    layer
                );
            }
            let kind = scene.object_node_type(id).unwrap_or(0) & 0xff;
            if kind == 0x17 {
                // 开关存的是开关标志 +0xaf（126 `GetSwitch` 读的那个），不是状态 +0x50。
                // `SceneObj::OnLoad` case 23（`gamebase/sceneobj.cpp:4188`）：用完了（次数 0）、
                // 或者参数 0 非 0 且开着的，模型停在最后一帧；参数 0 为 1 的「可通过」跟着开关走。
                let p0 = scene.object_parameter(id).unwrap_or(0);
                // 存档里没记的取 `.scn` 里的初值。
                let on = match state
                    .global_state()
                    .persistent_state()
                    .scene_object_state_saved(&self.name, &self.sub_name, id)
                {
                    Some(v) => v != 0,
                    None => scene
                        .object_index_of(id)
                        .is_some_and(|i| scene.object_switch_init(i)),
                };
                if times == Some(0) || (p0 != 0 && on) {
                    scene.hold_object_anim_end(id);
                }
                if p0 == 1 {
                    if let Some(i) = scene.object_index_of(id) {
                        scene.set_object_passable(i, on);
                    }
                }
                continue;
            }
            if kind == 0x1a || kind == 0x29 {
                // 迷宫大宝箱开过了（状态 1，高 4 位是失败次数）/ 小宝箱开过了（状态 2）：模型停在最后一帧、
                // 不再挡路；大宝箱失败满 3 次的是碎箱子模型（`SceneObj::OnLoad` case 26 / 41，
                // `gamebase/sceneobj.cpp:4160`）。
                let opened = if kind == 0x1a {
                    object_state & 0xf == 1
                } else {
                    object_state == 2
                };
                if let (true, Some(i)) = (opened, scene.object_index_of(id)) {
                    let broken = (object_state >> 4) as u8;
                    if kind == 0x1a {
                        scene.set_object_state(id, 1);
                        scene.set_box_state(i, broken);
                        if broken >= 3 {
                            let name = scene.object_name_at(i).unwrap_or_default();
                            let digit = name
                                .len()
                                .checked_sub(5)
                                .and_then(|k| name.as_bytes().get(k))
                                .map_or(0, |b| b.wrapping_sub(b'0'));
                            scene.replace_object_model(i, &format!("wz20_{digit}.cvd"));
                        }
                    } else {
                        scene.set_object_state(id, 2);
                    }
                    scene.hold_object_anim_end(id);
                    let l = scene.object_layer_at(i).unwrap_or(0);
                    scene.set_object_tiles(i, true, l);
                }
                continue;
            }
            if kind == 0x22 {
                // 剑桥：伸出去时原位一起改了，存档里是伸出去的位置（上面已经摆好）；格子按它重新登记
                // （`SceneObj::OnLoad` case 34，`gamebase/sceneobj.cpp:4237`）。
                if let (true, Some(i)) = (position.is_some(), scene.object_index_of(id)) {
                    let l = scene.object_layer_at(i).unwrap_or(0);
                    scene.set_object_tiles(i, false, l);
                }
                continue;
            }
            if kind == 0xb {
                // 推箱子：位置是推完记的原位（上面已经摆好），格子按它重新登记；三态箱子的状态存在状态那一格。
                if let Some(i) = scene.object_index_of(id) {
                    scene.set_box_state(i, object_state as u8);
                    if position.is_some() {
                        let l = scene.object_layer_at(i).unwrap_or(0);
                        scene.set_object_tiles(i, false, l);
                    }
                }
                continue;
            }
            if kind == 0x1e {
                // 升降机关门（`SceneObj::OnLoad` case 30，`gamebase/sceneobj.cpp:4217`）：位置回到原位，
                // 沉下去了（状态 3）的再往下一个模型高；「可通过」= 开关标志。存档里存的是开关标志，开着就是
                // 沉下去停住的那个状态。
                if let Some(i) = scene.object_index_of(id) {
                    let on = state
                        .global_state()
                        .persistent_state()
                        .scene_object_state_saved(&self.name, &self.sub_name, id)
                        .map_or(scene.object_switch_init(i), |v| v != 0);
                    if let (Some(mut p), Some(entity)) =
                        (scene.object_scn_position(i), scene.get_root_object(id))
                    {
                        if on {
                            p.y -= scene.object_model_height(i);
                        }
                        entity.transform().borrow_mut().set_position(&p);
                    }
                    scene.set_object_passable(i, on);
                    scene.set_object_state(id, if on { 3 } else { 0 });
                }
                continue;
            }
            if kind == 0x1c {
                // 剑冢的锤（`FUN_0042ad40` case 0x1c，参数 0 为 1）：「可通过」= 开关标志；关着的落在脚下地面上
                // （`FUN_00421440`），开着的由每帧上下动摆位置。
                if let Some(i) = scene.object_index_of(id) {
                    if scene.object_parameter(id) == Some(1) {
                        let on = state
                            .global_state()
                            .persistent_state()
                            .scene_object_state_saved(&self.name, &self.sub_name, id)
                            .map_or(scene.object_switch_init(i), |v| v != 0);
                        scene.set_object_passable(i, on);
                        if let (false, Some(mut p), Some(entity)) =
                            (on, scene.object_scn_position(i), scene.get_root_object(id))
                        {
                            let l = scene.object_layer_at(i).unwrap_or(0);
                            p.y = scene.floor_height(l, &p);
                            entity.transform().borrow_mut().set_position(&p);
                        }
                    }
                }
                continue;
            }
            if kind == 0x24 {
                // 坠落的障碍物：掉下来了（状态 2）就停在落下的位置（上面已按存档摆好）、不可通过、触发区清空、
                // 按位置登记格子（`SceneObj::OnLoad` case 36，`gamebase/sceneobj.cpp:4242`）。
                if object_state == 2 {
                    if let Some(i) = scene.object_index_of(id) {
                        scene.set_object_state(id, 2);
                        scene.set_object_passable(i, false);
                        scene.clear_object_trigger_rect(i);
                        let l = scene.object_layer_at(i).unwrap_or(0);
                        scene.set_object_tiles(i, false, l);
                    }
                }
                continue;
            }
            if matches!(kind, 7 | 8) {
                // 旋转石梁：原版整份 `ObjectInfo` 进存档，`SceneObj::OnLoad` case 7（`gamebase/sceneobj.cpp:4102`）
                // 按存下的角度把格子重新登记一遍。
                let deg = state
                    .global_state()
                    .persistent_state()
                    .scene_object_rotation(&self.name, &self.sub_name, id);
                if let (Some(deg), Some(i)) = (deg, scene.object_index_of(id)) {
                    scene.set_object_yaw(i, deg);
                    let l = scene.object_layer_at(i).unwrap_or(0);
                    scene.set_object_tiles(i, false, l);
                }
                continue;
            }
            if kind == 0x5 {
                // 移动浮板存的是自己的开关标志 +0xaf（开关推它时写的），不是状态；位置是到头时改的原位
                // `m_info.pos`（上面已经摆好），读档后状态又是 0，格子按新位置重新登记。
                // 存档里没记就保持 `.scn` 里的初值。
                if state
                    .global_state()
                    .persistent_state()
                    .scene_object_state_saved(&self.name, &self.sub_name, id)
                    .is_some()
                {
                    scene.set_platform_switch(id, object_state != 0);
                }
                if position.is_some() {
                    if let Some(i) = scene.object_index_of(id) {
                        let l = scene.object_layer_at(i).unwrap_or(0);
                        scene.set_object_tiles(i, false, l);
                    }
                }
                // 参数 0 为 1 的那种（海底城）存的是停在折线第几个点。
                if scene.object_parameter(id) == Some(1) {
                    if let Some(i) = scene.object_index_of(id) {
                        scene.set_box_state(i, object_state as u8);
                    }
                }
                continue;
            }
            if object_state != 0 {
                scene.set_object_state(id, object_state as u8);
                if matches!(kind, 0x13 | 0x1d) {
                    scene.hold_object_anim_end(id);
                }
                // 吊桥放下了、升降平台停在上面（`SceneObj::OnLoad` case 22：`cango = 1`）：可通过。
                if kind == 0x13 || (kind == 0x16 && object_state == 2) {
                    if let Some(i) = scene.object_index_of(id) {
                        scene.set_object_passable(i, true);
                    }
                }
            }
        }

        // 神树花：场上有总开关（类型 0x17、参数 2 为 1）就照它的开关定花开没开（`FUN_00424f20` 建物件时，
        // exe 0x425300：`DAT_00d8bc38 = 开关 == 0`），再把这个场景的花按它换模型、登记格子
        // （`FUN_0042ad40` case 0x25 → `FUN_0042c280`）。[移植] 原版花在建表时按 `.scn` 顺序和别的物件一起
        // 登记，这里是建完表、套完存档以后补登记，和别的物件压同一格时谁盖谁可能不一样。
        if let Some(id) = scene.flower_master_switch() {
            let on = state.global_state().persistent_state().scene_object_state(
                &self.name,
                &self.sub_name,
                id,
            ) != 0;
            state.global_state_mut().set_divine_tree_flower(!on);
        }
        scene.switch_flowers(state.global_state().divine_tree_flower());
        // 总开关自己的模型也照花开没开换（`FUN_0042b5b0` 读档时参数 2 为 1 那一支调 `FUN_0042c730`）。
        if let Some(i) = scene.flower_master_index() {
            scene.flower_master_model(i, state.global_state().divine_tree_flower());
        }
    }

    /// 新场景把 `ROLE_2` 一律建成蓝龙葵（`ScnScene::map_role_id` 把 2 映到
    /// 105），所以剧情里换成红龙葵之后，每次换场景都要照存档里记的形象再换
    /// 一次——原版的模型名跟着角色记录走，不随场景重建。
    fn restore_longkui_model(&self, scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        let mode = state.global_state().persistent_state().longkui_mode();
        story_flags::apply_longkui_model(&scene_manager, mode);
    }

    /// 读档：场景已经照存档建好，把存档里记的物件状态套上去（[`Self::restore_scene_objects`]）。
    /// 原版读档同样是照存档里的物件记录建物件的。
    pub fn restore_after_save_load(
        &self,
        scene_manager: ComRc<ISceneManager>,
        state: &mut SceState,
    ) {
        self.restore_scene_objects(scene_manager, state);
    }

    /// Same swap as [`Self::load`], for the agent server's debug
    /// scene jump.
    pub fn load_for_debug(&self, scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        self.load(scene_manager, state);
    }

    /// Perform the actual scene swap immediately (no transition).
    /// Also used by `SceCommandEntryRow`, which needs the load done
    /// within its single update call.
    pub(crate) fn load(&self, scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        let last_scene = scene_manager.scn_scene();
        let cpk_changed = last_scene
            .and_then(|s| Some(s.inner::<crate::openpal3::scene::ScnScene>().name() != &self.name))
            .or(Some(true))
            .unwrap();

        log::info!("sce: LoadScene {}/{}", self.name, self.sub_name);
        scene_manager.pop_scene();
        scene_manager.push_scene(state.asset_mgr().load_scn(&self.name, &self.sub_name));
        // 新场景里找不到主角（数据问题）就别崩，留一条错误日志。
        if let Some(e) = scene_manager.get_resolved_role(state, -1) {
            let r = RoleController::get_role_controller(e.clone()).unwrap();
            r.inner::<RoleController>().set_active(true);
        } else {
            log::error!(
                "LoadScene {}/{}: 新场景里找不到主角",
                self.name,
                self.sub_name
            );
        }

        state
            .global_state_mut()
            .persistent_state_mut()
            .set_scene_name(self.name.clone(), self.sub_name.clone());

        self.restore_scene_objects(scene_manager.clone(), state);
        self.restore_longkui_model(scene_manager.clone(), state);
        if cpk_changed {
            let sce = Rc::new(state.asset_mgr().load_sce(&self.name));
            state.context_mut().set_sce(sce, self.name.clone());
            state.global_state_mut().bgm_source().stop();
            state.global_state_mut().play_default_bgm();
        }

        state.try_call_proc_by_name(&format!("_{}_{}", self.name, self.sub_name));

        // The original engine fades around every scene load itself — the
        // door procs carry no Fade commands (verified in Q01). When the
        // script hasn't faded the screen out, snap to black now and let
        // the VM ease back in; a script-managed curtain (already at 1)
        // is left alone.
        if state.curtain() < 1. {
            state.set_curtain(1.);
            state.set_auto_fade_in(true);
        }
    }
}
