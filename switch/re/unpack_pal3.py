#!/usr/bin/env python3
"""Statically unpack PAL3.exe (StarForce, cracked via pal3.dll loader) by emulating
pal3.dll's export #1 with unicorn and dumping the rebuilt image.

Output: re/PAL3_unpacked.exe with
  - original sections restored (.text/.rdata/.data/.bss/.rsrc...)
  - pal3.dll's .text appended as section '.pal3dll' (holds the stolen code the
    patched E9 jmps point into)
  - a rebuilt import table (.idata2) describing the IAT the loader filled
"""
import os, struct, sys, pefile
from unicorn import *
from unicorn.x86_const import *

# Directory holding the retail PAL3.exe + pal3.dll (never committed).
GAME = os.environ.get("PAL3_DIR", "/Users/admin/Documents/crewplus-dev/仙剑奇侠传3/PAL3/")
OUT = sys.argv[1] if len(sys.argv) > 1 else "re/PAL3_unpacked.exe"

EXE_BASE = 0x400000
DLL_BASE = 0x18C0000          # right after exe SizeOfImage (0x14c0000)
API_BASE = 0x7F000000         # fake API stubs
HEAP_BASE = 0x30000000
STACK_BASE = 0x0F000000
STACK_SIZE = 0x100000
OEP_CONT = EXE_BASE + 0x15507C

exe = pefile.PE(GAME + "PAL3.exe")
dll = pefile.PE(GAME + "pal3.dll")
dll.relocate_image(DLL_BASE)
dll.OPTIONAL_HEADER.ImageBase = DLL_BASE

exe_img = exe.get_memory_mapped_image(ImageBase=EXE_BASE)
exe_size = exe.OPTIONAL_HEADER.SizeOfImage
dll_img = dll.get_memory_mapped_image(ImageBase=DLL_BASE)
dll_size = (dll.OPTIONAL_HEADER.SizeOfImage + 0xFFF) & ~0xFFF

mu = Uc(UC_ARCH_X86, UC_MODE_32)
mu.mem_map(EXE_BASE, exe_size)
mu.mem_write(EXE_BASE, exe_img)
mu.mem_map(DLL_BASE, dll_size)
mu.mem_write(DLL_BASE, dll_img)
mu.mem_map(STACK_BASE, STACK_SIZE)
mu.mem_map(API_BASE, 0x10000)
mu.mem_map(HEAP_BASE, 0x1000000)

# ---- fake API surface --------------------------------------------------
apis = ["GetModuleHandleA", "GetProcAddress", "DisableThreadLibraryCalls", "VirtualAlloc", "VirtualFree"]
api_addr = {n: API_BASE + i * 0x10 for i, n in enumerate(apis)}
mu.mem_write(API_BASE, b"\xC3" * 0x100)
# point pal3.dll's IAT at them
for imp in dll.DIRECTORY_ENTRY_IMPORT:
    for i in imp.imports:
        n = i.name.decode()
        mu.mem_write(i.address, struct.pack("<I", api_addr[n]))

modules = {}            # name(lower) -> fake handle
resolved = {}           # fake proc addr -> (dll, name_or_ordinal)
proc_by_key = {}
heap_top = [HEAP_BASE]
next_proc = [API_BASE + 0x1000]

def rstr(addr):
    out = b""
    while True:
        c = mu.mem_read(addr, 1)
        if c == b"\0": return out.decode("latin1")
        out += bytes(c); addr += 1

def stdcall_ret(nargs, eax):
    esp = mu.reg_read(UC_X86_REG_ESP)
    ret = struct.unpack("<I", mu.mem_read(esp, 4))[0]
    mu.reg_write(UC_X86_REG_ESP, esp + 4 + 4 * nargs)
    mu.reg_write(UC_X86_REG_EAX, eax)
    mu.reg_write(UC_X86_REG_EIP, ret)

def arg(i):
    esp = mu.reg_read(UC_X86_REG_ESP)
    return struct.unpack("<I", mu.mem_read(esp + 4 + 4 * i, 4))[0]

def on_api(name):
    if name == "GetModuleHandleA":
        p = arg(0)
        if p == 0:
            stdcall_ret(1, EXE_BASE); return
        n = rstr(p).lower()
        if n not in modules: modules[n] = 0x70000000 + len(modules) * 0x100000
        stdcall_ret(1, modules[n])
    elif name == "GetProcAddress":
        h, p = arg(0), arg(1)
        modname = next((k for k, v in modules.items() if v == h), "?")
        key = (modname, p if p < 0x10000 else rstr(p))
        if key not in proc_by_key:
            a = next_proc[0]; next_proc[0] += 4
            proc_by_key[key] = a; resolved[a] = key
        stdcall_ret(2, proc_by_key[key])
    elif name == "VirtualAlloc":
        size = arg(1)
        a = heap_top[0]; heap_top[0] += (size + 0xFFFF) & ~0xFFFF
        stdcall_ret(4, a)
    elif name == "VirtualFree":
        stdcall_ret(3, 1)
    elif name == "DisableThreadLibraryCalls":
        stdcall_ret(1, 1)

def hook_code(uc, address, size, user):
    if API_BASE <= address < API_BASE + 0x100:
        on_api(apis[(address - API_BASE) // 0x10])
    elif address == OEP_CONT:
        uc.emu_stop()
    elif address in resolved:
        # game code called a fake import (e.g. VirtualAlloc in stolen OEP code)
        key = resolved[address]
        print("  game called import", key)
        if key[1] == "VirtualAlloc": on_api("VirtualAlloc")
        else: raise SystemExit("unhandled import call %r" % (key,))

mu.hook_add(UC_HOOK_CODE, hook_code)

# ---- run export #1 -----------------------------------------------------
exp = dll.DIRECTORY_ENTRY_EXPORT.symbols[0].address + DLL_BASE
# DllMain(hinst, DLL_PROCESS_ATTACH) first: it stores hinstance at 0x10e1
dllmain = dll.OPTIONAL_HEADER.AddressOfEntryPoint + DLL_BASE
esp = STACK_BASE + STACK_SIZE - 0x1000
mu.mem_write(esp, struct.pack("<IIII", 0xDEAD0001, DLL_BASE, 1, 0))
mu.reg_write(UC_X86_REG_ESP, esp)
mu.emu_start(dllmain, 0xDEAD0001)
print("DllMain done, eax=%x" % mu.reg_read(UC_X86_REG_EAX))

esp = STACK_BASE + STACK_SIZE - 0x1000
mu.mem_write(esp, struct.pack("<I", 0xDEAD0002))
mu.reg_write(UC_X86_REG_ESP, esp)
try:
    mu.emu_start(exp, 0xDEAD0002)
except UcError as e:
    print("emulation error:", e, "eip=%x" % mu.reg_read(UC_X86_REG_EIP))
    raise
eip = mu.reg_read(UC_X86_REG_EIP)
print("stopped at eip=%x (expect %x)" % (eip, OEP_CONT))
assert eip == OEP_CONT
print("modules:", modules)
print("resolved imports:", len(resolved))

# ---- dump ---------------------------------------------------------------
img = bytearray(mu.mem_read(EXE_BASE, exe_size))
dllcode = bytes(mu.mem_read(DLL_BASE, dll_size))

# IAT slots: find every dword in .rdata/.data holding a fake proc address
iat_slots = {}
for rva in range(0x1000, 0x20C000, 4):
    v = struct.unpack_from("<I", img, rva)[0]
    if v in resolved: iat_slots[rva] = resolved[v]
# also the stub IAT area near the end
for rva in range(0x1497000, exe_size - 4, 4):
    v = struct.unpack_from("<I", img, rva)[0]
    if v in resolved: iat_slots[rva] = resolved[v]
print("IAT slots found:", len(iat_slots), "range %x..%x" % (min(iat_slots), max(iat_slots)))

# group contiguous runs per dll
runs = []
for rva in sorted(iat_slots):
    d, n = iat_slots[rva]
    if runs and runs[-1]["dll"] == d and runs[-1]["end"] == rva:
        runs[-1]["end"] = rva + 4; runs[-1]["names"].append(n)
    else:
        runs.append({"dll": d, "start": rva, "end": rva + 4, "names": [n]})
print("import runs:", [(r["dll"], hex(r["start"]), len(r["names"])) for r in runs])

# build .idata2: descriptors + INT arrays + hint/name strings + dll names
def align(x, a): return (x + a - 1) & ~(a - 1)
IDATA_RVA = align(exe_size + dll_size, 0x1000)
desc_size = 20 * (len(runs) + 1)
blob = bytearray(desc_size)
int_arrays = {}
for i, r in enumerate(runs):
    int_arrays[i] = len(blob)
    blob += b"\0" * (4 * (len(r["names"]) + 1))
name_rvas = {}
for i, r in enumerate(runs):
    for j, n in enumerate(r["names"]):
        if isinstance(n, int):
            val = 0x80000000 | n
        else:
            while len(blob) % 2: blob += b"\0"
            off = len(blob); blob += struct.pack("<H", 0) + n.encode() + b"\0"
            val = IDATA_RVA + off
        struct.pack_into("<I", blob, int_arrays[i] + 4 * j, val)
for i, r in enumerate(runs):
    off = len(blob); blob += r["dll"].encode() + b"\0"
    struct.pack_into("<IIIII", blob, 20 * i, IDATA_RVA + int_arrays[i], 0, 0, IDATA_RVA + off, r["start"])
    # and write the INT into the IAT slots too (loader overwrites them anyway)
    for j, n in enumerate(r["names"]):
        struct.pack_into("<I", img, r["start"] + 4 * j, struct.unpack_from("<I", blob, int_arrays[i] + 4 * j)[0])
IMPORT_DIR = (IDATA_RVA, desc_size)
idata = bytes(blob)

# ---- write PE -----------------------------------------------------------
# sections (RVA, vsize, name, chars), raw data taken from img
R, W, X, CODE, IDATA_, UDATA = 0x40000000, 0x80000000, 0x20000000, 0x20, 0x40, 0x80
secs = [
    (".text",   0x1000,    0x169000 - 0x1000,   R | X | CODE),
    (".rdata",  0x16A000,  0x180000 - 0x16A000, R | X | CODE | IDATA_),
    (".data",   0x180000,  0x20C000 - 0x180000, R | W | IDATA_),
    (".bss",    0x20C000,  0x1497000 - 0x20C000, R | W | UDATA),
    (".rsrc",   0x1497000, 0x1499000 - 0x1497000, R | IDATA_),
    (".tail",   0x1499000, exe_size - 0x1499000, R | W | X | IDATA_),
    (".pal3dll", exe_size, dll_size, R | W | X | CODE),
    (".idata2", IDATA_RVA, align(len(idata), 0x1000), R | W | IDATA_),
]
FILE_ALIGN = 0x200
hdr_size = 0x400
out = bytearray()
raw_ptr = hdr_size
sec_hdrs = []
body = bytearray()
for name, rva, vsz, ch in secs:
    if name == ".pal3dll": data = dllcode
    elif name == ".idata2": data = idata
    else: data = bytes(img[rva:rva + vsz])
    # trim trailing zeros for the huge bss but keep at least what is nonzero
    if name == ".bss":
        data = b""
    rsz = align(len(data), FILE_ALIGN)
    sec_hdrs.append((name, rva, vsz, raw_ptr if rsz else 0, rsz, ch))
    body += data + b"\0" * (rsz - len(data))
    raw_ptr += rsz

# clone original headers and rewrite
hdr = bytearray(exe_img[:hdr_size])
pe_off = struct.unpack_from("<I", hdr, 0x3C)[0]
fh = pe_off + 4
struct.pack_into("<H", hdr, fh + 2, len(sec_hdrs))
oh = fh + 20
struct.pack_into("<I", hdr, oh + 16, 0x15507C - 0)  # placeholder, fixed below
# entry point: stolen OEP code lives in pal3.dll at 0x5760 -> use that
entry = (DLL_BASE - EXE_BASE) + 0x5760
struct.pack_into("<I", hdr, oh + 16, entry)
struct.pack_into("<I", hdr, oh + 32, 0x1000)      # SectionAlignment
struct.pack_into("<I", hdr, oh + 36, FILE_ALIGN)  # FileAlignment
new_image_size = align(IDATA_RVA + len(idata), 0x1000)
struct.pack_into("<I", hdr, oh + 56, new_image_size)
struct.pack_into("<I", hdr, oh + 60, hdr_size)
struct.pack_into("<I", hdr, oh + 64, 0)           # checksum
# data directories: import, resource; clear reloc, iat, bound import etc.
dd = oh + 96
for i in range(16): struct.pack_into("<II", hdr, dd + 8 * i, 0, 0)
struct.pack_into("<II", hdr, dd + 8 * 1, IMPORT_DIR[0], IMPORT_DIR[1])
struct.pack_into("<II", hdr, dd + 8 * 2, 0x1497000, 0x53CA)
# section table
st = oh + 224
for i, (name, rva, vsz, ptr, rsz, ch) in enumerate(sec_hdrs):
    struct.pack_into("<8sIIIIIIHHI", hdr, st + 40 * i, name.encode().ljust(8, b"\0"), vsz, rva, rsz, ptr, 0, 0, 0, 0, ch)
for i in range(len(sec_hdrs), (hdr_size - st) // 40):
    struct.pack_into("<40s", hdr, st + 40 * i, b"\0" * 40)

with open(OUT, "wb") as f:
    f.write(hdr + body)
print("wrote", OUT, len(hdr) + len(body), "bytes; entry rva %x; image size %x" % (entry, new_image_size))
