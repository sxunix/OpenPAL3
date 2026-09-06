# 山洞初体验 / 山洞之狼（SCE 112）

M18「山洞之狼小游戏」里那一段：龙葵说想玩捉迷藏，说明写的是「在规定时间内，
用任何方式**捉住她**，之后……随你怎样都可以！！！」。地址都是 PAL3 v1.4
未加壳 `PAL3_unpacked.exe`。

## 1. 开局

`LAB_00437c20` → `FUN_00406ff0(0)`：

```text
DAT_00bfda30 = 0            // 结束标记
DAT_00bfda2c = 0            // 0 = 从剧情里进来；非 0 是标题画面那条路
写注册表 SOFTWARE\SOFTSTAR\PAL3\S_GAME / S_GAME_A
FUN_00432170(99, 0)         // init.sce 的过程 99「初始化山洞之狼」
逐个角色 FUN_00419ae0()      // 角色表 0xc08160，跨度 0x2200
FUN_0040e5f0(0, 1)
DAT_00bfda6c = 7            // 游戏状态 7 = 这个小游戏
_DAT_00bfda58 = 0           // 计时器
```

`init.sce` 的过程 99 干的事（用 `pal3_sce_opcode_census` 的 `dump_one_proc`
读的）：`RoleCtrl 0` → `LoadScene m18 _b` → `RoleActive 2/1` →
`RoleSetPos 2 (46,34)`、`1 (83,33)`、`-1 (60,71)` → `RoleFace -1 4`。

## 2. 每帧（主循环 `case 7`，0x405690 起）

```text
_DAT_00bfda58 += dt
if (DAT_00bfda30 == 0) {                 // 还没结束
    if (_DAT_00bfda58 >= 60.0)           // 常量在 0x56a6c0
        FUN_004071c0(-1);                // 时间到
}
…推进场景与角色…
if (DAT_00bfda30 == 0) {
    for 角色 in 角色表:
        if (角色 + 0x278 != 0 &&          // 活着
            |角色位置 − 主角位置| < 24.0)  // 常量在 0x56a65c，代码里 fadd st,st 翻倍
            FUN_004071c0(角色下标);        // 抓到了
}
```

`+0x278` 是角色加载时从 `.scn` 角色记录的 `+0x54` 抄来的
（`*(uint *)(in_ECX + 0x278) = (uint)(byte)puVar2[0x54]`），角色的每帧更新
开头也拿它当「要不要跑这一帧」的闸（`if (role+0x278 == 0) return;`）——就是
移植版的 `RoleController::is_active()`。

## 3. 收尾 `FUN_004071c0(param)`

```text
角色表逐个 FUN_004181e0()、状态置 2
if (DAT_00bfda2c != 0)                   // 标题画面那条路
    param != -1 → 过程 0x60 = 96「标题画面-胜利结束山洞之狼」
    param == -1 → 过程 0x5f = 95「标题画面-失败结束山洞之狼」
else
    DAT_00bfda6c = 2                     // 回冒险模式
    param != -1 → 过程 0x62 = 98「胜利结束山洞之狼」
                  且 DAT_00bfda44 < 4 时发一件奖品，随后计数 +1
                  奖品表（0x40726f 起压栈）：0x1388f 0x14444 0x1fd9 0x1fda
    param == -1 → 过程 0x61 = 97「失败结束山洞之狼」
```

所以**抓到 = 胜利**（过程 98 只有一句 `16 1002`，把 M18 的 1002 号过程跑
起来），**时间到 = 失败**（过程 97 → M18 的 1003）。

## 4. 移植

- `shared/src/openpal3/states/cave_wolf.rs`：60 秒、24 距离、三个过程号、
  四件奖品，外加一局的计时（先判时限再判距离，和 `case 7` 同序）。
- `scripting/sce/commands/cave_wolf.rs`：SCE 112 只负责开局——跑 init.sce
  的过程 99，再在 `SceState::ext` 上留个标记。
- `AdventureDirector::step_cave_wolf`：每帧推进这一局，抓到就发奖品（胜利
  次数记在存档的 `cave_wolf_wins`，对应 `DAT_00bfda44`）并跑过程 98，
  时间到跑过程 97。原版这一段本来就在主循环而不是脚本里，所以放在导演上。
- 标题画面那条路（`DAT_00bfda2c != 0`，过程 95/96）没做：移植还没有从标题
  画面进小游戏的入口。
