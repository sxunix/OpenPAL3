# `cbdata\flexeff.data` — 战斗特效定义表与特效组表

读者：`fileformats::pal3::flexeff`，census：`yaobow/shared/tests/pal3_flexeff_census.rs`。

## 先说那个坑

这个文件我一开始整份当表解析，得到的是满屏乱数——特效组的计数字段动辄几十
亿。原因不在布局，在**读法**：

`basedata.cpk` 里这个条目的表项 flag 是 `0x10005`，带 `IsNotCompressed`
位，`packed_size == origin_size == 9297120`。cpk 层不做任何解压，取出来就是
9297120 字节。而引擎**只读它的前 0x2c047(180295) 字节**，再自己 LZO 解压成
0x8ddce0 = 9297120 字节。后面那 9.1 MB 是写定长缓冲时没清干净的残留（开发机
上的源码路径、堆里的碎片都在里面），跟表没关系。

所以正确的读法是：取原始字节的前 `0x2c047`，`CPK::DeCompress` 一次，拿到的
9297120 字节才是两张表。

## 加载与转储

`EffMgr.cpp` 里一读一写两个函数，别搞混：

* **加载器 0x4ce100**，打开 `cbdata\flexeff.data`：

  ```text
  gbVFileSystem::OpenFile("cbdata\\flexeff.data", 0x201)
  malloc(0x2c047); malloc(0x8ddce0)
  gbVFileSystem::Read(src, 0x2c047, file)
  CPK::DeCompress(dst, src, 0x2c047)          // 返回值必须 == 0x8ddce0
  memcpy(0x174d330, dst,         0x5a820)     // 定义表
  memcpy(0xec9e70,  dst+0x5a820, 0x8834c0)    // 特效组表
  ```

  `DeCompress` 是 `gbengine.dll` 的 `?DeCompress@CPK@@QAEKPAX0K@Z`，就是 cpk
  自己那套 LZO1X。

* **转储器 0x4ce2a0**，用 `"wb"` 把同样两块内存原样 `fwrite` 到
  `basedata\cbdata\flexeff.data`（注意多一层 `basedata\`）。这是开发期的
  调试出口，游戏里没有任何地方读它——`0x5d558c` 那个串全程只有一处引用。

## 定义表：1324 × 280

条数和记录大小都能从索引算式里读出来。`FUN_004cdca0` 0x4cdcb3 起：

```text
ecx = eax*8;  ecx -= eax;      // 7a
eax = ecx + ecx*4;             // 35a
ebx = eax*8 + 0x174d330;       // 280a + 基址
```

查找是 `FUN_004ce000` 的线性扫描，两个关键偏移直接写在里面：

```text
ecx = 0x174d354                // = 表基址 + 0x24
loop:
  bl = byte [ecx + 0xb0]       // 记录 +0xd4 = 212，为 0 就跳过这条
  if (bl && dword [ecx] == key) return index;   // 记录 +0x24 = 36
  ecx += 0x118;  index++;      // 0x118 = 280
  while (ecx < 0x17a7b74)      // 0x17a7b74 - 0x174d354 = 0x5a820 → 1324 条
```

于是：

* **`+36` 是查找键**（一个 CRC），特效组里存的就是它，不是下标。这解释了
  为什么组里那串数看着像"几十亿的乱数"。
* **`+212` 是有效位**，为零的记录不参与查找。

这两条是从指令里证出来的。其余字段沿用 Pal3.Unity 的
`EffectDefinitionFile.cs`（GPL-3.0，0x7c13）：

```text
  0  ClassNameCrc u32
  4  TypeName char[32]        GBK
 36  TypeNameCrc u32          ← 查找键（已证）
 40  union char[128]          贴图 / .par / 音效前缀 / 视频前缀
168  FramesPerRow  172 FramesPerColumn  176 FrameCount  180 FramePerSecond f32
184/188/192 InitX/Y/ZPosition   196 InitYRotation|InitAngle
200  InitSize   204 InitColor   208 MaxQuadCount
212  IsValid    213 IsAdditiveBlend  214 IsBillboard  215 IsZTestIgnored   ← 212 已证
216  InitXRotation  220 InitZRotation  224 RotationAngle  228 Width
232  DivisionCount  236 TopShrink  240 BottomShrink
244  TopRadius  248 BottomRadius  252/256 Top/BottomRadiusIncreaseSpeed
260/264 Top/BottomRiseSpeed   268 TextureRepeat   272 Param[2]
```

`272 + 8 = 280`，正好收口。Unity 那边的 `NUM_OF_EFFECTS = 1324` 和这里
`0x5a820 / 280` 算出来的完全一致。（他们的 `EffectDefinitionFileReader`
在项目里没有任何调用点，是写了没接的死代码，但布局是对的。）

解出来的实况：**1263 / 1324 条 `+212` 非零**。类型名是 GBK 中文，
按出现次数排是「景天」11、「镜头」10、「雪」8、「紫」6、「龙葵」6、
「长卿」4、「时间」2，其余基本一名一条。资源名的扩展名分布：

| 扩展名 | 条数 |
|---|---|
| `.tga` | 1079 |
| `.par` | 59 |
| `.cvd` | 57 |
| `.bmp` | 7 |
| `.pol` | 2 |
| 空 | 52 |

也就是说战斗特效的主体是贴图序列，模型和粒子各占几十条。

## 特效组表：560 × 15940

步长同样来自索引算式，0x4ce3a8 起：

```text
ecx = eax << 5;  ecx -= eax;   // 31a
ebx = eax + ecx*8;             // 249a
ebx <<= 4;  ebx += eax;        // 3985a
ebx <<= 2;                     // 15940a
```

组内布局来自紧接着的遍历：

```text
eax = dword [ebx + 0xec9e70];  test eax,eax;  jle skip   // +0 计数，≤0 整组跳过
edx = ebx + 0xec9e74;          // +4    id 数组，外层 add edx,4
esi = ebx + 0xec9eb4;          // +0x44 并行数组，外层 add edx,0x3e0
  ebp = 8;
  al = byte [esi+0x58];  if (al) …;  add esi,0x7c;  dec ebp;  jnz   // 8 × 124
```

代进总长就闭合了：

```text
4 + 16*4 + 16*992 = 15940      其中 992 = 0x3e0 = 8 * 0x7c
```

没有别的数组长度能凑出 15940，所以 **`count` 的合法区间是 1..=16**。这就是
把有效组从残留垃圾里分出来的边界。

`+4` 的 id 数组和 `+0x44` 的槽位数组同步推进（外层同时 `add edi,4` 和
`add edx,0x3e0`），所以第 i 个 id 配第 i 个槽位。子块的 `+0x58` 是启用标志，
原版只对非零的子块发虚函数调用（`call [edx+0x24]` / `call [edx+0x30]`）。

解出来的实况，560 组的计数直方图：

```text
0:37  1:23  2:202  3:35  4:35  5:19  6:23  7:21  8:26
9:14  10:9  11:10  12:20  13:11  14:10  15:24  16:41
```

**一个越界的都没有**，37 个 0 对应整组全零的槽位。523 个有效组一共 3125 个
id，**全部**能在定义表里按 `+36` 查到一条 `+212` 非零的记录；计数之外的 id
槽全是零；25000 个子块的启用标志只取 0/1（其中 5647 个为 1）。census 就是钉
这几条。

## 类注册表：15 个特效类

定义记录 `+0`（Unity 叫 `ClassNameCrc`）是交给 `FUN_004cdfd0` 查的，查的是
**管理器对象内部一张 32 条的类注册表**，形状和定义表那次扫描一模一样：

```text
ecx += 0x5c                    // this + 0x5c
  bl = byte [ecx - 4]          // this+0x58 + i*0x48   启用位
  if (bl && dword [ecx] == key) return i;   // this+0x5c + i*0x48   类 CRC
  ecx += 0x48;  i++;  while (i < 0x20)
```

表是 `FUN_004cded0` 填的，一串 `push 0; push 0; push <类名>; mov ecx,esi;
call <工厂>`。**15 个类，按注册顺序**：

| # | 类名 | 工厂 | CRC | 有效定义条数 | 资源 |
|---|---|---|---|---|---|
| 1 | `particle` | 0x4ceb60 | 0xf3669c0c | 59 | `.par` |
| 2 | `quad` | 0x4cec80 | 0x71756164 | 669 | `.tga` |
| 3 | `action` | 0x4ceda0 | 0x993b8f14 | 36 | 空 |
| 4 | `camera` | 0x4ceec0 | 0x2947377a | 11 | 空 |
| 5 | `time` | 0x4cefe0 | 0x74696d65 | 3 | 空 |
| 6 | `cylinder` | 0x4cf100 | 0x725e9cda | 184 | `.tga` |
| 7 | `model` | 0x4cf220 | 0x43fb6563 | 59 | `.cvd` / `.POL` |
| 8 | `callback` | 0x4cf340 | 0x7ad15de0 | 1 | 空 |
| 9 | `grid` | 0x4cf460 | 0x67726964 | 15 | `.tga` |
| 10 | `explosive` | 0x4cf580 | 0xf8038d1f | 21 | `.tga` |
| 11 | `meteor` | 0x4cf6a0 | 0x4cdda041 | 162 | `.tga` |
| 12 | `audio` | 0x4cf7c0 | 0x6cf7f204 | 6 | 音效前缀 `wa`..`wf` |
| 13 | `shock` | 0x4cf8e0 | 0x346f181e | 35 | `.tga` |
| 14 | `bink` | 0x4cfa00 | 0x62696e6b | 1 | 视频前缀 `2DEFFECT\t` |
| 15 | `quake` | 0x4cfb20 | 0x20e32b7e | 1 | 空 |

1263 条有效定义全部落在这 15 个类里，**一条漏网的都没有**。`particle` 的 59
条正好等于资源名以 `.par` 结尾的 59 条，`model` 的 59 条对应 `.cvd`/`.POL`
——`union` 那个说法就是这么落实的。

CRC 由 `FUN_004d58d0`（全局对象 0x1895580 上的方法）算出、存进注册表项的
`+0x5c`。算法就是 cpk 给文件名算校验和的那套：头 4 个字节大端拼成初值、取
反、逐字节 `table[v>>24] ^ (b | v<<8)`、末尾再取反，表是多项式 0x04C11DB7 的
MSB-first CRC-32 表。名字不超过 4 个字符时循环体一次都不跑、两次取反抵消，
所以 `quad`/`grid`/`time`/`bink` 的 CRC 就等于名字本身的 ASCII——算个免费的
自检。

注册表项 0x48 字节，从 `this+0x1c` 起：类名串 `+0`（0x1e 字节）、第二个串
`+0x1e`、启用位 `+0x3c`、类 CRC `+0x40`。工厂还会 `operator_new(0x50)` 造一
个原型实例挂在 `+0x44`。

## 槽位子块那 124 字节

`particle` 的 vtable 在 0x56f8cc，组遍历调的 `[edx+0x30]` 是 0x4c8e90。它先
在实例数组里找空槽（每条 0x98 字节，`+0x94` 是占用位），然后：

```text
004c8f1b  mov esi, [esp+0x18]           // 传进来的子块
004c8f3f  lea edi, [ecx + ebp + 0x18]   // 实例 +0x18
004c8f43  mov ecx, 0x1f                 // 31 dword = 124 字节
004c8f48  rep movsd                     // 整块照抄（参数为 NULL 时改成清零）
```

所以**这 124 字节就是特效实例的参数块，原样拷进实例的 `+0x18`**；实例记录
`0x98 = 0x18 头 + 0x7c 参数 + 0x94 占用位 + 0x95 一个标志`，尺寸自洽。一个组
条目 = 一条特效定义 + 最多 8 个带不同参数的实例。

字段含义按类不同，还没逐个跟。5647 个启用子块的普查给出这些**已证的结构性
事实**：

* `+96..124` 那 28 字节恒为零，实际只用 96 字节。
* `+0x58`(88) 起是四个标志字节，取值只有 0/1；第 0 个就是遍历看的启用位，
  第 3 个从来没被用过（第 1、2 个分别有 55 和 13 次为 1）。
* 跨全部 15 个类恒非零的只有 **`+76`**（浮点，取值集中在 0.2/0.5/1.0/2.0，
  语义未定 `[推断]`）和 `+88`。其余字段各类用各类的：`time`/`camera`/`bink`
  只用 `+0`、`+76`、`+88`；`quake` 多用 `+4`、`+8`；`audio` 用 `+0..+12`、
  `+32`、`+40..+48`；`quad`/`cylinder` 集中在 `+0`、`+44`、`+56`、`+72`、
  `+80`、`+92`。
* `+80` 是整数不是浮点（100 / 10 / 1 / 1000 / 2 …）。

## 指令 143 / 148 怎么用这两张表

```text
148 CEft_Load(g):  FUN_004ce460(mgr = 0xbfda78, g)
143 CEft(g):       FUN_004ce340(mgr, g, x, y, z, 0)
                   x/y/z 取自 [0xd8e478] / [0xd8e47c] / [0xd8e480]，
                   也就是 142 CEft_Pos 和 144 CEft_Role 刚写的那个位置
```

`FUN_004ce340` 的遍历就是上一节那段：按组取 id、查定义、对每个启用的子块给
实例发 `vfunc+0x30`。两条指令都不阻塞脚本。

脚本实际引用的特效组（143 用 66 处、148 用 53 处，去重后二十来个组）里，各
类出现次数是 **quad 64 / cylinder 19 / audio 17 / particle 17 / callback 10 /
grid 2 / meteor 1**。也就是说要看到画面，绕不开 `quad`。

## 槽位子块：又认了三个字段

在原有的「`+0x58` 起四个标志字节、`+96..124` 恒零、`+76` 跨类恒非零」之外：

| 偏移 | 含义 |
|---|---|
| `+72` | **起始延迟（秒）**。同一组里同一个特效的多份拷贝就是靠它错开——实机某组的 8 份 `fuzi.tga` 依次是 0.0 / 0.1 / 0.2 / 0.3 / 0.4 / 0.5。 |
| `+76` | 像时长但**还不能这么用** `[推断]`。一帧的 `fuzi` 是 0.2、`dongdongbo` 是 1.0/3.0 都合理，可 32 帧 16fps（合 2.0 秒）的 `languang` 只有 0.5，对不上。 |
| `+80` | `quad` 类的 `vfunc+0x30`（0x4d2090 的 `mov eax,[edi+0x50]; mov [ecx+0x38],eax`）把它读进对象的 `+0x38`。这是这 124 字节里唯一确知去向的字段。 |

另外 `+0` 和 `+92` 在实测里恒等（−10 / −50 / 100 / 15 这样成对出现），`+56`
基本是 100 或 200，都还没定性。

## `quad` 类为什么还没做

`quad` 的原型是 `new(4272)` + `FUN_004d0ce0`，实例 vtable 在 0x56fa30，
`+0x30`（0x4d2080）建实例、`+0x08`（0x4d0ec0）每帧更新。建实例那一步除了把
子块整块拷进 `+0x18`，还通过 `FUN_004d3010` 挂了两条时间线回调
（0x4d22f0 和 0x4d2e10），子对象用的是 0x4ca400 那张 vtable。

定义记录里画一个 quad 需要的静态量都有——图集行列数与帧数、fps、初始位置/
大小/颜色（ARGB）、加法混合 / 公告板 / 忽略深度三个标志，实测取值也都合理
（`languang` 是 8×4=32 帧 @16fps，`fuzi` 是单帧，颜色分别是淡蓝和淡红）。
**缺的是「活多久、期间大小和透明度怎么变」**，那完全由上面那两条时间线决
定。静态 quad 的寿命猜错，画出来比不画更误导，所以先不画。

## 还没做的

* 子块 124 字节里各类自己那部分的字段语义——要逐个类跟它们的 update。
* 定义表里 `[推断]` 那些字段没有逐个回到指令核对，只核了取值合理性。
* `flexcam.dat` / `efflinker.dat` 两张表。
