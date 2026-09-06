# 道具使用（战斗外 道具页 / 战斗内 道具指令）

两条通路都以 `pal3_softstar.gdb` 的 `GameItemInfo`（800 字节）为准。移植端的
结构在 `yaobow/fileformats/src/pal3/gdb.rs`。用到的字段：

| 偏移 | 字段 | 说明 |
|---|---|---|
| 0x264 | `item_type` | 1 治疗 2 投掷 3 仙宝 … |
| 0x274 | `item_special_type` | 见下表 |
| 0x275 | `target_range_type` | 1 我方单体 2 我方全体 3 敌单体 4 敌全体 |
| 0x276 | `place_of_use_type` | **两条通路都不看** |
| 0x277 | `attribute_impact_type[12]` u8 | 0 绝对 1 百分比 2 回满 3 加上限 |
| 0x284 | `attribute_impact_value[12]` i16 | 精气神武防速运 水火风雷土 |
| 0x29c | `combat_state_impact_types[31]` u8 | 实机数据全 0，没人读 |
| 0x2bc | `combat_state_impact_value[31]` i16 | **状态码**：1 加 / 2 解，按状态枚举下标 |

状态枚举（0 基；战斗位标记 `k = 枚举 + 1`）：0–4 风雷水火土毒，5–12
武±防±运±速±，13 定 14 封 15 禁 16 眠 17 乱 18 狂 19 镜 20 避 21 界 22 隐形
23 濒死 24 死亡 25 避毒 26 任意毒 27 辟邪 28 退魔 29 神眼 30 精进。

`combat_ai.md` §3 里那条「0x2bc 数组类型宽度对不上」的 [存疑] 可以撤：
0x29c 才是 u8 的 `types`，0x2bc 是 i16 的 `value`，AI 按 short 步长读是对的。

## 实机数据里有意义的道具

`switch/re/gdbitems.py` 导出，取要点：

- 治疗 6201–6234：6201 止血草 精+100 + 码[23 濒死]=2；6211–6213 还魂
  精百分 10/50/100 + 码[24 死亡]=2；6214–6219 各解 定/封/禁/眠/狂/乱；
  6220 镜=1、6221 避=1、6222 界=1；6225–6229 各解一种毒；6230 毒龙胆
  （0x1856）解全部五毒；6231–6234 加 速+/运+/武+防+/四项全加。
- 投掷 6101–6125：6113 蛊（0x17e1）special 1 range 3；6114 鬼遁水 special 2
  range 4；6115–6118 加 武−/防−/速−/运−；6119–6125 五灵珠（0x17e7–0x17ed）
  range 4 精 −1300…−2000，**不消耗**，扣充能。
- 仙宝 6401–6427（place 全是 2）：6401 引路蜂 special 3 range 2；6402 驱魔香
  码[27 辟邪]=1；6403 极目水 码[29 神眼]=1；6404 狂龙香 码[28 退魔]=1；6405
  紫幽粉 special 4；6406 神炼水 码[30 精进]=1；6407 搜宝鼠 special 5；6408
  龙精石 special 6（永不可用）；6409–6413 炎宿/雷觉/风源/水境/土隐 special
  7–11（学 0x132/0x137/0x12d/0x13c/0x141）；6414 银杏子 special 12 精上限
  +10；6415 舍利子 special 13 神上限 +6；6416 试炼果 special 14 气上限 +3；
  6417–6425 其它上限（防 2/武 2/运 1/速 1/五灵 1，special 0，走 type 3）；
  6426 金蛊王 special 15 升一级；6427 土灵珠（0x191b）special 3 range 2，
  **不消耗**。

移植端 `ItemSpecialType` 枚举只有 None/Poison(1)/Flee(2)/LevelIncrease(15)/
ExpIncrease…(20/21)，3–14 没有名字，代码里直接用数值。

---

## 一、战斗外（道具页）

### 1.1 点行 `FUN_0049c2c0(sel, click)`

- 页签 0（治疗）：队内任一人（`+0x26d+i`，i∈0..5）`FUN_0049ca70(item, role)`
  可用 → 1；否则弹 `DAT_00dc9f3c`（"\n      %s"，格式串 0x5b0bac）+ wa004。
- 页签 1（仙宝）：
  - 0x1908 龙精石 → 不可用。
  - 0x191b 土灵珠 / 0x1901 引路蜂 / 0x1905 紫幽粉 → 需 `DAT_00d8bbec == 2`
    （土灵珠还要 `FUN_004235b0("M22",0)` 为假，即当前城市不是 M22，字符串
    0x59062c）。
  - special 7–11 → 对**当前选中的头像** `+0x274` 判可用；不行弹
    `DAT_00dd4b3c`（"\n   %s"，0x5cc3b4）。
  - special 15 → 1。
  - 码[27..30] 任一非零 → 1。
  - 其余 range 1/2 → 队内任一人可用。
- 其它页签 → 不可用。

`DAT_00d8bbec` 在 `FUN_00422950`（场景加载）从 `DAT_00c12ba8` 拷来。经验上
`.scn` 头部 +114 那个 u32（移植 `ScnFile.global_scene_index`）在 M01/M06/
M11/M22 都是 2、Q01/Q02 都是 0，所以把它当「场景类型，2 = 迷宫」
`[推断: 写入 DAT_00c12ba8 的那一处没定位]`。

### 1.2 可用判定 `FUN_0049ca70(item, role)`

```text
0x1908 → false
0x191b → 城市≠M22 && 迷宫
0x1901 / 0x1905 → 迷宫
special 5 → true
special 7–11 → 技能 FUN_00479080(id) 的 +0x23c+role（可学者位）非零
               且 role 尚未会该技能 → true，否则 false
special 15 → true
码[27..30] 任一非零 → true
否则 need = #(impact type == 3)
          + Σ_{12 项, value>0}(max − cur)
          + Σ_{毒下标 i∈0..5, 码[i]≠0} role.毒[i]
      → need > 0
```

### 1.3 默认目标 `FUN_0049c660(sel)`

置 `+0x28e = 1`。对队内每人算 need[i] 与 potential：

```text
for attr in 0..12:
    if value[attr] > 0: need += max − cur; potential += value
    for p in 0..5: if 码[p] != 0: need += role.毒[p]; potential += 码[p]
```

**毒的贡献在 12 次循环里各加一次（×12）**，照抄。全队 potential 为 0 →
清箭头、`+0x28e = 0`、返回 −2。

目标：第一个 need>0 的人；再用 `<=` 取最大（并列取靠后）；若
need[target]==0 → 0 号；若 need[target]==need[首个正数] → 首个正数。

- range 1：箭头落在 target 当且仅当 `usable(item, target)`；`+0x28f = 0`；
  返回 target。
- range 2：**所有**队内成员亮箭头，条件仍是 `usable(item, target)`（只查
  target 一个人）；`+0x28f = 1`；返回 −1。
- 其它 range → −3。

箭头贴图 `ui/GameMainUI/Communal/NeedArrow.tga` 25×27，摆放位置
`[推断: 头像板左上]`。

### 1.4 施用 `FUN_0049cf80(role)`

守卫：`+0x28e && +0x28d`，range∈{1,2}，且（可用 或 range≠1）。

按物品 id 分支：

| id | 行为 |
|---|---|
| 0x1902 驱魔香 | `FUN_00410580(0x1b)` |
| 0x1904 狂龙香 | `(0x1c)` |
| 0x1903 极目水 | `(0x1d)` |
| 0x1906 神炼水 | `(0x1e)` |
| 0x1905 紫幽粉 | `(0x1f)` |
| 0x1901 引路蜂 | 按名字跑 SCE 过程 `_ylf_<城市>_<场景>`（格式串 0x586218；`FUN_004226f0` 取场景结构 +0 / +0x20；`FUN_004325a0` 查名；`FUN_00432170(id,0)` 执行）→ `FUN_004a5010(9)` 关菜单 |
| 0x191b 土灵珠 | 当前 sce 里有过程 0x835（2101，`FUN_00432600`）→ 跑它、关菜单；**不消耗** |
| special 5 搜宝鼠 | 场景里有「活着的 type 0x18 且参数块第二个 dword == 1」的对象 → 特效 `FUN_00424c20(DAT_00c116ec, 0xb, 0, 0.001)`，否则放音 "XMouse"（`FUN_00430570`+`FUN_00410360`）；两支都关菜单。扫描细节见 1.5 |
| special 7–11 | 不可学 → 弹 "\n   %s%s"（角色名 + `DAT_00dd4b3c`）；可学 → 插入技能，弹 `DAT_00dd733c+role*0x100` / `DAT_00dd443c` / `DAT_00dd463c`… + 技能名，`+0x28c/28e/28d = 0`，`FUN_0049f9b0(1)` |
| special 15 金蛊王 | 等级 +1（上限 99）：新基值 = 曲线(new) + (旧基值 − 曲线(old))，精气神武防五项；`FUN_0040be80(0)`；当前值钳位 `cur<max ? (cur<1→0) : max`，**不回血** |
| 0x1856 毒龙胆 | 目标在该物品码非零的毒下标上一个毒都没中 → `cur精 = 1`（配 `FUN_0043fca0/fdf0` 显示）；否则清五个毒字节（队长再 `FUN_00410470(0x1f,1,0)`，`FUN_004a6030(role)` 刷图标） |

`FUN_00410580(k)`：k==0x1f → `DAT_00c10cc1 = 1`，否则 `role0.state[k] = 1`
（`(&DAT_00c0464c)[k]`；角色 0 = 景天，结构基址 0xc045f8、跨度 0x580、状态
字节在 +0x54）。两支都 `DAT_00c10cd0 = 0`（计时器）。主循环里游戏时间累计到
60 s 时 `DAT_00c04667..6a`（角色 0 的状态 27..30）和 `DAT_00c10cc1` 一起清零
（decompiled.c ~8323 行）。

通用 range 1（单体）：

```text
for attr in 0..12 with value != 0:
    type 1 → cur += ROUND(value * 0.01 * max)     // 浮点，四舍五入
    type 3 → base(max) += value
    else   → cur += value
for p in 0..5: 码[p] >= 2 → 清 role.毒[p]
for s in 13..30: role.state[s] = (char)码[s]      // 原样拷贝
FUN_0040be80(0); 12 项钳位到 [0, max]
```

通用 range 2（全体）：对每个**可用**的队内成员做同样的事，但百分比那支是
`cur += ROUND((value / 100 整除) * max)`——和单体那支不一样，照抄。做完
只对仍可用者闪一下头像（`FUN_0049fbe0(role, color)`，纯表现）。

收尾：`+0x28c`（连用）且数量 > 1 且还有人可用 → 重跑 `FUN_0049c660`；
`FUN_0040cf60` 扣 1 个（0x191b 除外）；`FUN_0049b2e0(tab, 0)` 刷列表；非连用
→ `+0x28e = 0, +0x28d = 0`，`FUN_0049f9b0(1)`。

### 1.5 搜宝鼠扫描 `FUN_00421ea0` 与引路蜂过程名

搜宝鼠遍历场景对象表：表在 `scene+0xe0`，步长 0x380，个数在 `scene+0xdc`
（`FUN_00553627(in_ECX + 0xe0, 0x380, 0x40, …)`；按下标取记录的地方写作
`param_1 * 0x380 + 0xe0 + in_ECX`）。函数里以 `in_ECX + 0x1d4` 起步、每轮
`+= 0x380`，判据三条同时成立：

| 表达式 | 记录内偏移 | 含义 |
|---|---|---|
| `pcVar1[-0x47] != 0` | +0xad | 对象活着。加载时从 `.scn` 记录抄进来（`FUN_0041fa90`: `in_ECX[iVar3 * 0x380 + 0x18d] = *(char *)(iVar2 + 4)`），脚本 ObjectActive 也是写它；对象绘制入口 `if (*(char *)(in_ECX + 0xad) == '\0') return;` |
| `*pcVar1 == 0x18` | +0xf4 | 节点类型 24 |
| `*(int *)(pcVar1 + 8) == 1` | +0xfc | 参数块第二个 dword（`.scn` 记录 0x50，移植版 `ScnNode.ladder_nav_coord1.1`）|

移植版对应 `ScnScene::treasure_object_present()`：节点 `node_type == 0x18
&& ladder_nav_coord1.1 == 1` 且对应实体可见（可见性就是移植版的 +0xad）。

引路蜂的过程名格式串 0x586218 是 `_ylf_%s_%s`，两个参数取自场景结构 +0 /
+0x20（cpk 名 / 场景名）。把全部迷宫 cpk 的 `.sce` 抽出来核对过：每个迷宫
都有 `_YLF_mNN_<子场景>` 形式的过程（id 从 2102 起，如 `_YLF_m01_1`、
`_YLF_m08_1a`/`_YLF_m08_1b`），大小写与格式串相反是大写，而移植版
`SceProcContext::new_from_name` 按小写比较，对得上。土灵珠要的 proc 2101
名字是 `T_mNN`，**M22 没有 2101**——正好和「土灵珠要求当前城市不是 M22」
一条对上。M04/M05/Q01 两者都没有。

### 1.6 移植

- `RoleState.states: [u8; 31]`——原版角色记录 +0x54 起的那 31 个字节。
- `PersistentState.purple_powder`（`DAT_00c10cc1`）、`maze_effect_timer`
  （`DAT_00c10cd0`），在 `advance_play_time` 里累计，≥60 s 清角色 0 的
  27..30 和紫幽粉。
- `PersistentState.pearl_charges: [u8; 7]`，新游戏全 3（`DAT_00bfdb70..77`）。
- 逻辑在 `yaobow/shared/src/openpal3/states/item_use.rs`：`usable` /
  `default_target` / `apply`，输入 `UseContext { in_maze, city_is_m22,
  proc_2101_exists, treasure_present }`。
- 服务端 `item_usable` / `item_use_target` / `use_item`，`use_item` 返回
  `UseOutcome` 码；引路蜂/土灵珠要跑的过程记在 `pending_item_proc`，
  `pump_pre_update` 里交给 `SceVm`。
- `ScnScene::scene_type()`（`.scn` 头 +114）和 `treasure_object_present()`
  提供 `UseContext` 的场景侧两项，服务端在 `pump_pre_update` 里每帧缓存一份
  ——道具页是在导演的 update 里画的，那时候够不到场景。
- `item_menu.p7`：页签 0/1 点行进入「选人」态（状态值加 100000），头像板
  上画箭头（`menu_chrome.paint_head_arrows`），点头像施用；引路蜂/土灵珠/
  搜宝鼠用完直接 CLOSE；右键退出选人 `[推断: 原版取消键没定位]`。
  两处和原版有出入，都记在脚本注释里：
  - 仙宝页 special 7–11（仙术书）原版判的是**当前选中的头像**，移植没有这
    个概念，改判「队里有人能学」`[推断]`；
  - 进选人态额外要求默认目标不是 −2/−3，免得进去了谁也点不动 `[推断]`。
- 菜单音效（wa002/wa004）没有 UI 侧播放通路，其它页也没接，留空。搜宝鼠命
  中的特效和 "XMouse" 同理，先只关菜单。

---

## 二、战斗内（道具指令 `0x106`）

### 2.1 列表 `FUN_004f95f0`，筛选 `FUN_0040d010(case)`

`FUN_0040d010` 的参数是**分支号**不是掩码：

| 子页 | case | 条件 |
|---|---|---|
| 0 | 0xe | type 1 且（码[24]==2 或 impact type[0..2]==2 或 value[0..2]>0） |
| 1 | 2 | type 2 |
| 2 | 0xf | type 1 且 ∃k∈0..25, k∉{23,24}: 码[k]∈{1,2} |

按 id 升序；仙宝不出现；行可点，除非是充能为 0 的五灵珠。

### 2.2 可用 `FUN_004f7500 → FUN_004f6fb0`

持有（五灵珠看充能）。禁（k=16）挡住整个道具指令。

### 2.3 施用 `FUN_004f2750`

1. 0x1856 毒龙胆打在**没中毒**的目标上 →
   `FUN_004f2c70(target, −99,999,999, 1, −1)` 直接打死；物品照常消耗。
2. 否则 `FUN_004f1fa0` special → `FUN_004ef4a0` 状态（码 1 加 / 2 解，
   `FUN_004f3e20(target, k+1, set, 1)`；音效 wd017 定/封/禁、wd018 眠/乱/狂、
   wd019 六个都解）→ `FUN_004eef20` 属性（原始 i16，我方使用者加
   `round(v × user.f32[+0x2b4] × 0.01)`，+0x2b4 = 装备特殊 0x16 道具效果增强，
   v>0 才加）。
3. 目标迭代 `FUN_004f1a30`：range 2/4 全体，或 13 单位内，或复活物；结果 <0
   且目标活着 → 打断（`interrupt`）。
4. special 1 蛊 `FUN_004f3d10`：避毒 → 0；未中毒 → 随机一种毒
   `rand15() % 5 + 1`。
5. special 2 鬼遁水 → 强制撤离（`+0x225 = 1`，任务 0x10b），除非禁止逃跑。
   `combat_flow.md` 里把这支写成「怪物或剧情驱动」是错的，它就是物品
   special 2 走的路。

消耗在执行器 0x51514e–0x51519d：五灵珠 → `FUN_004f7140` 扣充能字节
（`DAT_00bfdb70..77`；0x17e7→db73、0x17e8→db74、0x17e9→db71、0x17ea→db72、
0x17eb→db75、0x17ec→db77、0x17ed→db76；新游戏全 3；存档 8 字节），否则
`FUN_0040cf60`。

`FUN_004f3e20` 的规则：隐形目标挡其它状态；加毒时已中毒或避毒则跳过，并清
其它毒；加 k∈14..23 清 14..23；时长按 `combat_formulas.md` §6.2；对死者解
k=0x19 = 复活（`+0x242 = 1, +0x24c = 0`，两条条清零）。

### 2.4 移植

`session.rs`：`MenuEntry::Item`（道具）、`Menu::Items { index }`、
`Action::UseItem(i)`、`Pending::Item { slot, target, item }`；候选
`ItemChoice { name, count, pearl, effect: ItemEffect }` 在
`commands/combat.rs::start` 按 0xe/2/0xf 三条并起来（背包本身就是按 id 升序
的 `BTreeMap`，天然去重）；`Battle::use_item`（`actions.rs`）顺序
毒龙胆即死 → special → 状态 → 属性；用掉的记在 `used_items`，`finish()`
写回（五灵珠扣充能，其它扣数量）。`+0x2b4` 道具效果增强的装备位移植还没有，
按 0 `[推断]`——等于原版没戴那件装备。

`FUN_004f3e20` 那套状态规则移植成 `Battle::set_state(slot, k, on)`，时长表
逐个 case 抄自 0x4f3ffc 起的 switch；`FUN_004f3cc0`（五毒免疫表）移植还没有，
按「不免疫」`[推断]`。解「死亡」= `Combatant::revive()`：在场位 1、精归零、
状态与计时器全清，随后由道具自己的属性效果把精抬起来。

移植的战斗菜单是文字占位，五项排成 攻击/仙术/道具/防御/逃跑，**这个次序没
从原版读出来** `[推断]`；合击还没有。全体道具（range 2/4）打整边，复活类
（码[24]==2）的目标表把倒下的人也算进去。
