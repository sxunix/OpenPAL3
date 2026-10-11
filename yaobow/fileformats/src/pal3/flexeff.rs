//! `cbdata\flexeff.data` — 战斗特效的定义表和特效组表。
//!
//! # 文件是怎么装的
//!
//! `PAL3.exe` 的 `EffMgr.cpp` 里有一读一写两个函数，写的那个（0x4ce2a0）
//! 把内存里两张表原样 `fwrite` 到 `basedata\cbdata\flexeff.data`，是开发期
//! 的转储；真正的加载器在 0x4ce100：
//!
//! ```text
//! gbVFileSystem::OpenFile("cbdata\\flexeff.data", 0x201)
//! malloc(0x2c047); malloc(0x8ddce0)
//! gbVFileSystem::Read(src, 0x2c047, file)
//! CPK::DeCompress(dst, src, 0x2c047)   // 返回值必须等于 0x8ddce0
//! memcpy(0x174d330, dst,          0x5a820)    // 定义表
//! memcpy(0xec9e70,  dst+0x5a820,  0x8834c0)   // 特效组表
//! ```
//!
//! 这里有个容易踩的坑：这个条目在 `basedata.cpk` 里的表项 flag 是
//! `0x10005`，带 `IsNotCompressed` 位，`packed_size == origin_size ==
//! 9297120`。也就是说 **cpk 层不解压，取出来就是 9297120 字节**，而引擎
//! 只读它的**前 0x2c047 字节**，再用 `CPK::DeCompress`（LZO1X）自己解成
//! 0x8ddce0 = 9297120 字节。后面那 9.1 MB 是没清干净的缓冲区残留，全是高熵
//! 垃圾。直接把整个文件当表解析会得到满屏乱数——计数字段动辄几十亿就是这么
//! 来的。[`decode`] 负责把这一步做对。
//!
//! # 定义表
//!
//! 0x5a820 / 280 = 1324 条，每条 280 字节。记录大小和条数都能从 exe 的索引
//! 算式里读出来，`FUN_004cdca0` 0x4cdcb3 起：
//!
//! ```text
//! ecx = eax*8;  ecx -= eax;      // 7a
//! eax = ecx + ecx*4;             // 35a
//! ebx = eax*8 + 0x174d330;       // 280a + 基址
//! ```
//!
//! 查找是 `FUN_004ce000` 的线性扫描，两个偏移直接写在里面：
//!
//! ```text
//! ecx = 0x174d354                // = 基址 + 0x24
//! loop:
//!   bl = byte [ecx + 0xb0]       // 记录 +0xd4 = 212，为 0 就跳过这条
//!   if (bl && dword [ecx] == key) return index;   // 记录 +0x24 = 36
//!   ecx += 0x118;  index++;      // 0x118 = 280
//!   while (ecx < 0x17a7b74)      // 0x17a7b74 - 0x174d354 = 0x5a820
//! ```
//!
//! 所以 **+36 是查找键（CRC）**、**+212 是有效位**。
//!
//! 布局最初沿用 Pal3.Unity 的 `EffectDefinitionFile.cs`（GPL-3.0，0x7c13），
//! 后来按「哪个特效类读哪个偏移」逐条回到了指令：取 15 个类的 vtable，读
//! `+4`（Create）和 `+8`（Start）两个槽，看它们从记录里取什么。结论是
//! **168..268 这一段每个字段都有确切的消费点**，名字和 Unity 给的一致；
//! 逐字段的出处见 `switch/re/notes/flexeff_format.md`，要点：
//!
//! * `quad` 的 `FUN_004d1090` / `FUN_004d0ec0` 覆盖 168/172/176/180/184/188/
//!   192/196/200/204/208/213/214/215/216/220；
//! * `cylinder` 的 `FUN_004c4060` 覆盖 224..268（`grid`、`meteor` 同读）；
//! * `particle` / `model` / `audio` / `bink` 只取 40 那段资源名。
//!
//! 收尾的 **272..280（Unity 叫 `Param[2]`）是死字节**：15 个类的 Create /
//! Start 都不读它，而且全表只有两种取值——1216 条是
//! `ca 04 00 00 2d d5 df 77`、47 条是 `cc` × 8（MSVC 没初始化的栈填充）。
//! 一条随记录变的都没有，所以它不是数据，是转储时带出来的残留。
//!
//! # 特效组表
//!
//! 0x8834c0 / 15940 = 560 组。步长同样来自索引算式（0x4ce3a8 起
//! `<<5`、`-a`、`+a*8`、`<<4`、`+a`、`<<2` 合起来是 15940a），组内布局来自
//! 紧接着的遍历：
//!
//! ```text
//! eax = dword [ebx + 0xec9e70];  test eax,eax;  jle skip   // +0 是计数
//! edx = ebx + 0xec9e74;          // +4   id 数组，循环里 add edx,4
//! esi = ebx + 0xec9eb4;          // +0x44 并行数组，循环里 add edx,0x3e0
//!   ebp = 8;  al = byte [esi+0x58];  …;  add esi,0x7c;  dec ebp   // 8 × 124
//! ```
//!
//! 代进总长就闭合了：`4 + 16*4 + 16*992 = 15940`，其中 `992 = 8 * 124`。
//! 数组长 16 这件事没有别的写法能凑出 15940，所以**计数的合法区间就是
//! 1..=16** ——这正是把有效组从残留垃圾里分出来的边界。实测 560 组的计数
//! 直方图落在 0..=16 之内，一个越界的都没有；37 组计数为 0（整组全零），
//! 其余 523 组的 3125 个 id 全部能在定义表里查到。
//!
//! id 数组和 `+0x44` 的并行数组是同步推进的（外层同时 `add edi,4` 和
//! `add edx,0x3e0`），所以第 i 个 id 配第 i 个槽位。

use std::io::{Error, ErrorKind, Result};

fn bad(msg: String) -> Error {
    Error::new(ErrorKind::InvalidData, msg)
}

/// 定义表一条记录的字节数——`FUN_004cdca0` 的 `35a*8`。
pub const DEF_SIZE: usize = 280;

/// 定义表条数——`FUN_004ce000` 的扫描区间 0x5a820 除以 280。
pub const DEF_COUNT: usize = 1324;

/// 定义表总长，也是 memcpy 的第一个长度。
pub const DEF_BLOCK: usize = DEF_SIZE * DEF_COUNT;

/// 一组的字节数——0x4ce3a8 的 `15940a`。
pub const GROUP_SIZE: usize = 15940;

/// 组数。
pub const GROUP_COUNT: usize = 560;

/// 组表总长，memcpy 的第二个长度。
pub const GROUP_BLOCK: usize = GROUP_SIZE * GROUP_COUNT;

/// 解压后的总长，`CPK::DeCompress` 的返回值必须等于它。
pub const PAYLOAD_SIZE: usize = DEF_BLOCK + GROUP_BLOCK;

/// 加载器从文件头读走的字节数，也是喂给 LZO 的长度。
pub const PACKED_SIZE: usize = 0x2C047;

/// 每组的 id 数组长度。`4 + 16*4 + 16*GROUP_SLOT_SIZE == GROUP_SIZE`。
pub const GROUP_MAX_ENTRIES: usize = 16;

/// 并行数组每个槽位的字节数，外层循环的 `add edx, 0x3e0`。
pub const GROUP_SLOT_SIZE: usize = 0x3e0;

/// 一个槽位里的子块数，内层的 `mov ebp, 8`。
pub const GROUP_SUBS: usize = 8;

/// 子块字节数，内层的 `add esi, 0x7c`。
pub const GROUP_SUB_SIZE: usize = 0x7C;

/// 子块里的启用标志，`mov al, byte ptr [esi + 0x58]`。它同时也是 `+0x58`
/// 起那四个标志字节的头一个。
pub const GROUP_SUB_ENABLED: usize = 0x58;

/// 子块里实际用到的字节数。`+96` 往后那 28 字节在全部 5647 个启用子块里恒
/// 为零。
pub const GROUP_SUB_USED: usize = 96;

/// 这一段动作的**时长（秒）**。`FUN_004d2080` 0x4d229d 把它当第 6 个参数
/// 交给 `FUN_004d3010`，落到时间线记录的 `+0x18`；`FUN_004d22f0` 0x4d2904
/// 起拿它裁本帧的 dt：`if (now − 起始 > 时长) dt -= (now − 起始 − 时长)`，
/// 让补间正好停在时长上。每段动作都得有时长，所以 15 个类里它都非零；
/// 取值集中在 0.2 / 0.5 / 1.0 / 2.0。`+72` 是配套的延迟。
pub const GROUP_SUB_DURATION: usize = 76;

/// `+72`：这一段相对**上一段开始**的延迟（秒）。`FUN_004d3010` 的第 5 个
/// 参数，落到时间线记录的 `+0x14`。负数表示「等上一段跑完」。
pub const GROUP_SUB_DELAY: usize = 72;

/// 特效实例记录的字节数——`vfunc+0x30` 里搜空槽的步长。
pub const EFFECT_INSTANCE_SIZE: usize = 0x98;

/// 子块被整块拷进实例的哪个偏移：`lea edi,[ecx+ebp+0x18]; mov ecx,0x1f;
/// rep movsd`（0x1f 个 dword 正好 124 字节）。
pub const EFFECT_INSTANCE_PARAMS: usize = 0x18;

/// 并行数组相对组头的偏移，`lea esi, [ebx + 0xec9eb4]`。
pub const GROUP_SLOTS: usize = 0x44;

// 记录内的字段偏移。36 和 212 出自 `FUN_004ce000`，168..268 出自各特效类
// 的 Create / Start（见模块文档），272 还没有消费点。
pub const DEF_CLASS_CRC: usize = 0;
pub const DEF_TYPE_NAME: (usize, usize) = (4, 32);
pub const DEF_KEY: usize = 36;
pub const DEF_ASSET: (usize, usize) = (40, 128);
pub const DEF_FRAMES_PER_ROW: usize = 168;
pub const DEF_FRAMES_PER_COLUMN: usize = 172;
pub const DEF_FRAME_COUNT: usize = 176;
pub const DEF_FPS: usize = 180;
pub const DEF_INIT_POSITION: usize = 184;
pub const DEF_INIT_Y_ROTATION: usize = 196;
pub const DEF_INIT_SIZE: usize = 200;
pub const DEF_INIT_COLOR: usize = 204;
pub const DEF_MAX_QUADS: usize = 208;
pub const DEF_VALID: usize = 212;
pub const DEF_ADDITIVE: usize = 213;
pub const DEF_BILLBOARD: usize = 214;
pub const DEF_NO_ZTEST: usize = 215;
pub const DEF_INIT_X_ROTATION: usize = 216;
pub const DEF_INIT_Z_ROTATION: usize = 220;
pub const DEF_ROTATION_ANGLE: usize = 224;
pub const DEF_WIDTH: usize = 228;
pub const DEF_DIVISION_COUNT: usize = 232;
pub const DEF_TOP_SHRINK: usize = 236;
pub const DEF_BOTTOM_SHRINK: usize = 240;
pub const DEF_TOP_RADIUS: usize = 244;
pub const DEF_BOTTOM_RADIUS: usize = 248;
pub const DEF_TOP_RADIUS_SPEED: usize = 252;
pub const DEF_BOTTOM_RADIUS_SPEED: usize = 256;
pub const DEF_TOP_RISE_SPEED: usize = 260;
pub const DEF_BOTTOM_RISE_SPEED: usize = 264;
pub const DEF_TEXTURE_REPEAT: usize = 268;
pub const DEF_PARAM: usize = 272;

/// `FUN_004cded0` 注册的 15 个特效类，按注册顺序。定义记录 `+0` 存的是类名
/// 的 CRC，[`class_of`] 拿它换回名字。
pub const EFFECT_CLASSES: [&str; 15] = [
    "particle",
    "quad",
    "action",
    "camera",
    "time",
    "cylinder",
    "model",
    "callback",
    "grid",
    "explosive",
    "meteor",
    "audio",
    "shock",
    "bink",
    "quake",
];

/// `FUN_004d58d0` 用的表，就是多项式 0x04C11DB7 的 MSB-first CRC-32 表。
const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u32) << 24;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000_0000 != 0 {
                (c << 1) ^ 0x04C1_1DB7
            } else {
                c << 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

/// `FUN_004d58d0`：头 4 个字节（不足补零）大端拼成初值，取反，然后逐字节
/// `table[v >> 24] ^ (b | v << 8)`，最后再取反。空串返回 0。cpk 给文件名算
/// 校验和用的是同一套。
pub fn name_crc(name: &[u8]) -> u32 {
    if name.is_empty() {
        return 0;
    }
    let mut init = 0u32;
    for i in 0..4 {
        init = (init << 8) | *name.get(i).unwrap_or(&0) as u32;
    }
    let mut v = !init;
    for b in name.iter().skip(4) {
        v = CRC_TABLE[(v >> 24) as usize] ^ (*b as u32 | (v << 8));
    }
    !v
}

/// 类 CRC 换回 [`EFFECT_CLASSES`] 里的名字。表里 1263 条有效定义全部命中。
pub fn class_of(crc: u32) -> Option<&'static str> {
    EFFECT_CLASSES
        .iter()
        .copied()
        .find(|n| name_crc(n.as_bytes()) == crc)
}

/// 解压后的两张表。切片按 exe 的两次 memcpy 划分。
pub struct FlexEff {
    payload: Vec<u8>,
}

/// 一条特效定义。字段按 [`DEF_KEY`] 一族的偏移取。
#[derive(Debug, Clone)]
pub struct EffectDef<'a> {
    pub index: usize,
    bytes: &'a [u8],
}

/// 一个特效组：一串特效 id 加上等长的槽位数组。
#[derive(Debug, Clone)]
pub struct EffectGroup<'a> {
    pub index: usize,
    bytes: &'a [u8],
}

fn u32_at(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

/// GBK 解码到第一个 NUL 为止——类型名是「景天」「龙葵」这样的中文。
fn gbk(b: &[u8], offset: usize, len: usize) -> String {
    use encoding::Encoding;
    let s = &b[offset..offset + len];
    let s = &s[..s.iter().position(|c| *c == 0).unwrap_or(len)];
    encoding::all::GBK
        .decode(s, encoding::DecoderTrap::Replace)
        .unwrap_or_default()
}

impl FlexEff {
    /// 走一遍加载器：取前 [`PACKED_SIZE`] 字节 LZO 解压成 [`PAYLOAD_SIZE`]。
    /// `raw` 是 `cbdata\flexeff.data` 从 cpk 里取出来的原样字节。
    pub fn decode(raw: &[u8]) -> Result<Self> {
        if raw.len() < PACKED_SIZE {
            return Err(bad(format!(
                "flexeff.data 只有 {} 字节，加载器要读 {PACKED_SIZE}",
                raw.len()
            )));
        }
        let lzo = minilzo_rs::LZO::init().map_err(|e| bad(format!("LZO 初始化失败: {e:?}")))?;
        let payload = lzo
            .decompress(&raw[..PACKED_SIZE], PAYLOAD_SIZE)
            .map_err(|e| bad(format!("flexeff.data 解压失败: {e:?}")))?;
        if payload.len() != PAYLOAD_SIZE {
            return Err(bad(format!(
                "解出 {} 字节，CPK::DeCompress 的返回值应当是 {PAYLOAD_SIZE}",
                payload.len()
            )));
        }
        Ok(Self { payload })
    }

    /// 第一次 memcpy 的那一块。
    pub fn definitions_block(&self) -> &[u8] {
        &self.payload[..DEF_BLOCK]
    }

    /// 第二次 memcpy 的那一块。
    pub fn groups_block(&self) -> &[u8] {
        &self.payload[DEF_BLOCK..]
    }

    /// 第 `index` 条定义，越界返回 `None`。
    pub fn definition(&self, index: usize) -> Option<EffectDef<'_>> {
        (index < DEF_COUNT).then(|| EffectDef {
            index,
            bytes: &self.payload[index * DEF_SIZE..(index + 1) * DEF_SIZE],
        })
    }

    /// 有效的定义（`+212` 非零），按表内顺序。
    pub fn definitions(&self) -> impl Iterator<Item = EffectDef<'_>> {
        (0..DEF_COUNT)
            .filter_map(|i| self.definition(i))
            .filter(|d| d.is_valid())
    }

    /// `FUN_004ce000`：跳过 `+212` 为零的记录，按 `+36` 比对，返回下标。
    pub fn find(&self, key: u32) -> Option<usize> {
        self.definitions().find(|d| d.key() == key).map(|d| d.index)
    }

    /// 第 `index` 组。
    pub fn group(&self, index: usize) -> Option<EffectGroup<'_>> {
        (index < GROUP_COUNT).then(|| {
            let base = DEF_BLOCK + index * GROUP_SIZE;
            EffectGroup {
                index,
                bytes: &self.payload[base..base + GROUP_SIZE],
            }
        })
    }

    /// 计数落在 1..=[`GROUP_MAX_ENTRIES`] 的那些组——其余是从未填过的槽位。
    pub fn groups(&self) -> impl Iterator<Item = EffectGroup<'_>> {
        (0..GROUP_COUNT)
            .filter_map(|i| self.group(i))
            .filter(|g| g.is_valid())
    }
}

impl<'a> EffectDef<'a> {
    pub fn raw(&self) -> &'a [u8] {
        self.bytes
    }

    /// `FUN_004ce000` 的 `byte [ecx + 0xb0]`：为零的记录不参与查找。
    pub fn is_valid(&self) -> bool {
        self.bytes[DEF_VALID] != 0
    }

    /// `FUN_004ce000` 比对的那个 dword——特效组里存的就是它。
    pub fn key(&self) -> u32 {
        u32_at(self.bytes, DEF_KEY)
    }

    /// 类名的 CRC——`FUN_004cdca0` 把它交给 `FUN_004cdfd0` 去查那张 32 条的
    /// 类注册表。
    pub fn class_crc(&self) -> u32 {
        u32_at(self.bytes, DEF_CLASS_CRC)
    }

    /// 这条定义属于哪个特效类。
    pub fn class(&self) -> Option<&'static str> {
        class_of(self.class_crc())
    }

    /// 类型名，GBK。表里出现的是「景天」「镜头」「雪」这一类。
    pub fn type_name(&self) -> String {
        gbk(self.bytes, DEF_TYPE_NAME.0, DEF_TYPE_NAME.1)
    }

    /// 资源名。按类型不同分别是贴图、`.par`、音效前缀或视频前缀，所以
    /// 原版是个 union；这里只按字符串取。
    pub fn asset(&self) -> String {
        gbk(self.bytes, DEF_ASSET.0, DEF_ASSET.1)
    }

    /// 图集的列数、行数、总帧数。
    ///
    /// `FUN_004d1090` 把前两个交给 `FUN_004d1fd0(列, 行)`，那里按
    /// 「外层走行、内层走列」建表，每格 16 字节写
    /// `(u0, v0, u1, v1) = (列/列数, 1−(行+1)/行数, u0+1/列数, 1−行/行数)`
    /// ——**行优先、v 翻转**。第三个（`+176`）原版是 `mov dl, byte [edi+0xb0]`
    /// 只取一个字节，拿来给帧号取模；实机 1263 条里没有一条超过 255。
    pub fn frames(&self) -> (u32, u32, u32) {
        (
            u32_at(self.bytes, DEF_FRAMES_PER_ROW),
            u32_at(self.bytes, DEF_FRAMES_PER_COLUMN),
            u32_at(self.bytes, DEF_FRAME_COUNT),
        )
    }

    /// 播放帧率。`FUN_004d0ec0` 搬进 `quad` 对象的 `+0x54`，帧号按
    /// `帧 += fps × dt` 走（`FUN_004d17d0`）。
    pub fn fps(&self) -> f32 {
        f32_at(self.bytes, DEF_FPS)
    }

    /// 初始位置——是**加在调用方给的发射点上的偏移**，不是绝对坐标。
    /// `FUN_004d0ec0` 0x4d0e8c 起：`对象+0x5c = 参数.x + 记录+184`，
    /// y / z 同理；`particle` 的 `FUN_004c8b80` 也是这三个。
    pub fn init_position(&self) -> (f32, f32, f32) {
        (
            f32_at(self.bytes, DEF_INIT_POSITION),
            f32_at(self.bytes, DEF_INIT_POSITION + 4),
            f32_at(self.bytes, DEF_INIT_POSITION + 8),
        )
    }

    /// 初始大小。`FUN_004d0ec0` 把这**一个**值同时写进 `quad` 的宽
    /// （`+0x68`）和高（`+0x6c`），所以四边形默认是正方的。
    pub fn init_size(&self) -> f32 {
        f32_at(self.bytes, DEF_INIT_SIZE)
    }

    /// 初始颜色，ARGB。`FUN_004d0ec0` 依次取 `>>24`、`>>16 & 0xff`、
    /// `>>8 & 0xff`、`& 0xff` 写进四个浮点；`cylinder` 的 `FUN_004c5350`
    /// 则把原值直接写进顶点的 diffuse（0x44 字节顶点的 `+0x40`），
    /// 那个位置就是 D3DCOLOR 的 `0xAARRGGBB`。
    pub fn init_color(&self) -> u32 {
        u32_at(self.bytes, DEF_INIT_COLOR)
    }

    /// 这条特效最多用掉的四边形数。`FUN_004d1090` 拿它开三个数组
    /// （`×0x50`、`×0x44`、`×4`），并写进对象的 `+0x44` 当上限。
    pub fn max_quads(&self) -> u32 {
        u32_at(self.bytes, DEF_MAX_QUADS)
    }

    /// 加法混合（+213）/ 公告板（+214）/ 关掉深度测试（+215）。
    ///
    /// 前后两个是从 `.gbf` 的名字证出来的：`FUN_004cda10` 在特效管理器上
    /// 建了四个渲染效果——`+8` `gfxscript/AdditiveBlendNoZ.gbf`、`+0xc`
    /// `AlphaBlendNoZ.gbf`、`+0x10` `AdditiveBlend.gbf`、`+0x14`
    /// `AlphaBlend.gbf`——而 `FUN_004d1300` 0x4d1320 起正是按
    /// 「`+215` 选 NoZ 那一半、`+213` 选 Additive 那一半」四选一。
    /// 中间那个在 `FUN_004d1ca0` 分叉：置位时四角直接按四边形自己的世界
    /// 位置摊开、绘制走 `0x4d136c` 那支去取相机矩阵，清零时四角摆在局部
    /// 原点、走普通变换。
    pub fn flags(&self) -> (bool, bool, bool) {
        (
            self.bytes[DEF_ADDITIVE] != 0,
            self.bytes[DEF_BILLBOARD] != 0,
            self.bytes[DEF_NO_ZTEST] != 0,
        )
    }
}

impl<'a> EffectGroup<'a> {
    pub fn raw(&self) -> &'a [u8] {
        self.bytes
    }

    /// 组头的那个有符号计数。
    pub fn count(&self) -> i32 {
        i32::from_le_bytes(self.bytes[0..4].try_into().unwrap())
    }

    /// `jle` 拒掉 0 和负数，数组长 16 给出上界。
    pub fn is_valid(&self) -> bool {
        let c = self.count();
        c > 0 && c as usize <= GROUP_MAX_ENTRIES
    }

    /// 组里的特效 id，喂给 [`FlexEff::find`]。无效组返回空。
    pub fn ids(&self) -> Vec<u32> {
        if !self.is_valid() {
            return vec![];
        }
        (0..self.count() as usize)
            .map(|k| u32_at(self.bytes, 4 + k * 4))
            .collect()
    }

    /// 第 `entry` 个 id 配的槽位里，第 `sub` 个子块。
    pub fn sub(&self, entry: usize, sub: usize) -> Option<&'a [u8]> {
        if entry >= GROUP_MAX_ENTRIES || sub >= GROUP_SUBS {
            return None;
        }
        let o = GROUP_SLOTS + entry * GROUP_SLOT_SIZE + sub * GROUP_SUB_SIZE;
        Some(&self.bytes[o..o + GROUP_SUB_SIZE])
    }

    /// 子块的 `+0x58` 标志——原版只对非零的子块发虚函数调用。
    pub fn sub_enabled(&self, entry: usize, sub: usize) -> bool {
        self.sub(entry, sub)
            .is_some_and(|b| b[GROUP_SUB_ENABLED] != 0)
    }

    /// `+0x58` 起那四个标志字节。第 0 个是启用位，第 3 个从来没被用过。
    pub fn sub_flags(&self, entry: usize, sub: usize) -> Option<[u8; 4]> {
        self.sub(entry, sub).map(|b| {
            b[GROUP_SUB_ENABLED..GROUP_SUB_ENABLED + 4]
                .try_into()
                .unwrap()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_group_layout_closes_on_15940() {
        // 数组长 16 不是猜的：只有它能把组内布局凑成 exe 的步长。
        assert_eq!(GROUP_SLOT_SIZE, GROUP_SUBS * GROUP_SUB_SIZE);
        assert_eq!(
            4 + GROUP_MAX_ENTRIES * 4 + GROUP_MAX_ENTRIES * GROUP_SLOT_SIZE,
            GROUP_SIZE
        );
        assert_eq!(GROUP_SLOTS, 4 + GROUP_MAX_ENTRIES * 4);
    }

    #[test]
    fn the_block_sizes_are_the_two_memcpy_lengths() {
        assert_eq!(DEF_BLOCK, 0x5A820);
        assert_eq!(GROUP_BLOCK, 0x8834C0);
        assert_eq!(PAYLOAD_SIZE, 0x8DDCE0);
    }

    #[test]
    fn four_letter_class_names_hash_to_their_own_bytes() {
        // 名字不超过 4 个字符时循环体一次都不跑，两次取反抵消，CRC 就等于
        // 名字本身的大端字节。表里这四个类正是这样，算个免费的自检。
        assert_eq!(name_crc(b"quad"), 0x7175_6164);
        assert_eq!(name_crc(b"grid"), 0x6772_6964);
        assert_eq!(name_crc(b"time"), 0x7469_6d65);
        assert_eq!(name_crc(b"bink"), 0x6269_6e6b);
        assert_eq!(name_crc(b""), 0);
    }

    #[test]
    fn the_crc_table_is_the_standard_one() {
        assert_eq!(CRC_TABLE[0], 0);
        assert_eq!(CRC_TABLE[1], 0x04C1_1DB7);
        assert_eq!(CRC_TABLE[255], 2985771188);
    }

    #[test]
    fn every_class_name_maps_back() {
        for n in EFFECT_CLASSES {
            assert_eq!(class_of(name_crc(n.as_bytes())), Some(n));
        }
        assert_eq!(class_of(0), None);
    }

    #[test]
    fn the_record_ends_right_after_the_last_field() {
        // Param[2] 收尾，正好填满 280。
        assert_eq!(DEF_PARAM + 8, DEF_SIZE);
    }
}
