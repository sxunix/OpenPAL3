# 战斗位记录的完整字段表（`CCBRoleState`）

先前笔记里那些 `+0x242`、`+0x470`、`+0x59c`「战斗位某某偏移」，基址其实是
**`CCBRoleState` 这个对象**（笔记里有时写成「战斗 / stage」，不准确），
`m_Role[11]` 就在它的 `+0x228`，每条 `tagRoleState` **正好 0x37c 字节**。
结构在 `data/pal3/combat/CBRoleState.h`，这里按 MSVC 的排布算出偏移。

对得上的独立锚点有 24 个，都是先前从 exe 单独逆出来、互不相干的：
`0x241` 在场、`0x242` 活着、`0x24c` 精、`0x250` 精满、`0x254` 气、`0x25c` 神、
`0x289`/`0x28a`/`0x28b` 横排 / 竖列 / 全场、`0x2c8` 特技 id、`0x2e0` 特技学会、
`0x2e8` 仙术 id、`0x3a8` 仙术学会、`0x3d8` 使用次数、`0x408` 仙术等级、
`0x448` 可偷的钱、`0x470` 五灵阵位、`0x474` 防御、`0x475` 状态位、
`0x490` 持续计数、`0x4c6` 潜伏计数、`0x590` 自己的回合数、`0x597` 正在逃、
`0x598` 已经逃、`0x59c` 逃跑率，外加**结构大小正好等于已知的步长 0x37c**。

## `CCBRoleState` 自己的头（`+0x000` .. `+0x227`）

| `+0x…` | 字段 | 含义 |
| --- | --- | --- |
| `0x000` | `char m_sCombatDialog[512]` | 战斗对话缓冲 |
| `0x200` | `float m_fLastDialogTime` | 上次战斗对话的时间 |
| `0x204` | `bool m_bCombatDialog` | 有没有战斗对话 |
| `0x208` | `CCBSystem* m_pCBSystem` | |
| `0x20c` | `CCBStage* m_pStage` | |
| `0x210` | `CCBUI* m_pUI` | |
| `0x214` | `CUtil* m_pUtil` | |
| `0x218` | `bool m_bWeSneakAttackEnemy` | 我方偷袭敌方 |
| `0x219` | `bool m_bEnemySneakAttackUs` | 敌方偷袭我方 |
| `0x21c` | `int m_nMaxRound` | 回合上限（指令 82 `CombatMaxRound`） |
| `0x220` | `int m_nBossRound` | **BOSS（10 号位）走过的回合数** |
| `0x224` | `bool m_bMustFail` | 必定失败 |
| `0x225` | `bool m_bFleeAlwaysSuccess` | 逃跑总是成功 |
| `0x226` | `bool m_bCreatedNewRole` | 本场有新人产生（分裂 / 召唤） |
| `0x228` | `tagRoleState m_Role[11]` | 11 个战斗位，步长 `0x37c` |
| `0x287c` | `tagRoleState m_RoleXor[11]` | 同样 11 条的异或影子（防改数值） |
| `0x4ed0` | `DWORD m_dwEXPTable[100]` | 各级所需经验 |
| `0x5060` | `DWORD m_dwWinTaskID` | 胜利任务 id |
| `0x5064` | `ECBFiveLineup m_eSameRow[3][3]` | 同横排的阵位 |
| `0x5088` | `ECBFiveLineup m_eSameColumn[3][2]` | 同竖列的阵位 |
| `0x50a0` | `float m_fSwordStarTime` | 武器技槽小星星计时 |

`m_eSameRow` / `m_eSameColumn` 的地址正是先前记下的 `+0x5064` / `+0x5088`。

## `tagRoleState`（`m_Role[0]` 的偏移，第 n 位再加 `n × 0x37c`）

| `+0x…` | 字段 | 含义 |
| --- | --- | --- |
| `0x228` | `DWORD dwID` | 怪物 / 角色 id |
| `0x22c` | `int nIndex` | 序号 |
| `0x230` | `char sModel[16]` | 模型名 |
| `0x240` | `BYTE byUIIndex` | UI 面板序号 |
| `0x241` | `bool bValid` | 在场 |
| `0x242` | `bool bAlive` | 活着 |
| `0x243` | `bool bFriend` | 我方 |
| `0x244` | `bool bBoss` | BOSS |
| `0x245` | `bool bCanAction` | 能行动 |
| `0x246` | `bool bWaitForFlee` | 等待逃跑 |
| `0x247` | `bool bEnemyFriend` | 敌人的朋友（不挨敌人打） |
| `0x248` | `int nLevel` | 等级 |
| `0x24c` | `int nHP` | 精 |
| `0x250` | `int nFullHP` | 精满 |
| `0x254` | `int nGP` | 气 |
| `0x258` | `int nFullGP` | 气满 |
| `0x25c` | `int nMP` | 神 |
| `0x260` | `int nFullMP` | 神满 |
| `0x264` | `int nAttack` | 武 |
| `0x268` | `int nDefence` | 防 |
| `0x26c` | `int nSpeed` | 速 |
| `0x270` | `int nLuck` | 运 |
| `0x274` | `int nLove` | 情 |
| `0x278` | `int nMoney` | 钱 |
| `0x27c` | `int nProtectRole` | 护着谁（只景天有效） |
| `0x280` | `DWORD dwAttackMode` | 普攻方式 |
| `0x284` | `int nGPInc` | 普攻对敌方气的影响 |
| `0x288` | `BYTE byWeaponType` | 武器类型 |
| `0x289` | `bool bAttackRow` | 武器打一横排 |
| `0x28a` | `bool bAttackColumn` | 武器打一竖列 |
| `0x28b` | `bool bAttackAll` | 武器打全场 |
| `0x28c` | `bool bMagicSword` | 景天装着魔剑 |
| `0x290` | `float fGPCostDec` | 气耗减 % |
| `0x294` | `float fMPCostDec` | 神耗减 % |
| `0x298` | `float fPunchPowerInc` | 暴击威力 +% |
| `0x29c` | `float fSkillCentInc` | 特技成功率 +% |
| `0x2a0` | `float fHPIncPerRound` | 每回合回精 % |
| `0x2a4` | `float fGPIncPerRound` | 每回合回气 % |
| `0x2a8` | `float fMPIncPerRound` | 每回合回神 % |
| `0x2ac` | `float fEXPIncPerCombat` | 战后经验 +% |
| `0x2b0` | `float fMoneyIncPerCombat` | 战后金钱 +% |
| `0x2b4` | `float fRecoverItemPowerInc` | 恢复类道具加强 % |
| `0x2b8` | `int nStealCent` | 偷窃概率 |
| `0x2bc` | `bool bLostMoneyGetLife` | 装齐了「千金散去」 |
| `0x2bd` | `bool bMirror` | 低概率反弹普攻（青铜护镜） |
| `0x2c0` | `int nEXP` | 原有经验 |
| `0x2c4` | `int nCombatEXP` | 本场经验 |
| `0x2c8` | `DWORD dwSkillID[6]` | 特技 id |
| `0x2e0` | `bool bSkillMastered[6]` | 特技学会没有 |
| `0x2e8` | `DWORD dwMagicID[6][8]` | 仙术 id |
| `0x3a8` | `bool bMagicMastered[6][8]` | 仙术学会没有 |
| `0x3d8` | `BYTE byMagicUseTime[6][8]` | 仙术使用次数 |
| `0x408` | `BYTE byMagicLevel[6][8]` | 仙术等级 |
| `0x438` | `DWORD dwWeaponSkillID` | 武器技 id |
| `0x43c` | `DWORD dwCleanCent` | 净化系数 |
| `0x440` | `float fWeaponSkillMass` | 武器技蓄力（满 200） |
| `0x444` | `DWORD dwCorpse` | 尸块 |
| `0x448` | `int nMoneyCanBeSteal` | 可以被偷的钱 |
| `0x44c` | `DWORD dwItemCanBeSteal` | 可以被偷的物品 |
| `0x450` | `int nItemCanBeSteal` | 可以被偷的物品数 |
| `0x454` | `DWORD dwFiveMagic[7]` | 五灵（高 16 位攻、低 16 位防） |
| `0x470` | `ECBFiveLineup Lineup` | 阵型位置 |
| `0x474` | `bool bDefence` | 防御中（受到的伤害减半） |
| `0x475` | `bool bSpecState[27]` | 状态位，下标 = 状态枚举 |
| `0x490` | `short snSpecStateLurk[27]` | 状态持续计数 |
| `0x4c6` | `short snSpecStateLast[27]` | 状态潜伏计数 |
| `0x4fc` | `int nSpecStateRound[27]` | 状态起始回合 |
| `0x568` | `bool bAIAttack` | AI 开普攻 |
| `0x569` | `bool bAIMagic` | AI 开仙术 |
| `0x56a` | `bool bAISkill` | AI 开特技 |
| `0x56b` | `bool bAIUseItem` | AI 开用物品 |
| `0x56c` | `bool bAINegative` | AI 万全（否则全力） |
| `0x56d` | `bool bAI` | AI 开 |
| `0x56e` | `bool bAbsorbHPWorm` | 中了吸精蛊 |
| `0x56f` | `BYTE byAbsorbHPWormRound` | 吸精蛊过了几回合 |
| `0x570` | `int nKiller` | 被谁杀 |
| `0x574` | `float fDeadTime` | 最后被杀的时间 |
| `0x578` | `gbGeomNode* pLeftWeapon[3]` | 左手武器模型 |
| `0x584` | `gbGeomNode* pRightWeapon[3]` | 右手武器模型 |
| `0x590` | `int nRound` | 这一位自己的回合计数 |
| `0x594` | `bool bRepeatLastOperation` | 按了 R 重复上次 |
| `0x595` | `bool bDoNormalAttack` | 按了 F 普攻 |
| `0x596` | `bool bTransMtl` | 支持透明材质 |
| `0x597` | `bool bFleeing` | 正在逃跑 |
| `0x598` | `bool bFleed` | 已经逃掉 |
| `0x59c` | `int nFleeCent` | 每回合逃跑概率 |
| `0x5a0` | `const char* pHPSign` | 血的位数标志（隐含显示 BOSS 血） |

## 这张表顺手结清的三个老问题

### 一、谁把存活位 `+0x242` 清成 0

`+0x242` 是 `bAlive`。清它的是 **`CCBRoleState::FixState()` = `FUN_004f7880`**，
每帧由 `CCBRoleState::Update()`（`FixState` → `CheckState` → `UpdateUI`）调一次：

```text
004f7883  add   ecx, 0x25c          ; ecx = &m_Role[0].nMP
004f7889  mov   edi, 0xb            ; 11 个位
004f7890  …                         ; nHP(-0x10) / nGP(-8) / nMP(0) 各自夹到 >= 0
004f78be  mov   eax, [ecx - 0x10]
004f78c3  jg    0x4f78d5
004f78c5  mov   byte ptr [ecx - 0x1a], bl    ; +0x242 bAlive   = 0
004f78c8  mov   byte ptr [ecx + 0x232], 1    ; +0x48e 状态 25 死亡 = 1
004f78cf  mov   byte ptr [ecx + 0x231], bl   ; +0x48d 状态 24 濒死 = 0
004f78d5  …                         ; nHP < nFullHP/10 且 > 0 → 濒死 = 1
004f78fb  add   ecx, 0x37c
```

先前三轮扫描（字面偏移、`lea`/`add` 折叠、按位移的逐字节反汇编）都没找到，
是因为**基址落在 `nMP` 上、位移是负的 `-0x1a`**，不是 `0x242` 也不是 `0x1a`。

结论：**存活位完全由「精 ≤ 0」推出来**，没有独立语义。移植端一直按「精 ≤ 0
判死」是对的。`+0x241`（`bValid`）只有建位时置 1，也确实没有配对的清零——
位不会被回收，逃走走的是 `bFleed`。

### 二、`+0x2519` 没有写入者

`0x2519 = 0x228 + 10 × 0x37c + 0x19`，就是 **`m_Role[10].bValid`**。
`CCBRoleState::IsValid()` 是 `__forceinline`，所以在调用方展开成了裸偏移，
看上去像个没人写的孤立标志；写它的是建位函数 `FUN_004f8bb0`。

三处读它的地方：

| 地址 | 源码 | 做什么 |
| --- | --- | --- |
| `0x4f031d` | `CCBRoleState::CanAttackBackRole`（`cbrolestate.cpp:3233`） | 我方**普攻**打不到敌方 6 号位，只要 10 号位在场、没逃、精 > 0 |
| `0x4ff4c5` | `CCBStage::ProcAllRoleToCoAttackPos`（`cbstage.cpp:2956`） | 单体合击把目标挪进盘心前，先把 10 号位摆回原位并 `SetVisible(false)` |
| `0x4ff7ed` | `CCBStage::ProcAllRoleToInitPos`（`cbstage.cpp:3141`） | 收场时再 `SetVisible(true)` |

所以「零售版里恒为 0」那条推断是错的，两件事都要做，移植端已补上
（`Battle::attackable` 的后一半、舞台的 `StageActor::stowed`）。

`CanAttackBackRole` 的两半：

```c
// 敌打我：按阵型，挡路的是阵位 6（盘心）
if (IS_ENEMY(src) && IS_FRIEND(dest)) {
    if (src.bAttackColumn || src.bAttackAll) return true;
    if (dest.Lineup == GetGreaterFiveMagic(场景五灵))
        for (n = 0; n < 5; n++)
            if (n 在场 && !逃 && n.Lineup == CBFL_Middle && GetHP(n) > 0) return false;
}
// 我打敌：敌方没有阵型，位置写死
if (IS_FRIEND(src) && IS_ENEMY(dest)) {
    if (src.bAttackColumn || src.bAttackAll) return true;
    if (dest == 6 && m_Role[10].bValid && !bFleed && !bFleeing && GetHP(10) > 0)
        return false;
}
```

只有**普攻**走这条——`CanBeSelect` 先查 `dwCmdKind == CB_ID_ATT_BUTTON`，
仙术 / 特技 / 道具都不受限。`bAttackAll`（`+0x28b`）零售版里没有任何途径置上
（`FUN_004ed8c0` 的 switch 只写 `+0x289` / `+0x28a`，`FUN_004f8bb0` 只写
`+0x289`），所以移植端只看竖列。

### 三、全场回合数为什么数 10 号位

`m_nBossRound`（`+0x220`）这个名字就是答案。`CCBRoleState::CheckPerRound`：

```c
if (m_nBossRound >= m_nMaxRound) return true;
if (m_Role[n].nRound >= GetAttackSequen()->GetRound(n)) return false;
m_Role[n].nRound++;
if (n == 10) m_nBossRound++;          // CB_BOSS_INDEX = 10
HandleSpecStateBout(n);               // 状态结算在回合数之后
…吸精蛊 → 装备每回合回复 → 怪物按概率逃跑…
```

和 2026-09-14 从二进制改出来的读法**逐句一致**（`+0x590` = `nRound`，
状态结算在回合数之后）。`CB_BOSS_INDEX = 10` 写在 `CBDefine.h:67`。

## 顺带改掉的两处字段误读

* **`+0x488` 不是「伤害翻倍」的孤立标志**，它是 `bSpecState[19]` = `CBRSS_Mad`
  **狂**。写它的是通用的 `SetSpecState`（落在 `+0x475 + k`），所以按字面
  `0x488` 扫当然扫不到。移植端的 `K_BERSERK = 19` 本来就是它。
* **`+0x2bc` 不是护盾**，是 `bLostMoneyGetLife`——装齐「千金散去」五件套
  （绿幞头 80015 + 通神 84018 + 元宝鞋 83012 + 右利 8153 + 左利 8154，
  后两件左右不论）。效果在 `ChangeHP`（`cbrolestate.cpp:5181`）：掉血时先拿钱抵，
  钱够就一点血不掉，不够就把钱花光、剩下的照掉。**移植端没做**，要做得先把
  `Combatant::change_hp` 收到 `Battle` 一级（钱在 `Battle::money`）。

## 状态枚举（`ECBRoleSpecState`，`CBDefine.h:130`）

1 风毒 2 雷毒 3 水毒 4 火毒 5 土毒 6 武增 7 武减 8 防增 9 防减 10 运增
11 运减 12 速增 13 速减 14 定 15 封 16 禁 17 眠 18 乱 19 **狂** 20 镜
21 **壁** 22 界 23 隐 24 濒死 25 死亡 26 避毒（`CBRSS_End` = 27）。

移植端 `K_EVADE = 21` 对应的是 **壁**（挡仙术），名字起偏了，行为没错。
