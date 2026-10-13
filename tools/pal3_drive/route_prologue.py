#!/usr/bin/env python3
"""路线 01：开新游戏 → 跳过片头 → 走完「雪见登场」序章。

    python3 tools/pal3_drive/route_prologue.py --out runs/prologue

先把游戏起起来（见 README）。跑完 `runs/<名字>/run.jsonl` 就是基线，
下次改了东西再跑一遍 `diff` 即可。
"""

from __future__ import annotations

import argparse
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pal3_drive import Client, Run  # noqa: E402

#: 序章这一段全是台词，没有分支；推不动就是卡住了。
MAX_ADVANCE = 400


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default=os.environ.get("PAL3_AGENT", "http://127.0.0.1:8777"))
    ap.add_argument("--out", default="runs/prologue")
    ap.add_argument("--no-shots", action="store_true", help="只记状态，不截图（快）")
    ap.add_argument("--every", type=int, default=25, help="每多少句台词截一张")
    args = ap.parse_args()

    c = Client(args.base)
    if not c.up():
        print("连不上 agent server，游戏起了吗？", file=sys.stderr)
        return 2

    run = Run(c, args.out, shots=not args.no_shots)
    run.mark("title")

    c.new_game()
    # 片头是 Bink，按 Esc 跳。`movie_playing` 为真才说明真在放；
    # 脚本一进 init 就先排 movie，所以等它起来再跳。
    try:
        c.wait(lambda s: s.get("movie_playing"), timeout=30)
        run.mark("movie")
        c.key("Escape")
    except TimeoutError:
        print("没等到片头（movies 关了？），继续", flush=True)
    s = c.wait(lambda s: bool(s.get("scene")), timeout=60)
    run.mark(f"scene_{s['scene']}_{s['block']}")

    seen: set[str] = set()
    last_fn = s.get("current_script_fn")
    for i in range(MAX_ADVANCE):
        c.dialog_advance()
        time.sleep(0.45)
        s = c.state()
        fn = s.get("current_script_fn")
        if fn != last_fn:
            run.mark(f"proc_{fn}")
            last_fn = fn
        dlg = (s.get("dialog") or {})
        if dlg.get("open") and dlg.get("text") and dlg["text"] not in seen:
            seen.add(dlg["text"])
            if len(seen) % args.every == 0:
                run.mark(f"dlg_{len(seen)}")
        if not s.get("script_running"):
            run.mark("player_control")
            break
    else:
        run.mark("stuck")

    path = run.write()
    print(f"\n{len(run.steps)} 步，{len(seen)} 句台词 → {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
