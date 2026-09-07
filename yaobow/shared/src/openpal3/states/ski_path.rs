//! 滑雪的雪道路径：`SubGame/Skee/SkeeScene/Scene/Skee_path.ASE`
//! （exe 里写死读 `basedata\Skee_path.ase`）。
//!
//! 原版的解析在 `FUN_0052d7c0`：扫 ASE 找 `*NODE_NAME "skee_path"` 那个
//! `GEOMOBJECT`，读它的 `*MESH_VERTEX` 列表，每个点摊成 **32 字节**一条：
//!
//! ```text
//! +0x00 x    +0x04 y    +0x08 z
//! +0x0c 从起点到这里的累计距离
//! +0x10 到上一个点的距离（SQRT(dx²+dy²+dz²)）
//! +0x14/18/1c 指向下一个点的单位方向（存在**上一个**点上）
//! ```
//!
//! 读的时候三个数不是按 ASE 的顺序进去的——那句 `scanf` 是
//! `"%f %f %f"` 配 `(基址, 基址+8, 基址+4)`：ASE 的第二个数进 `+0x08`、
//! 第三个数进 `+0x04`，然后 `+0x08` 再取反。换句话说
//! **`y = ASE 的 z`、`z = −ASE 的 y`**。写入的位置按文件里那个顶点下标走，
//! 不是按出现顺序。
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
    /// 指向下一个点的单位方向（原版 `+0x14/18/1c`，存在这个点上）。
    /// 最后一个点没有下一段，留 0。
    pub forward: (f32, f32, f32),
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

    /// 解析 ASE。只取名为 `skee_path` 的那个物体的顶点列表；坐标按原版的
    /// `y = ASE.z`、`z = −ASE.y` 摆，距离、累计距离和逐段方向照
    /// `FUN_0052d7c0` 现算。
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
            // `*MESH_VERTEX <下标> <x> <y> <z>`。原版是 `%s` 读一个词再
            // `strcmp`，所以 `*MESH_VERTEX_LIST` 这行不算。
            let mut it = line.split_whitespace();
            if it.next() != Some("*MESH_VERTEX") {
                continue;
            }
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
                // ASE 的第三个数进 `+0x04`，第二个数进 `+0x08` 再取反。
                y: z,
                z: -y,
                distance: 0.,
                step: 0.,
                forward: (0., 0., 0.),
            };
            if let Some(prev) = points.last().copied() {
                let (dx, dy, dz) = (p.x - prev.x, p.y - prev.y, p.z - prev.z);
                p.step = (dx * dx + dy * dy + dz * dz).sqrt();
                p.distance = prev.distance + p.step;
                // 方向存在上一个点上。
                if p.step > f32::EPSILON {
                    let last = points.last_mut().unwrap();
                    last.forward = (dx / p.step, dy / p.step, dz / p.step);
                }
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
        // `*MESH_VERTEX 0  0.0 0.0 1.0` → y 拿 ASE 的第三个数，z 拿第二个取反。
        assert_eq!(path.points[0].y, 1., "y = ASE 的 z");
        assert_eq!(path.points[1].z, -4., "z = −ASE 的 y");
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
        assert_eq!(path.at(0.), Some((0., 1., -0.)));
        assert_eq!(path.at(5.), Some((3., 1., -4.)));
        // 走到一半的第二段。
        let (x, y, z) = path.at(7.5).unwrap();
        assert!((x - 3.).abs() < 1e-5 && (z - (-4.)).abs() < 1e-5);
        assert!((y - 3.5).abs() < 1e-5, "y = 1 到 6 的中点");
        // 超出去的夹在两头。
        assert_eq!(path.at(-10.), path.at(0.));
        assert_eq!(path.at(999.), path.at(10.));
    }

    #[test]
    fn each_point_carries_the_direction_to_the_next() {
        let path = SkiPath::parse(SAMPLE);
        // 第一段是 (3, 0, −4)，长 5。
        let f = path.points[0].forward;
        assert!((f.0 - 0.6).abs() < 1e-5 && f.1.abs() < 1e-5 && (f.2 - -0.8).abs() < 1e-5);
        // 第二段整段沿 +y。
        assert_eq!(path.points[1].forward, (0., 1., 0.));
        // 最后一个点没有下一段。
        assert_eq!(path.points[2].forward, (0., 0., 0.));
    }

    #[test]
    fn an_empty_file_is_an_empty_path() {
        let path = SkiPath::parse("*3DSMAX_ASCIIEXPORT 200\n");
        assert!(path.points.is_empty());
        assert_eq!(path.length(), 0.);
        assert_eq!(path.at(1.), None);
    }
}
