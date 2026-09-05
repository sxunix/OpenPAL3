//! Runs a battle against the shipped game data and prints a blow-by-blow
//! log, so the ported formulas can be eyeballed against the real game.
//!
//! Skipped (not failed) when the database fixture is absent, so CI without
//! game data stays green. To produce it:
//!
//! ```text
//! cargo run -p packfs --example pal3_probe -- <PAL3 dir> \
//!     --dump /basedata/basedata/cbdata/pal3_softstar.gdb /tmp/pal3_softstar.gdb
//! YAOBOW_PAL3_GDB=/tmp/pal3_softstar.gdb \
//!     cargo test -p shared --test pal3_battle_log -- --nocapture
//! ```

use std::io::Cursor;

use fileformats::pal3::gdb::GdbFile;
use shared::openpal3::combat::actions::{ATTR_HP, ATTRIBUTE_SLOTS, SkillEffect};
use shared::openpal3::combat::ai::{self, Decision, SkillOption};
use shared::openpal3::combat::formulas::CombatRng;
use shared::openpal3::combat::state::{Battle, Combatant, Due, Outcome, SLOTS, Side};
use shared::openpal3::states::game_db::combat_id_of_party_member;
use shared::openpal3::states::growth;

fn load_gdb() -> Option<GdbFile> {
    let path =
        std::env::var("YAOBOW_PAL3_GDB").unwrap_or_else(|_| "/tmp/pal3_softstar.gdb".to_string());
    let raw = std::fs::read(&path).ok()?;
    GdbFile::read(&mut Cursor::new(raw)).ok()
}

/// A playable character at `level`, built the way a new game builds one:
/// attributes off the growth curves, 速/运 off the database row.
fn player(gdb: &GdbFile, party_index: u32, level: i32) -> Combatant {
    let combat_id = combat_id_of_party_member(party_index).expect("party index 0..4");
    let row = gdb
        .combat_actors
        .iter()
        .find(|a| a.id == combat_id)
        .expect("playable character in the database");
    let g = growth::at_level(party_index as usize, level);

    let mut c = Combatant::new(combat_id, Side::Party);
    c.level = level;
    c.max_hp = g.max_hp;
    c.hp = g.max_hp;
    c.max_sp = g.max_sp;
    c.sp = g.max_sp;
    c.max_mp = g.max_mp;
    c.mp = g.max_mp;
    c.attack = g.attack;
    c.defense = g.defense;
    c.speed = row.attribute_values[5];
    c.luck = row.attribute_values[6];
    for (i, v) in row.element_attribute_values.iter().enumerate() {
        c.elements[i + 1] = *v > 0;
    }
    c
}

/// A monster straight off its database row.
fn monster(gdb: &GdbFile, actor: u32) -> Combatant {
    let row = gdb
        .combat_actors
        .iter()
        .find(|a| a.id == actor)
        .expect("monster in the database");

    let mut c = Combatant::new(actor, Side::Enemy);
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
    c
}

/// A skill's name, for the log.
fn effect_name(gdb: &GdbFile, skill_id: u32) -> String {
    gdb.skills
        .iter()
        .find(|s| s.id == skill_id)
        .and_then(|s| s.name.as_str().ok())
        .unwrap_or_else(|| format!("仙术#{skill_id}"))
}

/// What the AI weighs: a monster its four database skills, a party member
/// whatever we handed it.
fn ai_options(
    gdb: &GdbFile,
    actor_id: u32,
    side: Side,
    player_skill: &Option<SkillEffect>,
) -> Vec<SkillOption> {
    match side {
        Side::Party => player_skill
            .iter()
            .map(|s| SkillOption {
                id: s.id,
                hp_impact: s.impacts[ATTR_HP].1 as i32,
            })
            .collect(),
        Side::Enemy => gdb
            .combat_actors
            .iter()
            .find(|a| a.id == actor_id)
            .map(|a| {
                a.skill_ids
                    .iter()
                    .copied()
                    .filter(|id| *id != 0)
                    .filter_map(|id| {
                        let s = gdb.skills.iter().find(|s| s.id == id)?;
                        Some(SkillOption {
                            id,
                            hp_impact: s.attribute_impact_value[ATTR_HP] as i32,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn name(gdb: &GdbFile, actor: u32) -> String {
    gdb.combat_actors
        .iter()
        .find(|a| a.id == actor)
        .and_then(|a| a.name.as_str().ok())
        .unwrap_or_else(|| format!("#{actor}"))
}

/// Turn a database skill row into the effect the combat code wants.
fn skill_effect(gdb: &GdbFile, skill_id: u32) -> Option<SkillEffect> {
    let s = gdb.skills.iter().find(|s| s.id == skill_id)?;
    let mut impacts = [(0u8, 0i16); ATTRIBUTE_SLOTS];
    for slot in 0..ATTRIBUTE_SLOTS {
        impacts[slot] = (
            s.attribute_impact_type[slot],
            s.attribute_impact_value[slot],
        );
    }
    let mut elements = s
        .element_attributes
        .iter()
        .enumerate()
        .filter(|(_, v)| **v != 0)
        .map(|(i, _)| i as u8 + 1);
    Some(SkillEffect {
        id: s.id,
        level: s.level as i32,
        success_rate_level: s.success_rate_level,
        elements: (elements.next(), elements.next()),
        impacts,
        sp_cost: s.sp_consume_value,
        mp_cost: s.mp_consume_value,
    })
}

#[test]
fn a_battle_against_the_shipped_data_plays_out() {
    let Some(gdb) = load_gdb() else {
        eprintln!("skipping: no pal3_softstar.gdb fixture (set YAOBOW_PAL3_GDB)");
        return;
    };

    let mut battle = Battle::new(0x1234_5678);
    // 景天 and 雪见 at level 5 against the first two monsters in the table.
    battle.place(0, player(&gdb, 0, 5));
    battle.place(1, player(&gdb, 1, 5));

    let monsters: Vec<u32> = gdb
        .combat_actors
        .iter()
        .filter(|a| a.actor_type != 0)
        .take(2)
        .map(|a| a.id)
        .collect();
    for (i, id) in monsters.iter().enumerate() {
        battle.place(5 + i, monster(&gdb, *id));
    }

    // 景天's opening skill, if the database gives him one usable here.
    let skill = skill_effect(&gdb, 4101);

    println!("=== 出场 ===");
    for slot in 0..SLOTS {
        if let Some(c) = battle.get(slot) {
            println!(
                "  [{slot:2}] {:<12} Lv{:<3} 精{:>5}/{:<5} 武{:>4} 防{:>4} 速{:>3} 行动条速率{:>4}",
                name(&gdb, c.actor_id),
                c.level,
                c.hp,
                c.max_hp,
                c.attack,
                c.defense,
                c.speed,
                c.gauge_rate(),
            );
        }
    }

    println!("=== 开打 ===");
    // The AI draws from its own generator, as the original does.
    let mut ai_rng = CombatRng::new(0x9e37_79b9);
    let mut elapsed = 0.0f32;
    let mut turns = 0;
    // 两段行动条：条满先选行动并进入出手段，出手条满了才结算。选定的
    // 行动挂在这里等，和原版行动任务把它记在 `task->+0x2c` 是一个意思。
    let mut pending: Option<(usize, Decision)> = None;
    while battle.outcome() == Outcome::Ongoing && turns < 400 {
        elapsed += 0.1;
        let Some(due) = battle.update(0.1) else {
            continue;
        };

        let slot = match due {
            Due::Ready(slot) => {
                let actor = battle.get(slot).unwrap();
                let side = actor.side;
                let actor_id = actor.actor_id;
                let confused = actor.has_state(shared::openpal3::combat::state::K_CONFUSED);
                let options = ai_options(&gdb, actor_id, side, &skill);
                let decision = ai::choose(&battle, &mut ai_rng, slot, &options, confused);
                if matches!(decision, Decision::Pass) {
                    battle.finish_turn(slot);
                    continue;
                }
                pending = Some((slot, decision));
                // 倍率取 1：技能表里那个字节还没接上，见 session.rs 的
                // `cast_multiplier`。
                battle.begin_execute(slot, 1);
                continue;
            }
            Due::Executed(slot) => slot,
        };

        turns += 1;
        let Some((owner, decision)) = pending.take() else {
            battle.finish_turn(slot);
            continue;
        };
        if owner != slot {
            pending = Some((owner, decision));
            battle.finish_turn(slot);
            continue;
        }
        let actor_id = battle.get(slot).unwrap().actor_id;

        let line = match decision {
            Decision::Pass => "按兵不动".to_string(),
            Decision::Attack { target } => {
                let target_id = battle.get(target).unwrap().actor_id;
                let moved = battle.basic_attack(slot, target);
                format!("普攻 → {} 精{:+}", name(&gdb, target_id), moved)
            }
            Decision::Cast { target, skill: id } => {
                let target_id = battle.get(target).unwrap().actor_id;
                match skill_effect(&gdb, id) {
                    Some(effect) => {
                        let r = battle.cast_skill(slot, target, &effect);
                        if r.hit {
                            format!(
                                "{} → {} 精{:+}",
                                effect_name(&gdb, id),
                                name(&gdb, target_id),
                                r.deltas[ATTR_HP]
                            )
                        } else {
                            format!(
                                "{} → {} 未命中",
                                effect_name(&gdb, id),
                                name(&gdb, target_id)
                            )
                        }
                    }
                    None => {
                        let moved = battle.basic_attack(slot, target);
                        format!("普攻 → {} 精{:+}", name(&gdb, target_id), moved)
                    }
                }
            }
        };

        println!(
            "  {:6.1}s 第{:>2}回合 [{slot:2}] {:<12} {}",
            elapsed,
            battle.round,
            name(&gdb, actor_id),
            line
        );
        battle.finish_turn(slot);
    }

    println!(
        "=== 结束: {:?} (第 {} 回合) ===",
        battle.outcome(),
        battle.round
    );
    for slot in 0..SLOTS {
        if let Some(c) = battle.get(slot) {
            println!(
                "  [{slot:2}] {:<12} 精{:>5}/{:<5} {}",
                name(&gdb, c.actor_id),
                c.hp,
                c.max_hp,
                if c.alive { "" } else { "倒下" }
            );
        }
    }

    assert_ne!(
        battle.outcome(),
        Outcome::Ongoing,
        "战斗应在 400 次出手内分出胜负"
    );
}
