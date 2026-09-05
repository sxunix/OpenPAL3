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

## 还没做的

* 槽位子块那 124 字节的字段布局（只认了 `+0x58` 的启用标志）。
* 定义表里 `[推断]` 那些字段没有逐个回到指令核对，只核了取值合理性。
* `FUN_004cdfd0` 的 class CRC → 特效类映射，以及各类 `Init` 分别读记录的哪
  些字段——这是 143/148 真正要接的东西。
* `flexcam.dat` / `efflinker.dat` 两张表。
