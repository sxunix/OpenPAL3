"""扫某个结构体字段的**字面偏移**写入点。

    python scanfield.py 0x24c

从 Ghidra 导出的 decompiled.c 里取 6328 个函数入口，按 (地址, 长度) 逐个反
汇编——**不要用线性扫**，那会失步：实测 FUN_004f2c70 在五处写 +0x24c，四个
相位的线性扫一处都没抓到。

只找 `mov [... 0x24c ...], x` 这种**字面**形式。编译器常把结构体基址提前折
进寄存器（`lea esi,[base+0x24c]` 之后写 `mov [esi],1`），那种要用
scanfold.py。两次都栽在这上面：combat_flow.md 第 9.7、9.8 节。
"""
import sys, re
sys.path.insert(0,'/Users/admin/Documents/crewplus-dev/OpenPAL3/switch/re')
import disasm
from capstone import *
data = open('/Users/admin/Documents/crewplus-dev/仙剑奇侠传3/re/PAL3_unpacked.exe','rb').read()
secs, base = disasm.sections(data)
md = Cs(CS_ARCH_X86, CS_MODE_32)
src=open('/Users/admin/Documents/crewplus-dev/仙剑奇侠传3/re/out/decompiled.c',encoding='utf-8',errors='replace')
funcs=[]
for line in src:
    m=re.match(r'^// ==== (\S+) @ ([0-9a-f]+) \(size (\d+)\)', line)
    if m: funcs.append((m.group(1), int(m.group(2),16), int(m.group(3))))
target = sys.argv[1]
hits={}
for n,a,sz in funcs:
    o = disasm.to_offset(secs, base, a)
    if o is None: continue
    for x in md.disasm(data[o:o+sz], a):
        if x.mnemonic != 'mov': continue
        m = re.match(r'(byte|word|dword) ptr \[([^\]]*'+target+r'[^\]]*)\], (.+)$', x.op_str)
        if m: hits[x.address]=(n, x.op_str)
print(f"从 {len(funcs)} 个函数入口扫，对 {target} 的写 {len(hits)} 处:")
for a in sorted(hits):
    n,s = hits[a]
    print(f"  {a:08x}  {n:<18} mov {s}")
