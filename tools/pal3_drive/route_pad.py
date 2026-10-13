#!/usr/bin/env python3
"""路线 07：只用手柄键过那几个读 imgui 输入的小游戏——Switch 插底座时的玩法。

    python3 tools/pal3_drive/route_pad.py --out runs/pad --load 4

只发手柄键（`GamePadEast` = A 确定、`GamePadSouth` = B 取消、十字键、
`imgui:GamepadL1` / `imgui:GamepadR1` 肩键），**不发**回车、Esc、空格、数字键，
模拟没有触屏、只有手柄的情形。每个小游戏都得能走完或退出，不然就是卡死。

* 鉴定（104）：询问框 A → 展示页十字键翻两件、B 回总览 → A 开始鉴定 →
  每件十字键选「真」、A 确定、R 翻下一件，五件做完 → 退出。
* 宿营（106）：A 连按走完。
"""

from __future__ import annotations

import argparse
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pal3_drive import Client, Run, boot  # noqa: E402

A, B = "GamePadEast", "GamePadSouth"
LEFT, RIGHT = "GamePadDPadLeft", "GamePadDPadRight"
L1, R1 = "imgui:GamepadL1", "imgui:GamepadR1"


def press(c: Client, key: str, wait: float = 0.8) -> None:
    c.key(key)
    time.sleep(wait)


def finish(c: Client, run: Run, name: str, key: str, limit: int) -> bool:
    """连按 `key` 直到指令结束；超过 `limit` 下还没完就算卡住。"""
    for i in range(limit):
        if not c.state().get("script_running"):
            run.mark(f"{name}_done")
            return True
        press(c, key, 1.4)
        if i % 3 == 0:
            run.mark(f"{name}_{i:02d}")
    run.mark(f"{name}_stuck")
    return False


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default=os.environ.get("PAL3_AGENT", "http://127.0.0.1:8777"))
    ap.add_argument("--out", default="runs/pad")
    ap.add_argument("--load", type=int, default=4)
    args = ap.parse_args()

    c = Client(args.base)
    if not c.up(90):
        print("连不上 agent server", file=sys.stderr)
        return 2
    run = Run(c, args.out)
    boot(c, args.load)
    ok = True

    # ---- 鉴定 ----
    c.eval("appraise")
    time.sleep(2.0)
    run.mark("appr_ask")
    press(c, A, 2.5)                     # 是
    run.mark("appr_exhibit")
    press(c, RIGHT, 1.2)                 # 放大第一件
    run.mark("appr_zoom0")
    press(c, RIGHT, 1.2)                 # 翻到第二件
    run.mark("appr_zoom1")
    press(c, B, 1.2)                     # 回总览
    run.mark("appr_overview")
    press(c, A, 2.5)                     # 开始鉴定
    run.mark("appr_do")
    for k in range(5):
        press(c, LEFT, 0.5)              # 选「真」
        press(c, A, 2.0)                 # 确定
        run.mark(f"appr_judged{k}")
        if not c.state().get("script_running"):
            break
        press(c, R1, 1.0)                # 下一件
    ok &= finish(c, run, "appr_exit", A, 12)

    # ---- 宿营 ----
    c.eval("encamp", 0)
    time.sleep(3.0)
    run.mark("encamp_open")
    ok &= finish(c, run, "encamp", A, 16)

    path = run.write()
    print(f"\n{len(run.steps)} 步 → {path}  {'全部走完' if ok else '有卡住的'}")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
