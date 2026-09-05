# `.par` 粒子特效格式

`effect\gbParticle\*.par`，gb 引擎的粒子特效。SCE 指令 143（播放特效）、
148（预载特效组）、152（水特效）都要它。

来源不是 `PAL3.exe`，而是随游戏一起装的 **`gbengine.dll`**（1.2 MB，
游戏根目录）。这个 dll 的导出表带完整的 C++ 修饰名，1019 个具名导出，
粒子相关的整套都在里面，不需要靠符号推测：

| 地址 | 导出名 |
|---|---|
| `0x1002bca0` | `gbParticleEffect::LoadFromFile(char*)` |
| `0x1002a7f0` | `gbPartGroupParam::LoadFromFile(gbVFile*)` |
| `0x10029cf0` | `gbPartEmitter::LoadFromFile(gbVFile*)` |
| `0x1002b0b0` | `gbParticleGroup::LoadFromFile(gbVFile*, gbParticleEffect*)` |
| `0x1002afc0` | `gbPartGroupParam::InitParticleParam(gbParticleUnit*)` |
| `0x1002aed0` | `gbPartGroupParam::ComputeParticleParam(gbParticleUnit*, float, float)` |
| `0x1002ad80` | `gbPartGroupParam::ComputeAirResist(gbParticleUnit*, float)` |
| `0x1002a2c0` | `gbPartEmitter::ComputeEmitterData(gbEmitterData*, double, double)` |
| `0x1002a400` | `gbPartEmitter::ComputeParticleInitPos(gbParticleUnit*)` |
| `0x1002a590` | `gbPartEmitter::ComputeParticleInitVel(gbEmitterData*, gbParticleUnit*)` |
| `0x1002b2a0` | `gbParticleGroup::CreateOneParticle(gbEmitterData*)` |
| `0x1002b590` | `gbParticleGroup::UpdateOneParticle(gbParticleUnit*, double)` |
| `0x1002b6a0` | `gbParticleGroup::Update(double)` |
| `0x1002b770` | `gbParticleGroup::Render()` |
| `0x100299b0` | 曲线取值（未导出名，`ComputeParticleParam` 四次调用它） |

## 文件布局

```text
char[6]  "sgpar\0"                 // 0x1002bcf9 处比较
u32      版本                       // > 0x10000 时多一个字段
u32      ?                          // 出厂文件里只有 200 和 400 两种
u32      ?                          // -> effect+0xb4
u32      ?                          // -> effect+0xb8
f32      ?                          // -> effect+0x118，仅版本 > 0x10000
u32      group_param 数             // -> effect+0x9c
u32      emitter 数                 // -> effect+0xa0
u32      group 数                   // -> effect+0x98
gbPartGroupParam[group_param 数]    // 每条 21560 字节
gbPartEmitter[emitter 数]           // 每条 32008 字节
gbParticleGroup[group 数]           // 每条 76 字节
```

头固定 38 字节（出厂文件版本都是 0x10001，那个 float 都有）。三段
记录的顺序就是三个装载循环的顺序：0x1002bf16、0x1002bf4e、0x1002bf7d。

### 记录大小是怎么定下来的

`gbParticleGroup` 那条能直接从反汇编数出来：`0x40` 字节 + 三个 dword
= 76。另外两条是几十次读取、中间有分支，静态数不出总数，改用出厂文件
反解——55 个 `.par` 只有四种 (params, emitters, groups) 形状：

| 形状 | 文件大小 |
|---|---|
| (1,1,1) | 53682 |
| (2,2,2) | 107326 |
| (2,1,2) | 75318 |
| (2,2,1) | 107250 |

四个方程定死三个未知数：**21560 / 32008 / 76**。group 的 76 和反汇编
独立对上，是这组解正确的交叉验证。`pal3_par_census` 测试要求 55 个
文件消耗的字节数都正好等于文件长度。

## gbPartGroupParam（21560 字节）

`LoadFromFile` 的每次读取都落在结构体自己的偏移上，所以文件布局就是
对象布局：

```text
0x000  char[0x20]   名字      "Liquid" / "fire" / "smoke" / "Fog" / "chengtu"
0x020  u32 × 6
0x038  char[0x100]  贴图      "partexture\smoke.jpg" / "partexture\huo3.jpg"
0x020  ---          引擎不读          // 见下
0x024  f32          重力/加速度      // snow 和 huohuo 都是 9.81，火焰是负值
0x028  f32          阻力的速度阈值    // 低于它不施加阻力
0x02c  u32          没找到读它的地方   // 出厂值只有 0 和 1
0x030  u32          阻力算法          // 非 0 按当前尺寸缩放，0 走线性
0x034  f32          阻力系数          // snow 0.4，烟和火 3~16
0x038  char[0x100]  贴图      "partexture\smoke.jpg" / "partexture\huo3.jpg"
0x138  u32 × 8                 // 其中 0x148、0x14c 是自转的随机区间
0x15c  u32          初始帧的随机上界  // CreateOneParticle 取 rand(0, 这个-1)
0x160  u32          贴图帧数
0x164  u32          混合模式  // 0 = p_add.gbf 加法，非 0 = p_alpha.gbf
0x168  5300 字节    轨道 0  Size graph: m/t
0x161c 5300 字节    轨道 1  Rotation graph: d/t
0x2ad0 5300 字节    轨道 2  VertexAlpha graph: d/t
0x3f84 5300 字节    轨道 3  RgbColor graph: d/t
0x48f8 指针         运行时的 gbTexture      // GetTexture 返回它
0x493c 指针         运行时的 gbRenderEffect // GetRenderEffect 返回它
```

**轨道在文件里的间距是 0x14b4 = 5300，不是反汇编里看到的 0x11e4。**
内存布局每条轨道少 720 字节（装载时有一段 720 字节没往轨道里放），
所以 `ComputeParticleParam` 里的 0x134c / 0x2530 / 0x3714 是内存偏移，
文件偏移要按 5300 递推。这个差每条累加 720：0、720、1440、2160。

每条轨道自带标签，就在 `轨道 + 8`：`Size graph: m/t`、
`Rotation graph: d/t`、`VertexAlpha graph: d/t`、`RgbColor graph: d/t`
——**文件自己说明了四条轨道各是什么**，和从 `ComputeParticleParam`
反推的结论逐条吻合。

名字和贴图两条在 55 个文件里全部读得通，census 已经断言贴图必须以
`partexture\` 开头。

### 四条轨道各驱动什么

`ComputeParticleParam(unit, t, ?)`（0x1002aed0）按归一化寿命 `t` 取值，
写进 `gbParticleUnit`：

```text
unit+0x14 = clamp((int)(param[0x160] * t), 0, 帧数-1)    // 贴图第几帧
轨道 0 (0x168)  -> unit+0x40      // Render 拿它乘大小，是尺寸曲线
轨道 1 (0x134c) -> 和 rand(param[0x148], param[0x14c]) 一起累加进 unit+0x4c
轨道 2 (0x2530) -> unit+0x44
轨道 3 (0x3714) -> 分量 0/1/2 取整写进 unit+0x4a / 0x49 / 0x48  // R/G/B
unit+0x4b（alpha）= (int)(unit+0x44 * 255.0)                      // 即轨道 2
```

`InitParticleParam`（0x1002afc0）就是 t=0 时的同一套，它把 alpha 那步
写得最清楚：`fld [esi+0x44]; fmul 255.0; ... mov byte [esi+0x4b], al`。
所以**轨道 2 是 0..1 的 alpha，乘 255 进颜色字节**，和它
`VertexAlpha graph` 的标签对得上。

`huo.par` 的 RgbColor 三个分量末值是 255 / 140.2 / 139.0，正是火焰的
橙红；`paopao.par` 是 127.5 / 113.5 / 100.7，泡泡的灰蓝。

`Render`（0x1002b770）每颗粒子读 `unit+0x34/0x38/0x3c`（位置）、
`unit+0x40`（乘进尺寸）、`unit+0x48`（整个 dword 当颜色）、`unit+0x18`。
所以 0x48..0x4b 这四个字节是 RGBA，轨道 3 写的正是其中三个。

### 轨道的取值方式

曲线取值函数 `0x100299b0(out_vec3, t)`：

* 每条轨道是**三分量**曲线，最多 **100 个关键帧**（`cmp edx, 0x64`）；
* 关键帧时间在 `轨道+0x3bc` 起、步长 0xc；
* 找到第一个时间大于 `t` 的关键帧后，在它和前一帧之间做线性插值，
  三个分量分别插，结果写进 `out_vec3` 的三个 float；
* `t` 落在范围外时直接取 `轨道+0x3b4`、`+0x870`、`+0xd2c`（各分量的
  末帧值），或者 `+0x864`、`+0xd20`、`+0x11dc`。

关键帧的排布已经定死并在 55 个文件上验过：

```text
轨道 + 0x3b0 + 分量 * 0x4bc + i * 12  ->  { f32 时间, f32 取值, f32 未用 }
```

* 一个关键帧 12 字节，第一帧在轨道 +0x3b0；
* 文件里三个分量相隔 **0xf0 = 240 = 20 × 12**，即每个分量 20 帧；
  内存里它们相隔 0x4bc（装载时展开成 101 格），取值函数的越界回落
  常量 0x3b4 / 0x870 / 0xd2c 说的是内存布局；
* 前三条轨道是标量，`ComputeParticleParam` 只取 `out[0]`；只有
  RgbColor 那条把三个分量当 R/G/B 全用上；
* 时间是归一化到粒子寿命的，从 0 起递增、**正好收在 1.0**；
* 用不到的帧位是 0，个别文件里是 MSVC 的 0xCC 填充。

这个排布和取值函数的算式自洽：`fsub [ecx + 12i + 0x3a4]` 是第 i−1 帧
的时间、`[ecx + 12i + 0x3b0]` 是第 i 帧的时间，取值在各自后面四字节
（`0x3a8 + 12i`、`0x3b4 + 12i`）；越界分支回落到 `0x3b4`、`0x870`、
`0xd2c`，正是三个分量第 0 帧的取值，间距同样是 0x4bc。

实例（`paopao.par` 的尺寸曲线）：

```text
(0.000, 0.680) (0.292, 0.837) (0.510, 1.055) (0.777, 1.150) (1.000, 1.390)
```

`LoadFromFile` 读一条轨道的顺序是 `{u32, 64, 128, 12, 12, 720, 3636}`，
合计 4576，而相邻两条轨道的起点相差 0x11E4 = 4580。

## 粒子的生命周期

* `CreateOneParticle`（0x1002b2a0）：`unit+0x10` = 寿命 =
  `rand(发射器数据[0x1c], [0x20])`；`unit+0x50` = ±1 的自转方向；
  `unit+0x18` = `rand(0, param[0x15c] - 1)` 的起始帧；随后按 t=0 取四条
  轨道。
* `UpdateOneParticle`（0x1002b590）：`unit+0x08`（double）累加 dt，
  超过 `unit+0x10` 就算死；`t = 年龄 / 寿命`；
  `unit+0x14 = (int)(param[0x160] · t)` 是当前帧；再依次调
  `ComputeAirResist` 和 `ComputeParticleParam`，最后按 `param[0x24]`
  把加速度并进速度（`unit+0x1c/0x20/0x24`）。
* `ComputeAirResist`（0x1002ad80）：对速度取模、归一化，拿速度和
  `param[0x28]` 比，再按 `param[0x30]` 选分支。

## gbPartEmitter（32008 字节）

```text
0x00  char[0x40]   名字         "fire Emitter" / "Snow Emitter"
0x40  f32 × 3      发射原点
0x4c  f32 × 3      方向扩散      // InitVel 用 rand(-[0x4c], [0x4c])
0x5c  char[0x40]   形状网格      "csf\star.csf" / "csf\yuan.CSF"，不用时为空
0x98  u32          形状
0x9c, 0xa0  f32    周期的两端    // 归一化进度 = 已过时间 / (0x9c - 0xa0)
0xa4  u32          是否循环发射   // SetLoopEmit 写的就是它
0xac..0xc0        形状 0（盒）的三轴 min/max：x rand(0xac,0xb8)、
                   y rand(0xb0,0xbc)、z rand(0xb4,0xc0)
0xc4, 0xc8        形状 1 用的两个半径
0xcc              形状 ≥2：一张点表，随机挑一个点（步长 12），
                   再用 0xd8 处的矩阵变换
0x0d0  5300 字节   轨道 0  DirectionEmitter graph: x,y,z/t
0x1584 5300 字节   轨道 1  ElementRandomVelocity graph: m/s/t
0x2a38 5300 字节   轨道 2  EmissionRate Graph: rate/t
0x3eec 5300 字节   轨道 3  ElementLifeTime Graph: t/t
0x53a0 5300 字节   轨道 4  SizeMultiplier Graph: */t
0x6854 5300 字节   轨道 5  RotationMultiplier Graph: */t
```

发射器一样自带标签（`轨道 + 8`），六条轨道把发射行为全说清楚了。
轨道 0 后面还跟一句注释 `Red:X,Green:y,Blue:z`，说明它的三个分量是
xyz 而不是颜色。

`ComputeParticleInitPos`（0x1002a400）按 `0x98` 分三种形状，最后都加上
发射原点。`ComputeParticleInitVel`（0x1002a590）从发射器数据取速率区间，
再用 `±[0x4c]` 做方向扩散。

## 发射率

`gbParticleGroup::Update(dt)`（0x1002b6a0）：

```text
group+0x40 (double) += dt                       // 发射器活了多久
n = ComputeEmitterData(&group+0x48, 已过时间, dt)
重复 n 次 CreateOneParticle(&group+0x48)
再逐颗 UpdateOneParticle
```

`ComputeEmitterData`（0x1002a2c0）先算本周期内的归一化进度：

```text
周期 = emitter[0x9c] - emitter[0xa0]
n = (int)(已过时间 / 周期)
若 n > 1 且 emitter[0xa4] == 0（不循环）-> 返回 0，不再发射
t = (已过时间 - n·周期) / 周期
```

再按 `t` 取四条轨道，两两取小大填进 `gbEmitterData`：

| 发射器轨道 | 落到 | 用途 |
|---|---|---|
| 0 `DirectionEmitter` | data+0x00..0x08 | 发射方向（三分量 xyz） |
| 1 `ElementRandomVelocity` | data+0x0c / +0x10 | 初速区间（m/s） |
| 2 `EmissionRate` | data+0x14 / +0x18 | **每秒发射个数的区间** |
| 3 `ElementLifeTime` | data+0x1c / +0x20 | 寿命区间（秒） |

最后一步就是发射率本身：

```text
n_float = rand(data[0x14], data[0x18]) · dt + data[0x28]
n       = (int)n_float
data[0x28] = n_float - n        // 小数留到下一帧
return n
```

`data+0x28` 是个 double 累加器，和 `gbDropGroup::CreateUnit` 里那套一个
思路，保证帧率变化时每秒的发射数不变。

## gbParticleGroup（76 字节）

```text
0x00  byte[0x40]   头
0x40  u32          group_param 下标   // 0x1002b121 处乘 0x4940 定位
0x44  u32          emitter 下标       // 0x1002b138 处乘 0x6c78 定位
0x48  u32          读进 this+0x84 后立刻被 effect+0xb0 覆盖，是死值
```

## 混合模式

`param+0x164` 决定用哪个渲染效果，0x1002ad40 处：

```text
mov eax, [ebx + 0x164]
test eax, eax
jne  -> push "gfxscript\p_alpha.gbf"
     -> push "gfxscript\p_add.gbf"
call 取渲染效果
mov [ebx + 0x493c], eax          // GetRenderEffect 返回的就是它
```

两个 `.gbf` 在 `basedata\gfxscript\` 下都在。出厂数据里 68 个 group
param 有 **65 个走加法**，走 alpha 的只有三个，而且全叫 `Snow`
（`snow.par`、`xue.par`、`kuzhilanye.par`）——雪片用 alpha、火和烟用
加法，语义上完全说得通。

`gbDropGroup`（140 下雪用的那套，和 .par 是两个系统）在构造里直接
`push "gfxscript\p_add.gbf"`（0x10031ed3），所以**飘雪也是加法混合**。

绕过的弯路记一笔：`0x30` 一度看着像加法混合的开关（snow 是 0、火和
泡泡是 1，方向正好对），但 `ComputeAirResist` 在读它，这个猜测是错的。
真正的字段是 0x164。

Pal3.Unity 在这个问题上帮不上忙：它根本不读 `.par`，而是
`Resources.Load(prefab)`，每个特效在 Unity 里手工重做了一遍
（`EffectManager.Execute(EffectPlayCommand)` → `GetVfxEffectPrefab`）。

## 空气阻力

`ComputeAirResist(unit, dt)`（0x1002ad80，`edi` 是 group param、
`ebx` 是粒子）：先对速度取模归一化，拿速度和 `param[0x28]` 比，低于
阈值就不施加阻力；再按 `param[0x30]` 分两种算法——

* 非 0：阻力 = `unit[0x40]`（当前尺寸，也就是 Size 轨道的取值）×
  `param[0x34]` × 归一化方向，粒子越大越受阻；
* 0：直接 `param[0x34]` × 速度，线性阻力。

所以 `param[0x34]` 是阻力系数：雪 0.4，烟和火 3~16。

## 0x20 和 0x2c

`0x20` **引擎根本不读**。两条独立证据：

1. 把 gbengine.dll 里粒子相关的整段代码（约 4900 条指令，涵盖
   `ComputeAirResist` / `ComputeParticleParam` / `InitParticleParam` /
   `CreateOneParticle` / `UpdateOneParticle` / `Update` / `Render` /
   `ComputeEmitterData` 及邻近函数）扫一遍，以 group param 为基址读
   `+0x20` 的指令一条也没有，只有 `LoadFromFile` 自己往里写的那个 `lea`。
2. 出厂数据自己作证：68 个 group param 里 **19 个在 0x20 处是
   `0xCDCDCDCD`**，MSVC 的未初始化填充。真要拿这个值参与运算，那 19 个
   特效早就不成样子了。

`0x2c` 同样没找到读它的地方，但它的值一直是干净的 0 或 1（68 个里 8 个
是 1），所以不能断言它是死字段，只能说粒子这套代码不看它。可能是编辑器
自己留的开关。

census 把这两条也钉住了：0x20 只允许是 0 或未初始化填充，0x2c 只允许
0/1，出现别的值就说明这套读法有问题。

## 查过但没用的路

* **Pal3.Unity**：它的 DataReader 只有 Bmp/Cpk/Cvd/Dat/Dxt/Gdb/Ini/Lgt/
  Mov/Msh/Mtl/Mv3/Nav/Pol/Sce/Scn/Tga/Txt，**没有 Par**。特效走
  `EffectManager` → `GetVfxEffectPrefab` → `Resources.Load(预制体)`，
  每个特效在 Unity 里手工重做了一遍，对这个格式帮不上忙。
* **公开资料**：搜 `sgpar` / gbengine 粒子格式，中英文都没有人写过。
* **`newPEdit.exe`**（就在 `effect\gbParticle\` 里，548 KB，做这些 .par
  的编辑器）：确认那十条轨道标签就出自它，但对话框上的字段名是运行时
  填的，资源段里只有控件框架，没拿到更多字段名。真要继续挖，得反它的
  保存流程。

代码侧的解析器在 `yaobow/fileformats/src/pal3/par.rs`，测试在
`yaobow/shared/tests/pal3_par_census.rs`。
