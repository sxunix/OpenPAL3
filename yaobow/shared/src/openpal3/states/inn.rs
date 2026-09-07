//! 客栈住店（SCE 119 `UIShowHotelMenu`）。
//!
//! 配置的解析在 [`super::shop::RestScript`]（和店铺共用
//! `DealScript/` 那个目录，但格式是另一套）。这里放的是**住一晚发生了
//! 什么**——照 `PAL3_unpacked.exe` 里那个客栈界面对象的每帧函数
//! （`0x44b0f0`，Ghidra 没识别出来，是读反汇编读的）写的，逆向记录在
//! `switch/re/notes/inn.md`。

/// 住店回哪几项：**精（`hp`）和神（`mp`）回满，气（`sp`）不回**。
///
/// 原版 0x44b2d7 那个循环：
///
/// ```text
/// eax = 0xc0476c                       // 角色 0 的 +0x3cc = 当前精
/// ecx = 0
/// do {
///     if ((&DAT_00c05f58)[ecx] != 0) {  // 这个角色在队里
///         [eax]     = [eax - 0x30]      // +0x3cc = +0x39c  当前精 = 精上限
///         [eax + 8] = [eax - 0x28]      // +0x3d4 = +0x3a4  当前神 = 神上限
///     }
///     eax += 0x580; ecx++;
/// } while (eax < 0xc062ec);             // 五个角色
/// ```
///
/// 中间那一项（`+0x3d0` 当前气 ← `+0x3a0` 气上限）**没有被写**——不是漏
/// 读，那两条指令不在循环里。
pub const RESTORES_SP: bool = false;

/// 住店会顺手推进的剧情号（`g_story`，脚本里的全局变量 −32768）。
///
/// 原版 0x44b317：`FUN_004343f0("g_story")` 取到变量之后
///
/// ```text
/// if (值 == 0x2b5d)   值 = 0x2bc0;      // 11101 → 11200
/// else if (值 == 0x1d5ed) 值 = 0x1d5ee; // 120301 → 120302
/// ```
pub const STORY_ADVANCES: [(i32, i32); 2] = [(11101, 11200), (120301, 120302)];

/// 脚本里 `g_story` 的变量号。
pub const G_STORY: i16 = -32768;

/// 住完之后 `g_story` 变成多少（不在表里就原样）。
pub fn story_after_rest(g_story: i32) -> i32 {
    STORY_ADVANCES
        .iter()
        .find(|(from, _)| *from == g_story)
        .map(|(_, to)| *to)
        .unwrap_or(g_story)
}

/// 钱够不够住（原版 `cmp 钱, 价格; jge 够`，所以**等于也算够**）。
pub fn can_afford(money: i32, price: i32) -> bool {
    money >= price
}

/// 玩家在「要不要住店」那个是/否框里选了什么
/// （原版 `FUN_004a1cf0()`：0 = 住，1 = 不住）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Rest,
    Leave,
}

/// 住一晚的结果，命令层照着走。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestOutcome {
    /// 钱够，住下了：扣了 `paid` 文，跑 `enough_money_proc` 再跑
    /// `after_rest_proc`。
    Rested { paid: i32 },
    /// 钱不够：一文没扣，跑 `not_enough_money_proc`。
    TooPoor,
    /// 选了不住。
    Left,
}

/// 一个角色的三项，回血就在这上面做。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Vitals {
    pub hp: i32,
    pub max_hp: i32,
    pub sp: i32,
    pub max_sp: i32,
    pub mp: i32,
    pub max_mp: i32,
}

impl Vitals {
    /// 住一晚：精和神回满，气不动。
    pub fn rest(&mut self) {
        self.hp = self.max_hp;
        self.mp = self.max_mp;
        if RESTORES_SP {
            self.sp = self.max_sp;
        }
    }
}

/// 结算一次住店。`in_party` 说五个槽里哪几个在队上——原版按
/// `(&DAT_00c05f58)[i]` 挑，不在队上的不回。
pub fn rest(
    answer: Answer,
    money: &mut i32,
    price: i32,
    party: &mut [(bool, Vitals)],
    g_story: &mut i32,
) -> RestOutcome {
    if answer == Answer::Leave {
        return RestOutcome::Left;
    }
    if !can_afford(*money, price) {
        return RestOutcome::TooPoor;
    }

    // 原版的顺序是先回血、再扣钱、最后改剧情号。
    for (in_party, vitals) in party.iter_mut() {
        if *in_party {
            vitals.rest();
        }
    }
    *money -= price;
    *g_story = story_after_rest(*g_story);
    RestOutcome::Rested { paid: price }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hurt() -> Vitals {
        Vitals {
            hp: 3,
            max_hp: 300,
            sp: 4,
            max_sp: 40,
            mp: 5,
            max_mp: 200,
        }
    }

    #[test]
    fn a_night_refills_jing_and_shen_but_not_qi() {
        let mut v = hurt();
        v.rest();
        assert_eq!(v.hp, 300, "精要回满");
        assert_eq!(v.mp, 200, "神要回满");
        assert_eq!(v.sp, 4, "气不回——原版那个循环只写了两项");
    }

    #[test]
    fn only_the_party_gets_the_bed() {
        let mut money = 100;
        let mut story = 0;
        let mut party = [(true, hurt()), (false, hurt()), (true, hurt())];
        let out = rest(Answer::Rest, &mut money, 50, &mut party, &mut story);
        assert_eq!(out, RestOutcome::Rested { paid: 50 });
        assert_eq!(money, 50);
        assert_eq!(party[0].1.hp, 300);
        assert_eq!(party[1].1.hp, 3, "不在队上的不回");
        assert_eq!(party[2].1.hp, 300);
    }

    #[test]
    fn exactly_enough_is_enough() {
        assert!(can_afford(50, 50), "原版是 jge，等于也住得起");
        assert!(!can_afford(49, 50));

        let mut money = 50;
        let mut story = 0;
        let mut party = [(true, hurt())];
        assert_eq!(
            rest(Answer::Rest, &mut money, 50, &mut party, &mut story),
            RestOutcome::Rested { paid: 50 }
        );
        assert_eq!(money, 0);
    }

    #[test]
    fn no_money_means_no_bed_and_no_charge() {
        let mut money = 49;
        let mut story = 11101;
        let mut party = [(true, hurt())];
        assert_eq!(
            rest(Answer::Rest, &mut money, 50, &mut party, &mut story),
            RestOutcome::TooPoor
        );
        assert_eq!(money, 49, "钱不够不能扣钱");
        assert_eq!(party[0].1.hp, 3, "钱不够不能回血");
        assert_eq!(story, 11101, "钱不够剧情也不动");
    }

    #[test]
    fn walking_away_changes_nothing() {
        let mut money = 999;
        let mut story = 11101;
        let mut party = [(true, hurt())];
        assert_eq!(
            rest(Answer::Leave, &mut money, 50, &mut party, &mut story),
            RestOutcome::Left
        );
        assert_eq!(money, 999);
        assert_eq!(party[0].1.hp, 3);
        assert_eq!(story, 11101);
    }

    #[test]
    fn a_night_can_advance_the_story() {
        assert_eq!(story_after_rest(11101), 11200);
        assert_eq!(story_after_rest(120301), 120302);
        // 别的剧情号住店不动。
        assert_eq!(story_after_rest(11200), 11200);
        assert_eq!(story_after_rest(0), 0);
        assert_eq!(story_after_rest(130400), 130400);

        let mut money = 100;
        let mut story = 120301;
        let mut party = [(true, hurt())];
        rest(Answer::Rest, &mut money, 1, &mut party, &mut story);
        assert_eq!(story, 120302);
    }
}
