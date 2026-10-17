//! 战斗外施法——仙术页上双击一门仙术、再双击一个头像之后发生的事（参考源码
//! `gamemainui/magicui.cpp`，exe 里 `CheckUsable` 是 `FUN_0048c610`，逐段对得上）。
//!
//! * [`tab_of`]       ← `GetRoleSkill`：一门仙术落在哪个页签；
//! * [`can_use_this`] ← `CanUseThis`：这个人有没有用；
//! * [`check_usable`] ← `CheckUsable`：能不能开始施（不能就给提示的语言表号）；
//! * [`need_to`]      ← `CheckNeedTo`：箭头落在谁头上；
//! * [`cast`]         ← `UsingMagicRole` + `UpdateMagicLV`：施出去、记次数、升级。
//!
//! 只算数据；提示框、音效、选人状态由调用方按返回值处理。

use fileformats::pal3::gdb::{PLAYER_ACTOR_COUNT, SkillInfo};

use super::attributes::{self, ATTRIBUTE_SLOTS, SLOT_HP, SLOT_MP, SLOT_SP};
use super::game_db::GameDb;
use super::item_use::clamp_current;
use super::persistent_state::{PersistentState, RoleState, STATE_BYTES};

/// 页签：技、水、火、雷、风、土、双、合（屏幕上的顺序）。
pub const TAB_SKILL: usize = 0;
pub const TAB_DUAL: usize = 6;

/// `FightState` 里五毒的下标：风、雷、水、火、土。
const POISONS: [usize; 5] = [0, 1, 2, 3, 4];
/// 单体支不拷的状态：`FS_FENGDU`..`FS_SU_SUB`。
const SINGLE_SKIP: std::ops::RangeInclusive<usize> = 0..=12;
/// 全体支不拷的状态：`FS_WU_ADD`..`FS_SU_SUB`——五毒的码原样拷过去。
const ALL_SKIP: std::ops::RangeInclusive<usize> = 5..=12;

/// 数据库 `element_attributes` 的下标（水火风雷土）。
const EL_WATER: usize = 0;
const EL_FIRE: usize = 1;
const EL_WIND: usize = 2;
const EL_THUNDER: usize = 3;
const EL_EARTH: usize = 4;

/// `GetRoleSkill`：`actor` 学会的这门仙术列在哪个页签（`user[actor]` 为 0 的不列）。特技（type 0）
/// 进「技」；仙术里风雷水火土某一个非 0、其余四个都不是 1 的进那一页（判断次序风、雷、水、火、土），
/// 其余进「双」。
pub fn tab_of(skill: &SkillInfo, actor: usize) -> Option<usize> {
    if skill.applicable_actors.get(actor).copied().unwrap_or(0) == 0 {
        return None;
    }
    if skill.skill_type == 0 {
        return Some(TAB_SKILL);
    }
    let a = &skill.element_attributes;
    let only = |k: usize| a[k] != 0 && (0..5).filter(|&o| o != k).all(|o| a[o] != 1);
    Some(if only(EL_WIND) {
        4
    } else if only(EL_THUNDER) {
        3
    } else if only(EL_WATER) {
        1
    } else if only(EL_FIRE) {
        2
    } else if only(EL_EARTH) {
        5
    } else {
        TAB_DUAL
    })
}

fn team(state: &PersistentState) -> impl Iterator<Item = u32> + '_ {
    (0..PLAYER_ACTOR_COUNT as u32).filter(move |a| state.is_in_team(*a))
}

/// 这个人这一项离上限还差多少（只有精气神有当前值）。
fn deficit(role: &RoleState, eff: &attributes::EffectiveStats, slot: usize) -> i32 {
    match slot {
        SLOT_HP => eff.max_hp - role.hp,
        SLOT_SP => eff.max_sp - role.sp,
        SLOT_MP => eff.max_mp - role.mp,
        _ => 0,
    }
}

fn poisoned(skill: &SkillInfo, role: &RoleState) -> i32 {
    POISONS
        .iter()
        .filter(|&&p| skill.combat_state_impact_types[p] != 0)
        .map(|&p| role.states[p] as i32)
        .sum()
}

/// `CanUseThis`：仙术加的那几项这个人有缺、或者中了它解的毒。
pub fn can_use_this(skill: &SkillInfo, state: &PersistentState, db: &GameDb, actor: u32) -> bool {
    let Some(role) = state.role(actor) else {
        return false;
    };
    let eff = attributes::effective(role, db, state.magic_sword());
    let mut need = 0;
    for slot in 0..ATTRIBUTE_SLOTS {
        if skill.attribute_impact_value[slot] > 0 {
            need += deficit(role, &eff, slot);
        }
    }
    need + poisoned(skill, role) > 0
}

/// `CheckUsable` 不放行时要说的话。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refusal {
    /// 全队没人用得上（108，并放 wa003）——只在要出提示时查。
    NobodyNeeds,
    /// 不是我方用的（108）。
    NotOurs,
    /// 气 / 神不够（特技 238，仙术 109）。
    Cost(usize),
    /// 钱不够（244）。
    Money,
    /// 没有影响（108）。
    NoEffect,
}

impl Refusal {
    /// 提示框里的语言表号和开头的空格（`"\n%s"` 前面那串）。
    pub fn text(self) -> (usize, &'static str) {
        match self {
            Refusal::Cost(n) => (n, "      "),
            Refusal::Money => (244, "      "),
            _ => (108, "        "),
        }
    }
}

/// `CheckUsable(l_index, l_type, bdisinfo)`：`user` 现在能不能开始施这门。`show` 是 `bdisinfo`
/// ——只有要出提示时才先查「全队有没有人用得上」。
pub fn check_usable(
    skill: &SkillInfo,
    state: &PersistentState,
    db: &GameDb,
    user: u32,
    skill_tab: bool,
    show: bool,
) -> Result<(), Refusal> {
    if show && !team(state).any(|a| can_use_this(skill, state, db, a)) {
        return Err(Refusal::NobodyNeeds);
    }
    // 有没有影响：原版把毒的贡献放在 12 次属性循环里，每项都加一次——只影响是否 >0，照写。
    let mut effect = 0;
    for a in team(state) {
        let Some(role) = state.role(a) else { continue };
        let eff = attributes::effective(role, db, state.magic_sword());
        for slot in 0..ATTRIBUTE_SLOTS {
            if skill.attribute_impact_value[slot] > 0 {
                effect += deficit(role, &eff, slot);
            }
            effect += poisoned(skill, role);
        }
    }
    let range = skill.target_range_type;
    if range != 1 && range != 2 {
        return Err(Refusal::NotOurs);
    }
    let Some(me) = state.role(user) else {
        return Err(Refusal::NotOurs);
    };
    if me.sp < skill.sp_consume_value || me.mp < skill.mp_consume_value {
        return Err(Refusal::Cost(if skill_tab { 238 } else { 109 }));
    }
    if skill.special_consume_type == 10 {
        let total = money_heal_total(state, db, user);
        if state.money() < total || state.money() <= 0 || total <= 0 {
            return Err(Refusal::Money);
        }
    }
    if effect <= 0 {
        return Err(Refusal::NoEffect);
    }
    Ok(())
}

/// 特殊消耗 10（拿钱补血）：每人补的精不超过 `40 + 2 × 施术者等级`，花的钱等于补的精。
fn money_heal_total(state: &PersistentState, db: &GameDb, user: u32) -> i32 {
    let level = state.role(user).map_or(1, |r| r.level);
    let cap = 40 + 2 * level;
    team(state)
        .filter_map(|a| {
            let role = state.role(a)?;
            let eff = attributes::effective(role, db, state.magic_sword());
            Some((eff.max_hp - role.hp).min(cap))
        })
        .sum()
}

/// `CheckNeedTo`：箭头落在谁头上（按角色号）。单体：伤得最重的那个（它用得上的话）；全体：每个
/// 用得上的。原版两处毛病照留：毒的贡献按「加不加」各算一次；最需要的人一点没伤时把「最小值」当成
/// 下标（越界就没有箭头）。
pub fn need_to(
    skill: &SkillInfo,
    state: &PersistentState,
    db: &GameDb,
) -> [bool; PLAYER_ACTOR_COUNT] {
    let mut arrows = [false; PLAYER_ACTOR_COUNT];
    let mut needs = [0i32; PLAYER_ACTOR_COUNT];
    let mut any_effect = false;
    for a in team(state) {
        let Some(role) = state.role(a) else { continue };
        let eff = attributes::effective(role, db, state.magic_sword());
        let mut value = 0;
        for slot in 0..ATTRIBUTE_SLOTS {
            if skill.attribute_impact_value[slot] != 0 {
                needs[a as usize] += deficit(role, &eff, slot);
                value += skill.attribute_impact_value[slot] as i32;
            }
            for &p in &POISONS {
                if skill.combat_state_impact_types[p] != 0 {
                    needs[a as usize] += role.states[p] as i32;
                    value += 1;
                }
            }
            if value > 0 {
                any_effect = true;
            }
        }
    }
    if !any_effect {
        return arrows;
    }
    let members: Vec<usize> = team(state).map(|a| a as usize).collect();
    let Some(&first) = members.first() else {
        return arrows;
    };
    let mut min = needs[first];
    let mut need = 0usize;
    for &a in &members {
        if needs[a] > min {
            min = needs[a];
            need = a;
        }
    }
    if needs[need] == 0 {
        need = min.max(0) as usize;
    }
    match skill.target_range_type {
        1 => {
            if need < PLAYER_ACTOR_COUNT && can_use_this(skill, state, db, need as u32) {
                arrows[need] = true;
            }
        }
        2 => {
            for &a in &members {
                arrows[a] = can_use_this(skill, state, db, a as u32);
            }
        }
        _ => {}
    }
    arrows
}

/// 施出去之后的结果。
#[derive(Clone, Debug, PartialEq)]
pub enum Cast {
    /// 没施成。`refusal` 有值时要出提示框（108，并放 wa004）。
    Refused { refusal: bool },
    /// 施成了。`level_up` 是升级提示（已排好版）；`keep` 是还留不留在选人状态。
    Done {
        targets: Vec<u32>,
        level_up: Option<String>,
        keep: bool,
    },
}

/// 原版 `Warp(n, 0.1)`：在 ±10% 里随机挪一下。`rand` 返回 0..32767。
fn warp(n: i32, rand: &mut dyn FnMut() -> i32) -> i32 {
    let w = ((n as f32) * 0.1) as i32;
    let w = w.abs();
    if w == 0 {
        return 0;
    }
    let d = rand() % w;
    if rand() % 1024 >= 512 { n + d } else { n - d }
}

/// 给 `target` 加上这门仙术的效果（单体支 / 全体支各一种状态拷法）。
fn affect(
    skill: &SkillInfo,
    state: &mut PersistentState,
    db: &GameDb,
    user_level: i32,
    target: u32,
    skip: std::ops::RangeInclusive<usize>,
    rand: &mut dyn FnMut() -> i32,
) {
    let Some(role) = state.role_mut(target) else {
        return;
    };
    for slot in 0..ATTRIBUTE_SLOTS {
        let value = skill.attribute_impact_value[slot] as i32;
        let n = ((value as f32) * (user_level as f32 / 20.) * 1.5) as i32;
        let n = warp(n.abs(), rand);
        let n = if value > 0 { n } else { -n };
        match slot {
            SLOT_HP => role.hp += n,
            SLOT_SP => role.sp += n,
            SLOT_MP => role.mp += n,
            _ => {}
        }
    }
    for k in 0..STATE_BYTES {
        if !skip.contains(&k) {
            role.states[k] = skill.combat_state_impact_types[k] as u8;
        }
    }
    for &p in &POISONS {
        if skill.combat_state_impact_types[p] >= 2 {
            role.states[p] = 0;
        }
    }
    clamp_current(state, target, db);
}

/// `UsingMagicRole(user, target, …)` 加 `UpdateMagicLV`。`skill_tab` 是这门在「技」页（特技不升级）；
/// `lang` 取语言表（升级提示用）。
pub fn cast(
    skill: &SkillInfo,
    state: &mut PersistentState,
    db: &GameDb,
    user: u32,
    target: u32,
    skill_tab: bool,
    lang: &dyn Fn(usize) -> String,
    rand: &mut dyn FnMut() -> i32,
) -> Cast {
    let range = skill.target_range_type;
    if range != 1 && range != 2 {
        return Cast::Refused { refusal: false };
    }
    if range == 1 && !can_use_this(skill, state, db, target) {
        return Cast::Refused { refusal: true };
    }
    let Some(me) = state.role(user) else {
        return Cast::Refused { refusal: false };
    };
    // 这里比的是基值（`rolevalue`），不是当前值。
    if me.max_sp < skill.sp_consume_value || me.max_mp < skill.mp_consume_value {
        return Cast::Refused { refusal: false };
    }
    let user_level = me.level;
    let mut targets = vec![];
    if skill.special_consume_type == 10 {
        if state.money() < skill.special_consume_value {
            return Cast::Refused { refusal: false };
        }
        if skill.special_consume_impact_type == 0 {
            // 拿钱补血：每人补的精不超过 40 + 2 × 等级，钱照补的扣；只扣气，不记升级。
            let cap = 40 + 2 * user_level;
            let mut spent = 0;
            let members: Vec<u32> = team(state).collect();
            for a in members {
                let used = can_use_this(skill, state, db, a);
                let Some(role) = state.role(a) else { continue };
                let eff = attributes::effective(role, db, state.magic_sword());
                let real = (eff.max_hp - role.hp).min(cap);
                if let Some(role) = state.role_mut(a) {
                    role.hp += real;
                }
                clamp_current(state, a, db);
                spent += real;
                if used {
                    targets.push(a);
                }
            }
            state.add_money(-spent);
            if let Some(role) = state.role_mut(user) {
                role.sp -= skill.sp_consume_value;
            }
            bump_uses(state, user, skill.id);
            let keep = anyone_short(skill, state, db, false);
            return Cast::Done {
                targets,
                level_up: None,
                keep,
            };
        }
    }

    if range == 1 {
        affect(skill, state, db, user_level, target, SINGLE_SKIP, rand);
        targets.push(target);
    } else {
        let members: Vec<u32> = team(state).collect();
        for a in members {
            if can_use_this(skill, state, db, a) {
                affect(skill, state, db, user_level, a, ALL_SKIP, rand);
                targets.push(a);
            }
        }
    }
    if let Some(role) = state.role_mut(user) {
        role.sp -= skill.sp_consume_value;
        role.mp -= skill.mp_consume_value;
    }
    clamp_current(state, user, db);
    bump_uses(state, user, skill.id);

    let level_up = if skill_tab {
        None
    } else {
        level_up(skill, state, db, user, lang)
    };
    let keep = level_up.is_none()
        && anyone_short(skill, state, db, true)
        && check_usable(skill, state, db, user, skill_tab, false).is_ok();
    Cast::Done {
        targets,
        level_up,
        keep,
    }
}

fn bump_uses(state: &mut PersistentState, user: u32, skill_id: u32) {
    if let Some(s) = state
        .role_mut(user)
        .and_then(|r| r.skills.iter_mut().find(|s| s.id == skill_id))
    {
        s.uses = s.uses.saturating_add(1);
    }
}

/// 收尾那段「还有没有人要用」：五个人都算（不看在不在队）；拿钱补血那一支不算毒。
fn anyone_short(
    skill: &SkillInfo,
    state: &PersistentState,
    db: &GameDb,
    with_poison: bool,
) -> bool {
    let mut value = 0;
    for a in 0..PLAYER_ACTOR_COUNT as u32 {
        if !with_poison && !state.is_in_team(a) {
            continue;
        }
        let Some(role) = state.role(a) else { continue };
        let eff = attributes::effective(role, db, state.magic_sword());
        for slot in 0..ATTRIBUTE_SLOTS {
            if skill.attribute_impact_value[slot] > 0 {
                value += deficit(role, &eff, slot);
            }
        }
        if with_poison {
            value += poisoned(skill, role);
        }
    }
    value > 0
}

/// `UpdateMagicLV`：用够次数升一级（门槛是 `lvup[级+1]`，和战斗结算用的不一样）；升到 3 级且人够级、
/// 仙术够级就学会下一门。返回提示框的字。
fn level_up(
    skill: &SkillInfo,
    state: &mut PersistentState,
    db: &GameDb,
    user: u32,
    lang: &dyn Fn(usize) -> String,
) -> Option<String> {
    let role_level = state.role(user)?.level;
    let learned = state
        .role_mut(user)?
        .skills
        .iter_mut()
        .find(|s| s.id == skill.id)?;
    if learned.level >= 3 {
        return None;
    }
    let lv = (learned.level as usize).min(2);
    if (learned.uses as i32) < skill.times_before_level_up[lv + 1] as i32 {
        return None;
    }
    learned.level += 1;
    let level = learned.level;
    let who = lang(64 + user as usize);
    let name = skill.name.as_str().unwrap_or_default();
    let mut text = format!(
        "\n\n    {who} {name} {}{}{}\n",
        lang(376),
        level + 1,
        lang(55)
    );
    if level >= 3 && role_level >= skill.required_actor_level as i32 && level >= skill.magic_level {
        let next = skill.next_level_skill_id;
        if next > 0 && state.learn_skill(user, next) {
            let next_name = db
                .skill(next)
                .and_then(|s| s.name.as_str().ok())
                .unwrap_or_default();
            text.push_str(&format!("    {who}{} {next_name}\n", lang(377)));
        }
    }
    Some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(elements: [i32; 5], kind: u8) -> SkillInfo {
        let mut s = SkillInfo::default();
        s.skill_type = kind;
        s.element_attributes = elements;
        s.applicable_actors = [1; PLAYER_ACTOR_COUNT];
        s
    }

    #[test]
    fn tabs_follow_get_role_skill() {
        assert_eq!(tab_of(&skill([0, 0, 0, 0, 0], 0), 0), Some(TAB_SKILL));
        // 特技带属性也进「技」。
        assert_eq!(tab_of(&skill([1, 0, 0, 0, 0], 0), 0), Some(TAB_SKILL));
        assert_eq!(tab_of(&skill([0, 0, 1, 0, 0], 1), 0), Some(4));
        assert_eq!(tab_of(&skill([0, 0, 0, 1, 0], 1), 0), Some(3));
        assert_eq!(tab_of(&skill([1, 0, 0, 0, 0], 1), 0), Some(1));
        assert_eq!(tab_of(&skill([0, 1, 0, 0, 0], 1), 0), Some(2));
        assert_eq!(tab_of(&skill([0, 0, 0, 0, 1], 1), 0), Some(5));
        assert_eq!(tab_of(&skill([1, 1, 0, 0, 0], 1), 0), Some(TAB_DUAL));
        // 仙术没有属性：进「双」。
        assert_eq!(tab_of(&skill([0, 0, 0, 0, 0], 1), 0), Some(TAB_DUAL));
        let mut not_mine = skill([0, 0, 0, 0, 0], 0);
        not_mine.applicable_actors[2] = 0;
        assert_eq!(tab_of(&not_mine, 2), None);
    }

    #[test]
    fn warp_stays_within_ten_percent() {
        let mut seed = 1u32;
        let mut rand = || {
            seed = seed.wrapping_mul(214013).wrapping_add(2531011);
            ((seed >> 16) & 0x7fff) as i32
        };
        for _ in 0..200 {
            let v = warp(300, &mut rand);
            assert!((270..=330).contains(&v), "{v}");
        }
        // 原版 `nWarp == 0` 时返回 0 而不是 n：不到 10 点的效果全没了。
        assert_eq!(warp(5, &mut rand), 0);
    }
}
