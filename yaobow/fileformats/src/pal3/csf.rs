//! `.csf` —— 粒子发射器的自定义形状点云。
//!
//! 形状 2 的发射器（实机 65 个里有 27 个）不按盒也不按球壳撒，而是在一份
//! 点云里随机挑一个点。那份点云就是 `.csf`，`gbEmitterShape::load_fromBinFile`
//! （`engine/physics/gbparteffect.cpp:79`）读的格式很短：
//!
//! ```text
//! char  header[4];              // "csf\0"
//! int   version;                // 实机 23 个文件全是 0x64
//! int   count;
//! float points[count][3];
//! ```
//!
//! 读完它还会算一张相邻两点的距离表（`disarray`），那是 `get_vec(f, frame, …)`
//! 沿折线取点用的；`ComputeParticleInitPos` 走的是另一个重载
//! `get_vec(gbVec3D&)`——`gbRand(0, count-1)` 随机挑一个点、过一次
//! `curmat`（`set_transform` 设的，粒子这条路上没人设过，所以是单位阵），
//! 所以这里只留点表。
//!
//! 实机的 23 个文件长度都正好是 `12 + count × 12`。

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::{Error, ErrorKind, Read, Result};

/// 一份 `.csf` 里的点。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CsfShape {
    pub version: u32,
    pub points: Vec<[f32; 3]>,
}

pub fn read_csf(reader: &mut dyn Read) -> Result<CsfShape> {
    let mut header = [0u8; 4];
    reader.read_exact(&mut header)?;
    if &header[0..3] != b"csf" {
        return Err(Error::new(ErrorKind::InvalidData, "不是 .csf"));
    }
    let version = reader.read_u32::<LittleEndian>()?;
    let count = reader.read_u32::<LittleEndian>()? as usize;
    let mut points = Vec::with_capacity(count.min(1 << 16));
    for _ in 0..count {
        points.push([
            reader.read_f32::<LittleEndian>()?,
            reader.read_f32::<LittleEndian>()?,
            reader.read_f32::<LittleEndian>()?,
        ]);
    }
    Ok(CsfShape { version, points })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_two_point_shape_round_trips() {
        let mut data = vec![];
        data.extend_from_slice(b"csf\0");
        data.extend_from_slice(&0x64u32.to_le_bytes());
        data.extend_from_slice(&2u32.to_le_bytes());
        for v in [1f32, 2., 3., -4., 5., 6.] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let shape = read_csf(&mut data.as_slice()).unwrap();
        assert_eq!(shape.version, 0x64);
        assert_eq!(shape.points, vec![[1., 2., 3.], [-4., 5., 6.]]);
    }

    #[test]
    fn a_wrong_magic_is_rejected() {
        let data = b"xxx\0\0\0\0\0\0\0\0\0".to_vec();
        assert!(read_csf(&mut data.as_slice()).is_err());
    }
}
