# `basedata.cpk` 里的那份源码：怎么用、信到什么程度

## 它是什么

`basedata.cpk` 里嵌了两个 zip（EOCD 在 `0x2e705a` 和 `0x4f1fb8`）。大的那个
925 个条目，是 PAL3 + gbengine 的源码树，22.7 万行 cpp/h。解到仓库外的
`仙剑奇侠传3/re/src_from_cpk/`，**不进版本库**。

```
data/pal3/{ui, gamemainui, gameui, gamebase, combat, subgame, script,
           effect, fight, support, assistant system}
data/engine/{gfx, map, actor, audio, effect, geometry, math, physics,
             globalvr, support}
data/cpack
```

文件是 GBK + CRLF：`grep` 要 `LC_ALL=C grep -a`，读要过
`iconv -f gbk -t utf-8 -c | sed 's/\r$//'`。

一条硬规矩：**yaobow 是 GPL，这是软星的源码，一行都不能抄进移植端**。
用法是当事实来源查行为和字段含义，注释和笔记里照旧写地址，源码只做交叉验证。

## 信到什么程度

### 怎么核：exe 里还留着 `__FILE__` + `__LINE__`

`EXCEPTION(msg)` 这个宏在调用点压 `__LINE__` 和 `__FILE__`，release 版没优化掉。
`switch/re/srcmap.py` 把 `.text` 里每一处 `push <"D:\Project\PAL3\…cpp">` 和
紧挨着的那条 `push imm` 配对打出来：**28 个文件、108 处**。把行号丢回
`src_from_cpk` 对应文件的那一行，**几乎每一处都正好落在 `throw EXCEPTION(...)` 上**
（少数几处是启发式抓错了相邻的 push，不是版本不同）。覆盖到的文件：
`FileSys.cpp`、`PalScript.cpp`、`UIListBox.cpp`、`LoadData.cpp`、`RenderTarget.cpp`、
`EffectSystem.cpp`、`EffCylinder/Explosive/Grid/Meteor/Model/Quad/Shock/Mgr.cpp`、
`CB2DEffect.cpp`、`CB3DEffect.cpp`、**`CBRoleState.cpp`（1484 / 1494）**、
`CBStage.cpp`、`CBStageCam.cpp`、`CBStageEffect.cpp`、`CBStageRole.cpp`、
`CBStageScene.cpp`、`CBStageShadow.cpp`、`CBSystem.cpp`、`CBUI.cpp`、
`Encampment_scene.cpp`、`UI_TextureInfo.cpp`。

也就是说这些文件是**逐行相同**的源码，不只是「差不多」。

另外几条手工核过的：


| 源码 | exe |
| --- | --- |
| `ui/uilistbox.cpp:51` `m_seltexcolor(168,168,168,200)` | `0x43e045` 起写 `+0x498..0x49b` |
| `uibase.cpp:533` `UIRenderQuadList` 用 `UI_EFFECT_TEX_COLOR` | `FUN_00540c60` 取 `DAT_01895058` |
| `scene.cpp:1036` `GetObjByID` 拿 `BYTE id` 和 WORD 参数整个比 | `0x42157a` `movzx bx, byte [edx]` / `cmp bx, di` |
| `role.cpp:2982` `min(100, box.ylen*0.6f)` | `0x418e94` 起 `fmul [0x56aa4c]` / `fcomp` 取小 |
| `rolecamera.cpp:487` `r=(1-cos(r*PI))*0.5f` | 镜头任务的插值类型 1 |

**但也不是同一次编译。** `gameui/UIBigMap.cpp` 的 38 个目的地里，36 个和 exe
逐字段一致，下标 15（黄泉路 M21）和 25（熔岩地狱 M20）这两**整块**
（矩形、贴图、按钮色）在 exe 里是对调着写的：

| | 下标 15 = 黄泉路 | 下标 25 = 熔岩地狱 |
| --- | --- | --- |
| 源码 `UIBigMap.cpp:106`/`:122` | `huangquanlu.tga`，(1007,100) 209×61 | `rongyandiyu.tga`，(903,0) 120×110 |
| exe（`[0x56af24]=IDB[15]` @0x446b04、`[0x56af4c]=IDB[25]` @0x446c69） | `rongyandiyu.tga`，(903,0) 120×110 | `huangquanlu.tga`，(1007,100) 209×61 |

两块各自自洽，所以画出来的地图两种写法一模一样，差别只在点哪一块去哪儿。
按贴图内容（`huangquanlu.tga` 是一条细长的发光的路，`rongyandiyu.tga` 是
火鬼那一团）和地理位置（细长那条紧挨着酆都 (1006,155)）看，源码是对的，
出货的 exe 把这两个下标写反了。移植跟 exe，见 `bigmap.rs` 的
`the_two_swapped_blocks_are_kept_as_the_exe_has_them`。

## `data/engine` 和 `gbengine.dll`

> **撤回（2026-10-12）**：先前这里写「`data/engine` 这一半不一样」，证据是
> `gbGeomNode::GetTotalOBBox` 在 DLL 里「x 中心写了两遍、z 中心丢了」。
> **那是读错了。** `0x10010222` 起的三段中间夹着一条 `pop edi`（`0x1001022c`），
> esp 挪了 4，所以第二段的 `[esp+0x14]`、第三段的 `[esp+0x18]` 指的是第一段
> `[esp+0x18]`、`[esp+0x1c]` 那两个格子——三个轴各算各的，和源码
> `gbgeomnode.cpp:104` 一模一样。**读带 push/pop 的浮点序列时，每条访存都要
> 按当时的 esp 重算。** 移植端照着错版抄的那一处（多网格 CVD 的 OBB 中心
> `[cx, cx, cy]`）已经改回 `[cx, cy, cz]`。

引擎这一半也能拿 `__FILE__`/`__LINE__` 核。DLL 里只剩三个路径字符串
（`D:\Project\gbengine9\support\cpack\…` 的 `CPKFast.cpp`、`DiscIOMgr.cpp`、
`MemMgr.cpp`），25 处 xref，行号全部落在 `data/engine/support/cpack/` 对应文件的
`ERR(...)` 上——**cpack 这一块是逐行相同的**。几何、gfx 那些模块没有留下锚点，
目前**没有**任何一条已知的 `data/engine` ↔ DLL 分歧；但能核的只有 cpack，
所以引擎侧改行为之前还是先去 DLL 里看一眼。

对过的一处：**一致**的一处：`gbIpolMesh` 读 `.cvd` 时把所有帧扫一遍取包围盒
（`geometry/gbipolmesh.cpp:234`），出货的 DLL 一样。`gbIpolMesh` 没有导出符号，
是顺着导出的 `gbGfxManager::NewMesh`（0x1001e160，`type==1` 时 `new(0x8c)` +
构造 0x10011b40、虚表 0x100f5614）和 `gbGeomNode::LoadCVDFromStream`
（0x10010cd0，在 0x10010e00 调 mesh 的 `LoadFromStream`）找到
`gbIpolMesh::LoadFromStream = 0x10011c60` 的：0x10011ea1 把 min/max 置成
±FLT_MAX，外层帧循环 0x10011ed7…0x10012022、内层顶点循环
0x10011f35…0x10012000（每顶点 `esi += 0x20`），六个 min/max 比较
（0x10011f79 起）在两层循环里面。移植端 `object_model_box` 已改成取并集：
503 个 `.cvd` 的 1267 个网格节点里，179 个的并集比第 0 帧大，最大一处
（`/scene/Q09/C/_cn12a.CVD`）差 34262 单位。

## 几份特别有用的文件

| 文件 | 内容 |
| --- | --- |
| `data/pal3/script/pal3.def` | SCE 指令定义表，带中文说明。只到 113 号，是早期快照，但 0–113 的参数和语义权威 |
| `data/pal3/script/scriptcmd.cpp` | 每条指令的实现，3714 行 |
| `data/pal3/gamebase/scenedata.h` | `.scn` 的全部结构：`SceneInfo`、`NPCInfo`、`ObjectInfo`（含 45 种物件类型的中文注释）、`FLOOR_KIND` 24 种地面 |
| `data/pal3/uibase.cpp` | 全部 `UIRenderQuad*` 画法 + 十张 `.gbf` 的用途，见 [`ui_render_states.md`](ui_render_states.md) |
| `data/pal3/ui/*` | 整套控件库：listbox、button、progressbar、static、dialog |
| `data/pal3/combat/Eff*.cpp` | flexeff 十几个类的完整实现，见 [`flexeff_format.md`](flexeff_format.md) |
| `data/engine/math/gbmath.cpp` | 角度按度、右手系、矩阵按行向量、矩阵栈「后压的先作用」——这四条的出处 |
| `data/engine/physics/gbparteffect.cpp` | `.par` 发射器和 `.csf` 点云的读法 |

## `.scn` 磁盘布局 = 内存里的 `ObjectInfo`

`scenedata.h` 的 `ObjectInfo` 和移植端 `scn_loader.rs` 的字段逐个对得上：

| 文件偏移 | `ObjectInfo` | `ScnNode` |
| --- | --- | --- |
| 0 | `BYTE id` + `BYTE active` | `index`（低字节才是 id） |
| 2 | `BYTE times` + `BYTE siwtch` | `w2` |
| 4 | `char name[32]` | `name` |
| 0x24 | `BYTE ctype` + `BYTE cango` | `w24` |
| 0x28 | `gbVec3D pos` | `position` |
| 0x34 | `float rotate` | `rotation` |
| 0x38 | `RECT rect` | `nav_trigger_coord_*` |
| 0x48 | `BYTE type` + `BYTE savestate` | `node_type` |
| 0x4a | `BYTE onlayer` + `BYTE wuling` | `nav_layer` |
| 0x4c | `int param[6]` | `ladder_nav_coord*` / `ladder_switch_layer` / `b49` 头 4 字节 |
| 0x80 | `DWORD scriptid` | `sce_proc_id` |
| 0x84 | `PolyLine path`（`int numpt` + 16 × `gbVec3D`） | `d84` + `b88[192]` |
| 0x148 | `WORD linkobj` | `w148` |

所以 `index` 是 `id | active << 8`——M02 那种「从 257 编起」的场景，
高字节就是 `active`。
