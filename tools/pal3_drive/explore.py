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
WANDER_LIMIT = 80         # 没东西可试、只好走出去的连续次数上限（渝州城一圈门多，30 次会漏掉没走到的块）
BACK_OUT_SECONDS = 15.0  # 没对话框、画面不变超过这么久就按 B 往外退
CHOICE_TURNS: dict[tuple, int] = {}   # 每个选单（按选项文字）已经选过几次
SETTLE_LIMIT = 240.0      # 一次收尾最多等多久


def fingerprint(c: Client) -> tuple[str, str, str]:
    s = c.state()
    g = c.globals()
    h = hashlib.sha1(json.dumps(sorted(g.items())).encode()).hexdigest()[:10]
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
        if s.get("current_script_fn") == "<agent walk>":
            pass  # 引擎带着主角在走（fire_trigger），别按键
        elif s.get("movie_playing"):
            c.key("Escape")
        elif dlg.get("open"):
            choices = dlg.get("choices") or []
            if choices:
                # 同一个选单每来一次换下一项：「是 / 否」先选「是」，「1. 关于… 2. … 3. 不想知道」
                # 这种选了会绕回来的，轮到最后一项就出去了。选单每次打开光标都在第一项。
                key_ = tuple(choices)
                k = CHOICE_TURNS.get(key_, 0) % len(choices)
                CHOICE_TURNS[key_] = CHOICE_TURNS.get(key_, 0) + 1
                issues.append({"kind": "choice", "at": label, "choices": choices, "picked": k})
                for _ in range(k):
                    c.key("GamePadDPadDown")
                    time.sleep(0.05)
                c.key("GamePadEast")
            else:
                c.dialog_advance()
            lines += 1
        else:
            # 没有对话框：可能是小游戏（鉴定、宿营……）或别的界面。按手柄 A——引擎里
            # 是确定（`GamePadEast`），imgui 里是 `GamepadFaceDown`，两边都认；空格
            # 鉴定的询问框不认。隔一下夹个十字键左：鉴定页得先选一边，A 才确定得了。
            # 15 秒没动静多半是商店、当铺这类等玩家自己退出的界面，改按 B（取消）。
            pokes += 1
            if time.time() - last_change > BACK_OUT_SECONDS:
                c.key("GamePadSouth")
            else:
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


# 最近一次 candidates() 看到、但从主角现在的位置走不到的触发点，和挡在半路的物件——走不下去时报出来。
LAST_UNREACHABLE: dict = {}
# 最近一次 candidates() 里各触发点的位置；进块时主角站的位置。挑出口时同样没走过的先挑离进来的
# 地方远的——近的那个多半就是来时的门。
CENTERS: dict = {}
ARRIVAL: dict = {}


def far(name: str) -> float:
    """触发点离进块时主角站的位置多远（x、z）。不知道的当 0。"""
    a, b = ARRIVAL.get("pos"), CENTERS.get(name)
    if not a or not b:
        return 0.0
    return ((a[0] - b[0]) ** 2 + (a[2] - b[2]) ** 2) ** 0.5


def candidates(c: Client) -> tuple[list[str], list[str]]:
    """当前场景里从主角现在的位置**走得到**的触发点：(人物 + 物件 + 拾取点, 导航格)。

    「走得到」问引擎（`script_eval reachable`：走路、梯子 / 藤、换层区、迷宫里的跳，含物件挡路，
    不出这个块）。触发点是传送过去触发的，不筛的话会穿过机关、跳过要跳的沟，看不出机关挡路。"""
    roles = [r for r in c.roles()
             if r.get("visible") and r.get("talk_function") and r["name"].startswith("role:")]
    trig = c.triggers()
    touch = [t for t in trig if t["shape"] in ("aabb", "item")]
    nav = [t["name"] for t in trig if t["shape"] == "nav"]
    points = [r["position"] for r in roles] + [t["center"] for t in touch]
    names = [r["name"] for r in roles] + [t["name"] for t in touch]
    here = c.state().get("leader_pos") or [0, 0, 0]
    flat = [v for p in (points or [here]) for v in p]
    res = (c.eval("reachable", *flat).get("data") or {}).get("result")
    if not res:
        return names, nav
    ok = res.get("points") or []
    acts = [n for n, good in zip(names, ok) if good]
    reach_nav = set(res.get("nav_triggers") or [])
    navs = [n for n in nav if n in reach_nav]
    CENTERS.clear()
    CENTERS.update({t["name"]: t["center"] for t in trig})
    CENTERS.update({r["name"]: r["position"] for r in roles})
    LAST_UNREACHABLE.clear()
    LAST_UNREACHABLE.update({
        "unreachable": [n for n, good in zip(names, ok) if not good] + [n for n in nav if n not in reach_nav],
        "blockers": res.get("blockers") or [],
    })
    return acts, navs


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base", default=os.environ.get("PAL3_AGENT", "http://127.0.0.1:8777"))
    ap.add_argument("--out", default="runs/explore")
    ap.add_argument("--load", type=int, default=None)
    ap.add_argument("--steps", type=int, default=150)
    ap.add_argument("--save", type=int, default=3, help="剧情推进时存到这个档（0 = 不存）")
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
    # 每个触发（按场景 + 块 + 名字）在所有剧情状态下一共试过几次。挑的时候少的先来：
    # 剧情推进一次全局变量就变、状态就全新，不排序的话每推进一次就把整座城重逛一遍。
    count: dict[tuple, int] = {}
    wander = 0            # 连续「这里没有没试过的、只好走出去」的次数
    progress = 0
    for step in range(args.steps):
        # 走动中撞上野怪：战斗舞台压在场景栈顶，列不出触发点（409），先打完。
        try:
            acts, navs = candidates(c)
        except AgentError as e:
            if e.status != 409:
                raise
            r = settle(c, issues, f"step{step}_encounter")
            if r.get("crashed"):
                break
            continue
        key = fingerprint(c)
        done = tried.setdefault(key, set())
        n_of = lambda n: count.get((key[0], key[1], n), 0)  # noqa: E731
        if ARRIVAL.get("block") != (key[0], key[1]):
            ARRIVAL.update({"block": (key[0], key[1]), "pos": c.state().get("leader_pos")})
        todo = sorted([n for n in acts + navs if n not in done],
                      key=lambda n: (n_of(n), n in navs, -far(n)))
        if todo:
            name = todo[0]
            done.add(name)
            wander = 0
        else:
            # 这里在当前剧情状态下全试过了：走出去换个地方找，挑走得最少的出口。
            # 连续 WANDER_LIMIT 次都这样才算真的走不下去。
            wander += 1
            if wander > WANDER_LIMIT or not navs:
                issues.append({"kind": "dead_end", "scene": key[0], "block": key[1],
                               "tried": sorted(done), **LAST_UNREACHABLE})
                run.mark(f"dead_end_{key[0]}_{key[1]}", shot=True)
                break
            name = min(navs, key=lambda n: (n_of(n), -far(n)))
        count[(key[0], key[1], name)] = n_of(name) + 1
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
        if args.save and after[2] != key[2]:
            # 全局变量变了就是剧情往前走了：存一档，下次 `--load` 从这里接着走。
            try:
                c.save(args.save)
            except AgentError as e:
                issues.append({"kind": "save_error", "at": name, "detail": str(e)})
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
