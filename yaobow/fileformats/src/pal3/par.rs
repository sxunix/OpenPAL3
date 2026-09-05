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
//! This module only cuts the file into its records; what the bytes
//! inside a group param or an emitter mean is still open.

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

/// The name a group param opens with: `gbPartGroupParam::LoadFromFile`
/// reads 0x100 bytes first, and every shipped record starts with a
/// NUL-terminated string there (`shenshu Particle`, …).
pub fn group_param_name(record: &[u8]) -> String {
    let end = record[..0x100.min(record.len())]
        .iter()
        .position(|b| *b == 0)
        .unwrap_or(0);
    String::from_utf8_lossy(&record[..end]).to_string()
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
