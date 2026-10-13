#!/usr/bin/env python3
"""路线 03：打赢一场，看结算页和升级。

    python3 tools/pal3_drive/route_win.py --out runs/win

从新游戏开始（保证景天满血），快进序章 → 换到 m01/1 → 拉 1、2 号进队 →
`prime_level` 让景天差 1 点经验升 2 级、雪见龙葵预备到 `--level` 级 → 开一只怪
→ **看着快照里的 `combat.menu` 按键**（`main` 才按攻击、`target` 才按确认）
→ `combat.outcome` 一出来就停手连拍结算页 → 再逐页按过去。

不能盲按：角色跑过去出手的那几秒里 Space 是「跳过演出」，盲按大半落空，
真轮到我方时反而没按到——头两次就是这么输的。
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
    ap.add_argument("--out", default="runs/win")
    ap.add_argument("--monster", type=int, default=7368)
    ap.add_argument("--timeout", type=float, default=240, help="一场最多打多久（秒）")
    ap.add_argument("--level", type=int, default=10, help="雪见、龙葵预备到几级")
    args = ap.parse_args()

    c = Client(args.base)
    if not c.up(90):
        print("连不上 agent server", file=sys.stderr)
        return 2
    run = Run(c, args.out)

    # ---- 新游戏，快进序章 ----
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
    run.mark("prologue_done", shot=False)

    # ---- 换场、补队、预备升级 ----
    eval_(c, "load_scene", "m01", "1")
    c.wait(lambda s: s.get("scene", "").lower() == "m01", timeout=180)
    time.sleep(3)
    for actor in (1, 2):
        eval_(c, "team_add", actor)
    eval_(c, "prime_level", 0, 2)          # 景天差 1 点升 2 级
    for actor in (1, 2):
        eval_(c, "prime_level", actor, args.level)
    time.sleep(1)
    run.mark("ready")

    # ---- 开战，打到赢 ----
    eval_(c, "start_combat", 1, args.monster, 0, 0, 0, 0, 0)
    c.wait(lambda s: (s.get("combat") or {}).get("active"), timeout=30)
    time.sleep(2)
    run.mark("combat_open")

    # 看着快照按：菜单在 main 才按「攻击」，到 target 才按「确认」。
    # 胜负一出来（`combat.outcome`）就停手——结算页是在那之后才演的。
    deadline = time.time() + args.timeout
    turns = 0
    outcome = None
    while time.time() < deadline:
        cb = c.state().get("combat") or {}
        outcome = cb.get("outcome")
        if outcome:
            break
        menu = cb.get("menu")
        if menu == "main":
            c.key("Space")          # 命令盘中心＝攻击
            turns += 1
            if turns % 3 == 1:
                foes = [(u["slot"], u["hp"], u["max_hp"]) for u in cb.get("units", []) if u["enemy"]]
                print(f"  第 {turns} 次出手，敌方 {foes}", flush=True)
        elif menu == "target":
            c.key("Space")          # 默认目标
        time.sleep(0.15)
    run.mark(f"result_{outcome}")
    if outcome != "win":
        print(f"没打赢：{outcome}", file=sys.stderr)
        run.write()
        return 1

    # ---- 结算：先不按，连拍；再逐页按过去 ----
    for i in range(6):
        time.sleep(1.0)
        run.mark(f"settle_{i}")
    for page in range(10):
        c.key("Space")
        time.sleep(1.2)
        s = run.mark(f"page_{page}")
        if not s.get("script_running"):
            break
    time.sleep(1)
    run.mark("back_to_field")

    path = run.write()
    print(f"\n{len(run.steps)} 步 → {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
