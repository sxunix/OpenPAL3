#!/usr/bin/env python3
"""符号执行一个界面布局函数：跳过所有调用，在调用处记下参数和 rect。
用法: layout_trace.py START SIZE [field=value ...]   (field 是对象偏移的十六进制)"""
import re, struct, sys, capstone
EXE = "/Users/admin/Documents/crewplus-dev/仙剑奇侠传3/re/PAL3_unpacked.exe"
d = open(EXE, "rb").read()
pe = struct.unpack_from("<I", d, 0x3C)[0]; n = struct.unpack_from("<H", d, pe + 6)[0]; opt = struct.unpack_from("<H", d, pe + 20)[0]
secs = []
for i in range(n):
    o = pe + 24 + opt + i * 40; vs, va, rs, ro = struct.unpack_from("<IIII", d, o + 8); secs.append((0x400000 + va, max(vs, rs), ro, rs))
def off(v):
    for va, vs, ro, rs in secs:
        if va <= v < va + vs and v - va < rs: return ro + v - va
def u32(v):
    o = off(v); return None if o is None else struct.unpack_from("<I", d, o)[0]
def cstr(v):
    o = off(v)
    if o is None: return None
    e = d.find(b"\0", o, o + 120)
    try:
        s = d[o:e].decode("gbk")
        return s if s and all(ch.isprintable() for ch in s) else None
    except: return None
def ret_n(tgt):
    o = off(tgt)
    if o is None: return None
    md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_32)
    for ins in md.disasm(d[o:o + 0x4000], tgt):
        if ins.mnemonic == "ret":
            return int(ins.op_str, 0) if ins.op_str else 0
    return None
OBJ = 0x10000000; STK = 0x7f0000
start = int(sys.argv[1], 16); size = int(sys.argv[2], 0)
R = {"esp": STK, "esi": None, "edi": None, "ebx": None, "ebp": None, "eax": None, "ecx": OBJ, "edx": None}
M = {}
for kv in sys.argv[3:]:
    k, v = kv.split("="); M[OBJ + int(k, 16)] = int(v, 0)
M[STK] = 0xdeadbeef  # return address
REG8 = {"al": "eax", "bl": "ebx", "cl": "ecx", "dl": "edx", "ah": "eax"}
def val(opnd):
    opnd = opnd.strip()
    if opnd in R: return R[opnd]
    if opnd in REG8: return None
    if re.fullmatch(r"-?0x[0-9a-f]+|-?\d+", opnd): return int(opnd, 0) & 0xffffffff
    m = re.match(r"(?:(dword|byte|word) ptr )?(?:\w+:)?\[(.+)\]", opnd)
    if m:
        a = addr(m.group(2))
        if a is None: return None
        if m.group(1) not in ("dword", None): return None
        if a in M: return M[a]
        if a < OBJ: return u32(a)
        return None
    return None
def addr(expr):
    total = 0
    for term in re.findall(r"[+-]?\s*[^+-]+", expr.replace(" ", "")):
        sign = -1 if term.startswith("-") else 1
        t = term.lstrip("+-")
        if "*" in t:
            r, k = t.split("*")
            v = R.get(r)
            if v is None: return None
            total += sign * v * int(k)
        elif t in R:
            if R[t] is None: return None
            total += sign * R[t]
        else:
            total += sign * int(t, 0)
    return total & 0xffffffff
def store(dst, v):
    dst = dst.strip()
    if dst in R: R[dst] = v; return
    if dst in REG8: R[REG8[dst]] = None; return
    m = re.match(r"(?:(dword|byte|word) ptr )?(?:\w+:)?\[(.+)\]", dst)
    if m:
        a = addr(m.group(2))
        if a is None: return
        if m.group(1) in ("dword", None): M[a] = v
        else: M[a] = None
def sv(v):
    if v is None: return "?"
    if v >= OBJ and v < OBJ + 0x200000: return "obj+%#x" % (v - OBJ)
    if 0x7e0000 <= v <= STK + 0x1000: return "stk%+#x" % (v - STK)
    s = cstr(v) if 0x500000 <= v < 0x700000 else None
    if s: return repr(s)
    return hex(v) if v > 0xffff else str(v if v < 0x80000000 else v - (1 << 32))
md = capstone.Cs(capstone.CS_ARCH_X86, capstone.CS_MODE_32)
o = off(start); pc = start; end = start + size
code = {ins.address: ins for ins in md.disasm(d[o:o + size], start)}
last_push_count = 0
while pc in code and pc < end:
    ins = code[pc]; mn, ops = ins.mnemonic, ins.op_str
    a = [x.strip() for x in re.split(r",(?![^\[]*\])", ops)] if ops else []
    nxt = pc + ins.size
    try:
        if mn == "mov" or mn == "movzx" or mn == "movsx":
            store(a[0], val(a[1]))
        elif mn == "lea":
            m = re.match(r"(?:\w+ ptr )?\[(.+)\]", a[1]); store(a[0], addr(m.group(1)) if m else None)
        elif mn in ("add", "sub", "and", "or", "imul", "shl", "sar", "shr"):
            x, y = val(a[0]), val(a[1]) if len(a) > 1 else None
            if mn == "imul" and len(a) == 3:
                x, y = val(a[1]), val(a[2])
            r = None
            if x is not None and y is not None:
                r = {"add": x + y, "sub": x - y, "and": x & y, "or": x | y, "imul": x * y,
                     "shl": x << y, "sar": (x - (1 << 32) if x & 0x80000000 else x) >> y, "shr": x >> y}[mn] & 0xffffffff
            if mn == "sub" and a[0] == "esp": R["esp"] = (R["esp"] - y) & 0xffffffff
            elif mn == "add" and a[0] == "esp": R["esp"] = (R["esp"] + y) & 0xffffffff
            else: store(a[0], r)
        elif mn == "xor":
            if a[0] == a[1]: store(a[0], 0)
            else: store(a[0], None)
        elif mn in ("inc", "dec"):
            x = val(a[0]); store(a[0], None if x is None else (x + (1 if mn == "inc" else -1)) & 0xffffffff)
        elif mn == "neg":
            x = val(a[0]); store(a[0], None if x is None else (-x) & 0xffffffff)
        elif mn == "cdq":
            R["edx"] = None
        elif mn == "idiv" or mn == "div":
            R["eax"] = None; R["edx"] = None
        elif mn == "push":
            R["esp"] -= 4; M[R["esp"]] = val(a[0]); last_push_count += 1
        elif mn == "pop":
            store(a[0], M.get(R["esp"])); R["esp"] += 4
        elif mn == "call":
            tgt = None
            if re.fullmatch(r"0x[0-9a-f]+", ops): tgt = int(ops, 16)
            nxt_ins = code.get(nxt)
            cdecl = nxt_ins is not None and nxt_ins.mnemonic == "add" and nxt_ins.op_str.startswith("esp,")
            if tgt is not None:
                nret = 0 if cdecl else (ret_n(tgt) or 0)
            else:
                nret = 0 if cdecl else last_push_count * 4
            args = [M.get(R["esp"] + 4 * i) for i in range(max(nret // 4, 0) if not cdecl else 6)]
            kind = ops
            line = "%08x call %-22s ecx=%-12s args=%s" % (pc, kind, sv(R["ecx"]), ", ".join(sv(x) for x in args))
            # rect pointer args → dump
            for x in args:
                if x is not None and (0x7e0000 <= x <= STK or OBJ <= x < OBJ + 0x200000):
                    rect = [M.get(x + 4 * k) for k in range(4)]
                    if any(v is not None for v in rect) and "call   0x553082" not in kind:
                        line += "  rect@%s=(%s)" % (sv(x), ",".join(sv(v) for v in rect))
            print(line)
            R["esp"] += nret
            R["eax"] = None; R["edx"] = None
            if not cdecl: R["ecx"] = None
            last_push_count = 0
        elif mn == "jmp":
            if re.fullmatch(r"0x[0-9a-f]+", ops): nxt = int(ops, 16)
            else: break
        elif mn == "ret":
            break
        elif mn.startswith("j") or mn in ("cmp", "test", "nop") or mn.startswith("f") or mn.startswith("rep") or mn.startswith("sto") or mn.startswith("set"):
            pass
        else:
            for x in a[:1]:
                if x in R: R[x] = None
    except Exception as e:
        print("ERR", hex(pc), mn, ops, e)
    pc = nxt
