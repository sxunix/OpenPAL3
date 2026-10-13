#!/usr/bin/env python3
"""路线 02：换到有战场的场景，开一场战斗，打几下。

    python3 tools/pal3_drive/route_combat.py --out runs/combat

跑这条之前要先有个存档或者先跑 `route_prologue.py`——`script_eval` 的
调试入口都要求有个活着的 `AdventureDirector`（没开新游戏时返回 409）。

两处踩过的坑，写在这儿省得下次再踩：

* **舞台按「当前场景_块」查 `combat.ini`**（`CombatData::combat_scene`，
  键是 `{场景}_{块}` 小写）。内景查不到就只建 HUD 不建战场，日志里是
  `Combat: 没算出战场名，舞台不建`——那是对的行为，不是渲染坏了。
  发行数据里有 105 条映射，`m01_1 → z08` 是最早能用的一条。
* **`start_combat` 普通那一支第一个参数是怪的只数，不是 id**
  （`SceCommandCombat::new(count, m1, m2, m3)`）；第 7 位要显式给 0，
  不给默认走 boss 那一支。
"""

from __future__ import annotations

import argparse
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pal3_drive import Client, Run  # noqa: E402


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default=os.environ.get("PAL3_AGENT", "http://127.0.0.1:8777"))
    ap.add_argument("--out", default="runs/combat")
    ap.add_argument("--scene", default="m01", help="换到哪个场景（要在 combat.ini 里有战场）")
    ap.add_argument("--block", default="1")
    ap.add_argument("--count", type=int, default=2, help="怪的只数")
    ap.add_argument("--monsters", default="7368,7370,0", help="随机池里的三个 id")
    ap.add_argument("--swings", type=int, default=6, help="打几下")
    args = ap.parse_args()

    c = Client(args.base)
    if not c.up():
        print("连不上 agent server，游戏起了吗？", file=sys.stderr)
        return 2
    run = Run(c, args.out)

    c._json("POST", "/v1/script/eval",
            {"function": "load_scene", "args": [args.scene, args.block]}, timeout=180)
    c.wait(lambda s: s.get("scene", "").lower() == args.scene.lower(), timeout=180)
    time.sleep(3)
    run.mark(f"{args.scene}_{args.block}")

    # 一个人打不动，把 1 号 2 号拉进队。
    for actor in (1, 2):
        c._json("POST", "/v1/script/eval", {"function": "team_add", "args": [actor]})
    time.sleep(1)
    run.mark("team")

    m = [int(x) for x in args.monsters.split(",")]
    while len(m) < 3:
        m.append(0)
    c._json("POST", "/v1/script/eval",
            {"function": "start_combat", "args": [args.count, m[0], m[1], m[2], 0, 0, 0]})
    time.sleep(5)
    run.mark("combat_open")

    # 命令盘中心就是「攻击」，确认两下＝选攻击 + 选目标。
    for r in range(args.swings):
        c.key("Space"); time.sleep(0.4)
        c.key("Space"); time.sleep(1.2)
        run.mark(f"swing_{r}")
        time.sleep(2.0)

    path = run.write()
    print(f"\n{len(run.steps)} 步 → {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
