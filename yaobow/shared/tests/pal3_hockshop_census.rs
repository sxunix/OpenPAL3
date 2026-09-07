//! 拿实机的 `SubGame/HockShop/Data/HockShopLevel.hsl` 跑一遍等级表解析器。
//!
//! 文件里带着完整注释（见 `switch/re/notes/hockshop.md`），所以这里查的是
//! 「五级都解出来了，而且和文件里写的数对得上」：功能开关随等级放开、
//! 打折随等级变低、四种经营方针的比例加起来接近 1、红利加权随等级放大
//! 到 ±0.25，乘进去之后红利率始终为正。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_hockshop_census
//! ```

use std::path::PathBuf;

use common::store_ext::StoreExt2;
use encoding::Encoding;
use mini_fs::MiniFs;
use shared::openpal3::states::hockshop::{HockShopLevels, LEVELS, MODES, TIERS};

const HSL_PATH: &str = "/basedata/basedata/SubGame/HockShop/Data/HockShopLevel.hsl";

fn read_gbk(vfs: &MiniFs, path: &str) -> Option<String> {
    let bytes = vfs.read_to_end(PathBuf::from(path)).ok()?;
    encoding::all::GBK
        .decode(&bytes, encoding::DecoderTrap::Ignore)
        .ok()
}

fn table() -> Option<HockShopLevels> {
    let assets = std::env::var("YAOBOW_PAL3_ASSETS").ok()?;
    let vfs = packfs::init_virtual_fs(&assets, None);
    let text = read_gbk(&vfs, HSL_PATH)?;
    Some(HockShopLevels::parse(&text))
}

#[test]
fn the_five_levels_parse() {
    let Some(t) = table() else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    // E 级只能卖和增资；A 级什么都能做，只是不能再增资。
    let e = t.level(0);
    assert!(e.sale && e.invest, "E 级该能卖能增资");
    assert!(
        !e.buy && !e.melon_cutting && !e.deal_in && !e.exhibit,
        "E 级功能没开"
    );
    let a = t.level(4);
    assert!(a.sale && a.buy && a.melon_cutting && a.deal_in && a.exhibit && a.rest);
    assert!(!a.invest, "A 级是顶格，不能再增资");

    // 死当物越高级越多，打折越狠。
    let props: Vec<i32> = (0..LEVELS as i32).map(|i| t.level(i).prop_num).collect();
    assert_eq!(props, vec![0, 10, 20, 30, 40], "剧情段死当物总数");
    let discounts: Vec<f32> = (0..LEVELS as i32)
        .map(|i| t.level(i).buy_discount)
        .collect();
    for w in discounts.windows(2) {
        assert!(w[1] <= w[0], "折扣该越来越低：{discounts:?}");
    }
    assert_eq!(discounts[4], 0.7);

    // 显示件数：下限不超过上限，且随等级涨。
    for i in 0..LEVELS as i32 {
        let lv = t.level(i);
        assert!(
            lv.display_min <= lv.display_max,
            "{i} 级的显示件数上下限反了"
        );
    }
    assert!(t.level(4).display_max > t.level(1).display_max);
}

#[test]
fn every_mode_has_proportions_that_add_up() {
    let Some(t) = table() else { return };
    // E 级全是 0（还没开经营），D 级往上四种方针的比例都该接近 1。
    for level in 1..LEVELS as i32 {
        for mode in 0..MODES {
            let p = t.level(level).proportions[mode];
            let sum = p.resume + p.attack + p.ancient;
            assert!(
                (sum - 1.0).abs() < 0.01,
                "{level} 级方针 {mode} 的比例加起来是 {sum}"
            );
        }
    }
}

#[test]
fn the_dividend_weights_stay_small() {
    let Some(t) = table() else { return };
    for level in 0..LEVELS as i32 {
        let lv = t.level(level);
        assert!(
            (0. ..=1.).contains(&lv.base_dividend),
            "{level} 级的基础红利 {}",
            lv.base_dividend
        );
        for mode in 0..MODES {
            let w = lv.dividends[mode];
            // 加权值随等级变大：E 全 0、D 到 ±0.1、A 到 ±0.25。
            for arr in [w.period, w.accrual, w.unhock] {
                for tier in 0..TIERS {
                    assert!(
                        arr[tier].abs() <= 0.25,
                        "{level} 级方针 {mode} 的加权值 {} 太大",
                        arr[tier]
                    );
                }
            }
            // 加权是**乘**上去的（`基础 × (1 + 加权和)`，见 FUN_0051f130），
            // 所以再差也不会变成负的：最差是 A 级奇珍异宝挑「短当期 + 高利率
            // + 高拆当率」，加权和 −0.75 → 0.25 × 0.25 = 0.0625。
            for p in 0..TIERS {
                for a in 0..TIERS {
                    for u in 0..TIERS {
                        let r = lv.dividend_rate(mode, p, a, u);
                        assert!(
                            (0. ..=1.0).contains(&r),
                            "{level}/{mode}/{p}{a}{u} 的红利率 {r} 离谱"
                        );
                        assert!(
                            lv.weight_sum(mode, p, a, u) > -1.0,
                            "{level}/{mode}/{p}{a}{u} 的加权和到了 −1，红利会归零"
                        );
                    }
                }
            }
        }
    }
}
