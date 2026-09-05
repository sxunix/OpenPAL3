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
//! An emitter is the same idea with six tracks, and it labels them too:
//! `DirectionEmitter graph: x,y,z/t`, `ElementRandomVelocity graph:
//! m/s/t`, `EmissionRate Graph: rate/t`, `ElementLifeTime Graph: t/t`,
//! `SizeMultiplier Graph: */t` and `RotationMultiplier Graph: */t`.
//! `gbPartEmitter::ComputeEmitterData` samples the first four over the
//! emitter's own period and hands them to `gbParticleGroup::Update`,
//! which is where the emission rate comes from — see [`EMITTER_TRACKS`].

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
/// own speed against (0x1002adb6), below which it does not drag.
pub const GROUP_PARAM_DRAG_SPEED: usize = 0x28;

/// `param+0x30` — which drag `ComputeAirResist` applies (0x1002adc4):
/// non-zero scales it by the particle's current size (`unit+0x40`, the
/// Size track), zero uses plain linear drag.
pub const GROUP_PARAM_DRAG_MODE: usize = 0x30;

/// `param+0x34` — the drag coefficient both branches of
/// `ComputeAirResist` multiply by (0x1002ade6, 0x1002ae31). Snow uses
/// 0.4, smoke and flames 3 to 16.
pub const GROUP_PARAM_DRAG: usize = 0x34;

/// `param+0x20` — **written by the loader and never read.** A scan of
/// the whole particle region of `gbengine.dll` finds no instruction
/// that reads it off a group param, and the shipped data agrees: 19 of
/// the 68 group params hold `0xCDCDCDCD` here, MSVC's uninitialised
/// fill. An effect would not survive that value being used.
pub const GROUP_PARAM_UNUSED: usize = 0x20;

/// `param+0x2c` — no reader found either, though unlike
/// [`GROUP_PARAM_UNUSED`] it is always a clean 0 or 1 (8 of 68 are 1).
/// Possibly a flag the editor keeps for itself.
pub const GROUP_PARAM_UNKNOWN_FLAG: usize = 0x2c;

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

/// Where the emitter's six tracks start in the file, 5300 apart like the
/// group param's. In memory they sit at 0x120, 0x1304, 0x24e8, 0x36cc, …
/// — the offsets `ComputeEmitterData` uses.
pub const EMITTER_TRACKS: [usize; 6] = [0xd0, 0x1584, 0x2a38, 0x3eec, 0x53a0, 0x6854];

/// The emitter's own labels, in the file's words.
pub const EMITTER_TRACK_LABELS: [&str; 6] = [
    "DirectionEmitter graph: x,y,z/t",
    "ElementRandomVelocity graph: m/s/t",
    "EmissionRate Graph: rate/t",
    "ElementLifeTime Graph: t/t",
    "SizeMultiplier Graph: */t",
    "RotationMultiplier Graph: */t",
];

/// `emitter+0x00` — its name, e.g. `fire Emitter`.
pub const EMITTER_NAME: (usize, usize) = (0x00, 0x40);

/// `emitter+0x5c` — the `.csf` mesh a point-cloud emitter scatters over
/// (`csf\star.csf`, `csf\yuan.CSF`). Empty for the box and radius
/// shapes.
pub const EMITTER_MESH: (usize, usize) = (0x5c, 0x40);

/// `emitter+0x40` — where the emitter sits, added to every particle's
/// start position by `ComputeParticleInitPos`.
pub const EMITTER_ORIGIN: usize = 0x40;

/// `emitter+0x4c` — the spread `ComputeParticleInitVel` randomises the
/// direction within, as `rand(-v, v)` per axis.
pub const EMITTER_SPREAD: usize = 0x4c;

/// 文件 `0x58` — 发射形状，`ComputeParticleInitPos` 的三个分支：0 是盒
/// （[`EMITTER_BOX_MIN`] / [`EMITTER_BOX_MAX`]），1 是球壳
/// （[`EMITTER_RADII`]），其余按 [`EMITTER_MESH`] 那个 `.csf` 的点云撒。
/// 实机 65 个发射器里三种分别是 26 / 12 / 27 个。
///
/// 注意这里给的是**文件**偏移，不是对象偏移。`gbPartEmitter::LoadFromFile`
/// 是一连串 `Read(&this->field, size)`，读入顺序和对象里的排列并不一致：
/// 形状在对象里是 `+0x98`，在文件里排在网格名前面，落在 `0x58`。
pub const EMITTER_SHAPE: usize = 0x58;

/// 文件 `0xa0` 和 `0xa4`（对象 `+0x9c` / `+0xa0`）——`ComputeEmitterData`
/// 拿它们的差当一个发射周期，再用经过的时间除出归一化进度。实机里后一个
/// 恒为 0，所以周期就是前一个。
pub const EMITTER_PERIOD: (usize, usize) = (0xa0, 0xa4);

/// 文件 `0xa8`（对象 `+0xa4`）——过了一个周期还继续发。
/// `ComputeEmitterData` 在 `n > 1` 时看它，为 0 就整个停掉；
/// `gbPartEmitter::SetLoopEmit` 写的也是它。
pub const EMITTER_LOOP: usize = 0xa8;

/// 文件 `0xb0`（对象 `+0xc4`/`+0xc8`）——球壳形状的两个半径。
/// `ComputeParticleInitPos` 抛一次硬币，一半用第一个当上界，一半用两个的
/// 和，然后在 `[0, 上界]` 里取半径。
pub const EMITTER_RADII: usize = 0xb0;

/// 文件 `0xb8`（对象 `+0xac`）——盒形状三个轴的下界。
pub const EMITTER_BOX_MIN: usize = 0xb8;

/// 文件 `0xc4`（对象 `+0xb8`）——盒形状三个轴的上界。上下界连在一起是
/// `LoadFromFile` 的一次 `Read(&this->0xac, 0x18)`，六个浮点。
pub const EMITTER_BOX_MAX: usize = 0xc4;

/// The emitter's name.
pub fn emitter_name(record: &[u8]) -> String {
    c_string(record, EMITTER_NAME)
}

/// The emitter's shape mesh, empty when it does not use one.
pub fn emitter_mesh(record: &[u8]) -> String {
    c_string(record, EMITTER_MESH)
}

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

/// A track's label, which says what it drives. The field is
/// `char[0x20]`, so the longer labels come back truncated —
/// `ElementRandomVelocity graph: m/s/t` loses its last two characters.
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

/// A keyframed curve: `(time, value)` with time normalised to whatever
/// the track measures against, always ending at 1.0.
pub type Curve = Vec<(f32, f32)>;

/// Sample a curve the way `0x100299b0` does: hold the ends, linearly
/// interpolate between the bracketing keys.
pub fn sample(curve: &Curve, t: f32) -> f32 {
    match curve.len() {
        0 => 0.,
        1 => curve[0].1,
        _ => {
            if t <= curve[0].0 {
                return curve[0].1;
            }
            for pair in curve.windows(2) {
                let (t0, v0) = pair[0];
                let (t1, v1) = pair[1];
                if t < t1 {
                    let span = t1 - t0;
                    let u = if span > 0. { (t - t0) / span } else { 0. };
                    return v0 + (v1 - v0) * u;
                }
            }
            curve.last().unwrap().1
        }
    }
}

/// What one `gbPartGroupParam` says about the particles it owns.
#[derive(Debug, Clone)]
pub struct ParGroupParam {
    pub name: String,
    /// As written in the file, e.g. `partexture\huo3.jpg`.
    pub texture: String,
    pub blend: ParBlend,
    /// How many frames the texture is split into.
    pub frames: u32,
    /// A new particle starts on `rand(0, this - 1)`.
    pub first_frame_range: u32,
    /// Added to the velocity each step; `snow.par` has 9.81 here.
    pub gravity: f32,
    /// Below this speed `ComputeAirResist` leaves the particle alone.
    pub drag_speed: f32,
    /// Drag scales with the particle's current size rather than being
    /// linear.
    pub drag_by_size: bool,
    pub drag: f32,
    /// `Size graph: m/t`.
    pub size: Curve,
    /// `Rotation graph: d/t`.
    pub rotation: Curve,
    /// `VertexAlpha graph: d/t`, 0..1 — the colour's alpha is this × 255.
    pub alpha: Curve,
    /// `RgbColor graph: d/t`, three 0..255 curves.
    pub color: [Curve; 3],
}

/// What one `gbPartEmitter` says about where particles come from.
#[derive(Debug, Clone)]
pub struct ParEmitter {
    pub name: String,
    /// The `.csf` a point-cloud emitter scatters over, when it uses one.
    pub mesh: String,
    pub origin: [f32; 3],
    pub spread: [f32; 3],
    pub shape: u32,
    /// 盒形状的下界与上界，`shape == 0` 时用。
    pub box_min: [f32; 3],
    pub box_max: [f32; 3],
    /// 球壳形状的两个半径，`shape == 1` 时用。
    pub radii: (f32, f32),
    /// How long one emission cycle lasts.
    pub period: f32,
    pub looping: bool,
    /// `DirectionEmitter graph: x,y,z/t`.
    pub direction: [Curve; 3],
    /// `ElementRandomVelocity graph: m/s/t`, as (min, max) curves.
    pub speed: (Curve, Curve),
    /// `EmissionRate Graph: rate/t`, particles a second.
    pub rate: (Curve, Curve),
    /// `ElementLifeTime Graph: t/t`, seconds.
    pub life: (Curve, Curve),
    /// `SizeMultiplier Graph: */t` — scales the group's own size curve.
    pub size_mul: Curve,
    /// `RotationMultiplier Graph: */t`.
    pub rotation_mul: Curve,
}

/// One playable group: its particles' look and where they come from.
#[derive(Debug, Clone)]
pub struct ParGroupDef {
    pub param: ParGroupParam,
    pub emitter: ParEmitter,
}

/// Everything a `.par` says, decoded.
#[derive(Debug, Clone)]
pub struct ParEffect {
    pub groups: Vec<ParGroupDef>,
}

fn f32_at(record: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(record[offset..offset + 4].try_into().unwrap_or([0; 4]))
}

fn u32_at(record: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(record[offset..offset + 4].try_into().unwrap_or([0; 4]))
}

fn vec3_at(record: &[u8], offset: usize) -> [f32; 3] {
    [
        f32_at(record, offset),
        f32_at(record, offset + 4),
        f32_at(record, offset + 8),
    ]
}

impl ParEffect {
    /// Turn the raw records into the fields the runtime needs.
    pub fn decode(file: &ParFile) -> Self {
        let groups = file
            .groups
            .iter()
            .filter_map(|group| {
                let param = file.group_params.get(group.group_param as usize)?;
                let emitter = file.emitters.get(group.emitter as usize)?;
                Some(ParGroupDef {
                    param: ParGroupParam {
                        name: group_param_name(param),
                        texture: group_param_texture(param),
                        blend: group_param_blend(param),
                        frames: u32_at(param, GROUP_PARAM_FRAME_COUNT).max(1),
                        first_frame_range: u32_at(param, GROUP_PARAM_FIRST_FRAME_RANGE).max(1),
                        gravity: f32_at(param, GROUP_PARAM_GRAVITY),
                        drag_speed: f32_at(param, GROUP_PARAM_DRAG_SPEED),
                        drag_by_size: u32_at(param, GROUP_PARAM_DRAG_MODE) != 0,
                        drag: f32_at(param, GROUP_PARAM_DRAG),
                        size: track_curve(param, GROUP_PARAM_TRACKS[0], 0),
                        rotation: track_curve(param, GROUP_PARAM_TRACKS[1], 0),
                        alpha: track_curve(param, GROUP_PARAM_TRACKS[2], 0),
                        color: [
                            track_curve(param, GROUP_PARAM_TRACKS[3], 0),
                            track_curve(param, GROUP_PARAM_TRACKS[3], 1),
                            track_curve(param, GROUP_PARAM_TRACKS[3], 2),
                        ],
                    },
                    emitter: ParEmitter {
                        name: emitter_name(emitter),
                        mesh: emitter_mesh(emitter),
                        origin: vec3_at(emitter, EMITTER_ORIGIN),
                        spread: vec3_at(emitter, EMITTER_SPREAD),
                        shape: u32_at(emitter, EMITTER_SHAPE),
                        box_min: vec3_at(emitter, EMITTER_BOX_MIN),
                        box_max: vec3_at(emitter, EMITTER_BOX_MAX),
                        radii: (
                            f32_at(emitter, EMITTER_RADII),
                            f32_at(emitter, EMITTER_RADII + 4),
                        ),
                        period: (f32_at(emitter, EMITTER_PERIOD.0)
                            - f32_at(emitter, EMITTER_PERIOD.1))
                        .abs()
                        .max(f32::EPSILON),
                        looping: u32_at(emitter, EMITTER_LOOP) != 0,
                        direction: [
                            track_curve(emitter, EMITTER_TRACKS[0], 0),
                            track_curve(emitter, EMITTER_TRACKS[0], 1),
                            track_curve(emitter, EMITTER_TRACKS[0], 2),
                        ],
                        speed: (
                            track_curve(emitter, EMITTER_TRACKS[1], 0),
                            track_curve(emitter, EMITTER_TRACKS[1], 1),
                        ),
                        rate: (
                            track_curve(emitter, EMITTER_TRACKS[2], 0),
                            track_curve(emitter, EMITTER_TRACKS[2], 1),
                        ),
                        life: (
                            track_curve(emitter, EMITTER_TRACKS[3], 0),
                            track_curve(emitter, EMITTER_TRACKS[3], 1),
                        ),
                        size_mul: track_curve(emitter, EMITTER_TRACKS[4], 0),
                        rotation_mul: track_curve(emitter, EMITTER_TRACKS[5], 0),
                    },
                })
            })
            .collect();
        Self { groups }
    }
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
