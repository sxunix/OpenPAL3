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
0x138  u32 × 13                // 其中 0x148、0x14c 是一对随机区间（见下）
0x160  u32          贴图帧数
0x168  4580 字节    轨道 0
0x134c 4580 字节    轨道 1
0x2530 4580 字节    轨道 2
0x3714 4580 字节    轨道 3
…      char[0x100] × 2
```

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
轨道 3 (0x3714) -> 三个分量分别取整写进 unit+0x49、0x4a…  // 颜色字节
```

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
* 三个分量相隔 0x4bc = 1212 = 101 × 12，正好各 101 帧的空间；
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

## gbPartEmitter（32008 字节）

同样的味道：`0x00` 起 0x40 字节的头、`0x40` 和 `0x4c` 各三个 float、
`0x58` 一个 `char[0x40]`、然后四段一样的 4580 字节轨道（起点 0xac 一带、
0x1304、0x24e8、0x36cc）。

## gbParticleGroup（76 字节）

```text
0x00  byte[0x40]   头
0x40  u32          group_param 下标   // 0x1002b121 处乘 0x4940 定位
0x44  u32          emitter 下标       // 0x1002b138 处乘 0x6c78 定位
0x48  u32          读进 this+0x84 后立刻被 effect+0xb0 覆盖，是死值
```

## 还差什么

要真正把 143/152 画出来，剩下的是：

1. `0x020` 和 `0x138` 两组 dword 里的发射率、寿命、初速、空气阻力
   （`ComputeAirResist` 会告诉哪个偏移是阻力系数）；
3. 发射器形状（`ComputeParticleInitPos` / `ComputeParticleInitVel`）；
4. 渲染侧：`Render` 用的混合模式和 `gbRenderEffect`。

代码侧的解析器在 `yaobow/fileformats/src/pal3/par.rs`，测试在
`yaobow/shared/tests/pal3_par_census.rs`。
