import sys, pefile
from capstone import *
pe = pefile.PE(sys.argv[1])
base = pe.OPTIONAL_HEADER.ImageBase
va = int(sys.argv[2],16); size=int(sys.argv[3],16) if len(sys.argv)>3 else 0x200
data = pe.get_data(va-base, size)
md = Cs(CS_ARCH_X86, CS_MODE_32)
for i in md.disasm(data, va):
    print("%08x  %-8s %s" % (i.address, i.mnemonic, i.op_str))
