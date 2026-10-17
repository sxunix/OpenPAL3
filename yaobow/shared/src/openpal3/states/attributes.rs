//! Turning a character's level-derived attributes plus what they are
//! wearing into the numbers the game actually uses.
//!
//! `FUN_0040be80` walks the twelve `ActorAttributeType` slots and, for
//! each of the six equipment slots, adds:
//!
//! * for slots 0..=6 (精气神武防速运) the item's
//!   `attribute_impact_value[i]`, but **only when that slot's
//!   `attribute_impact_type[i]` is 0 (Absolute)** — a percentage-typed
//!   entry on a piece of equipment contributes nothing;
//! * for slots 7..=11 (五灵 水火风雷土) a flat +1 if the item carries that
//!   element at all.
//!
//! The 魔剑 (6603) is special-cased there (`cmp esi, 0x19cb`): its holder
//! also gets the sword's own 武 (`g_dwWeaponWU`, `[0x58615c]`) and +1 in the
//! element of the sword's current block (`SwordUI::m_PurgeNum`, `[0x5d0750]`)
//! — see [`super::sword_purge`].
//!
//! The offsets it reads (`item + 0x277`, `+ 0x284`, `+ 0x26b`) land
//! exactly on our parsed `GameItemInfo`'s `attribute_impact_type`,
//! `attribute_impact_value` and `element_attributes`, which is one more
//! confirmation that the runtime item block is the database record.

use fileformats::pal3::gdb::GameItemInfo;

use super::game_db::GameDb;
use super::persistent_state::RoleState;
use super::sword_purge::{DEMON_SWORD, MagicSword};

/// 精 气 神 武 防 速 运, then 水 火 风 雷 土.
pub const ATTRIBUTE_SLOTS: usize = 12;
/// Slots below this take their bonus from the impact value; the rest are
/// the elements, which only ever add one.
pub const FIRST_ELEMENT_SLOT: usize = 7;

pub const SLOT_HP: usize = 0;
pub const SLOT_SP: usize = 1;
pub const SLOT_MP: usize = 2;
pub const SLOT_ATTACK: usize = 3;
pub const SLOT_DEFENSE: usize = 4;
pub const SLOT_SPEED: usize = 5;
pub const SLOT_LUCK: usize = 6;

/// What one piece of equipment contributes to each slot.
pub fn item_bonus(item: &GameItemInfo, slot: usize) -> i32 {
    if slot >= ATTRIBUTE_SLOTS {
        return 0;
    }
    if slot < FIRST_ELEMENT_SLOT {
        // Only an Absolute-typed entry counts here.
        if item.attribute_impact_type[slot] != 0 {
            return 0;
        }
        return item.attribute_impact_value[slot] as i32;
    }
    (item.element_attributes[slot - FIRST_ELEMENT_SLOT] != 0) as i32
}

/// A character's attributes after everything they are wearing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EffectiveStats {
    pub max_hp: i32,
    pub max_sp: i32,
    pub max_mp: i32,
    pub attack: i32,
    pub defense: i32,
    pub speed: i32,
    pub luck: i32,
    /// 水火风雷土.
    pub elements: [i32; 5],
}

/// Sum the level-derived base with every equipped piece. `sword` is the
/// 魔剑's purification state, which counts for whoever holds it.
pub fn effective(role: &RoleState, db: &GameDb, sword: &MagicSword) -> EffectiveStats {
    let mut totals = [0i32; ATTRIBUTE_SLOTS];
    for item_id in role.equipment.iter().copied().filter(|id| *id != 0) {
        let Some(item) = db.item(item_id) else {
            continue;
        };
        for (slot, total) in totals.iter_mut().enumerate() {
            *total += item_bonus(item, slot);
        }
        if item_id == DEMON_SWORD {
            let (wu, element) = sword.bonus();
            totals[SLOT_ATTACK] += wu;
            totals[FIRST_ELEMENT_SLOT + element] += 1;
        }
    }

    EffectiveStats {
        max_hp: role.max_hp + totals[SLOT_HP],
        max_sp: role.max_sp + totals[SLOT_SP],
        max_mp: role.max_mp + totals[SLOT_MP],
        attack: role.attack + totals[SLOT_ATTACK],
        defense: role.defense + totals[SLOT_DEFENSE],
        speed: role.speed + totals[SLOT_SPEED],
        luck: role.luck + totals[SLOT_LUCK],
        elements: [
            role.elements[0] + totals[FIRST_ELEMENT_SLOT],
            role.elements[1] + totals[FIRST_ELEMENT_SLOT + 1],
            role.elements[2] + totals[FIRST_ELEMENT_SLOT + 2],
            role.elements[3] + totals[FIRST_ELEMENT_SLOT + 3],
            role.elements[4] + totals[FIRST_ELEMENT_SLOT + 4],
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fileformats::pal3::gdb::GameItemInfo;

    fn blank_item(id: u32) -> GameItemInfo {
        let mut item = GameItemInfo::blank(id);
        item.item_type = 8;
        item
    }

    #[test]
    fn an_absolute_entry_contributes_and_a_percentage_one_does_not() {
        // 金刚斗笠's "防+28 速-1".
        let mut hat = blank_item(80009);
        hat.attribute_impact_value[SLOT_DEFENSE] = 28;
        hat.attribute_impact_value[SLOT_SPEED] = -1;
        assert_eq!(item_bonus(&hat, SLOT_DEFENSE), 28);
        assert_eq!(item_bonus(&hat, SLOT_SPEED), -1);

        // The same numbers typed as a percentage are ignored by the
        // equipment path.
        hat.attribute_impact_type[SLOT_DEFENSE] = 1;
        assert_eq!(item_bonus(&hat, SLOT_DEFENSE), 0);
        assert_eq!(item_bonus(&hat, SLOT_SPEED), -1, "另一格不受影响");
    }

    #[test]
    fn an_element_only_ever_adds_one() {
        let mut robe = blank_item(80010);
        // 玄天帽's "水 防+32": the element flag's magnitude is irrelevant.
        robe.element_attributes[0] = 9;
        robe.attribute_impact_value[SLOT_DEFENSE] = 32;
        assert_eq!(item_bonus(&robe, FIRST_ELEMENT_SLOT), 1);
        assert_eq!(item_bonus(&robe, FIRST_ELEMENT_SLOT + 1), 0);
        assert_eq!(item_bonus(&robe, SLOT_DEFENSE), 32);
    }

    #[test]
    fn slots_out_of_range_are_zero() {
        let hat = blank_item(1);
        assert_eq!(item_bonus(&hat, ATTRIBUTE_SLOTS), 0);
        assert_eq!(item_bonus(&hat, 999), 0);
    }
}
