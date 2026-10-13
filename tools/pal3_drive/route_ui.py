#!/usr/bin/env python3
"""路线 05：战斗外的界面巡一遍——状态菜单八页、买卖、当铺、冶炼。

    python3 tools/pal3_drive/route_ui.py --out runs/ui
    python3 tools/pal3_drive/route_ui.py --out runs/ui --load 4   # 读上次存的档，省掉序章

新游戏跑完序章后补满五人、加钱加物品，存到 4 号档（下次 `--load 4` 直接用），
然后挨个打开每个界面：开 → 截图 → 按两下方向键再截 → Esc 关 → 截图确认回到场景。
"""

from __future__ import annotations

import argparse
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pal3_drive import Client, Run, boot  # noqa: E402

STATUS_PAGES = ["state", "prop", "equip", "magic", "lineup", "sword", "book", "system"]

#: 测试用的物品（id 和名字对过发行 gdb）：古董给当铺、恢复类给道具页、
#: 蛊 / 鬼遁水 / 引路蜂是 special 1..3 那几样、矿石给冶炼。
ITEMS = [
    (6001, 2),  # 白釉烛台（古董）
    (6002, 1),  # 三彩骆驼（古董）
    (6201, 5),  # 止血草
    (6202, 3),  # 金创药
    (6113, 2),  # 蛊
    (6114, 1),  # 鬼遁水
    (6401, 1),  # 引路蜂
    (6528, 2),  # 绿松石（矿石）
]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default=os.environ.get("PAL3_AGENT", "http://127.0.0.1:8777"))
    ap.add_argument("--out", default="runs/ui")
    ap.add_argument("--load", type=int, default=None, help="读这个档代替跑序章")
    ap.add_argument("--save", type=int, default=4, help="准备好之后存到哪个档（0 = 不存）")
    args = ap.parse_args()

    c = Client(args.base)
    if not c.up(90):
        print("连不上 agent server", file=sys.stderr)
        return 2
    run = Run(c, args.out)

    boot(c, args.load)
    if args.load is None:
        c.eval("team_add", 1, 2, 3, 4)
        c.eval("add_money", 20000)
        for item, n in ITEMS:
            c.eval("add_item", item, n)
        time.sleep(0.5)
        if args.save:
            c.save(args.save)
    run.mark("field")

    # ---- 状态菜单 ----
    for page in STATUS_PAGES:
        c.status_menu(True, page)
        time.sleep(1.5)
        run.mark(f"status_{page}")
    c.status_menu(False)
    time.sleep(1.0)
    run.mark("status_closed")

    # ---- 三家调试入口：开、动两下、关 ----
    for name, call in (
        ("dealer", ("dealer",)),
        ("hockshop", ("hockshop", 0)),
        ("smelting", ("smelting", 0)),
    ):
        c.eval(*call)
        time.sleep(2.0)
        run.mark(f"{name}_open")
        for k in ("Down", "Down"):
            c.key(k)
            time.sleep(0.4)
        run.mark(f"{name}_moved")
        c.key("Escape")
        time.sleep(1.5)
        s = run.mark(f"{name}_closed")
        if s.get("script_running"):
            # 有的界面要两下（先退子页再退整页）
            c.key("Escape")
            time.sleep(1.5)
            run.mark(f"{name}_closed2")

    path = run.write()
    print(f"\n{len(run.steps)} 步 → {path}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
