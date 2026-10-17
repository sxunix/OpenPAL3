//! 合成术（仙术页第 8 个页签「合」）的规则。参考源码 `gamemainui/magicui.cpp`
//! （`InitComItem` / `CheckCom` / `ComboFinal`）、`gamemainui/ChoseCompose.cpp`（SCE 111 选人学会）、
//! `gamebase/playermgr.cpp`（熟练度初值），和出货 exe 对过：配方表在 `FUN_00487480`，熟练度
//! 初值在 `PlayerMgr::Init` 里写 `0xc01fb0`（8 × 5 个 float，exe `0x40931c` 起），两边逐项一样。
//!
//! 每个角色各自学会一些配方（最多 6 个），每人每方有一个熟练度（就是成功率），合一次不论成败都
//! 加 0.01，到 1 封顶。成功得一件成品；失败也扣材料。

/// 配方数（`c_NUM`）。
pub const RECIPE_COUNT: usize = 8;
/// 主角数（`MAX_ZHUJIAO`）。
pub const ROLE_COUNT: usize = 5;
/// 一个人最多学几个（`ChoseCompose::SetSpecifyCompose` 的 `maxLearn`）。
pub const MAX_LEARNT: usize = 6;

/// 一个配方：成品和最多三样材料（0 = 没有）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Recipe {
    pub product: u32,
    pub materials: [u32; 3],
}

const fn r(product: u32, a: u32, b: u32, c: u32) -> Recipe {
    Recipe {
        product,
        materials: [a, b, c],
    }
}

/// `MagicUI::InitComItem`（exe `FUN_00487480`）。
pub const RECIPES: [Recipe; RECIPE_COUNT] = [
    r(6202, 6201, 6201, 0),    // 金创药 = 止血草 ×2
    r(6204, 6203, 6203, 0),    // 紫菁玉蓉膏 = 首阳参 ×2
    r(6210, 6204, 6208, 0),    // 玄天异果 = 紫菁玉蓉膏 + 天仙玉露
    r(6206, 6205, 6205, 0),    // 还神丹 = 鼠儿果 ×2
    r(6223, 6214, 6215, 0),    // 无忧仙果 = 脉络散 + 解咒草
    r(6230, 6216, 6219, 6224), // 毒龙胆 = 石南藤 + 定神丸 + 天心海棠
    r(6224, 6217, 6218, 0),    // 天心海棠 = 百里香 + 抚心草
    r(6208, 6206, 6206, 6206), // 天仙玉露 = 还神丹 ×3
];

/// 熟练度初值，`[配方][角色]`（`PlayerMgr::Init`，exe `0xc01fb0`）。
pub const INITIAL_RATE: [[f32; ROLE_COUNT]; RECIPE_COUNT] = [
    [0.6, 0.8, 0.4, 0.4, 0.4],
    [0.4, 0.2, 0.2, 0.4, 0.4],
    [0.05, 0.2, 0.05, 0.1, 0.05],
    [0.4, 0.4, 0.8, 0.6, 0.4],
    [0.2, 0.1, 0.1, 0.4, 0.2],
    [0.9, 1.0, 0.9, 0.9, 0.9],
    [0.2, 0.1, 0.1, 0.2, 0.4],
    [0.05, 0.05, 0.2, 0.1, 0.05],
];

/// SCE 111 的参数是成品的物品号，按它找配方（`SetSpecifyCompose` 的 switch）。
pub fn recipe_for_product(product: u32) -> Option<usize> {
    RECIPES.iter().position(|r| r.product == product)
}

/// 列表上显示的百分数和掷骰用的整数：`(int)(熟练度 × 100)`，截尾。
pub fn rate_percent(rate: f32) -> i32 {
    (rate * 100.) as i32
}

/// `MagicUI::CheckCom`：材料够不够。够的话返回 `com_num`——前两样相同是 2、三样都相同是 3、
/// 否则 0，扣材料时要用。
///
/// 原版的判法照搬：只数前两样材料，**第三样不查**（毒龙胆缺天心海棠也能合）；
/// 三样都相同时要求够 3 个。
pub fn check_materials(recipe: &Recipe, count: impl Fn(u32) -> u32) -> Option<u32> {
    let [a, b, c] = recipe.materials;
    let n1 = if a > 0 { count(a) as i64 } else { 0 };
    let n2 = if b > 0 { count(b) as i64 } else { 0 };
    let mut com_num = if a == b { 2 } else { 0 };
    if a == b && a == c {
        com_num = 3;
    }
    let short = (n1 < com_num && n2 < com_num) || (n1 <= 0 && n2 <= 0) || n1 < 1 || n2 < 1;
    (!short).then_some(com_num as u32)
}

/// 熟练度加一次（`ComboFinal` 开头）：+0.01，到 1 封顶。
pub fn raise_rate(rate: f32) -> f32 {
    (rate + 0.01).min(1.)
}

/// `CUtil::Probability((int)(熟练度 × 100))`：`rand() % 1024 < f2i(10.24 × cent)`。
pub fn rolls(cent: i32, rand15: i32) -> bool {
    if cent <= 0 {
        return false;
    }
    let max = (1024. / 100. * cent as f32).round() as i32;
    rand15 % 1024 < max
}

/// `ComboFinal` 扣材料。原版边按下标走背包边删：
///
/// - 前两样相同（`com_num > 0`）：碰到第一个是材料的格子，扣 `com_num` 个就停；
/// - 三样不同：每个是材料的格子扣 1 个；扣空的格子被删掉，后面的格子前移一位，循环下标照样加一，
///   于是紧跟在后面的那一格**被跳过**——照原样。
///
/// `order` 是背包里的 (物品, 个数)，按遍历顺序。原版背包是按拿到的先后排的数组；移植端的背包按
/// 物品号排序，所以跳格会跳到哪一格和原版不一定一样 [近似]。返回要扣的 (物品, 个数)。
pub fn consumption(recipe: &Recipe, com_num: u32, order: &[(u32, u32)]) -> Vec<(u32, u32)> {
    let ids = recipe.materials;
    let is_material = |id: u32| ids.contains(&id);
    let mut items: Vec<(u32, u32)> = order.to_vec();
    let mut out = vec![];
    let mut i = 0;
    while i < items.len() {
        let (id, n) = items[i];
        if is_material(id) {
            let take = if com_num > 0 { com_num } else { 1 };
            out.push((id, take.min(n)));
            if take >= n {
                items.remove(i);
            } else {
                items[i].1 -= take;
            }
            if com_num > 0 {
                break;
            }
        }
        i += 1;
    }
    out
}

/// 「失去 X [数量N]」里的 N（`ComboFinal`）：前两样相同时是 1 + 后两样里和第一样相同的个数，
/// 三样不同时每行都是 1。
pub fn lost_count_label(recipe: &Recipe, com_num: u32) -> u32 {
    if com_num == 0 {
        return 1;
    }
    let [a, b, c] = recipe.materials;
    1 + u32::from(b == a) + u32::from(c == a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipes_are_found_by_product() {
        assert_eq!(recipe_for_product(6202), Some(0));
        assert_eq!(recipe_for_product(6208), Some(7));
        assert_eq!(recipe_for_product(6201), None);
    }

    #[test]
    fn two_of_the_same_need_two() {
        let r = &RECIPES[0];
        assert_eq!(check_materials(r, |_| 1), None);
        assert_eq!(check_materials(r, |_| 2), Some(2));
    }

    #[test]
    fn three_of_the_same_need_three() {
        let r = &RECIPES[7];
        assert_eq!(check_materials(r, |_| 2), None);
        assert_eq!(check_materials(r, |_| 3), Some(3));
    }

    #[test]
    fn the_third_material_is_never_checked() {
        // 毒龙胆：有石南藤和定神丸、没有天心海棠也算够。
        let r = &RECIPES[5];
        assert_eq!(
            check_materials(r, |id| if id == 6224 { 0 } else { 1 }),
            Some(0)
        );
        assert_eq!(
            check_materials(r, |id| if id == 6219 { 0 } else { 1 }),
            None
        );
    }

    #[test]
    fn rate_rises_by_a_hundredth_and_caps() {
        assert!((raise_rate(0.6) - 0.61).abs() < 1e-6);
        assert_eq!(raise_rate(0.995), 1.);
        assert_eq!(rate_percent(0.61), 61);
    }

    #[test]
    fn the_roll_is_out_of_1024() {
        // 60% → 614 / 1024。
        assert!(rolls(60, 613));
        assert!(!rolls(60, 614));
        assert!(!rolls(0, 0));
        assert!(rolls(100, 1023));
    }

    #[test]
    fn same_materials_come_out_of_one_stack() {
        let r = &RECIPES[0];
        let got = consumption(r, 2, &[(6100, 1), (6201, 5), (6300, 1)]);
        assert_eq!(got, vec![(6201, 2)]);
        assert_eq!(lost_count_label(r, 2), 2);
        assert_eq!(lost_count_label(&RECIPES[7], 3), 3);
    }

    #[test]
    fn a_used_up_stack_skips_the_next_one() {
        // 紫菁玉蓉膏 + 天仙玉露：前一格只剩 1 个被扣空，后一格挨着就被跳过。
        let r = &RECIPES[2];
        assert_eq!(consumption(r, 0, &[(6204, 1), (6208, 4)]), vec![(6204, 1)]);
        assert_eq!(
            consumption(r, 0, &[(6204, 2), (6208, 4)]),
            vec![(6204, 1), (6208, 1)]
        );
        assert_eq!(
            consumption(r, 0, &[(6204, 1), (6205, 3), (6208, 4)]),
            vec![(6204, 1), (6208, 1)]
        );
    }
}
