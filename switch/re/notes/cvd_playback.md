# PAL3.exe v1.4 — 场景里 CVD 动画什么时候播

## 1. 引擎默认

`gbGeomNode` 载进 `.cvd` 后默认就在播：循环、倍率 1。游戏在建物件时按类型改两样东西：
`gbVisNode::SetTimeScale(t)`（IAT 0x56a5d0，t = 0 就是停住）和 `gbGeomNode::SetAnimLoop(n)`（IAT 0x56a32c，
n = 1 只播一遍）。CVD 的动画包括节点的位置/旋转/缩放关键帧和网格顶点帧，走的是同一个时间。

## 2. 物件：`FUN_00423a70` 建物件那一段（0x42405a 起）

按物件类型（运行时 `+0xf4`，scn 记录 +0x48）走两级跳表（字节表 0x4243b4 → 入口表 0x42439c）：

| 类型 | 入口 | 做什么 |
|---|---|---|
| 1、2、12、19、23、26、29、32、41 | 0x424073 | 模型是 CVD（`[+0x58] == 3`）就 `SetTimeScale(0)` + `[vtbl+0x18](0)`，停住 |
| 4、6、14 | 0x424175 | `SetTimeScale(0)`，停住（不看模型类型） |
| 17 | 0x42409f | 参数 p1（`+0xfc`，scn 记录 +0x50）非 0 且是 CVD 才停住；另外 `+0x44 = 0`、`+0x48 = rand % 10` |
| 13 | 0x4240f0 | 冲撞类：记下起点、按朝向算 280 长的位移、`+0x54 = rand % 300 × 常数` |
| 5 | 0x42418f | 移动浮板：分配 0x1c28 的路径表 |
| 其余 | — | 不动，照默认循环 |

之后（0x4242e4）：模型是 CVD 且类型是 4、6、12、14、16、19、23、26、32、41 的，`SetAnimLoop(1)`
改成只播一遍——这些都是等交互触发的（落物、陷阱、特技机关、门、调查点、吊桥、开关、宝箱……）。
其中只有 16 没被停住，所以调查点进场景会播一遍再停。

交互时（`FUN_00424c20`，也按 `+0xf4` 分支）：1 → `SetTimeScale(1)` 播一遍；2 → `SetAnimLoop(1)` +
`SetTimeScale(1)`；开关 23 和开门命令（87，`FUN_0042bea0`）都是 `SetTimeScale(2.0)`。

## 3. 场景自己的 CVD

`FUN_0041fe30` 载场景时，在地面 `%s\%s.pol`（夜里 `%s\1\%s.pol`）之后，找同目录的 `%s\%s.cvd`
（**只找白天那份**），存在就载进 `+0x78650`。`FUN_004200a0` 每帧画场景时先 `SetTimeScale(0.2)` 再画。
实机 1134 个场景里 22 个有：M01/1、M17 的十个、M19/4、M20 的五个、M24/2、m10/3、Q10、Q11、Q13、Q14。

## 4. 和 Pal3.Unity 的差别

Unity（`GameScene.cs` / `SceneObject.cs`）：场景 CVD 按 0.2 循环（和 exe 一致）；物件只有 `General`（25）循环，
`AutoTrigger`（0）没有触发区域时循环，`StaticOrAnimated`（17）`Parameters[1] == 0` 时每隔 0.5–3.5 秒随机播一遍、
`Parameters[2]` = 1 上下浮动 / 2 每秒转 80°。exe 是「默认都循环、少数类型停住」，比 Unity 宽；移植端照 exe。
17 的 `+0x48 = rand % 10` 用途没追到（可能就是 Unity 那个随机等待），移植端先按默认循环 [推断]；
Unity 的浮动 / 旋转在 exe 里没核对，没做。

## 5. 移植端

- `cvd_entity::scene_object_initial_play(类型, p1)` 按 §2 给出进场景时的播放方式；`play_cvd_subtree`
  把物件下所有 CVD 节点一起播。场景 CVD 在 `ScnScene::load_objects` 里按 §3 载、0.2 倍率循环。
- 节点关键帧（`CvdModelComponent`）和顶点帧（radiance `AnimatedMeshComponent`，新加时间倍率和 `stop`）
  由 `CvdModelComponent::play` 统一驱动；以前顶点帧不管类型一律一载入就循环，门、开关也在动。
- p1 在 `ScnNode` 里是 `ladder_nav_coord1.1`（参数六个 int 从记录 +0x4c 起，被读成两对坐标 + 一个层号）。
- **修掉的一个老 bug**：`crosscom/idl/openpal3.idl` 里组件类写成了 `class CvdModel: IComponent {}`，没实现
  `ICvdModel`，所以 `get_component(ICvdModel::uuid())` 拿到组件后 `query_interface::<ICvdModel>()` 永远是空。
  开门（87，`open_door.rs`）、交互（124，`interact.rs` 的 1/2/23 类）、flexeff 模型的循环（`flexeff_model.rs`）
  这几处从 c7ee54a / 7e778d8 起其实一直找不到 CVD 组件、什么也没播，也不报错。改成
  `class CvdModel: ICvdModel {}` 之后才真正生效；`tests/pal3_cvd_playback.rs` 拿 M01/1、Q10 的场景 CVD
  验「不叫不动、叫了会动」（M01/1 两个顶点帧在播，Q10 十二个节点里九个动了）。

