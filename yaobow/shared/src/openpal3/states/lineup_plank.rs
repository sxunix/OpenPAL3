//! 阵法页每个人那块「设置板」：六个托管开关（参考源码 `gamemainui/lineupui.cpp`）。
//!
//! 按钮下标（`LINEUP_TYPE`）：0 攻击、1 特技、2 仙术、3 物品、4 全力 / 万全、5 AI；
//! 贴图 `Lineup\Lineup<下标>0/1.tga`（弹起 / 按下）。每格的状态 `Info`：0 不能点、1 能点没按、2 按下。
//!
//! - 进页面（`LoadMgrData` + `CheckLineupPlank`）：AI 开着的人六个全能点、为真的那几格按下；
//!   AI 关着的人只有 AI 能点、全部弹起。
//! - 点一格（`UIWM_COMMAND` 里 `IDB_LINEUP_STATE`）：不能点 → wa004；能点就在按下 / 弹起间切换，放 wa001。
//!   点的是 AI 或攻击：按下时六个全能点、AI 和攻击都按下；弹起时全清、只剩 AI 能点（**攻击弹起也会把 AI
//!   关掉**，照原样）。
//! - 写回（`WriteBackMgr`，离开页面时整块写）：按下的那几格为真。第 4 格按下是 `bAINegative`「万全」，
//!   弹起是「全力」。AI 关着的人读进来时全清了，所以进过这一页，新档默认的「万全」也会被写成假——照原样。

use crate::openpal3::combat::ai::Delegation;

pub const ATTACK: usize = 0;
pub const SKILL: usize = 1;
pub const MAGIC: usize = 2;
pub const PROP: usize = 3;
pub const STRIVE: usize = 4;
pub const AI: usize = 5;
pub const BUTTONS: usize = 6;

/// 一块设置板。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Plank {
    /// `Info`：0 不能点、1 能点没按、2 按下。
    info: [u8; BUTTONS],
    enabled: [bool; BUTTONS],
}

impl Plank {
    /// 进页面时按角色的托管设置摆好。
    pub fn load(d: &Delegation) -> Self {
        let on = |b: bool| if b { 2 } else { 0 };
        let mut info = [0u8; BUTTONS];
        info[ATTACK] = on(d.attack);
        info[SKILL] = on(d.special);
        info[MAGIC] = on(d.magic);
        info[PROP] = on(d.item);
        info[STRIVE] = on(d.cautious);
        info[AI] = on(d.on);
        let mut p = Self {
            info,
            enabled: [false; BUTTONS],
        };
        if info[AI] == 2 {
            p.enabled = [true; BUTTONS];
        } else {
            p.info = [0; BUTTONS];
            p.enabled[AI] = true;
        }
        p
    }

    pub fn pushed(&self, b: usize) -> bool {
        self.info.get(b) == Some(&2)
    }

    pub fn enabled(&self, b: usize) -> bool {
        self.enabled.get(b).copied().unwrap_or(false)
    }

    /// 点第 `b` 格。返回点没点进去（false 时放 wa004）。
    pub fn click(&mut self, b: usize) -> bool {
        if !self.enabled(b) {
            return false;
        }
        let pushed = !self.pushed(b);
        self.info[b] = if pushed { 2 } else { 1 };
        if b == AI || b == ATTACK {
            if pushed {
                self.enabled = [true; BUTTONS];
                self.info[AI] = 2;
                self.info[ATTACK] = 2;
            } else {
                self.info = [0; BUTTONS];
                self.info[AI] = 1;
                self.enabled = [false; BUTTONS];
                self.enabled[AI] = true;
            }
        }
        true
    }

    /// 写回角色记录的托管开关。
    pub fn delegation(&self) -> Delegation {
        Delegation {
            on: self.pushed(AI),
            attack: self.pushed(ATTACK),
            magic: self.pushed(MAGIC),
            special: self.pushed(SKILL),
            item: self.pushed(PROP),
            cautious: self.pushed(STRIVE),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_off_leaves_only_the_ai_button_live() {
        let p = Plank::load(&Delegation::default());
        assert!(p.enabled(AI) && !p.enabled(ATTACK) && !p.pushed(AI));
    }

    #[test]
    fn ai_on_presses_attack_too_and_unlocks_the_rest() {
        let mut p = Plank::load(&Delegation::default());
        assert!(!p.clone().click(MAGIC));
        assert!(p.click(AI));
        assert!(p.pushed(AI) && p.pushed(ATTACK) && p.enabled(MAGIC) && !p.pushed(MAGIC));
        assert!(p.click(MAGIC));
        let d = p.delegation();
        assert!(d.on && d.attack && d.magic && !d.special && !d.cautious);
    }

    #[test]
    fn releasing_attack_turns_the_whole_plank_off() {
        let mut p = Plank::load(&Delegation::all_on());
        assert!(p.click(ATTACK));
        assert!(!p.pushed(AI) && !p.enabled(MAGIC));
        let d = p.delegation();
        assert!(!(d.on || d.attack || d.magic || d.special || d.item || d.cautious));
    }

    #[test]
    fn loaded_settings_show_pressed() {
        let d = Delegation {
            on: true,
            attack: true,
            cautious: true,
            ..Default::default()
        };
        let p = Plank::load(&d);
        assert!(p.pushed(STRIVE) && p.enabled(SKILL) && !p.pushed(SKILL));
        assert_eq!(p.delegation(), d);
    }
}
