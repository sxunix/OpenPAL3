//! 合击技（gdb 里 10 条，id 501..510）。
//!
//! 原版不在菜单里单列。我方有人出手时，行动任务 `FUN_00514870` 先拿这次的技能 id
//! 去问 `FUN_004eea70`：有一条合击技配得上，就把行动改成 0x10e、复位行动条，交给
//! 编排任务 0x514220 直接开演，不走出手段。生效在 `FUN_004ef8b0` → `FUN_004f1c40`，
//! 对每个目标走 `FUN_004eef20` 的 0x10e 分支，**不扣任何消耗**。
//!
//! 逆向记在 `switch/re/notes/combat_flow.md` §9.4、§9.10。

use fileformats::pal3::gdb::ComboSkillInfo;

use super::state::{Battle, SLOTS};

/// 我方占前五个战斗位。
pub const PARTY_SLOTS: usize = 5;

/// 状态要求里的 0x1a（`FUN_004f3cc0`）：战斗位 `+0x476..+0x47a` 五个字节里有一个是 1。
/// 那五个字节是什么还没查；实机只有「十方封魔斩」要它，而那条本来就配不上（技能 id 0），
/// 这里一律当不满足。
const STATE_ANY_OF_FIVE: i32 = 0x1a;

/// 一条合击技，从 [`ComboSkillInfo`] 取判定和生效要用的字段。
#[derive(Debug, Clone, PartialEq)]
pub struct ComboDef {
    pub id: u32,
    pub name: String,
    /// 参与的人（战斗角色 id，101 景天那一套；0 = 空）。第一个必须是出手的那个。
    pub actors: [u32; 4],
    /// 每人要站的五灵阵位：1..6 必须相等，7 风火雷任意，8 水火土任意，0 不要求。
    pub positions: [u8; 4],
    /// 触发它的技能。
    pub skill_id: u32,
    /// 每人手上武器的类型，**逐个相等**（0 也要相等，没有「不限」）。
    pub weapons: [u8; 4],
    /// 第 2..4 个人要带的状态（`ActorCombatStateType`，查 `has_state(值 + 1)`）；负数不要求。
    pub states: [i32; 3],
    /// `FUN_004f0360` 的选目标方式：1 我方全体、3 敌方全体、其余打选中的那个。
    pub range: u8,
    /// 精那一格（`combo+0x25e`）的正负：负是打、正是治。原版只用它的符号。
    pub hp_sign: i16,
}

impl ComboDef {
    pub fn from_gdb(c: &ComboSkillInfo) -> Self {
        Self {
            id: c.id,
            name: c.name.as_str().unwrap_or_default(),
            actors: c.main_actor_requirements,
            positions: c.element_position_requirements,
            skill_id: c.skill_id,
            weapons: c.weapon_type_requirements,
            states: c.combat_state_requirements,
            range: c.target_range_type,
            hp_sign: c.attribute_impact_value[0].signum(),
        }
    }

    /// 参与者的战斗位，按记录里的顺序。
    pub fn participant_slots(&self) -> Vec<usize> {
        self.actors
            .iter()
            .filter(|a| **a != 0)
            .filter_map(|a| slot_of(*a))
            .collect()
    }
}

/// 战斗角色 id → 战斗位，`FUN_004eea70` 里写死的那张表（0x4eeb0a 的 switch）。
/// 105 / 106 是龙葵两种形象，同一个位；103 不在表里。
pub fn slot_of(actor_id: u32) -> Option<usize> {
    match actor_id {
        101 => Some(0),
        104 => Some(1),
        105 | 106 => Some(2),
        107 => Some(3),
        102 => Some(4),
        _ => None,
    }
}

/// 阵位要求（`FUN_004eea70`）：`req` 1..6 要和站位相等，7 是风火雷（3/2/4），8 是水火土
/// （1/2/5）；0 和 8 以上不查。
pub fn position_fits(required: u8, standing: u8) -> bool {
    match required {
        1..=6 => required == standing,
        7 => matches!(standing, 3 | 2 | 4),
        8 => matches!(standing, 1 | 2 | 5),
        _ => true,
    }
}

/// `caster` 这次放 `skill_id`，这条合击技配不配得上（`FUN_004eea70` 的循环体）。
pub fn eligible(def: &ComboDef, caster: usize, skill_id: u32, battle: &Battle) -> bool {
    // 只有我方：原版的条件是「槽 < 5 或 > 10」。
    if caster >= PARTY_SLOTS {
        return false;
    }
    let Some(me) = battle.get(caster) else {
        return false;
    };
    if def.skill_id != skill_id || def.actors[0] != me.actor_id {
        return false;
    }
    for j in 0..4 {
        let actor = def.actors[j];
        if actor == 0 {
            continue;
        }
        // 表里没有的 id 原版会拿 −1 当槽号去读，这里当不满足。
        let Some(c) = slot_of(actor).and_then(|s| battle.get(s)) else {
            return false;
        };
        // 原版只看「在场」（`+0x241`），不看死活。
        if !position_fits(def.positions[j], c.element_position) {
            return false;
        }
        if def.weapons[j] != c.weapon_type {
            return false;
        }
    }
    // 状态要求对的是第 2..4 个人。表里没有的 id 这一格跳过（原版 switch 落到 default）。
    for t in 0..3 {
        let st = def.states[t];
        let Some(c) = slot_of(def.actors[t + 1]).and_then(|s| battle.get(s)) else {
            continue;
        };
        if (0..STATE_ANY_OF_FIVE).contains(&st) && !c.has_state(st as usize + 1) {
            return false;
        }
        if st == STATE_ANY_OF_FIVE {
            return false;
        }
    }
    true
}

/// 按记录顺序找第一条配得上的（`FUN_00479260` 就是按表的顺序）。
pub fn find(defs: &[ComboDef], caster: usize, skill_id: u32, battle: &Battle) -> Option<usize> {
    defs.iter()
        .position(|d| eligible(d, caster, skill_id, battle))
}

/// 打谁（`FUN_004f0360`）。1 我方全体、3 敌方全体，都只挑还站着的 [推断：原版另看战斗位
/// `+0x598`，没查]；其余打选中的那个（范围 2 在选中的是自己人时会改挑一个，这里没做）。
pub fn targets(def: &ComboDef, chosen: usize, battle: &Battle) -> Vec<usize> {
    let standing = |r: std::ops::Range<usize>| -> Vec<usize> {
        r.filter(|s| battle.get(*s).is_some_and(|c| c.in_play()))
            .collect()
    };
    match def.range {
        1 => standing(0..PARTY_SLOTS),
        3 => standing(PARTY_SLOTS..SLOTS),
        _ => vec![chosen],
    }
}

/// 对 `target` 精的改变量（`FUN_004eef20` 0x10e 分支）：量是 `FUN_004f2800` 算的
/// ——参与者各打一下的物理伤害加起来翻倍、再 ±10%——正负按记录里精那一格。
pub fn hp_delta(def: &ComboDef, target: usize, battle: &mut Battle) -> i32 {
    if def.hp_sign == 0 {
        return 0;
    }
    let v = battle
        .joint_physical(&def.participant_slots(), target)
        .abs();
    if def.hp_sign < 0 { -v } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openpal3::combat::state::{Combatant, Side};

    /// 「连环破斩」：景天放洒金笺，雪见要带状态 14，两人各站风 / 土，拿剑 / 刺。
    fn lianhuan() -> ComboDef {
        ComboDef {
            id: 501,
            name: "连环破斩".into(),
            actors: [101, 104, 0, 0],
            positions: [3, 5, 0, 0],
            skill_id: 4102,
            weapons: [2, 6, 0, 0],
            states: [14, -1, -1],
            range: 3,
            hp_sign: -1,
        }
    }

    fn fighter(actor: u32, side: Side, position: u8, weapon: u8) -> Combatant {
        let mut c = Combatant::new(actor, side);
        c.hp = 500;
        c.max_hp = 500;
        c.attack = 120;
        c.defense = 10;
        c.level = 10;
        c.element_position = position;
        c.weapon_type = weapon;
        c
    }

    fn ready() -> Battle {
        let mut b = Battle::new(7);
        b.place(0, fighter(101, Side::Party, 3, 2));
        let mut xj = fighter(104, Side::Party, 5, 6);
        xj.states[15].active = true;
        b.place(1, xj);
        b.place(5, fighter(7301, Side::Enemy, 0, 0));
        b.place(6, fighter(7301, Side::Enemy, 0, 0));
        b
    }

    #[test]
    fn the_positions_read_like_the_original() {
        assert!(position_fits(0, 4));
        assert!(position_fits(3, 3) && !position_fits(3, 4));
        assert!(position_fits(6, 6) && !position_fits(6, 0));
        // 7 风火雷任意、8 水火土任意。
        assert!([3, 2, 4].iter().all(|p| position_fits(7, *p)) && !position_fits(7, 1));
        assert!([1, 2, 5].iter().all(|p| position_fits(8, *p)) && !position_fits(8, 3));
    }

    #[test]
    fn the_slot_table_matches_the_party_order() {
        assert_eq!(
            [101, 104, 106, 107, 102].map(slot_of),
            [Some(0), Some(1), Some(2), Some(3), Some(4)]
        );
        assert_eq!(slot_of(105), Some(2));
        assert_eq!(slot_of(103), None);
    }

    #[test]
    fn a_combo_needs_every_condition() {
        let defs = [lianhuan()];
        let b = ready();
        assert_eq!(find(&defs, 0, 4102, &b), Some(0));
        // 别的技能、别人出手、敌方出手都不行。
        assert_eq!(find(&defs, 0, 4101, &b), None);
        assert_eq!(find(&defs, 1, 4102, &b), None);
        assert_eq!(find(&defs, 5, 4102, &b), None);

        let mut wrong_spot = ready();
        wrong_spot.get_mut(1).unwrap().element_position = 4;
        assert_eq!(find(&defs, 0, 4102, &wrong_spot), None);

        let mut wrong_weapon = ready();
        wrong_weapon.get_mut(0).unwrap().weapon_type = 1;
        assert_eq!(find(&defs, 0, 4102, &wrong_weapon), None);

        let mut no_state = ready();
        no_state.get_mut(1).unwrap().states[15].active = false;
        assert_eq!(find(&defs, 0, 4102, &no_state), None);

        // 雪见不在场。
        let mut missing = Battle::new(7);
        missing.place(0, fighter(101, Side::Party, 3, 2));
        missing.place(5, fighter(7301, Side::Enemy, 0, 0));
        assert_eq!(find(&defs, 0, 4102, &missing), None);
    }

    #[test]
    fn an_all_enemies_combo_hits_everyone_still_standing() {
        let mut b = ready();
        assert_eq!(targets(&lianhuan(), 5, &b), vec![5, 6]);
        b.get_mut(6).unwrap().die();
        assert_eq!(targets(&lianhuan(), 5, &b), vec![5]);
    }

    #[test]
    fn the_damage_is_twice_the_joint_blow() {
        let mut b = ready();
        let d = hp_delta(&lianhuan(), 5, &mut b);
        // 每人一下约 (1 + 10×0.005) × (120 − 10) ≈ 115，两人翻倍约 462，再 ±10%。
        assert!((-520..=-400).contains(&d), "{d}");
        let heal = ComboDef {
            hp_sign: 1,
            ..lianhuan()
        };
        assert!(hp_delta(&heal, 5, &mut b) > 0);
    }
}
