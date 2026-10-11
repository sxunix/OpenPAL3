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

`data/pal3` 这一半和出货的 `PAL3_unpacked.exe` 对得上，逐条核过的例子：

| 源码 | exe |
| --- | --- |
| `ui/uilistbox.cpp:51` `m_seltexcolor(168,168,168,200)` | `0x43e045` 起写 `+0x498..0x49b` |
| `uibase.cpp:533` `UIRenderQuadList` 用 `UI_EFFECT_TEX_COLOR` | `FUN_00540c60` 取 `DAT_01895058` |
| `scene.cpp:1036` `GetObjByID` 拿 `BYTE id` 和 WORD 参数整个比 | `0x42157a` `movzx bx, byte [edx]` / `cmp bx, di` |
| `role.cpp:2982` `min(100, box.ylen*0.6f)` | `0x418e94` 起 `fmul [0x56aa4c]` / `fcomp` 取小 |
| `rolecamera.cpp:487` `r=(1-cos(r*PI))*0.5f` | 镜头任务的插值类型 1 |

**`data/engine` 这一半不一样。** 至少有一处出货的 `gbengine.dll` 和源码对不上：

* `gbGeomNode::GetTotalOBBox`（`geometry/gbgeomnode.cpp:103`）源码写
  `tx/ty/tz = (min+max)*0.5` 三个轴各算各的；
* DLL 里 `0x10010222` 起是
  `[esi+0x30] = (max.x+min.x)*0.5`、`[esi+0x34] = (max.x+min.x)*0.5`、
  `[esi+0x38] = (max.y+min.y)*0.5`——**x 中心写了两遍、z 中心丢了**。

移植端 `ScnScene::object_model_box` 照 DLL 抄。所以：**引擎那一半的每一条
都要和 `gbengine.dll` 对过再用**，源码只能当线索。

还没对过的一处：`gbIpolMesh` 读 `.cvd` 时源码是把所有帧扫一遍取包围盒
（`geometry/gbipolmesh.cpp:234`），移植端现在只用第 0 帧；`gbIpolMesh`
不是导出符号，DLL 里那段还没找。

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
