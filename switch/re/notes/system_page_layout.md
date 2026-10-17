# PAL3 v1.4 — 系统页 (GameMainUI/System) 笔记

来源：参考源码 `gamemainui/SystemUI.cpp` / `SystemUI.h`（与出货 exe 同版），控件行为看
`ui/uidragbar.cpp`、`ui/uibutton.cpp`、`ui/uipushbutton.cpp`、`gamemainui/muidialog.cpp`。
坐标都是 800×600。移植：`yaobow/src/openpal3/system_page.rs`（页面）、`muidialog.rs`（确认框）、
`scripts/openpal3/system_menu.p7`（底色 + 工具栏）。

> 2026-10-17 以前移植端的系统页是按贴图形状「摆」出来的（4 条一页、9 页翻、储存/读取在右侧框里），
> 位置全是估的；现在照源码重做。

## 1. 布局

| 控件 | rect / 位置 | 贴图 |
|---|---|---|
| 右侧框 | (670,38) 114×505 | `System\Frame.tga` |
| 框线 | 源码里注释掉了，**本页不画** | — |
| 时间金钱牌 | (596,0) 203×64 | `Communal\TimeMoney.tga` |
| 金钱 / 时间字 | (618,−4,738,28) / (668,16,748,48)，色 (31,251,255) | — |
| 上 / 下装饰条 | (40,23) 584×20 / (40,500) 584×37 | `TopBar` / `BottomBar` |
| 储存进度 / 读取进度 | (146,5,264,37) / (398,5,516,37)，`UIPushButton` | `Save0/1` + `Saveon`、`Load0/1` + `Loadon` |
| 离开游戏 | (696,456,769,531)，`UIButton` | `ExitGame0/1/2` |
| 存档条 | 第 i 条 (12, 40+92·(i−首行)) 610×92，36 条露 5 条 | `Note0/1/2`，选中的禁用、盖 `Note2.tga` |
| 往回翻 | (634,38) 25×28（id `IDB_SYSTEM_PAGEUP`，`MoveDw`） | `pgup0/1/2` |
| 往后翻 | (634,502) 25×28（`IDB_SYSTEM_PAGEDOWN`，`MoveUp`） | `pgdw0/1/2` |
| 滑块 | 条 (640,66,654,503) `hdt.tga`；手柄 26×28 | `Communal\scroll\scrollbar0/1/2` |
| 音乐 / 音效字 | (684,368) / (726,105) 56×24 | `MusicText` / `MusicEffectText` |
| 音乐条 | (698,56,718,364) `MusicBar.tga`，手柄 26×30 | 同上 |
| 音效条 | (744,136,764,444) `MusicEffect.tga`，`DRAG_RIGHT` | 同上 |

存档条内（相对条左上）：编号 (29,30) 32×32 居中、字色是 `UIWnd` 默认的 (5,5,0)；模式字
(62,12) 29×64 `cc.tga`（储存）/`dq.tga`（读取）；头像从 (96,12) 起每个 58×64，只排在队的人
（`jt/xj/lk/zx/cq.tga` 亮、`…dk.tga` 暗）；三行字 (312,6)/(312,29)/(312,52) 156×32：
「AssisLang 49 时:分:秒」「48 金钱」「352 场景名」，色 (31,251,255)，时 ≥100 用三位；
缩略图 (490,12) 93×69，只在选中的那条显示。等级字源码里设成空串，不画。

## 2. 拖动条（`UIDragBar`，`VERTICAL_DRAG`）

长度 = (底 − 2 + `BRadius`) − (顶 + 2 + `TRadius`)；`SeekVal(v)` 后
手柄左 = `DRAG_LEFT`：条左 − 宽 + `DragRadius`，`DRAG_RIGHT`：条右 − 宽 + `DragRadius`；
手柄顶 = `DRAG_LEFT`：条顶 − 高/2 + 2 + `TRadius` + 进度，`DRAG_RIGHT`：条底 − 高/2 − 2 + `BRadius` − 进度。

| | TRadius | BRadius | DragRadius | 最大值 | 长度 | 手柄位置 |
|---|---|---|---|---|---|---|
| 滑块 | 14 | −14 | 20 | (36−4)·92−28 = 2916 | 405 | x 634，顶 68 起 |
| 音乐 | 20 | −15 | 23 | 100 | 269 | x 695，顶 63 起，**往下**变大 |
| 音效 | 15 | −20 | 2 | 100 | 269 | x 740，顶 407 起，往上变大 |

按住手柄拖（离手柄超过一个手柄高就脱手）；拖的时候每帧发 `UIWM_DRAGBAR_CHANGE`。
音乐条 → `SetMusicMasterVolume(v/100)`，音效条 → `Set2D/3DMasterVolume(v/100)`；Bink 视频的音量也取
2D 那个。音量存在存档里（`Archive` 的 soundOffset 段），读档套回。
移植：引擎加了「音效 / 音乐」两组音量（`AudioEngine::set_group_volumes`，音源默认是音效，背景音乐
`set_music(true)`），存档多一个 `volumes` 字段。

## 3. 行为

- 开页（`UIWM_ACTIVE`）：读全部存档摘要；能存就进储存、「储存」可点，否则进读取、「储存」禁用。
  `Reset` → `SeekNote`：选中上次存 / 读的那条（注册表 `PAL3_SAVE\SEEK`，移植端存在
  `Save/seek` 文件），序号 >4 时滚到让它在第 5 行。
- 单击存档条：选中，wa001。**双击**（移植端：点已选中的那条）或回车：
  - 储存：有档「覆盖」(286，`"\n      "`)、没档「存到这里」(287，`"\n "`)，是 → 存、wa029；
  - 读取：船迷宫 m04/m05 里弹 398；有档「读这个」(283)，是 → wa030、读档；没档弹 284。
  - 存 / 读的确认框是 `DoModel(false)`：点按钮不出声。其余框点「确定」放 wa001。弹框都放 wa003。
- 储存 / 读取按钮：换模式（重新 `Reset`），wa016；禁用的「储存」被点：按钮自己 wa004，页面再弹 343。
- 翻页键 / 滚轮 / 键盘上下：`MoveUp`/`MoveDw` 挪一条，wa016，到头 wa004。PgUp / PgDn 切储存 / 读取。
- 离开游戏：问 261，是 → `SwitchUIRender(M_COVER)` 回开始菜单。
- 框（308×109，左上 (250,200)，字色 (219,219,0)）开着时页面和工具栏都不收输入。
- 缩略图：进菜单前抓的整屏（`PAL3::m_screenImg`），存档时写 `Save\PAL3_%04d.JPG`。移植端在状态菜单
  打开的那帧前抓最后上屏的画面（菜单第一帧先不画），存档时取中间 4:3 缩到 186×138 存
  `Save/<槽>.png` [移植]；Switch 的 GL 后端没有读回，不出缩略图。

## 4. 原样保留的毛病

- 键盘上下换选中时不换头像亮暗（只有点选会换）。
- `SeekNote` 在序号 ≤4 时滑块位置按「序号 × 92」摆，和列表（不滚）对不上。
- 非选中的可见条亮、暗两套头像都画（暗的盖在上面）。
