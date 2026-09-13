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
| 17 | 0x42409f | 参数 p1（`+0xfc`，scn 记录 +0x50）非 0 且是 CVD 才停住；另外 `+0x44 = 0`、`+0x48 = (float)(rand % 10)`，见 §2.1 |
| 13 | 0x4240f0 | 冲撞类：记下起点、按朝向算 280 长的位移、`+0x54 = rand % 300 × 常数` |
| 5 | 0x42418f | 移动浮板：分配 0x1c28 的路径表 |
| 其余 | — | 不动，照默认循环 |

之后（0x4242e4）：模型是 CVD 且类型是 4、6、12、14、16、19、23、26、32、41 的，`SetAnimLoop(1)`
改成只播一遍——这些都是等交互触发的（落物、陷阱、特技机关、门、调查点、吊桥、开关、宝箱……）。
其中只有 16 没被停住，所以调查点进场景会播一遍再停。

### 2.1 类型 17 每帧（`FUN_00426eb0` 的 case 0x11）

    if (p1(+0xfc) == 0 && 模型是 CVD) {
      if (+0x51 == 0) { +0x44 += dt; if (+0x48 <= +0x44) { +0x48 = rand%10 + 1; +0x44 = 0; +0x51 = 1; SetTimeScale(2.0); } }
      else if (+0x51 == 1) { SetTimeScale(2.0); if (FUN_0042b1f0()) { +0x51 = 0; [vtbl+0x18](0); SetTimeScale(0); } }
    }
    if (p2(+0x100) == 1) { +0x54 += dt; y(+0x2c) = sin(+0x54) × 6 + 原高度(+0xd8); }
    else if (p2 == 2)    { 朝向(+0xe0) += dt × 80; }        // 度

`FUN_0042b1f0` 在节点的消息队列里找「播完」那条（按 CRC 比），类型 1、3 的节点才找。p2 在 scn 记录 +0x54
（`ScnNode::ladder_nav_coord2.0`）。

意思是：建好时还是默认的循环、倍率 1；等 0–9 秒（`rand % 10`）后改成 2 倍，等手上这一轮播完就回第 0 帧停住；
之后每隔 1–10 秒整从头按 2 倍播一遍 [推断：循环模式下每轮结束也发「播完」消息，`[vtbl+0x18](0)` 是回到第 0 帧]。
浮动和旋转和模型类型、p1 都无关，POL 物件也会动。

实机分布（全部 1134 个场景）：非 `+` 特效的类型 17 物件 243 个——CVD p1=0 的 140 个（11 个同时浮动），
CVD p1=1 的 51 个；p2=1 浮动的共 23 个（Q04/n01 的六个 `_01.cvd`、Q02/Q03/Q05 的 `jq38`/`jq40` 等），
p2=2 旋转的 28 个全在 m11/1（`_h2.pol`）。`+` 特效节点 173 个，p1、p2 都是 0。

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
17 的随机等待 exe 是 0–9 秒（第一次）/ 1–10 秒整（之后），播放倍率 2，Unity 的 0.5–3.5 秒是自己定的；
浮动（振幅 6、`sin(t)`）和旋转（80°/秒）与 Unity 一致（§2.1）。

## 5. 移植端

- `cvd_entity::scene_object_initial_play(类型, p1)` 按 §2 给出进场景时的播放方式；`play_cvd_subtree`
  把物件下所有 CVD 节点一起播。场景 CVD 在 `ScnScene::load_objects` 里按 §3 载、0.2 倍率循环。
- 节点关键帧（`CvdModelComponent`）和顶点帧（radiance `AnimatedMeshComponent`，新加时间倍率和 `stop`）
  由 `CvdModelComponent::play` 统一驱动；以前顶点帧不管类型一律一载入就循环，门、开关也在动。
- p1 在 `ScnNode` 里是 `ladder_nav_coord1.1`、p2 是 `ladder_nav_coord2.0`（参数六个 int 从记录 +0x4c 起，
  被读成两对坐标 + 一个层号）。
- 类型 17（§2.1）：`ScnScene::register_object_motion` 建物件时登记，`step_scene_objects` 每帧推
  （`adv_director` 里和 opcode 90 的滑动放在一起，过场时也走）。重播的计时是 `cvd_entity::IdleReplay`，
  到点调 `CvdModelComponent::finish_cycle(2.0)`（不回头、改成播完这一轮就停），播完调 `rewind`
  （回第 0 帧；顶点帧用 radiance 新加的 `AnimatedMeshComponent::rewind`）。随机数用单独一条 `CrtRand`，按时间播种
  （原版和别处共用 CRT `rand()`）。浮动只改 y，x、z 取当前值，所以 opcode 90 挪过的物件也照样浮。
- 上机核对：agent 加了 `object_transform [id]`（物件和子节点的局部矩阵）。Q04/n01 物件 3、4 的根 y 在
  42.6–54.5 之间变（原高度 48.58 ± 6），子节点在播放位和第 0 帧之间来回，两个物件时刻错开；m11/1 物件 4、5
  每 1.5 秒转约 122°（≈80°/秒）。画面上没对着拍到（镜头看不到这些物件），和原版的观感没对过。
- **修掉的一个老 bug**：`crosscom/idl/openpal3.idl` 里组件类写成了 `class CvdModel: IComponent {}`，没实现
  `ICvdModel`，所以 `get_component(ICvdModel::uuid())` 拿到组件后 `query_interface::<ICvdModel>()` 永远是空。
  开门（87，`open_door.rs`）、交互（124，`interact.rs` 的 1/2/23 类）、flexeff 模型的循环（`flexeff_model.rs`）
  这几处从 c7ee54a / 7e778d8 起其实一直找不到 CVD 组件、什么也没播，也不报错。改成
  `class CvdModel: ICvdModel {}` 之后才真正生效；`tests/pal3_cvd_playback.rs` 拿 M01/1、Q10 的场景 CVD
  验「不叫不动、叫了会动」（M01/1 两个顶点帧在播，Q10 十二个节点里九个动了）。


## 6. 战斗选目标的 3D 指针（`ui\combat\pointer\pointer.cvd`）

exe 里载了两份：
- 舞台相机对象 `+0x69090`（`FUN_005060d0`）：只给镜头编辑器用——`FUN_00505930` 画它的包围盒线框、
  `FUN_00506020` 拿射线点它，正常游戏不画。
- 舞台场景对象 `+0x268`（`FUN_00510540`）：这才是选目标时看到的那个。播放倍率跟战斗倍率走
  （`FUN_00513600` 设全场时，顺带 `SetTimeScale` 到它身上），自己的 CVD 动画一直循环。

画在 `FUN_004fc3a0`（画舞台那一帧）：对五个位（`in_ECX[6..10]`）里 `+0x118` 非 0（被选为目标）的每一个：

    p = FUN_0050a9b0()：角色 OBB 的中心，y 改成 中心 y + 身高(+0x16c)/2 + 10；
        战斗 id 是 0x2c3（707）的再 −80
    w = sin(战斗时间(+0x50) × 10 + i)
    Push; Translate(p.x, p.y + 6 + 5w, p.z); Scale(1, 1 + 0.2w, 1); 画指针; Pop

身高 `+0x16c` 在 `FUN_004ed680` 里和 `+0x164/+0x168` 一起从模型包围盒尺寸抄来 [推断：三个一组]。

移植端（`combat.rs` 的 `place_pointer`）：开战时和舞台一起载，平时藏着；轮到我方选目标时摆到当前目标头顶。
包围盒用角色渲染对象的 `local_aabb` 并起来近似 OBB；i 取位号 mod 5 [推断：那五个位是哪一边没追]；
原来「在招式横幅上写目标名」的临时做法去掉了。M01/1（战场 z08）实机截图看到指针在敌人头顶上下跳；
和原版截图的大小、颜色还没对过。

阵法页的 `ui\GameMainUI\Lineup\pointer.cvd`（`FUN_00483650`）也已做，见 `lineup_page_layout.md` §3。

## 7. 贴图的行序

gbengine 不用 D3DX 读图，TGA 由自己的 `gbImage2D::LoadImageFile`（0x1001e490，按扩展名 `.tga` 分到
0x1001ed50，`.bmp` 分到 0x1001eb80）读。读像素那段按 `descriptor & 0x30` 查跳转表（0x1001ef84 / 0x1001ef74）：

| descriptor & 0x30 | 走哪支 | 结果 |
|---|---|---|
| 0x00（左下原点） | 0x1001eef7：整块一次读进来 | 文件第一行（图的最下一行）在前 |
| 0x20（左上原点） | 0x1001ef0b：从最后一行往前逐行读 | 同样是最下一行在前 |
| 0x10 / 0x30 | 0x1001eed0 | 读失败 |

所以不管文件怎么存，内存里总是**最下一行在前**，D3D 的 v = 0 取的就是图的底边。零售 `.dds` 本来就是自下而上存的，
原样读也一样。Pal3.Unity 的 TGA 读取同样不看原点、按文件行序拷进 Unity（Unity 的 v = 0 在底边），uv 原样用。

移植端的 image crate 解 TGA 会按原点把图摆正（上行在前），和 `.dds` 反着。MV3 角色在 `role_controller.rs`
里把 v 取负补回来；CVD 以前没补，贴图只有 `.tga`（没有同名 `.dds`）的 CVD 就上下颠倒——侠义榜的书
（`close.CVD` 封面 `a.tga`，封面节点 `scale_factor = −1`）画出来「降妖谱」三个字是倒的。现在
`Pal3PropMaterialDef`（只有 CVD 用）解出非 `.dds` 的图后整张 flipv，截图看封面已经正了。
单贴图 POL 的 `Pal3GeomMaterialDef` 也改成一样（阵法页的 `wl.pol` 由此对上，见 `lineup_page_layout.md` §3.3）。
带光照图的 POL（`LightMapMaterialDef`，PAL4 的 dff 也用）现在走单独的 `create_pal3`，光照图和漫反射贴图都用
`decode_pal3_texture`，别的游戏不受影响：

- 光照图 `^L_*.bmp`（Q01 里 467 张）是 24 位、高度为正（自下而上存）。gbengine 的 BMP 读取（0x1001eb80）把
  `文件大小 − 数据偏移` 这一整块像素原样读进 `+0xb8`，不调行序，所以内存里也是最下一行在前；Pal3.Unity 的
  `BmpTextureLoader` 同样只翻高度为负的。image crate 会摆正，所以以前端口的光照图是上下颠倒的。
  `decode_pal3_texture` 现在对高度为负的 BMP 不翻（gbengine 原样读出来本来就是上行在前）。
- **顺带修掉一个更大的问题**：光照图材质以前用 `image::load_from_memory` 猜格式，TGA 没有魔数猜不出来，
  贴图直接载入失败、退成「MISSING TEXTURE」占位图。Q01/YN14 这种漫反射贴图是 `.tga` 的场景以前满屏占位图，
  改后柱子、墙、天花板都正常了（改前 / 改后截图对比）。`decode_pal3_texture` 猜不出时会按 TGA 再解。
- 场景贴图绝大多数是 `.dds`（Q01 有 20677 张，`.tga` 只有 34 张），DDS 本来就不翻，不受影响；Q01/XN05 改前改后画面一样。
- 光照图翻过来以后明暗落的位置对不对，没有原版截图对照，**没有画面核对**，只有 gbengine 和 Pal3.Unity 两边的读法作依据。

## 8. 界面上的 3D 视图

原版视图控件（构造 `FUN_00439030`，虚表 0x56ac20）自带一台正交 `gbCamera`，`FUN_0043a070(s)` 设半宽，
`FUN_00439230` 按扩展名载 `pol` / `cvd` / 角色。相机两种放法：

    dx = −(2x/800 − 1)·s        fy = (1 − 2y/600)·s
    FUN_00439850(x, y)     正面：eye (dx, −fy, 1600) → (dx, −fy, 0)，up (0,1,0)
    FUN_00439930(x, y, h)  俯视：eye (dx, h − fy, 1600) → (dx, −fy, 0)，up (0,1,0)

`FUN_0043a0e0(deg)` 写 `+0xf8`，是模型绕竖轴转的角度。gbengine 和 radiance 都是右手系（Pal3.Unity 把 GameBox
坐标 x 取反才进 Unity），照抄就行，不用额外镜像。

移植端在 `openpal3/model_view.rs`：一个只放这一个模型的离屏场景，每帧画进渲染目标（背景清成透明），
正交用窄视角透视近似 [近似]。物品预览（`item_preview.rs`）和 p7 菜单（`IPal3Service::draw_model_view`）共用；
侠义榜的书和怪物走后者。
