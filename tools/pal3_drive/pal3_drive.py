"""PAL3 agent-server 驱动：跑一条固定路线，留下可比对的痕迹。

用法见同目录 README.md。核心是两件事：

* [`Client`] —— `yaobow --pal3 --agent-port <port>` 那个 HTTP 面的瘦客户端。
  和 `tools/pal4_planner` 里那个不同的地方只有一处：`/v1/screenshot`
  回的是**裸 PNG 字节**不是 JSON，所以传输层要分流。
* [`Run`] —— 把每一步的状态摘要写成 `run.jsonl`、截图写成 `NN_标签.png`。
  回归比的是 jsonl（文本，diff 得动），截图留着出问题时自己看。
"""

from __future__ import annotations

import json
import os
import time
import urllib.error
import urllib.request
from dataclasses import dataclass, field
from typing import Any, Callable, Optional

DEFAULT_BASE = os.environ.get("PAL3_AGENT", "http://127.0.0.1:8777")


class AgentError(Exception):
    def __init__(self, status: int, kind: str, message: str):
        super().__init__(f"{status} {kind}: {message}")
        self.status = status
        self.kind = kind
        self.message = message


class Client:
    def __init__(self, base_url: str = DEFAULT_BASE, timeout: float = 30.0,
                 token: Optional[str] = None):
        self.base_url = base_url.rstrip("/")
        self.timeout = timeout
        self.token = token

    # ---- 传输 ---------------------------------------------------------
    def _raw(self, method: str, path: str, body: Any = None,
             timeout: Optional[float] = None) -> tuple[str, bytes]:
        data = json.dumps(body).encode() if body is not None else None
        headers = {}
        if data:
            headers["Content-Type"] = "application/json"
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        req = urllib.request.Request(self.base_url + path, method=method,
                                     data=data, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=timeout or self.timeout) as r:
                return r.headers.get("Content-Type", ""), r.read()
        except urllib.error.HTTPError as e:
            raw = e.read().decode("utf-8", "replace")
            kind, msg = "internal", raw
            try:
                env = json.loads(raw)
                if env.get("type") == "error":
                    d = env.get("data") or {}
                    kind, msg = d.get("kind", kind), d.get("message", raw)
            except json.JSONDecodeError:
                pass
            raise AgentError(e.code, kind, msg) from None

    def _json(self, method: str, path: str, body: Any = None, **kw) -> Any:
        _, raw = self._raw(method, path, body, **kw)
        return json.loads(raw) if raw else None

    # ---- 观察 ---------------------------------------------------------
    def state(self) -> dict:
        return (self._json("GET", "/v1/state") or {}).get("data", {})

    def log_tail(self, after_seq: int = 0, n: int = 256) -> dict:
        env = self._json("GET", f"/v1/log/tail?after_seq={after_seq}&n={n}")
        return (env or {}).get("data", {})

    def globals(self) -> dict[int, int]:
        """SCE 全局变量（剧情旗标）：{变量号: 值}。PAL3 的剧情变量号多是负的（主线
        进度是 -32768），服务端放在 `named` 里，名字就是变量号。"""
        env = self._json("GET", "/v1/script/globals?start=0&limit=1")
        named = (env or {}).get("data", {}).get("named", [])
        return {int(g["name"]): g["value"] for g in named}

    def triggers(self) -> list[dict]:
        """当前场景的触发点：`nav:<号>` 走进去就触发，`aabb:` / `item:` 要按调查键。"""
        env = self._json("GET", "/v1/scene/triggers")
        return (env or {}).get("data", {}).get("triggers", [])

    def roles(self) -> list[dict]:
        """当前场景的人物：`role:<号>:<名字>`，野怪是 `monster:`。"""
        env = self._json("GET", "/v1/scene/objects")
        return (env or {}).get("data", {}).get("npcs", [])

    def fire(self, name: str) -> None:
        """走引擎自己的触发路径：传送过去，该按调查键的按一下。"""
        self._json("POST", "/v1/scene/fire_trigger", {"name": name})

    def screenshot(self, path: Optional[str] = None) -> bytes:
        """`/v1/screenshot` 回裸 PNG（`Content-Type: image/png`）。"""
        ctype, blob = self._raw("GET", "/v1/screenshot", timeout=90.0)
        if "json" in ctype:            # 万一哪天改成 JSON 包 base64
            import base64
            body = json.loads(blob).get("data", {})
            rgba = body.get("rgba")
            blob = base64.b64decode(rgba) if isinstance(rgba, str) else bytes(rgba or [])
        if path and blob:
            os.makedirs(os.path.dirname(path) or ".", exist_ok=True)
            with open(path, "wb") as f:
                f.write(blob)
        return blob

    # ---- 驱动 ---------------------------------------------------------
    def key(self, name: str, action: str = "tap") -> None:
        self._json("POST", "/v1/input/key", {"key": name, "action": action})

    def axis(self, name: str, value: float) -> None:
        self._json("POST", "/v1/input/axis", {"axis": name, "value": value})

    def dialog_advance(self) -> None:
        self._json("POST", "/v1/dialog/advance", {})

    def fast_forward(self, on: bool) -> None:
        self._json("POST", "/v1/time/fast_forward", {"on": on})

    def pause(self) -> None:
        self._json("POST", "/v1/time/pause", {})

    def resume(self) -> None:
        self._json("POST", "/v1/time/resume", {})

    def step(self, frames: int, dt: Optional[float] = None) -> None:
        body: dict[str, Any] = {"frames": frames}
        if dt is not None:
            body["dt"] = dt
        self._json("POST", "/v1/time/step", body)

    def new_game(self) -> None:
        self._json("POST", "/v1/menu/new_game", {}, timeout=180.0)

    def save(self, slot: int) -> None:
        self._json("POST", "/v1/save", {"slot": slot})

    def load(self, slot: int) -> None:
        self._json("POST", "/v1/load", {"slot": slot}, timeout=180.0)

    def teleport(self, player: int, pos: tuple[float, float, float]) -> None:
        self._json("POST", "/v1/player/teleport",
                   {"player": player, "x": pos[0], "y": pos[1], "z": pos[2]})

    def eval(self, fn: str, *args) -> Any:
        """`script_eval` 的 26 个调试入口之一（见 README）。"""
        return self._json("POST", "/v1/script/eval", {"function": fn, "args": list(args)},
                          timeout=180.0)

    def status_menu(self, open_: bool, page: Optional[str] = None) -> None:
        """状态菜单：page 是 state / prop / equip / magic / lineup / sword / book / system。"""
        body: dict[str, Any] = {"open": open_}
        if page:
            body["page"] = page
        self._json("POST", "/v1/menu/status", body)

    # ---- 等待 ---------------------------------------------------------
    def wait(self, pred: Callable[[dict], bool], timeout: float = 60.0,
             poll: float = 0.25) -> dict:
        """轮询状态直到 `pred` 为真；超时抛 TimeoutError，带最后一次状态。"""
        deadline = time.time() + timeout
        s: dict = {}
        while time.time() < deadline:
            s = self.state()
            if pred(s):
                return s
            time.sleep(poll)
        raise TimeoutError(f"等不到条件，最后状态：{digest(s)}")

    def up(self, timeout: float = 60.0) -> bool:
        """等服务起来。"""
        deadline = time.time() + timeout
        while time.time() < deadline:
            try:
                self._json("GET", "/v1/state", timeout=3.0)
                return True
            except Exception:
                time.sleep(0.5)
        return False


# ---- 痕迹 ---------------------------------------------------------------

#: 进 `run.jsonl` 的字段。**不含** `frame` / `fps` / `dt`：它们跟机器快慢走，
#: 放进去回归永远不相等。帧数单独记在 `frames` 里，只看不比。
DIGEST_KEYS = ("scene", "block", "leader", "script_running", "current_script_fn",
               "movie_playing", "money")


def digest(s: dict) -> dict:
    d = {k: s.get(k) for k in DIGEST_KEYS}
    dlg = s.get("dialog") or {}
    d["dialog"] = dlg.get("text", "") if dlg.get("open") else None
    d["avatar"] = dlg.get("avatar") or None
    d["party"] = [[m["slot"], m["level"], m["hp"], m["max_hp"], m["in_team"]]
                  for m in s.get("party") or []]
    d["items"] = [[e["id"], e["count"]] for e in s.get("inventory") or []]
    pos = s.get("leader_pos") or [0, 0, 0]
    d["pos"] = [round(v, 1) for v in pos]
    return d


@dataclass
class Run:
    """一次回放。`out_dir/run.jsonl` 是比对基准，截图只作旁证。"""
    client: Client
    out_dir: str
    shots: bool = True
    steps: list[dict] = field(default_factory=list)
    _n: int = 0

    def __post_init__(self) -> None:
        os.makedirs(self.out_dir, exist_ok=True)

    def mark(self, label: str, shot: Optional[bool] = None) -> dict:
        """记一步：状态摘要 +（可选）截图。"""
        s = self.client.state()
        row = {"i": self._n, "label": label, "frames": s.get("frame"), **digest(s)}
        if (self.shots if shot is None else shot):
            name = f"{self._n:03d}_{label}.png"
            blob = self.client.screenshot(os.path.join(self.out_dir, name))
            row["shot"] = name if blob else None
            row["shot_bytes"] = len(blob)
        self.steps.append(row)
        self._n += 1
        print(json.dumps(row, ensure_ascii=False), flush=True)
        return row

    def write(self) -> str:
        path = os.path.join(self.out_dir, "run.jsonl")
        with open(path, "w", encoding="utf-8") as f:
            for row in self.steps:
                f.write(json.dumps(row, ensure_ascii=False) + "\n")
        return path


def boot(c: Client, load_slot: Optional[int] = None) -> dict:
    """进到能自由操作的状态。给了 `load_slot` 就读档，否则开新游戏、快进跳片头、
    逐句推完序章（快进下 `SceCommandMovie` 整段跳过，台词仍要推）。"""
    if load_slot is not None:
        c.load(load_slot)
        c.wait(lambda s: bool(s.get("scene")) and not s.get("script_running"), timeout=180)
        time.sleep(1.5)
        return c.state()
    c.new_game()
    c.fast_forward(True)
    c.wait(lambda s: bool(s.get("scene")), timeout=120)
    for _ in range(600):
        c.dialog_advance()
        time.sleep(0.12)
        if not c.state().get("script_running"):
            break
    c.fast_forward(False)
    time.sleep(1.0)
    return c.state()
