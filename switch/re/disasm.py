#!/usr/bin/env python3
"""反汇编脱壳 PAL3_unpacked.exe 里的一段地址。

Ghidra 对很多 SCE 指令处理函数只建了 LAB_ 标签而没有建函数，
`getfunc.py` 取不到，用这个直接读字节。

用法：
    python disasm.py 00435350            # 反汇编到第一个 ret / 跳出
    python disasm.py 00435350 -n 60      # 指定条数
    python disasm.py 0x435350 0x435380   # 多个地址

调用目标和全局变量会尽量换成 Ghidra 导出的名字（out/functions.txt）。
"""
import os
import re
import struct
import sys

import capstone

HERE = os.path.dirname(os.path.abspath(__file__))
RE_DIR = os.path.join(HERE, "..", "..", "..", "仙剑奇侠传3", "re")
EXE = os.environ.get("PAL3_EXE", os.path.join(RE_DIR, "PAL3_unpacked.exe"))
FUNCS = os.environ.get("PAL3_FUNCS", os.path.join(RE_DIR, "out", "functions.txt"))


def sections(data):
    """(virtual_address, virtual_size, raw_offset, raw_size) 与镜像基址。"""
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    n = struct.unpack_from("<H", data, pe + 6)[0]
    opt_size = struct.unpack_from("<H", data, pe + 20)[0]
    base = struct.unpack_from("<I", data, pe + 24 + 28)[0]
    out = []
    off = pe + 24 + opt_size
    for i in range(n):
        h = off + i * 40
        vsize, va, rsize, roff = struct.unpack_from("<IIII", data, h + 8)
        out.append((va, vsize, roff, rsize))
    return out, base


def to_offset(secs, base, va):
    rva = va - base
    for sva, vsize, roff, rsize in secs:
        if sva <= rva < sva + max(vsize, rsize):
            delta = rva - sva
            if delta < rsize:
                return roff + delta
    return None


def load_names():
    names = {}
    if not os.path.exists(FUNCS):
        return names
    for line in open(FUNCS, encoding="utf-8", errors="replace"):
        m = re.match(r"([0-9a-fA-F]+)\s+(\S+)", line)
        if m:
            names[int(m.group(1), 16)] = m.group(2)
    return names


def name_of(names, addr):
    return names.get(addr)


def run(va, count, data, secs, base, names):
    off = to_offset(secs, base, va)
    if off is None:
        print(f"{va:08x}: 不在任何节里")
        return
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_32)
    md.detail = True
    blob = data[off : off + 4096]
    print(f"// ==== {va:08x} ====")
    shown = 0
    for ins in md.disasm(blob, va):
        text = f"{ins.address:08x}  {ins.mnemonic:<7} {ins.op_str}"
        for m in re.finditer(r"0x([0-9a-f]+)", ins.op_str):
            n = name_of(names, int(m.group(1), 16))
            if n:
                text += f"    ; {n}"
                break
        print(text)
        shown += 1
        if count and shown >= count:
            break
        if not count and ins.mnemonic.startswith("ret"):
            break
    print()


def main(argv):
    count = 0
    args = []
    i = 0
    while i < len(argv):
        if argv[i] == "-n":
            count = int(argv[i + 1])
            i += 2
        else:
            args.append(argv[i])
            i += 1
    data = open(EXE, "rb").read()
    secs, base = sections(data)
    names = load_names()
    for a in args:
        run(int(a, 16), count, data, secs, base, names)


if __name__ == "__main__":
    main(sys.argv[1:])
