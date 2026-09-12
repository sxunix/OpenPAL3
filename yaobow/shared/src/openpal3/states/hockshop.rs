//! 当铺经营（SCE 103）的等级表：`SubGame/HockShop/Data/HockShopLevel.hsl`。
//!
//! 这个文件是带注释的文本，等级 E–A 各一段，逐条写着这一级开放哪些功能、
//! 买死当物打几折、剧情段内的死当物总数、每次进店显示几件，以及四种经营
//! 方针下各类物品的比例和红利的加权值。逆向记录在
//! `switch/re/notes/hockshop.md`。
//!
//! 分词器和 DealScript 是同一套（`$…&` 取值、`;` 起注释），关键字表在
//! 0x5824fa 起，每项 30 字节。

/// 五个等级：E D C B A（存档里的等级 0..4 就是这个顺序）。
pub const LEVELS: usize = 5;
/// 四种经营方针：灵丹妙药 / 南北杂货 / 奇珍异宝 / 来者不拒。
pub const MODES: usize = 4;
/// 当期 / 利率 / 拆当率各三档：低 中 高。
pub const TIERS: usize = 3;

/// 经营方针（`Manage` 类型，SCE 120 读的就是它）。
pub const MODE_CATHOLICON: usize = 0;
pub const MODE_CHANDLERY: usize = 1;
pub const MODE_JEWELLERY: usize = 2;
pub const MODE_ANY: usize = 3;

/// 投到 A 级要多少钱（`FUN_00524e00` 里的 500000，拖动条也按
/// `0x7a120` 封顶）。
pub const CAPITAL_FOR_TOP: i32 = 500_000;

/// 累计资本对应几级（`FUN_00524e00`）。
///
/// ```text
/// 比例 = 资本 / 500000
/// 资本 > 500000        → 4 (A)
/// 比例 < 0.25          → 0 (E)
/// 0.25 ≤ 比例 < 0.5    → 1 (D)
/// 0.5  ≤ 比例 < 0.75   → 2 (C)
/// 0.75 ≤ 比例 < 1.0    → 3 (B)
/// 比例 ≥ 1.0           → 4 (A)
/// ```
pub fn level_for_capital(capital: i32) -> i32 {
    if capital < 1 {
        return 0;
    }
    if capital > CAPITAL_FOR_TOP {
        return 4;
    }
    let ratio = capital as f32 / CAPITAL_FOR_TOP as f32;
    if ratio < 0.25 {
        0
    } else if ratio < 0.5 {
        1
    } else if ratio < 0.75 {
        2
    } else if ratio < 1.0 {
        3
    } else {
        4
    }
}

/// 这一次增资最多能投多少：身上的钱和「离顶格还差多少」取小的那个
/// （拖动条在 0x5249fe 一带就是这么夹的）。
pub fn max_investment(capital: i32, money: i32) -> i32 {
    let room = (CAPITAL_FOR_TOP - capital).max(0);
    money.max(0).min(room)
}

/// 投一笔：资本加上去、钱扣下来，等级跟着资本重算
/// （`FUN_00524b00` 一带：`资本 += 投入`、`钱 −= 投入`，再走
/// [`level_for_capital`]）。返回 (新资本, 新的钱, 新等级)。
pub fn invest(capital: i32, money: i32, amount: i32) -> (i32, i32, i32) {
    let amount = amount.clamp(0, max_investment(capital, money));
    let capital = capital + amount;
    (capital, money - amount, level_for_capital(capital))
}

/// 一种经营方针下三类物品的比例（`source.txt` 里那段生成代码按它取物品：
/// 恢复类 type 1、攻击/仙宝 type 2 或 3、古董 type 4）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Proportions {
    pub resume: f32,
    pub attack: f32,
    pub ancient: f32,
}

/// 一种经营方针下的红利加权：当期、利率、拆当率各三档。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DividendWeights {
    pub period: [f32; TIERS],
    pub accrual: [f32; TIERS],
    pub unhock: [f32; TIERS],
}

/// 一个等级的全部数据。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Level {
    /// 卖物品。
    pub sale: bool,
    /// 买死当物品。
    pub buy: bool,
    /// 分红。
    pub melon_cutting: bool,
    /// 经营。
    pub deal_in: bool,
    /// 增资。
    pub invest: bool,
    /// 休息。
    pub rest: bool,
    /// 陈列。
    pub exhibit: bool,
    /// 买死当物品时打几折：1 = 原价，0.7 = 七折。
    pub buy_discount: f32,
    /// 剧情段内死当物的总数量。
    pub prop_num: i32,
    /// 每次进店显示的件数下限 / 上限。
    pub display_min: i32,
    pub display_max: i32,
    /// 四种经营方针各自的物品比例。
    pub proportions: [Proportions; MODES],
    /// 基础红利分配百分比。
    pub base_dividend: f32,
    /// 四种经营方针各自的红利加权。
    pub dividends: [DividendWeights; MODES],
}

impl Level {
    /// 三档加权的和（`FUN_0051f130` 里连着两次 `fadd` 加出来的那个数）。
    pub fn weight_sum(&self, mode: usize, period: usize, accrual: usize, unhock: usize) -> f32 {
        let Some(w) = self.dividends.get(mode) else {
            return 0.;
        };
        let pick = |arr: &[f32; TIERS], i: usize| arr.get(i).copied().unwrap_or(0.);
        pick(&w.period, period) + pick(&w.accrual, accrual) + pick(&w.unhock, unhock)
    }

    /// 这一单红利占资本的比例：`基础红利 × (1 + 三档加权之和)`。
    ///
    /// **加权是乘上去的，不是加上去的**——`FUN_0051f130`：
    ///
    /// ```text
    /// st0 = 资本 × BaseDividend        // 先算好的「基础红利钱」+0x50158
    /// st0 = (加权1 + 加权2 + 加权3) × st0 + st0
    /// ```
    ///
    /// 也就是 `基础红利钱 × (1 + 加权和)`。先前按「基础值加三个加权」写是错
    /// 的，会把 A 级最差的组合算成负数；乘法这一版最差也只是打到 0.25 倍。
    pub fn dividend_rate(&self, mode: usize, period: usize, accrual: usize, unhock: usize) -> f32 {
        self.base_dividend * (1. + self.weight_sum(mode, period, accrual, unhock))
    }

    /// 这一单能分多少钱：`ftol(资本 × dividend_rate)`。
    pub fn dividend(
        &self,
        capital: i32,
        mode: usize,
        period: usize,
        accrual: usize,
        unhock: usize,
    ) -> i32 {
        (capital as f32 * self.dividend_rate(mode, period, accrual, unhock)) as i32
    }
}

/// 整张表。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HockShopLevels {
    pub levels: [Level; LEVELS],
}

impl HockShopLevels {
    /// 存档里的等级（0..4 = E D C B A）对应哪一级。
    pub fn level(&self, level: i32) -> &Level {
        let i = level.clamp(0, LEVELS as i32 - 1) as usize;
        &self.levels[i]
    }

    /// 解析 `HockShopLevel.hsl`（调用方负责 GBK 转 `str`）。
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        let mut level: Option<usize> = None;
        let mut mode: Option<usize> = None;
        let mut dividend: Option<usize> = None;

        for raw in text.lines() {
            let line = raw.split(';').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            // 段落标记：`&` 收物品比例，`*` 收红利，`#` 收整级。
            match line {
                "&" => {
                    mode = None;
                    continue;
                }
                "*" => {
                    dividend = None;
                    continue;
                }
                "#" => {
                    level = None;
                    continue;
                }
                "end" | "End" | "END" => break,
                _ => {}
            }
            if line.starts_with("Begin,") {
                continue;
            }

            if let Some(i) = level_index(line) {
                level = Some(i);
                continue;
            }
            if let Some(i) = mode_index(line) {
                mode = Some(i);
                continue;
            }
            if let Some(i) = dividend_index(line) {
                dividend = Some(i);
                continue;
            }

            let (Some(li), Some((key, value))) = (level, split_entry(line)) else {
                continue;
            };
            let lv = &mut out.levels[li];
            if let Some(mi) = mode {
                let p = &mut lv.proportions[mi];
                match key {
                    "RESUME" => p.resume = number(value),
                    "ATTACK" => p.attack = number(value),
                    "ANCIENT" => p.ancient = number(value),
                    _ => {}
                }
                continue;
            }
            if let Some(di) = dividend {
                let w = &mut lv.dividends[di];
                let (arr, tier) = match key {
                    "lo_PeriodPlus" => (&mut w.period, 0),
                    "mi_PeriodPlus" => (&mut w.period, 1),
                    "hi_PeriodPlus" => (&mut w.period, 2),
                    "lo_AccrualPlus" => (&mut w.accrual, 0),
                    "mi_AccrualPlus" => (&mut w.accrual, 1),
                    "hi_AccrualPlus" => (&mut w.accrual, 2),
                    "lo_UnHockPlus" => (&mut w.unhock, 0),
                    "mi_UnHockPlus" => (&mut w.unhock, 1),
                    "hi_UnHockPlus" => (&mut w.unhock, 2),
                    _ => continue,
                };
                arr[tier] = number(value);
                continue;
            }
            match key {
                "Sale" => lv.sale = number(value) != 0.,
                "Buy" => lv.buy = number(value) != 0.,
                "MelonCutting" => lv.melon_cutting = number(value) != 0.,
                "DealIn" => lv.deal_in = number(value) != 0.,
                "Invest" => lv.invest = number(value) != 0.,
                "Rest" => lv.rest = number(value) != 0.,
                "Exhibit" => lv.exhibit = number(value) != 0.,
                "BuyDiscount" => lv.buy_discount = number(value),
                "PropNum" => lv.prop_num = number(value) as i32,
                "DisMinNum" => lv.display_min = number(value) as i32,
                "DisMaxNum" => lv.display_max = number(value) as i32,
                "BaseDividend" => lv.base_dividend = number(value),
                _ => {}
            }
        }
        out
    }
}

/// `HL_E` … `HL_A` → 0..4。
fn level_index(line: &str) -> Option<usize> {
    let name = line.split_whitespace().next()?;
    match name {
        "HL_E" => Some(0),
        "HL_D" => Some(1),
        "HL_C" => Some(2),
        "HL_B" => Some(3),
        "HL_A" => Some(4),
        _ => None,
    }
}

fn mode_index(line: &str) -> Option<usize> {
    match line.split_whitespace().next()? {
        "CATHOLICON" => Some(MODE_CATHOLICON),
        "CHANDLERY" => Some(MODE_CHANDLERY),
        "JEWELLERY" => Some(MODE_JEWELLERY),
        "ANY" => Some(MODE_ANY),
        _ => None,
    }
}

fn dividend_index(line: &str) -> Option<usize> {
    match line.split_whitespace().next()? {
        "DIVIDEND_CATHOLICON" => Some(MODE_CATHOLICON),
        "DIVIDEND_CHANDLERY" => Some(MODE_CHANDLERY),
        "DIVIDEND_JEWELLERY" => Some(MODE_JEWELLERY),
        "DIVIDEND_ANY" => Some(MODE_ANY),
        _ => None,
    }
}

/// `<键>\t$<值>&`。
fn split_entry(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line.split_once('$')?;
    let value = rest.split('&').next().unwrap_or(rest);
    Some((key.trim(), value.trim()))
}

fn number(value: &str) -> f32 {
    value.trim().parse::<f32>().unwrap_or(0.)
}

/// 进店这一次摆哪几件（`FUN_0051f1b0` 0x51f574 起）：件数 = `DisMinNum + rand % (DisMaxNum − DisMinNum)`
/// （上限取不到；两个相等时原版会除零，这里就取下限）。池里没卖掉的不够就全摆（按池里的
/// 顺序）；够就随机挑、不重复。返回物品 id，按挑出来的顺序。
pub fn pick_display(pool: &[(u32, bool)], min: i32, max: i32, rng: &mut impl PawnRng) -> Vec<u32> {
    let count = if max > min {
        min + rng.below((max - min) as usize) as i32
    } else {
        min
    }
    .max(0) as usize;
    let mut left: Vec<u32> = pool
        .iter()
        .filter(|(id, sold)| *id != 0 && !*sold)
        .map(|(id, _)| *id)
        .collect();
    if count >= left.len() {
        return left;
    }
    let mut out = Vec::with_capacity(count);
    while out.len() < count {
        let k = rng.below(left.len());
        out.push(left.remove(k));
    }
    out
}

/// 攻击/仙宝那一类里要排除的：五灵珠和土灵珠（`source.txt` 里写死的）。
pub const EXCLUDED_ATTACK: [u32; 8] = [6119, 6120, 6121, 6122, 6123, 6124, 6125, 6427];
/// 古董那一类里要排除的：五本仙术书。
pub const EXCLUDED_ANCIENT: [u32; 5] = [6409, 6410, 6411, 6412, 6413];

/// 死当物的随机来源：`source.txt` 里那段循环要的两样东西——掷概率、
/// 从物品表里随机抽一个下标。
pub trait PawnRng {
    /// `CalProbability(p)`：按概率 `p` 掷一次。
    fn chance(&mut self, p: f32) -> bool;
    /// `rand() % item_num`。
    fn below(&mut self, n: usize) -> usize;
}

/// 按 `source.txt` 的循环挑 `total` 件死当物。
///
/// 每一件先按恢复类比例掷，中了就在物品表里随机摸，摸到 `type == 1` 的就
/// 收下；没中或者没摸到再按攻击/仙宝比例掷（`type == 2 || 3`，排除
/// [`EXCLUDED_ATTACK`]），再不成按古董比例掷（`type == 4`，排除
/// [`EXCLUDED_ANCIENT`]）。**重复的不要**。
///
/// `items` 是物品表里的 (id, type) 对，顺序就是数据库顺序；`tries` 是每一
/// 类摸多少次就放弃（原版是把整张表摸一遍，这里给个上限免得死循环）。
pub fn pick_dead_pawn_items(
    total: usize,
    p: Proportions,
    items: &[(u32, u8)],
    rng: &mut impl PawnRng,
) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::new();
    if items.is_empty() {
        return out;
    }
    let tries = items.len();
    // 一轮下来一件都没添上就收手，别在数据不够时空转。
    let mut idle_rounds = 0;
    while out.len() < total && idle_rounds < 8 {
        let before = out.len();
        for (chance, want) in [
            (p.resume, Category::Resume),
            (p.attack, Category::Attack),
            (p.ancient, Category::Ancient),
        ] {
            if out.len() >= total {
                break;
            }
            if !rng.chance(chance) {
                continue;
            }
            for _ in 0..tries {
                let (id, item_type) = items[rng.below(items.len())];
                if out.contains(&id) || !want.accepts(id, item_type) {
                    continue;
                }
                out.push(id);
                break;
            }
        }
        if out.len() == before {
            idle_rounds += 1;
        } else {
            idle_rounds = 0;
        }
    }
    out
}

/// `source.txt` 里那三类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Category {
    Resume,
    Attack,
    Ancient,
}

impl Category {
    fn accepts(self, id: u32, item_type: u8) -> bool {
        match self {
            Category::Resume => item_type == 1,
            Category::Attack => {
                (item_type == 2 || item_type == 3) && !EXCLUDED_ATTACK.contains(&id)
            }
            Category::Ancient => item_type == 4 && !EXCLUDED_ANCIENT.contains(&id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Begin,sec1
; 注释
HL_E\t\t\t\t\t;E级当铺数据
Sale\t\t\t$1&\t\t;卖物品
Buy\t\t\t\t$0&\t\t;买死党物品
Invest\t\t\t$1&\t\t;增资
BuyDiscount\t\t$1& \t;打折
PropNum\t\t\t$0&\t \t;总数量
DisMinNum\t\t$0&
DisMaxNum\t\t$0&
CATHOLICON
RESUME\t\t\t$0&
ATTACK\t\t\t$0&
ANCIENT\t\t\t$0&
&
BaseDividend\t$0.0&
#
HL_D
Sale\t\t\t$1&
Buy\t\t\t\t$1&
MelonCutting\t$1&
Exhibit\t\t\t$1&
BuyDiscount\t\t$1&
PropNum\t\t\t$10&
DisMinNum\t\t$2&
DisMaxNum\t\t$4&
CATHOLICON
RESUME\t\t\t$0.88&
ATTACK\t\t\t$0.1&
ANCIENT\t\t\t$0.02&
&
CHANDLERY
RESUME\t\t\t$0.58&
ATTACK\t\t\t$0.4&
ANCIENT\t\t\t$0.02&
&
BaseDividend\t$0.2&
DIVIDEND_CATHOLICON
lo_PeriodPlus \t$0.1&
mi_PeriodPlus\t$0.05&
hi_PeriodPlus\t$0.0&
lo_AccrualPlus\t$0.0&
mi_AccrualPlus\t$0.0&
hi_AccrualPlus\t$0.0&
lo_UnHockPlus\t$0.1&
mi_UnHockPlus\t$0.0&
hi_UnHockPlus\t$-0.1&
*
#
end
";

    #[test]
    fn the_level_flags_come_out_per_level() {
        let t = HockShopLevels::parse(SAMPLE);
        let e = t.level(0);
        assert!(e.sale && e.invest);
        assert!(!e.buy && !e.melon_cutting && !e.exhibit);
        assert_eq!(e.buy_discount, 1.0);
        assert_eq!(e.prop_num, 0);

        let d = t.level(1);
        assert!(d.buy && d.melon_cutting && d.exhibit);
        assert_eq!(d.prop_num, 10);
        assert_eq!((d.display_min, d.display_max), (2, 4));
        assert_eq!(d.base_dividend, 0.2);
    }

    #[test]
    fn each_mode_keeps_its_own_proportions() {
        let t = HockShopLevels::parse(SAMPLE);
        let d = t.level(1);
        assert_eq!(d.proportions[MODE_CATHOLICON].resume, 0.88);
        assert_eq!(d.proportions[MODE_CHANDLERY].attack, 0.4);
        // 没写的那两种保持 0。
        assert_eq!(d.proportions[MODE_JEWELLERY], Proportions::default());
    }

    #[test]
    fn the_dividend_rate_multiplies_the_base_by_one_plus_the_weights() {
        let t = HockShopLevels::parse(SAMPLE);
        let d = t.level(1);
        // 短当期 +0.1、低利率 0、低拆当率 +0.1 → 0.2 × (1 + 0.2) = 0.24。
        assert!((d.dividend_rate(MODE_CATHOLICON, 0, 0, 0) - 0.24).abs() < 1e-6);
        // 短当期 +0.1、低利率 0、高拆当率 −0.1 → 0.2 × 1.0 = 0.2。
        assert!((d.dividend_rate(MODE_CATHOLICON, 0, 0, 2) - 0.2).abs() < 1e-6);
        // 没有加权数据的方针就是基础值本身。
        assert!((d.dividend_rate(MODE_ANY, 0, 0, 0) - 0.2).abs() < 1e-6);
        // 分红的钱：资本 × 比例，向下取整。
        assert_eq!(d.dividend(100_000, MODE_CATHOLICON, 0, 0, 0), 24_000);
    }

    #[test]
    fn the_capital_ladder_matches_the_binary() {
        // 五档的边界：500000 的 0.25 / 0.5 / 0.75 / 1.0。
        assert_eq!(level_for_capital(0), 0);
        assert_eq!(level_for_capital(124_999), 0);
        assert_eq!(level_for_capital(125_000), 1);
        assert_eq!(level_for_capital(249_999), 1);
        assert_eq!(level_for_capital(250_000), 2);
        assert_eq!(level_for_capital(374_999), 2);
        assert_eq!(level_for_capital(375_000), 3);
        assert_eq!(level_for_capital(499_999), 3);
        assert_eq!(level_for_capital(500_000), 4);
        assert_eq!(level_for_capital(999_999), 4);
    }

    #[test]
    fn investing_is_capped_by_money_and_by_the_top() {
        // 身上的钱不够时以钱为准。
        assert_eq!(max_investment(0, 300), 300);
        // 离顶格只差一点时以差额为准。
        assert_eq!(max_investment(499_000, 100_000), 1_000);
        assert_eq!(max_investment(CAPITAL_FOR_TOP, 100_000), 0);

        let (capital, money, level) = invest(120_000, 50_000, 10_000);
        assert_eq!((capital, money), (130_000, 40_000));
        assert_eq!(level, 1, "过了 125000 就是 D 级");

        // 想投超过上限的，按上限投。
        let (capital, money, level) = invest(499_000, 10_000, 9_999);
        assert_eq!((capital, money, level), (500_000, 9_000, 4));
    }

    /// 掷骰按脚本给的序列走，好把生成过程钉死。
    struct ScriptedRng {
        chances: Vec<bool>,
        picks: Vec<usize>,
    }

    impl PawnRng for ScriptedRng {
        fn chance(&mut self, _p: f32) -> bool {
            if self.chances.is_empty() {
                return false;
            }
            self.chances.remove(0)
        }
        fn below(&mut self, n: usize) -> usize {
            if self.picks.is_empty() {
                return 0;
            }
            self.picks.remove(0) % n
        }
    }

    fn item_table() -> Vec<(u32, u8)> {
        vec![
            (6201, 1), // 恢复
            (6202, 1),
            (6101, 2), // 投掷
            (6119, 2), // 五灵珠，要排除
            (6427, 3), // 土灵珠，要排除
            (6401, 3), // 仙宝
            (6017, 4), // 古董
            (6409, 4), // 仙术书，要排除
        ]
    }

    #[test]
    fn the_generator_follows_the_shipped_source() {
        let p = Proportions {
            resume: 1.0,
            attack: 1.0,
            ancient: 1.0,
        };
        // 第一轮三类各中一次：恢复摸到 6201、攻击先摸到要排除的 6119 再摸到
        // 6101、古董先摸到 6409 再摸到 6017。
        let mut rng = ScriptedRng {
            chances: vec![true, true, true],
            picks: vec![0, 3, 2, 7, 6],
        };
        let got = pick_dead_pawn_items(3, p, &item_table(), &mut rng);
        assert_eq!(got, vec![6201, 6101, 6017]);
    }

    #[test]
    fn excluded_ids_never_show_up() {
        let p = Proportions {
            resume: 0.,
            attack: 1.0,
            ancient: 1.0,
        };
        // 一直摸到要排除的那几个，最后什么也添不上，函数得收手而不是空转。
        let mut rng = ScriptedRng {
            chances: vec![true; 64],
            picks: vec![3; 256],
        };
        let got = pick_dead_pawn_items(3, p, &item_table(), &mut rng);
        assert!(got.is_empty(), "五灵珠不该被当成死当物：{got:?}");
    }

    #[test]
    fn nothing_repeats() {
        let p = Proportions {
            resume: 1.0,
            attack: 0.,
            ancient: 0.,
        };
        let mut rng = ScriptedRng {
            chances: vec![true; 64],
            picks: vec![0; 256],
        };
        // 只摸得到 6201 一件，第二件添不上就收手。
        let got = pick_dead_pawn_items(3, p, &item_table(), &mut rng);
        assert_eq!(got, vec![6201]);
    }

    #[test]
    fn an_out_of_range_level_clamps() {
        let t = HockShopLevels::parse(SAMPLE);
        assert_eq!(t.level(-3), t.level(0));
        assert_eq!(t.level(99), t.level(LEVELS as i32 - 1));
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    /// 按给定顺序吐数的骰子。
    struct Seq(Vec<usize>);
    impl PawnRng for Seq {
        fn chance(&mut self, _: f32) -> bool {
            true
        }
        fn below(&mut self, n: usize) -> usize {
            if self.0.is_empty() {
                0
            } else {
                self.0.remove(0) % n.max(1)
            }
        }
    }

    #[test]
    fn a_visit_shows_unsold_pieces_only() {
        let pool = [(1, false), (2, true), (3, false), (4, false), (0, false)];
        // 件数 2 + 1 = 3，没卖的正好 3 件：全摆，按池里的顺序。
        assert_eq!(pick_display(&pool, 2, 4, &mut Seq(vec![1])), vec![1, 3, 4]);
        // 件数 2：从 [1, 3, 4] 里挑下标 2、再挑剩下 [1, 3] 的下标 0。
        assert_eq!(
            pick_display(&pool, 2, 4, &mut Seq(vec![0, 2, 0])),
            vec![4, 1]
        );
        // 上下限相等：就是下限。
        assert_eq!(pick_display(&pool, 1, 1, &mut Seq(vec![1])).len(), 1);
        assert!(pick_display(&[], 3, 5, &mut Seq(vec![])).is_empty());
    }
}
