#!/usr/bin/env python3
"""路线 04：放合击技，看演出。

    python3 tools/pal3_drive/route_combo.py --out runs/combo --combos 501,504

新游戏 → 换到 m01/1 → 拉雪见(1)、龙葵(2)进队 → 开几只小树妖 → 每轮到我方
选行动时调 `combat_combo <id>`（`CombatSession::debug_combo`：不看条件，打第一
个还站着的敌人）→ 放出去之后每 0.5 秒拍一张，拍满一段演出。

我方三人是 101 景天 / 104 雪见 / 106 龙葵，能用的合击：501 连环破斩（景天+雪见）、
502 魔剑飞仙（景天+龙葵）、504 断空噬地 / 505 灵奴奈落 / 506 飞鸟裂天（三人）。
"""

from __future__ import annotations

import argparse
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pal3_drive import Client, Run  # noqa: E402


def eval_(c: Client, fn: str, *args):
    return c._json("POST", "/v1/script/eval", {"function": fn, "args": list(args)},
                   timeout=180)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default=os.environ.get("PAL3_AGENT", "http://127.0.0.1:8777"))
    ap.add_argument("--out", default="runs/combo")
    ap.add_argument("--combos", default="501,504")
    ap.add_argument("--monster", type=int, default=7306)
    ap.add_argument("--frames", type=int, default=16, help="每招拍几张（0.5 秒一张）")
    args = ap.parse_args()
    combos = [int(x) for x in args.combos.split(",")]

    c = Client(args.base)
    if not c.up(90):
        print("连不上 agent server", file=sys.stderr)
        return 2
    run = Run(c, args.out)

    c.new_game()
    c.fast_forward(True)
    c.wait(lambda s: bool(s.get("scene")), timeout=120)
    for _ in range(500):
        c.dialog_advance()
        time.sleep(0.12)
        if not c.state().get("script_running"):
            break
    c.fast_forward(False)
    time.sleep(1)

    eval_(c, "load_scene", "m01", "1")
    c.wait(lambda s: s.get("scene", "").lower() == "m01", timeout=180)
    time.sleep(3)
    for actor in (1, 2):
        eval_(c, "team_add", actor)
    run.mark("ready", shot=False)

    eval_(c, "start_combat", len(combos), args.monster, 0, 0, 0, 0, 0)
    c.wait(lambda s: (s.get("combat") or {}).get("active"), timeout=30)
    time.sleep(2)
    run.mark("combat_open")

    for cid in combos:
        # 等轮到我方选行动
        try:
            c.wait(lambda s: (s.get("combat") or {}).get("menu") == "main"
                   or (s.get("combat") or {}).get("outcome"), timeout=60, poll=0.1)
        except TimeoutError:
            print(f"等不到我方回合，跳过 {cid}", file=sys.stderr)
            continue
        if (c.state().get("combat") or {}).get("outcome"):
            break
        eval_(c, "combat_combo", cid)
        for i in range(args.frames):
            time.sleep(0.5)
            run.mark(f"combo{cid}_{i:02d}")

    # 收尾：还没分出胜负就把剩下的打完
    deadline = time.time() + 120
    while time.time() < deadline:
        cb = c.state().get("combat") or {}
        if cb.get("outcome"):
            break
        if cb.get("menu") in ("main", "target"):
            c.key("Space")
        time.sleep(0.15)
    run.mark(f"result_{(c.state().get('combat') or {}).get('outcome')}")
    path = run.write()
    print(f"\n{len(run.steps)} 步 → {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
