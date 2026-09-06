//! 战斗外用道具——道具页上点一行、选一个头像之后发生的事。
//!
//! 三支函数各对应原版一个函数，规则见 `switch/re/notes/item_use.md` §1：
//!
//! * [`usable`]         ← `FUN_0049ca70(item, role)`
//! * [`default_target`] ← `FUN_0049c660(sel)`
//! * [`apply`]          ← `FUN_0049cf80(role)`
//!
//! 这里只算数据（`PersistentState` + `GameDb`），要不要关菜单、跑哪个 SCE
//! 过程、放什么音，都通过 [`UseOutcome`] 交给调用方。原版里几处把菜单
//! 的 UI 标志（`+0x28c/28d/28e`）和数据揉在一起的地方，UI 部分留给
//! `item_menu.p7`。

use fileformats::pal3::gdb::GameItemInfo;

use super::attributes::{self, ATTRIBUTE_SLOTS, SLOT_HP, SLOT_MP, SLOT_SP};
use fileformats::pal3::gdb::PLAYER_ACTOR_COUNT;

use super::game_db::GameDb;
use super::persistent_state::{PersistentState, RoleState, STATE_BYTES};

/// 引路蜂：按名字跑当前 `.sce` 里的 `_ylf_<城市>_<场景>`。
pub const ITEM_GUIDE_BEE: u32 = 0x1901;
/// 驱魔香：队长状态 27 辟邪。
pub const ITEM_EXORCISM_INCENSE: u32 = 0x1902;
/// 极目水：队长状态 29 神眼。
pub const ITEM_FAR_SIGHT_WATER: u32 = 0x1903;
/// 狂龙香：队长状态 28 退魔。
pub const ITEM_DRAGON_INCENSE: u32 = 0x1904;
/// 紫幽粉：`DAT_00c10cc1`，不是角色状态。
pub const ITEM_PURPLE_POWDER: u32 = 0x1905;
/// 神炼水：队长状态 30 精进。
pub const ITEM_REFINING_WATER: u32 = 0x1906;
/// 龙精石：`FUN_0049ca70` 开头就返回假，永远用不了。
pub const ITEM_DRAGON_CRYSTAL: u32 = 0x1908;
/// 仙宝页的土灵珠：跑当前 `.sce` 的过程 2101，不消耗。
pub const ITEM_EARTH_PEARL: u32 = 0x191b;
/// 毒龙胆：目标没中毒时把精打到 1。
pub const ITEM_POISON_DRAGON_GALL: u32 = 0x1856;

/// `item_special_type` 5：搜宝鼠。
pub const SPECIAL_TREASURE_MOUSE: u8 = 5;
/// `item_special_type` 7–11：炎宿/雷觉/风源/水境/土隐，各教一门仙术。
pub const SPECIAL_LEARN_FIRST: u8 = 7;
pub const SPECIAL_LEARN_LAST: u8 = 11;
/// `item_special_type` 15：金蛊王，升一级。
pub const SPECIAL_LEVEL_UP: u8 = 15;

/// 土灵珠要跑的过程号（`FUN_00432600(0x835)`）。实机每个迷宫的 `.sce`
/// 里 2101 都叫 `T_mNN`，唯独 M22 没有——和可用判定里的「城市≠M22」对上。
pub const EARTH_PEARL_PROC: u32 = 0x835;

/// 引路蜂过程名（格式串 0x586218）。实机写作 `_YLF_m01_1`，查名不分大小写。
pub fn guide_bee_proc_name(city: &str, scene: &str) -> String {
    format!("_ylf_{city}_{scene}")
}

/// special 7–11 各教哪门仙术（`FUN_0049cf80` 的五个分支写死的常量）。
pub fn skill_for_special(special: u8) -> Option<u32> {
    Some(match special {
        7 => 0x132,
        8 => 0x137,
        9 => 0x12d,
        10 => 0x13c,
        11 => 0x141,
        _ => return None,
    })
}

/// 状态字节里的五毒下标。
const POISON_COUNT: usize = 5;
/// 通用支从这个下标起把物品的状态码原样拷进角色状态字节。
const FIRST_COPIED_STATE: usize = 13;
/// `FUN_00410580(k)` 接受的四个队长状态 + 紫幽粉。
const MAZE_STATE_EXORCISM: usize = 27;
const MAZE_STATE_DISPEL: usize = 28;
const MAZE_STATE_SIGHT: usize = 29;
const MAZE_STATE_ADVANCE: usize = 30;

/// 判定和施用要看的场景侧事实。由服务端每帧从导演那里取。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UseContext {
    /// `DAT_00d8bbec == 2`。
    pub in_maze: bool,
    /// `FUN_004235b0("M22", 0)`：当前城市是 M22。
    pub city_is_m22: bool,
    /// 当前 `.sce` 里有过程 2101。
    pub proc_2101_exists: bool,
    /// 场景里有 type 0x18 且激活的对象（搜宝鼠找得到东西）。
    pub treasure_present: bool,
}

/// [`default_target`] 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPick {
    /// 全队 potential 为 0（−2）。
    None,
    /// range 不是 1/2（−3）。
    Invalid,
    /// range 1：默认目标；箭头只在 `arrow` 为真时画在他头上。
    Single { actor_id: u32, arrow: bool },
    /// range 2：所有队员亮箭头（条件只查了 `checked` 一个人）。
    All { checked: u32, arrow: bool },
}

/// [`apply`] 之后调用方该做的事。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UseOutcome {
    /// 守卫没过（range 1 且目标不可用，或 range 不是 1/2）。什么都没变。
    Rejected,
    /// 效果已写进存档态，物品按规则扣掉；菜单留在原地。
    Applied,
    /// 引路蜂：跑这个名字的过程，然后关菜单。
    RunProcByName(String),
    /// 土灵珠：跑这个过程，然后关菜单。没有该过程时返回 [`Self::Applied`]
    /// 且不扣数量（原版就是空转一圈）。
    RunProc(u32),
    /// 搜宝鼠：关菜单。`found` 真 → 特效，假 → 放 "XMouse"。
    TreasureMouse { found: bool },
    /// 炎宿等：学会了这门仙术。
    LearnedSkill(u32),
    /// 金蛊王：升了一级。
    LevelUp,
}

/// `FUN_0049ca70(item, role)`：这件东西对这个人现在能不能用。
pub fn usable(
    item: &GameItemInfo,
    state: &PersistentState,
    actor_id: u32,
    db: &GameDb,
    ctx: &UseContext,
) -> bool {
    let Some(role) = state.role(actor_id) else {
        return false;
    };
    match item.id {
        ITEM_DRAGON_CRYSTAL => return false,
        ITEM_EARTH_PEARL => return !ctx.city_is_m22 && ctx.in_maze,
        ITEM_GUIDE_BEE | ITEM_PURPLE_POWDER => return ctx.in_maze,
        _ => {}
    }
    match item.item_special_type {
        SPECIAL_TREASURE_MOUSE => return true,
        SPECIAL_LEARN_FIRST..=SPECIAL_LEARN_LAST => {
            return can_learn(item.item_special_type, role, actor_id, db);
        }
        SPECIAL_LEVEL_UP => return true,
        _ => {}
    }
    if (MAZE_STATE_EXORCISM..=MAZE_STATE_ADVANCE).any(|k| item.combat_state_impact_value[k] != 0) {
        return true;
    }

    let eff = attributes::effective(role, db);
    let mut need = item
        .attribute_impact_type
        .iter()
        .filter(|t| **t == 3)
        .count() as i32;
    for slot in 0..ATTRIBUTE_SLOTS {
        if item.attribute_impact_value[slot] > 0 {
            let (cur, max) = current_and_max(role, &eff, slot);
            need += max - cur;
        }
    }
    for p in 0..POISON_COUNT {
        if item.combat_state_impact_value[p] != 0 {
            need += role.states[p] as i32;
        }
    }
    need > 0
}

/// special 7–11 的可学判定：仙术的可用者位非零，且还没学。
fn can_learn(special: u8, role: &RoleState, actor_id: u32, db: &GameDb) -> bool {
    let Some(skill_id) = skill_for_special(special) else {
        return false;
    };
    let Some(skill) = db.skill(skill_id) else {
        return false;
    };
    let allowed = skill
        .applicable_actors
        .get(actor_id as usize)
        .is_some_and(|a| *a != 0);
    allowed && !role.skills.iter().any(|s| s.id == skill_id)
}

/// 12 项属性的「当前 / 上限」。原版角色记录里 12 项都有两组银行；移植端
/// 只有精气神分当前和上限，武防速运五灵是单值，当前恒等于上限。
fn current_and_max(role: &RoleState, eff: &attributes::EffectiveStats, slot: usize) -> (i32, i32) {
    match slot {
        SLOT_HP => (role.hp, eff.max_hp),
        SLOT_SP => (role.sp, eff.max_sp),
        SLOT_MP => (role.mp, eff.max_mp),
        _ => {
            let v = single_value(eff, slot);
            (v, v)
        }
    }
}

fn single_value(eff: &attributes::EffectiveStats, slot: usize) -> i32 {
    match slot {
        3 => eff.attack,
        4 => eff.defense,
        5 => eff.speed,
        6 => eff.luck,
        7..=11 => eff.elements[slot - 7],
        _ => 0,
    }
}

/// 队内成员，按角色号升序——原版 `+0x26d+i` 那 5 个在队位就是这样排的。
fn party(state: &PersistentState) -> impl Iterator<Item = u32> + '_ {
    (0..PLAYER_ACTOR_COUNT as u32).filter(move |a| state.is_in_team(*a))
}

/// `FUN_0049c660`：点了一行之后默认把箭头落在谁头上。
///
/// 每个人的 `need` 和全队的 `potential` 按原版的双重循环算：**毒的贡献在
/// 12 次属性循环里各加一次**，也就是乘了 12，照抄。
pub fn default_target(
    item: &GameItemInfo,
    state: &PersistentState,
    db: &GameDb,
    ctx: &UseContext,
) -> TargetPick {
    let mut needs: Vec<(u32, i32)> = Vec::new();
    let mut potential = 0i32;
    for actor_id in party(state) {
        let Some(role) = state.role(actor_id) else {
            continue;
        };
        let eff = attributes::effective(role, db);
        let mut need = 0i32;
        for slot in 0..ATTRIBUTE_SLOTS {
            let value = item.attribute_impact_value[slot] as i32;
            if value > 0 {
                let (cur, max) = current_and_max(role, &eff, slot);
                need += max - cur;
                potential += value;
            }
            for p in 0..POISON_COUNT {
                let code = item.combat_state_impact_value[p] as i32;
                if code != 0 {
                    need += role.states[p] as i32;
                    potential += code;
                }
            }
        }
        needs.push((actor_id, need));
    }
    if potential == 0 || needs.is_empty() {
        return TargetPick::None;
    }

    // 第一个 need>0 的人；再用 <= 取最大（并列取靠后）；若最大者 need
    // 为 0 → 0 号位；若最大者和首个正数一样多 → 首个正数。
    let first_positive = needs.iter().position(|(_, n)| *n > 0);
    let mut target = 0usize;
    for (i, (_, n)) in needs.iter().enumerate() {
        if needs[target].1 <= *n {
            target = i;
        }
    }
    if needs[target].1 == 0 {
        target = 0;
    }
    if let Some(fp) = first_positive {
        if needs[target].1 == needs[fp].1 {
            target = fp;
        }
    }
    let actor_id = needs[target].0;
    let arrow = usable(item, state, actor_id, db, ctx);
    match item.target_range_type {
        1 => TargetPick::Single { actor_id, arrow },
        2 => TargetPick::All {
            checked: actor_id,
            arrow,
        },
        _ => TargetPick::Invalid,
    }
}

/// `FUN_0049cf80(role)`：对 `actor_id` 施用 `item`。
///
/// 数量的增减也在这里做：除土灵珠外都扣 1（`FUN_0040cf60`），没有该物品时
/// 直接 [`UseOutcome::Rejected`]。
pub fn apply(
    item: &GameItemInfo,
    state: &mut PersistentState,
    actor_id: u32,
    db: &GameDb,
    ctx: &UseContext,
) -> UseOutcome {
    if state.item_count(item.id) == 0 || state.role(actor_id).is_none() {
        return UseOutcome::Rejected;
    }
    let range = item.target_range_type;
    if range != 1 && range != 2 {
        return UseOutcome::Rejected;
    }
    if range == 1 && !usable(item, state, actor_id, db, ctx) {
        return UseOutcome::Rejected;
    }

    let outcome = match item.id {
        ITEM_EXORCISM_INCENSE => {
            state.set_leader_maze_state(MAZE_STATE_EXORCISM);
            UseOutcome::Applied
        }
        ITEM_DRAGON_INCENSE => {
            state.set_leader_maze_state(MAZE_STATE_DISPEL);
            UseOutcome::Applied
        }
        ITEM_FAR_SIGHT_WATER => {
            state.set_leader_maze_state(MAZE_STATE_SIGHT);
            UseOutcome::Applied
        }
        ITEM_REFINING_WATER => {
            state.set_leader_maze_state(MAZE_STATE_ADVANCE);
            UseOutcome::Applied
        }
        ITEM_PURPLE_POWDER => {
            state.set_purple_powder();
            UseOutcome::Applied
        }
        ITEM_GUIDE_BEE => {
            let city = state.scene_name().unwrap_or_default();
            let scene = state.sub_scene_name().unwrap_or_default();
            UseOutcome::RunProcByName(guide_bee_proc_name(&city, &scene))
        }
        ITEM_EARTH_PEARL => {
            if ctx.proc_2101_exists {
                UseOutcome::RunProc(EARTH_PEARL_PROC)
            } else {
                UseOutcome::Applied
            }
        }
        ITEM_POISON_DRAGON_GALL => {
            apply_poison_dragon_gall(item, state, actor_id);
            UseOutcome::Applied
        }
        _ => match item.item_special_type {
            SPECIAL_TREASURE_MOUSE => UseOutcome::TreasureMouse {
                found: ctx.treasure_present,
            },
            SPECIAL_LEARN_FIRST..=SPECIAL_LEARN_LAST => {
                // range 1 已经要求 usable，所以到这里一定可学。
                let skill_id = skill_for_special(item.item_special_type).unwrap_or(0);
                if state.learn_skill(actor_id, skill_id) {
                    UseOutcome::LearnedSkill(skill_id)
                } else {
                    UseOutcome::Applied
                }
            }
            SPECIAL_LEVEL_UP => {
                let level = state.role(actor_id).map(|r| r.level).unwrap_or(1);
                state.gain_level(actor_id, level + 1);
                clamp_current(state, actor_id, db);
                UseOutcome::LevelUp
            }
            _ if range == 1 => {
                apply_generic(item, state, actor_id, db, true);
                UseOutcome::Applied
            }
            _ => {
                let members: Vec<u32> = party(state).collect();
                for member in members {
                    if usable(item, state, member, db, ctx) {
                        apply_generic(item, state, member, db, false);
                    }
                }
                UseOutcome::Applied
            }
        },
    };

    if item.id != ITEM_EARTH_PEARL {
        state.remove_item(item.id, 1);
    }
    outcome
}

/// 通用支：12 项属性、五毒解除、状态码拷贝，再钳位。
///
/// `single` 区分两条只差一处的写法：单体支的百分比是浮点
/// `ROUND(value × 0.01 × max)`，全体支是先整除再乘 `ROUND((value / 100) × max)`。
fn apply_generic(
    item: &GameItemInfo,
    state: &mut PersistentState,
    actor_id: u32,
    db: &GameDb,
    single: bool,
) {
    let eff = state
        .role(actor_id)
        .map(|r| attributes::effective(r, db))
        .unwrap_or_default();
    let Some(role) = state.role_mut(actor_id) else {
        return;
    };
    for slot in 0..ATTRIBUTE_SLOTS {
        let value = item.attribute_impact_value[slot] as i32;
        match item.attribute_impact_type[slot] {
            1 => {
                let (_, max) = current_and_max(&*role, &eff, slot);
                let delta = if single {
                    (value as f32 * 0.01 * max as f32).round() as i32
                } else {
                    ((value / 100) as f32 * max as f32).round() as i32
                };
                add_current(role, slot, delta);
            }
            3 => add_base(role, slot, value),
            _ => add_current(role, slot, value),
        }
    }
    for p in 0..POISON_COUNT {
        if item.combat_state_impact_value[p] > 1 {
            role.states[p] = 0;
        }
    }
    for k in FIRST_COPIED_STATE..STATE_BYTES {
        role.states[k] = item.combat_state_impact_value[k] as u8;
    }
    clamp_current(state, actor_id, db);
}

/// 毒龙胆（0x1856）：物品码非零的那几种毒一个都没中 → 精 = 1；否则清掉
/// 五个毒字节。
fn apply_poison_dragon_gall(item: &GameItemInfo, state: &mut PersistentState, actor_id: u32) {
    let Some(role) = state.role_mut(actor_id) else {
        return;
    };
    let poisoned: i32 = (0..POISON_COUNT)
        .filter(|p| item.combat_state_impact_value[*p] != 0)
        .map(|p| role.states[p] as i32)
        .sum();
    if poisoned == 0 {
        role.hp = 1;
    } else {
        for p in 0..POISON_COUNT {
            role.states[p] = 0;
        }
    }
}

/// 往当前值银行加。移植端只有精气神有当前值；其它槽的当前值等于上限，
/// 加了也会被随后的钳位抹平，所以不做。
fn add_current(role: &mut RoleState, slot: usize, delta: i32) {
    match slot {
        SLOT_HP => role.hp += delta,
        SLOT_SP => role.sp += delta,
        SLOT_MP => role.mp += delta,
        _ => {}
    }
}

/// 往基值银行加（type 3「加上限」）。
fn add_base(role: &mut RoleState, slot: usize, delta: i32) {
    match slot {
        SLOT_HP => role.max_hp += delta,
        SLOT_SP => role.max_sp += delta,
        SLOT_MP => role.max_mp += delta,
        3 => role.attack += delta,
        4 => role.defense += delta,
        5 => role.speed += delta,
        6 => role.luck += delta,
        7..=11 => role.elements[slot - 7] += delta,
        _ => {}
    }
}

/// `FUN_0040be80(0)` 之后那 12 次钳位：`cur < max ? (cur < 1 → 0) : max`。
fn clamp_current(state: &mut PersistentState, actor_id: u32, db: &GameDb) {
    let Some(role) = state.role(actor_id) else {
        return;
    };
    let eff = attributes::effective(role, db);
    let Some(role) = state.role_mut(actor_id) else {
        return;
    };
    let clamp = |cur: &mut i32, max: i32| {
        if *cur < max {
            if *cur < 1 {
                *cur = 0;
            }
        } else {
            *cur = max;
        }
    };
    clamp(&mut role.hp, eff.max_hp);
    clamp(&mut role.sp, eff.max_sp);
    clamp(&mut role.mp, eff.max_mp);
}

/// 用完一件之后菜单要不要留在「选人」态继续用：还有存货，且还有人能用。
/// 对应收尾处的 `+0x28c` 连用检查。
pub fn can_continue(
    item: &GameItemInfo,
    state: &PersistentState,
    db: &GameDb,
    ctx: &UseContext,
) -> bool {
    state.item_count(item.id) > 0 && party(state).any(|a| usable(item, state, a, db, ctx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use fileformats::pal3::gdb::SkillInfo;

    const HP_MAX: i32 = 500;

    fn heal(id: u32, hp: i16) -> GameItemInfo {
        let mut item = GameItemInfo::blank(id);
        item.item_type = 1;
        item.target_range_type = 1;
        item.attribute_impact_value[SLOT_HP] = hp;
        item
    }

    fn setup(items: Vec<GameItemInfo>) -> (PersistentState, GameDb) {
        let mut skill = SkillInfo::blank(0x132);
        skill.applicable_actors = [1, 0, 0, 0, 0];
        let db = GameDb::from_tables(items, vec![skill]);
        let mut state = PersistentState::new("test".to_string());
        for actor in 0..2u32 {
            let mut role = super::super::persistent_state::growth_test_role();
            role.max_hp = HP_MAX;
            role.hp = HP_MAX;
            role.max_sp = 100;
            role.sp = 100;
            role.max_mp = 100;
            role.mp = 100;
            state.insert_role(actor, role);
            state.add_to_team(actor);
        }
        (state, db)
    }

    #[test]
    fn a_heal_is_only_usable_on_someone_hurt() {
        let item = heal(6201, 100);
        let (mut state, db) = setup(vec![]);
        let ctx = UseContext::default();
        assert!(!usable(&item, &state, 0, &db, &ctx), "满血不可用");
        state.role_mut(0).unwrap().hp = 400;
        assert!(usable(&item, &state, 0, &db, &ctx));
        assert!(!usable(&item, &state, 1, &db, &ctx), "另一个人还是满的");
    }

    #[test]
    fn maze_only_items_follow_the_scene_type() {
        let mut bee = GameItemInfo::blank(ITEM_GUIDE_BEE);
        bee.item_special_type = 3;
        bee.target_range_type = 2;
        let mut pearl = GameItemInfo::blank(ITEM_EARTH_PEARL);
        pearl.item_special_type = 3;
        pearl.target_range_type = 2;
        let (state, db) = setup(vec![]);
        let town = UseContext::default();
        let maze = UseContext {
            in_maze: true,
            ..Default::default()
        };
        assert!(!usable(&bee, &state, 0, &db, &town));
        assert!(usable(&bee, &state, 0, &db, &maze));
        assert!(usable(&pearl, &state, 0, &db, &maze));
        let m22 = UseContext {
            in_maze: true,
            city_is_m22: true,
            ..Default::default()
        };
        assert!(!usable(&pearl, &state, 0, &db, &m22), "M22 里土灵珠不可用");
    }

    #[test]
    fn default_target_prefers_the_most_hurt_and_ties_go_to_the_first() {
        let item = heal(6201, 100);
        let (mut state, db) = setup(vec![]);
        let ctx = UseContext::default();
        state.role_mut(0).unwrap().hp = 450;
        state.role_mut(1).unwrap().hp = 300;
        assert_eq!(
            default_target(&item, &state, &db, &ctx),
            TargetPick::Single {
                actor_id: 1,
                arrow: true
            }
        );
        state.role_mut(1).unwrap().hp = 450;
        assert_eq!(
            default_target(&item, &state, &db, &ctx),
            TargetPick::Single {
                actor_id: 0,
                arrow: true
            }
        );
        // 没人受伤：目标是 0 号，但箭头不亮。
        state.role_mut(0).unwrap().hp = HP_MAX;
        state.role_mut(1).unwrap().hp = HP_MAX;
        assert_eq!(
            default_target(&item, &state, &db, &ctx),
            TargetPick::Single {
                actor_id: 0,
                arrow: false
            }
        );
    }

    #[test]
    fn an_item_without_any_positive_value_has_no_default_target() {
        // 6220 烟月神镜：只加状态 19 镜，属性全 0。
        let mut mirror = GameItemInfo::blank(6220);
        mirror.item_type = 1;
        mirror.target_range_type = 1;
        mirror.combat_state_impact_value[19] = 1;
        let (state, db) = setup(vec![]);
        assert_eq!(
            default_target(&mirror, &state, &db, &UseContext::default()),
            TargetPick::None
        );
    }

    #[test]
    fn poison_counts_twelve_times_in_the_default_target_weighting() {
        // 6225 冰蟾子解火毒：一个人中了火毒（need 12），另一个人掉了 10 点血。
        // 冰蟾子本身没有正属性，所以借一件同时回血 1 又解毒的假物品来比。
        let mut item = heal(9999, 1);
        item.combat_state_impact_value[3] = 2;
        let (mut state, db) = setup(vec![]);
        state.role_mut(0).unwrap().hp = HP_MAX - 10;
        state.role_mut(1).unwrap().states[3] = 1;
        let ctx = UseContext::default();
        assert_eq!(
            default_target(&item, &state, &db, &ctx),
            TargetPick::Single {
                actor_id: 1,
                arrow: true
            }
        );
    }

    #[test]
    fn a_single_target_heal_adds_and_clamps_and_consumes_one() {
        let item = heal(6201, 100);
        let (mut state, db) = setup(vec![]);
        state.add_item(6201, 2);
        state.role_mut(0).unwrap().hp = 450;
        let ctx = UseContext::default();
        assert_eq!(apply(&item, &mut state, 0, &db, &ctx), UseOutcome::Applied);
        assert_eq!(state.role(0).unwrap().hp, HP_MAX, "450 + 100 钳到上限");
        assert_eq!(state.item_count(6201), 1);
        // 满血再用：守卫拦下，不扣数量。
        assert_eq!(apply(&item, &mut state, 0, &db, &ctx), UseOutcome::Rejected);
        assert_eq!(state.item_count(6201), 1);
    }

    #[test]
    fn percentage_heals_round_differently_for_single_and_party() {
        // 单体：ROUND(10 × 0.01 × 500) = 50；全体：ROUND((10 / 100) × 500) = 0。
        let mut item = heal(6211, 10);
        item.attribute_impact_type[SLOT_HP] = 1;
        let (mut state, db) = setup(vec![]);
        state.add_item(6211, 5);
        state.role_mut(0).unwrap().hp = 100;
        let ctx = UseContext::default();
        apply(&item, &mut state, 0, &db, &ctx);
        assert_eq!(state.role(0).unwrap().hp, 150);

        item.target_range_type = 2;
        state.role_mut(0).unwrap().hp = 100;
        apply(&item, &mut state, 0, &db, &ctx);
        assert_eq!(state.role(0).unwrap().hp, 100, "全体支先整除，10/100 = 0");
        assert_eq!(state.item_count(6211), 3);
    }

    #[test]
    fn the_copied_state_codes_overwrite_slots_13_to_30() {
        // 6201 止血草：精 +100，码[23 濒死] = 2。
        let mut item = heal(6201, 100);
        item.combat_state_impact_value[23] = 2;
        let (mut state, db) = setup(vec![]);
        state.add_item(6201, 1);
        {
            let role = state.role_mut(0).unwrap();
            role.hp = 400;
            role.states[19] = 1;
        }
        let ctx = UseContext::default();
        apply(&item, &mut state, 0, &db, &ctx);
        let role = state.role(0).unwrap();
        assert_eq!(role.states[23], 2, "码原样拷进去");
        assert_eq!(role.states[19], 0, "没写的位被清零");
    }

    #[test]
    fn a_pure_state_cure_has_nothing_to_need_outside_combat() {
        // 6214 脉络散只带码[13 定] = 2，没有正属性：need 恒为 0，用不了。
        let mut item = heal(6214, 0);
        item.combat_state_impact_value[13] = 2;
        let (mut state, db) = setup(vec![]);
        state.add_item(6214, 1);
        state.role_mut(0).unwrap().states[13] = 1;
        let ctx = UseContext::default();
        assert!(!usable(&item, &state, 0, &db, &ctx));
        assert_eq!(apply(&item, &mut state, 0, &db, &ctx), UseOutcome::Rejected);
    }

    #[test]
    fn poison_dragon_gall_cures_or_punishes() {
        let mut gall = GameItemInfo::blank(ITEM_POISON_DRAGON_GALL);
        gall.item_type = 1;
        gall.target_range_type = 1;
        for p in 0..5 {
            gall.combat_state_impact_value[p] = 2;
        }
        let (mut state, db) = setup(vec![]);
        state.add_item(ITEM_POISON_DRAGON_GALL, 2);
        state.role_mut(0).unwrap().states[2] = 1;
        let ctx = UseContext::default();
        assert!(usable(&gall, &state, 0, &db, &ctx));
        apply(&gall, &mut state, 0, &db, &ctx);
        assert_eq!(state.role(0).unwrap().states, [0; STATE_BYTES]);
        assert_eq!(state.role(0).unwrap().hp, HP_MAX);
        // 没中毒的人用不了（need 为 0），所以「精打到 1」只能从 range 守卫外
        // 直接调到 —— 用一件 range 2 的复制品验证那支。
        gall.target_range_type = 2;
        apply(&gall, &mut state, 1, &db, &ctx);
        assert_eq!(state.role(1).unwrap().hp, 1);
    }

    #[test]
    fn maze_incense_marks_the_leader_and_purple_powder_is_global() {
        let mut incense = GameItemInfo::blank(ITEM_EXORCISM_INCENSE);
        incense.item_type = 3;
        incense.target_range_type = 2;
        incense.combat_state_impact_value[27] = 1;
        let mut powder = GameItemInfo::blank(ITEM_PURPLE_POWDER);
        powder.item_type = 3;
        powder.item_special_type = 4;
        powder.target_range_type = 2;
        let (mut state, db) = setup(vec![]);
        state.add_item(ITEM_EXORCISM_INCENSE, 1);
        state.add_item(ITEM_PURPLE_POWDER, 1);
        let ctx = UseContext {
            in_maze: true,
            ..Default::default()
        };
        assert!(
            usable(&incense, &state, 1, &db, &UseContext::default()),
            "码 27 非零就可用"
        );
        apply(&incense, &mut state, 1, &db, &ctx);
        assert_eq!(state.role(0).unwrap().states[27], 1, "写的是队长");
        assert_eq!(state.role(1).unwrap().states[27], 0);
        apply(&powder, &mut state, 0, &db, &ctx);
        assert!(state.purple_powder());
        assert_eq!(state.item_count(ITEM_PURPLE_POWDER), 0);
        // 60 s 后一起清掉。
        state.advance_play_time(61.);
        assert!(!state.purple_powder());
        assert_eq!(state.role(0).unwrap().states[27], 0);
    }

    #[test]
    fn guide_bee_and_earth_pearl_hand_a_proc_back() {
        let mut bee = GameItemInfo::blank(ITEM_GUIDE_BEE);
        bee.item_special_type = 3;
        bee.target_range_type = 2;
        let mut pearl = GameItemInfo::blank(ITEM_EARTH_PEARL);
        pearl.item_special_type = 3;
        pearl.target_range_type = 2;
        let (mut state, db) = setup(vec![]);
        state.add_item(ITEM_GUIDE_BEE, 1);
        state.add_item(ITEM_EARTH_PEARL, 1);
        state.set_scene_name("m01".to_string(), "1".to_string());
        let ctx = UseContext {
            in_maze: true,
            proc_2101_exists: true,
            ..Default::default()
        };
        assert_eq!(
            apply(&bee, &mut state, 0, &db, &ctx),
            UseOutcome::RunProcByName("_ylf_m01_1".to_string())
        );
        assert_eq!(state.item_count(ITEM_GUIDE_BEE), 0);
        assert_eq!(
            apply(&pearl, &mut state, 0, &db, &ctx),
            UseOutcome::RunProc(EARTH_PEARL_PROC)
        );
        assert_eq!(state.item_count(ITEM_EARTH_PEARL), 1, "土灵珠不消耗");
        let no_proc = UseContext {
            proc_2101_exists: false,
            ..ctx
        };
        assert_eq!(
            apply(&pearl, &mut state, 0, &db, &no_proc),
            UseOutcome::Applied
        );
    }

    #[test]
    fn a_skill_book_teaches_once_and_only_the_right_person() {
        let mut book = GameItemInfo::blank(6409);
        book.item_type = 3;
        book.item_special_type = 7;
        book.target_range_type = 1;
        let (mut state, db) = setup(vec![]);
        state.add_item(6409, 2);
        let ctx = UseContext::default();
        assert!(usable(&book, &state, 0, &db, &ctx));
        assert!(!usable(&book, &state, 1, &db, &ctx), "1 号不在可学位上");
        assert_eq!(
            apply(&book, &mut state, 0, &db, &ctx),
            UseOutcome::LearnedSkill(0x132)
        );
        assert!(!usable(&book, &state, 0, &db, &ctx), "学过了");
        assert_eq!(apply(&book, &mut state, 0, &db, &ctx), UseOutcome::Rejected);
        assert_eq!(state.item_count(6409), 1);
    }

    #[test]
    fn the_level_up_item_raises_one_level_without_healing() {
        let mut item = GameItemInfo::blank(6426);
        item.item_type = 3;
        item.item_special_type = SPECIAL_LEVEL_UP;
        item.target_range_type = 1;
        let (mut state, db) = setup(vec![]);
        state.add_item(6426, 1);
        state.role_mut(0).unwrap().hp = 200;
        let ctx = UseContext::default();
        assert_eq!(apply(&item, &mut state, 0, &db, &ctx), UseOutcome::LevelUp);
        let role = state.role(0).unwrap();
        assert_eq!(role.level, 2);
        assert_eq!(role.hp, 200, "不回血");
        assert!(role.max_hp > HP_MAX || role.max_hp == HP_MAX);
    }

    #[test]
    fn a_max_raising_item_changes_the_base_not_the_current() {
        let mut item = GameItemInfo::blank(6414);
        item.item_type = 3;
        item.item_special_type = 12;
        item.target_range_type = 1;
        item.attribute_impact_type[SLOT_HP] = 3;
        item.attribute_impact_value[SLOT_HP] = 10;
        let (mut state, db) = setup(vec![]);
        state.add_item(6414, 1);
        let ctx = UseContext::default();
        assert!(
            usable(&item, &state, 0, &db, &ctx),
            "type 3 本身就算一次 need"
        );
        apply(&item, &mut state, 0, &db, &ctx);
        let role = state.role(0).unwrap();
        assert_eq!(role.max_hp, HP_MAX + 10);
        assert_eq!(role.hp, HP_MAX);
    }

    #[test]
    fn continue_needs_stock_and_someone_left_to_use_it_on() {
        let item = heal(6201, 100);
        let (mut state, db) = setup(vec![]);
        let ctx = UseContext::default();
        state.add_item(6201, 1);
        state.role_mut(0).unwrap().hp = 400;
        assert!(can_continue(&item, &state, &db, &ctx));
        state.role_mut(0).unwrap().hp = HP_MAX;
        assert!(!can_continue(&item, &state, &db, &ctx));
    }
}
