//! `effect\gbParticle\*.par` — the gb engine's particle effects.
//!
//! Read straight off `gbengine.dll`, which exports the loaders with
//! their C++ names:
//!
//! * `gbParticleEffect::LoadFromFile` @ 0x1002bca0 — the header and the
//!   three arrays;
//! * `gbPartGroupParam::LoadFromFile` @ 0x1002a7f0;
//! * `gbPartEmitter::LoadFromFile` @ 0x10029cf0;
//! * `gbParticleGroup::LoadFromFile` @ 0x1002b0b0.
//!
//! The header is a fixed sequence of reads:
//!
//! ```text
//! char[6] "sgpar\0"
//! u32     version                 // > 0x10000 adds one field below
//! u32     ?                       // 200 or 400 in every shipped file
//! u32     ?                       // -> effect+0xb4
//! u32     ?                       // -> effect+0xb8
//! f32     ?                       // -> effect+0x118, only when version > 0x10000
//! u32     group_param_count       // -> effect+0x9c
//! u32     emitter_count           // -> effect+0xa0
//! u32     group_count             // -> effect+0x98
//! ```
//!
//! then `group_param_count` group-param records, `emitter_count`
//! emitter records and `group_count` group records, in that order — the
//! order of the three load loops at 0x1002bf16, 0x1002bf4e and
//! 0x1002bf7d.
//!
//! Each record is a fixed size. `gbParticleGroup`'s is legible straight
//! from its loader: `0x40` bytes followed by three dwords, so 76. The
//! other two are long sequences of reads with no easy total, so they
//! were solved from the shipped files instead: the 45 `.par` under
//! `effect\gbParticle` come in four (params, emitters, groups) shapes —
//! (1,1,1), (2,2,2), (2,1,2) and (2,2,1) — which pin all three sizes
//! exactly, and the group's 76 comes out matching the disassembly. See
//! [`GROUP_PARAM_SIZE`] and friends.
//!
//! Inside a group param the reads land on the struct's own offsets, so
//! the file layout mirrors the object's. Walking
//! `gbPartGroupParam::LoadFromFile` gives:
//!
//! ```text
//! 0x000  char[0x20]   名字        "Liquid" / "fire" / "smoke" / …
//! 0x020  u32 × 6
//! 0x038  char[0x100]  贴图        "partexture\\smoke.jpg" / …
//! 0x138  u32 × 13
//! 0x168  4580 字节    轨道 0      {u32, 64, 128, 12, 12, 720, 3636}
//! 0x134c 4580 字节    轨道 1
//! 0x2530 4580 字节    轨道 2
//! 0x3714 4580 字节    轨道 3
//! …      char[0x100] × 2
//! ```
//!
//! The four blocks are keyframed tracks, and
//! `gbPartGroupParam::ComputeParticleParam` (0x1002aed0) says what each
//! drives as a particle ages: track 0 is the size Render multiplies in,
//! track 3 writes the colour bytes, track 1 feeds a spin accumulator
//! together with the random range at 0x148/0x14c, and the dword at
//! 0x160 is the texture's frame count (`frame = (int)(count * t)`).
//! A track is a three-component curve of up to 100 keys, linearly
//! interpolated by the sampler at 0x100299b0; see [`track_curve`] for
//! the key layout.
//!
//! See `switch/re/notes/par_format.md` for the full write-up. Only the
//! name and the texture are exposed here so far — those two hold across
//! all 55 shipped files.
//!
//! An emitter has the same flavour: a 0x40 header, a `char[0x40]` at
//! 0x58, and four of the same 4580-byte blocks.

use std::io::{Error, ErrorKind, Read, Result};

use byteorder::{LittleEndian, ReadBytesExt};

/// `"sgpar\0"`, compared at 0x1002bcf9.
pub const MAGIC: &[u8; 6] = b"sgpar\0";

/// Above this the header carries one more float (0x1002bd8f).
pub const VERSION_WITH_EXTRA: u32 = 0x10000;

/// `gbPartGroupParam` on disk. In memory it is 0x4940.
pub const GROUP_PARAM_SIZE: usize = 21560;

/// `gbPartEmitter` on disk. In memory it is 0x6c78.
pub const EMITTER_SIZE: usize = 32008;

/// `gbParticleGroup` on disk: `0x40` bytes then three dwords. In memory
/// it is 0x90.
pub const GROUP_SIZE: usize = 0x40 + 4 * 3;

/// One `gbParticleGroup`: the record that ties a group param to an
/// emitter.
#[derive(Debug, Clone)]
pub struct ParGroup {
    /// The 0x40 bytes the loader reads over `this` before anything else.
    pub head: [u8; 0x40],
    /// Index into [`ParFile::group_params`] — 0x1002b121 turns it into
    /// `effect+0xa8 + i * 0x4940`.
    pub group_param: u32,
    /// Index into [`ParFile::emitters`] — 0x1002b138, `i * 0x6c78`.
    pub emitter: u32,
    /// Read into `this+0x84` and immediately overwritten from
    /// `effect+0xb0`, so the stored value is dead.
    pub unused: u32,
}

/// A parsed `.par`, cut into its records.
#[derive(Debug, Clone)]
pub struct ParFile {
    pub version: u32,
    /// The three header dwords whose meaning is still open. The first
    /// is 200 or 400 in every shipped file.
    pub unknown: [u32; 3],
    /// Only present when `version > 0x10000`; every shipped file has it,
    /// and it is always a small float (1.0 … 20.0).
    pub extra: Option<f32>,
    pub group_params: Vec<Vec<u8>>,
    pub emitters: Vec<Vec<u8>>,
    pub groups: Vec<ParGroup>,
}

impl ParFile {
    /// Bytes this file should be, given its counts. The shipped files
    /// match exactly, which is what pins the record sizes.
    pub fn expected_len(&self) -> usize {
        let header = 6 + 4 * 4 + if self.extra.is_some() { 4 } else { 0 } + 4 * 3;
        header
            + self.group_params.len() * GROUP_PARAM_SIZE
            + self.emitters.len() * EMITTER_SIZE
            + self.groups.len() * GROUP_SIZE
    }
}

fn bad(msg: &str) -> Error {
    Error::new(ErrorKind::InvalidData, msg.to_string())
}

pub fn read_par<R: Read>(reader: &mut R) -> Result<ParFile> {
    let mut magic = [0u8; 6];
    reader.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(bad("不是 sgpar 文件"));
    }

    let version = reader.read_u32::<LittleEndian>()?;
    let unknown = [
        reader.read_u32::<LittleEndian>()?,
        reader.read_u32::<LittleEndian>()?,
        reader.read_u32::<LittleEndian>()?,
    ];
    let extra = if version > VERSION_WITH_EXTRA {
        Some(reader.read_f32::<LittleEndian>()?)
    } else {
        None
    };

    let group_param_count = reader.read_u32::<LittleEndian>()? as usize;
    let emitter_count = reader.read_u32::<LittleEndian>()? as usize;
    let group_count = reader.read_u32::<LittleEndian>()? as usize;

    let mut group_params = Vec::with_capacity(group_param_count);
    for _ in 0..group_param_count {
        let mut buf = vec![0u8; GROUP_PARAM_SIZE];
        reader.read_exact(&mut buf)?;
        group_params.push(buf);
    }

    let mut emitters = Vec::with_capacity(emitter_count);
    for _ in 0..emitter_count {
        let mut buf = vec![0u8; EMITTER_SIZE];
        reader.read_exact(&mut buf)?;
        emitters.push(buf);
    }

    let mut groups = Vec::with_capacity(group_count);
    for _ in 0..group_count {
        let mut head = [0u8; 0x40];
        reader.read_exact(&mut head)?;
        groups.push(ParGroup {
            head,
            group_param: reader.read_u32::<LittleEndian>()?,
            emitter: reader.read_u32::<LittleEndian>()?,
            unused: reader.read_u32::<LittleEndian>()?,
        });
    }

    Ok(ParFile {
        version,
        unknown,
        extra,
        group_params,
        emitters,
        groups,
    })
}

/// `param+0x24` — the acceleration the velocity picks up each step
/// (`UpdateOneParticle` multiplies it into the velocity at 0x1002b61f).
/// `snow.par` and `huohuo.par` both carry **9.81** here, which is what
/// makes the reading certain; flames use small negatives so they rise.
pub const GROUP_PARAM_GRAVITY: usize = 0x24;

/// `param+0x28` — a speed `ComputeAirResist` compares the particle's
/// own speed against (0x1002adb6).
pub const GROUP_PARAM_DRAG_SPEED: usize = 0x28;

/// `param+0x30` — picks which branch `ComputeAirResist` takes
/// (0x1002adc4). Not a blend mode, though it looks like a flag.
pub const GROUP_PARAM_DRAG_MODE: usize = 0x30;

/// `param+0x15c` — `CreateOneParticle` starts each particle on a random
/// frame in `[0, this - 1]` (0x1002b3bb).
pub const GROUP_PARAM_FIRST_FRAME_RANGE: usize = 0x15c;

/// `param+0x164` — which of the engine's two particle render effects
/// this group draws with. `gbPartGroupParam` picks the file by it at
/// 0x1002ad40:
///
/// ```text
/// 0   -> gfxscript\p_add.gbf     加法混合
/// !=0 -> gfxscript\p_alpha.gbf   alpha 混合
/// ```
///
/// 65 of the 68 shipped group params take the additive path; the three
/// that do not are all named `Snow`.
pub const GROUP_PARAM_BLEND: usize = 0x164;

/// The dword at 0x160: how many frames the group's texture has.
/// `ComputeParticleParam` picks the particle's frame with
/// `clamp((int)(count * t), 0, count - 1)`.
pub const GROUP_PARAM_FRAME_COUNT: usize = 0x160;

/// Where the four keyframed tracks start inside a group param **in the
/// file**. The in-memory bases the disassembly shows (0x168, 0x134c,
/// 0x2530, 0x3714) are 720 bytes closer together per track, because the
/// loader drops one 720-byte chunk of each track on the way in.
///
/// Each track carries its own label eight bytes in, and those labels are
/// what the tracks are named after: `Size graph: m/t`,
/// `Rotation graph: d/t`, `VertexAlpha graph: d/t`, `RgbColor graph: d/t`.
pub const GROUP_PARAM_TRACKS: [usize; 4] = [0x168, 0x161c, 0x2ad0, 0x3f84];

/// What each track drives, in the file's own words (the label at
/// `track + 8`).
pub const TRACK_LABELS: [&str; 4] = [
    "Size graph: m/t",
    "Rotation graph: d/t",
    "VertexAlpha graph: d/t",
    "RgbColor graph: d/t",
];

/// Where a track's label sits, relative to the track.
pub const TRACK_LABEL: usize = 8;

/// A track's first key, relative to the track's own start.
///
/// The sampler at 0x100299b0 reads key `i`'s time at `0x3b0 + 12i` and
/// key `i-1`'s at `0x3a4 + 12i`, and the matching values four bytes
/// further along (`0x3b4 + 12i`, `0x3a8 + 12i`), so a key is twelve
/// bytes and the run starts here.
pub const TRACK_FIRST_KEY: usize = 0x3b0;

/// One key: `{ f32 time, f32 value, f32 unused }`.
pub const TRACK_KEY_SIZE: usize = 12;

/// Distance between a track's three components in the file: twenty keys
/// each. (In memory they sit 0x4bc apart — the sampler's out-of-range
/// branch falls back to `0x3b4`, `0x870` and `0xd2c` — because the
/// loader expands each into a 101-slot array.)
pub const TRACK_COMPONENT_STRIDE: usize = 0xf0;

/// Keys a component holds in the file.
pub const TRACK_MAX_KEYS: usize = 20;

/// How a group's particles are blended into the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParBlend {
    /// `gfxscript\p_add.gbf`.
    Additive,
    /// `gfxscript\p_alpha.gbf`.
    Alpha,
}

/// Read [`GROUP_PARAM_BLEND`].
pub fn group_param_blend(record: &[u8]) -> ParBlend {
    let raw = u32::from_le_bytes(
        record[GROUP_PARAM_BLEND..GROUP_PARAM_BLEND + 4]
            .try_into()
            .unwrap_or([0; 4]),
    );
    if raw == 0 {
        ParBlend::Additive
    } else {
        ParBlend::Alpha
    }
}

/// A track's label, which says what it drives.
pub fn track_label(record: &[u8], track: usize) -> String {
    c_string(record, (track + TRACK_LABEL, 0x20))
}

/// One component of one track, as `(time, value)` pairs.
///
/// Times are normalised to the particle's life and ascend; the shipped
/// curves end at exactly 1.0. Unused slots after the last key are zero
/// (or MSVC's `0xCC` fill in a few files), so the run stops as soon as
/// the time stops advancing.
pub fn track_curve(record: &[u8], track: usize, component: usize) -> Vec<(f32, f32)> {
    let base = track + TRACK_FIRST_KEY + component * TRACK_COMPONENT_STRIDE;
    let mut keys: Vec<(f32, f32)> = Vec::new();
    for i in 0..TRACK_MAX_KEYS {
        let offset = base + i * TRACK_KEY_SIZE;
        if offset + 8 > record.len() {
            break;
        }
        let time = f32::from_le_bytes(record[offset..offset + 4].try_into().unwrap());
        let value = f32::from_le_bytes(record[offset + 4..offset + 8].try_into().unwrap());
        if !time.is_finite() || !value.is_finite() {
            break;
        }
        if i > 0 && time <= keys[i - 1].0 {
            break;
        }
        keys.push((time, value));
        if time >= 1.0 {
            break;
        }
    }
    keys
}

/// Offset and length of a group param's name — the 0x20 bytes the
/// loader reads first (0x1002a80a).
pub const GROUP_PARAM_NAME: (usize, usize) = (0x00, 0x20);

/// Offset and length of a group param's texture, the `char[0x100]` at
/// 0x38 (0x1002a888). Always a `partexture\…` path in the shipped
/// files.
pub const GROUP_PARAM_TEXTURE: (usize, usize) = (0x38, 0x100);

fn c_string(record: &[u8], (offset, len): (usize, usize)) -> String {
    let end = (offset + len).min(record.len());
    let field = &record[offset.min(end)..end];
    let stop = field.iter().position(|b| *b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..stop]).to_string()
}

/// The group param's name.
pub fn group_param_name(record: &[u8]) -> String {
    c_string(record, GROUP_PARAM_NAME)
}

/// The texture the group's particles are drawn with, as written in the
/// file (a backslash path relative to `basedata`'s effect folder).
pub fn group_param_texture(record: &[u8]) -> String {
    c_string(record, GROUP_PARAM_TEXTURE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal file, built to the layout above.
    fn synthetic(params: usize, emitters: usize, groups: usize) -> Vec<u8> {
        let mut data = MAGIC.to_vec();
        data.extend_from_slice(&0x10001u32.to_le_bytes());
        for v in [400u32, 0, 1] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        data.extend_from_slice(&2.0f32.to_le_bytes());
        data.extend_from_slice(&(params as u32).to_le_bytes());
        data.extend_from_slice(&(emitters as u32).to_le_bytes());
        data.extend_from_slice(&(groups as u32).to_le_bytes());
        data.resize(data.len() + params * GROUP_PARAM_SIZE, 0);
        data.resize(data.len() + emitters * EMITTER_SIZE, 0);
        for _ in 0..groups {
            data.resize(data.len() + 0x40, 0);
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
        }
        data
    }

    #[test]
    fn the_record_sizes_are_the_ones_the_shipped_files_pin() {
        // 45 files in four shapes: (1,1,1) = 53682, (2,2,2) = 107326,
        // (2,1,2) = 75318 and (2,2,1) = 107250, all with a 38-byte
        // header. Those four equations have exactly one solution.
        let header = 38;
        let size = |p: usize, e: usize, g: usize| {
            header + p * GROUP_PARAM_SIZE + e * EMITTER_SIZE + g * GROUP_SIZE
        };
        assert_eq!(size(1, 1, 1), 53682);
        assert_eq!(size(2, 2, 2), 107326);
        assert_eq!(size(2, 1, 2), 75318);
        assert_eq!(size(2, 2, 1), 107250);
        // And the group's size is legible straight from its loader.
        assert_eq!(GROUP_SIZE, 76);
    }

    #[test]
    fn a_file_parses_into_exactly_its_records() {
        for (p, e, g) in [(1, 1, 1), (2, 2, 2), (2, 1, 2), (2, 2, 1)] {
            let data = synthetic(p, e, g);
            let par = read_par(&mut data.as_slice()).unwrap();
            assert_eq!(par.group_params.len(), p);
            assert_eq!(par.emitters.len(), e);
            assert_eq!(par.groups.len(), g);
            assert_eq!(par.expected_len(), data.len());
        }
    }

    #[test]
    fn a_file_that_is_not_a_par_is_rejected() {
        assert!(read_par(&mut b"not a par file".as_slice()).is_err());
    }
}
