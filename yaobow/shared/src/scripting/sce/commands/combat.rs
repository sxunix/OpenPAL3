//! The script side of a battle: opcodes 73 `Combat`, 80 `CombatBoss`,
//! and the three one-shot flags 82 `CombatMaxRound`, 83 `CombatMustFail`,
//! 131 `CombatNotGameOver` that precede them. 132 `GetCombat` reads what
//! these leave behind (see `get_combat.rs`).
//!
//! How the original sequences it (`0x437280` / `0x4373d0`): fill the
//! descriptor, start the combat system (`FUN_00407320`), then park the
//! script on a type-6 waiter until the combat system reports back through
//! `FUN_004055d0` case 3, which sets `DAT_005833d0 = (result == 2)`,
//! throws the game over if the party lost while `DAT_005833cc` was set,
//! and resets the three flags. This command is that whole span: it blocks
//! the script (returns `false`) while the [`CombatSession`] runs, draws a
//! placeholder overlay for it, and on exit writes the party's state, the
//! spoils and the `GetCombat` value back.
//!
//! 画面分两层：战场和人摆在 3D 舞台上（`build_stage`），界面按原版
//! `CBUI` 的版面贴原版贴图（`combat::hud`）。两层都只读规则层的状态。

use crate::openpal3::combat::actions::{ATTRIBUTE_SLOTS, GU_ITEM_ID, ITEM_STATE_CODES, ItemEffect};
use crate::openpal3::combat::encounter::{CrtRand, Encounter, NO_ROUND_CAP, PendingFlags};
use crate::openpal3::combat::hud::{self, ButtonLook, Gauge, RingButton};
use crate::openpal3::combat::motion::{self, ActKind, MotionSet, Play, Snapshot};
use crate::openpal3::combat::session::Pending;
use crate::openpal3::combat::session::{
    CombatSession, Input, ItemChoice, Menu, MenuEntry, Profile, Reward, SkillChoice,
};
use crate::openpal3::combat::state::{Battle, Combatant, Outcome, PARTY_SLOTS, Phase, SLOTS, Side};
use crate::openpal3::directors::SceneManagerExtensions;
use crate::openpal3::scene::RoleAnimationRepeatMode;
use crate::openpal3::scene::RoleController;
use crate::openpal3::states::game_db::{GameDb, combat_id_of_party_member};
use crate::openpal3::states::persistent_state::pearl_charge_index;
use crate::openpal3::states::{attributes, skills};
use crate::scripting::sce::{SceCommand, SceState};
use crosscom::ComRc;
use fileformats::pal3::gdb::GameItemInfo;
use imgui::{Condition, Ui};
use radiance::comdef::{IEntityExt, IScene, ISceneExt, ISceneManager};
use radiance::imgui::{GameFontSize, game_font};
use radiance::input::Key;
use radiance::math::Vec3;
use radiance::rendering::Sprite;
use std::path::PathBuf;
use std::rc::Rc;

use super::appraise::{overlay, play_sfx};
use super::encamp::draw_sprite;

/// `SceState::ext` key for the [`PendingFlags`] the flag opcodes set.
pub const EXT_PENDING: &str = "pal3.combat.pending";
/// `SceState::ext` key for the last battle's `GetCombat` value
/// (`DAT_005833d0`), a `bool`.
pub const EXT_RESULT: &str = "pal3.combat.result";

/// `FUN_00407320`'s fallback when the party stands outside the nav map.
const FLOOR_TYPE_OUT_OF_RANGE: i32 = 2;

fn pending_flags_mut(state: &mut SceState) -> &mut PendingFlags {
    state
        .ext_mut()
        .entry(EXT_PENDING.to_string())
        .or_insert_with(|| Box::new(PendingFlags::default()))
        .downcast_mut::<PendingFlags>()
        .expect("pal3.combat.pending holds PendingFlags")
}

/// Take the flags for the battle about to start and leave the defaults
/// behind — the original resets them only *after* the battle, but nothing
/// reads them in between, so taking them now is equivalent.
fn take_pending_flags(state: &mut SceState) -> PendingFlags {
    state
        .ext_mut()
        .remove(EXT_PENDING)
        .and_then(|b| b.downcast::<PendingFlags>().ok())
        .map(|b| *b)
        .unwrap_or_default()
}

/// What the last battle left for `GetCombat`: `None` if there was none.
pub fn last_result(state: &mut SceState) -> Option<bool> {
    state
        .ext_mut()
        .get(EXT_RESULT)
        .and_then(|b| b.downcast_ref::<bool>())
        .copied()
}

/// Opcode 82 — `DAT_00c1171c = n` (0x437f20 area).
#[derive(Debug, Clone)]
pub struct SceCommandCombatMaxRound {
    max_round: i32,
}

impl SceCommandCombatMaxRound {
    pub fn new(max_round: i32) -> Self {
        Self { max_round }
    }
}

impl SceCommand for SceCommandCombatMaxRound {
    fn update(&mut self, _: ComRc<ISceneManager>, _: &Ui, state: &mut SceState, _: f32) -> bool {
        pending_flags_mut(state).max_round = self.max_round;
        true
    }
}

/// Opcode 83 — `DAT_00c11748 = 1`.
#[derive(Debug, Clone)]
pub struct SceCommandCombatMustFail;

impl SceCommandCombatMustFail {
    pub fn new() -> Self {
        Self
    }
}

impl SceCommand for SceCommandCombatMustFail {
    fn update(&mut self, _: ComRc<ISceneManager>, _: &Ui, state: &mut SceState, _: f32) -> bool {
        pending_flags_mut(state).must_fail = true;
        true
    }
}

/// Opcode 131 — `DAT_005833cc = 0` (0x438440), the only writer of 0.
#[derive(Debug, Clone)]
pub struct SceCommandCombatNotGameOver;

impl SceCommandCombatNotGameOver {
    pub fn new() -> Self {
        Self
    }
}

impl SceCommand for SceCommandCombatNotGameOver {
    fn update(&mut self, _: ComRc<ISceneManager>, _: &Ui, state: &mut SceState, _: f32) -> bool {
        pending_flags_mut(state).game_over_on_loss = false;
        true
    }
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    /// Opcode 73.
    Normal { count: i32, ids: [u32; 3] },
    /// Opcode 80.
    Boss { ids: [u32; 6] },
}

/// Opcodes 73 and 80.
#[derive(Clone)]
pub struct SceCommandCombat {
    kind: Kind,
    session: Option<CombatSession>,
    /// Party actor id per battle slot 0..5.
    /// 在场的我方成员，**存的是队伍下标，而下标就是战斗位号**
    /// （见 `game_db::PLAYER_COMBAT_IDS` 的注释和 `combat_flow.md` 第 9.7
    /// 节）。所以拿它当槽位用，不要再枚举一次。
    party_actor_ids: Vec<u32>,
    /// Whether we switched the BGM to the battle theme and owe the scene
    /// its own back.
    switched_bgm: bool,
    game_over_on_loss: bool,
    /// The 游戏结束 screen is up, waiting for a confirm.
    game_over: bool,
    /// 这一场用哪张战场（`z**`），由 `combatScn.txt` 按脚下地面类型选出。
    combat_scene: Option<String>,
    /// 摆好的战斗舞台。压在场景栈上，散场时弹掉。
    stage: Option<ComRc<IScene>>,
    /// 界面贴图，开战时一次读齐。
    hud: Option<HudTextures>,
    /// 舞台上每个战斗位的人（`SLOTS` 个，没人的是 `None`）。
    actors: Vec<Option<StageActor>>,
    /// 正在演的那一次行动（`CombatSession::staging`）。
    run: Option<StageRun>,
    /// 战斗镜头（`combat::camera`）：开战播一个序列，播完停在最后的机位。
    camera: Option<crate::openpal3::combat::camera::CamPlayer>,
    /// `flexcam.dat`，散场时还要播胜 / 败 / 逃的序列。
    flexcam: Option<crate::openpal3::combat::camera::Flexcam>,
    /// 分出胜负、正在播散场镜头。
    ending: Option<Outcome>,
    /// 结算画面的时间轴（`combat::result`）。
    result: Option<crate::openpal3::combat::result::ResultPlayer>,
    /// 结算时收集的升级 / 钱 / 物品消息。
    result_messages: Vec<crate::openpal3::combat::result::ResultMessage>,
    /// 结算面板上每个人的样子。
    result_panels: Vec<ResultPanel>,
    result_tex: Option<ResultTextures>,
    /// 散场一开始就结算过了（打赢 / 逃掉）。
    settled: bool,
}

/// 结算面板上的一个人。
#[derive(Clone)]
struct ResultPanel {
    /// 我方战斗位，也是 `result\face\<位>.tga` 的编号。
    slot: usize,
    /// 「名字 等级 级」（`%s %d %s`，`FUN_005184e0`）。
    label: String,
    /// 精 / 气 / 神：(当前, 上限)。
    gauges: [(i32, i32); 3],
}

/// 结算画面的贴图（坐标在 `combat::result`）。
#[derive(Clone)]
struct ResultTextures {
    big_win: Tex,
    big_lose: Tex,
    back: Tex,
    faces: Vec<Tex>,
    slot: Tex,
    fills: [Tex; 3],
    label: Tex,
    levelup: Tex,
    getitem: Tex,
}

impl ResultTextures {
    fn load(state: &SceState) -> Self {
        use crate::openpal3::combat::result as res;
        let asset_mgr = state.asset_mgr();
        let tex = |rel: &str| {
            asset_mgr.load_sprite(
                PathBuf::from("/basedata/basedata")
                    .join(hud::UI_DIR)
                    .join(rel),
            )
        };
        Self {
            big_win: tex(res::BIG_WIN),
            big_lose: tex(res::BIG_LOSE),
            back: tex(res::PANEL_BACK),
            faces: (0..PARTY_SLOTS)
                .map(|s| tex(&res::face_texture(s)))
                .collect(),
            slot: tex(res::GAUGE_SLOT),
            fills: res::GAUGE_FILLS.map(tex),
            label: tex(res::GAUGE_LABEL),
            levelup: tex(res::LEVELUP_PANEL),
            getitem: tex(res::GETITEM_PANEL),
        }
    }
}

/// 一次行动的演出：攻击是走过去 → 出手 → 结算 → 走回来，其余原地出手。
#[derive(Clone, Copy)]
struct StageRun {
    slot: usize,
    kind: ActKind,
    step: StageStep,
}

#[derive(Clone, Copy, PartialEq)]
enum StageStep {
    Begin,
    WalkOut,
    /// 出手动作开始了 `t` 秒。
    Strike {
        t: f32,
    },
    WalkBack,
}

/// 舞台上的一个人，和上一帧看到的状态。
#[derive(Clone)]
struct StageActor {
    entity: ComRc<radiance::comdef::IEntity>,
    set: MotionSet,
    last: Snapshot,
    /// 怪物倒下后过一会儿收起来（怪物没有倒地动作）。
    hide_in: Option<f32>,
    /// 站位（`[RolePos]`），走位去了还要回来。
    home: Vec3,
    walk: Option<Walk>,
}

/// 走位中。
#[derive(Clone, Copy)]
enum Walk {
    /// 去目标跟前，到了面朝 `face`。
    To { dest: Vec3, face: Vec3 },
    /// 回站位。
    Home,
}

fn face_point(pos: Vec3, dir: (f32, f32)) -> Vec3 {
    Vec3::new(pos.x + dir.0 * 100., pos.y, pos.z + dir.1 * 100.)
}

/// 怪物倒下后多久收起来 **[推断]**：原版怪物目录里没有倒地动作，这里等
/// 挨打那一下做完。
const MONSTER_HIDE_SECONDS: f32 = 0.6;

type Tex = Option<Rc<Sprite>>;

/// 战斗界面要用的贴图（清单和坐标在 `combat::hud`）。
#[derive(Clone)]
struct HudTextures {
    control_back: Tex,
    /// 按 `RingButton::ALL` 的顺序，每个按钮四张（`ButtonLook` 0..3）。
    buttons: Vec<[Tex; 4]>,
    /// `[StatePanel0, StatePanel]`
    panels: [Tex; 2],
    gauge_back: Tex,
    /// 精、气、神。
    gauge_fill: [Tex; 3],
    sword_bar: Tex,
    portraits: Vec<Tex>,
    names: Vec<Tex>,
    sequence_back: Tex,
    /// 每个战斗位的行动条头像。
    heads: Vec<Tex>,
    magic_back: Tex,
    item_back: Tex,
    row_select: Tex,
    trick_back: Tex,
    trick_back2: Tex,
}

impl HudTextures {
    /// `models` 是每个战斗位的模型名，行动条头像按它找图。
    fn load(state: &SceState, models: &[Option<String>]) -> Self {
        let asset_mgr = state.asset_mgr();
        let tex = |rel: &str| {
            asset_mgr.load_sprite(
                PathBuf::from("/basedata/basedata")
                    .join(hud::UI_DIR)
                    .join(rel),
            )
        };
        let looks = [
            ButtonLook::Normal,
            ButtonLook::Focused,
            ButtonLook::Pressed,
            ButtonLook::Disabled,
        ];
        Self {
            control_back: tex(hud::CONTROL_BACK),
            buttons: RingButton::ALL
                .iter()
                .map(|&b| {
                    looks.map(|look| {
                        (look != ButtonLook::Disabled || b.has_disabled_look())
                            .then(|| tex(&hud::button_texture(b, look)))
                            .flatten()
                    })
                })
                .collect(),
            panels: [tex(hud::panel_texture(0)), tex(hud::panel_texture(1))],
            gauge_back: tex(hud::GAUGE_BACK),
            gauge_fill: Gauge::ALL.map(|g| tex(g.fill_texture())),
            sword_bar: tex(hud::SWORD_BAR),
            portraits: (0..PARTY_SLOTS)
                .map(|slot| hud::portrait_texture(slot).and_then(|p| tex(&p)))
                .collect(),
            names: (0..PARTY_SLOTS)
                .map(|slot| hud::name_texture(slot).and_then(|p| tex(&p)))
                .collect(),
            sequence_back: tex(hud::SEQUENCE_BACK),
            heads: models
                .iter()
                .enumerate()
                .map(|(slot, m)| {
                    // 我方的头像名和舞台模型名不全一样（龙葵 106 / 105）。
                    let name = hud::PARTY_HEADS.get(slot).copied().or(m.as_deref());
                    name.and_then(|m| tex(&hud::head_texture(m)))
                })
                .collect(),
            magic_back: tex(hud::MAGIC_BACK),
            item_back: tex(hud::ITEM_BACK),
            row_select: tex(hud::ROW_SELECT),
            trick_back: tex(hud::TRICK_BACK),
            trick_back2: tex(hud::TRICK_BACK2),
        }
    }
}

/// 列表文字的颜色 **[推断]**：表头是印在底图上的深色字，行文字跟着用深色。
const LIST_TEXT: [f32; 4] = [0.08, 0.08, 0.08, 1.0];
const LIST_TEXT_DISABLED: [f32; 4] = [0.42, 0.42, 0.42, 1.0];
const BANNER_TEXT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

impl std::fmt::Debug for SceCommandCombat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SceCommandCombat")
            .field("kind", &self.kind)
            .field("game_over", &self.game_over)
            .finish()
    }
}

impl SceCommandCombat {
    pub fn new(count: i32, m1: i32, m2: i32, m3: i32) -> Self {
        Self::with_kind(Kind::Normal {
            count,
            ids: [m1 as u32, m2 as u32, m3 as u32],
        })
    }

    pub fn new_boss(m1: i32, m2: i32, m3: i32, m4: i32, m5: i32, m6: i32) -> Self {
        Self::with_kind(Kind::Boss {
            ids: [m1, m2, m3, m4, m5, m6].map(|m| m as u32),
        })
    }

    fn with_kind(kind: Kind) -> Self {
        Self {
            kind,
            session: None,
            party_actor_ids: Vec::new(),
            switched_bgm: false,
            game_over_on_loss: true,
            game_over: false,
            combat_scene: None,
            stage: None,
            hud: None,
            actors: Vec::new(),
            run: None,
            camera: None,
            flexcam: None,
            ending: None,
            result: None,
            result_messages: Vec::new(),
            result_panels: Vec::new(),
            result_tex: None,
            settled: false,
        }
    }

    /// 每个战斗位的模型名（我方查 `PARTY_MODELS`，敌方查数据库）。
    fn slot_models(&self, state: &SceState) -> Vec<Option<String>> {
        use crate::openpal3::combat::stage;
        let db = state.global_state().game_db().cloned();
        let battle = self.session.as_ref().map(|s| &s.battle);
        (0..SLOTS)
            .map(|slot| {
                let c = battle?.get(slot)?;
                let enemy_model = db
                    .as_ref()
                    .and_then(|db| db.combat_actor(c.actor_id))
                    .and_then(|a| a.model_id.as_str().ok());
                stage::model_name(slot, enemy_model.as_deref())
            })
            .collect()
    }

    /// The floor type under the controlled character — `FUN_00407320`
    /// reads byte `+5` of the nav tile, i.e. the high byte of what the
    /// loader stores as `distance_to_border`.
    fn floor_type(scene_manager: &ComRc<ISceneManager>, state: &SceState) -> i32 {
        let Some(role) = scene_manager.get_resolved_role(state, -1) else {
            return FLOOR_TYPE_OUT_OF_RANGE;
        };
        let Some(scn) = scene_manager.scn_scene() else {
            return FLOOR_TYPE_OUT_OF_RANGE;
        };
        let Some(controller) = RoleController::get_role_controller(role.clone()) else {
            return FLOOR_TYPE_OUT_OF_RANGE;
        };
        let layer = controller.inner::<RoleController>().nav_layer();
        let position = role.transform().borrow().position();
        let s = scn.inner::<crate::openpal3::scene::ScnScene>();
        let (nx, nz) = s.scene_coord_to_nav_coord(layer, &position);
        // `__ftol`: truncation, not rounding.
        s.nav()
            .get(layer, nx.trunc() as i32, nz.trunc() as i32)
            .map(|tile| ((tile.distance_to_border >> 8) & 0xff) as i32)
            .unwrap_or(FLOOR_TYPE_OUT_OF_RANGE)
    }

    fn skill_choice(db: &GameDb, id: u32) -> Option<SkillChoice> {
        let s = db.skill(id)?;
        Some(SkillChoice {
            name: s
                .name
                .as_str()
                .map(|n| n.to_string())
                .unwrap_or_else(|_| format!("仙术#{id}")),
            effect: skills::effect_of(s),
        })
    }

    /// 战斗道具列表要不要收这一行（`FUN_0040d010` 的三个分支合起来）。
    ///
    /// 原版的道具指令下有三个子页，各调一次筛选：
    ///
    /// * case 0xe（治疗）：type 1 且「码[24]==2（复活）或前三项属性的影响
    ///   类型是 2（回满）或值 > 0」；
    /// * case 2（投掷）：type 2；
    /// * case 0xf（解状态）：type 1 且存在 k∈0..25、k∉{23,24}，码[k]∈{1,2}。
    ///
    /// 移植的菜单只有一层，所以三支并起来去重、按 id 升序。
    fn shows_in_combat(item: &GameItemInfo) -> bool {
        match item.item_type {
            2 => true,
            1 => {
                let heals = item.combat_state_impact_value[24] == 2
                    || (0..3).any(|k| {
                        item.attribute_impact_type[k] == 2 || item.attribute_impact_value[k] > 0
                    });
                let cures = (0..26)
                    .filter(|k| *k != 23 && *k != 24)
                    .any(|k| matches!(item.combat_state_impact_value[k], 1 | 2));
                heals || cures
            }
            _ => false,
        }
    }

    fn item_effect(item: &GameItemInfo) -> ItemEffect {
        let mut impacts = [(0u8, 0i16); ATTRIBUTE_SLOTS];
        for (i, slot) in impacts.iter_mut().enumerate() {
            *slot = (
                item.attribute_impact_type[i],
                item.attribute_impact_value[i],
            );
        }
        let mut state_codes = [0i16; ITEM_STATE_CODES];
        state_codes.copy_from_slice(&item.combat_state_impact_value);
        ItemEffect {
            id: item.id,
            special: item.item_special_type,
            target_range: item.target_range_type,
            impacts,
            state_codes,
        }
    }

    /// Build the battle and the session from the party and the database.
    /// 把战场和参战的人摆出来，压到场景栈上。
    ///
    /// 战场模型是 `cbScene/<Z**>/<z**>.pol`，站位是 `combat.ini` 的
    /// `[RolePos]`（我方 0..4、敌方 5..10），模型名见
    /// `combat::stage::model_name`。缺哪一件就少摆哪一个，不影响战斗本身
    /// ——规则层不碰这些实体。
    fn build_stage(&mut self, scene_manager: &ComRc<ISceneManager>, state: &mut SceState) {
        use crate::openpal3::combat::stage;
        use common::store_ext::StoreExt2;
        use mini_fs::StoreExt;
        use radiance::scene::{CoreScene, SceneLighting};

        let Some(scene_name) = self.combat_scene.clone() else {
            log::warn!("Combat: 没算出战场名，舞台不建");
            return;
        };
        log::info!("Combat: 战场 {scene_name}");
        let Some(db) = state.global_state().game_db().cloned() else {
            return;
        };
        let asset_mgr = state.asset_mgr();
        let vfs = asset_mgr.vfs();
        let factory = asset_mgr.component_factory();

        let model_path = stage::scene_model_path(&scene_name);
        if vfs.open(&model_path).is_err() {
            log::warn!("Combat: 战场模型不在包里 {}", model_path.display());
            return;
        }

        let scene = CoreScene::create();
        scene.set_lighting(SceneLighting::new([1., 1., 1.], vec![]));
        scene.add_entity(crate::openpal3::loaders::pol::create_entity_from_pol_model(
            &factory,
            vfs,
            &model_path,
            format!("COMBAT_STAGE_{scene_name}"),
            true,
        ));

        // 参战的人。站位表短了或者模型缺了就跳过那一个。
        let positions = &db.combat().role_positions;
        let Some(battle) = self.session.as_ref().map(|s| &s.battle) else {
            log::warn!("Combat: session 还没建，舞台上没人");
            return;
        };
        for slot in 0..stage::STAGE_SLOTS {
            let Some(c) = battle.get(slot) else { continue };
            if !c.alive {
                continue;
            }
            let enemy_model = db
                .combat_actor(c.actor_id)
                .and_then(|a| a.model_id.as_str().ok());
            let Some(model) = stage::model_name(slot, enemy_model.as_deref()) else {
                continue;
            };
            let Some(pos) = stage::stage_position(positions, slot) else {
                continue;
            };
            // 我方那套还是怪物那套，见 `combat::motion`（+0x243）。
            let set = if slot < PARTY_SLOTS {
                MotionSet::Party
            } else {
                MotionSet::Monster
            };
            let idle = set.name(motion::Motion::Idle).unwrap_or("z1");
            let Some(entity) =
                asset_mgr.load_role(&model, idle, format!("COMBAT_ROLE_{slot}"), true)
            else {
                log::warn!("Combat: 战斗模型 {model} 加载不了（位 {slot}）");
                continue;
            };
            let home = Vec3::new(pos.x, 0., pos.z);
            entity
                .transform()
                .borrow_mut()
                .set_position(&home)
                .look_at(&face_point(home, motion::rest_facing(set)));
            if let Some(rc) = RoleController::get_role_controller(entity.clone()) {
                let rc = rc.inner::<RoleController>();
                rc.set_active(true);
                rc.play_anim(idle, RoleAnimationRepeatMode::Loop);
            }
            scene.add_entity(entity.clone());
            if self.actors.len() < SLOTS {
                self.actors.resize(SLOTS, None);
            }
            self.actors[slot] = Some(StageActor {
                entity,
                set,
                last: Snapshot::of(c, None),
                hide_in: None,
                home,
                walk: None,
            });
        }

        // 相机：开战播 flexcam 的序列（combat::camera），先摆到第一帧。
        {
            use crate::openpal3::combat::camera::{self, CamPlayer, Flexcam};
            let editor = asset_mgr
                .vfs()
                .read_to_end_from_gbk(stage::COMBAT_CAMERA_INI)
                .ok()
                .map(|t| stage::parse_cameras(&t))
                .unwrap_or_default();
            let stand = editor.first().copied().unwrap_or_default();
            self.flexcam = asset_mgr
                .vfs()
                .read_to_end(camera::FLEXCAM_PATH)
                .ok()
                .and_then(|b| Flexcam::parse(&b));
            let player = self
                .flexcam
                .as_ref()
                .and_then(|f| CamPlayer::new(f.sequence(camera::STAND_SEQUENCE), stand));
            if player.is_none() {
                log::warn!("Combat: flexcam.dat 读不出来，镜头停在 EditorCam0");
            }
            let pose = player.as_ref().map(|p| p.pose).unwrap_or(stand);
            scene
                .camera_mut()
                .set_fov43(stage::COMBAT_FOV_DEGREES.to_radians());
            Self::place_camera(&scene, pose);
            self.camera = player;
        }

        let entities = scene.entities().len();
        scene_manager.push_scene(scene.clone());
        let top_entities = scene_manager
            .scene()
            .map(|s| s.entities().len())
            .unwrap_or(0);
        log::info!("Combat: 舞台压栈，{entities} 个实体；栈顶场景有 {top_entities} 个实体");
        self.stage = Some(scene);
    }

    /// 把舞台相机摆到 `pose`（位置 + 朝 `forward()` 看；roll 数据里全是 0，不管）。
    fn place_camera(scene: &ComRc<IScene>, pose: crate::openpal3::combat::stage::CombatCamera) {
        let (fx, fy, fz) = pose.forward();
        scene
            .camera_mut()
            .transform_mut()
            .set_position(&Vec3::new(pose.x, pose.y, pose.z))
            .look_at(&Vec3::new(
                pose.x + fx * 100.,
                pose.y + fy * 100.,
                pose.z + fz * 100.,
            ));
    }

    /// 分出胜负：播胜 / 败 / 逃的镜头序列（`FUN_004f7910`），打赢了我方活着的人做
    /// 胜利动作。起不来（没舞台、没 flexcam、序列空）返回 `false`，直接散场。
    fn begin_ending(&mut self, state: &mut SceState, outcome: Outcome) -> bool {
        use crate::openpal3::combat::camera::{CamPlayer, ending_sequence};
        use crate::openpal3::combat::result::{ResultPlayer, timeline};
        if self.stage.is_none() {
            return false;
        }
        let Some(seq) = ending_sequence(outcome) else {
            return false;
        };
        let current = self.camera.as_ref().map(|p| p.pose).unwrap_or_default();
        if let Some(player) = self
            .flexcam
            .as_ref()
            .and_then(|f| CamPlayer::new(f.sequence(seq), current))
        {
            log::info!("Combat: 散场镜头 {seq} 号（{outcome:?}）");
            self.camera = Some(player);
        }
        self.ending = Some(outcome);
        if outcome == Outcome::Win {
            // 每人排的任务 0x18 → 胜利动作 z20（`FUN_0050fa20`）；做完停住 [推断]。
            let alive: Vec<bool> = (0..self.actors.len())
                .map(|slot| {
                    self.session
                        .as_ref()
                        .and_then(|s| s.battle.get(slot))
                        .is_some_and(|c| c.alive)
                })
                .collect();
            for (slot, actor) in self.actors.iter().enumerate().take(PARTY_SLOTS) {
                let Some(actor) = actor.as_ref() else {
                    continue;
                };
                if !alive[slot] {
                    continue;
                }
                if let (Some(rc), Some(name)) = (
                    RoleController::get_role_controller(actor.entity.clone()),
                    actor.set.name(motion::Motion::Win),
                ) {
                    let rc = rc.inner::<RoleController>();
                    rc.set_auto_play_idle(false);
                    rc.play_anim(name, RoleAnimationRepeatMode::Repeat(1));
                }
            }
        }
        // 打赢 / 逃掉：先结算，收集结算画面要的消息（败的等散场后再写回）。
        if matches!(outcome, Outcome::Win | Outcome::Escaped) {
            self.settled = true;
            self.finish(state, outcome);
        }
        let steps = timeline(outcome, std::mem::take(&mut self.result_messages));
        self.result = Some(ResultPlayer::new(steps));
        self.result_tex = Some(ResultTextures::load(state));
        true
    }

    /// 把舞台相机摆到 `pose` 的位置，朝 `at` 看（盯目标的镜头记录）。
    fn place_camera_looking_at(
        scene: &ComRc<IScene>,
        pose: crate::openpal3::combat::stage::CombatCamera,
        at: Vec3,
    ) {
        scene
            .camera_mut()
            .transform_mut()
            .set_position(&Vec3::new(pose.x, pose.y, pose.z))
            .look_at(&at);
    }

    /// 散场：把舞台弹掉。
    fn drop_stage(&mut self, scene_manager: &ComRc<ISceneManager>) {
        self.actors.clear();
        self.run = None;
        self.camera = None;
        self.flexcam = None;
        self.ending = None;
        self.result = None;
        self.result_tex = None;
        self.result_panels.clear();
        if self.stage.take().is_some() {
            scene_manager.pop_scene();
        }
    }

    fn start(&mut self, scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        let flags = take_pending_flags(state);
        self.game_over_on_loss = flags.game_over_on_loss;

        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() ^ (d.as_secs() as u32))
            .unwrap_or(0x1234_5678);
        let encounter = match self.kind {
            Kind::Normal { count, ids } => {
                Encounter::scripted(&mut CrtRand::new(seed), count, ids, flags)
            }
            Kind::Boss { ids } => Encounter::boss(ids, flags),
        };

        let floor_type = Self::floor_type(&scene_manager, state);

        let Some(db) = state.global_state().game_db().cloned() else {
            log::error!("Combat: no game database; skipping the battle");
            return;
        };

        let mut battle = Battle::new(seed.wrapping_mul(0x9e37_79b9));
        if encounter.flags.max_round != NO_ROUND_CAP {
            battle.max_round = encounter.flags.max_round;
        }
        battle.must_fail = encounter.flags.must_fail;
        battle.escape_forbidden = encounter.escape_forbidden;

        let mut profiles: Vec<Profile> = (0..SLOTS).map(|_| Profile::default()).collect();
        let mut items: Vec<ItemChoice> = Vec::new();

        {
            let mut persistent = state.global_state_mut().persistent_state_mut();

            let (scene, block) = (
                persistent.scene_name().unwrap_or_default(),
                persistent.sub_scene_name().unwrap_or_default(),
            );
            let combat_scene = db
                .combat()
                .combat_scene(&scene, &block, floor_type)
                .map(|s| s.to_string());
            battle.scene_element = combat_scene
                .as_deref()
                .and_then(|name| db.combat().scene_element(name));
            self.combat_scene = combat_scene;

            // 花钱的技能和蛊术要看队伍身上的钱和背包里的蛊，散场时再把花掉
            // 的写回去（`FUN_004f3430` 改的是全队共用的钱，蛊是物品 6113）。
            battle.money = persistent.money();
            battle.gu = persistent.item_count(GU_ITEM_ID);

            // ---- party ----
            let team: Vec<u32> = persistent
                .team()
                .iter()
                .copied()
                .take(PARTY_SLOTS)
                .collect();
            self.party_actor_ids = team.clone();
            // 战斗位是**按角色定的，不是按入队顺序**。原版
            // `FUN_004eea70` 里那张写死的表把战斗 actor id 映到位号
            // （0x65→0, 0x68→1, 0x69/0x6a→2, 0x6b→3, 0x66→4），而
            // `PLAYER_COMBAT_IDS` 的排法正好让**队伍下标 == 位号**。
            // 先前这里用的是 `team` 的枚举下标，队长换人整队站位就跟着
            // 移位了。
            for actor_id in team.iter().copied() {
                let slot = actor_id as usize;
                if slot >= PARTY_SLOTS {
                    log::warn!("Combat: 队伍里有下标 {slot}，超出五个战斗位");
                    continue;
                }
                let Some(role) = persistent.role(actor_id) else {
                    continue;
                };
                let Some(combat_id) = combat_id_of_party_member(actor_id) else {
                    continue;
                };
                let eff = attributes::effective(role, &db);

                let mut c = Combatant::new(combat_id, Side::Party);
                c.level = role.level;
                c.max_hp = eff.max_hp;
                c.hp = role.hp.min(eff.max_hp);
                c.max_sp = eff.max_sp;
                c.sp = role.sp.min(eff.max_sp);
                c.max_mp = eff.max_mp;
                c.mp = role.mp.min(eff.max_mp);
                c.attack = eff.attack;
                c.defense = eff.defense;
                c.speed = eff.speed;
                c.luck = eff.luck;
                for (i, v) in eff.elements.iter().enumerate() {
                    c.elements[i + 1] = *v > 0;
                }
                // 五灵阵位：原版 `FUN_004ed250` 从冒险侧记录的 `+0x29a`
                // 搬进战斗位的 `+0x470`，那就是阵法页存的位。
                c.element_position = persistent.lineup_position(actor_id).unwrap_or(0);
                if c.hp <= 0 {
                    c.alive = false;
                }
                battle.place(slot, c);

                profiles[slot] = Profile {
                    name: db
                        .combat_actor(combat_id)
                        .and_then(|a| a.name.as_str().ok())
                        .unwrap_or_else(|| format!("#{combat_id}")),
                    skills: role
                        .skills
                        .iter()
                        .filter_map(|s| Self::skill_choice(&db, s.id))
                        .collect(),
                    reward: Reward::default(),
                };
            }

            // ---- enemies ----
            for (i, id) in encounter.monsters.iter().copied().enumerate() {
                if id == 0 {
                    continue;
                }
                let Some(row) = db.combat_actor(id) else {
                    log::warn!("Combat: monster {id} is not in the database");
                    continue;
                };
                let slot = PARTY_SLOTS + i;
                let mut c = Combatant::new(id, Side::Enemy);
                c.level = row.level;
                c.max_hp = row.attribute_values[0];
                c.hp = c.max_hp;
                c.max_sp = row.attribute_values[1];
                c.sp = c.max_sp;
                c.max_mp = row.attribute_values[2];
                c.mp = c.max_mp;
                c.attack = row.attribute_values[3];
                c.defense = row.attribute_values[4];
                c.speed = row.attribute_values[5];
                c.luck = row.attribute_values[6];
                for (i, v) in row.element_attribute_values.iter().enumerate() {
                    c.elements[i + 1] = *v > 0;
                }
                battle.place(slot, c);

                profiles[slot] = Profile {
                    name: row.name.as_str().unwrap_or_else(|_| format!("#{id}")),
                    skills: row.skill_ids[..3]
                        .iter()
                        .copied()
                        .filter(|s| *s != 0)
                        .filter_map(|s| Self::skill_choice(&db, s))
                        .collect(),
                    reward: Reward {
                        experience: row.experience,
                        money: row.money_when_killed as i32,
                        loot: (row.normal_loot != 0 && row.normal_loot_count > 0)
                            .then_some((row.normal_loot, row.normal_loot_count as i32)),
                    },
                };
                persistent.mark_monster_seen(id);
            }

            // ---- 道具 ----
            // 背包是按 id 升序的 BTreeMap，正好就是原版列表的次序。五灵珠
            // 用的是充能不是数量（`FUN_004f7140`）。
            items = persistent
                .inventory()
                .iter()
                .filter_map(|(id, count)| {
                    let row = db.item(*id)?;
                    if !Self::shows_in_combat(row) {
                        return None;
                    }
                    let pearl = pearl_charge_index(*id).is_some();
                    let count = if pearl {
                        persistent.pearl_charge(*id) as u32
                    } else {
                        *count
                    };
                    Some(ItemChoice {
                        name: row
                            .name
                            .as_str()
                            .map(|n| n.to_string())
                            .unwrap_or_else(|_| format!("道具#{id}")),
                        count,
                        pearl,
                        effect: Self::item_effect(row),
                    })
                })
                .collect();
        }

        if !encounter.keep_bgm {
            self.switched_bgm = state.global_state_mut().play_combat_bgm();
        }

        {
            let ours: Vec<String> = (0..PARTY_SLOTS)
                .filter_map(|i| battle.get(i))
                .map(|c| format!("{}({}/{})", c.actor_id, c.hp, c.max_hp))
                .collect();
            let theirs: Vec<String> = (PARTY_SLOTS..SLOTS)
                .filter_map(|i| battle.get(i))
                .map(|c| format!("{}({}/{})", c.actor_id, c.hp, c.max_hp))
                .collect();
            log::info!("Combat: 我方 {ours:?} 敌方 {theirs:?}");
        }
        let mut session = CombatSession::new(battle, profiles, seed.rotate_left(13) ^ 0x5bd1_e995);
        session.set_items(items);
        self.session = Some(session);
    }

    /// `FUN_004055d0` case 3: write the result and the party's state
    /// back, pay out, restore the music. Returns true when the game is
    /// over.
    fn finish(&mut self, state: &mut SceState, outcome: Outcome) -> bool {
        let Some(session) = self.session.as_ref() else {
            return false;
        };
        let won = outcome.counts_as_win();
        let db = state.global_state().game_db().cloned();
        let mut raw: Option<(Vec<(usize, i32)>, i32, Vec<(u32, i32)>)> = None;

        {
            let mut persistent = state.global_state_mut().persistent_state_mut();
            for actor_id in self.party_actor_ids.iter().copied() {
                // 队伍下标就是战斗位号。
                let Some(c) = session.battle.get(actor_id as usize) else {
                    continue;
                };
                if let Some(role) = persistent.role_mut(actor_id) {
                    // 战后写回：`FUN_004ed870`（0x4ed870，由胜负判定
                    // 0x4f7dc1 / 0x4f7f97 在战斗结束时调）逐个我方战斗位：
                    //
                    // ```text
                    // if (在场 +0x241) {
                    //     if (精 +0x24c < 1) { 精 = 1; FUN_004f34e0(slot, -2); }
                    //     冒险记录 +0xe8 = 精;
                    // }
                    // ```
                    //
                    // 所以**倒下的人确实回到 1 精**（外加 `+0x274`「情」扣 2，
                    // 见 `FUN_004f34e0`；情不写回冒险记录，所以这里不管）。
                    //
                    // 而且它**只写回精那一个 dword**——`mov eax,[esi];
                    // mov [edi],eax`，气和神没有写回，战后会退回进战斗前的值。
                    // 进战斗时是 `FUN_004ed250` 把冒险记录整块拷进战斗位的，
                    // 两边是两张表（冒险跨度 0x580、战斗 0x37c）。
                    role.hp = c.hp.max(1);
                }
            }

            // 花掉的钱和用掉的蛊无论胜负都算数——它们是行动开始时就扣的
            // （`FUN_004f2910`），不是结算的一部分。
            if session.battle.spent_money > 0 {
                persistent.add_money(-session.battle.spent_money);
            }
            if session.battle.spent_gu > 0 {
                persistent.remove_item(GU_ITEM_ID, session.battle.spent_gu);
            }

            // 战斗里用掉的道具：五灵珠扣一格充能（物品本身不消耗），其余扣
            // 一个。执行器 0x51514e–0x51519d 就是这么分的。
            for id in session.used_items.iter().copied() {
                if pearl_charge_index(id).is_some() {
                    persistent.spend_pearl_charge(id);
                } else {
                    persistent.remove_item(id, 1);
                }
            }

            if won {
                let spoils = session.spoils();
                let mut level_ups: Vec<(usize, i32)> = Vec::new();
                if let Some(db) = db.as_ref() {
                    for (slot, amount) in spoils.experience {
                        if let Some(actor_id) = self
                            .party_actor_ids
                            .iter()
                            .copied()
                            .find(|a| *a as usize == slot)
                        {
                            if persistent.award_experience(actor_id, db, amount) > 0 {
                                let level = persistent.role(actor_id).map(|r| r.level).unwrap_or(0);
                                level_ups.push((slot, level));
                            }
                        }
                    }
                    for (slot, skill_id) in session.casts.iter().copied() {
                        let (Some(actor_id), Some(row)) = (
                            self.party_actor_ids
                                .iter()
                                .copied()
                                .find(|a| *a as usize == slot),
                            db.skill(skill_id),
                        ) else {
                            continue;
                        };
                        persistent.use_skill(actor_id, skill_id, &row.times_before_level_up);
                    }
                }
                persistent.add_money(spoils.money);
                let loot = spoils.loot.clone();
                for (item, count) in spoils.loot {
                    if count > 0 {
                        persistent.add_item(item, count as u32);
                    }
                }
                raw = Some((level_ups, spoils.money, loot));
            }
        }

        // 结算画面要的消息和面板（combat::result）。
        if let Some((level_ups, money, loot)) = raw {
            use crate::openpal3::combat::result::{
                LANG_PATH, MessageKind, ResultLang, ResultMessage,
            };
            use common::store_ext::StoreExt2;
            let lang = state
                .asset_mgr()
                .vfs()
                .read_to_end_from_gbk(LANG_PATH)
                .ok()
                .map(|t| ResultLang::parse(&t))
                .unwrap_or_else(|| ResultLang::parse(""));
            let mut msgs = Vec::new();
            for (slot, level) in level_ups {
                msgs.push(ResultMessage {
                    kind: MessageKind::LevelUp,
                    text: lang.level_up(session.name(slot), level),
                });
            }
            if money > 0 {
                msgs.push(ResultMessage {
                    kind: MessageKind::Money,
                    text: lang.money(money),
                });
            }
            for (item, count) in loot {
                if count <= 0 {
                    continue;
                }
                let name = db
                    .as_ref()
                    .and_then(|d| d.item(item))
                    .and_then(|r| r.name.as_str().ok())
                    .unwrap_or_else(|| format!("#{item}"));
                msgs.push(ResultMessage {
                    kind: MessageKind::Item,
                    text: lang.item(&name, count),
                });
            }
            let persistent = state.global_state().persistent_state();
            let panels: Vec<ResultPanel> = (0..PARTY_SLOTS)
                .filter_map(|slot| {
                    let c = session.battle.get(slot)?;
                    let level = persistent
                        .role(slot as u32)
                        .map(|r| r.level)
                        .unwrap_or(c.level);
                    Some(ResultPanel {
                        slot,
                        label: format!("{} {} {}", session.name(slot), level, lang.level),
                        gauges: [(c.hp.max(0), c.max_hp), (c.sp, c.max_sp), (c.mp, c.max_mp)],
                    })
                })
                .collect();
            drop(persistent);
            self.result_messages = msgs;
            self.result_panels = panels;
        }

        state
            .ext_mut()
            .insert(EXT_RESULT.to_string(), Box::new(won));

        // 散场一开始就结算的，音乐等结算画面走完再换回去。
        if !self.settled {
            self.restore_bgm(state);
        }

        outcome == Outcome::Lose && self.game_over_on_loss
    }

    fn restore_bgm(&mut self, state: &mut SceState) {
        if self.switched_bgm {
            self.switched_bgm = false;
            let g = state.global_state_mut();
            g.bgm_source().stop();
            g.play_default_bgm();
        }
    }

    fn poll_input(state: &SceState) -> Option<Input> {
        let input = state.input();
        let pressed = |k: Key| input.get_key_state(k).pressed();
        if pressed(Key::Up) || pressed(Key::GamePadDPadUp) {
            Some(Input::Up)
        } else if pressed(Key::Down) || pressed(Key::GamePadDPadDown) {
            Some(Input::Down)
        } else if pressed(Key::Left) || pressed(Key::GamePadDPadLeft) {
            Some(Input::Left)
        } else if pressed(Key::Right) || pressed(Key::GamePadDPadRight) {
            Some(Input::Right)
        } else if pressed(Key::Space) || pressed(Key::GamePadEast) {
            Some(Input::Confirm)
        } else if pressed(Key::Escape) || pressed(Key::GamePadSouth) {
            Some(Input::Cancel)
        } else {
            None
        }
    }

    /// 演交给舞台的那一次行动（`CombatSession::staging`）：攻击走过去 →
    /// 出手 → 隔 `HIT_DELAY` 结算 → 走回来；仙术、道具原地出手。演完放开时钟。
    fn drive_staging(&mut self, delta_sec: f32) {
        use motion::Motion;
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let Some(staging) = session.staging else {
            self.run = None;
            return;
        };
        let slot = staging.slot;
        // 新的一次：从登记的行动里读种类和目标。
        if self.run.map(|r| r.slot) != Some(slot) {
            let kind = match session.pending {
                Some(Pending::Attack { .. }) => ActKind::Attack,
                Some(Pending::Cast { .. }) => ActKind::Cast,
                Some(Pending::Item { target, .. }) => ActKind::Item {
                    hostile: (target < PARTY_SLOTS) != (slot < PARTY_SLOTS),
                },
                _ => {
                    session.release();
                    return;
                }
            };
            self.run = Some(StageRun {
                slot,
                kind,
                step: StageStep::Begin,
            });
        }
        let target = match session.pending {
            Some(Pending::Attack { target, .. })
            | Some(Pending::Cast { target, .. })
            | Some(Pending::Item { target, .. }) => Some(target),
            _ => None,
        };
        let target_home = target
            .and_then(|t| self.actors.get(t))
            .and_then(|a| a.as_ref())
            .map(|a| a.home);
        let Some(run) = self.run.as_mut() else { return };
        let Some(actor) = self.actors.get_mut(slot).and_then(|a| a.as_mut()) else {
            // 台上没这个人：直接结算、放开。
            session.release();
            self.run = None;
            return;
        };
        let Some(rc) = RoleController::get_role_controller(actor.entity.clone()) else {
            session.release();
            self.run = None;
            return;
        };
        let rc = rc.inner::<RoleController>();
        let set = actor.set;
        let play = |m: Motion, mode: RoleAnimationRepeatMode| {
            set.name(m).is_some_and(|name| rc.play_anim(name, mode))
        };
        let start_strike = |run: &mut StageRun| {
            rc.set_auto_play_idle(true);
            if let Some(m) = motion::strike_motion(run.kind) {
                play(m, RoleAnimationRepeatMode::Repeat(1));
            }
            run.step = StageStep::Strike { t: 0. };
        };

        match run.step {
            StageStep::Begin => match (run.kind, target_home) {
                (ActKind::Attack, Some(to)) => {
                    let me = actor.home;
                    let (x, z) = motion::approach_point((to.x, to.z), (me.x, me.z));
                    actor.walk = Some(Walk::To {
                        dest: Vec3::new(x, me.y, z),
                        face: to,
                    });
                    rc.set_auto_play_idle(true);
                    play(Motion::Move, RoleAnimationRepeatMode::Loop);
                    run.step = StageStep::WalkOut;
                }
                _ => start_strike(run),
            },
            StageStep::WalkOut => {
                if actor.walk.is_none() {
                    start_strike(run);
                }
            }
            StageStep::Strike { t } => {
                let t = t + delta_sec;
                run.step = StageStep::Strike { t };
                if t >= motion::HIT_DELAY && !staging.landed {
                    session.strike();
                }
                let done = t >= motion::HIT_DELAY
                    && rc.state() != crate::openpal3::scene::RoleState::PlayingAnimation;
                if done {
                    let at = actor.entity.transform().borrow().position();
                    let (dx, dz) = (at.x - actor.home.x, at.z - actor.home.z);
                    let away = (dx * dx + dz * dz).sqrt() > motion::ARRIVE_DISTANCE;
                    let alive = session.battle.get(slot).is_some_and(|c| c.alive);
                    if away && alive {
                        actor.walk = Some(Walk::Home);
                        play(Motion::Move, RoleAnimationRepeatMode::Loop);
                        run.step = StageStep::WalkBack;
                    } else {
                        session.release();
                        self.run = None;
                    }
                }
            }
            StageStep::WalkBack => {
                if actor.walk.is_none() {
                    session.release();
                    self.run = None;
                }
            }
        }
    }

    /// 让舞台上的人跟着战况做动作（`combat::motion`）。
    fn animate_stage(&mut self, delta_sec: f32) {
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let pending = session.pending;
        let homes: Vec<Option<Vec3>> = self
            .actors
            .iter()
            .map(|a| a.as_ref().map(|a| a.home))
            .collect();
        for slot in 0..self.actors.len() {
            let Some(actor) = self.actors[slot].as_mut() else {
                continue;
            };
            let Some(c) = session.battle.get(slot) else {
                continue;
            };
            let acting = pending.filter(|p| p.slot() == slot).map(|p| match p {
                Pending::Attack { .. } => ActKind::Attack,
                Pending::Cast { .. } => ActKind::Cast,
                Pending::Item { target, .. } => ActKind::Item {
                    hostile: (target < PARTY_SLOTS) != (slot < PARTY_SLOTS),
                },
                Pending::Escape { .. } => ActKind::Escape,
            });
            let now = Snapshot::of(c, acting);
            let Some(rc) = RoleController::get_role_controller(actor.entity.clone()) else {
                continue;
            };
            let rc = rc.inner::<RoleController>();

            if let Some(t) = actor.hide_in.as_mut() {
                *t -= delta_sec;
                if *t <= 0. {
                    actor.entity.set_visible(false);
                    actor.hide_in = None;
                }
            }

            let set = actor.set;
            let play = |m: motion::Motion, mode: RoleAnimationRepeatMode| {
                set.name(m).is_some_and(|name| rc.play_anim(name, mode))
            };

            // 走一帧。
            if let Some(walk) = actor.walk {
                let (dest, face_at_end) = match walk {
                    Walk::To { dest, face } => (dest, face),
                    Walk::Home => {
                        let (fx, fz) = motion::rest_facing(set);
                        (actor.home, face_point(actor.home, (fx, fz)))
                    }
                };
                let pos = actor.entity.transform().borrow().position();
                let ((x, z), arrived) =
                    motion::step_toward((pos.x, pos.z), (dest.x, dest.z), delta_sec);
                let next = Vec3::new(x, pos.y, z);
                let look = if arrived { face_at_end } else { dest };
                let transform = actor.entity.transform();
                transform
                    .borrow_mut()
                    .set_position(&next)
                    .look_at(&Vec3::new(look.x, next.y, look.z));
                if arrived {
                    actor.walk = None;
                    if now.alive {
                        rc.idle();
                    }
                }
            }

            match motion::react(&actor.last, &now) {
                Some(Play::Once(m)) => {
                    rc.set_auto_play_idle(true);
                    play(m, RoleAnimationRepeatMode::Repeat(1));
                }
                Some(Play::Loop(m)) => {
                    rc.set_auto_play_idle(true);
                    play(m, RoleAnimationRepeatMode::Loop);
                }
                Some(Play::Stay(m)) => {
                    rc.set_auto_play_idle(false);
                    if !play(m, RoleAnimationRepeatMode::Repeat(1)) {
                        // 怪物没有倒地动作：挨一下，然后收起来。
                        play(
                            motion::Motion::BeAttacked,
                            RoleAnimationRepeatMode::Repeat(1),
                        );
                        actor.hide_in = Some(MONSTER_HIDE_SECONDS);
                    }
                }
                Some(Play::Rest) => {
                    rc.set_auto_play_idle(true);
                    actor.entity.set_visible(true);
                    actor.hide_in = None;
                    let rest = motion::rest_motion(&now);
                    if rest == motion::Motion::Idle || !play(rest, RoleAnimationRepeatMode::Loop) {
                        rc.idle();
                    }
                }
                None => {
                    // 一次性的动作做完、自己回到了待机，而此刻该防着或睡着。
                    let rest = motion::rest_motion(&now);
                    if now.alive
                        && rest != motion::Motion::Idle
                        && rc.state() == crate::openpal3::scene::RoleState::Idle
                    {
                        play(rest, RoleAnimationRepeatMode::Loop);
                    }
                }
            }
            actor.last = now;
        }
    }

    /// 按原版 `CBUI` 的版面画界面，坐标见 `combat::hud`。
    fn draw(ui: &Ui, session: &CombatSession, tex: &HudTextures) {
        overlay(ui, "CombatHud", |ui, at, scale| {
            let list = ui.get_window_draw_list();
            let battle = &session.battle;

            // 右上角的行动条：底，加每个人的头像按条走（`FUN_004dbb20`）。
            draw_sprite(
                &list,
                tex.sequence_back.as_deref(),
                hud::SEQUENCE_BACK_RECT,
                at,
            );
            for slot in 0..SLOTS {
                let Some(c) = battle.get(slot) else { continue };
                if !c.alive {
                    continue;
                }
                let x = hud::head_x(c.gauge, c.cast_gauge, c.phase == Phase::Executing);
                let head = tex.heads.get(slot).and_then(|t| t.as_deref());
                draw_sprite(&list, head, hud::head_rect(x), at);
            }

            // 底下一排状态面板。原版按角色记录 `+0x18` 那个字节选面板
            // （`FUN_00518b30`）；这里按战斗位顺序依次占 **[推断]**。
            let party: Vec<usize> = (0..PARTY_SLOTS)
                .filter(|&slot| battle.get(slot).is_some())
                .take(hud::PANELS)
                .collect();
            for (panel, &slot) in party.iter().enumerate() {
                let Some(c) = battle.get(slot) else { continue };
                let back = tex.panels[(panel != 0) as usize].as_deref();
                draw_sprite(&list, back, hud::panel_rect(panel), at);
                if panel == 0 {
                    draw_sprite(&list, tex.sword_bar.as_deref(), hud::sword_rect(), at);
                }
                for (k, gauge) in Gauge::ALL.iter().enumerate() {
                    let bar = gauge.rect(panel);
                    draw_sprite(&list, tex.gauge_back.as_deref(), bar, at);
                    let (value, max) = match gauge {
                        Gauge::Hp => (c.hp, c.max_hp),
                        Gauge::Gp => (c.sp, c.max_sp),
                        Gauge::Mp => (c.mp, c.max_mp),
                    };
                    if let Some(fill) =
                        hud::fill_rect(bar, hud::GAUGE_PADDING, value as f32, max as f32)
                    {
                        draw_sprite(&list, tex.gauge_fill[k].as_deref(), fill, at);
                    }
                }
            }

            let Some(turn) = session.turn else {
                if let Some((notice, _)) = session.notice.as_ref() {
                    Self::draw_banner(ui, &list, tex.trick_back2.as_deref(), notice, at, scale);
                }
                return;
            };

            // 左上角：正在选行动的人的头像和竖排名字。
            if turn.slot < PARTY_SLOTS {
                let portrait = tex.portraits.get(turn.slot).and_then(|t| t.as_deref());
                draw_sprite(&list, portrait, hud::PORTRAIT_RECT, at);
                let name = tex.names.get(turn.slot).and_then(|t| t.as_deref());
                draw_sprite(&list, name, hud::NAME_RECT, at);
            }

            match turn.menu {
                Menu::Main { index } => {
                    let panel = hud::CONTROL_PANEL;
                    let back = hud::offset(hud::CONTROL_BACK_RECT, panel);
                    draw_sprite(&list, tex.control_back.as_deref(), back, at);
                    let focus = RingButton::of_entry(MenuEntry::ALL[index]);
                    for (i, &button) in RingButton::ALL.iter().enumerate() {
                        if !button.shown_at_start() {
                            continue;
                        }
                        let look = if button.entry().is_none() {
                            ButtonLook::Disabled
                        } else if button == focus {
                            ButtonLook::Focused
                        } else {
                            ButtonLook::Normal
                        };
                        let sprite = tex.buttons[i][look as usize].as_deref();
                        draw_sprite(&list, sprite, hud::offset(button.rect(), panel), at);
                    }
                }
                Menu::Skills { index } => {
                    let rows: Vec<(Vec<String>, bool)> = session
                        .skills(turn.slot)
                        .iter()
                        .enumerate()
                        .map(|(i, s)| {
                            let cells = vec![s.name.clone(), s.effect.mp_cost.to_string()];
                            (cells, session.can_afford(turn.slot, i))
                        })
                        .collect();
                    Self::draw_list(
                        ui,
                        &list,
                        (tex.magic_back.as_deref(), tex.row_select.as_deref()),
                        hud::MAGIC_PANEL,
                        &hud::MAGIC_COLUMNS,
                        &rows,
                        index,
                        at,
                        scale,
                    );
                }
                Menu::Items { index } => {
                    let rows: Vec<(Vec<String>, bool)> = session
                        .items()
                        .iter()
                        .enumerate()
                        .map(|(i, it)| {
                            let cells = vec![it.name.clone(), it.count.to_string()];
                            (cells, session.item_available(i))
                        })
                        .collect();
                    Self::draw_list(
                        ui,
                        &list,
                        (tex.item_back.as_deref(), tex.row_select.as_deref()),
                        hud::ITEM_PANEL,
                        &hud::ITEM_COLUMNS,
                        &rows,
                        index,
                        at,
                        scale,
                    );
                }
                Menu::Target { index, .. } => {
                    // 原版在目标头顶放一个 3D 指针（`ui\combat\pointer\pointer.cvd`），
                    // CVD 还没移植；先把目标的名字写在招式名横幅上。
                    let targets = session.targets();
                    if let Some(&target) = targets.get(index) {
                        let name = session.name(target).to_string();
                        Self::draw_banner(ui, &list, tex.trick_back.as_deref(), &name, at, scale);
                    }
                }
            }

            if let Some((notice, _)) = session.notice.as_ref() {
                Self::draw_banner(ui, &list, tex.trick_back2.as_deref(), notice, at, scale);
            }
        });
    }

    /// 结算画面：大字、人物面板、消息横幅、渐黑（坐标见 `combat::result`）。
    fn draw_result(
        ui: &Ui,
        player: &crate::openpal3::combat::result::ResultPlayer,
        panels: &[ResultPanel],
        tex: Option<&ResultTextures>,
    ) {
        use crate::openpal3::combat::result::{self as res, MessageKind};
        let Some(tex) = tex else { return };
        overlay(ui, "CombatResult", |ui, at, scale| {
            let list = ui.get_window_draw_list();
            if let Some(since) = player.panels_elapsed() {
                let _font = game_font(GameFontSize::SMALL).map(|f| ui.push_font(f));
                for (k, p) in panels.iter().enumerate().take(res::PANELS) {
                    let dy = res::panel_slide_y(k, since);
                    let base = res::panel_rect(k);
                    let panel = (base.0, base.1 + dy, base.2, base.3 + dy);
                    draw_sprite(&list, tex.back.as_deref(), panel, at);
                    let face = tex.faces.get(p.slot).and_then(|t| t.as_deref());
                    draw_sprite(&list, face, hud::offset(res::FACE_RECT, panel), at);
                    let label = hud::offset(res::GAUGE_LABEL_RECT, panel);
                    draw_sprite(&list, tex.label.as_deref(), label, at);
                    for (i, (value, max)) in p.gauges.iter().enumerate() {
                        let bar = hud::offset(res::GAUGE_RECTS[i], panel);
                        draw_sprite(&list, tex.slot.as_deref(), bar, at);
                        if let Some(fill) =
                            hud::fill_rect(bar, res::GAUGE_PADDING, *value as f32, *max as f32)
                        {
                            draw_sprite(&list, tex.fills[i].as_deref(), fill, at);
                        }
                    }
                    let r = hud::offset(res::NAME_RECT, panel);
                    list.add_text(at(r.0 + 8, r.1 + 8), BANNER_TEXT, &p.label);
                }
            }
            if let Some(win) = player.banner() {
                let big = if win { &tex.big_win } else { &tex.big_lose };
                draw_sprite(&list, big.as_deref(), res::BIG_RECT, at);
            }
            if let Some(m) = player.message() {
                let back = match m.kind {
                    MessageKind::LevelUp => tex.levelup.as_deref(),
                    _ => tex.getitem.as_deref(),
                };
                let r = m.panel().1;
                draw_sprite(&list, back, r, at);
                let _font = game_font(GameFontSize::LARGE).map(|f| ui.push_font(f));
                let [w, h] = ui.calc_text_size(&m.text);
                let [x0, y0] = at(r.0, r.1);
                let (bw, bh) = ((r.2 - r.0) as f32 * scale, (r.3 - r.1) as f32 * scale);
                list.add_text(
                    [x0 + (bw - w) * 0.5, y0 + (bh - h) * 0.5],
                    BANNER_TEXT,
                    &m.text,
                );
            }
            if let Some(alpha) = player.fade() {
                let [sw, sh] = ui.io().display_size;
                list.add_rect([0., 0.], [sw, sh], [0., 0., 0., alpha])
                    .filled(true)
                    .build();
            }
        });
    }

    /// 仙术 / 道具列表：底图、选中行、各列文字。
    #[allow(clippy::too_many_arguments)]
    fn draw_list(
        ui: &Ui,
        list: &imgui::DrawListMut<'_>,
        (back, select): (Option<&Sprite>, Option<&Sprite>),
        panel: hud::Rect,
        columns: &[i32],
        rows: &[(Vec<String>, bool)],
        index: usize,
        at: &dyn Fn(i32, i32) -> [f32; 2],
        scale: f32,
    ) {
        draw_sprite(list, back, hud::offset(hud::LIST_BACK_RECT, panel), at);
        let area = hud::offset(hud::LIST_ROWS, panel);
        let first = hud::first_visible(index, rows.len());
        let _font = game_font(GameFontSize::SMALL).map(|f| ui.push_font(f));
        let font_px = ui.current_font_size();
        for (line, (cells, enabled)) in rows.iter().enumerate().skip(first).take(hud::VISIBLE_ROWS)
        {
            let y = area.1 + (line - first) as i32 * hud::ROW_HEIGHT;
            if line == index {
                draw_sprite(list, select, (area.0, y, area.2, y + hud::ROW_HEIGHT), at);
            }
            let color = if *enabled {
                LIST_TEXT
            } else {
                LIST_TEXT_DISABLED
            };
            let mut x = area.0;
            for (cell, width) in cells.iter().zip(columns) {
                let [sx, sy] = at(x + 2, y);
                let dy = (hud::ROW_HEIGHT as f32 * scale - font_px) * 0.5;
                list.add_text([sx, sy + dy], color, cell);
                x += width;
            }
        }
    }

    /// 招式名横幅，字居中。
    fn draw_banner(
        ui: &Ui,
        list: &imgui::DrawListMut<'_>,
        back: Option<&Sprite>,
        text: &str,
        at: &dyn Fn(i32, i32) -> [f32; 2],
        scale: f32,
    ) {
        let r = hud::TRICK_NAME_RECT;
        draw_sprite(list, back, r, at);
        let _font = game_font(GameFontSize::LARGE).map(|f| ui.push_font(f));
        let [w, h] = ui.calc_text_size(text);
        let [x0, y0] = at(r.0, r.1);
        let (bw, bh) = ((r.2 - r.0) as f32 * scale, (r.3 - r.1) as f32 * scale);
        list.add_text(
            [x0 + (bw - w) * 0.5, y0 + (bh - h) * 0.5],
            BANNER_TEXT,
            text,
        );
    }

    fn draw_game_over(ui: &Ui) {
        let [window_width, window_height] = ui.io().display_size;
        ui.window("GameOver")
            .collapsible(false)
            .title_bar(false)
            .resizable(false)
            .always_auto_resize(true)
            .position_pivot([0.5, 0.5])
            .position([window_width / 2., window_height / 2.], Condition::Always)
            .build(|| {
                let _font = radiance::imgui::game_font(radiance::imgui::GameFontSize::LARGE)
                    .map(|f| ui.push_font(f));
                ui.text("游戏结束");
                ui.text("按确定退出");
            });
    }
}

impl SceCommand for SceCommandCombat {
    fn initialize(&mut self, scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        self.start(scene_manager.clone(), state);
        // 战场和参战的人。规则层不碰这些实体，摆不出来也照打。
        self.build_stage(&scene_manager, state);
        if self.session.is_some() {
            let models = self.slot_models(state);
            self.hud = Some(HudTextures::load(state, &models));
        }
    }

    fn update(
        &mut self,
        scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if self.game_over {
            Self::draw_game_over(ui);
            // The original switches the game's state object to the
            // title screen; the same request 147 Switch2Menu makes.
            if Self::poll_input(state) == Some(Input::Confirm) || state.fast_forward() {
                log::info!("Combat: 游戏结束，返回主菜单");
                state.request_title();
                return true;
            }
            return false;
        }

        let Some(session) = self.session.as_mut() else {
            // No database or nothing to fight: report a win so the script
            // moves on.
            state
                .ext_mut()
                .insert(EXT_RESULT.to_string(), Box::new(true));
            return true;
        };

        // 有舞台、没在快进，才把出手交给舞台演。
        session.stage_driven = !state.fast_forward() && !self.actors.is_empty();
        if !session.stage_driven && session.staging.is_some() {
            session.release();
        }

        // Fast-forward drives the whole battle in one frame, the AI taking
        // the party's turns; a real frame takes one step.
        let outcome = if state.fast_forward() {
            let mut outcome = None;
            for _ in 0..100_000 {
                session.auto_play();
                outcome = session.tick(0.1);
                if outcome.is_some() {
                    break;
                }
            }
            outcome.or(Some(Outcome::Win))
        } else {
            if let Some(input) = Self::poll_input(state) {
                session.input(input);
            }
            session.tick(delta_sec)
        };

        self.drive_staging(delta_sec);
        self.animate_stage(delta_sec);
        if let (Some(player), Some(stage)) = (self.camera.as_mut(), self.stage.as_ref()) {
            let moving = !player.finished();
            let pose = if moving {
                player.update(delta_sec)
            } else {
                player.pose
            };
            match player.look_target() {
                // 盯着人：朝他现在站的地方看（记录 +0x65 / +0x68）。
                Some(target) => {
                    let at = usize::try_from(target)
                        .ok()
                        .and_then(|t| self.actors.get(t))
                        .and_then(|a| a.as_ref())
                        .map(|a| a.entity.transform().borrow().position())
                        // -1 = 战场中心（CBStage +0x384..+0x38c），按原点算 [推断]。
                        .unwrap_or(Vec3::new(0., 0., 0.));
                    Self::place_camera_looking_at(stage, pose, at);
                }
                None if moving => Self::place_camera(stage, pose),
                None => {}
            }
        }
        if let Some(player) = self.result.as_ref() {
            // 结算期间战斗界面收起来（`FUN_00518b30(0)` / `FUN_004db940(0)`）。
            Self::draw_result(ui, player, &self.result_panels, self.result_tex.as_ref());
        } else if let (Some(session), Some(hud)) = (self.session.as_ref(), self.hud.as_ref()) {
            Self::draw(ui, session, hud);
        }

        match outcome {
            None => false,
            Some(outcome) => {
                // 先演散场：镜头 + 结算画面，时间轴走完再散（快进不演）。
                if !state.fast_forward() {
                    if self.ending.is_none() {
                        self.begin_ending(state, outcome);
                    }
                    if self.ending.is_some() {
                        let skip = Self::poll_input(state) == Some(Input::Confirm);
                        let running = match self.result.as_mut() {
                            Some(player) => {
                                if let Some(sound) =
                                    player.update(delta_sec, skip).and_then(|s| s.sound())
                                {
                                    play_sfx(state, sound);
                                }
                                !player.finished()
                            }
                            None => self.camera.as_ref().is_some_and(|p| !p.finished()),
                        };
                        if running {
                            return false;
                        }
                    }
                }
                log::info!("Combat: finished with {outcome:?}");
                self.drop_stage(&scene_manager);
                let settled = std::mem::take(&mut self.settled);
                if settled {
                    self.restore_bgm(state);
                }
                if !settled && self.finish(state, outcome) {
                    self.game_over = true;
                    false
                } else {
                    true
                }
            }
        }
    }
}
