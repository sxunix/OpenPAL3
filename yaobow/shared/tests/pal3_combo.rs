//! 合击技：拿实机 `pal3_softstar.gdb` 核对 [`ComboDef`] 读出来的东西。
//!
//! ```sh
//! YAOBOW_PAL3_GDB=/path/to/pal3_softstar.gdb cargo test -p shared --test pal3_combo
//! ```

use fileformats::pal3::gdb::GdbFile;
use shared::openpal3::combat::combo::{ComboDef, slot_of};

fn combos() -> Option<Vec<ComboDef>> {
    let path = std::env::var("YAOBOW_PAL3_GDB").ok()?;
    let bytes = std::fs::read(path).ok()?;
    let gdb = GdbFile::read(&mut std::io::Cursor::new(bytes)).ok()?;
    Some(gdb.combo_skills.iter().map(ComboDef::from_gdb).collect())
}

#[test]
fn the_ten_combos_read_right() {
    let Some(c) = combos() else {
        eprintln!("skipping: set YAOBOW_PAL3_GDB");
        return;
    };
    assert_eq!(
        c.iter().map(|d| d.id).collect::<Vec<_>>(),
        (501..=510).collect::<Vec<_>>()
    );

    // 连环破斩：景天放洒金笺，景天风位、雪见土位（正是新游戏的阵形），剑 / 刺。
    let lh = &c[0];
    assert_eq!(lh.name, "连环破斩");
    assert_eq!(lh.skill_id, 4102);
    assert_eq!(lh.actors, [101, 104, 0, 0]);
    assert_eq!(lh.positions, [3, 5, 0, 0]);
    assert_eq!(lh.weapons, [2, 6, 0, 0]);
    assert_eq!(lh.states, [14, -1, -1]);
    assert_eq!(lh.range, 3);
    assert_eq!(lh.hp_sign, -1, "打人");

    // 珠联璧合是唯一一条治疗的，打我方全体。
    let heal: Vec<_> = c.iter().filter(|d| d.hp_sign > 0).map(|d| d.id).collect();
    assert_eq!(heal, vec![503]);
    assert_eq!(c[2].range, 1);

    // 十方封魔斩的技能 id 是 0，配不上任何一次施放。
    assert_eq!(c[7].name, "十方封魔斩");
    assert_eq!(c[7].skill_id, 0);

    // 参与者都在写死的角色表里；第一个一律是景天或紫萱 / 龙葵这几个主角。
    for d in &c {
        for a in d.actors.iter().filter(|a| **a != 0) {
            assert!(slot_of(*a).is_some(), "{} 的 {a} 不在表里", d.name);
        }
        assert!(d.hp_sign != 0, "{} 没有精那一格", d.name);
    }
}
