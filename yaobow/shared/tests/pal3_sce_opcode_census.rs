//! Walk every `.sce` in a real PAL3 install and tally which opcodes the
//! scripts actually use.
//!
//! The vm panics on an opcode it has no arm for, so "which opcodes occur
//! in shipped scripts" is the list that decides whether the game can be
//! played through at all. The instruction stream has no framing — an
//! instruction's length comes from its signature — so the walk needs the
//! same parameter table the vm uses, and any disagreement shows up as a
//! nonsense opcode a few instructions later.
//!
//! Point it at an installed game and run it:
//!
//! ```text
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 \
//!     cargo test -p shared --test pal3_sce_opcode_census -- --nocapture
//! ```
//!
//! Without the variable the test skips.

use std::collections::BTreeMap;
use std::path::PathBuf;

use shared::openpal3::loaders::sce_loader::sce_load_from_file;

/// Parameter kinds, matching the type bytes in the original's dispatch
/// table at `0xd8c464` (see `switch/re/notes/sce_opcodes.md`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Param {
    /// 4-byte literal, or a 2-byte variable index when the user-variable
    /// mask has this parameter's bit set.
    Int,
    /// Always 2 bytes: a variable index the command writes back to.
    Var,
    Float,
    /// 4-byte jump target.
    Jump,
    /// 2-byte length then that many bytes.
    Str,
    /// 2-byte count, then per item a type byte and a value.
    List,
}

use Param::*;

/// Signature per opcode, in declaration order. Taken from the dispatch
/// table's type bytes, cross-checked against Pal3.Unity's command
/// classes. Opcodes absent here are unknown to the original too.
fn signature(op: u16) -> Option<&'static [Param]> {
    Some(match op {
        1 => &[Float],
        2 => &[Int],
        3 => &[Jump],
        4 => &[],
        5 => &[Int],
        6..=9 | 11 => &[Var, Int],
        10 => &[Var, Int],
        12 => &[Jump],
        13 | 14 | 15 => &[Var, Int],
        16 => &[Int],
        17 => &[Var, Int],
        18 => &[],
        19 => &[Var, Int, Int],
        20 => &[Int, Int, Int, Int],
        21 => &[Int, Int, Int],
        22 => &[Int, Str, Int],
        23 | 24 => &[Int, Int],
        25 | 26 => &[],
        27 => &[Int],
        28 | 29 => &[Int, Int],
        30 => &[Int],
        31 => &[Float, Float, Float],
        32 => &[Float, Float, Int],
        33 => &[Float, Float, Float, Int],
        34 => &[Float, Float, Float, Float, Int],
        35 => &[Float, Float, Float, Int],
        36 => &[Float, Float, Float, Float, Float, Float],
        37 => &[Int],
        38 | 39 => &[],
        40 | 41 => &[Int, Int, Int],
        42 => &[Int],
        43 | 44 => &[Int, Int],
        45 => &[Int, Int, Int],
        46 => &[Int, Int],
        47 | 48 => &[Int],
        49 => &[Var],
        50 => &[Var, Int],
        51 => &[Int, Int],
        52 | 53 => &[Var],
        54 => &[Int, Int],
        55 => &[],
        60 => &[Int, Int],
        61 => &[Int],
        62 => &[Str],
        63 => &[Str, Str],
        64 => &[Int, Str],
        65 => &[List],
        66 => &[Var],
        67 => &[Int, Str, Int],
        68 => &[Str],
        69 | 70 => &[],
        71 => &[Int],
        72 => &[Int, Int],
        73 => &[Int, Int, Int, Int],
        74 => &[Int, Int],
        75 => &[],
        76 => &[Str],
        77 => &[Var],
        78 => &[Int],
        79 => &[Str, Int],
        80 => &[Int, Int, Int, Int, Int, Int],
        81 => &[],
        82 => &[Int],
        83 => &[],
        84 => &[Int],
        85 => &[Int, Int],
        86 => &[Str, Int],
        87 | 88 => &[Int],
        89 => &[Float, Float, Float],
        90 => &[Int, Float, Float, Float, Float],
        91 | 92 => &[],
        100 => &[Str],
        101 => &[Int],
        102 | 103 => &[Int],
        104 | 105 => &[],
        106 | 107 => &[Int],
        108 => &[Var],
        109 | 110 | 111 => &[Int],
        112 | 113 | 114 => &[],
        115 => &[Str],
        116 => &[Int, Str],
        117 => &[Int, Int, Int],
        118 => &[Float, Float],
        119 => &[Str, Int, Int, Int],
        120..=123 => &[Int],
        124 => &[Int],
        125 => &[Int, Int],
        126 => &[Str, Int, Var],
        127 => &[Int, Int],
        128 => &[Int, Int, Int],
        129 => &[Int],
        130 => &[Var, Var],
        131 => &[],
        132 => &[Var],
        133 => &[Str, Int],
        134 => &[],
        135 | 136 | 137 | 138 => &[Int],
        139 => &[Float, Int, Int, Int],
        140 | 141 => &[Int],
        142 => &[Float, Float, Float],
        143 | 144 => &[Int],
        145 => &[Int, Int],
        146 => &[Int],
        147 => &[],
        148 | 149 => &[Int],
        150 => &[Int, Str],
        151 | 152 => &[Int],
        153 => &[],
        154 => &[Int, Int, Int],
        155 => &[Float],
        156 => &[],
        157 => &[Str],
        158 => &[Int],
        159 | 160 => &[],
        201 => &[Int, Int, Int, Int],
        202 | 203 => &[Int, Int],
        204 => &[Int],
        205 => &[Int, Int],
        206 => &[Int, Float],
        207 => &[Int, Int],
        208 => &[Int, Float],
        209 | 210 => &[Int, Int],
        211 | 212 => &[],
        213 => &[Int, Float, Float, Float],
        214 => &[Int, Int, Int, Int],
        215 => &[],
        221 => &[Int],
        222 => &[],
        250 | 251 => &[Int],
        252 => &[],
        _ => return None,
    })
}

/// Opcodes the vm has an arm for, whether real or a no-op stub. Kept in
/// sync by hand with `scripting/sce/vm.rs`; the census fails if a script
/// uses anything outside it, since that is a panic at runtime.
const HANDLED: &[u16] = &[
    1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29,
    30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 41, 42, 43, 46, 47, 48, 49, 50, 51, 52, 53, 54, 62, 63,
    65, 66, 67, 68, 69, 70, 71, 72, 73, 74, 76, 77, 78, 79, 80, 81, 82, 83, 85, 86, 87, 88, 89, 90,
    91, 100, 101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113, 115, 116, 117, 118,
    119, 120, 121, 122, 123, 124, 125, 126, 127, 128, 130, 131, 132, 133, 134, 135, 136, 137, 138,
    140, 141, 142, 143, 144, 145, 146, 147, 148, 149, 150, 152, 153, 154, 155, 156, 157, 158, 159,
    201, 202, 203, 204, 205, 206, 207, 208, 209, 210, 211, 212, 214, 221, 250, 251,
];

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn u16(&mut self) -> Option<u16> {
        let v = self.data.get(self.pos..self.pos + 2)?;
        self.pos += 2;
        Some(u16::from_le_bytes([v[0], v[1]]))
    }

    fn skip(&mut self, n: usize) -> Option<()> {
        (self.pos + n <= self.data.len()).then(|| self.pos += n)
    }
}

/// Walk one proc's instruction bytes, calling `seen` for each opcode.
/// Returns `Err` at the first instruction it cannot make sense of.
pub fn walk(inst: &[u8], mut seen: impl FnMut(u16)) -> Result<(), String> {
    let mut c = Cursor { data: inst, pos: 0 };
    let mut trail: Vec<u16> = vec![];
    while c.pos < inst.len() {
        let at = c.pos;
        let op = c
            .u16()
            .ok_or_else(|| format!("truncated opcode after {trail:?}"))?;
        let mask = c.u16().ok_or("truncated user-variable mask")?;
        let sig =
            signature(op).ok_or_else(|| format!("unknown opcode {op} at {at} after {trail:?}"))?;
        trail.push(op);
        if trail.len() > 4 {
            trail.remove(0);
        }
        seen(op);
        // Parameters are stored last-first: the original pushes them in
        // source order onto the stack at 0xd8c36c and the handler pops
        // them with FUN_00434c00, so the file holds them reversed. The
        // user-variable mask's bit i still refers to declaration index i.
        for (i, p) in sig.iter().enumerate().rev() {
            let is_var = *p == Var || (*p == Int && mask & (1 << i) != 0);
            if is_var {
                c.skip(2).ok_or("truncated variable index")?;
                continue;
            }
            match p {
                Int | Float | Jump => c.skip(4).ok_or("truncated 4-byte parameter")?,
                Str => {
                    let len = c.u16().ok_or("truncated string length")? as usize;
                    c.skip(len)
                        .ok_or_else(|| format!("truncated string after {trail:?}"))?
                }
                List => {
                    let n = c.u16().ok_or("truncated list length")?;
                    for _ in 0..n {
                        c.skip(1).ok_or("truncated list item type")?;
                        let len = c.u16().ok_or("truncated list item length")? as usize;
                        c.skip(len).ok_or("truncated list item")?;
                    }
                }
                Var => unreachable!("handled above"),
            }
        }
    }
    Ok(())
}

#[test]
fn every_opcode_the_shipped_scripts_use_has_an_arm() {
    let Ok(assets) = std::env::var("YAOBOW_PAL3_ASSETS") else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS to an installed PAL3 directory");
        return;
    };

    let vfs = packfs::init_virtual_fs(&assets, None);
    let mut files: Vec<PathBuf> = vec![PathBuf::from("/basedata/basedata/init.sce")];
    for entry in std::fs::read_dir(PathBuf::from(&assets).join("scene")).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(stem) = name.strip_suffix(".cpk") {
            files.push(PathBuf::from(format!("/scene/{stem}/{stem}.sce")));
        }
    }

    let mut counts: BTreeMap<u16, usize> = BTreeMap::new();
    let mut walked = 0usize;
    let mut broken = vec![];
    for path in &files {
        let sce = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            sce_load_from_file(&vfs, path)
        }));
        let Ok(sce) = sce else { continue };
        for proc in sce.procs.values() {
            match walk(&proc.inst, |op| *counts.entry(op).or_default() += 1) {
                Ok(()) => walked += 1,
                Err(e) => broken.push(format!("{}::{} — {e}", path.display(), proc.name)),
            }
        }
    }

    assert!(walked > 0, "no procs walked; is {assets} a PAL3 install?");
    println!("走通 {walked} 个 proc，用到 {} 个指令号", counts.len());

    let unhandled: Vec<_> = counts
        .iter()
        .filter(|(op, _)| !HANDLED.contains(op))
        .map(|(op, n)| format!("{op} ×{n}"))
        .collect();
    for line in &broken {
        println!("走不通: {line}");
    }
    println!("脚本用到但 vm 没有分支的指令: {unhandled:?}");

    // T01/T02 的「客栈掌柜住宿」两个 proc 只有 34 字节：opcode 119 的
    // 字符串之后就没了，三个脚本 id 参数不在自己的 proc 里。原版把整个
    // 文件当一块缓冲区顺序读，会接着读进下一个 proc 的 12 个字节；
    // 按 proc 切片的读法读不到。这是随游戏出厂的脚本数据缺陷，不是
    // 参数表的问题。
    const KNOWN_TRUNCATED: usize = 2;
    assert!(
        broken.len() <= KNOWN_TRUNCATED,
        "多了走不通的 proc，参数表可能有错: {} / {walked}\n{}",
        broken.len(),
        broken[..broken.len().min(10)].join("\n")
    );
    assert!(
        unhandled.is_empty(),
        "这些指令号会让 vm panic: {unhandled:?}"
    );
}

/// Like [`walk`], but hands the caller each instruction's offset,
/// opcode and decoded arguments. Used by the ad-hoc proc dumper.
pub fn walk_verbose(inst: &[u8], mut seen: impl FnMut(usize, u16, String)) -> Result<(), String> {
    let mut c = Cursor { data: inst, pos: 0 };
    while c.pos < inst.len() {
        let at = c.pos;
        let op = c.u16().ok_or("truncated opcode")?;
        let mask = c.u16().ok_or("truncated mask")?;
        let sig = signature(op).ok_or_else(|| format!("unknown opcode {op} at {at}"))?;
        let mut args = vec![String::new(); sig.len()];
        for (i, p) in sig.iter().enumerate().rev() {
            let is_var = *p == Var || (*p == Int && mask & (1 << i) != 0);
            if is_var {
                let v = c.u16().ok_or("truncated var")? as i16;
                args[i] = format!("var{v}");
                continue;
            }
            match p {
                Int | Jump => {
                    let b = c.data.get(c.pos..c.pos + 4).ok_or("truncated int")?;
                    c.pos += 4;
                    args[i] = i32::from_le_bytes([b[0], b[1], b[2], b[3]]).to_string();
                }
                Float => {
                    let b = c.data.get(c.pos..c.pos + 4).ok_or("truncated float")?;
                    c.pos += 4;
                    args[i] = format!("{:.2}", f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
                }
                Str => {
                    let len = c.u16().ok_or("truncated len")? as usize;
                    let b = c.data.get(c.pos..c.pos + len).ok_or("truncated str")?;
                    c.pos += len;
                    args[i] = format!("{:?}", String::from_utf8_lossy(&b[..len.saturating_sub(1)]));
                }
                List => {
                    let n = c.u16().ok_or("truncated list")?;
                    for _ in 0..n {
                        c.skip(1).ok_or("t")?;
                        let len = c.u16().ok_or("t")? as usize;
                        c.skip(len).ok_or("t")?;
                    }
                    args[i] = format!("[{n} items]");
                }
                Var => unreachable!(),
            }
        }
        seen(at, op, args.join(", "));
    }
    Ok(())
}

/// Find every proc that uses a given opcode, for picking a cheap place to
/// verify a newly implemented command in the running game:
///
/// ```sh
/// YAOBOW_PAL3_ASSETS=... YAOBOW_FIND_OP=135 \
///   cargo test -p shared --test pal3_sce_opcode_census find_op -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn find_the_procs_that_use_an_opcode() {
    let (Ok(assets), Ok(want)) = (
        std::env::var("YAOBOW_PAL3_ASSETS"),
        std::env::var("YAOBOW_FIND_OP"),
    ) else {
        eprintln!("set YAOBOW_PAL3_ASSETS and YAOBOW_FIND_OP");
        return;
    };
    let want: u16 = want.trim().parse().unwrap();

    let vfs = packfs::init_virtual_fs(&assets, None);
    let mut files: Vec<PathBuf> = vec![PathBuf::from("/basedata/basedata/init.sce")];
    for entry in std::fs::read_dir(PathBuf::from(&assets).join("scene")).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        if let Some(stem) = name.strip_suffix(".cpk") {
            files.push(PathBuf::from(format!("/scene/{stem}/{stem}.sce")));
        }
    }

    for path in &files {
        let sce = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            sce_load_from_file(&vfs, path)
        }));
        let Ok(sce) = sce else { continue };
        for (id, proc) in &sce.procs {
            let mut lines = vec![];
            let _ = walk_verbose(&proc.inst, |pos, op, args| {
                if op == want {
                    lines.push(format!("      @{pos} {args}"));
                }
            });
            if !lines.is_empty() {
                println!(
                    "{} proc {id} 「{}」 {} 字节",
                    path.display(),
                    proc.name,
                    proc.inst.len()
                );
                for l in lines {
                    println!("{l}");
                }
            }
        }
    }
}

/// Disassemble one proc:
///
/// ```sh
/// YAOBOW_PAL3_ASSETS=... YAOBOW_DUMP_PROC=Q06:1204 \
/// # 或者 YAOBOW_DUMP_PROC=ui/bigmap/bigmap:2000
///   cargo test -p shared --test pal3_sce_opcode_census dump_one_proc -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn dump_one_proc() {
    let (Ok(assets), Ok(what)) = (
        std::env::var("YAOBOW_PAL3_ASSETS"),
        std::env::var("YAOBOW_DUMP_PROC"),
    ) else {
        eprintln!("set YAOBOW_PAL3_ASSETS and YAOBOW_DUMP_PROC=<scene>:<proc id>");
        return;
    };
    let (scene, id) = what.split_once(':').expect("want <scene>:<proc id>");
    let id: u32 = id.trim().parse().unwrap();

    let vfs = packfs::init_virtual_fs(&assets, None);
    // `@` 开头的当成完整 vfs 路径（`@/basedata/basedata/init.sce`），带斜杠
    // 的按 basedata 下的相对路径算（`ui/bigmap/bigmap`），其余按场景名。
    let path = if let Some(raw) = scene.strip_prefix('@') {
        PathBuf::from(raw.to_string())
    } else if scene.contains('/') {
        PathBuf::from(format!("/basedata/basedata/{scene}.sce"))
    } else {
        PathBuf::from(format!("/scene/{scene}/{scene}.sce"))
    };
    let sce = sce_load_from_file(&vfs, &path);
    let proc = &sce.procs[&id];
    println!(
        "{scene} proc {id} 「{}」 {} 字节",
        proc.name,
        proc.inst.len()
    );
    let r = walk_verbose(&proc.inst, |pos, op, args| {
        println!("  @{pos:5} {op:3}  {args}")
    });
    if let Err(e) = r {
        println!("  走不通: {e}");
    }
}
