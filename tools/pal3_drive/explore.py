#!/usr/bin/env python3
"""探路：从当前进度往主线后面走，找卡死、崩溃、走不下去的地方。

    python3 tools/pal3_drive/explore.py --out runs/explore            # 新游戏开始
    python3 tools/pal3_drive/explore.py --out runs/explore --load 2   # 从某个存档开始

只走引擎自己的触发路径（`/v1/scene/fire_trigger`：传送进导航格、站到物件 /
人物旁按调查键），不直接调 proc、不改旗标——剧情条件全照原来的判。

状态 = 场景 + 块 + 全局变量指纹。每个状态下按「人物对话 → 物件 / 拾取点 →
导航格」的次序挑没试过的触发；全局变量变了就是剧情往前走了，换个状态接着挑。
一个状态下全试完都没推进就停，报告「走不下去」——可能是真卡死，也可能是这个
探路法够不着的东西（要用道具、要选特定对话分支、要走固定路线）。

每一步的结果写进 `run.jsonl`，问题汇总写进 `issues.json`。
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pal3_drive import AgentError, Client, Run, boot  # noqa: E402

STUCK_SECONDS = 90.0      # 脚本在跑但快照什么都不变，超过这么久算卡住（章节字幕 SceCommandCaption 是按时间走的，13 列要三四十秒，进度不在快照里）
SETTLE_LIMIT = 240.0      # 一次收尾最多等多久


def fingerprint(c: Client) -> tuple[str, str, str]:
    s = c.state()
    g = c.globals()
    h = hashlib.sha1(json.dumps(g).encode()).hexdigest()[:10]
    return s.get("scene", ""), s.get("block", ""), h


def settle(c: Client, issues: list, label: str) -> dict:
    """把正在跑的脚本推完：对话、选项、片子、战斗。返回最后的状态和过程摘要。"""
    t0 = time.time()
    last_change = t0
    last_sig = None
    lines = 0
    pokes = 0
    battles = 0
    in_battle = False
    while True:
        try:
            s = c.state()
        except AgentError as e:
            issues.append({"kind": "agent_error", "at": label, "detail": str(e)})
            return {"crashed": True}
        except Exception as e:  # 进程没了
            issues.append({"kind": "crash", "at": label, "detail": repr(e)})
            return {"crashed": True}
        cb = s.get("combat") or {}
        dlg = s.get("dialog") or {}
        sig = (s.get("scene"), s.get("block"), s.get("current_script_fn"),
               dlg.get("text"), cb.get("menu"), cb.get("staging"),
               tuple(u.get("hp") for u in cb.get("units", [])), s.get("movie_playing"))
        if sig != last_sig:
            last_sig = sig
            last_change = time.time()
        if cb.get("active"):
            if not in_battle:
                battles += 1
                in_battle = True
            if cb.get("menu") in ("main", "target"):
                c.key("Space")
            time.sleep(0.15)
            continue
        in_battle = False
        if cb.get("outcome") == "lose" and not s.get("script_running"):
            issues.append({"kind": "battle_lost", "at": label})
        if not s.get("script_running"):
            return {"lines": lines, "battles": battles, "seconds": round(time.time() - t0, 1)}
        if s.get("movie_playing"):
            c.key("Escape")
        elif dlg.get("open"):
            if dlg.get("choices"):
                issues.append({"kind": "choice", "at": label, "choices": dlg["choices"]})
            c.dialog_advance()
            lines += 1
        else:
            # 没有对话框：可能是小游戏（鉴定、宿营……）或别的界面。按手柄 A——引擎里
            # 是确定（`GamePadEast`），imgui 里是 `GamepadFaceDown`，两边都认；空格
            # 鉴定的询问框不认。隔一下夹个十字键左：鉴定页得先选一边，A 才确定得了。
            pokes += 1
            c.key("GamePadDPadLeft" if pokes % 3 == 0 else "GamePadEast")
        now = time.time()
        if now - last_change > STUCK_SECONDS:
            issues.append({"kind": "stuck", "at": label, "fn": s.get("current_script_fn"),
                           "dialog": dlg.get("text"), "scene": s.get("scene")})
            return {"stuck": True, "lines": lines}
        if now - t0 > SETTLE_LIMIT:
            issues.append({"kind": "settle_timeout", "at": label, "fn": s.get("current_script_fn")})
            return {"stuck": True, "lines": lines}
        time.sleep(0.3)


def candidates(c: Client) -> list[str]:
    """当前场景里能触发的东西，按尝试次序排。"""
    roles = [r["name"] for r in c.roles()
             if r.get("visible") and r.get("talk_function") and r["name"].startswith("role:")]
    trig = c.triggers()
    touch = [t["name"] for t in trig if t["shape"] in ("aabb", "item")]
    nav = [t["name"] for t in trig if t["shape"] == "nav"]
    return roles + touch + nav


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default=os.environ.get("PAL3_AGENT", "http://127.0.0.1:8777"))
    ap.add_argument("--out", default="runs/explore")
    ap.add_argument("--load", type=int, default=None)
    ap.add_argument("--steps", type=int, default=150)
    ap.add_argument("--shots", action="store_true", help="每次推进都截图")
    args = ap.parse_args()

    c = Client(args.base)
    if not c.up(90):
        print("连不上 agent server", file=sys.stderr)
        return 2
    run = Run(c, args.out, shots=args.shots)
    issues: list = []
    boot(c, args.load)
    settle(c, issues, "boot")
    run.mark("start", shot=True)

    tried: dict[tuple, set] = {}
    progress = 0
    for step in range(args.steps):
        key = fingerprint(c)
        done = tried.setdefault(key, set())
        todo = [n for n in candidates(c) if n not in done]
        if not todo:
            issues.append({"kind": "dead_end", "scene": key[0], "block": key[1],
                           "tried": sorted(done)})
            run.mark(f"dead_end_{key[0]}_{key[1]}", shot=True)
            break
        name = todo[0]
        done.add(name)
        try:
            c.fire(name)
        except AgentError as e:
            issues.append({"kind": "fire_error", "trigger": name, "detail": str(e)})
            continue
        time.sleep(0.6)
        result = settle(c, issues, f"{key[0]}/{key[1]} {name}")
        if result.get("crashed"):
            # 进程没了，别再去问它要状态。
            print(f"  [{step}] {key[0]}/{key[1]} {name} → 崩溃 / 无响应", flush=True)
            break
        after = fingerprint(c)
        moved = after != key
        if moved:
            progress += 1
        row = run.mark(f"{step:03d}_{name}{'_progress' if moved else ''}", shot=args.shots and moved)
        row.update({"trigger": name, "result": result, "moved": moved,
                    "from": list(key), "to": list(after)})
        print(f"  [{step}] {key[0]}/{key[1]} {name} → {'推进' if moved else '无变化'} "
              f"{'换场 ' + after[0] + '/' + after[1] if after[:2] != key[:2] else ''} {result}",
              flush=True)

    run.write()
    with open(os.path.join(args.out, "issues.json"), "w", encoding="utf-8") as f:
        json.dump(issues, f, ensure_ascii=False, indent=1)
    kinds = {}
    for i in issues:
        kinds[i["kind"]] = kinds.get(i["kind"], 0) + 1
    print(f"\n推进 {progress} 次；问题 {kinds}；→ {args.out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
