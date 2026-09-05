//! 大地图（`SwitchMode(13)`）的目的地表。
//!
//! 逆向写在 `switch/re/notes/bigmap.md`。要点：
//!
//! * 状态类是 `FUN_0044f8d0` 里模式 13 那一支造的 `new(0x5660)`，
//!   构造函数 `FUN_004460d0`；元件数组在 `esi + 0x204 + i * 0x1dc`，
//!   元件内 `+0x00`/`+0x04` 是左上角 x/y，`+0x48`/`+0x4c` 是宽高。构造函数
//!   当场把 `+0x08 = x + w`、`+0x0c = y + h` 也算好了，可以拿来校验。
//! * 选中第 `i` 个就跑 `ui\bigmap\bigmap.sce` 的 `i + 100` 号过程，再
//!   `SwitchMode(0)` 回冒险模式（`FUN_0044a4f0`，`0 <= i < 0x26`）。
//! * 哪些点亮由指令 125 决定：`LAB_00436bd0` 写 `i32 状态[38]` 和
//!   `u8 可见[38] = (option > 0)`，绘制循环只看后者。
//!
//! 坐标都在背景那张 1536×1024 的图上，背景本身是 6×4 张 256×256 的
//! `BigMap0.tga` .. `BigMap23.tga`。

/// 一个目的地。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Destination {
    /// 过程名里的地名，例如「雪岭镇」。
    pub name: &'static str,
    /// `ui\BigMap\Element\<texture>.tga`。
    pub texture: &'static str,
    /// 图标在 1536×1024 地图上的位置与大小。
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// 背景图的像素尺寸——构造函数里的 `push 0x600` / `push 0x400`。
pub const MAP_WIDTH: i32 = 1536;
pub const MAP_HEIGHT: i32 = 1024;

/// 背景分块：6 列 × 4 行的 256×256。
pub const TILE: i32 = 256;
pub const TILES_X: i32 = MAP_WIDTH / TILE;
pub const TILES_Y: i32 = MAP_HEIGHT / TILE;

/// 目的地 `i` 对应 `bigmap.sce` 的过程号——`FUN_0044a4f0` 的 `n + 0x64`。
pub const FIRST_PROC: u32 = 100;

/// 大地图自己的脚本。
pub const SCE_PATH: &str = "/basedata/basedata/ui/bigmap/bigmap.sce";

/// 背景分块所在目录。
pub const MAP_DIR: &str = "/basedata/basedata/ui/BigMap/Map";

/// 目的地图标所在目录。
pub const ELEMENT_DIR: &str = "/basedata/basedata/ui/BigMap/Element";

/// 38 个目的地，下标就是 `FUN_0044a4f0` 收到的那个号。
///
/// 有两条是照抄 exe、**不要按名字"修正"**的：下标 15（过程 115
/// 「从大地图进入黄泉路」）配的图标是 `rongyandiyu`，下标 25（过程 125
/// 「从大地图进入熔岩地狱」）配的是 `huangquanlu`——0x446a4e 和 0x446bb3
/// 处 exe 就是这么推的，两张图相对过程名是对调的。
pub const DESTINATIONS: [Destination; 38] = [
    Destination {
        name: "安宁村",
        texture: "anningchun",
        x: 122,
        y: 472,
        width: 116,
        height: 73,
    }, // 0 -> 过程 100
    Destination {
        name: "安溪",
        texture: "anxi",
        x: 1230,
        y: 566,
        width: 128,
        height: 113,
    }, // 1 -> 过程 101
    Destination {
        name: "宾化",
        texture: "binhua",
        x: 852,
        y: 569,
        width: 174,
        height: 126,
    }, // 2 -> 过程 102
    Destination {
        name: "璧山",
        texture: "bishan",
        x: 671,
        y: 419,
        width: 142,
        height: 106,
    }, // 3 -> 过程 103
    Destination {
        name: "冰风谷",
        texture: "bingfenggu",
        x: 6,
        y: 214,
        width: 112,
        height: 113,
    }, // 4 -> 过程 104
    Destination {
        name: "草海",
        texture: "chaohai",
        x: 440,
        y: 675,
        width: 110,
        height: 103,
    }, // 5 -> 过程 105
    Destination {
        name: "船长江",
        texture: "chuanchangjiang",
        x: 1113,
        y: 195,
        width: 97,
        height: 85,
    }, // 6 -> 过程 106
    Destination {
        name: "船海",
        texture: "chuanhai",
        x: 1405,
        y: 138,
        width: 113,
        height: 83,
    }, // 7 -> 过程 107
    Destination {
        name: "大渡口",
        texture: "dadukou",
        x: 923,
        y: 417,
        width: 214,
        height: 149,
    }, // 8 -> 过程 108
    Destination {
        name: "德阳",
        texture: "deyang",
        x: 204,
        y: 173,
        width: 104,
        height: 80,
    }, // 9 -> 过程 109
    Destination {
        name: "酆都",
        texture: "fengdu",
        x: 1006,
        y: 155,
        width: 139,
        height: 101,
    }, // 10 -> 过程 110
    Destination {
        name: "古城镇",
        texture: "guchengzhen",
        x: 81,
        y: 832,
        width: 134,
        height: 108,
    }, // 11 -> 过程 111
    Destination {
        name: "古藤林",
        texture: "gutenglin",
        x: 94,
        y: 590,
        width: 182,
        height: 136,
    }, // 12 -> 过程 112
    Destination {
        name: "鬼界外围",
        texture: "guijiewaiwei",
        x: 1089,
        y: 18,
        width: 112,
        height: 107,
    }, // 13 -> 过程 113
    Destination {
        name: "海底城",
        texture: "haidicheng",
        x: 1386,
        y: 625,
        width: 85,
        height: 155,
    }, // 14 -> 过程 114
    Destination {
        name: "黄泉路",
        texture: "rongyandiyu",
        x: 903,
        y: 46,
        width: 120,
        height: 110,
    }, // 15 -> 过程 115
    Destination {
        name: "剑冢",
        texture: "jianzhong",
        x: 1121,
        y: 329,
        width: 90,
        height: 140,
    }, // 16 -> 过程 116
    Destination {
        name: "九顶山",
        texture: "jiudingshan",
        x: 357,
        y: 47,
        width: 143,
        height: 117,
    }, // 17 -> 过程 117
    Destination {
        name: "九龙坡",
        texture: "jiulongpo",
        x: 728,
        y: 555,
        width: 120,
        height: 111,
    }, // 18 -> 过程 118
    Destination {
        name: "雷州",
        texture: "leizhou",
        x: 907,
        y: 784,
        width: 155,
        height: 139,
    }, // 19 -> 过程 119
    Destination {
        name: "蛮州",
        texture: "manzhou",
        x: 311,
        y: 778,
        width: 174,
        height: 132,
    }, // 20 -> 过程 120
    Destination {
        name: "神魔之井",
        texture: "mohuankongjian",
        x: 226,
        y: 880,
        width: 141,
        height: 145,
    }, // 21 -> 过程 121
    Destination {
        name: "蓬莱",
        texture: "penglai",
        x: 1336,
        y: 20,
        width: 100,
        height: 94,
    }, // 22 -> 过程 122
    Destination {
        name: "蓬莱迷宫",
        texture: "penglaimigong",
        x: 1426,
        y: 40,
        width: 65,
        height: 112,
    }, // 23 -> 过程 123
    Destination {
        name: "霹雳堂",
        texture: "pilitang",
        x: 170,
        y: 243,
        width: 75,
        height: 69,
    }, // 24 -> 过程 124
    Destination {
        name: "熔岩地狱",
        texture: "huangquanlu",
        x: 1007,
        y: 100,
        width: 209,
        height: 61,
    }, // 25 -> 过程 125
    Destination {
        name: "神界天门",
        texture: "shenjietianmen",
        x: 438,
        y: 858,
        width: 232,
        height: 166,
    }, // 26 -> 过程 126
    Destination {
        name: "神树",
        texture: "shenshu",
        x: 748,
        y: 918,
        width: 100,
        height: 96,
    }, // 27 -> 过程 127
    Destination {
        name: "蜀山",
        texture: "shushan",
        x: 406,
        y: 170,
        width: 142,
        height: 189,
    }, // 28 -> 过程 128
    Destination {
        name: "蜀山故道",
        texture: "shushangudao",
        x: 304,
        y: 273,
        width: 204,
        height: 256,
    }, // 29 -> 过程 129
    Destination {
        name: "锁妖塔",
        texture: "suoyaota",
        x: 518,
        y: 151,
        width: 135,
        height: 176,
    }, // 30 -> 过程 130
    Destination {
        name: "唐家堡",
        texture: "tangjiabao",
        x: 588,
        y: 504,
        width: 124,
        height: 96,
    }, // 31 -> 过程 131
    Destination {
        name: "灵山仙人洞",
        texture: "xianrendong",
        x: 164,
        y: 716,
        width: 161,
        height: 92,
    }, // 32 -> 过程 132
    Destination {
        name: "新仙界",
        texture: "xinxianjie",
        x: 656,
        y: 46,
        width: 141,
        height: 144,
    }, // 33 -> 过程 133
    Destination {
        name: "星森",
        texture: "xingsen",
        x: 585,
        y: 409,
        width: 41,
        height: 35,
    }, // 34 -> 过程 134
    Destination {
        name: "雪岭镇",
        texture: "xuelingzhen",
        x: 18,
        y: 125,
        width: 146,
        height: 105,
    }, // 35 -> 过程 135
    Destination {
        name: "渝州",
        texture: "yuzhou",
        x: 771,
        y: 298,
        width: 188,
        height: 120,
    }, // 36 -> 过程 136
    Destination {
        name: "镇江",
        texture: "zhenjiang",
        x: 1248,
        y: 200,
        width: 138,
        height: 113,
    }, // 37 -> 过程 137
];

/// 目的地 `index` 要跑的过程号。
pub fn proc_of(index: usize) -> Option<u32> {
    (index < DESTINATIONS.len()).then(|| FIRST_PROC + index as u32)
}

/// 第 `n` 块背景在地图上的左上角。
pub fn tile_origin(n: i32) -> (i32, i32) {
    ((n % TILES_X) * TILE, (n / TILES_X) * TILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn there_are_exactly_the_38_the_exe_bounds_at() {
        // `FUN_0044a4f0` 拒掉 >= 0x26，bigmap.sce 的过程正好是 100..=137。
        assert_eq!(DESTINATIONS.len(), 0x26);
        assert_eq!(proc_of(0), Some(100));
        assert_eq!(proc_of(37), Some(137));
        assert_eq!(proc_of(38), None);
    }

    #[test]
    fn every_icon_sits_inside_the_map() {
        // 神魔之井（下标 21）的 y + h = 1025，比图底多出一个像素。0x448870
        // 一带 exe 写的就是 x=0xe2, w=0x8d, h=0x91，不是抽错的，所以这里留
        // 一格余量而不是改数据。
        const OVERHANG: i32 = 1;
        for (i, d) in DESTINATIONS.iter().enumerate() {
            assert!(d.width > 0 && d.height > 0, "{i} {} 尺寸为零", d.name);
            assert!(
                d.x >= 0
                    && d.y >= 0
                    && d.x + d.width <= MAP_WIDTH + OVERHANG
                    && d.y + d.height <= MAP_HEIGHT + OVERHANG,
                "{i} {} 的矩形跑出图外: {:?}",
                d.name,
                (d.x, d.y, d.width, d.height)
            );
        }
        assert_eq!(
            DESTINATIONS[21].y + DESTINATIONS[21].height,
            MAP_HEIGHT + 1,
            "神魔之井那一个像素的溢出被改掉了"
        );
    }

    #[test]
    fn the_background_tiles_exactly_cover_the_map() {
        assert_eq!(TILES_X * TILES_Y, 24);
        assert_eq!(tile_origin(0), (0, 0));
        assert_eq!(tile_origin(5), (1280, 0));
        assert_eq!(tile_origin(6), (0, 256));
        assert_eq!(tile_origin(23), (1280, 768));
    }

    #[test]
    fn the_two_swapped_icons_are_kept_as_the_exe_has_them() {
        assert_eq!(DESTINATIONS[15].name, "黄泉路");
        assert_eq!(DESTINATIONS[15].texture, "rongyandiyu");
        assert_eq!(DESTINATIONS[25].name, "熔岩地狱");
        assert_eq!(DESTINATIONS[25].texture, "huangquanlu");
    }
}
