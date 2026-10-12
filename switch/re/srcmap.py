"""把 exe / dll 里残留的 `__FILE__` + `__LINE__` 配对打出来，当地址↔源码行的锚点。

    python srcmap.py                       # PAL3_unpacked.exe
    python srcmap.py /path/to/gbengine.dll

`EXCEPTION(msg)` 这个宏展开后在调用点压 `__LINE__` 和 `__FILE__`，release 版
里没被优化掉，所以 `.text` 里每一处 `push <"D:\\Project\\PAL3\\…cpp">` 前面
那条 `push imm` 就是源码行号。exe 里有 108 处，覆盖 28 个文件。

拿来做什么：**核对 cpk 里那份源码和出货的二进制是不是同一版**。把打出来的
(文件, 行号) 丢回 `src_from_cpk` 对应文件的那一行，落在 `throw EXCEPTION(...)`
上就是对得上。实测 exe 侧 108 处里除了几处启发式抓错 push 的，全部命中。

启发式只看紧挨着的那条 push（前 5 字节 `68`、前 2 字节 `6a`，再看后面），
抓不准的会打成别的行，对不上不代表版本不同——回去反汇编那一处看看。
"""
import sys, struct

PATHS = [b'D:\\Project\\PAL3\\', b'd:\\project\\pal3\\', b'D:\\project\\pal3\\',
         b'D:\\Project\\gbEngine\\', b'd:\\project\\gbengine\\',
         b'D:\\project\\gbengine\\', b'D:\\Project\\GBENGINE\\', b'D:\\Project\\gbengine9\\']


def load(path):
    d = open(path, 'rb').read()
    pe = struct.unpack_from('<I', d, 0x3c)[0]
    nsec = struct.unpack_from('<H', d, pe + 6)[0]
    optsz = struct.unpack_from('<H', d, pe + 20)[0]
    base = struct.unpack_from('<I', d, pe + 24 + 28)[0]
    secs = []
    off = pe + 24 + optsz
    for i in range(nsec):
        s = d[off + i * 40: off + (i + 1) * 40]
        name = s[:8].rstrip(b'\0').decode('latin1')
        vsz, va, rsz, roff = struct.unpack_from('<IIII', s, 8)
        secs.append((name, va, vsz, roff, rsz))
    return d, base, secs


def main(path):
    d, base, secs = load(path)

    def off2va(o):
        for _, sva, vsz, roff, rsz in secs:
            if roff <= o < roff + rsz:
                return base + sva + (o - roff)
        return None

    strs = {}
    for pat in PATHS:
        i = 0
        while True:
            i = d.find(pat, i)
            if i < 0:
                break
            end = d.find(b'\0', i)
            va = off2va(i)
            if va:
                strs[va] = d[i:end].decode('latin1')
            i = end + 1

    text = [(base + sva, roff, min(vsz, rsz))
            for name, sva, vsz, roff, rsz in secs if name.lower().startswith('.text')]
    rows = []
    for sva, s in strs.items():
        pat = b'\x68' + struct.pack('<I', sva)
        for tbase, toff, tsz in text:
            i = toff
            while True:
                i = d.find(pat, i, toff + tsz)
                if i < 0:
                    break
                line = None
                if d[i - 5] == 0x68:
                    line = struct.unpack_from('<I', d, i - 4)[0]
                elif d[i - 2] == 0x6a:
                    line = d[i - 1]
                elif d[i + 5] == 0x68:
                    line = struct.unpack_from('<I', d, i + 6)[0]
                elif d[i + 5] == 0x6a:
                    line = d[i + 6]
                rows.append((tbase + (i - toff), s, line))
                i += 1
    rows.sort()
    print('源码路径字符串 %d 个，xref %d 处' % (len(strs), len(rows)))
    for va, s, line in rows:
        print('%08x  %-24s %s' % (va, s.split('\\')[-1], line))


if __name__ == '__main__':
    main(sys.argv[1] if len(sys.argv) > 1
         else '/Users/admin/Documents/crewplus-dev/仙剑奇侠传3/re/PAL3_unpacked.exe')
