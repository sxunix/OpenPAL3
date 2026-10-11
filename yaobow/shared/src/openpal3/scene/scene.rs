use super::cvd_entity::{
    CvdModelComponent, CvdPlayMode, FLOAT_AMPLITUDE, IDLE_REPLAY_SPEED, IdleReplay, IdleReplayStep,
    SCENE_CVD_SPEED, SPIN_DEGREES_PER_SEC, cvd_components, play_cvd_subtree,
    scene_object_initial_play,
};
use crate::openpal3::asset_manager::AssetManager;
use crate::openpal3::combat::encounter::CrtRand;
use crate::openpal3::comdef::{ICvdModel, IRoleController};
use crate::openpal3::loaders::nav_loader::{NavFile, NavMapPoint};
use crate::openpal3::loaders::scn_loader::{ScnFile, ScnNode};
use crate::openpal3::scene::RoleController;
use crosscom::ComRc;
use radiance::comdef::{IComponentImpl, IEntity, IEntityExt, IScene, ISceneExt};
use radiance::math::Vec3;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// 野外怪物撞上主角时要带进战斗的东西（`FUN_00417420` 末尾 → `FUN_00407560`）。
#[derive(Debug, Clone, Copy)]
pub struct FieldEncounter {
    /// 撞上的是哪个 `.scn` 角色（`ROLE_<n>`），战后让它退场。
    pub role_index: u8,
    pub count: u8,
    pub ids: [u32; 3],
    /// 朝向判定：1 我方偷袭、2 敌方偷袭、0 都不是（成不成立还要进战斗再掷一次）。
    pub sneak: u8,
}

pub struct ScnScene {
    scene: ComRc<IScene>,
    asset_mgr: Rc<AssetManager>,
    cpk_name: String,
    scn_name: String,
    scn_file: ScnFile,
    nav: Nav,
    nav_triggers: RefCell<Vec<SceNavTrigger>>,
    aabb_triggers: RefCell<Vec<SceAabbTrigger>>,
    item_triggers: RefCell<Vec<SceItemTrigger>>,
    ladder_triggers: RefCell<Vec<LadderTrigger>>,
    /// Scene objects sliding to a new position — opcode 90. The original
    /// queues the move onto the object's own command ring and lets the
    /// script run on, so these are stepped per frame rather than held by
    /// the command.
    object_moves: RefCell<Vec<ObjectMove>>,
    /// 类型 17 物件的待机重播、浮动、旋转（`FUN_00426eb0` case 0x11），每帧推。
    object_motions: RefCell<Vec<ObjectMotion>>,
    /// 原版这里用的是 CRT 的 `rand()`，和别处共用一条序列；移植端单开一条，按时间播种。
    object_rng: RefCell<CrtRand>,
    /// 野外怪物的 AI（`FUN_00417420`），第一次推的时候按角色建。
    field_monsters: RefCell<Vec<super::field_monster::FieldMonster>>,
    /// 场景物件挡路的格子表（`FUN_00420940`），载入物件后建。
    object_blocking: RefCell<Option<super::object_block::ObjectBlocking>>,
    /// 场景物件的动作队列和状态（`FUN_00424c20` / `FUN_0042f460`），载入物件后建。
    object_actions: RefCell<super::object_action::ObjectActions>,
    /// 脚本 124 叫物件（事件 2）时先记下，冒险循环带着主角一起处理。
    object_events: RefCell<Vec<(i32, u8)>>,
    /// 上一帧主角脚下的物件（场景 `+0x7865c`）。
    player_object: RefCell<Option<usize>>,
}

ComObject_ScnSceneComponent!(super::ScnScene);

impl IComponentImpl for ScnScene {
    fn on_loading(&self) {
        self.load_objects();
        self.load_roles();
    }

    fn on_unloading(&self) {}

    fn on_updating(&self, _delta_sec: f32) {}
}

impl ScnScene {
    pub fn new(
        scene: ComRc<IScene>,
        asset_mgr: &Rc<AssetManager>,
        cpk_name: &str,
        scn_name: &str,
        scn_file: ScnFile,
        nav_file: NavFile,
    ) -> Self {
        Self {
            scene,
            asset_mgr: asset_mgr.clone(),
            cpk_name: cpk_name.to_string(),
            scn_name: scn_name.to_string(),
            scn_file,
            nav: Nav::new(nav_file),
            nav_triggers: RefCell::new(vec![]),
            aabb_triggers: RefCell::new(vec![]),
            item_triggers: RefCell::new(vec![]),
            ladder_triggers: RefCell::new(vec![]),
            object_moves: RefCell::new(vec![]),
            object_motions: RefCell::new(vec![]),
            field_monsters: RefCell::new(vec![]),
            object_blocking: RefCell::new(None),
            object_actions: RefCell::new(Default::default()),
            object_events: RefCell::new(vec![]),
            player_object: RefCell::new(None),
            object_rng: RefCell::new(CrtRand::new(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32))
                    .unwrap_or(0x1234_5678),
            )),
        }
    }

    pub fn name(&self) -> &str {
        &self.cpk_name
    }

    pub fn sub_name(&self) -> &str {
        &self.scn_name
    }

    pub fn nav(&self) -> &Nav {
        &self.nav
    }

    /// A scene ships either one or two nav layers, and a role carries the
    /// layer it was standing on across a scene change —
    /// `RoleController::switch_nav_layer` toggles modulo 2 whatever the
    /// scene has. Reading past the end aborted the process, so every
    /// layer lookup is clamped to what this scene actually holds.
    fn nav_layer(&self, layer: usize) -> usize {
        self.nav.layer(layer)
    }

    pub fn nav_min_coord(&self, layer: usize) -> Vec3 {
        self.nav.nav_file.maps[self.nav_layer(layer)].min_coord
    }

    pub fn nav_block_size(&self, layer: usize) -> (f32, f32) {
        self.nav.block_sizes[self.nav_layer(layer)]
    }

    pub fn get_distance_to_border_by_scene_coord(&self, layer: usize, coord: &Vec3) -> f32 {
        let nav_coord = self.scene_coord_to_nav_coord(layer, coord);
        if nav_coord.0.floor() as u32 >= self.nav.nav_file.maps[self.nav_layer(layer)].width
            || nav_coord.1.floor() as u32 >= self.nav.nav_file.maps[self.nav_layer(layer)].height
        {
            return 0.;
        }

        let nav_coord_floor = (
            (nav_coord.0.floor() as usize).clamp(
                0,
                self.nav.nav_file.maps[self.nav_layer(layer)].width as usize - 1,
            ),
            (nav_coord.1.floor() as usize).clamp(
                0,
                self.nav.nav_file.maps[self.nav_layer(layer)].height as usize - 1,
            ),
        );

        let nav_coord_ceil = (
            (nav_coord.0.ceil() as usize).clamp(
                0,
                self.nav.nav_file.maps[self.nav_layer(layer)].width as usize - 1,
            ),
            (nav_coord.1.ceil() as usize).clamp(
                0,
                self.nav.nav_file.maps[self.nav_layer(layer)].height as usize - 1,
            ),
        );
        let distance_floor = &self.nav.nav_file.maps[self.nav_layer(layer)].map[nav_coord_floor.1]
            [nav_coord_floor.0];
        let _distance_ceil =
            &self.nav.nav_file.maps[self.nav_layer(layer)].map[nav_coord_ceil.1][nav_coord_ceil.0];
        /*std::cmp::min(
            distance_floor.distance_to_border,
            distance_ceil.distance_to_border,
        ) as f32*/
        distance_floor.distance_to_border as f32
    }

    pub fn test_nav_trigger(&self, layer: usize, coord: &Vec3) -> Option<u32> {
        let nav_coord = self.scene_coord_to_nav_coord(layer, coord);
        let nav_coord = (nav_coord.0 as i32, nav_coord.1 as i32);

        for trigger in &*self.nav_triggers.borrow() {
            if Self::test_coord_in_bound(nav_coord, (trigger.nav_coord_min, trigger.nav_coord_max))
            {
                if trigger.node_type == 14
                    || (trigger.node_type == 0 && trigger.layer as usize == layer)
                {
                    return Some(trigger.sce_proc_id);
                }
            }
        }

        None
    }

    fn test_coord_in_bound(coord: (i32, i32), boundary: ((i32, i32), (i32, i32))) -> bool {
        coord.0 >= boundary.0.0
            && coord.1 >= boundary.0.1
            && coord.0 <= boundary.1.0
            && coord.1 <= boundary.1.1
    }

    pub fn test_nav_layer_trigger(&self, layer: usize, coord: &Vec3) -> bool {
        if let Some(layer_triggers) = &self.nav.nav_file.maps[self.nav_layer(layer)].layer_triggers
        {
            let nav_coord = self
                .nav
                .round_nav_coord(layer, self.scene_coord_to_nav_coord(layer, coord));
            for trigger in layer_triggers {
                if Self::test_coord_in_bound(
                    nav_coord,
                    (trigger.nav_coord_min, trigger.nav_coord_max),
                )
                /*|| Self::test_coord_in_bound(
                    nav_coord.1,
                    (trigger.nav_coord_min, trigger.nav_coord_max),
                ) */
                {
                    return true;
                }
            }
        }

        false
    }

    pub fn test_aabb_trigger(&self, coord: &Vec3) -> Option<u32> {
        const R: f32 = 50.;
        for trigger in &*self.aabb_triggers.borrow() {
            log::debug!(
                "Testing Aabb {:?} {:?} {:?}",
                &trigger.aabb_coord2,
                &trigger.aabb_coord1,
                &coord
            );
            if Self::test_sphere_aabb(coord, R, &trigger.aabb_coord1, &trigger.aabb_coord2) {
                return Some(trigger.sce_proc_id);
            }
        }

        None
    }

    pub fn test_item_trigger(&self, coord: &Vec3) -> Option<u32> {
        const D: f32 = 100.;
        for trigger in &*self.item_triggers.borrow() {
            log::debug!(
                "Testing item trigger {:?} {:?} {}",
                &trigger.coord,
                &coord,
                Vec3::sub(coord, &trigger.coord).norm2(),
            );
            if Vec3::sub(coord, &trigger.coord).norm2() < D * D {
                return Some(trigger.sce_proc_id);
            }
        }

        None
    }

    pub fn test_role_trigger(&self, coord: &Vec3, exclude_role: i32) -> Option<u32> {
        const D: f32 = 100.;
        for role in self.scene.entities() {
            if role.name() == format!("ROLE_{}", exclude_role) {
                continue;
            }

            if !role.visible() {
                continue;
            }

            let role_position = role.transform().borrow().position();
            if Vec3::sub(coord, &role_position).norm2() < D * D {
                let role_model = RoleController::get_role_controller(role);
                if role_model.is_none() {
                    continue;
                }

                return Some(role_model.unwrap().inner::<RoleController>().proc_id() as u32);
            }
        }

        None
    }

    pub fn test_ladder(&self, layer: usize, coord: &Vec3) -> Option<LadderTestResult> {
        println!("testing ladder layer {} coord {:?}", layer, coord);
        const D: f32 = 100.;
        let nav_coord = self
            .nav
            .round_nav_coord(layer, self.scene_coord_to_nav_coord(layer, coord));
        for ladder in &*self.ladder_triggers.borrow() {
            let new_layer = if ladder.switch_layer {
                (layer + 1) % 2
            } else {
                layer
            };

            let mut ladder_position = ladder.position;
            ladder_position.y = coord.y;
            if ladder.sce_proc_id != 0 {
                if Vec3::sub(coord, &ladder_position).norm2() < D * D {
                    return Some(LadderTestResult::SceProc(ladder.sce_proc_id as u32));
                }
            }

            if self
                .nav
                .check_connectivity(layer, nav_coord, ladder.nav_coord1)
            {
                return Some(LadderTestResult::NewPosition((
                    ladder.switch_layer,
                    self.nav_coord_to_scene_coord(
                        new_layer,
                        ladder.nav_coord2.0 as f32,
                        ladder.nav_coord2.1 as f32,
                    ),
                )));
            } else if self
                .nav
                .check_connectivity(layer, nav_coord, ladder.nav_coord2)
            {
                return Some(LadderTestResult::NewPosition((
                    ladder.switch_layer,
                    self.nav_coord_to_scene_coord(
                        new_layer,
                        ladder.nav_coord1.0 as f32,
                        ladder.nav_coord1.1 as f32,
                    ),
                )));
            }
        }

        None
    }

    pub fn get_height(&self, layer: usize, nav_coord: (f32, f32)) -> f32 {
        let x = (nav_coord.0.floor() as usize).clamp(
            0,
            self.nav.nav_file.maps[self.nav_layer(layer)].width as usize - 1,
        );
        let y = (nav_coord.1.floor() as usize).clamp(
            0,
            self.nav.nav_file.maps[self.nav_layer(layer)].height as usize - 1,
        );
        self.nav.nav_file.maps[self.nav_layer(layer)].map[y][x].height
    }

    pub fn scene_coord_to_nav_coord(&self, layer: usize, coord: &Vec3) -> (f32, f32) {
        let min_coord = self.nav_min_coord(layer);
        let block_size = self.nav_block_size(layer);
        (
            (coord.x - min_coord.x) / block_size.0,
            (coord.z - min_coord.z) / block_size.1,
        )
    }

    pub fn nav_coord_to_scene_coord(&self, layer: usize, nav_x: f32, nav_z: f32) -> Vec3 {
        let min_coord = self.nav_min_coord(layer);
        let block_size = self.nav_block_size(layer);
        Vec3::new(
            nav_x * block_size.0 + min_coord.x,
            self.get_height(layer, (nav_x, nav_z)),
            nav_z * block_size.1 + min_coord.z,
        )
    }

    pub fn get_object(&self, id: i32) -> Option<ComRc<IEntity>> {
        self.scene
            .entities()
            .iter()
            .find(|e| e.name() == format!("OBJECT_{}", id))
            .map(|e| e.clone())
    }

    /// Ids of every object the scene file declares.
    pub fn object_ids(&self) -> Vec<i32> {
        self.scn_file.nodes.iter().map(|n| n.object_id()).collect()
    }

    /// Where a scene object stands and which way it faces, straight out
    /// of the `.scn` node — the rotation in radians, the same value
    /// [`Self::apply_position_rotation`] turns into the entity's
    /// transform. Commands that place an actor relative to an object
    /// (74 攀爬) need the angle itself, which the transform no longer
    /// spells out.
    /// The `.scn` node type of a scene object — what the original
    /// switches on at `obj+0xf4` when the player interacts with it.
    pub fn object_node_type(&self, id: i32) -> Option<u16> {
        self.scn_file
            .nodes
            .iter()
            .find(|n| n.object_id() == id)
            .map(|n| n.node_type)
    }

    /// The first dword of a scene node's per-type parameter block —
    /// record offset `0x4c`, which the runtime object copies to
    /// `obj+0xf8` (0x4251xx copies six dwords from there). Ladders use it
    /// for a nav coordinate; a type-22 lift uses it as the height it
    /// rises to (`local_e4 = (float)*(int *)(in_ECX + 0xf8)`).
    pub fn object_parameter(&self, id: i32) -> Option<i32> {
        self.scn_file
            .nodes
            .iter()
            .find(|n| n.object_id() == id)
            .map(|n| n.ladder_nav_coord1.0)
    }

    /// 场景类型，`.scn` 头 +114 的那个 dword。原版把它搬到全局
    /// `DAT_00d8bbec`（`0x4229db`：从 `DAT_00c12ba8` 取），**跳跃探测就是拿
    /// 它 == 2 当门槛**（`0x41db97` 把 `ebp` 置 2，`0x41dba5` / `0x41dc5f`
    /// 两处比完才调 `FUN_0041ee60`）；== 1 的那支（`0x41c22f`）每帧把
    /// `+0x21c` 累加进 `+0x188`，像是跟地形高度那套。
    ///
    /// 全量普查 566 个 `.scn`：值 0 有 40 个、1 有 386 个、2 有 140 个，
    /// **是逐 block 的**（同一个 cpk 下三种都有，不是整张图一个值）；2 基本
    /// 集中在 M\* 的 block 上。
    ///
    /// 这个字段就是 `SceneInfo::type`（`gamebase/scenedata.h:238`）：
    /// `SCNHeader` 打包后 18 字节，`SceneInfo` 的前三个 `char[32]` 占 96，
    /// 所以 `type` 正好落在文件 `18 + 96 = 114`。枚举
    /// （同文件 `:226`）是
    ///
    /// * `SCENE_STORYA = 0` 「情节关城中大地」
    /// * `SCENE_STORYB = 1` 「情节关建筑内部」
    /// * `SCENE_FIGHT = 2` 「战斗关」
    ///
    /// 所以「2 才能跳」= 只有战斗关能跳，不是什么「迷宫」。
    pub fn scene_type(&self) -> u32 {
        self.scn_file.global_scene_index
    }

    /// 搜宝鼠（仙宝 6407）扫出来的东西在不在。
    ///
    /// `FUN_00421ea0` 遍历场景对象表（表在 scene+0xe0，步长 0x380，个数在
    /// scene+0xdc），三个字段同时成立才算命中：
    ///
    /// * `record+0xad != 0` —— 对象活着。这个字节是加载时从记录里抄进来的
    ///   （`FUN_0041fa90` 里 `in_ECX[iVar3 * 0x380 + 0x18d] = *(char *)(iVar2 + 4)`），
    ///   脚本命令 ObjectActive 改的也是它，所以这里对应移植版的实体可见性。
    /// * `record+0xf4 == 0x18` —— 节点类型 24。
    /// * `*(int *)(record+0xfc) == 1` —— 参数块的第二个 dword（`.scn` 记录
    ///   0x50，移植版的 `ladder_nav_coord1.1`）。
    ///
    /// 命中就放特效 `FUN_00424c20(DAT_00c116ec, 0xb, 0, 0.001)`，否则放音效
    /// "XMouse"。
    pub fn treasure_object_present(&self) -> bool {
        self.scn_file.nodes.iter().any(|n| {
            n.node_type == 0x18
                && n.ladder_nav_coord1.1 == 1
                && self
                    .get_root_object(n.object_id())
                    .map_or(false, |e| e.visible())
        })
    }

    pub fn object_placement(&self, id: i32) -> Option<(Vec3, f32)> {
        self.scn_file
            .nodes
            .iter()
            .find(|n| n.object_id() == id)
            .map(|n| (n.position, n.rotation.to_radians()))
    }

    pub fn get_root_object(&self, id: i32) -> Option<ComRc<IEntity>> {
        self.scene
            .root_entities()
            .iter()
            .find(|e| e.name() == format!("OBJECT_{}", id))
            .cloned()
    }

    pub fn get_role_entity(&self, id: i32) -> Option<ComRc<IEntity>> {
        let pos = self
            .scene
            .entities()
            .iter()
            .position(|e| e.name() == format!("ROLE_{}", id));

        if let Some(pos) = pos {
            self.scene.entities().get(pos).cloned()
        } else {
            None
        }
    }

    /// 野外怪物撞上主角（`FUN_00417420` 末尾）：场景里类型 2、带怪物 id、还在场（没藏）的角色，和主角
    /// 高度差 < 24、距离 < 两人碰撞半径之和就开战。离得 ≥ 12 时按朝向判偷袭（`FUN_0041ba10` 各查一次：
    /// 主角面前有没有怪、怪面前有没有主角）——怪看得见主角而主角背对它 = 敌方偷袭（2），反过来 = 我方偷袭（1）。
    pub fn touching_field_monster(
        &self,
        player: &ComRc<IEntity>,
        player_radius: f32,
    ) -> Option<FieldEncounter> {
        let p = player.transform().borrow().position();
        let pf = Self::facing_of(player);
        for role in &self.scn_file.roles {
            if !role.is_combat_npc() || role.monster_ids().iter().all(|id| *id == 0) {
                continue;
            }
            let Some(e) = self.get_role_entity(role.index as i32) else {
                continue;
            };
            if !e.visible() {
                continue;
            }
            let m = e.transform().borrow().position();
            // 原版比的是两人的 `+0xa4`，那就是世界高度（角色 `+8` 是相对
            // 高度、站地上恒 0，世界高另存在 `+0x98` / `+0xa4`，见
            // `role_move.md` §409），移植端的位置 y 存的就是世界高，等价。
            if (m.y - p.y).abs() >= 24. {
                continue;
            }
            let dist = Vec3::sub(&m, &p).norm();
            let r = RoleController::get_role_controller(e.clone())
                .map(|rc| rc.inner::<RoleController>().model_radius())
                .unwrap_or(super::role_controller::DEFAULT_MODEL_RADIUS);
            if player_radius + r <= dist {
                continue;
            }
            let sneak = if dist < 12. {
                0
            } else {
                let a = Self::in_front(&p, &pf, &m);
                let b = Self::in_front(&m, &Self::facing_of(&e), &p);
                match (a, b) {
                    (false, true) => 2,
                    (true, false) => 1,
                    _ => 0,
                }
            };
            return Some(FieldEncounter {
                role_index: role.index,
                count: role.monster_count(),
                ids: role.monster_ids(),
                sneak,
            });
        }
        None
    }

    /// 推野外怪物一帧（`FUN_00411f60` → `FUN_00417420`）：决策、受力、找路在
    /// [`super::field_monster::FieldMonster`]，这里挪位置——速度平方超过 0.005 才挪，一步最多 18（`FUN_00411e00`），落点由 `FieldMonster::settle` 判（`FUN_00414350`），高度取
    /// 导航网格，朝向跟速度（`FUN_004122c0`），走着播走的动作、停了播待机。
    pub fn step_field_monsters(
        &self,
        dt: f32,
        player: &ComRc<IEntity>,
        player_layer: usize,
        db: &crate::openpal3::states::game_db::GameDb,
        monster: &crate::openpal3::states::role_move::MonsterConstants,
        role_consts: &crate::openpal3::states::role_move::RoleConstants,
        frozen: bool,
    ) {
        use super::field_monster::{FieldMonster, Levels};
        let player_pos = player.transform().borrow().position();
        let mut monsters = self.field_monsters.borrow_mut();
        if monsters.is_empty() {
            for role in &self.scn_file.roles {
                if !role.is_combat_npc() || role.monster_ids().iter().all(|id| *id == 0) {
                    continue;
                }
                let Some(e) = self.get_role_entity(role.index as i32) else {
                    continue;
                };
                let levels = db
                    .combat_actor(role.monster_ids()[0])
                    .map(|a| Levels {
                        move_range: a.move_range_level,
                        move_speed: a.move_speed_level,
                        chase_speed: a.chase_speed_level,
                    })
                    .unwrap_or_default();
                monsters.push(FieldMonster::new(
                    role.index,
                    (role.position_x, role.position_z),
                    Self::facing_of(&e),
                    RoleController::get_role_controller(e.clone())
                        .map_or(0, |rc| rc.inner::<RoleController>().nav_layer()),
                    levels,
                    monster,
                    role_consts,
                ));
            }
        }
        let mut rng = self.object_rng.borrow_mut();
        for m in monsters.iter_mut() {
            let Some(e) = self.get_role_entity(m.role_index as i32) else {
                continue;
            };
            if !e.visible() {
                continue;
            }
            let Some(rc) = RoleController::get_role_controller(e.clone()) else {
                continue;
            };
            let rc = rc.inner::<RoleController>();
            m.layer = rc.nav_layer();
            m.frozen = frozen;
            let pos = e.transform().borrow().position();
            let facing = Self::facing_of(&e);
            let before = m.state;
            // 原版角色 +8 是离地相对高度（站地上恒 0），世界高度另存在 +0x98 / +0xa4，所以 AI 里的距离都不带 y。
            let flat = Vec3::new(pos.x, 0., pos.z);
            let player_flat = Vec3::new(player_pos.x, 0., player_pos.z);
            let (v, placed) = m.step(
                dt,
                &flat,
                &facing,
                &player_flat,
                player_layer,
                &mut rng,
                &SceneLayers { scene: self },
                monster,
                role_consts,
            );
            // 跨层追的时候换了层（`FUN_00413c00` / `FUN_0041b6c0`）。
            if m.layer != rc.nav_layer() {
                rc.set_nav_layer(m.layer);
            }
            let layer = m.layer;
            let nav = FieldNavView { scene: self, layer };
            if m.state != before {
                log::info!(
                    "Field AI: ROLE_{} {:?} → {:?}，位置 ({:.0}, {:.0})",
                    m.role_index,
                    before,
                    m.state,
                    pos.x,
                    pos.z
                );
            }
            let pos = match placed {
                Some(p) => self.place_field_monster(&e, layer, p, None),
                None => pos,
            };
            if v.x * v.x + v.z * v.z <= 0.005 {
                rc.idle();
                continue;
            }
            let mut step = Vec3::scalar_mul(dt, &v);
            let len = step.norm();
            if len > 18. {
                step = Vec3::scalar_mul(18. / len, &step);
            }
            match m.settle(&pos, &Vec3::add(&pos, &step), &facing, &nav) {
                // `FUN_00411f60`：落下去地面高度跟原来差 45 以上就退回原处、停下。
                Some(to)
                    if (self.get_height(layer, self.scene_coord_to_nav_coord(layer, &to))
                        - pos.y)
                        .abs()
                        > 45. =>
                {
                    m.stop();
                    rc.idle();
                }
                Some(to) => {
                    self.place_field_monster(&e, layer, to, Some(&v));
                    rc.walk();
                }
                None => rc.idle(),
            }
        }
    }

    /// 把怪放到 `to`，高度取导航网格；给了速度就面朝速度方向。
    fn place_field_monster(
        &self,
        e: &ComRc<IEntity>,
        layer: usize,
        mut to: Vec3,
        facing: Option<&Vec3>,
    ) -> Vec3 {
        to.y = self.get_height(layer, self.scene_coord_to_nav_coord(layer, &to));
        let transform = e.transform();
        let mut t = transform.borrow_mut();
        t.set_position(&to);
        if let Some(v) = facing {
            t.look_at(&Vec3::new(to.x + v.x, to.y, to.z + v.z));
        }
        to
    }

    /// 打赢 / 逃掉以后撞上的那只怪退场（`FUN_004055d0`：`*(DAT_00c11754 + 0x278) = 0`）。重进场景时从
    /// `.scn` 重新摆，所以还会回来。
    pub fn retire_field_monster(&self, role_index: u8) {
        if let Some(e) = self.get_role_entity(role_index as i32) {
            e.set_visible(false);
            if let Some(rc) = RoleController::get_role_controller(e.clone()) {
                rc.inner::<RoleController>().set_active(false);
            }
        }
    }

    /// 角色面朝哪边：`Transform::look_at` 把「背离目标」写在矩阵第 2 列，取反就是面朝的方向。
    pub fn facing_of(entity: &ComRc<IEntity>) -> Vec3 {
        let t = entity.transform();
        let t = t.borrow();
        let m = t.matrix();
        Vec3::new(-m[0][2], -m[1][2], -m[2][2])
    }

    /// `FUN_0041ba10(this, pos)`：`pos` 在 `this` 面前 500 以内（点积 > 0；贴得不到 1 也算）。
    fn in_front(this: &Vec3, facing: &Vec3, pos: &Vec3) -> bool {
        let d = Vec3::sub(pos, this);
        let len = d.norm();
        if len > 500. {
            return false;
        }
        if len < 1. {
            return true;
        }
        (d.x * facing.x + d.y * facing.y + d.z * facing.z) / len > 0.
    }

    /// All role entities currently in the scene (named `ROLE_<n>`).
    pub fn role_entities(&self) -> Vec<ComRc<IEntity>> {
        self.scene
            .entities()
            .iter()
            .filter(|e| e.name().starts_with("ROLE_"))
            .cloned()
            .collect()
    }

    /// Advance ambient NPC patrols by `delta_sec`. Only roles that were given a
    /// patrol path at load time (non-scripted townsfolk) move; scripted roles
    /// are untouched. Movement is ground-snapped via the role's nav layer.
    /// Start sliding scene object `object_id` by `offset` over
    /// `duration` seconds — opcode 90 (`LAB_004378e0`). A second move on
    /// the same object replaces the first, the way re-queueing onto the
    /// object's ring does.
    /// Returns where the object will end up, or `None` if there is no
    /// such object.
    pub fn move_object(&self, object_id: i32, offset: Vec3, duration: f32) -> Option<Vec3> {
        let entity = self.get_root_object(object_id).or_else(|| {
            log::warn!("ObjectMove: 找不到场景物件 {object_id}");
            None
        })?;
        let from = entity.transform().borrow().position();
        let to = Vec3::add(&from, &offset);

        let mut moves = self.object_moves.borrow_mut();
        moves.retain(|m| m.object_id != object_id);
        if duration <= 0. {
            entity.transform().borrow_mut().set_position(&to);
        } else {
            moves.push(ObjectMove {
                object_id,
                entity,
                from,
                to,
                duration,
                elapsed: 0.,
            });
        }
        Some(to)
    }

    /// Advance the opcode-90 slides. Called once a frame beside
    /// [`Self::step_patrols`].
    pub fn step_object_moves(&self, delta_sec: f32) {
        let mut moves = self.object_moves.borrow_mut();
        moves.retain_mut(|m| {
            m.elapsed += delta_sec;
            let done = m.elapsed >= m.duration;
            let t = if done { 1. } else { m.elapsed / m.duration };
            let position = Vec3::add(&m.from, &Vec3::scalar_mul(t, &Vec3::sub(&m.to, &m.from)));
            m.entity.transform().borrow_mut().set_position(&position);
            !done
        });
    }

    /// 类型 17 物件每帧的动作（`FUN_00426eb0` case 0x11，见 `cvd_playback.md` §2.1）。
    /// 和 [`Self::step_object_moves`] 一起每帧调。
    pub fn step_scene_objects(&self, delta_sec: f32) {
        let mut rng = self.object_rng.borrow_mut();
        for m in self.object_motions.borrow_mut().iter_mut() {
            if let Some(replay) = m.replay.as_mut() {
                let finished = m
                    .cvds
                    .iter()
                    .all(|c| c.inner::<CvdModelComponent>().finished());
                match replay.step(delta_sec, finished, || rng.rand()) {
                    IdleReplayStep::Play => {
                        for c in &m.cvds {
                            c.inner::<CvdModelComponent>()
                                .finish_cycle(IDLE_REPLAY_SPEED);
                        }
                    }
                    IdleReplayStep::Rewind => {
                        for c in &m.cvds {
                            c.inner::<CvdModelComponent>().rewind();
                        }
                    }
                    IdleReplayStep::Nothing => {}
                }
            }
            match m.motion {
                1 => {
                    m.clock += delta_sec;
                    let transform = m.entity.transform();
                    let mut t = transform.borrow_mut();
                    let mut p = t.position();
                    p.y = m.clock.sin() * FLOAT_AMPLITUDE + m.base_y;
                    t.set_position(&p);
                }
                2 => {
                    m.entity.transform().borrow_mut().rotate_axis_angle_local(
                        &Vec3::UP,
                        (delta_sec * SPIN_DEGREES_PER_SEC).to_radians(),
                    );
                }
                _ => {}
            }
        }
    }

    /// 建物件时记下类型 17 要每帧做的事（`FUN_00423a70` `0x42409f`）。
    fn register_object_motion(&self, entity: &ComRc<IEntity>, obj: &ScnNode) {
        if obj.node_type != 17 {
            return;
        }
        // 原版不管 p1 都抽一次 `+0x48 = rand % 10`。
        let first = self.object_rng.borrow_mut().rand();
        let mut cvds = vec![];
        cvd_components(entity, &mut cvds);
        // p1（+0xfc）为 0 且模型是 CVD 才重播；POL 物件没有 CVD 组件。
        let replay =
            (obj.ladder_nav_coord1.1 == 0 && !cvds.is_empty()).then(|| IdleReplay::new(first));
        // p2（+0x100，scn 记录 +0x54）：1 浮动、2 旋转。
        let motion = obj.ladder_nav_coord2.0;
        if replay.is_none() && motion != 1 && motion != 2 {
            return;
        }
        self.object_motions.borrow_mut().push(ObjectMotion {
            entity: entity.clone(),
            cvds,
            replay,
            motion,
            clock: 0.,
            base_y: obj.position.y,
        });
    }

    pub fn step_patrols(&self, delta_sec: f32) {
        // Default walk speed when the role record carries no per-role speed
        // (0x150 is zero for most patrol roles).
        const DEFAULT_PATROL_SPEED: f32 = 60.;
        for entity in self.role_entities() {
            let rc = match RoleController::get_role_controller(entity.clone()) {
                Some(rc) => rc,
                None => continue,
            };
            let controller = rc.inner::<RoleController>();
            if !controller.has_patrol() {
                continue;
            }
            let target = match controller.patrol_target() {
                Some(t) => t,
                None => continue,
            };
            let layer = controller.nav_layer();
            let position = entity.transform().borrow().position();
            let to = Vec3::new(target.x - position.x, 0., target.z - position.z);
            let dist = (to.x * to.x + to.z * to.z).sqrt();
            let speed = {
                let s = controller.patrol_speed();
                if s > f32::EPSILON {
                    s
                } else {
                    DEFAULT_PATROL_SPEED
                }
            };
            let step = speed * delta_sec;

            if dist <= step.max(1.0) {
                controller.advance_patrol();
                continue;
            }

            let dir = Vec3::new(to.x / dist, 0., to.z / dist);
            let mut new_position = Vec3::add(&position, &Vec3::scalar_mul(step, &dir));
            let nav_coord = self.scene_coord_to_nav_coord(layer, &new_position);
            new_position.y = self.get_height(layer, nav_coord);

            let look_at = Vec3::new(target.x, new_position.y, target.z);
            entity
                .transform()
                .borrow_mut()
                .look_at(&look_at)
                .set_position(&new_position);
            controller.walk();
        }
    }

    fn test_sphere_aabb(s: &Vec3, r: f32, aabb1: &Vec3, aabb2: &Vec3) -> bool {
        macro_rules! dist_sqr {
            ($s: expr_2021, $min: expr_2021, $max: expr_2021) => {
                if $s < $min {
                    ($min - $s) * ($min - $s)
                } else if $s > $max {
                    ($s - $max) * ($s - $max)
                } else {
                    0.
                }
            };
        }

        let x = vec![aabb1.x.min(aabb2.x), aabb1.x.max(aabb2.x)];
        let y = vec![aabb1.y.min(aabb2.y), aabb1.y.max(aabb2.y)];
        let z = vec![aabb1.z.min(aabb2.z), aabb1.z.max(aabb2.z)];

        let dist =
            dist_sqr!(s.x, x[0], x[1]) + dist_sqr!(s.y, y[0], y[1]) + dist_sqr!(s.z, z[0], z[1]);

        dist < r * r
    }

    fn load_objects(&self) {
        let ground_pol_name = self.scn_file.scn_base_name.clone() + ".pol";
        let mut entities: Vec<ComRc<IEntity>> = vec![];

        let scn_object = self.asset_mgr.load_scn_pol(
            &self.cpk_name,
            &self.scn_file.scn_base_name,
            &ground_pol_name,
            self.scn_file.is_night,
            std::u16::MAX,
        );

        if let Some(scn_object) = scn_object {
            Self::apply_position_rotation(scn_object.clone(), &Vec3::new(0., 0., 0.), 0.);
            entities.push(scn_object);
        }

        // 场景自己的 `<场景名>.cvd`（水车、旗子之类，1134 个场景里 22 个有）：原版只找白天
        // 那份（`FUN_0041fe30`，不看夜间的 `1\`），一直按 0.2 倍率循环。
        let scene_cvd = self.asset_mgr.load_scn_cvd(
            &self.cpk_name,
            &self.scn_file.scn_base_name,
            &format!("{}.cvd", self.scn_file.scn_base_name),
            false,
            u16::MAX - 1,
        );
        if let Some(cvd) = scene_cvd {
            Self::apply_position_rotation(cvd.clone(), &Vec3::new(0., 0., 0.), 0.);
            play_cvd_subtree(&cvd, CvdPlayMode::Loop, SCENE_CVD_SPEED);
            entities.push(cvd);
        }

        for obj in &self.scn_file.nodes {
            let mut entity: Option<ComRc<IEntity>> = None;
            if obj.nav_trigger_coord_min.0 != 0
                || obj.nav_trigger_coord_min.1 != 0
                || obj.nav_trigger_coord_max.0 != 0
                || obj.nav_trigger_coord_max.1 != 0
            {
                self.nav_triggers.borrow_mut().push(SceNavTrigger {
                    nav_coord_max: obj.nav_trigger_coord_max,
                    nav_coord_min: obj.nav_trigger_coord_min,
                    node_type: obj.node_type,
                    layer: obj.nav_layer,
                    sce_proc_id: obj.sce_proc_id,
                });
            }

            let visible = obj.node_type != 17 && obj.node_type != 25;
            if obj.node_type != 37 && obj.node_type != 43 && obj.name.len() != 0 {
                if obj.name.as_bytes()[0] as char == '_' {
                    match self.asset_mgr.load_scn_pol(
                        &self.cpk_name,
                        &self.scn_file.scn_base_name,
                        &obj.name,
                        self.scn_file.is_night,
                        obj.object_id() as u16,
                    ) {
                        Some(p) => {
                            entity = Some(p);
                        }
                        _ => {
                            match self.asset_mgr.load_scn_cvd(
                                &self.cpk_name,
                                &self.scn_file.scn_base_name,
                                &obj.name,
                                self.scn_file.is_night,
                                obj.object_id() as u16,
                            ) {
                                Some(c) => {
                                    entity = Some(c);
                                }
                                _ => {
                                    log::error!("Cannot load object: {}", obj.name);
                                }
                            }
                        }
                    }
                } else if obj.name.to_lowercase().ends_with(".pol") {
                    entity = Some(
                        self.asset_mgr
                            .load_object_item_pol(&obj.name, obj.object_id() as u16, visible)
                            .unwrap(),
                    );
                } else if obj.name.to_lowercase().ends_with(".cvd") {
                    entity = Some(
                        self.asset_mgr
                            .load_object_item_cvd(&obj.name, obj.object_id() as u16, visible)
                            .unwrap(),
                    );
                } else if obj.name.as_bytes()[0] as char == '+' {
                    entity = self.build_scene_effect(obj);
                } else {
                    entity = Some(
                        self.asset_mgr
                            .load_object_item_pol(&obj.name, obj.object_id() as u16, visible)
                            .unwrap(),
                    );
                }
            }

            match obj.node_type {
                ScnNodeTypes::LADDER | ScnNodeTypes::LADDER2 => {
                    self.ladder_triggers.borrow_mut().push(LadderTrigger {
                        position: obj.position,
                        nav_coord1: obj.ladder_nav_coord1,
                        nav_coord2: obj.ladder_nav_coord2,
                        switch_layer: obj.ladder_switch_layer != obj.nav_layer as i32
                            || obj.node_type == ScnNodeTypes::LADDER2, // ?? should be obj.ladder_target_layer?
                        sce_proc_id: obj.sce_proc_id,
                    })
                }
                ScnNodeTypes::ITEM_TRIGGER
                | ScnNodeTypes::ITEM_TRIGGER2
                | ScnNodeTypes::ITEM_TRIGGER3
                | ScnNodeTypes::TRIGGER_SOURCE => {
                    self.item_triggers.borrow_mut().push(SceItemTrigger {
                        coord: obj.position,
                        sce_proc_id: obj.sce_proc_id,
                    });
                }
                ScnNodeTypes::TRIGGER_TARGET => {}
                ScnNodeTypes::AABB_TRIGGER => {
                    self.aabb_triggers.borrow_mut().push(SceAabbTrigger {
                        aabb_coord2: obj.aabb_trigger_coord2,
                        aabb_coord1: obj.aabb_trigger_coord1,
                        sce_proc_id: obj.sce_proc_id,
                    });
                }
                _ => {}
            }

            if let Some(p) = entity {
                Self::apply_position_rotation(p.clone(), &obj.position, obj.rotation.to_radians());
                if let Some((mode, speed)) =
                    scene_object_initial_play((obj.node_type & 0xff) as u8, obj.ladder_nav_coord1.1)
                {
                    play_cvd_subtree(&p, mode, speed);
                }
                self.register_object_motion(&p, obj);
                entities.push(p);
            }
        }

        for entity in entities {
            self.scene.add_entity(entity);
        }
        self.build_object_blocking();
        let nodes: Vec<_> = self
            .scn_file
            .nodes
            .iter()
            .map(|n| super::object_action::ObjNode {
                object_id: n.object_id(),
                kind: (n.node_type & 0xff) as u8,
                // `FUN_0041fa90`：层号超出这个场景的层数就当 0。
                layer: if ((n.nav_layer & 0xff) as usize) < self.nav.layer_count() {
                    (n.nav_layer & 0xff) as usize
                } else {
                    0
                },
                p: [
                    n.ladder_nav_coord1.0,
                    n.ladder_nav_coord1.1,
                    n.ladder_nav_coord2.0,
                    n.ladder_nav_coord2.1,
                ],
                link_id: n.w148,
                sound: n.sound_name(),
                proc: n.sce_proc_id as i32,
            })
            .collect();
        *self.object_actions.borrow_mut() = super::object_action::ObjectActions::new(&nodes);
    }

    /// `FUN_00421230`：一格的地面高度。格子上登记了在场的物件时按类型换成物件的高度（物件 +0x2c）：
    /// 5/7/0x13/0x21/0x26/0x28 取物件高度，0x1d（乌龟）再加 24，0x16/0x22/0x25 格子能走时取两者大的、不能走
    /// 时取物件高度。越界是 0。[未做] 0xb 且参数 0 为 1 的按模型矩阵取（`FUN_00429db0`），这里还按格子。
    pub fn floor_height(&self, layer: usize, p: &Vec3) -> f32 {
        use super::field_monster::Layers as _;
        use super::nav_path::{FieldNav as _, NavGrid as _};
        let nav = FieldNavView { scene: self, layer };
        let (x, z) = nav.tile_of(p);
        let (w, h) = self.nav.size(layer);
        if x < 0 || z < 0 || x >= w as i32 || z >= h as i32 {
            return 0.;
        }
        let base = SceneLayers { scene: self }.height(layer, x, z);
        let owner = self.object_blocking.borrow().as_ref().and_then(|b| {
            b.owner(layer, x, z)
                .filter(|i| b.active(*i))
                .map(|i| (i, b.kind(i).unwrap_or(0)))
        });
        let Some((i, kind)) = owner else {
            return base;
        };
        let Some(y) = self
            .object_entity_at(i)
            .map(|e| e.transform().borrow().position().y)
        else {
            return base;
        };
        match kind {
            0x16 | 0x22 | 0x25 if nav.tile_type(x, z) > 1 => base.max(y),
            0x16 | 0x22 | 0x25 | 5 | 7 | 0x13 | 0x21 | 0x26 | 0x28 => y,
            0x1d => y + 24.,
            _ => base,
        }
    }

    /// `.scn` 里第 `i` 个物件的实体。
    pub fn object_entity_at(&self, i: usize) -> Option<ComRc<IEntity>> {
        self.scn_file
            .nodes
            .get(i)
            .and_then(|n| self.get_root_object(n.object_id()))
    }

    /// 第 `i` 个物件在不在场（+0xad）。
    pub fn object_active_at(&self, i: usize) -> bool {
        self.object_blocking
            .borrow()
            .as_ref()
            .is_some_and(|b| b.active(i))
    }

    /// 物件动作 10：第 `i` 个物件的「可通过」。
    pub fn set_object_passable(&self, i: usize, passable: bool) {
        if let Some(b) = self.object_blocking.borrow_mut().as_mut() {
            b.set_passable(i, passable);
        }
    }

    /// 物件动作 9：清掉第 `i` 个物件压住的格子，或者按它现在的位置在第 `layer` 层重新登记。
    pub fn set_object_tiles(&self, i: usize, clear: bool, layer: usize) {
        let pos = self
            .object_entity_at(i)
            .map(|e| e.transform().borrow().position());
        if let Some(b) = self.object_blocking.borrow_mut().as_mut() {
            b.clear(i);
            if !clear {
                if let Some(p) = pos {
                    b.register_at(i, layer, p);
                }
            }
        }
    }

    /// 第 `i` 个物件模型包围盒的 y 边长（`FUN_00429db0` 的 +0xa4）。
    pub fn object_model_height(&self, i: usize) -> f32 {
        self.object_blocking
            .borrow()
            .as_ref()
            .map_or(0., |b| b.model_height(i))
    }

    /// 第 `layer` 层 (`x`, `z`) 格的中心（y 取格子自己的高度，越界是 0）和连物件的地面高度。
    pub fn tile_point(&self, layer: usize, x: i32, z: i32) -> (Vec3, f32) {
        use super::field_monster::Layers as _;
        let min = self.nav_min_coord(layer);
        let (w, h) = self.nav.size(layer);
        let inside = x >= 0 && z >= 0 && x < w as i32 && z < h as i32;
        let y = if inside {
            SceneLayers { scene: self }.height(layer, x, z)
        } else {
            0.
        };
        let c = Vec3::new(
            (x as f32 + 0.5) * NAV_BLOCK_SIZE + min.x,
            y,
            (z as f32 + 0.5) * NAV_BLOCK_SIZE + min.z,
        );
        (c, self.floor_height(layer, &c))
    }

    /// 主角脚下那格登记的物件（在场的）和它的类型。
    fn tile_object(&self, layer: usize, x: i32, z: i32) -> Option<(usize, u8)> {
        let b = self.object_blocking.borrow();
        let b = b.as_ref()?;
        let i = b.owner(layer, x, z)?;
        b.active(i).then(|| (i, b.kind(i).unwrap_or(0)))
    }

    /// `FUN_0041ee60`：主角脚下那格是跳跃区（0x2c）或神树传送点（0x26），能跳。
    ///
    /// `FUN_00420a10(层, x, z)` 确认就是物件格子表：先按该层的
    /// `+0x98`/`+0x9c`（格子宽高）做边界检查，再 `FUN_004209f0` 从字节网格
    /// 取一字节（`表[+0xc] + 行距[+4]·z + x`）当物件号，小于物件数 `+0xdc`
    /// 就返回物件记录 `+0xe0 + 号·0x380`；`FUN_0041ee60` 拿记录的 `+0xf4`
    /// （物件类型）比 0x2c / 0x26。
    pub fn jump_zone(&self, layer: usize, pos: &Vec3) -> bool {
        use super::nav_path::FieldNav as _;
        let (x, z) = FieldNavView { scene: self, layer }.tile_of(pos);
        matches!(self.tile_object(layer, x, z), Some((_, 0x2c | 0x26)))
    }

    /// `FUN_00414a30` 定落点：离 `from` 超过 168 就拉近，高度取连物件的地面高度；从主角那格到落点那格的格子直线
    /// （`FUN_004218b0`）不通就落在直线上最后一个能走的格子中心。直线上每格要在界内，且是跳跃区、能走（记录 +4
    /// 字节 > 1）或地面类型 1（缺口）（`FUN_004214b0`）；能走按 `FUN_00421020`。
    pub fn jump_target(&self, layer: usize, from: &Vec3, aim: &Vec3) -> Vec3 {
        use super::nav_path::FieldNav as _;
        use super::player_jump::{clamp_aim, line};
        let mut t = clamp_aim(from, aim);
        t.y = self.floor_height(layer, &t);
        let nav = FieldNavView { scene: self, layer };
        let (w, h) = self.nav.size(layer);
        let passable = |x: i32, z: i32| {
            if x < 0 || z < 0 || x >= w as i32 || z >= h as i32 {
                return false;
            }
            let owner_kind = self
                .object_blocking
                .borrow()
                .as_ref()
                .and_then(|b| b.owner(layer, x, z).and_then(|i| b.kind(i)));
            if owner_kind == Some(0x2c) {
                return true;
            }
            let raw = self
                .nav
                .get(layer, x, z)
                .map_or(0, |p| p.distance_to_border);
            (raw & 0xff) > 1 || (raw >> 8) & 0xff == 1
        };
        let walkable = |x: i32, z: i32| self.tile_walkable(layer, x, z);
        let (ok, (lx, lz)) = line(nav.tile_of(from), nav.tile_of(&t), passable, walkable);
        if !ok {
            let (c, floor) = self.tile_point(layer, lx, lz);
            t = Vec3::new(c.x, floor, c.z);
        }
        t
    }

    /// 跳下来落地（`FUN_00420b40(主角, dt, 1)`）：脚下那格是在场的神树传送点就发事件 3（带「上一帧也在它上面」），
    /// 不改记下的物件。
    pub fn player_landed<W: super::object_action::World>(
        &self,
        layer: usize,
        pos: &Vec3,
        dt: f32,
        w: &mut W,
    ) {
        use super::nav_path::FieldNav as _;
        let (x, z) = FieldNavView { scene: self, layer }.tile_of(pos);
        if let Some((i, 0x26)) = self.tile_object(layer, x, z) {
            let same = *self.player_object.borrow() == Some(i);
            self.object_actions.borrow_mut().event(i, 3, same, dt, w);
        }
    }

    /// `.scn` 里第 `i` 个物件的脚本编号。
    pub fn object_id_at(&self, i: usize) -> Option<i32> {
        self.scn_file.nodes.get(i).map(|n| n.object_id())
    }

    /// 物件 `object_id` 在 `.scn` 里的序号。
    pub fn object_index_of(&self, object_id: i32) -> Option<usize> {
        self.object_actions.borrow().index_of(object_id)
    }

    /// 读档 / 换场景回来时把物件的状态摆回去。
    pub fn set_object_state(&self, object_id: i32, state: u8) {
        let mut a = self.object_actions.borrow_mut();
        if let Some(i) = a.index_of(object_id) {
            a.set_state(i, state);
        }
    }

    /// 把物件的动画停在最后一帧（放下的吊桥读档回来就是放下的）。
    pub fn hold_object_anim_end(&self, object_id: i32) {
        let Some(e) = self.get_root_object(object_id) else {
            return;
        };
        let mut cvds = vec![];
        cvd_components(&e, &mut cvds);
        for c in cvds {
            c.inner::<CvdModelComponent>().hold_last_frame();
        }
    }

    /// 物件换层（动作 0xe 的物件那一半，读档回来也走这里）：清掉旧格子、按现在的位置在新层登记。
    pub fn set_object_layer(&self, object_id: i32, layer: usize) {
        let Some(i) = self.object_actions.borrow().index_of(object_id) else {
            return;
        };
        self.set_object_tiles(i, false, layer);
    }

    /// 物件 `object_id` 的状态（+0x50）。
    pub fn object_state(&self, object_id: i32) -> Option<u8> {
        let a = self.object_actions.borrow();
        a.index_of(object_id).and_then(|i| a.state(i))
    }

    /// 有物件的动作队列没做完。
    pub fn object_actions_busy(&self) -> bool {
        self.object_actions.borrow().busy()
    }

    /// 脚本叫物件（124 → `FUN_00424c20(player, 2, 0, 0.001)`）：先记下，冒险循环里带着主角处理。
    pub fn post_object_event(&self, object_id: i32, ev: u8) {
        self.object_events.borrow_mut().push((object_id, ev));
    }

    /// 每帧推物件的动作队列和状态（`FUN_00426eb0` 里 `FUN_0042f460` 和状态机那部分），先把脚本记下的事件发出去。
    pub fn step_object_actions<W: super::object_action::World>(&self, dt: f32, w: &mut W) {
        let events: Vec<_> = self.object_events.borrow_mut().drain(..).collect();
        let mut a = self.object_actions.borrow_mut();
        for (id, ev) in events {
            if let Some(i) = a.index_of(id) {
                // 124 调的是 `FUN_00424c20(主角, 2, 0, 0.001)`。
                a.event(i, ev, false, 0.001, w);
            }
        }
        a.step(dt, w);
    }

    /// `FUN_00420b40`（事件 0）：主角脚下那格登记了在场的物件就叫它（带「上一帧也站在它上面」）；换了物件时
    /// 旧的那个走 `FUN_0042af80`。
    pub fn player_on_object<W: super::object_action::World>(
        &self,
        layer: usize,
        pos: &Vec3,
        dt: f32,
        w: &mut W,
    ) {
        use super::nav_path::FieldNav as _;
        let (x, z) = FieldNavView { scene: self, layer }.tile_of(pos);
        let here = self
            .object_blocking
            .borrow()
            .as_ref()
            .and_then(|b| b.owner(layer, x, z));
        let last = *self.player_object.borrow();
        let mut a = self.object_actions.borrow_mut();
        if let Some(i) = here {
            if self.object_active_at(i) {
                a.event(i, 0, here == last, dt, w);
            }
        }
        if let (Some(l), true) = (last, last != here) {
            a.leave(l);
        }
        *self.player_object.borrow_mut() = here;
    }

    /// 建场景物件挡路的格子表（`FUN_0041fa90` → `FUN_00420940`），见 [`super::object_block`]。
    fn build_object_blocking(&self) {
        use super::object_block::{BlockNode, LayerGrid, ObjectBlocking};
        let layers = self.nav.layer_count();
        let grids = (0..layers)
            .map(|l| {
                let (w, h) = self.nav.size(l);
                let min = self.nav_min_coord(l);
                LayerGrid {
                    w: w as i32,
                    h: h as i32,
                    min_x: min.x,
                    min_z: min.z,
                }
            })
            .collect();
        let nodes: Vec<BlockNode> = self
            .scn_file
            .nodes
            .iter()
            .map(|n| {
                let layer = (n.nav_layer & 0xff) as usize;
                BlockNode {
                    object_id: n.object_id(),
                    name: n.name.clone(),
                    kind: (n.node_type & 0xff) as u8,
                    trigger_type: (n.w24 & 0xff) as u8,
                    non_blocking: (n.w24 >> 8) as u8,
                    init_active: (n.index >> 8) != 0,
                    switch_state: (n.w2 >> 8) as u8,
                    // `FUN_0041fa90`：层号超出这个场景的层数就当 0。
                    layer: if layer < layers { layer } else { 0 },
                    p0: n.ladder_nav_coord1.0,
                    p3: n.ladder_nav_coord2.1,
                    trigger_rect: [
                        n.nav_trigger_coord_min.0,
                        n.nav_trigger_coord_min.1,
                        n.nav_trigger_coord_max.0,
                        n.nav_trigger_coord_max.1,
                    ],
                    position: n.position,
                    y_deg: n.rotation,
                    x_deg: f32::from_bits(n.dw184.first().copied().unwrap_or(0)),
                }
            })
            .collect();
        let models: Vec<_> = self
            .scn_file
            .nodes
            .iter()
            .map(|n| self.object_model_box(&n.name))
            .collect();
        let b = ObjectBlocking::build(
            &nodes,
            &models,
            grids,
            (&self.cpk_name, &self.scn_file.scn_base_name),
        );
        for (i, id, kind, tiles, nb) in b.summary() {
            log::debug!(
                "物件挡路：{}/{} #{i} OBJECT_{id} 类型 {kind:#x} {} 压 {tiles} 格 {}",
                self.cpk_name,
                self.scn_file.scn_base_name,
                self.scn_file.nodes[i].name,
                if nb { "可通过" } else { "挡路" }
            );
        }
        *self.object_blocking.borrow_mut() = Some(b);
    }

    /// 物件模型的包围盒（`FUN_00429af0` 载入时把动画停在第 0 帧调 `gbGeomNode::GetTotalOBBox`）。
    ///
    /// `gbGeomNode::GetTotalOBBox`（`engine/geometry/gbgeomnode.cpp:72`）先把自己和所有子节点里
    /// 带网格的收集起来：
    ///
    /// * 0 个 → 自己的 `GetOBBox`，没网格就三个边长都是 0；
    /// * 1 个 → 那个节点的 `GetOBBox`：矩阵取节点的 to-world，边长取网格 AABB 的
    ///   `max − min`，再把 AABB 的中心 `LeftTranslate` 上去；
    /// * ≥ 2 个 → 每个网格 AABB 过自己节点的 to-world，取并集，矩阵是单位阵只放中心。
    ///
    /// **最后那一步出货的 `gbengine.dll` 和这份源码不一样**：源码写的是
    /// `tx/ty/tz = (min+max)*0.5` 三个轴各算各的，DLL 里 `0x10010222` 起却是
    /// `[esi+0x30]` 和 `[esi+0x34]` 都写 `(max.x+min.x)*0.5`、`[esi+0x38]` 写
    /// `(max.y+min.y)*0.5`——**x 中心写了两遍、z 中心丢了**。这里照 DLL 抄，
    /// 因为跑起来的是 DLL。（也就是说 cpk 里那份 `data/engine` 和出货的
    /// gbengine.dll 不是同一次编译，用它当参考时每条都得对一遍。）
    ///
    /// POL 取文件里各网格包围盒的并——`gbStaticMesh` 的 AABB 是**从文件里直接读的**
    /// （`engine/geometry/gbstaticmesh.cpp:305`），不是算出来的。
    ///
    /// CVD 这里用的是第 0 帧顶点的最小 / 最大值；源码里 `gbIpolMesh` 读文件时
    /// 是**把所有帧都扫一遍**取并集（`engine/geometry/gbipolmesh.cpp:234`），
    /// 但上面那条说明这份源码不能直接当 DLL 的行为用，DLL 里这一段还没对过，
    /// 所以先按现状留着。
    fn object_model_box(&self, name: &str) -> super::object_block::Model {
        use super::object_block::{Model, Obb};
        use common::store_ext::StoreExt2;
        use radiance::math::{Mat44, Transform};
        if name.is_empty() {
            return Model::None;
        }
        if name.starts_with('+') {
            return Model::Fx;
        }
        let Some(path) = self.asset_mgr.scn_object_model_path(
            &self.cpk_name,
            &self.scn_file.scn_base_name,
            name,
            self.scn_file.is_night,
        ) else {
            return Model::None;
        };
        let translate = |c: [f32; 3]| {
            let mut m = Mat44::new_identity();
            m.floats_mut()[0][3] = c[0];
            m.floats_mut()[1][3] = c[1];
            m.floats_mut()[2][3] = c[2];
            m
        };
        let xf = |m: &Mat44, p: [f32; 3]| {
            let f = m.floats();
            [0, 1, 2].map(|r| f[r][0] * p[0] + f[r][1] * p[1] + f[r][2] * p[2] + f[r][3])
        };
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
        if ext.as_deref() == Some("pol") {
            let Ok(data) = self.asset_mgr.vfs().read_to_end(&path) else {
                return Model::None;
            };
            let Ok(pol) = fileformats::pol::read_pol(&mut std::io::Cursor::new(data)) else {
                return Model::None;
            };
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for m in &pol.meshes {
                for (k, (a, b)) in [
                    (m.aabb_min.x, m.aabb_max.x),
                    (m.aabb_min.y, m.aabb_max.y),
                    (m.aabb_min.z, m.aabb_max.z),
                ]
                .into_iter()
                .enumerate()
                {
                    lo[k] = lo[k].min(a);
                    hi[k] = hi[k].max(b);
                }
            }
            if lo[0] > hi[0] {
                return Model::None;
            }
            return Model::Box(Obb {
                m: translate([0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5)),
                ext: [0, 1, 2].map(|k| hi[k] - lo[k]),
            });
        }
        let Ok(cvd) =
            crate::openpal3::loaders::cvd_loader::cvd_load_from_file(self.asset_mgr.vfs(), &path)
        else {
            return Model::None;
        };
        // 各带网格节点：第 0 帧的节点矩阵（和 `CvdModelComponent::setup_transform` 同样的拼法，父子逐级相乘）
        // 和网格第 0 帧的最小 / 最大点。
        fn walk(
            nodes: &[crate::openpal3::loaders::cvd_loader::CvdModelNode],
            parent: &Mat44,
            out: &mut Vec<(Mat44, [f32; 3], [f32; 3])>,
        ) {
            for node in nodes {
                let mut world = *parent;
                if let Some(model) = &node.model {
                    let mut t = Transform::new();
                    if let Some(f) = model
                        .position_keyframes
                        .as_ref()
                        .and_then(|k| k.frames.first())
                    {
                        t.translate_local(&f.position);
                    }
                    let sf = model.scale_factor;
                    t.scale_local(&Vec3::new(sf, sf, sf));
                    if let Some(f) = model
                        .rotation_keyframes
                        .as_ref()
                        .and_then(|k| k.frames.first())
                    {
                        t.rotate_quaternion_local(&f.quaternion);
                    }
                    if let Some(f) = model
                        .scale_keyframes
                        .as_ref()
                        .and_then(|k| k.frames.first())
                    {
                        let q2 = f.quaternion;
                        let mut q3 = q2;
                        q3.inverse();
                        t.rotate_quaternion_local(&q2)
                            .scale_local(&f.scale)
                            .rotate_quaternion_local(&q3);
                    }
                    world = Mat44::multiplied(parent, t.matrix());
                    if let Some(frame) = model.mesh.frames.first().filter(|f| !f.is_empty()) {
                        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                        for v in frame {
                            for (k, c) in [v.position.x, v.position.y, v.position.z]
                                .into_iter()
                                .enumerate()
                            {
                                lo[k] = lo[k].min(c);
                                hi[k] = hi[k].max(c);
                            }
                        }
                        out.push((world, lo, hi));
                    }
                }
                if let Some(children) = &node.children {
                    walk(children, &world, out);
                }
            }
        }
        let mut meshes = vec![];
        walk(&cvd.models, &Mat44::new_identity(), &mut meshes);
        match meshes.as_slice() {
            [] => Model::None,
            [(m, lo, hi)] => Model::Box(Obb {
                m: Mat44::multiplied(m, &translate([0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5))),
                ext: [0, 1, 2].map(|k| hi[k] - lo[k]),
            }),
            many => {
                let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
                for (m, a, b) in many {
                    for c in 0..8 {
                        let p = xf(
                            m,
                            [
                                if c & 1 == 0 { a[0] } else { b[0] },
                                if c & 2 == 0 { a[1] } else { b[1] },
                                if c & 4 == 0 { a[2] } else { b[2] },
                            ],
                        );
                        for k in 0..3 {
                            lo[k] = lo[k].min(p[k]);
                            hi[k] = hi[k].max(p[k]);
                        }
                    }
                }
                let cx = (hi[0] + lo[0]) * 0.5;
                let cy = (hi[1] + lo[1]) * 0.5;
                Model::Box(Obb {
                    m: translate([cx, cx, cy]),
                    ext: [0, 1, 2].map(|k| hi[k] - lo[k]),
                })
            }
        }
    }

    /// `FUN_00421020`：导航格 (x, z) 能不能走——格子记录 +4 字节 > 1，再按格子上登记的场景物件改判。
    pub fn tile_walkable(&self, layer: usize, x: i32, z: i32) -> bool {
        let base = self
            .nav
            .get(layer, x, z)
            .is_some_and(|t| (t.distance_to_border & 0xff) > 1);
        match &*self.object_blocking.borrow() {
            Some(b) => b.walkable(self.nav.layer(layer), x, z, base),
            None => base,
        }
    }

    /// 场景坐标落在的那一格能不能走（格子坐标截断）。
    pub fn walkable_at(&self, layer: usize, p: &Vec3) -> bool {
        let (x, z) = self.scene_coord_to_nav_coord(layer, p);
        self.tile_walkable(layer, x as i32, z as i32)
    }

    /// 脚本改了物件在不在场（`ObjectActive`），挡路跟着变。
    pub fn set_object_active(&self, object_id: i32, active: bool) {
        if let Some(b) = self.object_blocking.borrow_mut().as_mut() {
            b.set_active(object_id, active);
        }
    }

    /// Build a PAL3 scene "effect" entity for a `+`-prefixed node.
    ///
    /// These nodes mark `EffectScn` effects (candles, oil lamps, torches,
    /// fires). The visual-effect class is node_type 17 ("+3"); the effect
    /// variant is selected by `dw184[3]` (record offset 0x190). We build
    /// the static holder mesh (where the effect has one) plus an animated
    /// additive flame billboard.
    fn build_scene_effect(
        &self,
        obj: &crate::openpal3::loaders::scn_loader::ScnNode,
    ) -> Option<ComRc<IEntity>> {
        if obj.node_type != 17 {
            return None;
        }
        let effect_id = obj.dw184.get(3).copied().unwrap_or(0);
        self.asset_mgr
            .load_scene_effect(effect_id, obj.object_id() as u16)
    }

    fn apply_position_rotation(entity: ComRc<IEntity>, position: &Vec3, rotation: f32) {
        entity
            .transform()
            .borrow_mut()
            .set_position(position)
            .rotate_axis_angle_local(&Vec3::UP, rotation);
    }

    fn map_role_id(role_id: i32) -> i32 {
        match role_id {
            0 => 101,
            1 => 104,
            2 => 105,
            3 => 107,
            4 => 102,
            5 => 109,
            // 11 => 110,
            x => x,
        }
    }

    fn load_roles(&self) {
        for i in &[0, 1, 2, 3, 4, 5] {
            let entity_name = format!("ROLE_{}", i);
            let model_name = Self::map_role_id(*i).to_string();
            let entity = self
                .asset_mgr
                .load_role(&model_name, "C01", entity_name, false)
                .unwrap();
            self.scene.add_entity(entity);
        }

        let mut entities = vec![];
        for role in &self.scn_file.roles {
            if let Some(entity) = self.asset_mgr.load_role(
                &role.name,
                &role.action_name,
                format!("ROLE_{}", role.index),
                false,
            ) {
                let entity = entity;
                let layer = (role.nav_layer as usize).min(self.nav.layer_count().saturating_sub(1));
                let nav_coord = self.scene_coord_to_nav_coord(
                    layer,
                    &Vec3::new(role.position_x, 0., role.position_z),
                );
                let height = self.get_height(layer, nav_coord);
                // Role model's authored forward is +Z (SOUTH); apply the role's
                // facing (degrees, 0x44) on top of that base. Sign mirrors
                // SceCommandRoleTurnFace (`-degree.to_radians()`). The base is a
                // single tunable constant for visual calibration.
                const ROLE_FACING_BASE: f32 = std::f32::consts::PI;
                let facing = ROLE_FACING_BASE - role.facing.to_radians();
                entity
                    .transform()
                    .borrow_mut()
                    .set_position(&Vec3::new(role.position_x, height, role.position_z))
                    .rotate_axis_angle_local(&Vec3::UP, facing);

                let role_controller = entity.get_component(IRoleController::uuid()).unwrap();
                let role_controller = role_controller
                    .query_interface::<IRoleController>()
                    .unwrap();
                {
                    let rc = role_controller.inner::<RoleController>();
                    rc.set_nav_layer(layer);
                    // 开场在不在场只看 `.scn` 的 InitActive（+0x54，`FUN_00417040` 拷到角色 +0x100；
                    // Pal3.Unity `ActorController.Init` 也只看它）。以前只激活有巡逻路径或挂脚本的，
                    // 原地站着的野外怪、路人都没摆出来，挂脚本却初始不在场的反倒提前露面。
                    rc.set_active(role.flag_0x54 != 0);
                    // Only non-scripted ambient NPCs auto-patrol, so they don't
                    // fight SCE RoleMoveTo/RolePathTo.
                    // 野外怪物不走 `.scn` 的路径：`FUN_00417040` 把它们的行为一律改成 5（追击）。
                    if role.sce_proc_id == 0 && role.patrol_path.len() >= 2 && !role.is_combat_npc()
                    {
                        rc.set_patrol(
                            role.patrol_path.clone(),
                            role.path_mode,
                            role.move_speed as f32,
                        );
                    }
                }
                if role.sce_proc_id != 0 {
                    let rc = role_controller.inner::<RoleController>();
                    rc.set_proc_id(role.sce_proc_id as i32);
                }

                entities.push(entity);
            }
        }

        for e in entities {
            self.scene.add_entity(e);
        }
    }
}

struct ScnNodeTypes;
impl ScnNodeTypes {
    pub const LADDER: u16 = 15;
    pub const ITEM_TRIGGER2: u16 = 11;
    pub const ITEM_TRIGGER: u16 = 16;
    pub const AABB_TRIGGER: u16 = 20;
    pub const TRIGGER_TARGET: u16 = 22;
    pub const TRIGGER_SOURCE: u16 = 23;
    pub const ITEM_TRIGGER3: u16 = 33;

    // Elevators
    pub const LADDER2: u16 = 40;
}

/// 导航格宽（见 [`Nav::new`]）。
pub const NAV_BLOCK_SIZE: f32 = 12.;

pub struct Nav {
    nav_file: NavFile,
    block_sizes: Vec<(f32, f32)>,
}

impl Nav {
    /// See [`ScnScene::nav_layer`].
    pub fn layer(&self, layer: usize) -> usize {
        layer.min(self.nav_file.maps.len().saturating_sub(1))
    }

    /// 一层的格数（宽, 高）。
    pub fn size(&self, layer: usize) -> (u32, u32) {
        let m = &self.nav_file.maps[self.layer(layer)];
        (m.width, m.height)
    }

    pub fn new(nav_file: NavFile) -> Self {
        let mut block_sizes = vec![];
        // 原版的格宽是写死的 12（寻路、找层间通道、传送板三处各有一份全局，都在静态初始化时从
        // 0x56a6cc / 0x56a984 / 0x56ab10 拷过来，值都是 12.0），不按文件里的范围算。原先这里用
        // 范围 / (格数 + 1)，M01 上是 11.999 / 11.994，离最小坐标远了会差出一格。
        for _ in 0..nav_file.maps.len() {
            block_sizes.push((NAV_BLOCK_SIZE, NAV_BLOCK_SIZE))
        }
        Self {
            nav_file,
            block_sizes,
        }
    }

    pub fn round_nav_coord(&self, layer: usize, nav_coord: (f32, f32)) -> (i32, i32) {
        let nav_coord_floor = (
            (nav_coord.0.floor() as i32)
                .clamp(0, self.nav_file.maps[self.layer(layer)].width as i32 - 1),
            (nav_coord.1.floor() as i32)
                .clamp(0, self.nav_file.maps[self.layer(layer)].height as i32 - 1),
        );

        nav_coord_floor
    }

    pub fn layer_count(&self) -> usize {
        self.nav_file.maps.len()
    }

    pub fn get_map_size(&self, layer: usize) -> (usize, usize) {
        (
            self.nav_file.maps[self.layer(layer)].width as usize,
            self.nav_file.maps[self.layer(layer)].height as usize,
        )
    }

    pub fn get(&self, layer: usize, nav_coord_x: i32, nav_coord_z: i32) -> Option<NavMapPoint> {
        if nav_coord_x < 0
            || nav_coord_z < 0
            || nav_coord_x as u32 >= self.nav_file.maps[self.layer(layer)].width
            || nav_coord_z as u32 >= self.nav_file.maps[self.layer(layer)].height
        {
            None
        } else {
            Some(
                self.nav_file.maps[self.layer(layer)].map[nav_coord_z as usize]
                    [nav_coord_x as usize],
            )
        }
    }

    pub fn check_connectivity(
        &self,
        layer: usize,
        nav_coord_src: (i32, i32),
        nav_coord_dest: (i32, i32),
    ) -> bool {
        let mut visited = HashSet::new();
        self.check_connectivity_internal(layer, nav_coord_src, nav_coord_dest, &mut visited, 6)
    }

    pub fn print_map(&self) {
        for layer in 0..self.nav_file.maps.len() {
            for j in 0..self.nav_file.maps[self.layer(layer)].height {
                for i in 0..self.nav_file.maps[self.layer(layer)].width {
                    print!(
                        "{} ",
                        self.nav_file.maps[self.layer(layer)].map[j as usize][i as usize]
                            .distance_to_border
                    );
                }

                println!();
            }
            println!("==========");
        }
    }

    fn check_connectivity_internal(
        &self,
        layer: usize,
        nav_coord_src: (i32, i32),
        nav_coord_dest: (i32, i32),
        visited: &mut HashSet<(i32, i32)>,
        remaining_steps: i32,
    ) -> bool {
        if nav_coord_src == nav_coord_dest {
            return true;
        }

        if remaining_steps < 0 {
            return false;
        }

        visited.insert(nav_coord_src);

        // Obviously we can optimize this
        let directions = [(1, 0), (-1, 0), (0, 1), (0, -1)];
        for d in &directions {
            let target_coord = (nav_coord_src.0 + d.0, nav_coord_src.1 + d.1);
            if visited.contains(&target_coord) {
                continue;
            }

            if let Some(point) = self.get(layer, target_coord.0, target_coord.1) {
                if point.distance_to_border != 0
                    && self.check_connectivity_internal(
                        layer,
                        target_coord,
                        nav_coord_dest,
                        visited,
                        remaining_steps - 1,
                    )
                {
                    return true;
                }
            }
        }

        visited.remove(&nav_coord_src);
        false
    }
}

pub struct SceNavTrigger {
    nav_coord_min: (i32, i32),
    nav_coord_max: (i32, i32),
    node_type: u16,
    layer: u16,
    sce_proc_id: u32,
}

pub struct SceAabbTrigger {
    aabb_coord1: Vec3,
    aabb_coord2: Vec3,
    sce_proc_id: u32,
}

pub struct SceItemTrigger {
    coord: Vec3,
    sce_proc_id: u32,
}

pub struct LadderTrigger {
    position: Vec3,
    nav_coord1: (i32, i32),
    nav_coord2: (i32, i32),
    switch_layer: bool,
    sce_proc_id: u32,
}

#[allow(dead_code)]
pub struct TriggerTarget {}

pub enum LadderTestResult {
    SceProc(u32),
    NewPosition((bool, Vec3)),
}

/// 一个类型 17 物件每帧要做的事，见 [`ScnScene::step_scene_objects`]。
struct ObjectMotion {
    entity: ComRc<IEntity>,
    cvds: Vec<ComRc<ICvdModel>>,
    replay: Option<IdleReplay>,
    /// `+0x100`：1 浮动、2 旋转、其余不动。
    motion: i32,
    /// `+0x54`：浮动用的秒数。
    clock: f32,
    /// `+0xd8`：浮动的基准高度。
    base_y: f32,
}

/// One in-flight opcode-90 slide.
struct ObjectMove {
    object_id: i32,
    entity: ComRc<IEntity>,
    from: Vec3,
    to: Vec3,
    duration: f32,
    elapsed: f32,
}

/// 野外怪物看到的几层导航网格和层间矩形（导航文件 v2 每层 8 个）。
struct SceneLayers<'a> {
    scene: &'a ScnScene,
}

impl super::field_monster::Layers for SceneLayers<'_> {
    type Nav<'b>
        = FieldNavView<'b>
    where
        Self: 'b;

    fn nav(&self, layer: usize) -> FieldNavView<'_> {
        FieldNavView {
            scene: self.scene,
            layer,
        }
    }

    fn trigger(&self, layer: usize, idx: usize) -> Option<[i32; 4]> {
        let map = self.scene.nav.nav_file.maps.get(layer)?;
        let t = map.layer_triggers.as_ref()?.get(idx)?;
        Some([
            t.nav_coord_min.0,
            t.nav_coord_min.1,
            t.nav_coord_max.0,
            t.nav_coord_max.1,
        ])
    }

    fn height(&self, layer: usize, x: i32, z: i32) -> f32 {
        self.scene.nav.get(layer, x, z).map_or(0., |t| t.height)
    }
}

/// 野外怪物找路看到的一层导航网格。能不能走是 [`ScnScene::tile_walkable`]（`FUN_00421020`，含场景物件），
/// 代价类别是格子记录 +4 那个字节。
struct FieldNavView<'a> {
    scene: &'a ScnScene,
    layer: usize,
}

impl super::nav_path::NavGrid for FieldNavView<'_> {
    fn walkable(&self, x: i32, z: i32) -> bool {
        self.scene.tile_walkable(self.layer, x, z)
    }

    fn tile_type(&self, x: i32, z: i32) -> u8 {
        self.scene
            .nav
            .get(self.layer, x, z)
            .map_or(0, |t| (t.distance_to_border & 0xff) as u8)
    }
}

impl super::nav_path::FieldNav for FieldNavView<'_> {
    fn tile_of(&self, p: &Vec3) -> (i32, i32) {
        let (x, z) = self.scene.scene_coord_to_nav_coord(self.layer, p);
        (x as i32, z as i32)
    }

    fn tile_center(&self, x: i32, z: i32) -> Vec3 {
        let min = self.scene.nav_min_coord(self.layer);
        let block = self.scene.nav_block_size(self.layer);
        Vec3::new(
            (x as f32 + 0.5) * block.0 + min.x,
            0.,
            (z as f32 + 0.5) * block.1 + min.z,
        )
    }
}
