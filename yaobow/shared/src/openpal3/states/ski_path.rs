//! 滑雪的雪道路径：`SubGame/Skee/SkeeScene/Scene/Skee_path.ASE`
//! （exe 里写死读 `basedata\Skee_path.ase`）。
//!
//! 原版的解析在 `FUN_0052d7c0`：扫 ASE 找 `*NODE_NAME "skee_path"` 那个
//! `GEOMOBJECT`，读它的 `*MESH_VERTEX` 列表，每个点摊成 **32 字节**一条：
//!
//! ```text
//! +0x00 x    +0x04 y    +0x08 z      // z 取反（0x52da8c 那一步）
//! +0x0c 从起点到这里的累计距离
//! +0x10 到上一个点的距离（SQRT(dx²+dy²+dz²)）
//! ```
//!
//! 实机那条路 4950 个点，从 x ≈ −10220 铺到 x ≈ +9020。逆向记录在
//! `switch/re/notes/ski.md`。

/// 路径上的一个点。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PathPoint {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// 从起点算到这里的累计距离（原版 `+0x0c`）。
    pub distance: f32,
    /// 到上一个点的距离（原版 `+0x10`），第一个点是 0。
    pub step: f32,
}

/// 整条雪道。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkiPath {
    pub points: Vec<PathPoint>,
}

impl SkiPath {
    /// 全长。
    pub fn length(&self) -> f32 {
        self.points.last().map(|p| p.distance).unwrap_or(0.)
    }

    /// 沿路走了 `distance` 之后在哪儿——两点之间线性插值。
    pub fn at(&self, distance: f32) -> Option<(f32, f32, f32)> {
        if self.points.is_empty() {
            return None;
        }
        let d = distance.clamp(0., self.length());
        // 点很多（实机 4950 个），二分找。
        let idx = match self.points.binary_search_by(|p| {
            p.distance
                .partial_cmp(&d)
                .unwrap_or(std::cmp::Ordering::Less)
        }) {
            Ok(i) => i,
            Err(i) => i,
        };
        if idx == 0 {
            let p = self.points[0];
            return Some((p.x, p.y, p.z));
        }
        let (a, b) = (
            self.points[idx - 1],
            self.points[idx.min(self.points.len() - 1)],
        );
        let span = b.distance - a.distance;
        let t = if span > f32::EPSILON {
            (d - a.distance) / span
        } else {
            0.
        };
        Some((
            a.x + (b.x - a.x) * t,
            a.y + (b.y - a.y) * t,
            a.z + (b.z - a.z) * t,
        ))
    }

    /// 解析 ASE。只取名为 `skee_path` 的那个物体的顶点列表；`z` 按原版取反，
    /// 距离和累计距离照 `FUN_0052d7c0` 现算。
    pub fn parse(text: &str) -> Self {
        const NODE: &str = "skee_path";
        let mut in_node = false;
        let mut points: Vec<PathPoint> = Vec::new();

        for raw in text.lines() {
            let line = raw.trim();
            if let Some(rest) = line.strip_prefix("*NODE_NAME") {
                in_node = rest.trim().trim_matches('"') == NODE;
                continue;
            }
            if !in_node {
                continue;
            }
            let Some(rest) = line.strip_prefix("*MESH_VERTEX") else {
                continue;
            };
            // `*MESH_VERTEX <下标> <x> <y> <z>`
            let mut it = rest.split_whitespace();
            let (Some(_index), Some(x), Some(y), Some(z)) =
                (it.next(), it.next(), it.next(), it.next())
            else {
                continue;
            };
            let (Ok(x), Ok(y), Ok(z)) = (x.parse::<f32>(), y.parse::<f32>(), z.parse::<f32>())
            else {
                continue;
            };
            let mut p = PathPoint {
                x,
                y,
                // 原版读完就把 z 取反。
                z: -z,
                distance: 0.,
                step: 0.,
            };
            if let Some(prev) = points.last() {
                let (dx, dy, dz) = (p.x - prev.x, p.y - prev.y, p.z - prev.z);
                p.step = (dx * dx + dy * dy + dz * dz).sqrt();
                p.distance = prev.distance + p.step;
            }
            points.push(p);
        }
        Self { points }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
*3DSMAX_ASCIIEXPORT 200
*GEOMOBJECT {
	*NODE_NAME "other_thing"
	*MESH {
		*MESH_VERTEX_LIST {
			*MESH_VERTEX    0	999.0	999.0	999.0
		}
	}
}
*GEOMOBJECT {
	*NODE_NAME "skee_path"
	*NODE_TM {
		*NODE_NAME "skee_path"
	}
	*MESH {
		*MESH_NUMVERTEX 3
		*MESH_VERTEX_LIST {
			*MESH_VERTEX    0	0.0	0.0	1.0
			*MESH_VERTEX    1	3.0	4.0	1.0
			*MESH_VERTEX    2	3.0	4.0	6.0
		}
	}
}
"#;

    #[test]
    fn only_the_skee_path_object_is_read() {
        let path = SkiPath::parse(SAMPLE);
        assert_eq!(path.points.len(), 3, "别的物体的顶点不能算进来");
        assert_eq!(path.points[0].x, 0.);
        assert_eq!(path.points[0].z, -1., "z 要取反");
    }

    #[test]
    fn the_distances_are_cumulative() {
        let path = SkiPath::parse(SAMPLE);
        assert_eq!(path.points[0].step, 0.);
        assert_eq!(path.points[1].step, 5., "3-4-5");
        assert_eq!(path.points[1].distance, 5.);
        assert_eq!(path.points[2].step, 5.);
        assert_eq!(path.points[2].distance, 10.);
        assert_eq!(path.length(), 10.);
    }

    #[test]
    fn walking_the_path_interpolates() {
        let path = SkiPath::parse(SAMPLE);
        assert_eq!(path.at(0.), Some((0., 0., -1.)));
        assert_eq!(path.at(5.), Some((3., 4., -1.)));
        // 走到一半的第二段。
        let (x, y, z) = path.at(7.5).unwrap();
        assert!((x - 3.).abs() < 1e-5 && (y - 4.).abs() < 1e-5);
        assert!((z - (-3.5)).abs() < 1e-5, "z = −1 到 −6 的中点");
        // 超出去的夹在两头。
        assert_eq!(path.at(-10.), path.at(0.));
        assert_eq!(path.at(999.), path.at(10.));
    }

    #[test]
    fn an_empty_file_is_an_empty_path() {
        let path = SkiPath::parse("*3DSMAX_ASCIIEXPORT 200\n");
        assert!(path.points.is_empty());
        assert_eq!(path.length(), 0.);
        assert_eq!(path.at(1.), None);
    }
}
