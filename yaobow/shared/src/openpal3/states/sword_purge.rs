//! 魔剑净化（御剑页，参考源码 `gamemainui/swordui.cpp`、`gamebase/playermgr.cpp`，和 exe 对过）。
//!
//! 魔剑分五块（按页面上的顺序 **水火雷风土**，和数据库的水火风雷土不一样），每块有阴、阳两面。
//! 用一颗龙晶石（6408）、往「净化列表」里放装备（古值合计不超过 300），确定后那一面就净化了；
//! 一块的两面都净化过算「完全净化」。每次净化重算：
//!
//! - 同步率 = 原同步率 + 非武器件数 / 5 − 武器件数 / 2，限在 1..20；
//! - 魔剑的武 = (武器武均值 + 佩戴武均值 + 防具防均值×(0.9+件数/100)/5 + 佩戴防均值) × (1 + 次数/50)，
//!   再按净化次数（0..9）查表夹在上下限之间；
//! - 武器技：五块「完全净化」的组合和哪条武器技（仙术表里类型 4）的五灵一致就是哪条，没有就是
//!   残影剑（2224）。
//!
//! 魔剑的「当前属性」是最近一次点的那一块（`SwordUI::m_PurgeNum`，exe `0x5d0750`，初值 3 = 风）。
//! 拿着魔剑的人属性里多算魔剑的武（`g_dwWeaponWU`，exe `0x58615c`，初值 35）和当前属性那一灵 +1
//! （exe `FUN_0040be80` 里对 0x19cb 的特判）。

use fileformats::pal3::gdb::GameItemInfo;
use serde::{Deserialize, Serialize};

/// 五块。
pub const BLOCKS: usize = 5;
/// 魔剑。
pub const DEMON_SWORD: u32 = 6603;
/// 龙晶石：每次净化用一颗。
pub const DRAGON_STONE: u32 = 6408;
/// 净化列表古值合计的上限（`FetchVal`）。
pub const GU_LIMIT: i32 = 300;
/// 没配上武器技时的那条：残影剑。
pub const DEFAULT_WEAPON_SKILL: u32 = 2224;

/// `InitPURGEINFO`：按净化次数（0..10）魔剑武的下限、上限。注释里写着对应的地方：九顶山、安宁村、
/// 龙泉岭、雷洲、神树、锁妖塔、灵山仙人洞、熔岩地狱、冰风谷、海底城、剑冢。次数最多到 9，所以最后
/// 一档（剑冢）用不到。
pub const PURGE_LIMITS: [(i32, i32); 11] = [
    (40, 70),
    (60, 90),
    (70, 110),
    (90, 120),
    (90, 130),
    (95, 145),
    (110, 160),
    (120, 185),
    (140, 200),
    (150, 220),
    (205, 265),
];

/// 页面上第 `block` 块（水火雷风土）对应的五灵下标（数据库的水火风雷土）。
pub fn element_of_block(block: usize) -> usize {
    [0, 1, 3, 2, 4][block.min(BLOCKS - 1)]
}

/// 魔剑的净化状态，进存档（源码 `Archive.cpp` 的 misc 段：`m_isPurged`、`m_isAllPurged`、
/// `m_totalNum`、`g_dwWeaponWU`、`m_PurgeNum`；武器技和同步率在景天的 `m_att[0]` 里）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MagicSword {
    /// `[0]` 阴面、`[1]` 阳面，按块。
    pub purged: [[bool; BLOCKS]; 2],
    /// 这一块两面都净化完了。
    pub all_purged: [bool; BLOCKS],
    /// 当前属性：页面上的块号（水火雷风土）。
    pub block: usize,
    /// 净化了几次（`g_totalPurgetime`，0..9）。
    pub count: i32,
    /// 魔剑的武（`g_dwWeaponWU`）。
    pub weapon_wu: i32,
    /// 景天的武器技（`dwWeaponSkillID`）。
    pub weapon_skill: u32,
    /// 景天的同步率（`dwCleanCent`）。
    pub clean_cent: i32,
}

impl Default for MagicSword {
    fn default() -> Self {
        Self {
            purged: [[false; BLOCKS]; 2],
            all_purged: [false; BLOCKS],
            block: 3,
            count: 0,
            weapon_wu: 35,
            weapon_skill: DEFAULT_WEAPON_SKILL,
            clean_cent: 10,
        }
    }
}

impl MagicSword {
    /// 这一块的这一面（`lunar` 阴）净化过没有（算上完全净化）。
    pub fn face_purged(&self, block: usize, lunar: bool) -> bool {
        let side = if lunar { 0 } else { 1 };
        self.purged[side][block] || self.all_purged[block]
    }

    /// 拿着魔剑时属性里多出来的：(武, 加一的那一灵的下标)。
    pub fn bonus(&self) -> (i32, usize) {
        (self.weapon_wu.max(35), element_of_block(self.block))
    }
}

/// 净化列表里的一种装备（`S_Distribute`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entry {
    pub id: u32,
    pub num: i32,
    /// 从哪个分页来的（武器、衣、帽、鞋、饰品）。
    pub page: usize,
}

/// 净化列表和它的合计。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Basket {
    pub entries: Vec<Entry>,
    /// 古值合计（`GuVal`）。
    pub gu: i32,
    weapon_num: i32,
    equip_num: i32,
    adorn_num: i32,
    def_num: i32,
    weapon_wu: i32,
    adorn_wu: i32,
    adorn_fang: i32,
    def_fang: i32,
}

/// 放不进净化列表的原因。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Refusal {
    /// 古值是 0（AssisLang 183 / 271）。
    NoGu,
    /// 放进去古值合计就超过 300 了（94）。
    TooMuch,
}

const WU: usize = 3;
const FANG: usize = 4;

impl Basket {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// `DistributeListData`：放一件进来。成功时调用方把背包里那件减 1。
    pub fn add(&mut self, item: &GameItemInfo, page: usize) -> Result<(), Refusal> {
        if item.ancient_value <= 0 {
            return Err(Refusal::NoGu);
        }
        if self.gu + item.ancient_value > GU_LIMIT {
            return Err(Refusal::TooMuch);
        }
        self.gu += item.ancient_value;
        let v = |k: usize| item.attribute_impact_value[k] as i32;
        match item.item_type {
            11 => {
                self.weapon_num += 1;
                self.weapon_wu += v(WU);
            }
            10 => {
                self.equip_num += 1;
                self.adorn_wu += v(WU);
                self.adorn_fang += v(FANG);
                self.adorn_num += 1;
            }
            7..=9 => {
                self.equip_num += 1;
                self.def_fang += v(FANG);
                self.def_num += 1;
            }
            _ => {}
        }
        match self.entries.iter_mut().find(|e| e.id == item.id) {
            Some(e) => e.num += 1,
            None => self.entries.push(Entry {
                id: item.id,
                num: 1,
                page,
            }),
        }
        Ok(())
    }

    /// `UnDistributeListData`：第 `index` 行拿回一件。返回拿回的物品号，调用方把它放回背包。
    pub fn remove(&mut self, index: usize, item: &GameItemInfo) -> Option<u32> {
        let e = self.entries.get_mut(index)?;
        let id = e.id;
        let v = |k: usize| item.attribute_impact_value[k] as i32;
        match item.item_type {
            11 => self.weapon_num -= 1,
            7..=9 => {
                self.equip_num -= 1;
                self.def_num -= 1;
            }
            10 => {
                self.equip_num -= 1;
                self.adorn_num -= 1;
            }
            _ => {}
        }
        self.gu = (self.gu - item.ancient_value).max(0);
        match item.item_type {
            11 => self.weapon_wu -= v(WU),
            10 => {
                self.adorn_wu -= v(WU);
                self.adorn_fang -= v(FANG);
            }
            7..=9 => self.def_fang -= v(FANG),
            _ => {}
        }
        e.num -= 1;
        if e.num <= 0 {
            self.entries.remove(index);
        }
        if self.entries.is_empty() {
            *self = Self::default();
        }
        Some(id)
    }

    /// 同步率（未夹之前要加上原同步率）。
    fn cent_delta(&self) -> i32 {
        self.equip_num / 5 - self.weapon_num / 2
    }

    /// 魔剑的武，按净化次数夹好。
    fn weapon_wu(&self, count: i32) -> i32 {
        let w = if self.weapon_wu > 0 && self.weapon_num > 0 {
            self.weapon_wu / self.weapon_num
        } else {
            0
        };
        let a = if self.adorn_wu > 0 && self.adorn_num > 0 {
            self.adorn_wu / self.adorn_num
        } else {
            0
        };
        let d = if self.def_fang > 0 && self.def_num > 0 {
            (self.def_fang / self.def_num) as f32 * (0.9 + self.def_num as f32 / 100.) / 5.
        } else {
            0.
        };
        let f = if self.adorn_fang > 0 && self.adorn_num > 0 {
            self.adorn_fang / self.adorn_num
        } else {
            0
        };
        let raw =
            ((w + a + d.round() as i32 + f) as f32 * (1. + count as f32 / 50.)).round() as i32;
        let (lo, hi) = PURGE_LIMITS[count.clamp(0, 10) as usize];
        raw.clamp(lo, hi)
    }
}

/// 一条武器技：号和五灵（数据库顺序水火风雷土，非 0 即有）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WeaponSkill {
    pub id: u32,
    pub elements: [bool; 5],
}

/// 按五块的完全净化找武器技（`Purged` 里那段匹配：页面顺序水火雷风土，所以第 2、3 块对调）。
pub fn match_weapon_skill(all_purged: &[bool; BLOCKS], skills: &[WeaponSkill]) -> u32 {
    let want = [
        all_purged[0],
        all_purged[1],
        all_purged[3],
        all_purged[2],
        all_purged[4],
    ];
    skills
        .iter()
        .find(|s| s.elements == want)
        .map_or(DEFAULT_WEAPON_SKILL, |s| s.id)
}

/// 「是/否」框里预先算的那些（`CheckPurge`）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Preview {
    pub cent: i32,
    pub weapon_wu: i32,
    pub weapon_skill: u32,
}

/// `CheckPurge`：古值是 0 就什么也不做（`None`）。否则**先把次数加一、这一块要是已经有一面净化过就
/// 预先记成完全净化**（按「否」时 [`cancel`] 撤回），算出要显示的数。
pub fn preview(sword: &mut MagicSword, basket: &Basket, skills: &[WeaponSkill]) -> Option<Preview> {
    if basket.gu <= 0 {
        return None;
    }
    let b = sword.block;
    sword.count = (sword.count.max(0) + 1).min(9);
    if sword.purged[0][b] || sword.purged[1][b] {
        sword.all_purged[b] = true;
    }
    let cent = (basket.cent_delta() + sword.clean_cent).clamp(1, 20);
    Some(Preview {
        cent,
        weapon_wu: basket.weapon_wu(sword.count),
        weapon_skill: match_weapon_skill(&sword.all_purged, skills),
    })
}

/// 「否」：撤回 [`preview`] 预先做的。
pub fn cancel(sword: &mut MagicSword) {
    sword.all_purged[sword.block] = false;
    sword.count = (sword.count - 1).max(0);
}

/// 「是」：`Purged()` 加 `Update` 里 `DLG_BT_YES` 那一支。`lunar` 是剑现在朝哪面。龙晶石由调用方扣。
pub fn confirm(sword: &mut MagicSword, basket: &Basket, skills: &[WeaponSkill], lunar: bool) {
    let b = sword.block;
    sword.weapon_skill = match_weapon_skill(&sword.all_purged, skills);
    sword.clean_cent = (basket.cent_delta() + sword.clean_cent).clamp(1, 20);
    sword.weapon_wu = basket.weapon_wu(sword.count);
    if !sword.all_purged[b] {
        if sword.purged[1][b] {
            // 走不到（阳面净化过的话 `preview` 已经记成完全净化），照原样留着。
            sword.purged[0][b] = true;
        } else if lunar {
            sword.purged[0][b] = true;
        } else {
            sword.purged[1][b] = true;
        }
    }
}

/// 剑身贴图上每块的位置（`Purge_rectA` 阳面、`Purge_rectB` 阴面）和净化字在 `J131Dst` 上的位置
/// （`dstRect`）。原版的坐标是**从图的底边往上数**的（`gbImage2D` 的行序）。
const SRC_X: [i32; 2] = [326, 69];
const SRC_Y: [(i32, i32); BLOCKS] = [(102, 35), (137, 25), (162, 26), (188, 25), (213, 24)];
const DST_Y: [i32; BLOCKS] = [0, 35, 60, 86, 111];
const BLOCK_W: i32 = 110;

/// 按净化状态拼剑的贴图（`CheckWSkill` 里那段 `MergeSrcData` / `UpdateMergeData`）：净化过的那一面，
/// 把 `J131Dst` 上那一块的字拷到 `J131Src` 对应位置。`src` / `dst` 是正常（自上而下）读出来的图。
pub fn purge_texture(
    src: &image::RgbaImage,
    dst: &image::RgbaImage,
    sword: &MagicSword,
) -> image::RgbaImage {
    let mut out = src.clone();
    let (sh, dh) = (src.height() as i32, dst.height() as i32);
    for side in 0..2 {
        let lunar = side == 0;
        for b in 0..BLOCKS {
            if !sword.face_purged(b, lunar) {
                continue;
            }
            let (sy, h) = SRC_Y[b];
            for row in 0..h {
                // 自下而上的第 y 行 = 自上而下的第 (高 − 1 − y) 行。
                let s_row = sh - 1 - (sy + row);
                let d_row = dh - 1 - (DST_Y[b] + row);
                if s_row < 0 || d_row < 0 {
                    continue;
                }
                for col in 0..BLOCK_W {
                    let (sx, dx) = (SRC_X[side] + col, col);
                    if sx >= src.width() as i32 || dx >= dst.width() as i32 {
                        continue;
                    }
                    let p = *dst.get_pixel(dx as u32, d_row as u32);
                    out.put_pixel(sx as u32, s_row as u32, p);
                }
            }
        }
    }
    out
}

/// 贴图的缓存名：每种净化状态一张。
pub fn purge_texture_key(sword: &MagicSword) -> String {
    let mut mask = 0u32;
    for b in 0..BLOCKS {
        if sword.face_purged(b, true) {
            mask |= 1 << b;
        }
        if sword.face_purged(b, false) {
            mask |= 1 << (b + 5);
        }
    }
    format!("*J131Purge{mask:03x}.tga")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u32, item_type: u8, gu: i32, wu: i16, fang: i16) -> GameItemInfo {
        let mut it = GameItemInfo::default();
        it.id = id;
        it.item_type = item_type;
        it.ancient_value = gu;
        it.attribute_impact_value[WU] = wu;
        it.attribute_impact_value[FANG] = fang;
        it
    }

    #[test]
    fn gu_must_be_positive_and_total_at_most_300() {
        let mut b = Basket::default();
        assert_eq!(b.add(&item(1, 11, 0, 10, 0), 0), Err(Refusal::NoGu));
        assert_eq!(b.add(&item(1, 11, 200, 10, 0), 0), Ok(()));
        assert_eq!(b.add(&item(2, 7, 101, 0, 5), 1), Err(Refusal::TooMuch));
        assert_eq!(b.add(&item(2, 7, 100, 0, 5), 1), Ok(()));
        assert_eq!(b.gu, 300);
        assert_eq!(b.entries.len(), 2);
    }

    #[test]
    fn the_same_item_stacks_and_comes_back_one_at_a_time() {
        let mut b = Basket::default();
        let it = item(5, 10, 20, 3, 4);
        b.add(&it, 4).unwrap();
        b.add(&it, 4).unwrap();
        assert_eq!(
            b.entries,
            vec![Entry {
                id: 5,
                num: 2,
                page: 4
            }]
        );
        assert_eq!(b.remove(0, &it), Some(5));
        assert_eq!(b.gu, 20);
        assert_eq!(b.remove(0, &it), Some(5));
        assert!(b.is_empty());
    }

    #[test]
    fn weapon_wu_is_averaged_scaled_and_clamped() {
        let mut b = Basket::default();
        b.add(&item(1, 11, 10, 80, 0), 0).unwrap();
        b.add(&item(2, 11, 10, 60, 0), 0).unwrap();
        // 武器均值 70，次数 1：70 × 1.02 = 71.4 → 71，夹在 (60, 90)。
        assert_eq!(b.weapon_wu(1), 71);
        // 次数 0 的上限 70。
        assert_eq!(b.weapon_wu(0), 70);
    }

    #[test]
    fn preview_counts_ahead_and_cancel_takes_it_back() {
        let mut s = MagicSword::default();
        let mut b = Basket::default();
        assert_eq!(preview(&mut s, &b, &[]), None);
        b.add(&item(1, 11, 10, 50, 0), 0).unwrap();
        s.block = 2;
        s.purged[1][2] = true;
        let p = preview(&mut s, &b, &[]).unwrap();
        assert_eq!(s.count, 1);
        assert!(s.all_purged[2]);
        assert_eq!(p.weapon_skill, DEFAULT_WEAPON_SKILL);
        cancel(&mut s);
        assert_eq!(s.count, 0);
        assert!(!s.all_purged[2]);
    }

    #[test]
    fn weapon_skill_matches_with_blocks_2_and_3_swapped() {
        let skills = [WeaponSkill {
            id: 2230,
            // 数据库顺序：风（下标 2）。
            elements: [false, false, true, false, false],
        }];
        let mut all = [false; 5];
        // 页面上第 3 块是风。
        all[3] = true;
        assert_eq!(match_weapon_skill(&all, &skills), 2230);
        all = [false; 5];
        all[2] = true;
        assert_eq!(match_weapon_skill(&all, &skills), DEFAULT_WEAPON_SKILL);
    }

    #[test]
    fn confirm_marks_the_face_the_sword_shows() {
        let mut s = MagicSword::default();
        let mut b = Basket::default();
        b.add(&item(1, 7, 10, 0, 40), 1).unwrap();
        s.block = 0;
        preview(&mut s, &b, &[]).unwrap();
        confirm(&mut s, &b, &[], true);
        assert!(s.purged[0][0] && !s.purged[1][0] && !s.all_purged[0]);
        assert_eq!(s.clean_cent, 10);
        // 防具防均值 40 × 0.91 / 5 = 7.28 → 7，× 1.02 → 7，夹到下限 60。
        assert_eq!(s.weapon_wu, 60);
    }

    #[test]
    fn block_order_maps_to_database_elements() {
        assert_eq!(element_of_block(2), 3);
        assert_eq!(element_of_block(3), 2);
        assert_eq!(MagicSword::default().bonus(), (35, 2));
    }
}
