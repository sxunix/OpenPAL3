//! 买卖：`basedata/DealScript/**/*.txt` 的解析，以及还价的掷骰与结算。
//!
//! 完整的逆向记录在 `switch/re/notes/shop.md`。数据文件是 GBK 文本，分词器
//! 是 `FUN_00457490`（关键字表 0x5b1a50），价格与还价在 `FUN_00461c50` /
//! `FUN_00460a90`。
//!
//! 卖那一半在卖页那个子对象里（`FUN_00471590` 一支）：卖价是
//! `price / 2`，不还价，剧情物品不能卖——见 `shop.md` §5。

use std::collections::HashMap;

/// 店铺类型（`ST`）。
pub const SHOP_MEDICINE: u8 = 0;
pub const SHOP_WEAPON: u8 = 1;
pub const SHOP_GENERAL: u8 = 2;
pub const SHOP_MERCHANT: u8 = 3;

/// 五个折扣档，下标 0..4 就是原版的 tier。
pub const TIERS: usize = 5;
/// 每种商品最多买到这么多个（`FUN_0045b7a0`：可买数 = 99 − 已持有）。
pub const STOCK_CAP: i32 = 99;

/// 一个折扣档的数据。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiscountTier {
    /// 接受概率。负数表示「每次现掷一个 0..99 的百分数当概率」。
    pub accept_chance: f32,
    /// 拒绝概率，同上。
    pub refuse_chance: f32,
    /// 严词拒绝概率。**原版不读它**——两次都没掷中就是严词拒绝；留着是为了
    /// 对得上数据文件。
    pub harsh_chance: f32,
    pub accept_talk: Vec<String>,
    pub refuse_talk: Vec<String>,
    pub harsh_talk: Vec<String>,
    pub force_accept_talk: Vec<String>,
    pub force_accept_reply: String,
}

/// 一个 DealScript 文件描述的店。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DealScript {
    /// `ST`：0 药店 1 武器店 2 杂货店 3 商人。
    pub shop_type: u8,
    /// `DK`：能不能还价（原版对 >1 的值一律按 1）。
    pub haggling: bool,
    /// `M_ID` 商人的角色 id。
    pub merchant_id: u16,
    pub merchant_name: String,
    /// `MC`：0 和善 1 暴躁 2 敦厚 3 软弱 4 谨慎 5 喜怒无常。
    pub temperament: u8,
    /// `Patience`：拒绝到第几次就强制成交。
    pub patience: i32,

    pub begin_talk: String,
    pub entry_buy: String,
    pub entry_sale: String,
    pub normal_accept_talk: String,
    pub normal_cancel_talk: String,
    pub no_enough_money: String,
    pub leave_talk: String,
    /// 被严词拒绝之后再来买东西时的话。
    pub harsh_talk_buy: String,

    /// 五折…九折。
    pub tiers: [DiscountTier; TIERS],

    /// `Equip` 段里的物品 id。
    pub equipment: Vec<u32>,
    /// `Prop` 段里的物品 id。
    pub props: Vec<u32>,
}

impl DealScript {
    /// 店里卖的全部东西，装备在前、道具在后——原版的两段列表就是这个顺序。
    pub fn goods(&self) -> Vec<u32> {
        let mut all = self.equipment.clone();
        all.extend(self.props.iter().copied());
        all
    }

    /// 解析一个 DealScript 文件（调用方负责把 GBK 转成 `str`）。
    ///
    /// 原版的分词器只认 `$…&` 里的值、`;` 起注释，段用 `FIVE`…`#` 这样的
    /// 裸词开头、`#` 结尾。同一个键出现多次时后一次覆盖前一次。
    pub fn parse(text: &str) -> Self {
        let mut shop = Self::default();
        let mut section = Section::Header;

        for raw in text.lines() {
            let line = raw.split(';').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line == "#" {
                section = Section::Header;
                continue;
            }
            match line {
                "FIVE" => {
                    section = Section::Tier(0);
                    continue;
                }
                "SIX" => {
                    section = Section::Tier(1);
                    continue;
                }
                "SEVEN" => {
                    section = Section::Tier(2);
                    continue;
                }
                "EIGHT" => {
                    section = Section::Tier(3);
                    continue;
                }
                "NINE" => {
                    section = Section::Tier(4);
                    continue;
                }
                "Equip" => {
                    section = Section::Equipment;
                    continue;
                }
                "Prop" => {
                    section = Section::Props;
                    continue;
                }
                "end" | "END" => break,
                _ => {}
            }
            if line.starts_with("Begin,") || line.starts_with("BEGIN,") {
                continue;
            }

            let Some((key, value)) = split_entry(line) else {
                continue;
            };
            match section {
                Section::Header => shop.set_header(key, value),
                Section::Tier(t) => shop.tiers[t].set(key, value),
                Section::Equipment | Section::Props => {
                    if key == "Item_ID" {
                        if let Ok(id) = value.trim().parse::<u32>() {
                            if section == Section::Equipment {
                                shop.equipment.push(id);
                            } else {
                                shop.props.push(id);
                            }
                        }
                    }
                }
            }
        }
        shop
    }

    fn set_header(&mut self, key: &str, value: &str) {
        let text = || unescape(value);
        match key {
            "ST" => self.shop_type = parse_int(value) as u8,
            // 原版：`if (1 < v) v = 1`.
            "DK" => self.haggling = parse_int(value) >= 1,
            "M_ID" => self.merchant_id = parse_int(value) as u16,
            "M_Name" => self.merchant_name = text(),
            "MC" => self.temperament = parse_int(value) as u8,
            "Patience" => self.patience = parse_int(value),
            "BeginTalk" => self.begin_talk = text(),
            "EntryBuy" => self.entry_buy = text(),
            "EntrySale" => self.entry_sale = text(),
            "NormalAcptTalk" => self.normal_accept_talk = text(),
            "NormalCnslTalk" => self.normal_cancel_talk = text(),
            "NoEnoughMoney" => self.no_enough_money = text(),
            "Leave" => self.leave_talk = text(),
            "HarshTalkbuy" => self.harsh_talk_buy = text(),
            _ => {}
        }
    }
}

impl DiscountTier {
    fn set(&mut self, key: &str, value: &str) {
        match key {
            "AcptQuantum" => self.accept_chance = parse_float(value),
            "RfusQuantum" => self.refuse_chance = parse_float(value),
            "HarshQuantum" => self.harsh_chance = parse_float(value),
            _ => {
                let (list, _) = match key.trim_end_matches(|c: char| c.is_ascii_digit()) {
                    "AcptTalk" => (&mut self.accept_talk, ()),
                    "RfusTalk" => (&mut self.refuse_talk, ()),
                    "HarshTalk" => (&mut self.harsh_talk, ()),
                    "ForceAcptTalkto" => (&mut self.force_accept_talk, ()),
                    "ForceAcptTalkback" => {
                        self.force_accept_reply = unescape(value);
                        return;
                    }
                    _ => return,
                };
                list.push(unescape(value));
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    Header,
    Tier(usize),
    Equipment,
    Props,
}

/// `<键> $<值>&`，值里可以有空格和中文。
fn split_entry(line: &str) -> Option<(&str, &str)> {
    let (key, rest) = line.split_once('$')?;
    let value = rest.rsplit_once('&').map(|(v, _)| v).unwrap_or(rest);
    Some((key.trim(), value))
}

fn parse_int(value: &str) -> i32 {
    value.trim().parse::<i32>().unwrap_or(0)
}

fn parse_float(value: &str) -> f32 {
    value.trim().parse::<f32>().unwrap_or(0.)
}

/// 台词里的 `\n` 是换行。
fn unescape(value: &str) -> String {
    value.replace("\\n", "\n")
}

/// 客栈的配置（`DealScript/Rest/*.txt`，`FUN_0044b8a0` 解析，关键字表在
/// 0x59ba98）。和店铺是两套格式，只是放在同一个目录下。
///
/// SCE 119 `UIShowHotelMenu` 传的是「文件名 + 钱够 / 钱不够 / 住完」三个过程
/// 号，实机里那三个过程只有一句对白（Q01 1421/1422/1423），所以配置里的
/// `M_Enough` / `M_NoEnough` / `M_RestEnd` 和它们是重复的。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RestScript {
    /// `M_ID`：掌柜的对话脚本编号，0 无效。
    pub keeper_id: u16,
    pub keeper_name: String,
    /// `M_RestPrice`：住一晚多少钱，不是数字按 0 算。
    pub price: i32,
    /// `M_Rest`：问住不住。
    pub ask: String,
    /// `M_Enough`：钱够时的欢迎词。
    pub enough: String,
    /// `M_NoEnough`：钱不够时的话。
    pub not_enough: String,
    /// `M_RestEnd`：住宿完毕。
    pub rest_end: String,
}

impl RestScript {
    pub fn parse(text: &str) -> Self {
        let mut inn = Self::default();
        for raw in text.lines() {
            let line = raw.split(';').next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with("Begin,") || line == "end" {
                continue;
            }
            let Some((key, value)) = split_entry(line) else {
                continue;
            };
            match key {
                "M_ID" => inn.keeper_id = parse_int(value) as u16,
                "M_Name" => inn.keeper_name = unescape(value),
                "M_RestPrice" => inn.price = parse_int(value),
                "M_Rest" => inn.ask = unescape(value),
                "M_Enough" => inn.enough = unescape(value),
                "M_NoEnough" => inn.not_enough = unescape(value),
                "M_RestEnd" => inn.rest_end = unescape(value),
                _ => {}
            }
        }
        inn
    }
}

/// 一次还价的结果（`FUN_00461c50` 写进 `+0x19d90` 的那个状态）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Haggle {
    /// 状态 0：拒绝，不成交。
    Refused,
    /// 状态 1：接受，按折后价成交。
    Accepted,
    /// 状态 2：忍耐度用光，强制成交——付的是**原价**。
    ForceAccepted,
    /// 状态 3：严词拒绝。此后再来买东西说的是 `HarshTalkbuy`。
    Harsh,
    /// 折后价都付不起：弹 `NoEnoughMoney`，不掷骰。
    NotEnoughMoney,
}

/// 原版的随机数（`FUN_005541f8`：MSVC 的 `rand`，只是走另一颗种子）。
#[derive(Debug, Clone)]
pub struct ShopRng {
    seed: u32,
}

impl ShopRng {
    pub fn new(seed: u32) -> Self {
        Self { seed }
    }

    fn next(&mut self) -> u32 {
        self.seed = self.seed.wrapping_mul(0x343F_D).wrapping_add(0x269E_C3);
        self.seed
    }

    /// `rand()`：15 位。
    pub fn rand15(&mut self) -> i32 {
        (self.next() >> 16 & 0x7FFF) as i32
    }

    /// 掷骰用的 10 位（`(seed >> 16) & 0x3ff`）。
    pub fn rand10(&mut self) -> i32 {
        (self.next() >> 16 & 0x3FF) as i32
    }
}

/// `FUN_00460a00(p)`：概率 `p`（0..1）掷一次。
///
/// ```text
/// n = ftol(p × 100)                 // 截断
/// n <= 0 → 假
/// (seed>>16 & 0x3ff) < round(n × 10.24)
/// ```
pub fn roll(rng: &mut ShopRng, chance: f32) -> bool {
    let percent = (chance * 100.0) as i32;
    if percent <= 0 {
        return false;
    }
    let threshold = (percent as f32 * 10.24).round() as i32;
    rng.rand10() < threshold
}

/// 折后价：`总价 × (5 + tier) / 10`，整数除法（`FUN_00460a90`）。
pub fn discounted(total: i32, tier: usize) -> i32 {
    if tier >= TIERS {
        return total;
    }
    total * (5 + tier as i32) / 10
}

/// 还一次价（`FUN_00461c50`）。
///
/// `refusals` 是这家店已经拒绝过几次，接受或强制成交之后由调用方清零；
/// 掷中拒绝时它会被加一。概率是负数的档表示「现掷一个 0..99 的百分数当
/// 概率」，也照抄。
pub fn haggle(
    shop: &DealScript,
    tier: usize,
    total: i32,
    money: i32,
    refusals: &mut i32,
    rng: &mut ShopRng,
) -> Haggle {
    if tier >= TIERS {
        return Haggle::Refused;
    }
    if money < discounted(total, tier) {
        return Haggle::NotEnoughMoney;
    }

    let t = &shop.tiers[tier];
    let accept = resolve_chance(t.accept_chance, rng);
    let refuse = resolve_chance(t.refuse_chance, rng);

    if roll(rng, accept) {
        return Haggle::Accepted;
    }
    if roll(rng, refuse) {
        *refusals += 1;
        if *refusals >= shop.patience {
            return Haggle::ForceAccepted;
        }
        return Haggle::Refused;
    }
    Haggle::Harsh
}

/// 负概率 → 现掷一个 `rand15() % 100 * 0.01`。
fn resolve_chance(chance: f32, rng: &mut ShopRng) -> f32 {
    if chance >= 0. {
        chance
    } else {
        (rng.rand15() % 100) as f32 * 0.01
    }
}

/// 成交时要付多少钱：接受按折后价，强制成交按原价。
pub fn price_to_pay(outcome: Haggle, total: i32, tier: usize) -> Option<i32> {
    match outcome {
        Haggle::Accepted => Some(discounted(total, tier)),
        Haggle::ForceAccepted => Some(total),
        _ => None,
    }
}

/// 这一件还能买几个：`99 − 已持有`，下限 0。
pub fn buyable(held: u32) -> i32 {
    (STOCK_CAP - held as i32).max(0)
}

/// 卖一件能拿多少：**`price / 2`，整数除法**（`FUN_004765f0` 里
/// `(*(int *)(… + 0x260) / 2) * 个数`，列表每行显示的也是这个数）。
///
/// 古董不走 `ancient_value`——那一项（`+0x270`）在卖页那一段只用来排序。
pub fn sell_price(price: i32) -> i32 {
    price / 2
}

/// 这一类能不能卖（`FUN_0040d010` 的六个分支 0/1/2/3/4/6）。
///
/// 治疗 1 · 投掷 2 · 仙宝 3 · 古董 4 · 矿石 5 · 尸块 12 · 装备 7–11 都能卖；
/// **剧情（6）不能**——卖页压根没建那张表。
pub fn sellable(item_type: u8) -> bool {
    matches!(item_type, 1..=5 | 7..=12)
}

/// 店里一件商品的单价就是数据库里的 `price`；这里只是把「id → 单价」摊平，
/// 方便界面一次取完。
pub fn unit_prices(goods: &[u32], price_of: impl Fn(u32) -> Option<i32>) -> HashMap<u32, i32> {
    goods
        .iter()
        .copied()
        .filter_map(|id| price_of(id).map(|p| (id, p)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
; 注释
Begin,sec1
ST $2&
DK $1&
M_ID $1411&
M_Name $阿郁&
MC $0&
Patience $10&
BeginTalk $阿郁：\\n来看看吧。&
EntryBuy $阿郁：\\n随便看看吧。&
NoEnoughMoney $阿郁：\\n您的钱不够。&

FIVE
AcptQuantum $0.1&
AcptTalk1 $接受一&
AcptTalk2 $接受二&
RfusQuantum $0.7&
RfusTalk1 $拒绝一&
RfusTalk2 $拒绝二&
HarshQuantum $0.2&
HarshTalk1 $严词一&
HarshTalk2 $严词二&
ForceAcptTalkto1 $一口价一&
ForceAcptTalkto2 $一口价二&
ForceAcptTalkback $景天：\\n那就原价吧。&
#
NINE
AcptQuantum $0.95&
RfusQuantum $0.05&
HarshQuantum $0.0&
#

Equip
Item_ID $80001& ;头巾
Item_ID $80002&
#

Prop
Item_ID $6201&
#
end
";

    #[test]
    fn the_header_comes_out_of_the_dollar_signs() {
        let shop = DealScript::parse(SAMPLE);
        assert_eq!(shop.shop_type, SHOP_GENERAL);
        assert!(shop.haggling);
        assert_eq!(shop.merchant_id, 1411);
        assert_eq!(shop.merchant_name, "阿郁");
        assert_eq!(shop.temperament, 0);
        assert_eq!(shop.patience, 10);
        assert_eq!(shop.begin_talk, "阿郁：\n来看看吧。");
        assert!(shop.entry_sale.is_empty());
    }

    #[test]
    fn the_five_tiers_and_the_two_goods_lists_are_separate() {
        let shop = DealScript::parse(SAMPLE);
        assert_eq!(shop.tiers[0].accept_chance, 0.1);
        assert_eq!(shop.tiers[0].refuse_chance, 0.7);
        assert_eq!(shop.tiers[0].accept_talk, vec!["接受一", "接受二"]);
        assert_eq!(shop.tiers[0].force_accept_talk.len(), 2);
        assert_eq!(shop.tiers[0].force_accept_reply, "景天：\n那就原价吧。");
        assert_eq!(shop.tiers[4].accept_chance, 0.95);
        assert_eq!(shop.equipment, vec![80001, 80002]);
        assert_eq!(shop.props, vec![6201]);
        assert_eq!(shop.goods(), vec![80001, 80002, 6201]);
    }

    #[test]
    fn a_discount_is_integer_division() {
        // 五折…九折。
        assert_eq!(discounted(101, 0), 50);
        assert_eq!(discounted(101, 1), 60);
        assert_eq!(discounted(101, 2), 70);
        assert_eq!(discounted(101, 3), 80);
        assert_eq!(discounted(101, 4), 90);
    }

    #[test]
    fn a_zero_chance_never_lands_and_a_full_one_always_does() {
        let mut rng = ShopRng::new(7);
        assert!(!roll(&mut rng, 0.0));
        // 0.009 × 100 截断成 0，也是不掷。
        assert!(!roll(&mut rng, 0.009));
        for _ in 0..50 {
            assert!(roll(&mut rng, 1.0), "100% 该次次都中");
        }
    }

    #[test]
    fn patience_turns_the_third_refusal_into_a_forced_sale() {
        let mut shop = DealScript::parse(SAMPLE);
        // 必拒。
        shop.tiers[0].accept_chance = 0.0;
        shop.tiers[0].refuse_chance = 1.0;
        shop.patience = 3;

        let mut rng = ShopRng::new(1);
        let mut refusals = 0;
        assert_eq!(
            haggle(&shop, 0, 100, 1000, &mut refusals, &mut rng),
            Haggle::Refused
        );
        assert_eq!(
            haggle(&shop, 0, 100, 1000, &mut refusals, &mut rng),
            Haggle::Refused
        );
        assert_eq!(
            haggle(&shop, 0, 100, 1000, &mut refusals, &mut rng),
            Haggle::ForceAccepted
        );
        assert_eq!(refusals, 3);
        assert_eq!(price_to_pay(Haggle::ForceAccepted, 100, 0), Some(100));
        assert_eq!(price_to_pay(Haggle::Accepted, 100, 0), Some(50));
    }

    #[test]
    fn neither_roll_landing_is_a_harsh_refusal() {
        let mut shop = DealScript::parse(SAMPLE);
        shop.tiers[0].accept_chance = 0.0;
        shop.tiers[0].refuse_chance = 0.0;
        let mut rng = ShopRng::new(3);
        let mut refusals = 0;
        assert_eq!(
            haggle(&shop, 0, 100, 1000, &mut refusals, &mut rng),
            Haggle::Harsh
        );
        assert_eq!(refusals, 0);
    }

    #[test]
    fn money_is_checked_against_the_discounted_price_before_any_roll() {
        let shop = DealScript::parse(SAMPLE);
        let mut rng = ShopRng::new(5);
        let mut refusals = 0;
        // 五折 50，身上 49 不够。
        assert_eq!(
            haggle(&shop, 0, 100, 49, &mut refusals, &mut rng),
            Haggle::NotEnoughMoney
        );
        // 够了就照常掷。
        assert_ne!(
            haggle(&shop, 0, 100, 50, &mut refusals, &mut rng),
            Haggle::NotEnoughMoney
        );
    }

    #[test]
    fn the_inn_config_is_its_own_little_format() {
        let inn = RestScript::parse(
            "\
Begin,sec1
; M_ID  >>客栈老板的对话脚本编号
M_ID $1802&
M_Name $客栈掌柜&
M_RestPrice $50&
M_Rest $客栈掌柜：\\n要不要住店？&
M_Enough $客栈掌柜：\\n小二！&
M_NoEnough $客栈掌柜：\\n对不住。&
M_RestEnd $客栈掌柜：\\n一路顺风。&
end
",
        );
        assert_eq!(inn.keeper_id, 1802);
        assert_eq!(inn.keeper_name, "客栈掌柜");
        assert_eq!(inn.price, 50);
        assert_eq!(inn.ask, "客栈掌柜：\n要不要住店？");
        assert_eq!(inn.rest_end, "客栈掌柜：\n一路顺风。");
    }

    #[test]
    fn selling_pays_half_the_price_rounded_down() {
        assert_eq!(sell_price(0), 0);
        assert_eq!(sell_price(1), 0);
        assert_eq!(sell_price(99), 49);
        assert_eq!(sell_price(100), 50);
    }

    #[test]
    fn everything_but_story_items_can_be_sold() {
        // 卖页的六张表：治疗 1 · 投掷 2 · 仙宝 3 · 古董 4 · 矿石 5 ·
        // 装备 7–11 · 尸块 12。剧情 6 没有表。
        for t in [1u8, 2, 3, 4, 5, 7, 8, 9, 10, 11, 12] {
            assert!(sellable(t), "类型 {t} 该能卖");
        }
        assert!(!sellable(6), "剧情物品卖不掉");
        assert!(!sellable(0));
        assert!(!sellable(13));
    }

    #[test]
    fn a_stack_tops_out_at_ninety_nine() {
        assert_eq!(buyable(0), 99);
        assert_eq!(buyable(98), 1);
        assert_eq!(buyable(99), 0);
        assert_eq!(buyable(150), 0);
    }
}
