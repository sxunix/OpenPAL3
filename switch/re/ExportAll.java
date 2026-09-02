// Export every function's decompiled C plus a symbol/xref index to text files.
// Usage (headless):
//   analyzeHeadless <proj> pal3 -import PAL3_unpacked.exe -scriptPath <dir> -postScript ExportAll.java <outdir>
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.address.*;
import ghidra.program.model.symbol.*;
import ghidra.program.model.data.*;
import java.io.*;
import java.util.*;

public class ExportAll extends GhidraScript {
    @Override
    public void run() throws Exception {
        String[] args = getScriptArgs();
        File outDir = new File(args.length > 0 ? args[0] : ".");
        outDir.mkdirs();

        DecompInterface ifc = new DecompInterface();
        DecompileOptions opts = new DecompileOptions();
        ifc.setOptions(opts);
        ifc.openProgram(currentProgram);

        FunctionManager fm = currentProgram.getFunctionManager();
        int total = fm.getFunctionCount();
        println("functions: " + total);

        try (PrintWriter c = new PrintWriter(new BufferedWriter(new FileWriter(new File(outDir, "decompiled.c"))));
             PrintWriter idx = new PrintWriter(new BufferedWriter(new FileWriter(new File(outDir, "functions.txt"))))) {
            int n = 0;
            for (Function f : fm.getFunctions(true)) {
                if (monitor.isCancelled()) break;
                n++;
                if (n % 500 == 0) println("  " + n + "/" + total);
                String name = f.getName();
                Address ea = f.getEntryPoint();
                long size = f.getBody().getNumAddresses();
                idx.printf("%s %s size=%d callers=%d%n", ea, name, size, f.getCallingFunctions(monitor).size());
                if (f.isThunk() || f.isExternal()) continue;
                DecompileResults res = ifc.decompileFunction(f, 60, monitor);
                c.printf("%n// ==== %s @ %s (size %d) ====%n", name, ea, size);
                if (res != null && res.decompileCompleted() && res.getDecompiledFunction() != null) {
                    c.print(res.getDecompiledFunction().getC());
                } else {
                    c.printf("// decompile failed: %s%n", res == null ? "null" : res.getErrorMessage());
                }
            }
        }

        // strings with the functions that reference them
        try (PrintWriter s = new PrintWriter(new BufferedWriter(new FileWriter(new File(outDir, "strings_xref.txt"))))) {
            Listing listing = currentProgram.getListing();
            DataIterator it = listing.getDefinedData(true);
            ReferenceManager rm = currentProgram.getReferenceManager();
            while (it.hasNext() && !monitor.isCancelled()) {
                Data d = it.next();
                if (!(d.getDataType() instanceof StringDataType) && !(d.getDataType() instanceof TerminatedStringDataType)
                        && !(d.getDataType() instanceof UnicodeDataType)) continue;
                Object v = d.getValue();
                if (v == null) continue;
                StringBuilder refs = new StringBuilder();
                for (Reference r : rm.getReferencesTo(d.getAddress())) {
                    Function f = fm.getFunctionContaining(r.getFromAddress());
                    refs.append(f == null ? r.getFromAddress().toString() : f.getName() + "@" + f.getEntryPoint()).append(' ');
                }
                s.printf("%s\t%s\t%s%n", d.getAddress(), v.toString().replace("\n", "\\n").replace("\t", "\\t"), refs);
            }
        }
        ifc.dispose();
        println("done");
    }
}
