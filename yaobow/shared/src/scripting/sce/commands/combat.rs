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
use crate::openpal3::combat::ai;
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
use imgui::Ui;
use radiance::comdef::{IEntityExt, IScene, ISceneExt, ISceneManager};
use radiance::imgui::{GameFontSize, game_font};
use radiance::input::Key;
use radiance::math::Vec3;
use radiance::rendering::Sprite;
use std::path::PathBuf;
use std::rc::Rc;

use super::appraise::{overlay, play_sfx};
use super::encamp::draw_sprite;
use super::ui_sound;

/// `SceState::ext` key for the [`PendingFlags`] the flag opcodes set.
pub const EXT_PENDING: &str = "pal3.combat.pending";
/// `SceState::ext` key for the last battle's `GetCombat` value
/// (`DAT_005833d0`), a `bool`.
pub const EXT_RESULT: &str = "pal3.combat.result";

/// `FUN_00407320`'s fallback when the party stands outside the nav map.
/// 青铜护镜（`FUN_004edf10` 查饰品两格是不是它）。
const BRONZE_MIRROR: u32 = 0x1fae;
/// 「千金散去」那一套五件（`cbrolestate.cpp:1358`）：绿幞头、通神、元宝鞋、
/// 右利、左利。后两件戴在饰品两格，左右不论。
const LOST_MONEY_HAT: u32 = 80015;
const LOST_MONEY_CLOTH: u32 = 84018;
const LOST_MONEY_SHOES: u32 = 83012;
const LOST_MONEY_RIGHT: u32 = 8153;
const LOST_MONEY_LEFT: u32 = 8154;
const FLOOR_TYPE_OUT_OF_RANGE: i32 = 2;
/// 新怪出场的特效组（`CreateMonster` 收尾处放的 0x141）。
const SPAWN_EFFECT_GROUP: i32 = 0x141;

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
    /// 野外遇敌（`FUN_00417420` → `FUN_00407560`）：撞上的 `.scn` 角色 `role`、朝向判定 `sneak`。
    Field {
        count: i32,
        ids: [u32; 3],
        sneak: u8,
        role: u8,
    },
}

/// Opcodes 73 and 80.
#[derive(Clone)]
pub struct SceCommandCombat {
    kind: Kind,
    session: Option<CombatSession>,
    /// 偷袭成立了没有：（我方 `+0x218`，敌方 `+0x219`）。
    sneak: (bool, bool),
    /// 偷袭横幅的字（`lang.txt` 的 `WeSneakAttackEnemy` / `EnemySneakAttackUs`）。
    sneak_text: Option<String>,
    /// Party actor id per battle slot 0..5.
    /// 在场的我方成员，**存的是队伍下标，而下标就是战斗位号**
    /// （见 `game_db::PLAYER_COMBAT_IDS` 的注释和 `combat_flow.md` 第 9.7
    /// 节）。所以拿它当槽位用，不要再枚举一次。
    party_actor_ids: Vec<u32>,
    /// Whether we switched the BGM to the battle theme and owe the scene
    /// its own back.
    switched_bgm: bool,
    game_over_on_loss: bool,
    /// The 游戏结束 screen is up (see [`GameOverScreen`]).
    game_over: bool,
    game_over_screen: GameOverScreen,
    /// 这一场用哪张战场（`z**`），由 `combatScn.txt` 按脚下地面类型选出。
    combat_scene: Option<String>,
    /// 摆好的战斗舞台。压在场景栈上，散场时弹掉。
    stage: Option<ComRc<IScene>>,
    /// 选目标时目标头顶的 3D 指针（`ui\combat\pointer\pointer.cvd`，`FUN_00510540` 载进
    /// 舞台场景 `+0x268`），平时藏着。
    /// 原版只载一份、每帧对亮着的位各画一次；这里每个我方位一个实体。
    pointers: Vec<ComRc<radiance::comdef::IEntity>>,
    /// 五灵环（`CBStageScene`）：开场和阵型面板开关时淡入淡出。
    lineup_ring: Option<crate::openpal3::combat::lineup_ring::LineupRingFx>,
    /// 战斗时钟（战斗系统 `+0x50`）：开战 `FUN_00512580` 清零，每帧战斗更新
    /// `FUN_00513060` 一开头 += dt × 倍率 `+0x6c`。倍率默认 1.0（`DAT_00570054`），
    /// 只有 `time` 类特效（`FUN_004d53e0`，移植端没接）和调试对话框
    /// （`FUN_00515940`，停成 0）会改，所以这里就是开战以来的秒数。选目标指针的
    /// 上下跳和镜头震动都读它。
    battle_clock: f32,
    /// 界面贴图，开战时一次读齐。
    hud: Option<HudTextures>,
    /// 舞台上每个战斗位的人（`SLOTS` 个，没人的是 `None`）。
    actors: Vec<Option<StageActor>>,
    /// 正在演的那一次行动（`CombatSession::staging`）。
    run: Option<StageRun>,
    /// 合击技开演 / 收场时整屏压黑的程度，0..1（`0x502000` / `0x5020d0`）。
    stage_fade: f32,
    /// 战斗镜头（`combat::camera`）：开战播一个序列，播完停在最后的机位。
    camera: Option<crate::openpal3::combat::camera::CamPlayer>,
    /// `flexcam.dat`，散场时还要播胜 / 败 / 逃的序列。
    flexcam: Option<crate::openpal3::combat::camera::Flexcam>,
    /// `efflinker.dat`：技能 / 道具 → 特效组和结算时刻。
    linker: Option<crate::openpal3::combat::linker::EffectLinker>,
    /// 两方的定点：`[我方, 敌方]`（`FUN_00508930` 的 CBStage +0x384 / 10 号位）。
    side_anchors: [Vec3; 2],
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
    /// 特效 `action` 类的动作提示（`combat::stage_action`）。
    cues: Vec<crate::openpal3::combat::stage_action::ActionCue>,
    /// 特效 `quake` 类的镜头震动。
    shake: Option<crate::openpal3::combat::camera::Shake>,
    /// 特效 `bink` 类排着的 2D 动画，和正在播的那一段（源尺寸、贴图）。
    movies: Vec<String>,
    movie: Option<((u32, u32), Option<imgui::TextureId>)>,
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
    /// 仙术库六行各自的使用次数和（水 火 雷 风 土 双）。
    elements: [i32; 6],
}

/// 结算收集到的东西：升级（位, 新等级, 学会的特技名）、仙术升级（位, 仙术名, 新等级,
/// 五灵横幅的行）、钱、物品。
type Settled = (
    Vec<(usize, i32, Vec<String>)>,
    Vec<(usize, String, u8, Option<u8>)>,
    i32,
    Vec<(u32, i32)>,
);

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
    /// 五灵升级横幅，按 `result::ELEMENT_BANNERS` 的行。
    element_banners: [Tex; 6],
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
            element_banners: res::ELEMENT_BANNERS.map(tex),
        }
    }
}

/// 一次行动的演出：攻击是走过去 → 出手 → 结算 → 走回来，其余原地出手。
#[derive(Clone, Copy)]
struct StageRun {
    slot: usize,
    kind: ActKind,
    step: StageStep,
    /// 这一招在 efflinker 里的那一条（仙术、投掷道具才有）。
    link: Option<crate::openpal3::combat::linker::EffectLink>,
    target: Option<usize>,
    /// 第一 / 第二段特效放过没有。
    played: [bool; 2],
    /// 合击技（编排任务 0x514220）：前后多几步渐黑 / 摆位 / 渐亮和名字横幅。
    combo: bool,
    /// 普攻已经结算了几下（见 `motion::attack_events`）。
    hits_done: u32,
    /// 普攻已经处理到第几个标记事件。
    events_done: usize,
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
    /// 合击技：渐黑（1 秒），黑到底时把人摆到阵位上。
    ComboOpen {
        t: f32,
    },
    /// 渐亮（1 秒）。
    ComboShow {
        t: f32,
    },
    /// 招式横幅写合击技的名字（1 秒，`0x4fd880`）。
    ComboBanner {
        t: f32,
    },
    /// 演完：渐黑（1 秒），黑到底时摆回原位（`0x4ff740`）。
    ComboClose {
        t: f32,
    },
    /// 渐亮（1 秒），然后放开时钟。
    ComboReveal {
        t: f32,
    },
}

/// 合击技编排里渐黑 / 渐亮 / 横幅那几步各 1 秒（`FUN_004fc7d0` 的时长参数 1.0）。
const COMBO_STEP_SECONDS: f32 = 1.;

/// `FUN_00402b00`：按键表（`+0x1878` 起 0x10e 项）里有没有哪个键这一帧按下。移植端看常用的
/// 几个键、手柄四个面键和鼠标左键。
fn any_key_pressed(state: &SceState) -> bool {
    use radiance::input::{Key, MouseButton};
    let input = state.input();
    [
        Key::Space,
        Key::Escape,
        Key::GamePadEast,
        Key::GamePadSouth,
        Key::GamePadNorth,
        Key::GamePadWest,
    ]
    .iter()
    .any(|k| input.get_key_state(*k).pressed())
        || input.get_mouse_button_state(MouseButton::Left).pressed()
}

/// 舞台上的一个人，和上一帧看到的状态。
#[derive(Clone)]
struct StageActor {
    entity: ComRc<radiance::comdef::IEntity>,
    set: MotionSet,
    last: Snapshot,
    /// 死了的怪物等挨打动作播完再藏（见 `animate_stage`）。
    hide_on_finish: bool,
    /// 打赢时在做胜利动作 z20，做完回歇着的动作（见 `begin_ending`）。
    win_pose: bool,
    /// 合击把目标挪进盘心时，原先占着盘心的 10 号位被收起（`SetVisible(false)`，
    /// `CCBStage::ProcAllRoleToCoAttackPos`，0x4ff4c5 处的 `FUN_0050a1f0(0)`）。
    /// 收场时 `ProcAllRoleToInitPos` 再显出来。
    stowed: bool,
    /// 选目标时在闪（`+0x112`），染过色。
    pulsing: bool,
    /// 站位（`[RolePos]`），走位去了还要回来。
    home: Vec3,
    walk: Option<Walk>,
    /// 有几条特效 `action` 提示正在控制他（原版舞台角色 `+0x17c`）。大于 0 时常规的站姿 /
    /// 走位逻辑让开。
    scripted: u32,
    /// 刚被交还：下一帧回到该有的站姿。
    released: bool,
    /// 特效给的透明度（`gbVisNode::SetBlend`）。
    alpha: f32,
    /// 普攻的标记事件（结算次数、音效、特效），见 `motion::attack_events`。
    hit_events: Vec<motion::HitEvent>,
}

/// 走位中。
#[derive(Clone, Copy)]
enum Walk {
    /// 去目标跟前，到了面朝 `face`。
    To { dest: Vec3, face: Vec3 },
    /// 回站位。
    Home,
    /// 逃走：跑向 (800, 0, 800)（`LAB_005011b0` / `FUN_0050f590`），到了告诉战斗。
    Away { dest: Vec3 },
}

fn face_point(pos: Vec3, dir: (f32, f32)) -> Vec3 {
    Vec3::new(pos.x + dir.0 * 100., pos.y, pos.z + dir.1 * 100.)
}

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
    /// 按 `hud::STATE_ICONS`。
    state_icons: Vec<Tex>,
    /// `gbdata/number.tga`
    number: Tex,
    sequence_back: Tex,
    /// 每个战斗位的行动条头像。
    heads: Vec<Tex>,
    magic_back: Tex,
    item_back: Tex,
    skill_back: Tex,
    row_select: Tex,
    trick_back: Tex,
    trick_back2: Tex,
    ai_back: Tex,
    /// 按 `ai::Switch::ALL`，每格 [没勾, 勾上]。
    ai_switches: Vec<[Tex; 2]>,
    /// 确定：平时、指上去、按下。
    ai_ok: [Tex; 3],
    ai_names: Vec<Tex>,
    protect_back: Tex,
    protect_bar: Tex,
    protect_fill: Tex,
    protect_ball: Tex,
    /// 雪见、龙葵、紫萱，每格 [没勾, 勾上, 不可用]。
    protect_checks: Vec<[Tex; 3]>,
    protect_ok: [Tex; 3],
    lineup_back: Tex,
    /// 按我方战斗位，[平时, 暗色]。
    lineup_faces: Vec<[Tex; 2]>,
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
            state_icons: hud::STATE_ICONS
                .iter()
                .map(|(_, stem, _)| tex(&hud::state_icon_texture(stem)))
                .collect(),
            number: asset_mgr
                .load_sprite(PathBuf::from("/basedata/basedata").join(hud::NUMBER_FONT)),
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
            skill_back: tex(hud::SKILL_BACK),
            row_select: tex(hud::ROW_SELECT),
            trick_back: tex(hud::TRICK_BACK),
            trick_back2: tex(hud::TRICK_BACK2),
            ai_back: tex(hud::AI_BACK),
            ai_switches: ai::Switch::ALL
                .iter()
                .map(|s| [false, true].map(|on| tex(&hud::ai_switch_texture(s.stem(), on))))
                .collect(),
            ai_ok: [0u8, 1, 2].map(|look| tex(&hud::ai_ok_texture(look))),
            protect_back: tex(hud::PROTECT_BACK),
            protect_bar: tex(hud::PROTECT_BAR_BACK),
            protect_fill: tex(hud::PROTECT_BAR_FILL),
            protect_ball: tex(hud::PROTECT_BALL),
            protect_checks: (0..hud::PROTECT_NAMES.len())
                .map(|row| {
                    [0u8, 1, 3]
                        .map(|look| hud::protect_check_texture(row, look).and_then(|p| tex(&p)))
                })
                .collect(),
            protect_ok: [0u8, 1, 2].map(|look| tex(&hud::protect_ok_texture(look))),
            lineup_back: tex(hud::LINEUP_BACK),
            lineup_faces: (0..PARTY_SLOTS)
                .map(|slot| {
                    [false, true].map(|d| hud::lineup_face_texture(slot, d).and_then(|p| tex(&p)))
                })
                .collect(),
            ai_names: (0..PARTY_SLOTS)
                .map(|slot| hud::ai_name_texture(slot).and_then(|p| tex(&p)))
                .collect(),
        }
    }
}

/// 列表行文字的颜色（UIListBox 渲染 `0x43e989`）：能用的行取条目 `+0x44`，
/// 加条目时（`FUN_0043e5c0`）抄自列表控件 `+0x4`——控件基类构造 `FUN_00445aa0`
/// 设的 R 0、G 5、B 5，三个战斗列表都没改；不能用的行（`+0x4ac` 为 0）画
/// `DAT_00d91780`，和不可用按钮叠的是同一个颜色。
const LIST_TEXT: [f32; 4] = [0., 5. / 255., 5. / 255., 1.];
const LIST_TEXT_DISABLED: [f32; 4] = [
    0x3c as f32 / 255.,
    0x40 as f32 / 255.,
    0x40 as f32 / 255.,
    0xc8 as f32 / 255.,
];
const BANNER_TEXT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

/// ARGB → imgui 颜色。
fn argb(c: u32) -> [f32; 4] {
    let ch = |s: u32| ((c >> s) & 0xff) as f32 / 255.;
    [ch(16), ch(8), ch(0), ch(24)]
}

/// 带控件颜色（`FUN_00446020`）的贴图。
fn draw_sprite_tinted(
    list: &imgui::DrawListMut<'_>,
    sprite: Option<&Sprite>,
    r: hud::Rect,
    color: u32,
    at: &dyn Fn(i32, i32) -> [f32; 2],
) {
    let Some(s) = sprite else { return };
    let (uv0, uv1) = super::encamp::sprite_uv(s, r);
    list.add_image(s.imgui_texture_id(), at(r.0, r.1), at(r.2, r.3))
        .uv_min(uv0)
        .uv_max(uv1)
        .col(argb(color))
        .build();
}

/// 位图数字字（`hud::number_glyphs`），`(x, y)` 是左下角。
fn draw_number(
    list: &imgui::DrawListMut<'_>,
    font: Option<&Sprite>,
    text: &str,
    x: i32,
    y: i32,
    color: u32,
    at: &dyn Fn(i32, i32) -> [f32; 2],
) {
    let Some(font) = font else { return };
    let (o, e) = (at(0, 0), at(1000, 1000));
    let (sx, sy) = ((e[0] - o[0]) / 1000., (e[1] - o[1]) / 1000.);
    let p = |x: f32, y: f32| [o[0] + x * sx, o[1] + y * sy];
    for (q, uv) in hud::number_glyphs(text, x, y) {
        list.add_image(font.imgui_texture_id(), p(q[0], q[1]), p(q[2], q[3]))
            .uv_min([uv[0], uv[1]])
            .uv_max([uv[2], uv[3]])
            .col(argb(color))
            .build();
    }
}

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

    /// 野外遇敌，见 [`Kind::Field`]。
    pub fn new_field(count: i32, ids: [u32; 3], sneak: u8, role: u8) -> Self {
        Self::with_kind(Kind::Field {
            count,
            ids,
            sneak,
            role,
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
            sneak: (false, false),
            sneak_text: None,
            party_actor_ids: Vec::new(),
            switched_bgm: false,
            game_over_on_loss: true,
            game_over: false,
            game_over_screen: GameOverScreen::default(),
            combat_scene: None,
            stage: None,
            pointers: Vec::new(),
            lineup_ring: None,
            battle_clock: 0.,
            hud: None,
            actors: Vec::new(),
            run: None,
            stage_fade: 0.,
            camera: None,
            cues: Vec::new(),
            shake: None,
            movies: Vec::new(),
            movie: None,
            flexcam: None,
            linker: None,
            side_anchors: [Vec3::new(0., 0., 0.); 2],
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

    /// 特技是 `SkillType` 0（StandardMagic）那一类：数据里 26 条，每条只给一个主角、只耗气。
    fn is_special(db: &GameDb, id: u32) -> bool {
        db.skill(id).is_some_and(|s| s.skill_type == 0)
    }

    fn skill_choice(db: &GameDb, id: u32, multiplier: i32) -> Option<SkillChoice> {
        let s = db.skill(id)?;
        Some(SkillChoice {
            multiplier,
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
            kind: item.item_type,
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
    /// 把一只怪摆进 `slot`，照 `CCBRoleState::CreateMonster`（`FUN_004f8bb0`）
    /// 那一套从 gdb 搬属性，返回它的 [`Profile`]。开战时六个位各调一次，
    /// 战斗中分身（`Battle::pending_spawns`）也走这里。
    fn seat_monster(
        db: &GameDb,
        battle: &mut crate::openpal3::combat::state::Battle,
        slot: usize,
        id: u32,
    ) -> Option<Profile> {
        let Some(row) = db.combat_actor(id) else {
            log::warn!("Combat: monster {id} is not in the database");
            return None;
        };
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
        // 普攻方式、挨它一下我方加的气（`FUN_004ed250`：gdb +0x2ec → 战斗位 +0x280，
        // +0x30c → +0x284）。
        c.attack_mode = row.normal_attack_mode_id;
        c.sp_on_hit = row.sp_impact_value;
        // boss：模型名以 b/B 开头（`FUN_004ed250` 查 gdb `+0x206` 头一个字符）。
        c.boss = row
            .model_id
            .as_str()
            .map(|m| m.starts_with(['b', 'B']))
            .unwrap_or(false);
        let boss = c.boss;
        for (i, v) in row.element_attribute_values.iter().enumerate() {
            c.elements[i + 1] = *v > 0;
        }
        c.steal_money = row.stealable_money_amount as i32;
        // 逃走概率：`FUN_004ed250` 0x4ed595 把 gdb `+0x2d4` 取整写进 `+0x59c`。
        c.escape_rate = row.escape_rate as i32;
        c.steal_item = (row.stealable_item_id != 0 && row.stealable_item_count > 0)
            .then_some((row.stealable_item_id, row.stealable_item_count as i32));
        battle.place(slot, c);
        battle.apply_opening_states(
            slot,
            &row.combat_state_impact_types,
            boss || row.combat_state_impact_types[25] != 0,
        );
        battle.assign_enemy_pattern(slot);

        Some(Profile {
            name: row.name.as_str().unwrap_or_else(|_| format!("#{id}")),
            // 四招都进列表（怪物的挑选器都扫 `skill_ids[0..4]`）。出手倍率（`FUN_004f76c0`）：
            // 前三招是 gdb `skill_levels[i]`（不加 1），第四招（boss 那招）是 4，即瞬发。
            skills: row
                .skill_ids
                .iter()
                .enumerate()
                .filter(|(_, s)| **s != 0)
                .filter_map(|(i, s)| {
                    let multiplier = if i < 3 { row.skill_levels[i] as i32 } else { 4 };
                    Self::skill_choice(db, *s, multiplier)
                })
                .collect(),
            specials: Vec::new(),
            reward: Reward {
                experience: row.experience,
                money: row.money_when_killed as i32,
                loot: (row.normal_loot != 0 && row.normal_loot_count > 0)
                    .then_some((row.normal_loot, row.normal_loot_count as i32)),
            },
        })
    }

    /// 给一个战斗位造舞台角色：模型、站位、朝向、普攻的结算时刻。
    /// 开战时 [`Self::build_stage`] 逐位调，战斗中分身出来的怪也调它。
    /// 返回 `None` 就是这一位不上台（没人、倒着、站位表短了、模型缺了）。
    fn make_actor(&self, state: &SceState, db: &GameDb, slot: usize) -> Option<StageActor> {
        use crate::openpal3::combat::stage;
        use mini_fs::StoreExt;

        let asset_mgr = state.asset_mgr();
        let vfs = asset_mgr.vfs();
        let positions = &db.combat().role_positions;
        let battle = self.session.as_ref().map(|s| &s.battle)?;
        let c = battle.get(slot)?;
        if !c.alive {
            return None;
        }
        let enemy_model = db
            .combat_actor(c.actor_id)
            .and_then(|a| a.model_id.as_str().ok());
        let Some(model) = stage::model_name(slot, enemy_model.as_deref()) else {
            return None;
        };
        let Some(pos) = stage::stage_position(positions, slot) else {
            return None;
        };
        // 我方那套还是怪物那套，见 `combat::motion`（+0x243）。
        let set = if slot < PARTY_SLOTS {
            MotionSet::Party
        } else {
            MotionSet::Monster
        };
        let idle = set.name(motion::Motion::Idle).unwrap_or("z1");
        let Some(entity) = asset_mgr.load_role(&model, idle, format!("COMBAT_ROLE_{slot}"), true)
        else {
            log::warn!("Combat: 战斗模型 {model} 加载不了（位 {slot}）");
            return None;
        };
        let home = Vec3::new(pos.x, 0., pos.z);
        // 偷袭时被偷袭的一方背过身去（`FUN_005096d0`：敌方位看 `+0x218`、我方位看 `+0x219`，朝向 + 180°）。
        let (fx, fz) = motion::rest_facing(set);
        let turned = if slot < PARTY_SLOTS {
            self.sneak.1
        } else {
            self.sneak.0
        };
        let facing = if turned { (-fx, -fz) } else { (fx, fz) };
        entity
            .transform()
            .borrow_mut()
            .set_position(&home)
            .look_at(&face_point(home, facing));
        if let Some(rc) = RoleController::get_role_controller(entity.clone()) {
            let rc = rc.inner::<RoleController>();
            rc.set_active(true);
            // 选目标时要单独染色（见 `place_pointer`），同模型的怪不能共用材质。
            rc.set_unique_materials();
            rc.play_anim(idle, RoleAnimationRepeatMode::Loop);
        }
        // 普攻的结算时刻（`FUN_0050e550`）：攻击动作 MV3 里的 `work` / `work1` 标记。
        let hit_events = {
            let tags: Vec<(String, u32)> = set
                .name(motion::Motion::Attack)
                .map(|action| asset_mgr.mv3_path(&model, action))
                .and_then(|path| vfs.open(&path).ok())
                .and_then(|mut f| fileformats::mv3::read_mv3(&mut f).ok())
                .map(|m| {
                    m.action_desc
                        .iter()
                        .filter_map(|a| a.name.as_str().ok().map(|n| (n.to_string(), a.tick)))
                        .collect()
                })
                .unwrap_or_default();
            let at = |name: &str| {
                tags.iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, t)| *t as f32 / 4580.)
            };
            let monster_value = db
                .combat_actor(c.actor_id)
                .map(|a| a.properties[0] as i32)
                .unwrap_or(0);
            let mut events =
                motion::attack_events(slot, at("work"), at("work1"), c.weapon_type, c.weapon_hits);
            for e in &mut events {
                if let Some(tag) = e.tag {
                    e.sound =
                        motion::hit_sound(slot, tag, c.weapon_type, c.magic_sword, monster_value);
                    if slot >= PARTY_SLOTS {
                        e.effect = motion::hit_effect(monster_value);
                    }
                }
            }
            events
        };
        log::info!("Combat: 位 {slot}（{model}）普攻标记 {hit_events:?}");
        Some(StageActor {
            entity,
            set,
            hit_events,
            last: Snapshot::of(c, None),
            hide_on_finish: false,
            win_pose: false,
            stowed: false,
            pulsing: false,
            home,
            walk: None,
            scripted: 0,
            released: false,
            alpha: 1.,
        })
    }

    /// 战斗中分身 / 召唤出来的怪（`CCBRoleState::CreateMonster`，`FUN_004f8bb0`）：
    /// `Battle` 查不到 gdb，只把「哪一位、哪只怪」排进 `pending_spawns`，这里
    /// 每帧领走，照开战那一套填战斗位、补名片，再给它造个舞台角色。
    ///
    /// 原版造完还放 0x141 号特效当出场演出，移植端照放。
    fn spawn_pending(&mut self, state: &mut SceState) {
        let pending: Vec<(usize, u32)> = match self.session.as_mut() {
            Some(s) if !s.battle.pending_spawns.is_empty() => {
                std::mem::take(&mut s.battle.pending_spawns)
            }
            _ => return,
        };
        let Some(db) = state.global_state().game_db().cloned() else {
            return;
        };
        for (slot, id) in pending {
            let Some(session) = self.session.as_mut() else {
                return;
            };
            let Some(profile) = Self::seat_monster(&db, &mut session.battle, slot, id) else {
                continue;
            };
            session.set_profile(slot, profile);
            state
                .global_state_mut()
                .persistent_state_mut()
                .mark_monster_seen(id);
            log::info!("Combat: 位 {slot} 新造了一只 {id}");

            if self.stage.is_none() {
                continue;
            }
            if self.actors.len() < SLOTS {
                self.actors.resize(SLOTS, None);
            }
            if let Some(actor) = self.make_actor(state, &db, slot) {
                let at = actor.entity.transform().borrow().position();
                // 这一位先前那只（倒下的）只是被藏起来，`IScene` 没有删实体的口子；
                // 留它在场景里当个看不见的空壳，散场整个舞台一起扔。
                if let Some(old) = self.actors[slot].take() {
                    old.entity.set_visible(false);
                }
                if let Some(scene) = self.stage.as_ref() {
                    scene.add_entity(actor.entity.clone());
                }
                self.actors[slot] = Some(actor);
                state.play_effect(SPAWN_EFFECT_GROUP, at);
            }
        }
    }

    fn build_stage(&mut self, scene_manager: &ComRc<ISceneManager>, state: &mut SceState) {
        // 战斗外 143 播到 camera 类留下的请求，别让它顶掉开战镜头。
        let _ = state.take_combat_camera_requests();
        let _ = state.take_combat_action_requests();
        let _ = state.take_combat_shake_request();
        let _ = state.take_combat_movie_requests();
        self.cues.clear();
        self.shake = None;
        self.movies.clear();
        use crate::openpal3::combat::stage;
        use common::store_ext::StoreExt2;
        use mini_fs::StoreExt;
        use radiance::scene::CoreScene;

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
        // 原版战斗第一轮里灯是挂着的（`FUN_00510ca0` 画舞台时一并画灯光节点），
        // 角色吃这组灯；环境光是引擎默认的 0.2。舞台本身带光照贴图、不吃灯。
        scene.set_lighting(stage::scene_lighting(vfs, &scene_name));
        scene.add_entity(crate::openpal3::loaders::pol::create_entity_from_pol_model(
            &factory,
            vfs,
            &model_path,
            format!("COMBAT_STAGE_{scene_name}"),
            true,
        ));

        // 参战的人。站位表短了或者模型缺了就跳过那一个。
        for slot in 0..stage::STAGE_SLOTS {
            if self.actors.len() < SLOTS {
                self.actors.resize(SLOTS, None);
            }
            let Some(actor) = self.make_actor(state, &db, slot) else {
                continue;
            };
            scene.add_entity(actor.entity.clone());
            self.actors[slot] = Some(actor);
        }

        // 选目标时的 3D 指针（`FUN_00510540`），自己的 CVD 动画一直循环；位置每帧在
        // `place_pointer` 里摆。
        let pointer_path =
            std::path::PathBuf::from("/basedata/basedata/ui/combat/pointer/pointer.cvd");
        if vfs.open(&pointer_path).is_ok() {
            use crate::openpal3::scene::{
                CvdPlayMode, create_entity_from_cvd_model, play_cvd_subtree,
            };
            for slot in 0..PARTY_SLOTS {
                let pointer = create_entity_from_cvd_model(
                    factory.clone(),
                    vfs,
                    &pointer_path,
                    format!("COMBAT_POINTER{slot}"),
                    false,
                );
                play_cvd_subtree(&pointer, CvdPlayMode::Loop, 1.);
                pointer.set_visible(false);
                scene.add_entity(pointer.clone());
                self.pointers.push(pointer);
            }
        }

        // 五灵环（`FUN_0050fee0` 载两张贴图），平时不画，开场和阵型面板开关时淡入淡出。
        {
            let data = db.combat();
            if let Some(fx) = crate::openpal3::combat::lineup_ring::LineupRingFx::build(
                &factory,
                vfs,
                data.five_lineup.clone(),
                data.enemy_five_lineup.clone(),
            ) {
                for e in fx.entities() {
                    scene.add_entity(e);
                }
                self.lineup_ring = Some(fx);
            }
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
            self.linker = asset_mgr
                .vfs()
                .read_to_end(crate::openpal3::combat::linker::LINKER_PATH)
                .ok()
                .and_then(|b| crate::openpal3::combat::linker::EffectLinker::parse(&b));
            // 两方的定点（`FUN_00508930`）：敌方是 10 号位（读 +0x40 那个舞台角色）；我方读
            // 舞台场景 `+0x384..+0x38c`——构造 `FUN_0050fd00`（0x50fdc7）清零后全程序没人再写，
            // 就是原点。
            let positions = &db.combat().role_positions;
            let pos =
                |k: usize| stage::stage_position(positions, k).map(|p| Vec3::new(p.x, 0., p.z));
            self.side_anchors[0] = Vec3::new(0., 0., 0.);
            if let Some(p) = pos(10) {
                self.side_anchors[1] = p;
            }
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
            // 胜负判定 `FUN_004f7910` 给每个在场活着的我方排一组 0x18：`LAB_004fe550` 占住角色
            // （`FUN_0050a8f0`，+0x175/+0x178）→ `LAB_00501cf0` 等角色空闲、没眠没 0xe，挂胜利任务 →
            // `LAB_004fe600` 放开（`FUN_0050a920`）。胜利任务 `FUN_0050fa20`：第一帧换 z20（0x31），等引擎在动作播完
            // 时发的 "over"（`+0x19f4`）就结束。任务表空了 `FUN_0050b430` 每帧换回默认动作
            // `FUN_0050bbd0`（活着的我方按精：z04 / z17 濒死 / 眠定……），所以做完不停在最后一帧。
            let alive: Vec<bool> = (0..self.actors.len())
                .map(|slot| {
                    self.session
                        .as_ref()
                        .and_then(|s| s.battle.get(slot))
                        // 0x18 组第二步（`LAB_00501cf0`）：中了眠 0x11 或 0xe 号状态的不做 z20。
                        .is_some_and(|c| c.alive && !c.has_state(0x11) && !c.has_state(0xe))
                })
                .collect();
            for (slot, actor) in self.actors.iter_mut().enumerate().take(PARTY_SLOTS) {
                let Some(actor) = actor.as_mut() else {
                    continue;
                };
                if !alive[slot] {
                    continue;
                }
                actor.win_pose = true;
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
        let messages = std::mem::take(&mut self.result_messages);
        // 物品的 wa006 登记时就放（`FUN_004fb370`），不等横幅出来。
        for _ in 0..crate::openpal3::combat::result::item_sounds(&messages) {
            ui_sound::play(state, "wa006");
        }
        let steps = timeline(outcome, messages);
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
            Kind::Field {
                count, ids, sneak, ..
            } => Encounter::field(&mut CrtRand::new(seed), count, ids, sneak),
        };
        if matches!(self.kind, Kind::Field { .. }) {
            // `FUN_00417420` 开战前写死 `DAT_005833cc = 1`。
            self.game_over_on_loss = true;
        }

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
                // 托管开关：`FUN_004ed250` 从冒险记录 `+0x294..+0x299` 拷进 `+0x568..+0x56d`。
                c.delegation = role.delegation;
                // 情：`FUN_004ed250` 从 `DAT_00c04348` 那张好感表拷进 `+0x274`。
                c.affection = persistent.favor(actor_id);
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
                // 五灵阵位：原版 `FUN_004ed250` 从冒险侧记录的 `+0x29a` 搬进战斗位的
                // `+0x470`。阵法页写回时（`FUN_004845f0`）存的是「格子号 + 1」：1..5 水火
                // 风雷土、6 盘心；移植端存的是格子号 0..5。
                c.element_position = persistent
                    .lineup_position(actor_id)
                    .map(|p| p + 1)
                    .unwrap_or(0);
                // 手上武器的类型（战斗位 `+0x288`），合击技按它比对。
                c.weapon_type = persistent
                    .equipped(
                        actor_id,
                        crate::openpal3::states::persistent_state::SLOT_WEAPON,
                    )
                    .and_then(|id| db.item(id))
                    .map(|i| i.weapon_type)
                    // 没拿武器时 `FUN_004ed8c0` 给塞一把魔剑（0x19cb，武器类型 2）。
                    .unwrap_or(2);
                // 武器的连击数（gdb `combo_count`），龙葵普攻用（`FUN_0050e550`）。
                c.weapon_hits = persistent
                    .equipped(
                        actor_id,
                        crate::openpal3::states::persistent_state::SLOT_WEAPON,
                    )
                    .and_then(|id| db.item(id))
                    .map(|i| i.combo_count)
                    .unwrap_or(0);
                // 0 号位拿魔剑（没拿武器也按魔剑，`FUN_004ed8c0`），打击音效用。
                c.magic_sword = actor_id == 0
                    && persistent
                        .equipped(
                            actor_id,
                            crate::openpal3::states::persistent_state::SLOT_WEAPON,
                        )
                        .is_none_or(|id| id == 0x19cb);
                // 五件装备（武器不算）逐件累加（`FUN_004edf10`）：减气耗 `+0x290`、减神耗
                // `+0x294`、暴击加成 `+0x298`；特殊类型 0x17 的每件顺手偷 +5（`FUN_004ee430`，
                // 其余几种写的字段普攻不读）；饰品栏戴着青铜护镜的挨普攻会反弹（`+0x2bd`）。
                {
                    use crate::openpal3::states::persistent_state::{
                        SLOT_ADORN1, SLOT_ADORN2, SLOT_CLOTH, SLOT_HAT, SLOT_SHOES, SLOT_WEAPON,
                    };
                    for s in 0..SLOT_WEAPON {
                        let Some(item) =
                            persistent.equipped(actor_id, s).and_then(|id| db.item(id))
                        else {
                            continue;
                        };
                        c.sp_saving += item.sp_saving_percentage as i32;
                        c.mp_saving += item.mp_saving_percentage as i32;
                        c.crit_bonus += item.critical_attack_amplify_percentage as i32;
                        c.skill_bonus += item.special_skill_success_rate as i32;
                        // 每回合恢复 %（`+0x2a0..+0x2a8` ← 物品 `+0x284..+0x288`）。
                        c.hp_regen += f32::from(item.attribute_impact_value[0]);
                        c.sp_regen += f32::from(item.attribute_impact_value[1]);
                        c.mp_regen += f32::from(item.attribute_impact_value[2]);
                        if matches!(item.item_special_type, 0x15 | 0x16) {
                            c.item_boost += 10;
                        }
                        // 经验 / 金钱加成（`+0x2ac` / `+0x2b0`，`FUN_004ee430`）。
                        match item.item_special_type {
                            0x14 => c.exp_bonus += 5,
                            0x15 => {
                                c.exp_bonus += 15;
                                c.money_bonus += 5;
                            }
                            0x18 => c.exp_bonus += 20,
                            _ => {}
                        }
                        if item.item_special_type == 0x17 {
                            c.steal_chance += 5;
                        }
                        if item.id == BRONZE_MIRROR && (s == SLOT_ADORN1 || s == SLOT_ADORN2) {
                            c.reflect = true;
                        }
                    }
                    // 「千金散去」（`cbrolestate.cpp:1358`，战斗位 `+0x2bc`）：绿幞头 + 通神 +
                    // 元宝鞋 + 右利 + 左利，五件齐了才算；源码写了两支，左利右利戴反也认。
                    {
                        let at = |slot: usize| persistent.equipped(actor_id, slot);
                        let wear = [at(SLOT_ADORN1), at(SLOT_ADORN2)];
                        c.lost_money_get_life = at(SLOT_HAT) == Some(LOST_MONEY_HAT)
                            && at(SLOT_CLOTH) == Some(LOST_MONEY_CLOTH)
                            && at(SLOT_SHOES) == Some(LOST_MONEY_SHOES)
                            && (wear == [Some(LOST_MONEY_RIGHT), Some(LOST_MONEY_LEFT)]
                                || wear == [Some(LOST_MONEY_LEFT), Some(LOST_MONEY_RIGHT)]);
                        if c.lost_money_get_life {
                            log::info!("Combat: 位 {slot} 装齐了千金散去");
                        }
                    }
                    // 景天身上有精进（冒险侧状态 30；`DAT_00c0466a` 就是 0 号角色的那一格），
                    // 全队每人经验加成 +20（`FUN_004ed250`）。
                    if persistent.role(0).is_some_and(|r| r.states[30] != 0) {
                        c.exp_bonus += 20;
                    }
                    // 武器带的普攻附加状态（`FUN_004f2560` 读武器那一格）。
                    if let Some(w) = persistent
                        .equipped(actor_id, SLOT_WEAPON)
                        .and_then(|id| db.item(id))
                    {
                        for i in 0..crate::openpal3::combat::state::WEAPON_STATES {
                            if w.combat_state_impact_types[i] != 0
                                && w.combat_state_impact_value[i] > 0
                            {
                                c.weapon_states[i] = w.combat_state_impact_value[i];
                            }
                        }
                    }
                }
                if c.hp <= 0 {
                    c.alive = false;
                }
                battle.place(slot, c);

                profiles[slot] = Profile {
                    name: db
                        .combat_actor(combat_id)
                        .and_then(|a| a.name.as_str().ok())
                        .unwrap_or_else(|| format!("#{combat_id}")),
                    // 类型 0 是特技（控制盘「技」，战斗位 `+0x2c8`），其余进仙术。
                    skills: role
                        .skills
                        .iter()
                        .filter(|s| !Self::is_special(&db, s.id))
                        // 出手倍率：法术表里的等级 + 1（`FUN_004f76c0`）。
                        .filter_map(|s| Self::skill_choice(&db, s.id, s.level as i32 + 1))
                        .collect(),
                    specials: {
                        let mut ids: Vec<u32> = role
                            .skills
                            .iter()
                            .map(|s| s.id)
                            .filter(|id| Self::is_special(&db, *id))
                            .collect();
                        // 升级学会的特技（学会等级非 0 且已到）：老存档里可能还没写进 role.skills。
                        ids.extend(
                            db.skills()
                                .iter()
                                .filter(|s| {
                                    s.skill_type == 0
                                        && s.required_actor_level != 0
                                        && s.required_actor_level as i32 <= role.level
                                        && s.applicable_actors
                                            .get(actor_id as usize)
                                            .is_some_and(|v| *v != 0)
                                })
                                .map(|s| s.id),
                        );
                        ids.sort_unstable();
                        ids.dedup();
                        ids.into_iter()
                            // 特技不在法术表里，`FUN_004f76c0` 找不到返回 0。
                            .filter_map(|id| Self::skill_choice(&db, id, 0))
                            .collect()
                    },
                    reward: Reward::default(),
                };
            }

            // ---- enemies ----
            for (i, id) in encounter.monsters.iter().copied().enumerate() {
                if id == 0 {
                    continue;
                }
                let slot = PARTY_SLOTS + i;
                if let Some(p) = Self::seat_monster(&db, &mut battle, slot, id) {
                    profiles[slot] = p;
                    persistent.mark_monster_seen(id);
                }
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
        battle.assign_patterns();
        // 偷袭（`FUN_004edc60`）：朝向判定 1 / 2 还要掷 `rand & 0x3ff < 0x200` 才成立——1 → `+0x218` 我方偷袭，
        // 2 → `+0x219` 敌方偷袭（`FUN_00407320` 中间把 1、2 对调成描述符 `+0x44` 的 2、1，再对调回来）。
        // 脚本开战一律是 0。然后开场蓄力条（`FUN_004dbb80`）。
        let we = encounter.sneak == 1 && battle.rng.rand10() < 0x200;
        let enemy = encounter.sneak == 2 && battle.rng.rand10() < 0x200;
        self.sneak = (we, enemy);
        if encounter.sneak != 0 {
            log::info!(
                "Combat: 偷袭判定 {}，成立：我方 {we} 敌方 {enemy}",
                encounter.sneak
            );
        }
        if we || enemy {
            use common::store_ext::StoreExt2;
            let (key, fallback) = if we {
                ("WeSneakAttackEnemy", "我方偷袭成功")
            } else {
                ("EnemySneakAttackUs", "敌人偷袭成功")
            };
            self.sneak_text = Some(
                state
                    .asset_mgr()
                    .vfs()
                    .read_to_end_from_gbk(crate::openpal3::combat::result::LANG_PATH)
                    .ok()
                    .and_then(|t| crate::openpal3::combat::result::lang_value(&t, key))
                    .unwrap_or_else(|| fallback.to_string()),
            );
        }
        battle.seed_gauges(we, enemy);
        let mut session = CombatSession::new(battle, profiles, seed.rotate_left(13) ^ 0x5bd1_e995);
        session.set_items(items);
        session.set_combos(
            db.combo_skills()
                .iter()
                .map(crate::openpal3::combat::combo::ComboDef::from_gdb)
                .collect(),
        );
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
        log::info!("Combat: 结算 {:?}", outcome);
        let db = state.global_state().game_db().cloned();
        let mut raw: Option<Settled> = None;

        {
            let mut persistent = state.global_state_mut().persistent_state_mut();
            for actor_id in self.party_actor_ids.iter().copied() {
                // 队伍下标就是战斗位号。
                let Some(c) = session.battle.get(actor_id as usize) else {
                    continue;
                };
                // 五灵阵位写回：`FUN_004ed800` 把战斗位 `+0x470` 抄回冒险记录 `+0x29a`
                // （移植端存格子号 0..5）。
                if (1..=6).contains(&c.element_position) {
                    persistent.set_lineup_position(actor_id, c.element_position - 1);
                }
                if let Some(role) = persistent.role_mut(actor_id) {
                    // 托管开关写回（`FUN_004ed800`）。战斗结束判定 `FUN_004f7910` 三支都会走到它：
                    // 战败（`FUN_004ed0a0(0) == 1`）直接调；胜利 `FUN_004ed870` 之后、逃跑（`+0x78 == 4`）
                    // 各调一次整份写回 `FUN_004ed680`，它最后也调 `FUN_004ed800`。所以不分胜负都写回。
                    role.delegation = c.delegation;
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
            // 飞龙探云手偷到的钱记成负的花费，所以这里是 != 0。
            if session.battle.spent_money != 0 {
                persistent.add_money(-session.battle.spent_money);
            }
            for (id, n) in session.battle.stolen_items.iter().copied() {
                persistent.add_item(id, n.max(0) as u32);
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
            // 饿鬼整格吃掉的道具。
            for (id, n) in session.eaten_items.iter().copied() {
                persistent.remove_item(id, n);
            }

            if won {
                let spoils = session.spoils();
                let mut level_ups: Vec<(usize, i32, Vec<String>)> = Vec::new();
                let mut skill_ups: Vec<(usize, String, u8, Option<u8>)> = Vec::new();
                if let Some(db) = db.as_ref() {
                    for (slot, amount) in spoils.experience {
                        if let Some(actor_id) = self
                            .party_actor_ids
                            .iter()
                            .copied()
                            .find(|a| *a as usize == slot)
                        {
                            let old = persistent.role(actor_id).map(|r| r.level).unwrap_or(0);
                            if persistent.award_experience(actor_id, db, amount) > 0 {
                                let level = persistent.role(actor_id).map(|r| r.level).unwrap_or(0);
                                // 升级时对特技库扫一遍，够等级的学会（`FUN_004fa930`）。
                                let learned = persistent
                                    .learn_level_specials(db, actor_id, old, level)
                                    .into_iter()
                                    .filter_map(|id| db.skill(id))
                                    .map(|s| {
                                        s.name.as_str().map(|n| n.to_string()).unwrap_or_default()
                                    })
                                    .collect();
                                level_ups.push((slot, level, learned));
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
                        if let Some(level) =
                            persistent.use_skill(actor_id, skill_id, &row.times_before_level_up)
                        {
                            // 五灵横幅只给 `SkillType` 1..3（`FUN_004f69f0`），行按五灵定。
                            let banner = (1..=3)
                                .contains(&row.skill_type)
                                .then(|| {
                                    crate::openpal3::combat::result::magic_row(
                                        &row.element_attributes,
                                    )
                                })
                                .flatten();
                            let name = row.name.as_str().map(|n| n.to_string()).unwrap_or_default();
                            skill_ups.push((slot, name, level, banner));
                        }
                    }
                }
                persistent.add_money(spoils.money);
                let loot = spoils.loot.clone();
                for (item, count) in spoils.loot {
                    if count > 0 {
                        persistent.add_item(item, count as u32);
                    }
                }
                raw = Some((level_ups, skill_ups, spoils.money, loot));
            }
        }

        // 结算画面要的消息和面板（combat::result）。
        if let Some((level_ups, skill_ups, money, loot)) = raw {
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
            for (slot, level, learned) in level_ups {
                msgs.push(ResultMessage {
                    kind: MessageKind::LevelUp,
                    text: lang.level_up(session.name(slot), level),
                    row: None,
                });
                for skill in learned {
                    msgs.push(ResultMessage {
                        kind: MessageKind::Mastered,
                        text: lang.mastered(session.name(slot), &skill),
                        row: None,
                    });
                }
            }
            // 仙术升级（`LAB_00502cc0`）：显示的级数是内部等级 + 1（0x4fac30 压的 `bVar3 + 1`）。
            for (slot, skill, level, row) in skill_ups {
                msgs.push(ResultMessage {
                    kind: MessageKind::SkillUp,
                    text: lang.skill_up(session.name(slot), &skill, level as i32 + 1),
                    row,
                });
            }
            if money > 0 {
                msgs.push(ResultMessage {
                    kind: MessageKind::Money,
                    text: lang.money(money),
                    row: None,
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
                    row: None,
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
                    let mut elements = [0i32; 6];
                    if let (Some(role), Some(db)) = (persistent.role(slot as u32), db.as_ref()) {
                        for s in &role.skills {
                            let row = db.skill(s.id).filter(|k| k.skill_type != 0).and_then(|k| {
                                crate::openpal3::combat::result::magic_row(&k.element_attributes)
                            });
                            if let Some(r) = row {
                                elements[r as usize] += s.uses as i32;
                            }
                        }
                    }
                    Some(ResultPanel {
                        slot,
                        elements,
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
    fn drive_staging(&mut self, state: &mut SceState, delta_sec: f32) {
        use crate::openpal3::combat::linker::Anchor;
        use motion::Motion;
        let positions: Vec<Option<Vec3>> = self
            .actors
            .iter()
            .map(|a| a.as_ref().map(|a| a.entity.transform().borrow().position()))
            .collect();
        let homes: Vec<Option<Vec3>> = self
            .actors
            .iter()
            .map(|a| a.as_ref().map(|a| a.home))
            .collect();
        let side_anchors = self.side_anchors;
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
                Some(Pending::Cast { special: true, .. }) => ActKind::Special,
                Some(Pending::Cast { .. }) | Some(Pending::Combo { .. }) => ActKind::Cast,
                Some(Pending::Item { target, .. }) => ActKind::Item {
                    hostile: (target < PARTY_SLOTS) != (slot < PARTY_SLOTS),
                },
                _ => {
                    session.release();
                    return;
                }
            };
            let link_id = match session.pending {
                Some(Pending::Cast { skill, special, .. }) => session
                    .cast_choice(slot, skill, special)
                    .map(|s| s.effect.id),
                Some(Pending::Item { item, .. }) => session.items().get(item).map(|i| i.effect.id),
                // 合击技的特效挂在链接表里它自己的 id 下（501..510）。
                Some(Pending::Combo { combo, .. }) => session.combos().get(combo).map(|c| c.id),
                _ => None,
            };
            let link = link_id.and_then(|id| self.linker.as_ref()?.find(id));
            if let (Some(id), None) = (link_id, link) {
                log::debug!("Combat: efflinker 里没有 {id}");
            }
            self.run = Some(StageRun {
                slot,
                kind,
                step: StageStep::Begin,
                link,
                target: None,
                played: [false; 2],
                combo: matches!(session.pending, Some(Pending::Combo { .. })),
                hits_done: 0,
                events_done: 0,
            });
        }
        let target = match session.pending {
            Some(Pending::Attack { target, .. })
            | Some(Pending::Cast { target, .. })
            | Some(Pending::Item { target, .. })
            | Some(Pending::Combo { target, .. }) => Some(target),
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

        // 合击技摆位 / 复位要挪别人，等这一段借用结束后再做。第三项：朝向是否还原成站位的朝向。
        let mut moves: Vec<(usize, Vec3, bool)> = Vec::new();
        let mut restore_all = false;
        let mut stow_centre = false;
        // 合击技的渐黑 / 渐亮（0x502000 / 0x5020d0）按任意键就走到底（`FUN_00402b00`）。
        let skip_fade = any_key_pressed(state);
        match run.step {
            StageStep::Begin if run.combo => {
                run.target = target;
                run.step = StageStep::ComboOpen { t: 0. };
            }
            StageStep::ComboOpen { t } => {
                let t = if skip_fade {
                    COMBO_STEP_SECONDS
                } else {
                    t + delta_sec
                };
                self.stage_fade = (t / COMBO_STEP_SECONDS).min(1.);
                if t < COMBO_STEP_SECONDS {
                    run.step = StageStep::ComboOpen { t };
                } else {
                    // 0x4ff3d0：在场还站着的我方都挪到自己阵位对应的点上；范围 2 时目标挪到
                    // 10 号位的原位。
                    use crate::openpal3::combat::stage as st;
                    use crate::openpal3::states::combat_data::StagePos;
                    let range = match session.pending {
                        Some(Pending::Combo { combo, .. }) => {
                            session.combos().get(combo).map(|c| c.range)
                        }
                        _ => None,
                    };
                    if let Some(db) = state.global_state().game_db().cloned() {
                        let data = db.combat();
                        let ring = StagePos {
                            x: data.five_lineup.x,
                            z: data.five_lineup.z,
                        };
                        for s in 0..PARTY_SLOTS {
                            let Some(c) = session.battle.get(s).filter(|c| c.in_play()) else {
                                continue;
                            };
                            // 挪位的同时把朝向还原成站位的朝向（`+0x19e8 = +0x19ec`）。
                            if let Some(p) =
                                st::lineup_spot(&data.role_positions, ring, c.element_position)
                            {
                                moves.push((s, Vec3::new(p.x, 0., p.z), true));
                            }
                        }
                        if range == Some(2) {
                            if let (Some(t), Some(p)) =
                                (run.target, st::stage_position(&data.role_positions, 10))
                            {
                                // 目标只挪位置，朝向不动（0x4ff51b 起只抄 `+0x19b8`）。
                                if t >= PARTY_SLOTS && t != 10 {
                                    // 盘心原先有人（在场、没逃走、还站着）就先把它摆回原位
                                    // 收起来，腾出 10 号位给目标。
                                    if session.battle.get(10).is_some_and(|c| c.in_play()) {
                                        stow_centre = true;
                                    }
                                    moves.push((t, Vec3::new(p.x, 0., p.z), false));
                                }
                            }
                        }
                    }
                    run.step = StageStep::ComboShow { t: 0. };
                }
            }
            StageStep::ComboShow { t } => {
                let t = if skip_fade {
                    COMBO_STEP_SECONDS
                } else {
                    t + delta_sec
                };
                self.stage_fade = (1. - t / COMBO_STEP_SECONDS).max(0.);
                if t < COMBO_STEP_SECONDS {
                    run.step = StageStep::ComboShow { t };
                } else {
                    let name = match session.pending {
                        Some(Pending::Combo { combo, .. }) => {
                            session.combos().get(combo).map(|c| c.name.clone())
                        }
                        _ => None,
                    };
                    if let Some(name) = name {
                        session.announce(&name, COMBO_STEP_SECONDS);
                    }
                    run.step = StageStep::ComboBanner { t: 0. };
                }
            }
            StageStep::ComboBanner { t } => {
                let t = t + delta_sec;
                if t < COMBO_STEP_SECONDS {
                    run.step = StageStep::ComboBanner { t };
                } else {
                    // 出手者不摆出手动作：人怎么动由特效里的 action 提示管。
                    rc.set_auto_play_idle(true);
                    run.step = StageStep::Strike { t: 0. };
                }
            }
            StageStep::ComboClose { t } => {
                let t = if skip_fade {
                    COMBO_STEP_SECONDS
                } else {
                    t + delta_sec
                };
                self.stage_fade = (t / COMBO_STEP_SECONDS).min(1.);
                if t < COMBO_STEP_SECONDS {
                    run.step = StageStep::ComboClose { t };
                } else {
                    restore_all = true;
                    run.step = StageStep::ComboReveal { t: 0. };
                }
            }
            StageStep::ComboReveal { t } => {
                let t = if skip_fade {
                    COMBO_STEP_SECONDS
                } else {
                    t + delta_sec
                };
                self.stage_fade = (1. - t / COMBO_STEP_SECONDS).max(0.);
                if t >= COMBO_STEP_SECONDS {
                    self.stage_fade = 0.;
                    session.release();
                    self.run = None;
                } else {
                    run.step = StageStep::ComboReveal { t };
                }
            }
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
                run.target = run.target.or(target);
                // efflinker 那一条：按起播时刻放特效，结算落在它写的时刻。
                if let Some(link) = run.link {
                    let side = |s: usize| side_anchors[(s >= PARTY_SLOTS) as usize];
                    for (k, e) in link.effects().enumerate() {
                        if run.played[k] || t < e.at {
                            continue;
                        }
                        run.played[k] = true;
                        let at = match e.anchor {
                            Anchor::Actor => positions.get(slot).copied().flatten(),
                            Anchor::Target => {
                                run.target.and_then(|t| positions.get(t).copied().flatten())
                            }
                            Anchor::ActorSide => Some(side(slot)),
                            Anchor::TargetSide => run.target.map(side),
                            Anchor::Fixed(p) => Some(Vec3::new(p[0], p[1], p[2])),
                        };
                        if let Some(at) = at {
                            log::info!("Combat: 特效组 {} 于 {at:?}", e.group);
                            state.play_effect(e.group, at);
                        }
                    }
                }
                // 普攻按攻击动作里的 `work` / `work1` 标记逐个处理（`FUN_0050e550`，见
                // `motion::attack_events`）：先放打击音效、敌方的打击特效（目标站位），再结算那几下，
                // 最后一下才算行动做完。其余按 efflinker 写的时刻结算一次。
                let hit_at = if run.kind == ActKind::Attack && run.link.is_none() {
                    let events = &actor.hit_events;
                    let total: u32 = events.iter().map(|e| e.repeats).sum();
                    while run.events_done < events.len() && t >= events[run.events_done].at {
                        let e = &events[run.events_done];
                        run.events_done += 1;
                        if let Some(name) = &e.sound {
                            ui_sound::play(state, name);
                        }
                        let at = run.target.and_then(|t| homes.get(t).copied().flatten());
                        if let (Some(group), Some(at)) = (e.effect, at) {
                            state.play_effect(group, at);
                        }
                        for _ in 0..e.repeats {
                            run.hits_done += 1;
                            if run.hits_done >= total {
                                session.strike();
                            } else {
                                session.attack_hit();
                            }
                        }
                    }
                    events.last().map(|e| e.at).unwrap_or(motion::HIT_DELAY)
                } else {
                    let at = run.link.map(|l| l.result_at).unwrap_or(motion::HIT_DELAY);
                    if t >= at && run.hits_done == 0 {
                        run.hits_done = 1;
                        session.strike();
                    }
                    at
                };
                let effects_done = run
                    .link
                    .is_none_or(|l| l.effects().enumerate().all(|(k, _)| run.played[k]));
                let done = t >= hit_at
                    && effects_done
                    && rc.state() != crate::openpal3::scene::RoleState::PlayingAnimation;
                if done && run.combo {
                    run.step = StageStep::ComboClose { t: 0. };
                } else if done {
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
        for (s, p, face) in moves {
            if let Some(a) = self.actors.get_mut(s).and_then(|a| a.as_mut()) {
                a.walk = None;
                let transform = a.entity.transform();
                let mut t = transform.borrow_mut();
                t.set_position(&p);
                if face {
                    t.look_at(&face_point(p, motion::rest_facing(a.set)));
                }
            }
        }
        if stow_centre {
            if let Some(a) = self.actors.get_mut(10).and_then(|a| a.as_mut()) {
                a.walk = None;
                a.stowed = true;
                let home = a.home;
                a.entity.transform().borrow_mut().set_position(&home);
                a.entity.set_visible(false);
            }
        }
        // 0x4ff740：在场的都摆回站位，朝向也还原（`+0x19e8 = +0x19ec`）。
        if restore_all {
            for a in self.actors.iter_mut().flatten() {
                a.walk = None;
                let home = a.home;
                a.entity
                    .transform()
                    .borrow_mut()
                    .set_position(&home)
                    .look_at(&face_point(home, motion::rest_facing(a.set)));
            }
            // 收起来的那位显出来（`ProcAllRoleToInitPos` 末尾）。
            if let Some(a) = self.actors.get_mut(10).and_then(|a| a.as_mut()) {
                if a.stowed {
                    a.stowed = false;
                    if session.battle.get(10).is_some_and(|c| c.in_play()) {
                        a.entity.set_visible(true);
                    }
                }
            }
        }
    }

    /// 让舞台上的人跟着战况做动作（`combat::motion`）。
    /// 推特效 `action` 类的动作提示（`combat::stage_action`，原版 FUN_004c0c10）。
    fn drive_cues(&mut self, delta_sec: f32) {
        use crate::openpal3::combat::stage_action::yaw_direction;
        use crate::openpal3::scene::RoleState;
        if self.cues.is_empty() {
            return;
        }
        let mut cues = std::mem::take(&mut self.cues);
        for cue in cues.iter_mut() {
            let actor = usize::try_from(cue.slot)
                .ok()
                .and_then(|s| self.actors.get_mut(s))
                .and_then(|a| a.as_mut());
            let Some(actor) = actor else {
                // 位上没人：只等时间走完。
                cue.step(delta_sec, [0.; 3], 1., false);
                continue;
            };
            let rc = RoleController::get_role_controller(actor.entity.clone());
            let anim_done = rc.as_ref().is_some_and(|rc| {
                matches!(
                    rc.inner::<RoleController>().state(),
                    RoleState::AnimationFinished | RoleState::AnimationHolding
                )
            });
            let p = actor.entity.transform().borrow().position();
            let frame = cue.step(delta_sec, [p.x, p.y, p.z], actor.alpha, anim_done);
            if frame.begin {
                actor.scripted += 1;
                actor.walk = None;
                actor.hide_on_finish = false;
                actor.entity.set_visible(true);
                if let (Some(name), Some(rc)) = (cue.motion_name(), rc.as_ref()) {
                    let rc = rc.inner::<RoleController>();
                    // 原版播完停在最后一帧，由后面的提示或交还来换动作。
                    rc.set_auto_play_idle(false);
                    rc.play_anim(&name, RoleAnimationRepeatMode::Repeat(1));
                }
            }
            if let Some(next) = frame.position {
                actor
                    .entity
                    .transform()
                    .borrow_mut()
                    .set_position(&Vec3::new(next[0], next[1], next[2]));
            }
            if let Some(yaw) = frame.yaw {
                let pos = actor.entity.transform().borrow().position();
                let (dx, dz) = yaw_direction(yaw);
                actor
                    .entity
                    .transform()
                    .borrow_mut()
                    .look_at(&face_point(pos, (dx, dz)));
            }
            if let Some(alpha) = frame.alpha {
                actor.alpha = alpha;
                super::role_fade::set_alpha(&actor.entity, alpha);
            }
            if frame.end {
                actor.scripted = actor.scripted.saturating_sub(1);
                if actor.scripted == 0 {
                    actor.released = true;
                }
            }
        }
        cues.retain(|c| !c.finished());
        cues.extend(std::mem::take(&mut self.cues));
        self.cues = cues;
    }

    /// 3D 指针（`FUN_004fc3a0` 画舞台那段）：只看 0..4 号位（舞台角色 `+0x18..+0x28`）里
    /// `+0x118` 亮着的，每人画一个。亮谁：轮到谁选行动，`FUN_004dc3f0` 和控制盘状态
    /// 0x4dc7a0 给那个人亮；选目标时 0x4fd3d0 按亮起来的一组
    /// （[`CombatSession::highlighted`]）重设、组外的熄掉——所以敌方目标头上没有指针，它们
    /// 只闪（`+0x112`，见下面的染色）。
    /// 点取包围盒中心、y 取顶再 +10（`FUN_0050a9b0`；707 号怪再 −80），然后
    /// +6 + 5·sin(10t + i) 上下跳、y 方向拉伸 1 + 0.2·sin(…)；t 是战斗时钟，i 是位号。
    fn place_pointer(&mut self) {
        // 选目标时组里的人（我方敌方都算）`+0x112` 亮：角色渲染 `FUN_00509b90` 换
        // `geom_diffuse.gbf`（战斗系统 `+0x4c` 对象 `+0x984`，`FUN_005084c0` 载）来画，顶点色 =
        // 效果变量 6（`specular`）= 0.65 + 0.35·sin(战斗时钟 × 7.8)，不算光照。移植端在
        // 自身光照上乘这个灰度（渲染对象 tint.rgb）。
        let pulse: Vec<usize> = match self.session.as_ref() {
            Some(s)
                if s.turn
                    .is_some_and(|t| matches!(t.menu, Menu::Target { .. })) =>
            {
                s.highlighted()
            }
            _ => Vec::new(),
        };
        let c = 0.65 + 0.35 * (self.battle_clock * 7.8).sin();
        for (slot, actor) in self.actors.iter_mut().enumerate() {
            let Some(actor) = actor.as_mut() else {
                continue;
            };
            if pulse.contains(&slot) {
                super::role_fade::set_tint_rgb(&actor.entity, [c, c, c]);
                actor.pulsing = true;
            } else if actor.pulsing {
                super::role_fade::set_tint_rgb(&actor.entity, [1., 1., 1.]);
                actor.pulsing = false;
            }
        }
        let lit: Vec<usize> = match self.session.as_ref() {
            Some(s) => match s.turn {
                Some(t) if matches!(t.menu, Menu::Target { .. }) => s.highlighted(),
                Some(t) => vec![t.slot],
                None => Vec::new(),
            },
            None => Vec::new(),
        };
        for (slot, pointer) in self.pointers.iter().enumerate() {
            let top = lit
                .contains(&slot)
                .then(|| self.actors.get(slot)?.as_ref())
                .flatten()
                .and_then(|actor| head_point(&actor.entity));
            let Some(top) = top else {
                pointer.set_visible(false);
                continue;
            };
            let mut y = top.y + 10.;
            if self
                .session
                .as_ref()
                .and_then(|s| s.battle.get(slot))
                .is_some_and(|c| c.actor_id == 707)
            {
                y -= 80.;
            }
            let wave = (self.battle_clock * 10. + slot as f32).sin();
            y += 6. + 5. * wave;
            let mut t = radiance::math::Transform::new();
            t.set_position(&Vec3::new(top.x, y, top.z))
                .scale_local(&Vec3::new(1., 1. + 0.2 * wave, 1.));
            pointer.transform().borrow_mut().set_matrix(*t.matrix());
            pointer.set_visible(true);
        }
    }

    fn animate_stage(&mut self, delta_sec: f32) {
        let dodged = self
            .session
            .as_mut()
            .map(|s| std::mem::take(&mut s.dodged))
            .unwrap_or_default();
        let Some(session) = self.session.as_ref() else {
            return;
        };
        let pending = session.pending;
        let mut fled: Vec<usize> = Vec::new();
        for slot in 0..self.actors.len() {
            let Some(actor) = self.actors[slot].as_mut() else {
                continue;
            };
            let Some(c) = session.battle.get(slot) else {
                continue;
            };
            let acting = pending.filter(|p| p.slot() == slot).map(|p| match p {
                Pending::Attack { .. } => ActKind::Attack,
                Pending::Cast { special: true, .. } => ActKind::Special,
                Pending::Cast { .. } | Pending::Combo { .. } => ActKind::Cast,
                Pending::Item { target, .. } => ActKind::Item {
                    hostile: (target < PARTY_SLOTS) != (slot < PARTY_SLOTS),
                },
                Pending::Escape { .. } => ActKind::Escape,
            });
            let now = Snapshot::of(c, acting);
            if actor.scripted > 0 {
                // 特效在控制他：动作、位置、朝向都听 `drive_cues` 的。
                actor.last = now;
                continue;
            }
            let Some(rc) = RoleController::get_role_controller(actor.entity.clone()) else {
                continue;
            };
            let rc = rc.inner::<RoleController>();

            if actor.hide_on_finish
                && rc.state() == crate::openpal3::scene::RoleState::AnimationFinished
            {
                actor.entity.set_visible(false);
                actor.hide_on_finish = false;
            }
            if actor.win_pose && rc.state() == crate::openpal3::scene::RoleState::AnimationFinished
            {
                actor.win_pose = false;
                if let Some(name) = actor.set.name(motion::rest_motion(&now)) {
                    rc.set_auto_play_idle(true);
                    rc.play_anim(name, RoleAnimationRepeatMode::Loop);
                }
            }

            let set = actor.set;
            let play = |m: motion::Motion, mode: RoleAnimationRepeatMode| {
                set.name(m).is_some_and(|name| rc.play_anim(name, mode))
            };

            // 逃走（`LAB_005011b0`）：舞台角色空下来就开跑，挂 `FUN_0050f590`——动作 0x34（怪物那套的
            // 走，循环），朝 (800, 0, 800) 每秒跑 380。
            if c.fleeing && actor.walk.is_none() {
                log::info!("Combat: 位 {slot}（{}）逃走", c.actor_id);
                let (x, z) = motion::FLEE_POINT;
                actor.walk = Some(Walk::Away {
                    dest: Vec3::new(x, actor.home.y, z),
                });
                rc.set_auto_play_idle(false);
                play(motion::Motion::Move, RoleAnimationRepeatMode::Loop);
            }

            // 走一帧。
            if let Some(walk) = actor.walk {
                let (dest, face_at_end) = match walk {
                    Walk::To { dest, face } => (dest, face),
                    Walk::Home => {
                        let (fx, fz) = motion::rest_facing(set);
                        (actor.home, face_point(actor.home, (fx, fz)))
                    }
                    Walk::Away { dest } => (dest, dest),
                };
                let away = matches!(walk, Walk::Away { .. });
                let speed = if away {
                    motion::FLEE_SPEED
                } else {
                    motion::WALK_SPEED
                };
                let pos = actor.entity.transform().borrow().position();
                let ((x, z), arrived) =
                    motion::step_toward_at((pos.x, pos.z), (dest.x, dest.z), delta_sec, speed);
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
                    if away {
                        // 跑出去了（0x50f946）：`+0x598 = 1`，站在终点不再动。
                        log::info!("Combat: 位 {slot} 跑出去了");
                        fled.push(slot);
                    }
                }
            }

            // 回避（z15）：普攻落空的那一下。怪物那套没有这个动作，`play` 自己跳过。
            if dodged.contains(&slot) && now.alive && now.hp == actor.last.hp {
                rc.set_auto_play_idle(true);
                play(motion::Motion::Evade, RoleAnimationRepeatMode::Repeat(1));
            }
            let reaction = if std::mem::take(&mut actor.released) {
                Some(Play::Rest)
            } else {
                motion::react(&actor.last, &now)
            };
            match reaction {
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
                        // 怪物没有倒地动作。原版挨打任务（0x50eb00 起）给它换 b，等引擎在
                        // 动作播完时发的 "over"（`+0x19f4` = CRC("over")，`FUN_005092b0`）才结束，
                        // 结束时死了就 `+0x110 = 0`（0x50ed32），渲染 `FUN_00509b90` 不再画。
                        // 中了 0xe 号状态的不换动作，下一帧就结束。
                        if c.has_state(0xe) {
                            actor.entity.set_visible(false);
                        } else {
                            play(
                                motion::Motion::BeAttacked,
                                RoleAnimationRepeatMode::Repeat(1),
                            );
                            actor.hide_on_finish = true;
                        }
                    }
                }
                Some(Play::Rest) => {
                    rc.set_auto_play_idle(true);
                    actor.entity.set_visible(!actor.stowed);
                    actor.hide_on_finish = false;
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
        if let Some(session) = self.session.as_mut() {
            for slot in fled {
                session.battle.finish_fleeing(slot);
            }
        }
    }

    /// 按原版 `CBUI` 的版面画界面，坐标见 `combat::hud`。
    fn draw(ui: &Ui, session: &CombatSession, tex: &HudTextures, clock: f32) {
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

            // 底下一排状态面板。原版开战 `FUN_004ec910` 给在场的我方按战斗位顺序
            // 依次发面板号（角色记录 `+0x240`），这里同样按战斗位顺序占。
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
                // 状态小图、头像、名字（`FUN_004f8480`，控件建的顺序画）。
                for (i, &(k, _, _)) in hud::STATE_ICONS.iter().enumerate() {
                    if c.has_state(k) {
                        let icon = tex.state_icons[i].as_deref();
                        let rect = hud::state_icon_rect(i, panel);
                        draw_sprite_tinted(&list, icon, rect, hud::state_icon_tint(clock), at);
                    }
                }
                if slot < PARTY_SLOTS {
                    let tint = hud::portrait_tint(|k| c.has_state(k), c.hp);
                    let portrait = tex.portraits.get(slot).and_then(|t| t.as_deref());
                    draw_sprite_tinted(&list, portrait, hud::portrait_rect(panel), tint, at);
                    let name = tex.names.get(slot).and_then(|t| t.as_deref());
                    draw_sprite(&list, name, hud::name_rect(panel), at);
                }
                // 精气神的当前值（文字队列 type 2，最后一起画在界面上面）。
                for gauge in Gauge::ALL {
                    let value = match gauge {
                        Gauge::Hp => c.hp,
                        Gauge::Gp => c.sp,
                        Gauge::Mp => c.mp,
                    };
                    let (x, y) = gauge.number_at(panel);
                    let color = gauge.number_color();
                    draw_number(
                        &list,
                        tex.number.as_deref(),
                        &value.to_string(),
                        x,
                        y,
                        color,
                        at,
                    );
                }
            }

            let Some(turn) = session.turn else {
                if let Some((notice, _)) = session.notice.as_ref() {
                    Self::draw_banner(ui, &list, tex.trick_back2.as_deref(), notice, at, scale);
                }
                return;
            };

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
                        let look = if !button
                            .entry()
                            .is_some_and(|e| session.entry_enabled(turn.slot, e))
                        {
                            ButtonLook::Disabled
                        } else if button == focus {
                            ButtonLook::Focused
                        } else {
                            ButtonLook::Normal
                        };
                        let r = hud::offset(button.rect(), panel);
                        if look == ButtonLook::Disabled {
                            // `FUN_0043ad40`：状态停在平时那张，上面再叠第 4 张，
                            // 没有第 4 张就叠一块 `DAT_00d91780` 色。
                            let base = tex.buttons[i][ButtonLook::Normal as usize].as_deref();
                            draw_sprite(&list, base, r, at);
                            match tex.buttons[i][ButtonLook::Disabled as usize].as_deref() {
                                Some(over) => draw_sprite(&list, Some(over), r, at),
                                None => list
                                    .add_rect(at(r.0, r.1), at(r.2, r.3), argb(hud::DISABLED_FILL))
                                    .filled(true)
                                    .build(),
                            }
                        } else {
                            let sprite = tex.buttons[i][look as usize].as_deref();
                            draw_sprite(&list, sprite, r, at);
                        }
                    }
                }
                Menu::Protect { focus, draft } => {
                    let panel = hud::PROTECT_PANEL;
                    let back = hud::offset(hud::PROTECT_BACK_RECT, panel);
                    draw_sprite(&list, tex.protect_back.as_deref(), back, at);
                    let (ml, mt, mr, _) = hud::PROTECT_BAR_MARGINS;
                    for row in 0..hud::PROTECT_NAMES.len() {
                        let slot = row + 1;
                        let bar = hud::offset(hud::PROTECT_BAR_RECTS[row], panel);
                        draw_sprite(&list, tex.protect_bar.as_deref(), bar, at);
                        let v = session
                            .battle
                            .get(slot)
                            .map(|c| c.affection)
                            .unwrap_or(0)
                            .clamp(0, hud::PROTECT_BAR_MAX);
                        let (x0, y0) = (bar.0 + ml, bar.1 + mt);
                        let w = (bar.2 - mr - x0) * v / hud::PROTECT_BAR_MAX;
                        if w > 0 {
                            draw_sprite(
                                &list,
                                tex.protect_fill.as_deref(),
                                (x0, y0, x0 + w, y0 + 8),
                                at,
                            );
                        }
                        draw_sprite(
                            &list,
                            tex.protect_ball.as_deref(),
                            (x0 + w - 8, y0 - 4, x0 + w + 8, y0 + 12),
                            at,
                        );
                        let look = if !session.can_be_protected(slot) {
                            2
                        } else {
                            (draft == Some(slot)) as usize
                        };
                        let r = hud::offset(hud::PROTECT_CHECK_RECTS[row], panel);
                        draw_sprite(&list, tex.protect_checks[row][look].as_deref(), r, at);
                        if focus == row {
                            list.add_rect(at(r.0, r.1), at(r.2, r.3), [1.0, 0.85, 0.2, 1.0])
                                .thickness(2.0)
                                .build();
                        }
                    }
                    let ok = (focus == hud::PROTECT_OK_FOCUS) as usize;
                    draw_sprite(
                        &list,
                        tex.protect_ok[ok].as_deref(),
                        hud::offset(hud::PROTECT_OK_RECT, panel),
                        at,
                    );
                }
                Menu::Lineup { cursor, held, .. } => {
                    let panel = hud::LINEUP_PANEL;
                    let back = hud::offset(hud::LINEUP_BACK_RECT, panel);
                    draw_sprite(&list, tex.lineup_back.as_deref(), back, at);
                    for slot in 0..PARTY_SLOTS {
                        if held == Some(slot) {
                            continue;
                        }
                        let Some(c) = session.battle.get(slot).filter(|c| !c.retired) else {
                            continue;
                        };
                        let Some(r) = hud::lineup_face_rect(c.element_position) else {
                            continue;
                        };
                        let dim = !session.can_rearrange(slot) as usize;
                        let face = tex.lineup_faces[slot][dim].as_deref();
                        draw_sprite(&list, face, hud::offset(r, panel), at);
                    }
                    if let Some(r) = hud::lineup_face_rect(cursor) {
                        let r = hud::offset(r, panel);
                        // 抓着的头像跟着光标、放大 1.4 倍（原版跟着鼠标）。
                        if let Some(slot) = held {
                            let (cx, cy) = ((r.0 + r.2) / 2, (r.1 + r.3) / 2);
                            let (hw, hh) = (
                                (hud::LINEUP_FACE.0 as f32 * hud::LINEUP_HELD_SCALE / 2.) as i32,
                                (hud::LINEUP_FACE.1 as f32 * hud::LINEUP_HELD_SCALE / 2.) as i32,
                            );
                            if let Some(s) = tex.lineup_faces[slot][0].as_deref() {
                                let (uv0, uv1) = super::encamp::sprite_uv(s, r);
                                list.add_image(
                                    s.imgui_texture_id(),
                                    at(cx - hw, cy - hh),
                                    at(cx + hw, cy + hh),
                                )
                                .uv_min(uv0)
                                .uv_max(uv1)
                                .build();
                            }
                        }
                        // 光标框是移植端给手柄加的。
                        list.add_rect(at(r.0, r.1), at(r.2, r.3), [1.0, 0.85, 0.2, 1.0])
                            .thickness(2.0)
                            .build();
                    }
                }
                Menu::Ai { focus, draft } => {
                    let panel = hud::AI_PANEL;
                    let back = hud::offset(hud::AI_BACK_RECT, panel);
                    draw_sprite(&list, tex.ai_back.as_deref(), back, at);
                    let name = tex.ai_names.get(turn.slot).and_then(|t| t.as_deref());
                    draw_sprite(&list, name, hud::offset(hud::AI_NAME_RECT, panel), at);
                    for (i, s) in ai::Switch::ALL.iter().enumerate() {
                        let sprite = tex.ai_switches[i][draft.shows(*s) as usize].as_deref();
                        draw_sprite(
                            &list,
                            sprite,
                            hud::offset(hud::AI_SWITCH_RECTS[i], panel),
                            at,
                        );
                    }
                    let ok = (focus == hud::AI_OK_FOCUS) as usize;
                    draw_sprite(
                        &list,
                        tex.ai_ok[ok].as_deref(),
                        hud::offset(hud::AI_OK_RECT, panel),
                        at,
                    );
                    // 勾选框没有「指上去」那张图（原版只认鼠标），手柄焦点移植端描一圈。
                    if focus < hud::AI_OK_FOCUS {
                        let r = hud::offset(hud::AI_SWITCH_RECTS[focus], panel);
                        list.add_rect(at(r.0, r.1), at(r.2, r.3), [1.0, 0.85, 0.2, 1.0])
                            .thickness(2.0)
                            .build();
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
                Menu::Specials { index } => {
                    let rows: Vec<(Vec<String>, bool)> = session
                        .specials(turn.slot)
                        .iter()
                        .enumerate()
                        .map(|(i, s)| {
                            let cells = vec![s.name.clone(), s.effect.sp_cost.to_string()];
                            (cells, session.can_afford_special(turn.slot, i))
                        })
                        .collect();
                    Self::draw_list_in(
                        ui,
                        &list,
                        (tex.skill_back.as_deref(), tex.row_select.as_deref()),
                        hud::SKILL_PANEL,
                        (
                            hud::SKILL_BACK_RECT,
                            hud::SKILL_ROWS,
                            hud::SKILL_VISIBLE_ROWS,
                        ),
                        &hud::SKILL_COLUMNS,
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
                // 目标头顶的 3D 指针在 `place_pointer` 里摆，界面上不另画东西。
                Menu::Target { .. } => {}
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
                    // 六个数（`FUN_0051c330` 建的文字控件 0x202..0x207）：颜色 0x51c5dc 设成
                    // 0xffffffff，字号 `FUN_004427a0(0)` → 16px；`FUN_005184e0` 用
                    // `FUN_004427c0(文字, 0)` 写，不居中——控件渲染 0x442a40 从 rect 左上角
                    // 往里缩内边距 8（构造 `FUN_004422e0` 的 `+0x44`）开始写，不滚动。
                    let base_px = ui.current_font_size();
                    ui.set_window_font_scale(16. * scale / base_px);
                    for (row, v) in p.elements.iter().enumerate() {
                        let r = hud::offset(res::ELEMENT_NUMBER_RECTS[row], panel);
                        list.add_text(at(r.0 + 8, r.1 + 8), [1., 1., 1., 1.], v.to_string());
                    }
                    ui.set_window_font_scale(1.);
                }
            }
            if let Some(win) = player.banner() {
                let big = if win { &tex.big_win } else { &tex.big_lose };
                draw_sprite(&list, big.as_deref(), res::BIG_RECT, at);
            }
            if let Some(m) = player.message() {
                let back = match m.kind {
                    MessageKind::LevelUp => tex.levelup.as_deref(),
                    MessageKind::Mastered | MessageKind::SkillUp => m
                        .row
                        .and_then(|r| tex.element_banners[r as usize].as_deref())
                        .or(tex.levelup.as_deref()),
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
        textures: (Option<&Sprite>, Option<&Sprite>),
        panel: hud::Rect,
        columns: &[i32],
        rows: &[(Vec<String>, bool)],
        index: usize,
        at: &dyn Fn(i32, i32) -> [f32; 2],
        scale: f32,
    ) {
        Self::draw_list_in(
            ui,
            list,
            textures,
            panel,
            (hud::LIST_BACK_RECT, hud::LIST_ROWS, hud::VISIBLE_ROWS),
            columns,
            rows,
            index,
            at,
            scale,
        );
    }

    /// 同 [`Self::draw_list`]，底图 rect、行区、可见行数另给（特技列表和仙术 / 道具的不一样）。
    fn draw_list_in(
        ui: &Ui,
        list: &imgui::DrawListMut<'_>,
        (back, select): (Option<&Sprite>, Option<&Sprite>),
        panel: hud::Rect,
        (back_rect, rows_rect, visible): (hud::Rect, hud::Rect, usize),
        columns: &[i32],
        rows: &[(Vec<String>, bool)],
        index: usize,
        at: &dyn Fn(i32, i32) -> [f32; 2],
        scale: f32,
    ) {
        draw_sprite(list, back, hud::offset(back_rect, panel), at);
        let area = hud::offset(rows_rect, panel);
        let first = hud::first_visible_in(index, rows.len(), visible);
        let _font = game_font(GameFontSize::SMALL).map(|f| ui.push_font(f));
        // 行文字 16 号（`hud::LIST_FONT_PX`，800×600 下的像素）。
        let base_px = ui.current_font_size();
        ui.set_window_font_scale(hud::LIST_FONT_PX * scale / base_px);
        let shown = rows.iter().enumerate().skip(first).take(visible);
        for (line, (cells, enabled)) in shown {
            let color = if *enabled {
                LIST_TEXT
            } else {
                LIST_TEXT_DISABLED
            };
            let (mut x, y) = hud::list_text_at(area, line - first);
            for (cell, width) in cells.iter().zip(columns) {
                list.add_text(at(x, y), color, cell);
                x += width;
            }
        }
        ui.set_window_font_scale(1.);
        // 选中条（UIListBox 渲染 0x43eb66 `FUN_00540c60`，在文字之后画）：整张 SelPic 横向
        // 拉满（u 0..1），顶点色 `+0x498` = (0xa8, 0xa8, 0xa8, 0xc8)，效果 `ui_tex_color.gbf`
        // （SrcAlpha, One）叠加——黑底青光的图叠上去只提亮，不挡字。
        if (first..first + visible).contains(&index) && index < rows.len() {
            if let (Some(r), Some(s)) = (hud::list_select_rect(area, index - first), select) {
                let (uv0, uv1) = super::encamp::sprite_uv(s, r);
                radiance::imgui::push_blend_additive();
                list.add_image(s.imgui_texture_id(), at(r.0, r.1), at(r.2, r.3))
                    .uv_min([0., uv0[1]])
                    .uv_max([1., uv1[1]])
                    .col(argb(hud::LIST_SELECT_COLOR))
                    .build();
                radiance::imgui::push_blend_normal();
            }
        }
    }

    /// 招式名横幅，字居中。
    /// 偷袭横幅（开场时间轴里的 `FUN_004fd290`，3 秒）：敌方偷袭排在最前（0–3 秒，`TricknameBack2.tga`），
    /// 我方偷袭排在两步 4 秒的 `FUN_004fce70` / `FUN_004fcfb0` 之后（8–11 秒，`TricknameBack.tga`）。
    /// 横幅借招式名那一块（CBUI `+0x430` / `+0x434`，都是 (300,30)-(461,87)）。
    fn draw_sneak_banner(ui: &Ui, tex: &HudTextures, sneak: (bool, bool), text: &str, clock: f32) {
        let (back, from) = if sneak.1 {
            (tex.trick_back2.as_deref(), 0.)
        } else if sneak.0 {
            (tex.trick_back.as_deref(), 8.)
        } else {
            return;
        };
        if !(from..from + 3.).contains(&clock) {
            return;
        }
        overlay(ui, "CombatSneakBanner", |ui, at, scale| {
            let list = ui.get_window_draw_list();
            Self::draw_banner(ui, &list, back, text, at, scale);
        });
    }

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
}

impl SceCommand for SceCommandCombat {
    fn initialize(&mut self, scene_manager: ComRc<ISceneManager>, state: &mut SceState) {
        // 打仗时冒险模式整个停住（和对话、动画一样关掉冒险输入）；命令都跑完后 VM 会再打开。
        // 不关的话，没有先放过对话就进战斗时（调试口开战、或脚本一上来就打），冒险导演
        // 还会每帧去挪主角，而栈顶的战斗舞台里没有主角。
        state.global_state_mut().set_adv_input_enabled(false);
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
            // 原版切到模式 15 的状态对象，点一下再 `SwitchMode(4)` 回标题；
            // 移植端走和 147 Switch2Menu 同一条 `request_title`。
            if self.game_over_screen.frame(ui, state, delta_sec) {
                log::info!("Combat: 游戏结束，返回主菜单");
                state.request_title();
                return true;
            }
            return false;
        }

        // 特效 `bink` 类（FUN_004c24b0 → FUN_0053c5a0）：原版是自带消息循环的阻塞全屏播放，
        // 播完才回到战斗，所以播放期间整场战斗停住。Esc / 手柄 A 跳过，和 SCE 播片一样。
        self.movies.extend(state.take_combat_movie_requests());
        if self.movie.is_none() && !self.movies.is_empty() {
            use mini_fs::StoreExt;
            let name = self.movies.remove(0);
            let path = std::path::PathBuf::from("/movie/movie/2deffect")
                .join(&name)
                .with_extension("bik");
            if state.fast_forward() {
                // 快进不播。
            } else if state.asset_mgr().vfs().open(&path).is_err() {
                log::warn!("Combat: 特效 2D 动画 {} 不在包里", path.display());
            } else if let Some(size) = state.global_state_mut().play_movie(&name) {
                log::info!("Combat: 特效 2D 动画 {name}");
                self.movie = Some((size, None));
            }
        }
        if let Some((size, texture)) = self.movie.as_mut() {
            let skipped = state.fast_forward()
                || state
                    .input()
                    .get_key_state(radiance::input::Key::Escape)
                    .pressed()
                || state
                    .input()
                    .get_key_state(radiance::input::Key::GamePadSouth)
                    .pressed();
            let player = state.global_state_mut().video_player();
            if skipped {
                player.stop();
            }
            if skipped || player.get_state() == radiance::video::VideoStreamState::Stopped {
                self.movie = None;
            } else {
                *texture = crate::utils::play_movie(ui, player, *texture, *size, false);
                return false;
            }
        }

        // 片子放完才走钟：原版 bink 是阻塞播放，整个战斗循环停着。
        self.battle_clock += delta_sec;

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
                let Some(s) = self.session.as_mut() else {
                    break;
                };
                s.auto_play();
                outcome = s.tick(0.1);
                if outcome.is_some() {
                    break;
                }
                // 快进时也得把分身出来的怪接出来，不然 `pending_spawns` 一直攒着、
                // 新的那只永远不上场。
                if !s.battle.pending_spawns.is_empty() {
                    self.spawn_pending(state);
                }
            }
            outcome.or(Some(Outcome::Win))
        } else {
            if let Some(input) = Self::poll_input(state) {
                session.input(input);
            }
            for name in std::mem::take(&mut session.sfx) {
                ui_sound::play(state, name);
            }
            session.tick(delta_sec)
        };

        self.spawn_pending(state);
        self.drive_staging(state, delta_sec);
        // 普攻暴击的特效（`FUN_004ce340(0x1d6, 挨打的人的当前位置)`）。
        let effects = self
            .session
            .as_mut()
            .map(|s| std::mem::take(&mut s.effects))
            .unwrap_or_default();
        if !state.fast_forward() {
            for (group, slot) in effects {
                let at = self
                    .actors
                    .get(slot)
                    .and_then(|a| a.as_ref())
                    .map(|a| a.entity.transform().borrow().position());
                if let Some(at) = at {
                    state.play_effect(group, at);
                }
            }
        }
        self.cues.extend(state.take_combat_action_requests());
        self.drive_cues(delta_sec);
        self.animate_stage(delta_sec);
        self.place_pointer();
        if let Some(fx) = self.lineup_ring.as_mut() {
            let (element, panel_open) = self.session.as_ref().map_or((None, false), |s| {
                (
                    s.battle.scene_element,
                    s.turn
                        .as_ref()
                        .is_some_and(|t| matches!(t.menu, Menu::Lineup { .. })),
                )
            });
            fx.update(self.battle_clock, self.sneak.1, element, panel_open);
        }
        // 特效 `camera` 类（adv_director 从 flexeff 转过来）：先停当前序列
        // （FUN_00505800，停在原地），序列号 > 0 再从当前机位播它（FUN_00505770）。
        let mut camera_requests = state.take_combat_camera_requests();
        if let Some(session) = self.session.as_mut() {
            camera_requests.extend(std::mem::take(&mut session.camera_requests));
        }
        for seq in camera_requests {
            let Some(current) = self.camera.as_mut().map(|p| {
                p.stop();
                p.pose
            }) else {
                continue;
            };
            let player = usize::try_from(seq).ok().filter(|s| *s > 0).and_then(|s| {
                self.flexcam.as_ref().and_then(|f| {
                    crate::openpal3::combat::camera::CamPlayer::new(f.sequence(s), current)
                })
            });
            if let Some(player) = player {
                log::info!("Combat: 特效镜头播 flexcam 序列 {seq}");
                self.camera = Some(player);
            }
        }
        // 特效 `quake` 类：镜头每帧叠一个上下偏移（FUN_005054f0），震完那一帧再按原位姿摆一次。
        if let Some(id) = state.take_combat_combo_request() {
            if let Some(session) = self.session.as_mut() {
                if !session.debug_combo(id) {
                    log::warn!("Combat: 调试合击 {id} 没放出去（不是我方回合或没有这条）");
                }
            }
        }
        if let Some([amp, decay, freq]) = state.take_combat_shake_request() {
            self.shake = Some(crate::openpal3::combat::camera::Shake::new(
                amp, decay, freq,
            ));
        }
        let clock = self.battle_clock;
        let shake = self.shake.as_mut().map(|s| s.step(delta_sec, clock));
        if self.shake.is_some_and(|s| s.finished()) {
            self.shake = None;
        }
        if let (Some(player), Some(stage)) = (self.camera.as_mut(), self.stage.as_ref()) {
            let moving = !player.finished();
            let mut pose = if moving {
                player.update(delta_sec)
            } else {
                player.pose
            };
            if let Some(dy) = shake {
                pose.y += dy;
            }
            match player.look_target() {
                // 盯着人：朝他现在站的地方看（记录 +0x65 / +0x68）。
                Some(target) => {
                    let at = usize::try_from(target)
                        .ok()
                        .and_then(|t| self.actors.get(t))
                        .and_then(|a| a.as_ref())
                        .map(|a| a.entity.transform().borrow().position())
                        // -1 = 战场中心（CBStage +0x384..+0x38c）：构造 `FUN_0050fd00` 清零、全程序没人再写，就是原点。
                        .unwrap_or(Vec3::new(0., 0., 0.));
                    Self::place_camera_looking_at(stage, pose, at);
                }
                None if moving || shake.is_some() => Self::place_camera(stage, pose),
                None => {}
            }
        }
        if let Some(player) = self.result.as_ref() {
            // 结算期间战斗界面收起来（`FUN_00518b30(0)` / `FUN_004db940(0)`）。
            Self::draw_result(ui, player, &self.result_panels, self.result_tex.as_ref());
        } else if let (Some(session), Some(hud)) = (self.session.as_ref(), self.hud.as_ref()) {
            Self::draw(ui, session, hud, self.battle_clock);
            if let Some(text) = self.sneak_text.as_deref() {
                Self::draw_sneak_banner(ui, hud, self.sneak, text, self.battle_clock);
            }
        }
        if self.stage_fade > 0. {
            let fade = self.stage_fade;
            overlay(ui, "CombatStageFade", |ui, _, _| {
                let list = ui.get_window_draw_list();
                let [w, h] = ui.io().display_size;
                list.add_rect([0., 0.], [w, h], [0., 0., 0., fade])
                    .filled(true)
                    .build();
            });
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
                // 野外遇敌打赢 / 逃掉（结果码 2）：撞上的那只退场（`FUN_004055d0`：`DAT_00c11754 + 0x278 = 0`）。
                if let Kind::Field { role, .. } = self.kind {
                    if matches!(outcome, Outcome::Win | Outcome::Escaped) {
                        use crate::openpal3::directors::SceneManagerExtensions;
                        if let Some(scn) = scene_manager.scn_scene() {
                            scn.inner::<crate::openpal3::scene::ScnScene>()
                                .retire_field_monster(role);
                        }
                    }
                }
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

/// 战败画面（模式 15：`FUN_0044ffe0` 构造、`FUN_004500f0` 布局，逆向见
/// `switch/re/notes/combat_flow.md` §8）：全屏铺近白色 RGB(0xfd,0xf5,0xf3)，中间贴
/// `ui\GameOver\di.tga`（竖排「胜败乃兵家常事　大侠请重新来过……」），两者 alpha 按
/// `min(t × 0.2, 1)` 一起淡入（`0x450230`）；满 4.5 秒后按一下才回标题（`0x4501a0`，
/// 原版只认鼠标左右键，这里确定键 / A 也算）。
#[derive(Clone, Default)]
pub struct GameOverScreen {
    elapsed: f32,
    texture: Option<Rc<Sprite>>,
    loaded: bool,
}

impl GameOverScreen {
    const FADE_RATE: f32 = 0.2;
    const WAIT: f32 = 4.5;
    const RECT: (i32, i32, i32, i32) = ((800 - 256) / 2, 0, (800 - 256) / 2 + 256, 600);
    const GROUND: [f32; 3] = [0xfd as f32 / 255., 0xf5 as f32 / 255., 0xf3 as f32 / 255.];

    /// 画一帧；返回 true = 该回标题了。
    pub fn frame(&mut self, ui: &Ui, state: &SceState, dt: f32) -> bool {
        if !self.loaded {
            self.loaded = true;
            let am = state.asset_mgr();
            self.texture = am.load_ui_sprite(am.basedata_path().join("ui/GameOver/di.tga"));
        }
        if state.fast_forward() {
            return true;
        }
        let a = (self.elapsed * Self::FADE_RATE).min(1.);
        let texture = self.texture.clone();
        overlay(ui, "GameOver", |ui, at, _scale| {
            let list = ui.get_window_draw_list();
            let [w, h] = ui.io().display_size;
            let [r, g, b] = Self::GROUND;
            list.add_rect([0., 0.], [w, h], [r, g, b, a])
                .filled(true)
                .build();
            if let Some(t) = texture.as_deref() {
                let rect = Self::RECT;
                let (uv0, uv1) = super::encamp::sprite_uv(t, rect);
                list.add_image(t.imgui_texture_id(), at(rect.0, rect.1), at(rect.2, rect.3))
                    .uv_min(uv0)
                    .uv_max(uv1)
                    .col([1., 1., 1., a])
                    .build();
            }
        });
        let ready = self.elapsed >= Self::WAIT;
        self.elapsed += dt;
        let clicked = ui.is_mouse_clicked(imgui::MouseButton::Left)
            || ui.is_mouse_clicked(imgui::MouseButton::Right);
        let input = state.input();
        let confirm = [Key::Space, Key::GamePadEast]
            .iter()
            .any(|k| input.get_key_state(*k).pressed());
        ready && (clicked || confirm)
    }
}

/// 调试口用：只弹战败画面，按一下回标题。
#[derive(Clone, Default)]
pub struct SceCommandGameOverScreen {
    screen: GameOverScreen,
}

impl std::fmt::Debug for SceCommandGameOverScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SceCommandGameOverScreen")
    }
}

impl SceCommand for SceCommandGameOverScreen {
    fn update(
        &mut self,
        _scene_manager: ComRc<ISceneManager>,
        ui: &Ui,
        state: &mut SceState,
        delta_sec: f32,
    ) -> bool {
        if self.screen.frame(ui, state, delta_sec) {
            state.request_title();
            return true;
        }
        false
    }
}

/// 舞台角色包围盒的顶（世界坐标），x/z 取中心：把实体（和子实体）渲染对象的
/// `local_aabb` 并起来，过实体的矩阵（原版 `FUN_0050a560` 取的是 OBB，这里用 AABB 近似）。
fn head_point(entity: &ComRc<radiance::comdef::IEntity>) -> Option<Vec3> {
    fn collect(
        e: &ComRc<radiance::comdef::IEntity>,
        lo: &mut [f32; 3],
        hi: &mut [f32; 3],
        any: &mut bool,
    ) {
        if let Some(rc) = e.get_rendering_component() {
            for h in rc.render_objects() {
                if let Some((a, b)) = h.as_dyn().local_aabb() {
                    for k in 0..3 {
                        lo[k] = lo[k].min(a[k]);
                        hi[k] = hi[k].max(b[k]);
                    }
                    *any = true;
                }
            }
        }
        for c in e.children() {
            collect(&c, lo, hi, any);
        }
    }
    let (mut lo, mut hi, mut any) = ([f32::MAX; 3], [f32::MIN; 3], false);
    collect(entity, &mut lo, &mut hi, &mut any);
    if !any {
        return None;
    }
    let m = *entity.transform().borrow().matrix().floats();
    let c = [(lo[0] + hi[0]) / 2., hi[1], (lo[2] + hi[2]) / 2.];
    let p = |r: usize| m[r][0] * c[0] + m[r][1] * c[1] + m[r][2] * c[2] + m[r][3];
    Some(Vec3::new(p(0), p(1), p(2)))
}
