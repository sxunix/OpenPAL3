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
| `+0x243` | 用我方那套动作名（`FUN_004ed040`）。原先记成「已行动完毕 / 退场」是推断，已撤回：舞台角色初始化 0x509843 按它选 `z04`（非零）还是 `z1`（零）；敌人上场 `FUN_004f8bb0` 恒写 0。见 `combat_motion.md` |
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
| `+0x48` | `DAT_00c11748` | **必败**（`CombatMustFail`）：回合到顶也不算赢，见第 5 节 |
| `+0x50` | `DAT_00c11750` | **禁止逃跑**（所有脚本战斗都置 1）；同时关掉偷袭判定 |
| `+0x51` | `DAT_00c11751` | 不切战斗 BGM |
| `+0x54` | `DAT_00c11754` | 第二个怪物对象指针（被一并消灭） |

对应的 SCE 指令处理函数（已逐条读过，不再是推断）：

| 指令 | 处理函数 | 做的事 |
|---|---|---|
| 73 `Combat(count, m1, m2, m3)` | `0x437280` | `FUN_018c59e0` 抽怪填 `+0x00..`（见下）；`DAT_00c11750 = 1`；`FUN_00407320(&desc, 0, 0, muteBgm=0)`；挂 type-6 等待器直到战斗结束 |
| 80 `CombatBoss(m1..m6)` | `0x4373d0` | 六个 ID 原样写入；`DAT_00c11750 = 1`；`muteBgm = 1`（保留当前 BGM） |
| 82 `CombatMaxRound(n)` | — | `DAT_00c1171c = n` |
| 83 `CombatMustFail` | — | `DAT_00c11748 = 1` |
| 131 `CombatNotGameOver` | `0x438440` | `DAT_005833cc = 0`（全程唯一写 0 的地方） |
| 132 `GetCombat(var)` | — | `var = DAT_005833d0` |

三个一次性标志在战斗收尾时复位（`FUN_004055d0` case 3，0x405b1f–0x405b31）：
`+0x48 = 0`、`+0x50 = 0`、`+0x1c = 0x7fffffff`、`DAT_005833cc = 1`。

**抽怪 `FUN_018c59e0(count, m1, m2, m3, boss, out)`**：把 `m1..m3` 中非零的
压紧成池子（保持顺序），然后 `count` 个格子**各自独立**抽 `pool[rand() % n]`
——同一只怪可以占满所有格，也不保证每种都出现（Pal3.Unity 的"每种一只再洗牌"
不是二进制的做法）。`boss != 0` 时 `count += 1`，boss 占 0 号格，抽签从 1 号格起。
脚本 `Combat` 恒传 `boss = 0`。`rand()` 是 CRT 的 `FUN_005541f8`。
原版写 `out[6] = count` 且不做上限；移植里把 count 夹到 6。

**战斗曲**：`FUN_00407320` 在 `Prepare` 成功后、`!muteBgm` 时调
`FUN_0053e680(scene)`（0x407320 尾部），曲名来自 `datascript\\music.txt` 的
`BEGIN,combat` 节按城市名查表（`music\\%s.mp3`）；`CombatBoss` 传 `muteBgm = 1`
所以 boss 战沿用场景 BGM。地砖类型那一步的坐标转换是 `__ftol`（截断，不是四舍五入）。

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
| `FUN_004db730(k)` | **开始出手**：`gauge1 = 0, phase = 1`。调用点在行动任务 `FUN_00514870`（0x514933）和逃跑任务（0x515281） |
| `FUN_004db750(k,g1,g2,ph)` | 直接设定三者 |
| `FUN_004db780(k,v)` | 设出手速度 `+0x2d` |

### 谁下一个出手（`FUN_004db7a0`）

```c
best = -1; bestEta = 1e8;
for (k = 0; k < 11; k++) {
    if (!roleStates[k].active) continue;                    // +0x241
    if (FUN_004ed040(k)) continue;                          // +0x243（我方那套动作；此处用途待查）
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

### 4.1 分支 A / B 之后做什么

**分支 A（乱 `0x11` / 封 `0x0e`）→ 0x513ab8：这一回合什么都不做。**

```text
0x513925  if (FUN_004f3d90(k, 0x11)) goto 0x513ab8;   // 乱
0x513938  if (FUN_004f3d90(k, 0x0e)) goto 0x513ab8;   // 封
...
0x513ab8  FUN_004db750(k, 0, 0, 0);                   // 两条行动条清零、回蓄力段
0x513ac7  （落到循环递增：edi += 0x37c, esi++）
```

没有出手，**也不是交给 AI**。这就是笔记别处提到的那处 `FUN_004db750(k,0,0,0)`
（0x513ac2）。

**分支 B（`+0x345` / 狂 `0x12` / 镜 `0x13`）→ 0x513a36：交给 AI 代打。**

```text
0x513a36  FUN_004f7fc0(k);      // 战斗对白
0x513a3a  FUN_004d86d0(this, k) // ← 自动出手
```

`FUN_004d86d0(k)` 先过三道门（`+0x241` 在场、`FUN_004ed010` 存活参战、
`FUN_004ee870(k)->+0x24` 非零），把舞台角色的 `+0x19ec` 复制到 `+0x19e8`，
然后若 `+0x56c` 非零就调 `FUN_004d8760`，失败再退到 `FUN_004d8af0`。
`FUN_004d8760` 遍历战斗位挑目标，候选判定是
`FUN_004dab70` / `FUN_004daa40` / `FUN_004daca0` 三个。

> **移植对照**：先前 `CombatSession` 把「乱」当成 AI 代打
> （`side == Party && !confused` 才开菜单，否则 `act_by_ai(slot, confused)`），
> 和原版不符——原版是**空过一回合**。已改成空过，并加了回归测试。
> 封（0x0e）、狂（0x12）、镜（0x13）和 `+0x345` 这四个条件移植都还没建模，
> 所以 AI 代打那一支目前只有敌方在走。

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

先查我方全灭，再查敌方全灭 / 回合到顶 —— 顺序是这样，之前这里写反了：

```c
// 0x4f7940：败
if (FUN_004ed0a0(0) == 1 && cbSystem.phase == 2 && !FUN_004fca00(0xc)) {   // 我方全灭
    cbSystem.phase = 7;                 // 失败演出 → result = 3
    排镜头任务 0x19（LAB_00502300 / FUN_00501f30 / LAB_00502000 / LAB_005021c0）
    FUN_00505770(10);
    清每个参战单位的 stageRole +0x118 / +0x112
}

// 0x4f7a6a：胜
if ((FUN_004ed100(0) == 1                                   // 敌方全灭
     || (desc.maxRound (+0x1c) <= roleStates.round (+0x220)
         && desc.mustFail (+0x48) == 0))                    // 回合到顶且未标"必败"
    && cbSystem.phase == 2 && !FUN_004fca00(0xc)) {
    FUN_004f8af0();
    FUN_004fa930(1, -1);                // 结算：经验 / 钱 / 掉落
    FUN_00505770(3);
    每个参战单位排任务 0x18 → phase 5 → result = 2
}
// phase 4（逃跑任务 LAB_00515200 在 0x51571c 写入）也走同一结算 → phase 5 → result = 2
```

`cbSystem.result (+0x7c)` 的取值：**2 = 胜（含回合到顶、含逃跑成功）**，
**3 = 败**。4 / phase 6 在二进制里没有写入者，当作不可达。

主循环 `FUN_004055d0` case 3（0x405ae1）收尾：

```c
DAT_005833d0 = (cbSystem.result == 2);            // GetCombat 读的就是它
if (!DAT_005833d0 && DAT_005833cc != 0) {          // 败 且 未 CombatNotGameOver
    FUN_0044f610(0xf);                            // SwitchMode → 标题
    FUN_00432550();                               // 停掉所有脚本
}
desc.+0x48 = 0; desc.+0x50 = 0; desc.+0x1c = 0x7fffffff; DAT_005833cc = 1;
```

所以脚本眼里：`GetCombat` 得 1 = 打赢、回合耗尽、**逃跑成功**三者之一；
得 0 = 全灭。逃跑成功不触发 game over，且已击杀的敌人照常结算。

**逃跑**：玩家菜单的逃跑是掷骰子的（见 9.3，之前那节只读到了 AI 路径）。
`FUN_005024d0` 是逃跑结果的**提示任务**，读 `stage[...+0x39] == 1` 决定显示
`FleeSuccess` 还是 `FleeFail`（`0x5db758` / `0x5db764`），然后把该单位的
stage 状态置 4（退场）。

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

> 第 9 节已补完其中四项：行动条速率换算、行动代价、逃跑、合击技。
> 以下是**仍然开着**的。

- ~~移植里战败 game over 的去处~~ —— 通路已经有了：`SceState::request_title`
  → `AdventureDirector::take_title_request` → `Pal3Service::take_title_request`
  → `YaobowApplicationLoader::on_updating` 装标题 director，和 147 Switch2Menu
  以及 102 `SwitchMode(15)` 走同一条。**仍然不同的是演出**：原版战败走
  phase 7 的失败演出，移植只画一个"游戏结束"占位覆盖层。
- ~~战后倒下的队员回到多少精~~ —— **回到 1 精**，`FUN_004ed870`，见第 9.8 节
  （该节先前写「没有这一步」是错的，已在节内更正）。
- ~~我方的 `+0x470`（五灵阵位）找不到写入者~~ —— 是 `FUN_004ed250` 从冒险侧
  记录的 `+0x29a` 搬的，见第 9.7 节的后续更正。
- ~~`FUN_004ed0a0` / `FUN_004ed100` 的确切语义~~ —— 见第 9.8 节。
- ~~主调度任务 `LAB_00513840` 里分支 A/B 之后的具体行为~~ —— 见第 4.1 节。
- `flexeff.data` 的两张表、15 个特效类的注册表和类 CRC 映射都已解，见
  [`flexeff_format.md`](flexeff_format.md)；还差槽位子块那 124 字节里各类
  自己的字段语义（要逐个类跟 update）。
- `flexcam.dat` / `efflinker.dat` 两个二进制表未解析。

## 战斗舞台（移植侧）

规则层一直是完整的，缺的只是表现。第一步把战场和人摆出来了：

- 战场模型在 `basedata/cbScene/<大写>/<小写>.pol`（`cbScene/Z01/z01.pol`），
  同目录有 `.lgt` 和 `.DKM`。包里 44 张，`combatScn.txt` 会派到其中 **41
  张，模型和 `.lgt` 全部在包里**（`pal3_combat_stage_census` 核过）。
- 站位是 `combat.ini` 的 `[RolePos]` 十一行，**我方 0..4、敌方 5..10**。
  两军分居 **z** 的两侧（我方 z 全负、敌方 z 全正），x 上并不分开——敌方
  的 `Role9`（x = −40）比我方的 `Role1`（x = −75）还靠里：

  ```text
  我方 (-118,-209) (-75,-119) (-148,-46) (-244,-96) (-221,-194)
  敌方 (207,78) (181,235) (23,257) (69,4) (-40,118) (98,151)
  ```

- 模型名：**我方五个从 `PARTY_MODELS`（101/104/105/107/102）来，不是从
  数据库**——gdb 里主角那五条 `combat_actors` 的 `model_id` 写的是
  `"Null"`，他们直接用冒险模式的模型。敌方才用 `model_id`（`B01` 这种，
  对应 `basedata/ROLE/B01`）。167 个战斗角色都带 `model_id`。

`SceCommandCombat::initialize` 里建一个 `CoreScene`、放战场和活着的参战者、
压到场景栈；散场时弹掉。缺哪一件就少摆哪一个，规则层不碰这些实体。

### 已在实机上验证过

2026-09-09 在桌面版上跑通并截图确认：z08 战场铺满画面，右下角景天、左上角
霹雳堂弟子甲，两军按 `[RolePos]` 分居 z 的两侧，相机把两边都收进画面。

验证的路子（以后回归照这个走）：

1. 桌面版要带 `DYLD_*` 才找得到 `libvulkan.dylib`，而 macOS SIP 会在执行
   受保护二进制时把这些变量剥掉——**用 homebrew 的 python `os.execve`
   直接起进程**才带得住（`/opt/homebrew/bin/python3`，系统 `/usr/bin/env`
   不行）。
2. `--agent-port <n>` 起 agent server，`/v1/menu/new_game` +
   `/v1/time/fast_forward` 跳过开场，`/v1/script/eval` 的 `load_scene`
   跳到有战斗映射的场景（`combatScn.txt` 里 **q01 只有 `q01_x`**，
   `q01/yn09a` 查不到战场，舞台不会建；`M01/1` 的 default 是 `z08`）。
3. `start_combat` 直接开战（新加的调试 poke，见下），随后立刻
   `/v1/time/pause` —— **不暂停的话战斗几帧就结束、舞台跟着弹掉，截到的
   是弹掉之后的画面**。第一次排查卡在这儿，误以为舞台没上屏。
4. `/v1/screenshot` 取图。

`/v1/script/eval` 加了 `start_combat`（`[id0..id5, boss?]`）：每场脚本
战斗都藏在过场后面，没有它就得先演一遍剧情才看得到战斗。

### 还没做

- **镜头只做了静态一台**。`cbCam.ini` 有 47 节，其中
  `DefaultCam0..17` 十八台是游戏用的（其余 `EditorCam*` 是战斗编辑器留下
  的）。现在取 `DefaultCam0` 的位置，朝向看战场中心，FOV 39°。

  **`yaw`/`pitch`/`roll` 没用**——那套角度的约定没定死。拿 `DefaultCam0`
  验算：位置 (−70.4, 337, −533.7) 看向原点时俯仰 −32°，而 ini 写的
  `pitch` 是 −35°（差 3°）；`yaw` 是 −181.7°，加 π 得 −1.7°，实际水平角
  却是 7.5°（差 9°）。方向大致对得上但不精确，所以和 Pal3.Unity 一样只取
  位置（它的 `CombatManager.SetCameraPosition` 也把旋转那行注释掉了，改用
  `LookAt(Vector3.zero)`）。朝原点是有依据的：十一个站位的重心在
  (−24, 16)。FOV 39° 取自 Unity，exe 侧没查，是 **[推断]**。

  切镜头（十八台怎么选、`flexcam.dat`）还没接。
- **动作**。所有人都摆成 `z1` 站着不动，攻击/受击/施法都没有。
- 五灵环（`[FiveLineup]` / `[EnemyFiveLineup]` 的位置和半径已解析，没画）。
- 战场光照 `.lgt` 没读，现在给的是白色环境光。
- ~~升级成长曲线本身~~ —— 五条曲线（`FUN_004fa210` / `004fa280` / `004fa330` /
  `004fa3a0` / `004fa440`）和经验门槛表（`FUN_004fa4e0`）移植里都已实现，
  见 `states/growth.rs` 与 `GameDb::level_for_exp`。


---

## 9. 补完：行动条速率、行动代价、逃跑、合击技

本节补上第 8 节列出的四个缺口。全部读自指令，地址随文标注。

### 9.1 速度 → 行动条速率（**已闭环**）

写入点是 **`FUN_004db660(k)`**（0x4db660），一次写两个字节：

```c
void ATB::RefreshSpeed(int k) {
    rec[k].speed     = ATB::CalcSpeed(k, 0);   // +0x2c
    rec[k].castSpeed = ATB::CalcSpeed(k, 1);   // +0x2d
}
```

`FUN_004db640` 反汇编（0x4db640）：

```asm
mov  eax, [esp+4]        ; k —— 只读第一个参数
mov  ecx, [ecx+8]        ; this = ATB->[+8] = CBRoleState 数组
push eax
call 0x4f3920            ; 速_eff(k)，含 ±25% 的 速+/速− 状态
add  eax, 4
lea  eax, [eax+eax*4]    ; ×5
ret  8                   ; 吃掉 2 个参数
```

即

```c
速率 = (速_eff(k) + 4) * 5
```

> **那个 0/1 参数从头到尾没被读过**（`ret 8` 弹两个参数，但函数只取 `[esp+4]`）。
> 所以 **`speed` 与 `castSpeed` 永远相等**，蓄力段和出手段一样快。

代回行动条：填满一段需要

```
11500 / (50 * (速_eff + 4) * 5) = 46 / (速_eff + 4)  秒（战斗时间轴）
```

景天 1 级 速度 3 → 速率 `(3+4)*5 = 35` → 每段 **6.57 秒**，一个完整
「蓄力 + 出手」循环约 13.1 秒。第 8 节里 `230/速度` 的疑问到此解决：
写进行动条的不是原始属性，而是 `(速+4)*5`。

**溢出边界**：速率存在 u8 里（`mov byte ptr [ebx+0x2c], al`）。
`速_eff = 47` → 255 是上限；**48 会溢出成 4**，行动条骤然变慢。
`[推断]` 原版应该没有能把速度堆到 48 的途径。

速度来源与刷新：`FUN_004f39a0(k, delta)`（0x4f39a0）改
`roleState[k] + 0x26c`（战斗内的 速 属性）后立刻调 `FUN_004db660`；
`FUN_004f3e20`（状态施加/解除）与 `FUN_004f8bb0` 也会调它。
所以 速+/速− 一挂上，行动条速率当帧就变。

### 9.2 行动代价与「打断」

> **更正**：早先这里写「`FUN_004db730` 和 `FUN_004db780` 都没有调用方，是死
> API」「不同行动没有不同的时间代价」——两条都错，是当时扫调用点漏了。按
> `E8 rel32` 重扫，`FUN_004db730` 有 2 处、`FUN_004db780` 有 2 处调用，出手
> 速度也确实随行动不同。正确的写在下面第 9.2.1 节，原来关于「打断」的三档
> 仍然成立。

行动结束一律
`FUN_004f4820`（0x4f4820）→ 若不在蓄力段且 `roleStates+0x225 == 0`
就 `FUN_004db710(k)`（两条清零、回蓄力段）。

真正有差别的是**被打中时怎么打断对方**。三处调用点，条件都是
「目标在出手段（phase 1）且出手条未满」：

| 来源 | 对目标做的事 | 地址 |
|---|---|---|
| 仙术命中 | `FUN_004db750(k, 11500 − 仙术.level × 11500/8, 0, 0)` —— 退回蓄力段，退多少由**仙术等级**决定 | `FUN_004f1070` |
| 物理攻击 | `FUN_004db8b0(k)` → `gauge1 = 5750, gauge2 = 0, phase = 0` —— 退回蓄力段，固定退到**一半** | `FUN_004f0a10` |
| 另外四条效果路径 | `FUN_004db750(k, 0, 0, 1)` —— 留在出手段，但**出手条清零**，等于重新吟唱 | `FUN_004f1420` / `004f1a30` / `004f1b20` / `004f1c40` |

`仙术.level` 是 gdb `SkillInfo.level`（记录 +0x2c4 = 708）。
level 1 → 退到 10063（几乎没损失），level 8 → 退到 0（全清）。

调度器 `LAB_00513840` 里还有一处 `FUN_004db750(k,0,0,0)`（0x513ac2）：
该单位被状态（`FUN_004f3d90`）或 `+0x36f` 判定为不能行动时，**整条清零、跳过这次出手**。

### 9.2.1 单位自己进入出手段：行动任务 `FUN_00514870`

这就是先前缺的那一环。任务入口 0x514870，`edi` 指向调度器里一条 0x50 字节的
任务记录（`esi + 0x98 + 槽*0x50`），`edi+0x30` 是单位号 `k`：

```c
if (task->+0x25 != 0) goto 尾段;              // 已经开始过

if (FUN_004eea70(roleStates, k, task->+0x2c, &out) == 1) {
    task->+0x28 = 0x10e;  task->+0x2c = out;   // 目标没了，改指向
    FUN_004db710(k);                            // 复位，换任务 0x514220
} else {
    FUN_004db920(k, 0);                         // 清 +0x2f 那个字节标志
    if (!FUN_004db6a0(k)) {                     // 蓄力条没满 -> 这一帧不行动
        FUN_00515c50(this, 槽);  return -1;
    }
    FUN_004db730(k);                            // ← gauge1 = 0, phase = 1
    mult = FUN_004f76c0(roleStates, k, task->+0x2c);
    if (mult > 3) {
        FUN_004db750(k, 0, 11500, 1);           // 瞬发：出手条直接满
    } else {
        if (mult == 0) mult = 1;
        v = FUN_004db640(k, 1) * mult * 2;      // 基速 (速_eff+4)*5
        if (v > 255) v = 255;
        FUN_004db780(k, v);                     // -> +0x2d 出手速度
    }
    FUN_0050a8e0(actor[k], 4);                  // 播动作
    task->+0x25 = 1;
}
```

两条结论：

* **出手速度 = `min((速_eff+4)*5 × 倍率 × 2, 255)`**。倍率来自
  `FUN_004f76c0`：在角色的 8 个技能槽（`+0x2e8`，892 字节跨度）里找这个技能，
  命中就返回 `+0x408` 那一格的字节 `+1`，找不到返回 0（任务里当 1 用）。
  所以**即便是普通攻击，出手段也以两倍蓄力速度推进**——先前「两段一样快」
  的说法只对基速那一层成立，行动任务会把出手速度改掉。
* `FUN_004db640(k, which)` 的第二个参数**根本没读**（`mov eax,[esp+4]` 取的是
  第一个），所以「速度」和「出手速度」的**基数**确实相同，改的是倍率。

逃跑任务（`LAB_00515200`，0x51527a 起）走同一条路，只是出手速度换成
**`速_eff × 8 + 20`**（`shl al,3; add al,0x14`，按字节截断）；若
`roleStates->+0x225`（战斗收尾）非零则 `FUN_004db750(k, 0, 11500, 1)` 立即完成。

### 9.3 逃跑：玩家菜单**掷骰子**，AI 路径不掷

**玩家菜单**（`FUN_00500d50`，0x500e60–0x500f10）选"逃跑"时：

```c
enemy = FUN_004ee910(2);                         // 一个在场的敌人
v = FUN_004f98b0(3, self, enemy);                // 逃跑百分比，见下
if (v < 1) fail;
else {
    p = round(v * 10.24);                        // 百分比 → 10 位刻度
    ok = (((seed = seed*0x343fd + 0x269ec3) >> 16) & 0x3ff) < p;
}
if (roleStates->+0x225 != 0) ok = 1;             // 战斗收尾标志：强制成功
if (desc->+0x50 != 0) ok = 0;                    // 禁止逃跑：掷完再否决
```

`FUN_004f98b0` 的 `case 3`：

```
((自己的运_eff − 在场敌人中最小的运) × (自己的等级 − 在场敌人的平均等级) + 3) × 10
```

"在场"看 `+0x241`（在场）而不是 `+0x242`（活着）——死掉的敌人也算；等级取
`+0x248`，运走 `FUN_004f38a0`；平均值是整数除法，除数是在场数，没有防零。
注意两个差同为负时乘积为正，公式会给出几百个百分点。

成功时 `roleStates->+0x225 = 1`，排 `LAB_00515200`（动作码 `0x10b`）全队撤离，
该任务在 0x51571c 写 `cbSystem.phase = 4` → 走胜利结算 → `result = 2`。

**强制路径** `FUN_004f1fa0(param_1, param_2, kind)` 的 `case 2` 才是之前
记的"不掷骰子"：`+0x50 == 0` 直接置 `+0x225 = 1` 并排撤离任务，否则排
`FleeFail` 提示。`kind` 是**道具的 `ItemSpecialType`**——case 2 就是鬼遁水
那一类道具走的路（`FUN_004f2750` 施用道具时先过 `FUN_004f1fa0`，见
`item_use.md` §2.3）。原先这里写"怪物或剧情驱动的逃跑，不是玩家菜单"是错的：
它确实由玩家菜单触发，只是入口是道具而不是逃跑项。

> 数据库里 `CombatActorInfo.escape_rate`（+0x2d4）在 0x4d0000–0x520000
> 全段**没有任何读取者**（唯一命中是 `FUN_004fa930` 里对**仙术**记录
> +0x2d4 = `composite_skill_ids[0]` 的访问，属同偏移误撞）。

### 9.4 合击技的触发条件（**已闭环**）

判定在 **`FUN_004eea70(施法者槽, 技能id, out)`**（0x4eea70）：

```c
if (施法者槽 < 5 || 施法者槽 > 10) {          // 只有我方
  for (idx = 0; idx < ComboCount(); idx++) {
    ok = true;
    comboId = FUN_00479260(idx);
    rec     = FUN_004791e0(comboId);           // 合击技记录
    if (rec->skill_id(+0x38) != 技能id) continue;
    if (rec->main_actor_requirements[0](+0x24) != 施法者.+0x228) continue;

    for (j = 0; j < 4; j++) {
      req = rec->main_actor_requirements[j];    // +0x24 + 4j
      if (req == 0) continue;
      slot = 角色ID→槽(req);
      if (!roleState[slot].active(+0x241)) ok = false;      // 必须在场

      e = rec->element_position_requirements[j];            // +0x34 + j
      if (e != 0 && e < 7 && e != roleState[slot].+0x470) ok = false;
      if (e == 7 && roleState[slot].+0x470 ∉ {3,2,4}) ok = false;   // 风火雷任意
      if (e == 8 && roleState[slot].+0x470 ∉ {1,2,5}) ok = false;   // 水火土任意

      if (rec->weapon_type_requirements[j](+0x3c + j) != roleState[slot].+0x288)
          ok = false;                                        // 武器类型
    }

    if (ok) for (t = 0; t < 3; t++) {          // combat_state_requirements
      slot = 角色ID→槽(rec->main_actor_requirements[t+1]);
      st   = rec->combat_state_requirements[t];              // +0x44 + 4t
      if (0 <= st && st < 0x1a && !HasState(slot, st + 1)) ok = false;
      if (st == 0x1a && !FUN_004f3cc0(slot))               ok = false;
    }
    ...
```

**角色 ID → 战斗槽的映射表**（`switch` 于 0x4eeb0a）：

| 角色 ID | 槽 | 角色 |
|---|---|---|
| 0x65 | 0 | 景天 |
| 0x66 | 4 | 长卿 |
| 0x68 | 1 | 雪见 |
| 0x69 / 0x6a | 2 | 龙葵 |
| 0x6b | 3 | 紫萱 |

（`0x67` 不在表内。`0x69`/`0x6a` 同映到龙葵，`[推断]` 是龙葵红/蓝两态。）

两个新确认的战斗位字段：

| 偏移 | 含义 |
|---|---|
| `+0x228` | 该战斗位的**角色 ID**（0x65 那套编码） |
| `+0x470` | 该战斗位的**五灵阵位** 1..5，编码同 `ElementPositionRequirementType`（1 水 2 火 3 风 4 雷 5 土） |
| `+0x288` | 该战斗位的**武器类型**（gdb `WeaponType`） |

状态要求用的是 `HasState(slot, st + 1)`，与 `combat_formulas.md` 里
「状态标志下标 k = 枚举 + 1」的约定一致，互相印证。

### 9.5 顺带确认的行动码

`FUN_004f05a0`（0x4f05a0）按行动种类取「目标范围」字段，暴露了整套行动码：

| 行动码 | 含义 | 取记录 | 目标范围字段 |
|---|---|---|---|
| 0x104 / 0x105 / 0x10d | 仙术类 | `FUN_00479080` | SkillInfo +0x241 = `target_range_type` |
| 0x106 | 道具 | `FUN_00479130` | GameItemInfo +0x275 = `target_range_type` |
| 0x10b | 逃跑 | — | — |
| 0x10e | **合击技** | `FUN_004791e0` | ComboSkillInfo +0x250 = `target_range_type` |

三个 `target_range_type` 偏移与 gdb 布局逐一吻合（577 / 629 / 592），
**再次印证运行时的仙术、道具、合击技记录就是 gdb 记录本身**。

### 9.6 意外收获：升级成长曲线的入口

结算函数 `FUN_004fa930(0, …)`（胜利分支）里，经验累加到 `+0x7e` 后用
`FUN_004fa4e0(exp)` 求新等级，再用五个函数按 `(角色, 等级)` 取属性值、
以差值累加到角色身上：

```
FUN_004fa210(角色, 等级)   FUN_004fa280   FUN_004fa330   FUN_004fa3a0   FUN_004fa440
```

学新仙术的条件是 `skill->+0x2c9 <= 新等级`（= `required_actor_level`）。
这正是长期挂着的「升级/成长公式」的入口，但不在本次范围内，未展开。

## 10. 移植状态（2026-09-02）

代码在 `yaobow/shared/src/openpal3/combat/` 与
`yaobow/shared/src/scripting/sce/commands/combat.rs`：

| 原版 | 移植 |
|---|---|
| 描述符 + 三个一次性标志 | `combat::encounter::{Encounter, PendingFlags}`；标志存在 `SceState::ext` 的 `pal3.combat.pending`，`Combat` 取走后即复位 |
| `FUN_018c59e0` 抽怪 | `encounter::pick_monsters`，用同算法的 `CrtRand`（种子取自系统时钟，不是 `srand` 的复现） |
| 73 / 80 / 82 / 83 / 131 / 132 | `SceCommandCombat{,MaxRound,MustFail,NotGameOver}`、`SceCommandGetCombat` 读 `pal3.combat.result` |
| 战斗主循环 | `combat::session::CombatSession` 逐帧驱动 `Battle`；我方回合出菜单（攻击/仙术/防御/逃跑），敌方走 `ai::choose` |
| 画面 | 只有 imgui 文字 + HP 条的占位覆盖层（键盘 + 手柄），战斗演出未做 |
| 收尾（`FUN_004055d0` case 3） | `finish()`：写回 HP/SP/MP、经验、仙术使用次数、钱、掉落；`result==2` → `GetCombat = 1`；败且未 `NotGameOver` → "游戏结束"覆盖层后退出进程（原版是 `SwitchMode(0xf)` 回标题，引擎还没有这条路） |
| 快进 | `fast_forward` 时 `auto_play + tick(0.1)` 跑完整场，脚本测试不需要输入 |

没有 1:1 的地方都标在源码注释里：倒下队员战后 HP 取 1 `[推断]`、count 夹到 6、
种子来源。

## 附：胜利结算（`FUN_004fa930`，param_1 == 0 分支）

逐个我方战斗位（要求 `+0x241` 与 `+0x242` 都非零，即在场且活着）：

1. `经验(+0x2c0) += 本场所得(+0x2c4)`。
   （`+0x2c4` 确认是"经"：`FUN_004f2910` 里仙术的特殊消耗 case 6 就是从它扣，
   与 `special_consume_type` 的图例 `6 经` 吻合。）
2. `新等级 = FUN_004fa4e0(新经验)`（线性扫经验门槛表）。
3. 若 `新等级 > 旧等级`：
   - 排一个升级面板任务；写入新等级；
   - **对五条成长曲线各取差值 `f(角色, 新) − f(角色, 旧)`，加到现有属性上**，
     两处都加：冒险侧的基础属性和战斗侧的上限。
   - 然后扫每个仙术的 `times_before_level_up[当前仙术等级] <= 使用次数`
     决定仙术是否升级（从下标 2 往下找最高满足的一档），
     再按 `composite_required_*` 判定能否学会合成法术、
     `next_level_skill_id` 换成下一级法术。

> **升级用的是差值，不是重算。** 这一点与脚本驱动的 `FUN_0040ff70` 不同：
> 后者从零重算五条曲线并把上限整组复制到当前值（顺带全满）。
> 差值加法的意义在于**保住永久加成** —— 比如道具抬高过上限的那部分，
> 重算会把它悄悄抹掉。两条路径在移植里分别是 gain_level 与 set_level。

## 附：经验怎么分、钱和掉落（`FUN_004fa930` 另一分支 + `FUN_004fb370`）

### 经验（`FUN_004fa930` 的 `param_1 != 0` 分支）

```c
// 一、汇总,并记住"最后倒下的那只"
总经验 = Σ 已死敌人.经验;                       // 敌方战斗位 +0x2c0
最后一只 = 死亡时刻(+0x574) 最大的那只;
        // 扫描用的是严格 `>`,所以同一时刻并列时取**最先遍历到**的那只
凶手 = 最后一只.+0x570;                          // 击杀者的战斗位

// 二、凶手加成:只算"和最后一只同时刻、且同一个凶手"的那些
加成池 = Σ { e : e.死亡时刻 >= 最后一只.死亡时刻 且 e.凶手 == 凶手 } e.经验;
凶手.本场经验 += 加成池 * 3 / 10;                // 3 成

// 三、平分给还站着的人
份额 = 总经验 / 活着的我方人数;                   // FUN_004ee890(1)
每个活着的成员: 本场经验 += 份额;                 // 要求 精(+0x24c) > 0

// 四、各自的经验加成百分比(+0x2ac)最后作用在总数上
每个活着的成员: 本场经验 = ROUND(本场经验 * (加成% + 100) * 0.01);
```

倒下的成员**一分不得**,连凶手加成也拿不到。

### 钱与掉落（`FUN_004fb370`）

```c
钱 = Σ 已死敌人.money_when_killed;               // gdb +0x352
if (钱 > 0) 钱 += ftol(钱 * (Σ 在场成员.+0x2b0) * 0.01);
                                                 // [0x56a644] = 0.0 是基数
掉落: 对每只已死敌人,若 normal_loot != 0 且 normal_loot_count > 0
      → 直接进背包,**没有掉落判定**,有就掉。
```

> [未解决] `+0x2b0`(金钱加成百分比)由哪个装备属性喂,没找到 ——
> 数据库里没有明显对得上的列。没有加成装备时该和为 0,加成自然消失。

---

## 9.7 战斗位与五灵阵位：查清的部分和仍然开着的部分

**战斗位是按角色定死的，不是按入队顺序。** `FUN_004eea70`（合击技资格检查）
里有一张写死的表，把合击技记录里的参与者 actor id 映到战斗位：

```text
0x65 (101 景天) -> 0           0x68 (104 龙葵) -> 1
0x69 (105) / 0x6a (106) -> 2   0x6b (107 长卿) -> 3
0x66 (102 雪见) -> 4
```

105 和 106 共用位 2——那两个角色不会同时在队。移植的
`PLAYER_COMBAT_IDS = [101, 104, 106, 107, 102]` 排法正好让**队伍下标 == 位号**，
所以按下标摆位即可；先前 `SceCommandCombat` 用的是 `team` 的枚举下标，队长换
人整队站位就跟着移位，已改。

**`+0x470` 的写入者只有一个，而且只管敌人。** 全二进制扫下来（多相位线性反
汇编，避开失步），对 `+0x470` 的写只有 9 处，其中 7 处属于别的对象（UI 布局和
一串指针字段）。真正落在战斗位记录上的只有 `FUN_004f8bb0`（0x4f8c06）：

```text
iVar2 = base + slot * 0x37c;    // 战斗位记录，892 字节
… 清零 0x228 起 0xdf 个 dword …
*(int *)(iVar2 + 0x470) = slot - 4;
```

它的五个调用点（0x4ecf50 / 0x4ecffc / 0x4f8fad / 0x4f90cf / 0x4f90dd）传的槽位
全是 **5..10**，配的是怪物 id（0x1cd2 / 0x1ce3 / 0x2e3），也就是敌方的补位和召
唤。所以 `slot - 4` 给出的 1..6 只是**敌人的序号**，不是五灵阵位。

而 `FUN_004f8bb0` 是唯一同时写 `+0x241` / `+0x242` / `+0x228` 那一组建位字段的
函数，我方战斗位（0..4）的填充函数**没找到**。

**这件事要紧，因为位置要求是真在用的。** 实机 `pal3_softstar.gdb` 里 10 条合击
技**全部**带非零的 `element_position_requirements`（取值 1..8，例如「连环破斩」
是 [3,5]、「十方封魔斩」是 [1,2,3,4]），而 `FUN_004eea70` 的判据是

```text
if (req != 0 && req < 7 && req != roleState[slot].+0x470) 不可用;
```

`+0x470` 若停在初始化后的 0，任何 1..6 的要求都过不去。两种可能：

1. 我方战斗位是被别处**整块拷贝**填的，`+0x470` 随之带进来（拷贝点没找到）；
2. 或者它确实一直是 0，那么带位置要求的合击技在原版里就打不出来。

没证据之前不下结论。要接着查，入口是「谁填我方 0..4 号战斗位」。

> **后续更正（同一轮内）**：上面说「我方战斗位的填充函数没找到」是因为我按
> **字面位移 `0x470`** 去扫，而那个写在指令里是相对 `+0x2ac` 的 `+0x1c4`。
> 填充函数是 **`FUN_004ed250`**——它把冒险侧的角色表（`DAT_00c04684`）整块拷进
> 战斗位。里面两处
>
> ```text
> 0x4ed291 / 0x4ed324:  mov cl, byte [ebx + 0x29a];  mov [ebp + 0x1c4], ecx
> ```
>
> （`ebp` = 槽基址 + 0x2ac，`+0x1c4` 即 `+0x470`）把**冒险侧角色记录的
> `+0x29a` 字节**搬成五灵阵位——那正是阵法页存的位。同一函数 0x4ed42a 处还有
> 敌方那一支：`lea esi, [edx - 4]`，和 `FUN_004f8bb0` 同一个公式。
>
> 所以上面列的两种可能里，**是第一种**：整块拷贝带进来的，带位置要求的合击技
> 在原版里打得出来。移植已把 `Combatant::element_position` 接到
> `PersistentState::lineup_position`（阵法页存的值）。
>
> 教训还是那条：**按字面偏移扫指令会漏**，编译器常把结构体基址提前折进寄存器。

---

## 9.8 战后不回血；`FUN_004ed0a0` / `FUN_004ed100` 的语义

### 先说方法：线性扫二进制不可信

先前几处「全二进制扫下来只有 N 处写」的结论是用**多相位线性反汇编**得出的，
那个方法会失步。反例：`FUN_004f2c70` 明明在五处写 `+0x24c`，四个相位的线性扫
一处都没抓到。

可靠的做法是**从函数入口逐个反汇编**——Ghidra 导出的 `decompiled.c` 里有 6328
个 `// ==== FUN_xxx @ addr (size N)` 头，按它们的地址和长度反汇编，对齐一定正确。
本节和第 9.7 节的结论都已用这个方法重跑过。

### 战后没有回血

对战斗位 `+0x24c`（精）的写，全二进制只有三处落在战斗位上：

| 位置 | 做什么 |
|---|---|
| `FUN_004f2c70` | 受伤/治疗，钳进 `0..上限` |
| `FUN_004f3e20` @ 0x4f44d7 | **复活**：`+0x242` 置 1、`+0x24c` 置 **0**、清一批状态字段 |
| `FUN_004f8bb0` @ 0x4f8cbc | 敌方建位，从 gdb 取值 |

另有 0x530440 一带的七处，那是**存读档**——也就是说我方的战斗精**就是**持久
精，战斗结束不存在「写回冒险侧」这一步。

**结论：倒下的队员战后留在 0 精、存活位为 0，没有任何下限。** 要站起来只能靠
复活（`FUN_004f3e20` 的那一分支——所以「复活」是个**状态效果**，这也补上了
`combat_formulas.md` 里「找不到清/置 `0x242` 的战斗内路径」那条）。移植先前
`role.hp = c.hp.max(1)` 是标了 `[推断]` 的猜测，已按二进制改回 `c.hp`。

> 代价要说清楚：移植目前**没有用道具的通路**，所以倒下的成员会一直躺到脚本
> 调 `FullTeamAtt` 为止。这是移植的缺口，不是原版行为。

> ## 更正：上面这一节的结论是错的
>
> 「战后没有回血」不成立。**`FUN_004ed870`（0x4ed870）就是战后写回**，由胜负
> 判定在 0x4f7dc1 / 0x4f7f97 两处于战斗结束时调用：
>
> ```text
> edi = 0xc0476c;                 // 冒险记录的 精（+0xe8），跨度 0x580
> esi = 战斗位 + 0x24c;            // 战斗位的 精，跨度 0x37c
> do {
>     if (byte [esi - 0xb] != 0) {          // +0x241 在场
>         if ([esi] <= 0) {
>             [esi] = 1;                     // ← **精回到 1**
>             FUN_004f34e0(slot, -2);        // +0x274 扣 2，带音效 wa048
>         }
>         [edi] = [esi];                     // 写回冒险侧
>     }
>     edi += 0x580;  esi += 0x37c;  slot++;
> } while (edi < 0xc062ec);                  // 五个我方位
> ```
>
> 所以**倒下的队员战后确实回到 1 精**——移植原来的 `.max(1)` 是对的，我按错误
> 结论改掉之后又改了回来。
>
> **漏掉它的原因和第 9.7 节同一个**：那句是 `mov dword [esi], 1`，结构体基址
> 早就折进了 `esi`，按字面偏移 `0x24c` 扫指令根本扫不到。**教训：这种"全二进制
> 只有 N 处写"的论断，只要目标是结构体字段就不可靠**，必须顺着「谁持有这个
> 结构的指针」去找。
>
> 另一件顺带查实的事：**这个写回只写精那一个 dword**（`mov eax,[esi];
> mov [edi],eax`），**气和神不写回**——战后会退回进战斗前的值。进战斗时是
> `FUN_004ed250` 把冒险记录整块拷进战斗位（冒险表 `DAT_00c04684` 跨度 0x580、
> 战斗位跨度 0x37c），出来只回精这一项。移植已按此改：只写 `role.hp`。
>
> 本节前面关于 `FUN_004ed0a0` / `FUN_004ed100` 的两段仍然有效。

### `FUN_004ed0a0(mode)` = 我方是否全灭

扫战斗位 **0..4**，找一个符合条件的；找不到就返回 1。

```text
mode == 0：在场(+0x241 != 0) 且 存活位(+0x242) == 1
mode != 0：在场(+0x241 != 0) 且 精(+0x24c) >= 1
```

### `FUN_004ed100(mode)` = 敌方是否全灭

同样的形状，扫战斗位 **5..10**（起始偏移 `0x13ae = 5 × 0x37c + 0x242`，正好对
上），条件多一个 `+0x598 == 0`：

```text
mode == 0：在场 且 存活位 == 1 且 +0x598 == 0
mode != 0：在场 且 精 >= 1     且 +0x598 == 0
```

这两个函数也顺带确认了**我方是 0..4、敌方是 5..10**。

## 9.9 每回合的敌方钩子：分裂、逃走、回合上限

`FUN_004ecb20(slot)` 是「这一位又过了一个回合」的钩子，回合制的几件事都挂在
这里：

```c
if (stage->+0x21c <= stage->+0x220) return 1;      // 回合上限到了
if (slot->+0x590 < FUN_004db700(slot)) {           // 这一位该走下一个回合了
    slot->+0x590 += 1;
    if (slot == 10) stage->+0x220 += 1;            // 最后一个敌方位走完 = 全场过了一回合
    FUN_004f48c0(slot);                            // 状态的每回合结算
    ...
}
```

`+0x21c` 是本场的回合上限（`FUN_004ec830` 从战斗描述符 `+0x1c` 取，也就是脚本
指令 82 `CombatMaxRound` 设的那个），`+0x220` 是已经过去的回合数，`+0x224`
来自描述符 `+0x48`（`CombatMustFail`）。移植的 `Battle::max_round` /
`must_fail` 就是这两个。

同一个函数里还有两件按随机数触发的事：

* **敌人逃走**：掷 `rand & 0x3ff` 和一个阈值比，中了就把 `+0x597` 置 1 并放
  0x15 号特效。`+0x597` / `+0x598` 非零的位不能当目标（`FUN_004eecb0`），也不
  计进「敌方还有活口」（`FUN_004ed100`）。
* **敌人分裂/召唤**（0x4ecf00 一带）：怪物 id `0x1cd2` 且 `+0x590 > 2` 时，以
  `0x33/0x400`（≈5%）的概率找一个**空着或已经倒下**的敌方位
  （`+0x241 == 0 || +0x242 == 0`），`FUN_004f8bb0(位, 0x1cd2)` 造一只同类；
  id `0x1ce3` 且 `+0x590 == 2` 时以 `0xcd/0x400`（≈20%）找一个**完全空**的位造
  一只 `0x1ce3`。

另有 `FUN_004f8fd0`：满足两组旗标条件时，把 `0x1cc9 / 0x1cc8 / 0x1cca` 三个 id
洗牌 16 次，取前两个放进**第 5、第 7 位**。

### 建位函数 `FUN_004f8bb0(slot, 怪物id)`

它也是**槽位记录布局的证据**：一上来 `rep stosd` 从 `+0x228` 起清 0xdf 个
dword（正好 0x37c 字节），所以**每条记录是 `stage + slot*0x37c + 0x228` 起的
0x37c 字节**，`FUN_004ee870(slot)` 返回的就是这个地址。清完之后置 `+0x241`
（在场）、`+0x242`（存活）为 1，写怪物 id 到 `+0x228`，`+0x470`（五灵阵位）
= `slot - 4`，从 gdb 行搬精气神武防速运、名字（`+0x230`）、按 id 段位设
`+0x289` / `+0x28a` 两个打击范围档，最后放 0x141 号特效当出场演出。
