# SCE 指令分发表

从脱壳 exe 的初始化函数里直接读出来的，不是猜的。

## 表的形状

指令表基址 **`0xd8c464`**，每项 16 字节：

| 偏移 | 内容 |
|---|---|
| `+0x00` | dword，用途未定 `[推断：与执行模式/分组有关]` |
| `+0x04` | 处理函数指针（为 0 = 该号未实现） |
| `+0x08`..`+0x0f` | 最多 8 个参数类型字节，`0` 结束 |

`opcode N` 的项在 `0xd8c464 + N*16`。校验过三处：
73 `Combat` → `0xd8c8f4`，处理函数 `0x437280`，类型 `1 1 1 1`；
82 → `0xd8c984`/`0x4373a0`；131 → `0xd8cc94`/`0x438440`。

参数类型字节：

| 值 | 含义 | 文件里的宽度 |
|---|---|---|
| 1 | 整数 | 4 字节；若该位在 userVariableMask 里置位则是 2 字节变量号 |
| 2 | 浮点 | 4 字节 |
| 3 | 字符串 | 2 字节长度 + 正文 |
| 4 | 跳转目标 | 4 字节 |
| 5 | 选项表 | 2 字节个数，每项 1 字节类型 + 值 |

## 参数怎么传

解析器把参数依次压进 `0xd8c36c` 起的栈（每项 8 字节：`+0` 类型、`+4` 值），
处理函数用 `FUN_00434c00` 逐个弹出。**弹出顺序 = 声明顺序**，
所以文件里参数是倒着存的 —— yaobow 的 `command!` 宏反转读取顺序，
跟这个是一致的。弹出时如果类型是 5（变量引用），`FUN_004343b0` 会先解引用。

指令头是 `u16 opcode` + `u16 userVariableMask`。mask 的第 i 位表示第 i 个参数
存的是 2 字节的变量号而不是 4 字节的字面量。yaobow 目前对多数指令写死了类型，
只有 10 号按 mask 分支；实测大部分指令脚本只传字面量，所以没出问题，
但这是一类潜在的错位来源。

## 已修的错位

**251** 之前在 yaobow 里按 `i32, f32, f32, f32, f32` 读（"ObjectMove"），
表里 `0xd8d414` 只有一个 int 参数，处理函数 `0x435350`。
Pal3.Unity 也是 `CameraFocusOnSceneObjectCommand(int)`。多读四个 dword
会把该 proc 剩下的指令流全部错位。


## 后来查实的几处

**参数在文件里是倒序存的。** 解析器按源码顺序压栈，处理函数用
`FUN_00434c00` 逐个弹，所以文件里最后一个参数在最前面。yaobow 的
`command!` 宏反转读取顺序，跟这个一致。**手写的读法要自己反**：80 `CombatBoss` 以前按文件顺序读，
六个怪物整个倒了（古藤老人 711 该站 10 号位，被放到 5 号位），2026-09-14 改正。其余手写的（51、109、110、138）
都只有一个参数，不受影响。

**用户变量掩码**（指令头 `u16 opcode` 后面那个 `u16`）第 i 位置位时，
第 i 个参数是 2 字节的变量号而不是 4 字节字面量。位序按**声明顺序**，
不随文件里的倒序改变。

**33 和 35 是两回事。** 33 `CameraOrbit` 是 lookAt 固定、相机绕着转
（`FUN_0041cf20`，动画类型 2）；35 `CameraRotate` 是相机位置固定、
lookAt 转（`FUN_0041cf70`，类型 4）。yaobow 挂在 33 上的
`SceCommandCameraRotate` 做的是 35 的事。

**36 的参数含义。** `LAB_00435570` → `FUN_0041c9a0(yaw, pitch, distance,
x, y, z, 1)`：`(x,y,z)` 写进 `+0x1ec/1f0/1f4` 是 **lookAt 点**，第三个
标量是**距离**；随后 `FUN_0041c710(0)` 按 `相机位置 = lookAt + 距离·dir`
反推相机。yaobow 的 `camera_set.rs` 把 `(x,y,z)` 当相机位置、把距离叫
`_unknown`。

**117 / 128 的行为跟名字相反。** 都写 `DAT_00c12b28/2c/30`（世界旋转的
X/Y/Z 度数），都阻塞 5 秒，缓动 `v = (1 − cos(π·t/5))/2`：
- 117 初始化时直接设成 `(x,y,z)`，`global = (1 − v)·deg` 缓动**回正**，
  析构再清零（`FUN_004381b0`）。
- 128 初始化设 `(0,0,0)`，`global = v·deg` 缓动**转到目标**并留在那里。

这三个全局在 `gbSceneGraph::Begin` 之后、整个场景图绘制之前压进全局矩阵栈
（decompiled.c:4124–4132，依次绕 X、Y、Z 轴），所以转的是包括角色在内的整个世界。

**几个读出来的浮点常量**：`0x56a744` = 5.0（117/128/135/136 的时长）、
`0x56ab50` = 0.2、`0x56aa30` = π、`0x56a9f0` = 0.0174532924（度→弧度）、
`0x56aa5c` = 2.0（144 给特效 Y 加的偏移）。

**出厂脚本里的一处缺陷**：T01/T02 的「客栈掌柜住宿」两个 proc 只有 34 字节，
opcode 119 的三个脚本 id 参数不在自己的 proc 里。原版把整个文件当一块缓冲区
顺序读，会接着读进下一个 proc 的 12 个字节。

## 全表

`处理函数`列是 exe 里的地址；`yaobow` 列 `✓` = 已实现，`桩` = 读掉参数后什么都不做，`跳过桩` = 有指令类但只是跳过子系统（`minigames.rs`），不算实现。
`Pal3.Unity 类` 来自 `Pal3.Core/Command/SceCommands/`（GPL-3.0，0x7c13），
中文说明是它的原注释。

| 号 | 处理函数 | 参数 | Pal3.Unity 类 | yaobow | 说明 |
|---|---|---|---|---|---|
| 1 | `00434ce0` | float | ScriptRunnerWaitUntilTimeCommand | ✓ SceCommandIdle | 脚本在一定时间后才执行下一条指令（不影响游戏系统和其它脚本的执行）， |
| 2 | `00434d70` | int | ScriptRunnerChangeExecutionModeCommand | ✓ SceCommandScriptRunMode | 设置脚本运行模式， |
| 3 | `00436710` | jump | ScriptRunnerGotoCommand | ✓ SceCommandGoto | 使脚本跳转到指定位置， |
| 4 | `00436730` | （无） | — | —  |  |
| 5 | `00436760` | int | ScriptRunnerSetOperatorCommand | ✓ SceCommandFop | 指定逻辑指令与标志变量的操作， |
| 6 | `00436780` | int int | ScriptEvaluateVarIsGreaterThanCommand | ✓ SceCommandGt | 判断变量是否大于给定值并与临时变量计算结果 |
| 7 | `004367b0` | int int | ScriptEvaluateVarIsLessThanCommand | ✓ SceCommandLs | 判断变量是否小于给定值并与临时变量计算结果 |
| 8 | `004367e0` | int int | ScriptEvaluateVarIsEqualToCommand | ✓ SceCommandEq | 判断变量是否等于给定值并与临时变量计算结果 |
| 9 | `00436860` | int int | ScriptEvaluateVarIsNotEqualToCommand | ✓ SceCommandNeq | 判断变量是否不等于给定值并与临时变量计算结果 |
| 10 | `00436890` | int int | ScriptEvaluateVarIsGreaterThanOrEqualToCommand | ✓ SceCommandGeq | 判断变量是否大于等于给定值并与临时变量计算结果 |
| 11 | `004368e0` | int int | ScriptEvaluateVarIsLessThanOrEqualToCommand | ✓ SceCommandLeq | 判断变量是否小于等于给定值并与临时变量计算结果 |
| 12 | `00436910` | jump | ScriptRunnerGotoIfNotCommand | ✓ SceCommandTestGoto | 如果当前计算值为true的话就继续执行下一行指令，否则使脚本跳转至Offset处 |
| 13 | `00436360` | int int | ScriptVarSetValueCommand | ✓ SceCommandLet | 给变量赋值，Var <== Value， |
| 14 | `00436380` | int int | ScriptVarAddValueCommand | ✓ SceCommandLetAdd | 给变量增加值，Var += Value， |
| 15 | `004363b0` | int int | — | —  |  |
| 16 | `00436a60` | int | ScriptExecuteCommand | ✓ SceCommandCall | 调用另一段脚本， |
| 17 | `00436410` | int int | ScriptVarSetRandomValueCommand | ✓ SceCommandRnd | 给变量随机赋值一个正整数， |
| 18 | `00436740` | （无） | — | —  |  |
| 19 | `00436810` | int int int | ScriptEvaluateVarIsInRangeCommand | ✓ SceCommandBetween | 判断变量值是否在给定区间范围内并与临时变量计算结果 |
| 20 | `00434f90` | int int int int | ActorPathToCommand | ✓ SceCommandRolePathTo | 查找一个到TileMap上（X，Y）的路径，并使角色按此路径移动，第四参数 0走/1跑/2站姿，见 `role_move.md` |
| 21 | `00434d90` | int int int | ActorSetTilePositionCommand | ✓ SceCommandRoleSetPos | 瞬间移动角色到TileMap中该点， |
| 22 | `00435080` | int string int | ActorPerformActionCommand | ✓ SceCommandRoleShowAction | 使角色执行一个动作，第三个参数是循环次数：>0 播几遍、−1 无限循环（不阻塞脚本）、−2 播完保持末帧 |
| 23 | `00436190` | int int | ActorSetFacingDirectionCommand | ✓ SceCommandRoleSetFace | 瞬间使角色面向的方向改变， |
| 24 | `00436280` | int int | ActorRotateFacingCommand | ✓ SceCommandRoleTurnFace | 转动角色面向的方向， |
| 25 | `004365a0` | （无） | TeamOpenCommand | ✓ SceCommandTeamOpen | 玩家队伍散开 |
| 26 | `004365b0` | （无） | TeamCloseCommand | ✓ SceCommandTeamClose | 玩家队伍合并 |
| 27 | `004365e0` | int | PlayerEnableInputCommand | ✓ SceCommandRoleInput | 是否允许玩家控制当前主角， |
| 28 | `00436600` | int int | ActorActivateCommand | ✓ SceCommandRoleActive | 设置角色是否激活， |
| 29 | `00436680` | int int | ActorSetScriptCommand | ✓ SceCommandRoleScript | 设置角色的对话脚本， |
| 30 | `00435320` | int | CameraFocusOnActorCommand | ✓ SceCommandCameraFocusRole | 设置摄像机锁定角色， |
| 31 | `004352d0` | float float float | CameraFocusOnPositionCommand | ✓ SceCommandCameraFocusPoint | 设置摄像机视角锁定一个定点， |
| 32 | `004353a0` | float float int | CameraPushCommand | ✓ SceCommandCameraPush | 镜头推动作， |
| 33 | `004353f0` | float float float int | CameraOrbitCommand | ✓ SceCommandCameraRotate | 镜头环绕LookAt位置旋转（LookAt点固定）， |
| 34 | `004354f0` | float float float float int | CameraMoveCommand | ✓ SceCommandCameraMove | 镜头当前位置移动（保持当前角度）， |
| 35 | `00435470` | float float float int | CameraRotateCommand | ✓ SceCommandCameraSwing | 镜头摇动（from点固定）， |
| 36 | `00435570` | float float float float float float | CameraSetTransformCommand | ✓ SceCommandCameraSet | 设置镜头参数， |
| 37 | `004356c0` | int | CameraSetDefaultTransformCommand | ✓ SceCommandCameraDefault | 调用默认镜头参数， |
| 38 | `00435710` | （无） | CameraPushStateCommand | ✓ SceCommandCameraPushState | 保存镜头状态 |
| 39 | `00435720` | （无） | CameraPopStateCommand | ✓ SceCommandCameraPopState | 取出镜头状态 |
| 40 | `00435730` | int int int | — | —  |  |
| 41 | `00435770` | int int int | ActorSetAttributeValueCommand | ✓ SceCommandSetRoleAtt | 设置角色某项属性值的数值 |
| 42 | `00436b40` | int | LongKuiSwitchModeCommand | ✓ SceCommandLongKuiMode | 设置龙葵形象， |
| 43 | `004360f0` | int int | FavorAddCommand | ✓ SceCommandFavorAdd | 增减好感增， |
| 44 | `DAT_00436120` | int int | — | —  |  |
| 45 | `004357e0` | int int int | — | —  |  |
| 46 | `00435830` | int int | InventoryAddItemCommand | ✓ SceCommandAddItem | 增加物品， |
| 47 | `00435bf0` | int | InventoryRemoveItemCommand | ✓ SceCommandRemoveItem | 拿走一个物品， |
| 48 | `00437630` | int | InventoryAddMoneyCommand | ✓ SceCommandAddMoney | 增减金钱， |
| 49 | `00437610` | int | ScriptVarSetMoneyCommand | ✓ SceCommandGetMoney | 取出当前金钱数并赋值给变量， |
| 50 | `004375e0` | int int | ScriptVarSetActorFavorCommand | ✓ SceCommandGetFavor | 取出好感值并赋值给变量， |
| 51 | `00437550` | int int | ActorAddSkillCommand | ✓ SceCommandAddSkill | 某个主角学会新特技， |
| 52 | `004376a0` | int | ScriptVarSetMostFavorableActorIdCommand | ✓ SceCommandGetFavorite | 取得主角对谁的好感最高并赋值给变量， |
| 53 | `004376f0` | int | ScriptVarSetLeastFavorableActorIdCommand | ✓ SceCommandGetFavorite | 取得主角对谁的好感最低并赋值给变量， |
| 54 | `004357b0` | int int | ActorSetAttributeToFullCommand | ✓ SceCommandFullRoleAtt | 设置主角的某项属性为满值，例如精加满， |
| 55 | `-` | （无） | — | —  |  |
| 60 | `00436440` | int int | — | —  |  |
| 61 | `00436470` | int | — | —  |  |
| 62 | `00436490` | string | DialogueRenderTextCommand | ✓ SceCommandDlg | 显示对话，等待用户确定后脚本继续 |
| 63 | `004364e0` | string string | SceneLoadCommand | ✓ SceCommandLoadScene | 切换场景， |
| 64 | `00436530` | int string | — | —  |  |
| 65 | `00436940` | float list | DialogueAddSelectionsCommand | ✓ SceCommandDlgSel | 对话选择框， |
| 66 | `004369a0` | int | ScriptVarSetDialogueSelectionResultCommand | ✓ SceCommandGetDlgSel | 取得选择结果并赋值给变量， |
| 67 | `00436ab0` | int string int | DialogueRenderActorAvatarCommand | ✓ SceCommandDlgFace | 渲染人物头像立绘， |
| 68 | `00437170` | string | UIDisplayNoteCommand | ✓ SceCommandNote | 显示提示框信息 |
| 69 | `00437190` | （无） | CameraFadeOutCommand | ✓ SceCommandFadeOut | 屏幕淡出 |
| 70 | `004371b0` | （无） | CameraFadeInCommand | ✓ SceCommandFadeIn | 屏幕淡入 |
| 71 | `00435170` | int | ActorStopActionAndStandCommand | ✓ SceCommandRoleStop | 使角色结束当前行为，并站立在当前位置， |
| 72 | `00437210` | int int | ActorShowEmojiCommand | ✓ SceCommandRoleEmote | 在角色头顶出现表情符号， |
| 73 | `00437280` | int int int int | CombatEnterNormalFightCommand | ✓ SceCommandCombat | 进入普通战斗 |
| 74 | `00437440` | int int | ActorPerformClimbActionCommand | ✓ SceCommandClimb | 使当前玩家操纵角色攀爬场景物品， |
| 75 | `-` | （无） | — | —  |  |
| 76 | `004374a0` | string | DialogueRenderTextWithTimeLimitCommand | ✓ SceCommandDlgTime | 显示对话，等待用户确定后或者时间结束时脚本继续 |
| 77 | `004374e0` | int | ScriptVarSetLimitTimeDialogueSelectionResultCommand | ✓ SceCommandGetTimeSel | 取得限时选择结果并赋值给变量， |
| 78 | `00437500` | int | ScriptEvaluateVarIfPlayerHaveItemCommand | ✓ SceCommandHaveItem | 检查玩家是否拥有某个物品并与临时变量计算结果， |
| 79 | `00437650` | string int | PlaySfxCommand | ✓ SceCommandPlaySound | 播放音效， |
| 80 | `004373d0` | int int int int int int | CombatEnterBossFightCommand | ✓ SceCommandCombat | 进入Boss战 |
| 81 | `004371d0` | （无） | CameraFadeOutWhiteCommand | ✓ SceCommandFadeOutWhite | 屏幕淡出（白色） |
| 82 | `004373a0` | int | CombatSetMaxRoundCommand | ✓ SceCommandCombatMaxRound | 设置战斗的最大回合数 |
| 83 | `004373c0` | （无） | CombatSetUnbeatableCommand | ✓ SceCommandCombatMustFail | 设置战斗为必败 |
| 84 | `00437740` | int | — | —  |  |
| 85 | `004377f0` | int int | SceneActivateObjectCommand | ✓ SceCommandObjectActive | 设置某个物件出现或隐藏， |
| 86 | `00437b20` | string int | UIDisplayCaptionCommand | ✓ SceCommandCaption | 显示全屏字幕（例如显示诗句等）， |
| 87 | `00437a80` | int | SceneOpenDoorCommand | ✓ SceCommandOpenDoor | 播放门打开的动画 |
| 88 | `00437b90` | int | HuaYingSwitchBehaviourModeCommand | ✓ SceCommandHuaYingMode | 设置花盈的行为模式, |
| 89 | `00437bb0` | float float float | FlyingActorFlyToCommand | ✓ SceCommandHyFly | 让花盈飞到某空间点 |
| 90 | `004378e0` | int float float float float | SceneMoveObjectCommand | ✓ SceCommandObjectMove | 移动场景物体到某处， |
| 91 | `004371f0` | （无） | CameraFadeInWhiteCommand | ✓ SceCommandFadeInWhite | 屏幕淡入(白色） |
| 92 | `-` | （无） | — | —  |  |
| 100 | `00436c00` | string | UIShowDealerMenuCommand | ✓ SceCommandDealerMenu（买卖、还价；原版像素布局，见 `dealer_layout.md`） | 调用买卖系统， |
| 101 | `00436cb0` | int | ScriptVarSetPawnshopLevelCommand | ✓ SceCommandGetPawnshopLevel | 取出当前当铺经营游戏的当铺等级并赋值给变量， |
| 102 | `00436f90` | int | GameSwitchRenderingStateCommand | ✓ SceCommandSwitchMode | **实为 `SwitchMode(n)` 全局模式切换**，不是渲染状态；13 = 大地图、0 = 回冒险、15 = 回标题。见 [`bigmap.md`](bigmap.md) |
| 103 | `00436c60` | int | MiniGameStartPawnshopCommand | ✓ SceCommandMiniGamePawnshop（文字浮层，陈列/休息没做）| 进入当铺经营游戏，参数是**过程号** |
| 104 | `00437080` | （无） | MiniGameStartAppraisalsCommand | ✓ SceCommandAppraise | 进入鉴定游戏。见 [`appraise_minigame.md`](appraise_minigame.md) |
| 105 | `004370e0` | （无） | MiniGameStartSwatAFlyCommand | ✓ SceCommandMiniGameSwatAFly（照 `KF_Sys` 的贴图、拍子、说明框，见 `killflyer.md`）| 进入打苍蝇游戏 |
| 106 | `00437120` | int | MiniGameStartEncampCommand | ✓ SceCommandEncamp | 进入宿营游戏，参数是四方象枢方位号 0–3。见 [`encamp.md`](encamp.md) |
| 107 | `00436f40` | int | MiniGameStartSkiCommand | ✓ SceCommandMiniGameSki（3D 雪道、说明框 / 结算框，见 `ski.md`） | 进入滑雪游戏，参数是**过程号** |
| 108 | `004370c0` | int | ScriptVarSetAppraisalsResultCommand | ✓ SceCommandGetAppr | 取出鉴定小游戏的结果并设置给变量 |
| 109 | `00436fb0` | int | FeatureEnableSwordPurifyingCommand | ✓ SceCommandEnableFeature | 打开或关闭魔剑养成功能， |
| 110 | `00436fe0` | int | FeatureEnableJusticeBookCommand | ✓ SceCommandEnableFeature | 打开或关闭降妖谱功能， |
| 111 | `00437040` | int | PlayerLearnComposeCommand | ✓ SceCommandLearnCompose | 学会合成术， |
| 112 | `00437c20` | （无） | MiniGameStartCaveExperienceCommand | ✓ SceCommandMiniGameCaveExperience（导演里跑计时与抓人）| 进入山洞初体验游戏（山洞之狼）|
| 113 | `00437c30` | （无） | MiniGameStartHideFightCommand | ✓ SceCommandStartHideFight | 使当前场景进入<当铺&刺使府夜间特殊战斗关>状态 |
| 114 | `00437c40` | （无） | — | —  |  |
| 115 | `00437c50` | string | PlayVideoCommand | ✓ SceCommandMovie | 播放动画 |
| 116 | `00437ff0` | int string | ActorChangeTextureCommand | ✓ SceCommandSetRoleTexture | 给角色更换纹理贴图 |
| 117 | `00438030` | int int int | SceneRotateWorldCommand | ✓ SceCommandRotateWorldBack | 旋转整个场景， |
| 118 | `00438310` | float float | CameraShakeEffectCommand | ✓ SceCommandQuake | 摇晃镜头（模拟地震）效果， |
| 119 | `004369c0` | string int int int | UIShowHotelMenuCommand | ✓ SceCommandShowChatRest（客栈，见 inn.md） | 显示客栈入住菜单 |
| 120 | `00436d30` | int | ScriptVarSetPawnshopManageTypeCommand | ✓ SceCommandGetPawnshopManage | 取出当前当铺经营游戏的当前Manage类型并赋值给变量， |
| 121 | `00436db0` | int | ScriptVarSetPawnshopPeriodTypeCommand | ✓ SceCommandGetPawnshopPeriod | 取出当前当铺经营游戏的当前Period类型并赋值给变量， |
| 122 | `00436e10` | int | ScriptVarSetPawnshopAccrualTypeCommand | ✓ SceCommandGetPawnshopAccrual | 取出当前当铺经营游戏的当前Accrual类型并赋值给变量， |
| 123 | `00436e70` | int | ScriptVarSetPawnshopUnHockTypeCommand | ✓ SceCommandGetPawnshopUnHock | 取出当前当铺经营游戏的当前UnHock类型并赋值给变量， |
| 124 | `00438350` | int | PlayerInteractWithObjectCommand | ✓ SceCommandInteractWithObject | 与指定场景物品交互， |
| 125 | `00436bd0` | int int | WorldMapEnableRegionCommand | ✓ SceCommandSetBigMapElement | 大地图打开或关闭某个区域， |
| 126 | `00438390` | string int int | ScriptVarSetMazeSwitchStatusCommand | ✓ SceCommandGetSwitch | 脚本检查迷宫开关状态并赋值给变量， |
| 127 | `00436ed0` | int int | MiniGameStartSailingCommand | ✓ SceCommandEntryRow | 进入行船游戏 |
| 128 | `004381e0` | int int int | SceneRotateWorldInverseCommand | ✓ SceCommandRotateWorld | 反方向旋转整个场景， |
| 129 | `00436f40` | int | — | —  |  |
| 130 | `004363e0` | int int | ScriptVarDistractAnotherVarCommand | ✓ SceCommandVarDistract | 变量A减去B，如果减去后A小于零则A取反，并把结算结果重新赋值给变量A |
| 131 | `00438440` | （无） | CombatSetNoGameOverWhenLoseCommand | ✓ SceCommandCombatNotGameOver | 设置战斗失败不显示结束游戏 |
| 132 | `00438450` | int | ScriptVarSetCombatResultCommand | ✓ SceCommandGetCombat | 取得战斗结果（0输1赢）并赋值给变量， |
| 133 | `00438470` | string int | PlayMusicCommand | ✓ SceCommandMusic | 播放剧情音乐 |
| 134 | `004384b0` | （无） | StopMusicCommand | ✓ SceCommandStopMusic | 停止剧情音乐 |
| 135 | `004384d0` | int | ActorFadeOutCommand | ✓ SceCommandRoleFadeOut | 角色淡出 |
| 136 | `004385a0` | int | ActorFadeInCommand | ✓ SceCommandRoleFadeIn | 角色淡入 |
| 137 | `004368c0` | int | ScriptEvaluateVarIfActorInTeamCommand | ✓ SceCommandIfInTeam | 检测角色是否在队伍里并与临时变量计算结果， |
| 138 | `00437010` | int | FeatureEnableSwordSkillCommand | ✓ SceCommandEnableFeature | 打开或关闭魔剑技功能， |
| 139 | `004386c0` | float int int int | — | —  |  |
| 140 | `004387b0` | int | EffectShowSnowCommand | ✓ SceCommandSnow | 开启下雪特效， |
| 141 | `004388c0` | int | EffectSetScreenEffectCommand | ✓ SceCommandScreenEffect（0 水下、1 回忆色调，见 `screen_effect.rs`）| 设置全局镜头（屏幕）特效， |
| 142 | `00438930` | float float float | EffectSetPositionCommand | ✓ SceCommandEffectPosition | 设置VFX特效位置， |
| 143 | `004389a0` | int | EffectPlayCommand | ✓ SceCommandPlayCombatEffect | 播放VFX特效， |
| 144 | `004389d0` | int | EffectAttachToActorCommand | ✓ SceCommandEffectAtRole | 在角色位置播放特效， |
| 145 | `00438840` | int int | PlayerSetActorLevelCommand | ✓ SceCommandAverageLevel | 设置目标角色等级（所有当前角色等级的平均值+增加值）， |
| 146 | `00436a10` | int | UIShowSmeltingMenuCommand | ✓ SceCommandSmeltingMenu（界面是文字浮层）| 打开冶炼菜单，参数是**过程号** |
| 147 | `00438a20` | （无） | GameSwitchToMainMenuCommand | ✓ SceCommandSwitchToMainMenu | 退出游戏到主菜单 |
| 148 | `00438980` | int | EffectPreLoadCommand | ✓ SceCommandPreloadCombatEffect | 预加载VFX特效Group， |
| 149 | `00438a30` | int | ScriptVarSetLongKuiDressIdCommand | ✓ SceCommandGiveCloth | 把背包里所获的的服装给龙葵， |
| 150 | `00438b00` | int string | ActorLoadActionCommand | ✓ SceCommandLoadAction | 使角色预加载一个动作， |
| 151 | `00438b40` | int | — | —  |  |
| 152 | `00438b60` | int | EffectShowWaterMagicCommand | ✓ SceCommandWaterMagic | 在玩家角色周身渲染水特效， |
| 153 | `00438b80` | （无） | TeamSetAttributeToFullCommand | ✓ SceCommandFullTeamAtt | 瞬间回满全队精气神 |
| 154 | `00438ba0` | int int int | NavigationBeeFlyToCommand | 跳过 SceCommandNavigationBeeFlyTo（引路蜂，纯提示装饰） | 控制引路蜂飞向指定位置 |
| 155 | `00438d00` | float | CameraSetYawCommand | ✓ SceCommandCameraYaw | 相机设置Yaw角度 |
| 156 | `00438d20` | （无） | UIDisplayXueJianPictureCommand | ✓ SceCommandShowXueJianPicture | UI展示雪见图片 |
| 157 | `00438f10` | string | MiniGamePawnshopShowNoteCommand | ✓ SceCommandMiniGamePawnshopNote | 显示一条提示（`FUN_00410360`，和 68 同一个控件；名字里的「当铺」是误会）|
| 158 | `00438f30` | int | SceneObjectDoNotLoadFromSaveStateCommand | ✓ SceCommandObjectNotFromSave | 设置某个物件出场时候的不从存档里拿状态， |
| 159 | `00438f50` | （无） | SceneInitDivineTreeFlowerCommand | ✓ SceCommandInitFlower | 初始化神树之花 |
| 160 | `-` | （无） | MiniGameStartGhostHuntingCommand | —  | PAL3A捉鬼（龙葵）游戏 |
| 163 | `—` | （无） | ScriptVarSetObjectUsageStatusCommand | —  | 检测场景中物品（宝箱）是否被完全使用过并赋值给变量， |
| 164 | `—` | （无） | FengYaSongCommand | —  | 控制雷元戈头上鸟（风雅颂）的显示与隐藏 |
| 166 | `—` | （无） | UnknownCommand166 | —  | ??? |
| 167 | `—` | （无） | UnknownCommand167 | —  | 在场景固定位置处播放特效（蜀山雷电特效） |
| 169 | `—` | （无） | TaskOpenCommand | —  | 开启主线或支线任务， |
| 170 | `—` | （无） | TaskCompleteCommand | —  | 完成主线或支线任务， |
| 173 | `—` | （无） | ActorShowEmoji2Command | —  | 在角色头顶出现表情符号， |
| 174 | `—` | （无） | CameraOrbitHorizontalCommand | —  | 镜头朝向固定点左右Orbit |
| 175 | `—` | （无） | CameraOrbitVerticalCommand | —  | 镜头朝向固定点上下Orbit |
| 176 | `—` | （无） | UnknownCommand176 | —  | 南宫煌开启摄灵法阵 |
| 177 | `—` | （无） | WangPengXuEnableCombatTransformCommand | —  | 设置开启/关闭王蓬絮战斗中变身（朱仙变）， |
| 178 | `—` | （无） | ActorSetYPositionCommand | —  | 给角色设置Y坐标 |
| 179 | `—` | （无） | ActorWalkToUsingActionCommand | —  | 查找一个到TileMap上（X，Y）的路径，并使角色按此路径走动， |
| 181 | `—` | （无） | UnknownCommand181 | —  | ??? |
| 182 | `—` | （无） | ScriptVarSetWheelOfTheFiveElementsUsageCountCommand | —  | 取出五灵轮当前使用次数并赋值给变量， |
| 183 | `—` | （无） | UIDisplayNextCaptionCommand | —  | 显示全屏字幕（例如显示诗句等）， |
| 185 | `—` | （无） | UnknownCommand185 | —  | PAL3A打开算命小游戏UI |
| 187 | `—` | （无） | SceneCloseDoorCommand | —  | 播放门关闭的动画 |
| 188 | `—` | （无） | CameraMoveToLookAtPointCommand | —  | 移动相机并看向固定点 |
| 189 | `—` | （无） | CameraMoveToDefaultLookAtPointCommand | —  | 相机移动至默认焦距点（主角） |
| 192 | `—` | （无） | CombatEnterBossFightUsingMusicWithSpecialActorCommand | —  | 南宫煌以变身状态下进入Boss战斗 |
| 193 | `—` | （无） | UnknownCommand193 | —  | ??? |
| 194 | `—` | （无） | GameCompleteCommand | —  | PAL3A游戏结束的最后一个指令，标示游戏通关 |
| 195 | `—` | （无） | CombatEnterBossFightUsingMusicCommand | —  | 进入Boss战斗并使用指定音乐 |
| 196 | `—` | （无） | UnknownCommand196 | —  | ??? |
| 197 | `—` | （无） | UnknownCommand197 | —  | 设置风雅颂宿主角色ID？ |
| 198 | `—` | （无） | UnknownCommand198 | —  | ??? |
| 199 | `—` | （无） | NanGongHuangEnableTransformEffectCommand | —  | 开启南宫煌身体异常时（快要变身）的特效 |
| 200 | `—` | （无） | NanGongHuangDisableTransformEffectCommand | —  | 关闭南宫煌身体异常时（快要变身）的特效 |
| 201 | `00434ed0` | int int int int | ActorMoveOutOfScreenCommand | ✓ SceCommandRolePathOut | 使角色向指定位置走,直到走出屏幕（不一定走到指定的点）， |
| 202 | `004351a0` | int int | TeamAddOrRemoveActorCommand | ✓ SceCommandInTeam | 设置某个主角是否在队伍中出现， |
| 203 | `00434e50` | int int | ActorSetNavLayerCommand | ✓ SceCommandRoleSetLayer | 设置角色所在的地层， |
| 204 | `00437ac0` | int | ActorEnablePlayerControlCommand | ✓ SceCommandRoleCtrl | 设置玩家控制哪个主角， |
| 205 | `00437ae0` | int int | — | ✓ SceCommandRoleOverlap |  |
| 206 | `00434e90` | int float | ActorChangeScaleCommand | ✓ SceCommandRoleScale | 指定角色的缩放系数， |
| 207 | `004350f0` | int int | ActorAutoStandCommand | ✓ SceCommandRoleActAutoStand | 指定角色是否在PerformAction指令完成后自动切换成站立的动作， |
| 208 | `00436240` | int float | ActorMoveBackwardsCommand | ✓ SceCommandRoleMoveBack | 角色向后平移， |
| 209 | `004362d0` | int int | ActorLookAtActorCommand | ✓ SceCommandRoleFaceRole | 设定一个角色面向另一个角色（的位置）， |
| 210 | `00436130` | int int | ActorRotateFacingDirectionCommand | ✓ SceCommandRoleSetFace | 转动角色的方向， |
| 211 | `004365c0` | （无） | TeamOpenOnSceneChangeCommand | ✓ SceCommandTeamOpenOnSceneChange | 进入情节关时队伍散开 |
| 212 | `004365d0` | （无） | TeamCloseOnSceneChangeCommand | ✓ SceCommandTeamCloseOnSceneChange | 出情节关时队伍合并 |
| 213 | `004387d0` | int float float float | — | —  |  |
| 214 | `00435010` | int int int int | ActorMoveToCommand | ✓ SceCommandRoleMoveTo | 直线移动到TileMap上（X，Y），第四参数 0走/1跑/2站姿，见 `role_move.md`。原文「移动中使用指定 |
| 215 | `-` | （无） | — | —  |  |
| 216 | `—` | （无） | UnknownCommand216 | —  | ??? |
| 221 | `00435130` | int | ActorStopActionCommand | ✓ SceCommandRoleEndAction | ActorPerformActionCommand的循环次数为-1或-2时需要用 |
| 222 | `-` | （无） | — | —  |  |
| 250 | `00435380` | int | CameraFollowPlayerCommand | ✓ SceCommandCameraFollowPlayer | 镜头是否锁定并跟随在主角身上， |
| 251 | `00435350` | int | CameraFocusOnSceneObjectCommand | ✓ SceCommandCameraFocusObject | 设置摄像机锁定场景物品， |
| 252 | `-` | （无） | — | —  |  |
