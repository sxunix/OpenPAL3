# pal3_drive

把仙剑 3 跑起来、驱动它、把每一步留下痕迹。给**回归**用：改完东西重跑一条
路线，`diff` 两份 `run.jsonl` 就知道有没有动到不该动的地方；截图留着出问题
时自己看。

和 `tools/pal4_planner` 的关系：那个是 PAL4 的，按剧情触发器做目标搜索，这里
用不上（PAL3 的剧情入口是 SCE proc，不是 EVF 触发器）。两边唯一能共用的
传输层也有一处不同——PAL3 的 `/v1/screenshot` 回的是**裸 PNG 字节**，
PAL4 那个 client 一律 `json.loads`，接不住。所以这里另起一份，很薄。

## 先把游戏起起来

桌面版（渲染正确性的基准，见记忆 `openpal3-switch-run-env`）：

```bash
export VK_ICD_FILENAMES=$(ls /opt/homebrew/Cellar/molten-vk/*/etc/vulkan/icd.d/MoltenVK_icd.json | head -1)
export DYLD_FALLBACK_LIBRARY_PATH=/opt/homebrew/opt/openal-soft/lib:/opt/homebrew/lib
cargo build --release -p yaobow
./target/release/yaobow --pal3 --agent-port 8777
```

资产路径取 `~/Library/Application Support/yaobow/yaobow.toml` 的
`game.pal3.asset_path`。`--agent-port` 不给就不开 agent 面。

## 跑一条路线

```bash
python3 tools/pal3_drive/route_prologue.py --out runs/prologue
python3 tools/pal3_drive/route_prologue.py --out runs/prologue --no-shots   # 只记状态，快
python3 tools/pal3_drive/route_combat.py   --out runs/combat               # 要先有活着的存档/剧情
python3 tools/pal3_drive/route_win.py      --out runs/win --monster 7306   # 从新游戏打赢一场看结算
python3 tools/pal3_drive/route_combo.py    --out runs/combo --combos 501,504  # 放合击看演出
python3 tools/pal3_drive/route_ui.py       --out runs/ui                   # 状态菜单八页 + 三家店；顺手存 4 号档
python3 tools/pal3_drive/route_minigames.py --out runs/mini --load 4       # 阵法 / 当铺投资 / 宿营
python3 tools/pal3_drive/route_pad.py      --out runs/pad --load 4         # 只用手柄键过鉴定和宿营
```

`route_pad.py` 模拟 **Switch 插底座、只有手柄**：只发 A / B / 十字键 / 肩键，
不发回车、Esc、数字键。读 imgui 输入的小游戏原版是 PC 纯鼠标，Switch 上手柄
只产生 imgui 的 `Gamepad*` 键，没有触屏时某一步不收手柄就是卡死——鉴定（第一章
主线）和宿营都是这么抓出来的。新加读 imgui 输入的界面，要在这条路线里过一遍。

`route_prologue` 开新游戏走到玩家可操作；`route_combat` 换场到有战场的地方
开一场战斗再打几下。后者的两个坑写在它自己的文件头里。

产物：

* `run.jsonl` —— 每行一步，**这是比对基准**。
* `NNN_标签.png` —— 对应那一步的画面，2560×1856 的真帧（Vulkan 读 swapchain
  回来的，不是系统截屏，所以不需要录屏权限、锁屏也能抓）。

比对：

```bash
diff <(jq -c 'del(.frames, .shot_bytes)' runs/prologue/run.jsonl) \
     <(jq -c 'del(.frames, .shot_bytes)' runs/prologue.base/run.jsonl)
```

`frames` 和 `shot_bytes` 跟机器快慢、驱动版本走，不进比对；`DIGEST_KEYS`
里那些（场景、块、脚本 proc、台词、队伍、钱、物品、坐标）才是。

## 自己写一条

```python
from pal3_drive import Client, Run

c = Client("http://127.0.0.1:8777")
run = Run(c, "runs/我的路线")

c.new_game()
c.wait(lambda s: s.get("movie_playing"));  c.key("Escape")
c.wait(lambda s: bool(s.get("scene")))
run.mark("进场")

c.fast_forward(True)      # 跳过片子和脚本等待
c.teleport(0, (x, y, z))
c.key("Up", "down"); ...; c.key("Up", "up")
run.mark("走到门口")

run.write()
```

`c.wait(pred, timeout)` 轮询状态直到条件成立，超时抛 `TimeoutError` 并把
最后一次状态摘要带出来——卡在哪一句台词一眼可见。

## 已知的坑

* **`--agent-port` 之外没有别的开关**；没带就没有 HTTP 面，`c.up()` 直接超时。
* `"Return"` 不是合法键名（`radiance::input::Key` 里没有），回车用 `"Space"`。
* 片头 Bink 要 `Escape` 跳；也可以开 `fast_forward` 让 `SceCommandMovie`
  整段跳过（`commands/movie.rs:35`）。
* 快照里 `quest_percentage` 还是 0 —— PAL3 侧没有这个量，没接。
* **合成按键也灌给 imgui**（`radiance::imgui::queue_synthetic_key`）。引擎键表
  没有回车，发 `"Enter"`；任意 imgui 键发 `"imgui:<名字>"`，比如
  `"imgui:GamepadL1"`。手柄键按**含义**映射：`GamePadEast` = A = 确定
  （imgui `GamepadFaceDown`），`GamePadSouth` = B = 取消（`GamepadFaceRight`），
  和 Switch 平台一致。
* 调试入口推进来的指令（商店、宿营、鉴定）不经过 proc，但跑着的时候快照的
  `script_running` 也是真，可以靠它等结束。
* **战斗里别盲按**。快照的 `combat` 字段给出 `turn_slot`（轮到谁）、`menu`
  （`main` / `target` / …）、`staging`（舞台在演出手）、各位的精、`outcome`。
  角色跑过去出手的那几秒里 Space 是「跳过演出」，盲按大半落空——头几场
  就是这么输的。`route_win.py` 是看着 `menu` 按键的写法。
* `prime_level` 只抬等级和上限，**不补现血**；`7368` 冰晶女（4 级）一招全体
  冰系就能把 1 级景天加两个没补血的 9 级打穿。想稳赢用 `7306` 小树妖
  （3 级、55 精）。
* **战斗舞台按「当前场景_块」查 `combat.ini`**；内景查不到就只有 HUD 没有
  战场（日志 `Combat: 没算出战场名，舞台不建`）。要看战斗先 `load_scene`
  换到有映射的地方，`m01/1 → z08` 是最早能用的一条。
* `script_eval` 一共 26 个调试入口：`call_proc` `set_global` `load_scene`
  `start_combat` `combat_combo` `team_add` `add_item` `add_money`
  `prime_level` `learn_skill` `lineup_set` `lineup_click` `bigmap`
  `hockshop` `dealer` `smelting` `dlg_sel` `note` `caption` `xjpic`
  `play_effect` `set_role_alpha` `object_transform` `set_water_magic`
  `bigmap_set` `game_over`。都要有活着的 `AdventureDirector`，没开新游戏
  返回 409。
