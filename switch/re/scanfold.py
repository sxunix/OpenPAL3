"""扫**基址被折进寄存器**之后的字段写入点。

    python scanfold.py 0x488 0x4d0000 0x520000
                       目标偏移  地址区间

在每个函数里跟踪 `lea reg, [base + K]`，再看 `mov [reg + M], x` 是否
`K + M == 目标`。scanfield.py 只认字面偏移，漏的就是这一类。

自检：`scanfold.py 0x24c 0x4d0000 0x520000` 必须扫到 FUN_004ed870 的
`mov dword [esi], 1`（战后把精抬到 1 的那句）。

**命中数不是证据。** 寄存器跟踪很粗（`mov ebp, esp` 之类会造成假阳性），
每一条都要点开读。反过来「三种形式都是 0 处」才算像样的否定证据——
字面（scanfield）、`lea/add` 里出现目标常量、以及本脚本的折叠形式。
"""
import sys, re
sys.path.insert(0,'/Users/admin/Documents/crewplus-dev/OpenPAL3/switch/re')
import disasm
from capstone import *
data = open('/Users/admin/Documents/crewplus-dev/仙剑奇侠传3/re/PAL3_unpacked.exe','rb').read()
secs, base = disasm.sections(data)
md = Cs(CS_ARCH_X86, CS_MODE_32)
funcs=[]
for line in open('/Users/admin/Documents/crewplus-dev/仙剑奇侠传3/re/out/decompiled.c',encoding='utf-8',errors='replace'):
    m=re.match(r'^// ==== (\S+) @ ([0-9a-f]+) \(size (\d+)\)', line)
    if m: funcs.append((m.group(1), int(m.group(2),16), int(m.group(3))))
target = int(sys.argv[1], 16)
lo, hi = int(sys.argv[2],16), int(sys.argv[3],16)
LEA = re.compile(r'^(e[a-z]{2}), \[(e[a-z]{2})(?: \+ (e[a-z]{2}))?(?: \+ (0x[0-9a-f]+))?\]$')
ST  = re.compile(r'^(byte|word|dword) ptr \[(e[a-z]{2})(?: \+ (e[a-z]{2})(?:\*\d)?)?(?: \+ (0x[0-9a-f]+))?\], (.+)$')
found=[]
for n,a,sz in funcs:
    if not (lo <= a < hi): continue
    o = disasm.to_offset(secs, base, a)
    if o is None: continue
    folded={}   # reg -> K
    for x in md.disasm(data[o:o+sz], a):
        if x.mnemonic == 'lea':
            m = LEA.match(x.op_str)
            if m:
                dst, b, idx, disp = m.groups()
                folded[dst] = int(disp,16) if disp else 0
            continue
        if x.mnemonic == 'mov':
            m = ST.match(x.op_str)
            if m:
                w, b, idx, disp, src = m.groups()
                d = int(disp,16) if disp else 0
                if b in folded and folded[b] + d == target:
                    found.append((x.address, n, f"base+{hex(folded[b])} 里的 +{hex(d)}", x.op_str))
            continue
        # 目的寄存器被别的指令改写就作废
        d = x.op_str.split(',')[0].strip()
        if d in folded and x.mnemonic in ('mov','add','sub','xor','pop','inc','dec'):
            folded.pop(d, None)
print(f"折叠基址后落在 {hex(target)} 的写 {len(found)} 处:")
for a,n,how,s in found:
    print(f"  {a:08x}  {n:<18} {how}   mov {s}")
