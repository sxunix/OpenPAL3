#!/usr/bin/env python3
"""路线 06：阵法换位、当铺投资、宿营走一遍。

    python3 tools/pal3_drive/route_minigames.py --out runs/mini --load 4

要先有 `route_ui.py` 存的 4 号档（五人满队、2 万钱）。

* 阵法：开阵法页，`lineup_set` 把景天和雪见的五灵位对调，前后各拍一张。
* 当铺：开投资面板，右移 3 格（每格是上限的 1/20），确认——快照里的钱应当
  正好少掉投进去的数；再 Esc 退出。
* 宿营（106）：说明牌 → 过场框 → 转盘起转 → 点停 → 结果 → 面板 → 退出，
  每按一下 Space 拍一张，直到脚本不再跑。
"""

from __future__ import annotations

import argparse
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pal3_drive import Client, Run, boot  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default=os.environ.get("PAL3_AGENT", "http://127.0.0.1:8777"))
    ap.add_argument("--out", default="runs/mini")
    ap.add_argument("--load", type=int, default=4)
    ap.add_argument("--site", type=int, default=0, help="宿营方位号 0..3")
    args = ap.parse_args()

    c = Client(args.base)
    if not c.up(90):
        print("连不上 agent server", file=sys.stderr)
        return 2
    run = Run(c, args.out)
    boot(c, args.load)
    run.mark("field", shot=False)
    ok = True

    # ---- 阵法换位 ----
    c.status_menu(True, "lineup")
    time.sleep(2.0)
    run.mark("lineup_before")
    # 0 景天 ↔ 1 雪见。阵位号从 0 起（`PersistentState::set_lineup_position`，
    # < LINEUP_POSITIONS）；设某人到某位会把那一位原来的人挤掉，所以先把景天放
    # 到 1、再把被挤掉的雪见放回 0，就是对调。
    c.eval("lineup_set", 0, 1, 1, 0)
    time.sleep(2.0)
    run.mark("lineup_after")
    c.status_menu(False)
    time.sleep(1.0)

    # ---- 当铺投资 ----
    money0 = c.state().get("money", 0)
    c.eval("hockshop", 0)
    time.sleep(2.0)
    run.mark("hockshop_open")
    for _ in range(3):
        c.key("Right")
        time.sleep(0.3)
    run.mark("hockshop_amount")
    c.key("Space")
    time.sleep(1.5)
    s = run.mark("hockshop_invested")
    spent = money0 - s.get("money", 0)
    print(f"  当铺：投资前 {money0}，之后 {s.get('money')}，少了 {spent}", flush=True)
    if spent <= 0:
        print("  ✗ 投了钱但钱没少", file=sys.stderr)
        ok = False
    for _ in range(4):
        c.key("Escape")
        time.sleep(1.0)
        if not c.state().get("script_running"):
            break
    run.mark("hockshop_closed", shot=False)

    # ---- 宿营 ----
    c.eval("encamp", args.site)
    time.sleep(3.0)
    run.mark("encamp_open")
    # 宿营读的是 imgui 输入：说明牌、过场框、转盘「任意键」就走（Space），
    # 结果框认回车。两种交替按。
    for i in range(16):
        c.key("Space" if i % 2 == 0 else "Enter")
        time.sleep(1.6)
        s = run.mark(f"encamp_{i:02d}")
        if not s.get("script_running"):
            break
    else:
        print("  ✗ 宿营 16 下还没走完", file=sys.stderr)
        ok = False

    path = run.write()
    print(f"\n{len(run.steps)} 步 → {path}")
    return 0 if ok else 1


if __name__ == "__main__":
    raise SystemExit(main())
