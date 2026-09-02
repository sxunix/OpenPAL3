//! The static tables the combat system reads at startup, beyond the game
//! database itself: where combatants stand, and which battlefield a
//! random encounter drops you onto.
//!
//! * `datascript/combat.ini` — `[RolePos]` (eleven stage positions),
//!   `[FiveLineup]` / `[EnemyFiveLineup]` (the two 五灵 rings), and the
//!   per-character combat barks.
//! * `datascript/combatScn.txt` — for every adventure scene block, a map
//!   from the floor type under your feet to a combat scene `z**`, with a
//!   `default`; plus a `fivemagic` section giving each combat scene its
//!   own 五灵 attribute.
//!
//! Both files are GBK. `combatScn.txt` is not an ini: it is a series of
//! `BEGIN,<name>` … `END` blocks whose rows are `<key>\t$<value>&`, with
//! `;` and `//` comments.

use std::collections::HashMap;

/// One of the eleven stage slots in `[RolePos]` (x/z on the battlefield;
/// the ground supplies y).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct StagePos {
    pub x: f32,
    pub z: f32,
}

/// A 五灵 ring: where it sits, how big it is and how it is turned.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LineupRing {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub radius: f32,
    pub roll: f32,
}

/// Which combat scene a block drops you onto, by floor type.
#[derive(Debug, Clone, Default)]
pub struct CombatSceneTable {
    pub default: String,
    /// Floor type (the `.nav` attribute) → combat scene name.
    pub by_floor: HashMap<i32, String>,
}

impl CombatSceneTable {
    pub fn resolve(&self, floor_type: i32) -> &str {
        self.by_floor
            .get(&floor_type)
            .map(|s| s.as_str())
            .unwrap_or(self.default.as_str())
    }
}

/// Everything parsed out of the two files above.
#[derive(Debug, Default)]
pub struct CombatData {
    /// `[RolePos]` Role0..Role10.
    pub role_positions: Vec<StagePos>,
    pub five_lineup: LineupRing,
    pub enemy_five_lineup: LineupRing,

    /// Keyed by the `BEGIN,<scene>_<block>` name, lowercased.
    pub scene_tables: HashMap<String, CombatSceneTable>,
    /// Combat scene name → 五灵 (1 水 · 2 火 · 3 风 · 4 雷 · 5 土).
    pub scene_elements: HashMap<String, u8>,
}

impl CombatData {
    /// The battlefield for a party standing on `floor_type` in
    /// `<scene>_<block>`, or `None` when the block has no table.
    pub fn combat_scene(&self, scene: &str, block: &str, floor_type: i32) -> Option<&str> {
        let key = format!("{}_{}", scene, block).to_lowercase();
        Some(self.scene_tables.get(&key)?.resolve(floor_type))
    }

    /// A battlefield's own 五灵 attribute.
    pub fn scene_element(&self, combat_scene: &str) -> Option<u8> {
        self.scene_elements
            .get(&combat_scene.to_lowercase())
            .copied()
    }

    /// Parse both files. Either may be absent; whatever is missing simply
    /// stays empty.
    pub fn parse(combat_ini: &str, combat_scn: &str) -> Self {
        let mut data = Self::default();
        data.parse_combat_ini(combat_ini);
        data.parse_combat_scn(combat_scn);
        data
    }

    fn parse_combat_ini(&mut self, text: &str) {
        let mut section = String::new();
        let mut positions: HashMap<usize, (Option<f32>, Option<f32>)> = HashMap::new();
        let mut ring: HashMap<String, f32> = HashMap::new();

        let flush_ring = |ring: &mut HashMap<String, f32>| -> LineupRing {
            let take = |ring: &HashMap<String, f32>, k: &str| ring.get(k).copied().unwrap_or(0.);
            let out = LineupRing {
                x: take(ring, "x"),
                y: take(ring, "y"),
                z: take(ring, "z"),
                radius: take(ring, "radius"),
                roll: take(ring, "roll"),
            };
            ring.clear();
            out
        };

        for line in text.lines() {
            // Values carry trailing `;` comments and stray whitespace.
            let line = line.split(';').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                // Leaving a ring section commits it.
                match section.as_str() {
                    "FiveLineup" => self.five_lineup = flush_ring(&mut ring),
                    "EnemyFiveLineup" => self.enemy_five_lineup = flush_ring(&mut ring),
                    _ => ring.clear(),
                }
                section = name.to_string();
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let key = key.trim();
            let Ok(value) = value.trim().parse::<f32>() else {
                continue;
            };

            match section.as_str() {
                "RolePos" => {
                    // `Role<n>-x` / `Role<n>-z`.
                    let Some(rest) = key.strip_prefix("Role") else {
                        continue;
                    };
                    let Some((index, axis)) = rest.split_once('-') else {
                        continue;
                    };
                    let Ok(index) = index.parse::<usize>() else {
                        continue;
                    };
                    let slot = positions.entry(index).or_insert((None, None));
                    match axis {
                        "x" => slot.0 = Some(value),
                        "z" => slot.1 = Some(value),
                        _ => {}
                    }
                }
                "FiveLineup" | "EnemyFiveLineup" => {
                    ring.insert(key.to_string(), value);
                }
                _ => {}
            }
        }

        match section.as_str() {
            "FiveLineup" => self.five_lineup = flush_ring(&mut ring),
            "EnemyFiveLineup" => self.enemy_five_lineup = flush_ring(&mut ring),
            _ => {}
        }

        let count = positions.keys().max().map(|m| m + 1).unwrap_or(0);
        self.role_positions = (0..count)
            .map(|i| {
                let (x, z) = positions.get(&i).copied().unwrap_or((None, None));
                StagePos {
                    x: x.unwrap_or(0.),
                    z: z.unwrap_or(0.),
                }
            })
            .collect();
    }

    fn parse_combat_scn(&mut self, text: &str) {
        let mut block: Option<String> = None;
        let mut table = CombatSceneTable::default();

        for line in text.lines() {
            // Both comment styles appear, sometimes on the same line as a
            // row (`z01\t$5&\t//土`).
            let line = line.split("//").next().unwrap_or("");
            let line = line.split(';').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }

            if let Some(name) = line.strip_prefix("BEGIN,") {
                block = Some(name.trim().to_lowercase());
                table = CombatSceneTable::default();
                continue;
            }
            if line == "END" {
                if let Some(name) = block.take() {
                    if name != "fivemagic" {
                        self.scene_tables.insert(name, std::mem::take(&mut table));
                    }
                }
                continue;
            }
            let Some(name) = block.as_deref() else {
                continue;
            };

            // `<key>` whitespace `$<value>&`
            let mut parts = line.split_whitespace();
            let (Some(key), Some(value)) = (parts.next(), parts.next()) else {
                continue;
            };
            let value = value.trim_start_matches('$').trim_end_matches('&');
            if value.is_empty() {
                continue;
            }

            if name == "fivemagic" {
                if let Ok(element) = value.parse::<u8>() {
                    self.scene_elements.insert(key.to_lowercase(), element);
                }
                continue;
            }

            if key.eq_ignore_ascii_case("default") {
                table.default = value.to_lowercase();
            } else if let Ok(floor) = key.parse::<i32>() {
                table.by_floor.insert(floor, value.to_lowercase());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMBAT_INI: &str = "\
[RolePos]
Role0-x=-118.694534
Role0-z=-209.226364
Role1-x=-75.145691
Role1-z=-119.692291
Role5-x=207.959 \t; Role5-x=201.876007
Role5-z=78.8841\t\t; Role5-z=100.266052

[FiveLineup]
x=-162.497406
y=0.000000
z=-130.144135
radius=207.000000
roll=-0.500003

[EnemyFiveLineup]
x=87.4095
y=0.000000
z=140.431
radius=207.00000\t;radius=307.000000
roll=0.156633

[Level]
1=0
2=19
";

    const COMBAT_SCN: &str = "\
BEGIN,fivemagic\t;每个场景的五灵属性
z01\t$5&\t//土
z08\t$3&     //草
END

//璧山
BEGIN,m01_1
default\t$z08&
2\t$z01&
9\t$z10&
END

BEGIN,q01_x
default\t$z22&
END
";

    #[test]
    fn parses_stage_positions_and_rings() {
        let data = CombatData::parse(COMBAT_INI, "");

        // Slots are indexed, and the gaps in between stay present.
        assert_eq!(data.role_positions.len(), 6);
        assert_eq!(data.role_positions[0].x, -118.694534);
        assert_eq!(data.role_positions[1].z, -119.692291);
        assert_eq!(data.role_positions[2], StagePos::default(), "2..4 未定义");
        // The commented-out alternative on the same line must not win.
        assert_eq!(data.role_positions[5].x, 207.959);

        assert_eq!(data.five_lineup.radius, 207.0);
        assert_eq!(data.five_lineup.roll, -0.500003);
        assert_eq!(data.enemy_five_lineup.x, 87.4095);
        assert_eq!(data.enemy_five_lineup.radius, 207.0);
    }

    #[test]
    fn resolves_battlefields_by_floor_type() {
        let data = CombatData::parse("", COMBAT_SCN);

        // A listed floor type wins, anything else falls back to default.
        assert_eq!(data.combat_scene("m01", "1", 2), Some("z01"));
        assert_eq!(data.combat_scene("m01", "1", 9), Some("z10"));
        assert_eq!(data.combat_scene("m01", "1", 7), Some("z08"), "default");
        assert_eq!(data.combat_scene("M01", "1", 99), Some("z08"), "大小写无关");
        assert_eq!(data.combat_scene("q01", "x", 0), Some("z22"));
        assert_eq!(data.combat_scene("nope", "1", 0), None);

        // fivemagic is a table of its own, not a block.
        assert_eq!(data.scene_element("z01"), Some(5));
        assert_eq!(data.scene_element("z08"), Some(3));
        assert_eq!(data.scene_element("z99"), None);
        assert!(!data.scene_tables.contains_key("fivemagic"));
    }
}
