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
```

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
