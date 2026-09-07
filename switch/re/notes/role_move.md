# 脚本驱动的角色移动（SCE 20 / 214）

两条命令，同一套实现：

| opcode | handler | 名字 | 参数 |
|---|---|---|---|
| 20 | `00434f90` | `ActorPathToCommand` | int int int int |
| 214 | `00435010` | `ActorMoveToCommand` | int int int int |

参数是 `角色号, tile x, tile y, 方式`。两者只差一句：20 会额外把
`+0x5b4` 清零，然后 20 走 `FUN_004132b0`（寻路）、214 走 `FUN_00412a90`
（直线）。**第四个参数两者完全一样**，都过 `FUN_00434f60`。

地址都是 PAL3 v1.4 未加壳 `PAL3_unpacked.exe`。

## 1. 第四个参数是「怎么走」

```text
FUN_00434f60(param):
    param == 1 → 2
    param == 2 → 3
    其余（含 0） → 1
```

这个内部值存进角色的 `+0x504`（`FUN_00412b60`：距离 < 12 就不动，否则
`+0x4f4 = 4` 进移动状态，目标存 `+0x540..`）。

`FUN_00412710` 拿 `+0x504` 分两件事——挑动作、挑速度。

### 动作（`FUN_00412940` → `FUN_004126d0`）

```text
FUN_004126d0(内部值, 角色类型 +0xad):
    角色类型 ∈ {0,1,2,4} 且 内部值 != 0:
        内部值 == 1 → 0x34
        内部值 == 2 → 0x3a
    否则 → 0x32
```

动作号查 `(&PTR_DAT_00570904)[id]`：

| id | 名字 |
|---|---|
| 0 | `c01` |
| 1 | `c02` |
| 2 | `c03` |
| 0x32 (50) | `z1` |
| 0x34 (52) | `z3` |
| 0x3a (58) | `z3` |

**52 和 58 是同一个 `z3`**——走和跑播的是同一条动画，区别在倍速：

```text
FUN_00412940 末尾：
    if (+0x504 != 2) FUN_00536d00(1.0);
    else             FUN_00536d00(2.4);      // 0x4019999a
```

### 速度（`FUN_00412710` 的 `switch (+0x504)`）

```text
case 0/5/7 → 速度 0（不动）
case 2     → +0x508 = PRunSpeed;  +0x50c = RunForce
case 1/3   → +0x50c = WalkForce
             角色类型 +0xad == -1（主角） → +0x508 = PWalkSpeed
             +0xad == 2（怪物）           → 按 +0x58c 的 +0x2f4 取
                                            AttackSpeedS / M / L
             其余 NPC                     → 角色自带的 +0x1fc，为 0 才取
                                            WalkSpeed
```

**跑那一支无条件用主角的跑速**，脚本让 NPC 跑也一样。

### 汇总

| 脚本第四参数 | 内部值 | 动作 | 动画倍速 | 速度 |
|---|---|---|---|---|
| 0（和其他值） | 1 | `z3` 走 | 1.0 | 主角 `PWalkSpeed` / NPC 自带或 `WalkSpeed` |
| 1 | 2 | `z3` 走 | **2.4** | `PRunSpeed` |
| 2 | 3 | `z1` 站 | 1.0 | 同上一行 |

参数 2 是「保持站立姿势被挪过去」，实机看着就是倒退或被拖。

## 2. 速度常数从哪来

不在 exe 里。`FUN_0041efa0` 启动时读 **`datascript\scene.txt`**：

```text
FUN_004318b0("datascript\scene.txt")
FUN_004319f0("role")                       // 找 role 节
循环：读一个词，和名字表比对（从 0x58b37c "TalkRange" 起，步长 0x14）
      命中 → 读下一个词转 float → (&DAT_0058b378)[i * 5] = 值
FUN_004319f0("monster")                    // 再读 monster 节
```

`(&DAT_0058b378)[i*5]` 是 float 数组，`i*5` 个 float = `i*0x14` 字节。
名字表从 0x58b37c 起、值数组从 0x58b378 起——**每条记录是
`[值 4 字节][名字 16 字节]`，值在名字前面**。这一点定下来之后，代码里那些
`DAT_0058b3xx` 才对得上语义：

| 代码里的 | 名字在 | 是哪一项 |
|---|---|---|
| `DAT_0058b328` | 0x58b32c | AttackSpeedS |
| `DAT_0058b33c` | 0x58b340 | AttackSpeedM |
| `DAT_0058b350` | 0x58b354 | AttackSpeedL |
| `DAT_0058b38c` | 0x58b390 | **WalkSpeed** |
| `DAT_0058b3b4` | 0x58b3b8 | **WalkForce** |
| `DAT_0058b3c8` | 0x58b3cc | **RunForce** |
| `DAT_0058b3dc` | 0x58b3e0 | **PWalkSpeed** |
| `DAT_0058b3f0` | 0x58b3f4 | **PRunSpeed** |

（先按「值在名字后 +0x10」推过一版，那样走路速度会取到 `TalkRange`、跑
会取到 `PWalkSpeed`，语义讲不通；`FUN_0041efa0` 里两个基址差 4 定死了是
前面。）

出厂的 `role` 一节，连原版的注释一起：

```text
TalkRange    40
WalkSpeed    80     ;NPC走的速度
RunSpeed     180    ;NPC跑的速度
WalkForce    800    ;NPC走的加速度
RunForce     1200   ;NPC跑的加速度
PWalkSpeed   100    ;主角走的速度
PRunSpeed    200    ;主角跑的速度
JumpHeight   64     ;跳高
JumpDist     160    ;跳远
GForce       392    ;重力加速度,9.8*40
```

`monster` 一节另有 `FreeRange*` / `FreeSpeed*` / `FindFov` /
`FindRadius*` / `AttackSpeed*` / `AttackRange`，是怪物 AI 用的。

## 3. 移植

- 规则：`states/role_move.rs`——`MoveMode` 三态、`RoleConstants` 解析
  `scene.txt` 的 `role` 节、`speed()` 照 §1 的表挑速度，单测覆盖。
- 常数在 `GlobalState::new` 读一次（原版也是启动读一次），
  `GlobalState::role_constants()` 取。
- 两条命令：`commands/role_move_to.rs`（214）和 `role_path_to.rs`（20）
  共用 `apply_move_action` / `move_speed`。
- `pal3_scene_constants_census` 拿实机 `scene.txt` 核对十个字段，并核对
  三种方式各挑到哪个速度。

### 改之前是什么样

- **214 `RoleMoveTo`：第四个参数叫 `unknown`，从来没用过**，`initialize`
  里无条件 `run()`，速度硬编码 175。也就是说**全游戏由脚本驱动的角色移动
  都是跑的**，且不分主角和 NPC。
- 20 `RolePathTo`：已经按 `run == 1` 分了走跑，走 80（碰巧就是
  `WalkSpeed`）、跑 175（应为 `PRunSpeed` = 200），主角和 NPC 不分。
  `run == 2` 时把朝向反过来——这一条和 §1 的「站立姿势移动」对得上，作者
  应该是从实机行为观察到的，保留了。

### 还没做的

- **动画倍速**。原版跑是 `z3` 放 2.4 倍速，移植端的 `RoleController::run()`
  另挑 `c03`（真正的跑步动画），视觉上等价，但 `play_anim` 没有速度参数，
  要改 radiance 的动画组件才能照抄倍速。
- **角色自带的速度 `+0x1fc`**。NPC 走路时它优先于 `WalkSpeed`，移植端还
  没有对应的通路，一律当 0 处理。
- **怪物那一支**（`+0xad == 2` 的 `AttackSpeedS/M/L`）没接，脚本驱动的
  移动基本不会走到它。
- 「是不是主角」原版看角色类型 `+0xad == -1`，移植端按角色号在
  `PLAYER_ACTOR_IDS` 里判。
