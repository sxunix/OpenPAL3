#!/usr/bin/env python3
"""从 Ghidra 导出的 decompiled.c 里取出指定函数。

用法：
    python getfunc.py FUN_00537a90 [FUN_...]      # 按名字
    python getfunc.py 00537a90                    # 按地址（十六进制，可带 0x）
    python getfunc.py --list 0053                 # 列出地址前缀匹配的函数

decompiled.c 路径由环境变量 PAL3_DECOMP 指定，默认
../../../仙剑奇侠传3/re/out/decompiled.c（相对本脚本）。
"""
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT = os.path.join(HERE, "..", "..", "..", "仙剑奇侠传3", "re", "out", "decompiled.c")
PATH = os.environ.get("PAL3_DECOMP", DEFAULT)

HEADER = re.compile(r"^// ==== (\S+) @ ([0-9a-fA-F]+) \(size (\d+)\) ====$")


def iter_functions(path):
    name = addr = None
    buf = []
    with open(path, encoding="utf-8", errors="replace") as f:
        for line in f:
            m = HEADER.match(line.rstrip("\n"))
            if m:
                if name:
                    yield name, addr, "".join(buf)
                name, addr = m.group(1), m.group(2).lower()
                buf = [line]
            elif name:
                buf.append(line)
    if name:
        yield name, addr, "".join(buf)


def main(argv):
    if not argv:
        print(__doc__)
        return 2
    if argv[0] == "--list":
        prefix = argv[1].lower().removeprefix("0x")
        for name, addr, _ in iter_functions(PATH):
            if addr.startswith(prefix):
                print(name, addr)
        return 0
    wanted = set()
    for a in argv:
        a = a.lower().removeprefix("0x")
        wanted.add(a if a.startswith("fun_") else "fun_" + a)
        wanted.add(a)
    found = 0
    for name, addr, body in iter_functions(PATH):
        if name.lower() in wanted or addr in wanted:
            sys.stdout.write(body)
            found += 1
    return 0 if found else 1


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
