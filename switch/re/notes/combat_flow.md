# PAL3.exe v1.4 — 战斗系统：生命周期与回合模型

来源：`re/PAL3_unpacked.exe`（StarForce 脱壳后），Ghidra 反编译 `re/out/decompiled.c`
+ capstone 反汇编（`switch/re/x86dis.py`）。标 `[推断]` 的条目未在代码里 100% 闭环，
其余均直接读自指令。

战斗系统自称 **`PAL3 COMBAT SYSTEM 1.03`**（`0x5d8824`），代码大致占
`0x4D0000–0x516000`。本篇只覆盖**生命周期、回合/行动条模型、站位**；
伤害公式与状态效果（`Combat\CBRoleState.cpp`，`FUN_004ee540` 一带）不在本篇。

数据库字段名沿用 `gdb_database.md`。

## 0. 对象图

主循环 `FUN_004055d0` 按全局游戏状态 `DAT_00bfda6c` 分派，**状态 3 = 战斗**，
每帧调 `FUN_00513060(dt)` = `CBSystem::Update`。

战斗系统单例在 `DAT_00bfda74`（`operator_new(0x162d8)`，构造 `FUN_00512500`，
初始化 `FUN_005125e0`）。子对象指针数组在其起始处：

| 槽 | 大小 | 构造 / 初始化 | 用途 |
|---|---|---|---|
| `[0]` | 1 | `FUN_0051d5c0` / `FUN_0051d5f0` | 读 combat.ini 的配置对象 |
| `[1]` | 0xa48 | `FUN_005160a0` / `FUN_005181e0` | CBUI（状态条、结算面板） |
| `[2]` | — | — / `FUN_004dc220` | — |
| `[3]` | 0x1c8 | `FUN_004db490` / `FUN_004db4f0` | **行动条（ATB）**，见第 3 节 |
| `[4]` | 0x50a4 | `FUN_004ec680` / `FUN_004ec830` | **CBRoleState 数组**，见下 |
| `[5]` | — | — / `FUN_004fbf90` | CBStage |
| `[6]` | — | — / `FUN_004d70c0` | — |
| `[7]` | — | — / `FUN_004e0430` | 仅调试模式（`+0x85`）建立 |
| `[0xb]` `[0xc]` | — | `FUN_004cdb60` | 特效管理器 ×2 |

`CBSystem` 自身的关键字段：

| 偏移 | 含义 |
|---|---|
| `+0x10` | = `[4]`，CBRoleState 数组基址 |
| `+0x14` | CBStage |
| `+0x40` | 本帧缩放后 dt（double）= `dt * +0x6c` |
| `+0x48` / `+0x50` | 累计战斗时间（double / float）—— **行动条用的就是 `+0x50`** |
| `+0x58` | 本帧原始 dt（double） |
| `+0x60` / `+0x68` | 累计真实时间（double / float），调试串 `TimeFund=%f` |
| `+0x6c` | 时间缩放，调试串 `Speed%1.2f` |
| `+0x70` | **战斗描述符指针**（= `&DAT_00c11700`） |
| `+0x74` | 战斗结束标志（0 = 进行中）；主循环靠它退出 |
| `+0x78` | 战斗阶段：2 = 进行中，7 = 胜利演出 |
| `+0x7c` | 结果码，初值 1；主循环取 `== 2` 写入 `DAT_005833d0` |
| `+0x80` | 本局战斗计数，调试串 `Times of Combat=%d` |
| `+0x85` | 调试/测试战斗标志 |
| `+0x8c` | 任务数 |
| `+0x98 + n*0x58` | **任务槽 ×0x400**，见第 2 节 |

`CBRoleState` 数组：**每条 `0x37c` 字节、共 11 条**（5 我方 + 6 敌方；
`FUN_004db7a0` 的循环上界 `0x2654 = 11*0x37c` 是硬证据）。
只遍历我方时上界是 `0x116c = 5*0x37c`。已确认的条内偏移：

| 偏移 | 含义 |
|---|---|
| `+0x241` | 参战标志（该槽有人） |
| `+0x242` | 存活标志 |
| `+0x243` | 已行动完毕 / 退场标志（`FUN_004ed040`） |
| `+0x24c` | 精 当前 |
| `+0x250` | 精 上限 |
| `+0x254` | 气 当前 |
| `+0x25c` | 神 当前 |
| `+0x48d` | 濒死标志（精 < 上限/10 且 > 0） |
| `+0x48e` | 死亡标志（精 < 1） |
| `+0x590` | 该角色已行动次数 |

数组对象自身的字段（不带 `k*0x37c`）：

| 偏移 | 含义 |
|---|---|
| `+0x200` | 上次战斗对白时间戳 |
| `+0x208` | 回指 CBSystem |
| `+0x218` | 我方偷袭敌方 |
| `+0x219` | 敌方偷袭我方 |
| `+0x21c` | **最大回合数** |
| `+0x220` | **当前回合数** |
| `+0x224` | **战败不 GameOver** |
| `+0x225` | — |

## 1. 一场战斗是怎么开始的

唯一的真入口是 **`FUN_00407320(monsterIds, sneakMode, param3, muteBgm)`**：

```c
// 1) 取主角脚下的导航格地形类型
navTile = FUN_00420f30(role->navLayer);           // role+0x274
ix = (int)role.x, iz = (int)role.z;               // __ftol
terrain = (0<=ix<nav.w && 0<=iz<nav.h)
        ? navTile.cells[(nav.w*iz + ix)].byte5    // 每格 8 字节，取 +5
        : 2;                                      // 越界默认 2

// 2) 6 个怪物 ID 拷进描述符
for (i = 0; i < 6; i++) DAT_00c11700[i] = monsterIds[i];

// 3) 选战斗场景（combatScn.txt，gbengine 脚本表格式，不是 INI）
section = sprintf("%s_%s", sceneName, subSceneName);   // 例 "m01_1"
key     = sprintf("%d", terrain);                      // 例 "7"
scene   = combatScn[section][key];
if (!scene) scene = combatScn[section]["default"];      // 同一节内的 default 键
DAT_00c11720 = scene;                                   // 32 字节

// 4) 场景的五灵属性
DAT_00c11740 = atoi(combatScn["fivemagic"][scene]);     // 读不到则 1

// 5) 偷袭模式
DAT_00c11744 = (sneakMode == 1) ? 2 : (sneakMode == 2) ? 1 : 0;
DAT_00c11754 = param3;
DAT_00c11751 = muteBgm;

// 6) 进战斗
if (CBSystem::Prepare(&DAT_00c11700, /*debug=*/0))
    SwitchMode(6);        // FUN_0044f610(6) → DAT_00bfda6c = 3
```

`combatScn.txt` 实测格式（`BEGIN,<节> … END`，行内 `键<TAB>$值&`）：

```text
BEGIN,m01_1
default	$z08&
2	$z01&
7	$z08&
9	$z10&
10	$z04a&
END

BEGIN,fivemagic     ;1~5 分别对应 水火风雷土
z01	$5&     //土
z04	$1&     //泥
z08	$3&     //草
...
```

战斗场景美术在 `basedata\cbscene\<scene>\<scene>.pol/.cvd/.lgt`
（`CBStageScene.cpp` 的 `cbscene\%s\%s.pol` 等格式串）。

### 战斗描述符 `DAT_00c11700`

| 偏移 | 全局 | 含义 |
|---|---|---|
| `+0x00..0x14` | `DAT_00c11700` | 6 个怪物 ID |
| `+0x1c` | `DAT_00c1171c` | **最大回合数**（野外遭遇写 `0x7fffffff` = 不限） |
| `+0x20` | `DAT_00c11720` | 战斗场景名（32 字节） |
| `+0x40` | `DAT_00c11740` | 场景五灵属性 1..5 |
| `+0x44` | `DAT_00c11744` | 偷袭模式 0/1/2 |
| `+0x48` | `DAT_00c11748` | **战败不 GameOver** |
| `+0x50` | `DAT_00c11750` | 禁止偷袭判定 `[推断：也可能是"必败"]` |
| `+0x51` | `DAT_00c11751` | 不切战斗 BGM |
| `+0x54` | `DAT_00c11754` | 第二个怪物对象指针（被一并消灭） |

对应 SCE 指令 `[推断]`：`CombatMaxRound`(82) → `+0x1c`；
`CombatNotGameOver`(131) → `+0x48`；`CombatMustFail`(83) → `+0x50`。

### 调用方

| 调用者 | 场合 |
|---|---|
| `FUN_00417420` | **野外遭遇**：场景怪物的 AI 每帧检测，撞上就开战 |
| `FUN_00532f60` | 场景特效/投射物命中怪物 |
| `FUN_00406980` | 调试战斗（写死怪物 `0x1c88`/`0x1c89`，`Prepare(…, 1)`） |

两个真实调用方都走 `FUN_00407560(monsterObj + 0xac, sneakMode)` ——
怪物对象 `+0xac` 就是那 6 个怪物 ID 的数组。

野外遭遇的触发条件（`FUN_00417420` 末尾）：

```c
if (!player || player.state == 4) return;
if (DAT_00bfda6c != 2) return;             // 必须在冒险状态
if (|monster.y - player.y| >= 24.0) return;
d = |monster.pos - player.pos|;
if (d >= player.radius + monster.radius) return;
// 偷袭判定
sneak = 0;
if (d >= 12.0) {
    a = FUN_0041ba10(monster.pos);   // 朝向/背向测试
    b = FUN_0041ba10(player.pos);
    if (a == 0 && b != 0) sneak = 2;
    else if (a != 0 && b == 0) sneak = 1;
}
```

偷袭最终生效还要过一次随机（`FUN_004edc60`）：

```c
roleStates.weSneak = roleStates.enemySneak = 0;
if (desc->noSneak == 0) {                       // desc+0x50
    if (desc->sneakMode == 1 && rand10bit() < 0x200)  roleStates.enemySneak = 1; // +0x219
    if (desc->sneakMode == 2 && rand10bit() < 0x200)  roleStates.weSneak    = 1; // +0x218
}
// rand10bit(): seed = seed*0x343fd + 0x269ec3; (seed >> 16) & 0x3ff
```

即**偷袭只有 50% 概率真正成立**。`[推断]` `+0x218`/`+0x219` 与
`CBStage.cpp` 里 `WeSneakAttackEnemy` / `EnemySneakAttackUs`（`FUN_004fd290`）
一一对应，但两者哪个对哪个没有逐条闭环。

### `CBSystem::Prepare`（`FUN_00512f40(desc, debugFlag)`）

```c
this->debug      = debugFlag;   // +0x85
this->finished   = 0;           // +0x74
this->phase      = 2;           // +0x78
this->result     = 1;           // +0x7c
this->combatCount++;            // +0x80
DAT_00c10cd4++;                 // 全局战斗次数
this->desc       = desc;        // +0x70
ok = CBSystem::Init();          // FUN_00512d60

// 装备特效：队里谁戴着物品 0x1fb2，10.15% 概率开局附加战斗状态 0x17
for (k = 0; k < 5; k++)
    if (roleStates[k].active && (equip[k] == 0x1fb2))
        if (rand10bit() < 0x66) FUN_004f3e20(k, 0x17, 1, 1);
```

`CBSystem::Init`（`FUN_00512d60`）依次初始化上表的子对象，然后：

- 清每个我方成员 CBRoleState 的 `+0x36c` / `+0x36d`
- `FUN_00515b20(&LAB_00513eb0, 0,0,0,0)` —— **挂上开场任务**
- 按 `roleStates+0x219`（敌方偷袭）选镜头延时：`FUN_00505770(0x14)` 否则 `FUN_00505770(0)`

## 2. 任务调度器

战斗不是一个大 switch，而是**任务表驱动**。`CBSystem+0x98` 起 **0x400 个槽**，
每槽 `0x58` 字节：

| 槽内偏移 | 含义 |
|---|---|
| `+0x00` | 函数指针 |
| `+0x04/08/0c` | 三个参数 |
| `+0x10` | 任务 id（`FUN_005136a0` 递增） |
| `+0x14` | `0xffffffff` |
| `+0x1c` | 0 |
| `+0x24` | **激活标志** |
| `+0x25` | 0 |
| `+0x28` | 11 dword 用户数据 |

`FUN_005137f0` 每帧扫全部 0x400 槽，激活的就调用，调用期间
`CBSystem+0x90` = 当前槽号（`Thread=%d, StageTask=%d` 调试串）。
注册用 `FUN_00515b20(fn, p1, p2, p3, userData)`；`LAB_00513840` 被特判**去重**，
同一时刻只允许一个实例。

两个关键任务：

- **`LAB_00513eb0`** —— 开场：按偷袭标志排镜头序列
  （`0x4fc700` 建镜头任务、`0x4fc7d0` 压关键帧、`0x4fc940` 提交），
  最后注册 `LAB_00513840`。
- **`LAB_00513840`** —— **主调度循环**，见下一节。

## 3. 行动条（ATB）模型 —— 本篇的核心

战斗**不是回合制**，是**双阶段行动条**。行动条对象 = CBSystem `[3]`（`0x1c8` 字节），
每个战斗单位一条 **`0x28` 字节**的记录，共 11 条：

| 记录内偏移 | 类型 | 含义 |
|---|---|---|
| `+0x0c` | float | 本帧 dt（战斗时间轴） |
| `+0x10` | float | 上次时间戳 |
| `+0x20` | int | **行动条 1（蓄力/等待）** |
| `+0x24` | int | **行动条 2（出手/吟唱）** |
| `+0x28` | int | 已获得的行动次数 |
| `+0x2c` | u8 | **速度**（驱动行动条 1） |
| `+0x2d` | u8 | **出手速度**（驱动行动条 2） |
| `+0x2e` | u8 | 阶段：0 = 蓄力，1 = 出手 |
| `+0x2f` | u8 | 冻结（dt 记 0） |
| `+0x30` | u8 | 启用 |

**满值常量 `0x2cec = 11500`**，判定阈值 `0x2ceb = 11499`。
速率常量在 `0x56ab58` = **`50.0f`**。

### 每帧推进（`FUN_004db5c0` → `FUN_004dba60`）

```c
// FUN_004db5c0：算 dt（战斗时间轴，CBSystem+0x50）
for (k = 0; k < 11; k++) {
    rec[k].dt   = rec[k].frozen ? 0.0f : (cbSystem.combatTime - rec[k].lastStamp);
    rec[k].lastStamp = cbSystem.combatTime;
}

// FUN_004dba60：推进
for (k = 0; k < 11; k++) {
    if (!rec[k].enabled) continue;          // +0x30
    if (!IsAliveInCombat(k)) continue;      // FUN_004ed010
    if (rec[k].phase == 0) {                            // 蓄力
        int old = rec[k].gauge1;
        rec[k].gauge1 = (int)(rec[k].dt * 50.0f * rec[k].speed + old);
        if (rec[k].gauge1 >= 11500) {
            if (old < 11500) rec[k].turns++;            // +0x28
            rec[k].gauge1 = 11500;
        }
    } else {                                            // 出手
        rec[k].gauge2 = (int)(rec[k].dt * 50.0f * rec[k].castSpeed + rec[k].gauge2);
        if (rec[k].gauge2 > 11500) rec[k].gauge2 = 11500;
    }
}
```

即 **`条 += dt × 50 × 速度`，上限 11500**。
从 0 走满需要 `11500 / (50 × 速度) = 230 / 速度` 秒（战斗时间轴，
受 `CBSystem+0x6c` 时间缩放影响）。

### 查询 API

| 函数 | 语义 |
|---|---|
| `FUN_004db690(k)` | `rec[k].enabled`（`+0x30`） |
| `FUN_004db6a0(k)` | **可以行动**：`gauge1 > 11499` |
| `FUN_004db6c0(k)` | **出手完成**：`gauge2 > 11499` |
| `FUN_004db6e0(k)` | 处于蓄力阶段：`phase == 0` |
| `FUN_004db700(k)` | `rec[k].turns`（`+0x28`） |
| `FUN_004db710(k)` | 复位：`gauge1 = gauge2 = 0, phase = 0`（一次行动结束） |
| `FUN_004db730(k)` | 开始出手：`gauge1 = 0, phase = 1` |
| `FUN_004db750(k,g1,g2,ph)` | 直接设定三者 |
| `FUN_004db780(k,v)` | 设出手速度 `+0x2d` |

### 谁下一个出手（`FUN_004db7a0`）

```c
best = -1; bestEta = 1e8;
for (k = 0; k < 11; k++) {
    if (!roleStates[k].active) continue;                    // +0x241
    if (FUN_004ed040(k)) continue;                          // +0x243 已退场
    if (roleStates[k].+0x598 != 0) continue;
    if (FUN_004ee870(k)->+0x36f != 0) continue;
    if (FUN_004f2bb0(k, 0) <= 0) continue;
    float eta = (rec[k].phase == 0)
        ? (11500 - rec[k].gauge1) / (rec[k].speed     * 50.0f) + 10000.0f
        : (11500 - rec[k].gauge2) / (rec[k].castSpeed * 50.0f);
    if (eta < bestEta) { bestEta = eta; best = k; }
}
return best;
```

`+10000.0` 这一项让**所有正在出手（phase 1）的单位一律排在还在蓄力的单位前面**。

### 回合计数

回合不是行动条的产物，而是**由 10 号槽（最后一个敌方位）的行动次数驱动**
（`FUN_004ed7c0` 一带）：

```c
if (roleStates.maxRound <= roleStates.round) return 1;      // 已到上限
if (roleStates[k].actedTimes < atb.turns(k)) {              // +0x590 vs +0x28
    roleStates[k].actedTimes++;
    if (k == 10) roleStates.round++;                        // +0x220
    FUN_004f48c0(k);                                        // 该单位开始行动
    ...
}
```

## 4. 主调度任务 `LAB_00513840`

每帧执行，骨架（反汇编 0x513840 起）：

```c
if (debug && DAT_017a990c != 2) return;         // 调试暂停
if (FUN_004ed100(0)) return;                    // 我方全灭 → 交给结算
if (cbSystem.phase != 2) return;                // 只在"进行中"跑
if (ui->+0x24) { ...菜单打开分支... }
for (k = 0; k < 11; k++) {
    if (!roleStates[k].active) continue;
    if (!stageRole[k].ready())        continue;   // FUN_0050a6f0
    if (FUN_004ecb20(k))              continue;
    if (FUN_004db690(k) != 1) continue;           // ATB 启用
    if (FUN_004ed010(k) != 1) continue;           // 存活参战
    if (FUN_004ed040(k) != 1) continue;
    if (FUN_004db6a0(k) != 1) continue;           // **行动条满**
    if (HasState(k, 0x11) || HasState(k, 0x0e)) → 分支 A；   // 乱 / 封
    if (roleStates[k]->+0x345) → 分支 B;
    if (HasState(k, 0x12) || HasState(k, 0x13)) → 分支 B;   // 狂 / 镜
    FUN_004f7fc0(k);                              // 战斗对白（barks）
    ...
}
```

`FUN_004f3d90(k, state)` = 查询战斗状态位；状态编号用
`gdb_database.md` 的 `ActorCombatStateType`（0x0e = 14 封，0x11 = 17 乱，
0x12 = 18 狂，0x13 = 19 镜）。

### 战斗对白（`FUN_004f7fc0`，读 combat.ini）

节名 `[<角色英文名>-CombatDialog]`，触发条件与键名：

| 条件 | 键 |
|---|---|
| 自己 精 < 上限/10 | `Dying<0-9>` |
| 队友 精 < 上限×3/10 | `<角色>HPLow<0-9>` |
| 中毒 | `Poison<0-9>` |
| 睡眠/昏迷 | `<角色>Sleep<0-9>` |
| 气满 | `GPFull<0-9>` |

冷却：`cbSystem.combatTime - roleStates.lastBarkTime >= 10.0` 秒。
随机后缀 `rand15bit() % 10`。

## 5. 胜负与逃跑（`FUN_004f7910`，每帧）

```c
// 胜利
if (FUN_004ed0a0(0) == 1 && cbSystem.phase == 2 && !FUN_004fca00(0xc)) {
    cbSystem.phase = 7;                 // 胜利演出
    排镜头任务 0x19（LAB_00502300 / FUN_00501f30 / LAB_00502000 / LAB_005021c0）
    FUN_00505770(10);
    清每个参战单位的 stageRole +0x118 / +0x112
}

// 失败 / 回合耗尽
if ((FUN_004ed100(0) == 1                                   // 我方全灭
     || (roleStates.maxRound <= roleStates.round            // 回合到顶
         && roleStates.notGameOver == 0))
    && cbSystem.phase == 2 && !FUN_004fca00(0xc)) {
    FUN_004f8af0();
    FUN_004fa930(1, -1);
    FUN_00505770(3);
    每个参战单位排任务 0x18（LAB_004fe550 …）
}
```

主循环 `FUN_004055d0` case 3 在 `cbSystem.finished (+0x74)` 置位后收尾，
把 `cbSystem.result (+0x7c) == 2` 写进 `DAT_005833d0`
`[推断：2 = 战败，供剧情脚本查询]`。

**逃跑**：`FUN_005024d0` 是逃跑结果的**提示任务**，读
`stage[...+0x39] == 1` 决定显示 `FleeSuccess` 还是 `FleeFail`
（两个串都在 `CBStage.cpp` 区，`0x5db758` / `0x5db764`），
然后把该单位的 stage 状态置 4（退场）。**掷骰子的地方没找到**，
数据库里每只怪有 `escape_rate`（`CombatActorInfo` +0x2d4），
公式未闭环 —— 见"未解决"。

结算画面美术在 `ui\combat\result\`：`bigwin.tga` / `biglose.tga`
（`FUN_00516940`）、`panel\levelup.tga`、`panel\getitem.tga`、
`panel\<五灵>lvup.tga`（`FUN_00517df0`）、`face\HP/GP/MP.tga`（`FUN_0051c330`）。

## 6. 站位

`FUN_00511790` 读 `combat.ini`：

```c
for (i = 0; i <= 10; i++) {
    x = combat.ini["RolePos"]["Role<i>-x"];
    z = combat.ini["RolePos"]["Role<i>-z"];
    slot = i + 1;                       // 1..11
    dst = (slot < 6) ? (this + 0x5c + slot*0xc)      // 我方 5 个：+0x68..+0x98
                     : (this + 0xc8 + slot*0xc);     // 敌方 6 个：+0x110..+0x14c
    dst = (x, 0.0f, z);
    if (slot == 6) {                    // 顺带读五灵法阵中心
        this->+0xa4 = FiveLineup.x;  this->+0xa8 = 0;  this->+0xac = FiveLineup.z;
    }
}
// 最后把我方第 3、第 4 个位置对调
swap(this->+0x80..+0x88, this->+0x8c..+0x94);
```

即 **`Role0..Role4` 是我方五个站位、`Role5..Role10` 是敌方六个站位**，
且**我方 Role2 与 Role3 的坐标在读完后被交换**。

`combat.ini` 实测值（y 恒为 0）：

| 槽 | x | z |
|---|---|---|
| Role0 | -118.694534 | -209.226364 |
| Role1 | -75.145691 | -119.692291 |
| Role2 | -148.763443 | -46.891785 |
| Role3 | -244.564240 | -96.868942 |
| Role4 | -221.448181 | -194.068451 |
| Role5 | 207.959 | 78.8841 |
| Role6 | 181.707 | 235.674 |
| Role7 | 23.7562 | 257.011 |
| Role8 | 69.5522 | 4.50122 |
| Role9 | -40.0579 | 118.229 |
| Role10 | 98.150909 | 151.262329 |

`[FiveLineup]` = (-162.497406, 0, -130.144135)，radius 207，roll -0.500003；
`[EnemyFiveLineup]` = (87.4095, 0, 140.431)，radius 207，roll 0.156633。

## 7. 其他已确认的小事

- 全局随机数是 LCG：`seed = seed*0x343fd + 0x269ec3`，取 `(seed >> 16) & mask`；
  `DAT_005dfdec` 是种子，`DAT_017ad310` 是调用计数。
- `combatBE.ini`（Combat Balance Editor）只覆盖 7 件道具的 HP/GP/MP 数值，
  用来在不改数据库的前提下调平衡：金创药 500、止血草 50、首阳参 600、
  《降妖谱》123、无影神针 −600，流风散/两仪仙丹为 0。读取者 `FUN_004e5500`。
- 每帧钳位（`FUN_004f7880`，11 个单位）：精/气/神 负数归零；
  精 < 1 → 清存活位、置死亡位；精 < 上限/10 且 > 0 → 置濒死位。

## 8. 未解决

- **`+0x2c`（速度）与 `+0x2d`（出手速度）的写入点**没找到，因此
  "数据库的 速度 属性怎么换算成行动条速度"还差最后一步。按
  `230 / 速度` 秒填满、而景天 1 级速度只有 3 来看，写进去的显然不是原始属性值。
- 逃跑成功率公式（`escape_rate` 怎么用）。
- `CombatMustFail`(83) 到底写描述符的哪一位（`+0x50` 是推断）。
- 我方成员的**阵法阵位（0..4）如何映射到 Role0..Role4 这五个站位**；
  `FUN_00511790` 只负责读表，映射在 CBStageRole 的建立路径里。
- `cbSystem.result (+0x7c)` 各个取值的含义（只确认主循环比较 `== 2`）。
- `FUN_004ed0a0` / `FUN_004ed100` 的确切语义（按上下文分别是"敌方全灭"
  与"我方全灭"，未逐条闭环）。
- 主调度任务 `LAB_00513840` 里分支 A/B（乱、封、狂、镜）之后的具体行为。
- 合击技的触发点完全没碰到。
- `flexcam.dat` / `flexeff.dat` / `efflinker.dat` 三个二进制表未解析。
