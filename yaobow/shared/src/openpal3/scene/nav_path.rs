//! 原版的 A* 找路（`FUN_00408460` → `FUN_00401070`）和路径几何（`FUN_00407dc0` 一组），野外怪物追人、
//! 回家用。逆向记在 `switch/re/notes/combat_flow.md`「野外遇敌」一节。
//!
//! 格子坐标是 `__ftol((x − 导航最小坐标) / 格宽)`（截断，不是向下取整）。格子记录 +4 那个字节同时是
//! 「能不能走」（> 1 才能走，`FUN_00403180`）和代价类别（`0x408410`）——它就是离边的格数。

use radiance::math::Vec3;

/// `0x56a5e0`：按格子类别取的代价系数。原版只填了前 8 项（1, 1, 1.5, 1.25, 1, 1, 1, 1），类别 ≥ 8
/// 时照样按下标读，读到的是后面的常数和函数指针。这里连越界的部分一起按原字节搬过来。
const TILE_COST_BITS: [u32; 256] = [
    0x3f800000, 0x3f800000, 0x3fc00000, 0x3fa00000, 0x3f800000, 0x3f800000, 0x3f800000, 0x3f800000,
    0x00401030, 0x004014e0, 0x004014f0, 0x41400000, 0x3f000000, 0x41400000, 0x00580ad0, 0x41400000,
    0x00402bd0, 0x3dcccccd, 0x42000000, 0x42800000, 0x41a00000, 0x00000000, 0x437f0000, 0x3f800000,
    0x3ecccccd, 0x00000000, 0x3fc00000, 0x42200000, 0x41400000, 0xff7fffff, 0x3e800000, 0x41400000,
    0x00583754, 0x40490fdb, 0x43340000, 0x00404d40, 0x005420a2, 0x0054209c, 0x00542096, 0x00542090,
    0x0054208a, 0x00542084, 0x0054207e, 0x00542078, 0x00542072, 0x00405310, 0x005420ba, 0x005420b4,
    0x005420ae, 0x005420a8, 0x00405580, 0x005420d2, 0x005420cc, 0x00405560, 0x005420c6, 0x005420c0,
    0x42700000, 0x44ac8000, 0x40800000, 0x41400000, 0x004083b0, 0x004083e0, 0x00408410, 0x41400000,
    0x42700000, 0x3f99999a, 0x44160000, 0x00000000, 0x000000cf, 0x00000193, 0x00000258, 0x000001eb,
    0x000001eb, 0x000001eb, 0x000001eb, 0x00000000, 0x00000000, 0x3ff00000, 0x00000000, 0x40140000,
    0x00000000, 0x401c0000, 0x00000000, 0x40180000, 0x00000000, 0x40200000, 0x00000000, 0x40220000,
    0x420c0000, 0x40a00000, 0x41b00000, 0x41c00000, 0x41d00000, 0x40900000, 0x41900000, 0x41800000,
    0x40600000, 0x41500000, 0x40400000, 0x41880000, 0x40533333, 0x42c80000, 0x3d4ccccd, 0x42a00000,
    0xc0400000, 0x00410900, 0x00542108, 0x00542102, 0x005420fc, 0x005420f6, 0x005420f0, 0x005420ea,
    0x005420e4, 0x005420de, 0x005420d8, 0x00588990, 0x0058898c, 0x00588988, 0x00588984, 0x00588980,
    0x0058897c, 0x00588974, 0x00588970, 0x0058896c, 0x00588968, 0x00588964, 0x00588960, 0x0058895c,
    0x00588958, 0x00588954, 0x00588950, 0x0058894c, 0x00588948, 0x00588944, 0x00588940, 0x0058893c,
    0x00588938, 0x00588934, 0x00588930, 0x0058892c, 0x00588928, 0x00588924, 0x00588920, 0x0058891c,
    0x00588918, 0x00588914, 0x00588910, 0x0058890c, 0x00588908, 0x00588900, 0x005888fc, 0x005888f8,
    0x005888f4, 0x005888f0, 0x005888ec, 0x005888e8, 0x005888e4, 0x005888dc, 0x005888d4, 0x005888d0,
    0x005888cc, 0x005888c8, 0x005888c4, 0x005888c0, 0x005888bc, 0x005888b8, 0x005888b4, 0x005888b0,
    0x005888ac, 0x005888a8, 0x005888a4, 0x005888a0, 0x0058889c, 0x005888b0, 0x00000003, 0x00000003,
    0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003,
    0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003,
    0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003,
    0x00000003, 0x00000003, 0x00000003, 0x00000002, 0x00000002, 0x00000002, 0x00000002, 0x00000002,
    0x00000002, 0x00000002, 0x00000002, 0x00000002, 0x00000003, 0x00000002, 0x00000002, 0x00000002,
    0x00000002, 0x00000002, 0x00000002, 0x00000002, 0x00000001, 0x00000002, 0x00000003, 0x00000002,
    0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003, 0x00000003,
    0x00000000, 0x41400000, 0x3f000000, 0x3f000000, 0x43f50000, 0x41200000, 0x42480000, 0x43fa0000,
    0x3f5eb852, 0x0041b640, 0x0041b6a0, 0x0041be30, 0x0041be20, 0x00416500, 0x004af760, 0x00411bd0,
    0x00411bb0, 0x0055434f, 0x004af760, 0x00411b80, 0x42340000, 0x3ba3d70a, 0x42652ee1, 0x40c90fdb,
];

pub fn tile_cost(tile_type: u8) -> f32 {
    f32::from_bits(TILE_COST_BITS[tile_type as usize])
}

/// 一层导航网格，按格子问。
pub trait NavGrid {
    /// `FUN_00421020`：这一格走得通（越界算不通）。
    fn walkable(&self, x: i32, z: i32) -> bool;
    /// 格子记录 +4 那个字节（寻路对象虚表 +8，0x408410）。越界是 0。
    fn tile_type(&self, x: i32, z: i32) -> u8;
}

impl<T: NavGrid + ?Sized> NavGrid for &T {
    fn walkable(&self, x: i32, z: i32) -> bool {
        (**self).walkable(x, z)
    }
    fn tile_type(&self, x: i32, z: i32) -> u8 {
        (**self).tile_type(x, z)
    }
}

impl<T: FieldNav + ?Sized> FieldNav for &T {
    fn tile_of(&self, p: &Vec3) -> (i32, i32) {
        (**self).tile_of(p)
    }
    fn tile_center(&self, x: i32, z: i32) -> Vec3 {
        (**self).tile_center(x, z)
    }
}

/// 带坐标换算的导航网格。
pub trait FieldNav: NavGrid {
    /// 场景坐标落在哪一格（`__ftol`，截断）。
    fn tile_of(&self, p: &Vec3) -> (i32, i32);
    /// 格子中心，y 是 0（`FUN_00408460` 往路径里写点时 y 就写 0）。
    fn tile_center(&self, x: i32, z: i32) -> Vec3;

    fn walkable_at(&self, p: &Vec3) -> bool {
        let (x, z) = self.tile_of(p);
        self.walkable(x, z)
    }
}

/// `FUN_004215b0`：两格之间看得见——两头都得走得通，再从起点按 Bresenham 一格一格走主轴那么多步，
/// 每一格都走得通（终点那一格前面单独查过，循环里不再查）。
pub fn line_clear<N: NavGrid + ?Sized>(grid: &N, from: (i32, i32), to: (i32, i32)) -> bool {
    if !grid.walkable(from.0, from.1) || !grid.walkable(to.0, to.1) {
        return false;
    }
    let (mut x, mut z) = from;
    let (dx, sx) = if to.0 < x {
        (x - to.0, -1)
    } else {
        (to.0 - x, 1)
    };
    let (dz, sz) = if to.1 < z {
        (z - to.1, -1)
    } else {
        (to.1 - z, 1)
    };
    let x_major = dz <= dx;
    let (major, minor) = if x_major { (dx, dz) } else { (dz, dx) };
    let mut err = -major;
    for _ in 0..major {
        err += minor * 2;
        if !grid.walkable(x, z) {
            return false;
        }
        if err >= 0 {
            if x_major {
                z += sz;
            } else {
                x += sx;
            }
            err -= major * 2;
        }
        if x_major {
            x += sx;
        } else {
            z += sz;
        }
    }
    true
}

/// 展开次数上限：计数从 1 起，大于 300 就停（`FUN_00401070` 里 `cmp [esi+0x14], 0x12c`）。
const MAX_EXPANSIONS: i32 = 300;
/// 路径最多 256 个点（`FUN_00408460`：`+0xc00 < 0x100`）。
const MAX_POINTS: usize = 256;

struct Node {
    f: f32,
    h: f32,
    g: f32,
    x: i32,
    z: i32,
    parent: Option<usize>,
    /// 子节点（原版 +0x1c 起 8 个槽）：改小了父节点的 g 以后要顺着往下传。
    children: Vec<usize>,
    closed: bool,
}

/// `FUN_00401070`：A*。开放表是按 f 从小到大排的链表，新点插在第一个 f 不比它小的点前面（f 相同时新点
/// 在前）；开放表里的点 g 改小了不重排；关闭表里的点 g 改小了顺着子节点往下传（`FUN_004015b0`）。
/// 每次取开放表头，是终点就结束；不是就记下 h 最小的那个点（严格小于才换），按固定顺序展开 8 个邻格。
/// 展开超过 300 次或开放表空了还没到终点，就走到 h 最小的那个点。返回从起点到终点（或最近点）的格子。
pub fn a_star<N: NavGrid + ?Sized>(
    grid: &N,
    start: (i32, i32),
    goal: (i32, i32),
) -> Option<Vec<(i32, i32)>> {
    let id = |x: i32, z: i32| z * 512 + x;
    let goal_id = id(goal.0, goal.1);
    // h 按终点那一格的代价系数算（`FUN_004012a0` 里第二次调 +8 传的是终点）。
    let goal_cost = tile_cost(grid.tile_type(goal.0, goal.1)) as f64;
    let dist = |dx: i32, dz: i32| ((dx * dx + dz * dz) as f64).sqrt();
    let h_of = |x: i32, z: i32| (dist(x - goal.0, z - goal.1) * goal_cost) as f32;

    let mut nodes = vec![Node {
        f: h_of(start.0, start.1),
        h: h_of(start.0, start.1),
        g: 0.,
        x: start.0,
        z: start.1,
        parent: None,
        children: vec![],
        closed: false,
    }];
    let mut index = std::collections::HashMap::from([(id(start.0, start.1), 0usize)]);
    let mut open: Vec<usize> = vec![0];

    // `FUN_00401560`：插在第一个 f 不比它小的点前面。
    fn insert(open: &mut Vec<usize>, nodes: &[Node], n: usize) {
        let at = open
            .iter()
            .position(|&o| !(nodes[o].f < nodes[n].f))
            .unwrap_or(open.len());
        open.insert(at, n);
    }

    // 一步的代价：格间距离 × 目标格的代价系数 + 父节点的 g（x87 里算完再存成 float）。
    let step_g = |nodes: &[Node], from: usize, x: i32, z: i32| {
        (dist(nodes[from].x - x, nodes[from].z - z) * tile_cost(grid.tile_type(x, z)) as f64
            + nodes[from].g as f64) as f32
    };

    let mut best: Option<usize> = None;
    let mut expansions = 1;
    while !open.is_empty() {
        // `FUN_004014b0`：取开放表头，挪进关闭表。
        let n = open.remove(0);
        nodes[n].closed = true;
        if expansions > MAX_EXPANSIONS {
            break;
        }
        if id(nodes[n].x, nodes[n].z) == goal_id {
            best = Some(n);
            break;
        }
        if best.is_none_or(|b| nodes[n].h < nodes[b].h) {
            best = Some(n);
        }
        let (x, z) = (nodes[n].x, nodes[n].z);
        for (nx, nz) in [
            (x, z - 1),
            (x, z + 1),
            (x - 1, z),
            (x + 1, z),
            (x - 1, z - 1),
            (x + 1, z + 1),
            (x + 1, z - 1),
            (x - 1, z + 1),
        ] {
            // `FUN_004012a0`。
            if !grid.walkable(nx, nz) {
                continue;
            }
            let g = step_g(&nodes, n, nx, nz);
            if let Some(&o) = index.get(&id(nx, nz)) {
                nodes[n].children.push(o);
                if g < nodes[o].g {
                    nodes[o].parent = Some(n);
                    nodes[o].g = g;
                    nodes[o].f = g + nodes[o].h;
                    if nodes[o].closed {
                        // `AStarPathFinder::PropagateDown`（`gamebase/astarpathfinder.cpp:316`，
                        // exe 里 `FUN_004015b0`）：顺着子节点往下传。确实是后进先出——
                        // `Push` 插在链表头、`Pop` 从链表头取（同文件 `:363`、`:373`）。
                        // 原版先对 `Old` 的子节点走一遍再抽栈，这里把 `o` 先压进去，
                        // 第一轮循环做的是同一件事。
                        let mut stack = vec![o];
                        while let Some(p) = stack.pop() {
                            for c in nodes[p].children.clone() {
                                let g = step_g(&nodes, p, nodes[c].x, nodes[c].z);
                                if g < nodes[c].g {
                                    nodes[c].g = g;
                                    nodes[c].parent = Some(p);
                                    nodes[c].f = g + nodes[c].h;
                                    stack.push(c);
                                }
                            }
                        }
                    }
                }
            } else {
                let h = h_of(nx, nz);
                nodes.push(Node {
                    f: h + g,
                    h,
                    g,
                    x: nx,
                    z: nz,
                    parent: Some(n),
                    children: vec![],
                    closed: false,
                });
                let new = nodes.len() - 1;
                index.insert(id(nx, nz), new);
                insert(&mut open, &nodes, new);
                nodes[n].children.push(new);
            }
        }
        expansions += 1;
    }

    let mut tiles = vec![];
    let mut at = best;
    while let Some(n) = at {
        tiles.push((nodes[n].x, nodes[n].z));
        at = nodes[n].parent;
    }
    tiles.reverse();
    (!tiles.is_empty()).then_some(tiles)
}

/// `FUN_00408460`：从 `from` 找路到 `to`。起点那一格走不通就失败；终点那一格走不通时原版调
/// `FUN_00421710` 从终点往起点找——可它第一步就查终点那一格、走不通立刻返回，终点原样不变，等于没做，
/// A* 于是走到离终点最近的那个点。路径是各格中心（y = 0），最多 256 个点，不到 2 个点算失败。
pub fn find_path<N: FieldNav + ?Sized>(nav: &N, from: &Vec3, to: &Vec3) -> Option<Path> {
    let start = nav.tile_of(from);
    if !nav.walkable(start.0, start.1) {
        return None;
    }
    let tiles = a_star(nav, start, nav.tile_of(to))?;
    let points: Vec<Vec3> = tiles
        .iter()
        .take(MAX_POINTS)
        .map(|&(x, z)| nav.tile_center(x, z))
        .collect();
    (points.len() >= 2).then(|| Path::new(points))
}

/// 一条路径（角色 +0x5c8 起那一块）：点、各段长度和方向、总长。
#[derive(Debug, Clone)]
pub struct Path {
    pub points: Vec<Vec3>,
    /// 第 i 段（点 i−1 → 点 i）的长度，i ≥ 1；第 0 项抄第 1 项。
    lens: Vec<f32>,
    dirs: Vec<Vec3>,
    pub total: f32,
}

fn dot(a: &Vec3, b: &Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

impl Path {
    /// `FUN_00407dc0`：算各段长度、方向和总长。
    pub fn new(points: Vec<Vec3>) -> Self {
        let n = points.len();
        let mut lens = vec![0.; n];
        let mut dirs = vec![Vec3::new(0., 0., 0.); n];
        let mut total = 0.;
        for i in 1..n {
            let d = Vec3::sub(&points[i], &points[i - 1]);
            let l = d.norm();
            lens[i] = l;
            dirs[i] = Vec3::scalar_mul(1. / l, &d);
            total += l;
        }
        if n > 1 {
            lens[0] = lens[1];
            dirs[0] = dirs[1];
        }
        Self {
            points,
            lens,
            dirs,
            total,
        }
    }

    /// `FUN_00408190`：点到第 i 段的距离（三维，带 y）、最近点、沿段走了多远。
    fn to_segment(&self, p: &Vec3, i: usize) -> (f32, Vec3, f32) {
        let a = &self.points[i - 1];
        let t = dot(&Vec3::sub(p, a), &self.dirs[i]);
        if t < 0. {
            return (Vec3::sub(p, a).norm(), *a, 0.);
        }
        if self.lens[i] < t {
            let b = &self.points[i];
            return (Vec3::sub(p, b).norm(), *b, self.lens[i]);
        }
        let c = Vec3::add(a, &Vec3::scalar_mul(t, &self.dirs[i]));
        (Vec3::sub(p, &c).norm(), c, t)
    }

    /// `FUN_00407ec0`：离 `p` 最近的那一段（距离严格更小才换）。离它不到 6 算在路上，连那一段的方向一起返回。
    pub fn near(&self, p: &Vec3) -> (bool, Vec3) {
        let mut best = f32::MAX;
        let mut closest = Vec3::new(0., 0., 0.);
        let mut dir = Vec3::new(0., 0., 0.);
        for i in 1..self.points.len() {
            let (d, c, _) = self.to_segment(p, i);
            if d < best {
                best = d;
                closest = c;
                dir = self.dirs[i];
            }
        }
        (Vec3::sub(p, &closest).norm() < 6., dir)
    }

    /// `FUN_00408010`：`p` 投到路径上，从起点量过去有多远。
    pub fn project(&self, p: &Vec3) -> f32 {
        let (mut best, mut at, mut walked) = (f32::MAX, 0., 0.);
        for i in 1..self.points.len() {
            let (d, _, t) = self.to_segment(p, i);
            if d < best {
                best = d;
                at = walked + t;
            }
            walked += self.lens[i];
        }
        at
    }

    /// `FUN_004080f0`：从起点沿路径走 `d` 那一点；超过总长就没有。
    pub fn point_at(&self, mut d: f32) -> Option<Vec3> {
        for i in 1..self.points.len() {
            let l = self.lens[i];
            if d <= l {
                let (a, b) = (&self.points[i - 1], &self.points[i]);
                return Some(Vec3::add(a, &Vec3::scalar_mul(d / l, &Vec3::sub(b, a))));
            }
            d -= l;
        }
        None
    }
}

/// 测试用的网格：`w × h` 格、格宽 `cell`，最小坐标在原点，`walls` 里的格子走不通，其它格的类别都是 `kind`。
#[cfg(test)]
pub(crate) struct TestGrid {
    pub w: i32,
    pub h: i32,
    pub cell: f32,
    pub kind: u8,
    pub walls: std::collections::HashSet<(i32, i32)>,
}

#[cfg(test)]
impl TestGrid {
    pub fn open(w: i32, h: i32, cell: f32) -> Self {
        Self {
            w,
            h,
            cell,
            kind: 8,
            walls: Default::default(),
        }
    }
}

#[cfg(test)]
impl NavGrid for TestGrid {
    fn walkable(&self, x: i32, z: i32) -> bool {
        self.tile_type(x, z) > 1
    }
    fn tile_type(&self, x: i32, z: i32) -> u8 {
        if x < 0 || z < 0 || x >= self.w || z >= self.h || self.walls.contains(&(x, z)) {
            0
        } else {
            self.kind
        }
    }
}

#[cfg(test)]
impl FieldNav for TestGrid {
    fn tile_of(&self, p: &Vec3) -> (i32, i32) {
        ((p.x / self.cell) as i32, (p.z / self.cell) as i32)
    }
    fn tile_center(&self, x: i32, z: i32) -> Vec3 {
        Vec3::new(
            (x as f32 + 0.5) * self.cell,
            0.,
            (z as f32 + 0.5) * self.cell,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cost_table_reads_past_its_eight_entries_like_the_original() {
        assert_eq!(
            (0..8).map(tile_cost).collect::<Vec<_>>(),
            [1., 1., 1.5, 1.25, 1., 1., 1., 1.]
        );
        assert_eq!(tile_cost(11), 12.);
        assert_eq!(tile_cost(12), 0.5);
        assert!(
            tile_cost(8) < 1e-30,
            "下标 8 是个函数指针，当 float 读是个极小的数"
        );
    }

    #[test]
    fn a_star_goes_around_a_wall() {
        let mut g = TestGrid::open(20, 20, 10.);
        for z in 0..15 {
            g.walls.insert((10, z));
        }
        let path = a_star(&g, (2, 2), (18, 2)).unwrap();
        assert_eq!(path.first(), Some(&(2, 2)));
        assert_eq!(path.last(), Some(&(18, 2)));
        assert!(path.iter().all(|t| !g.walls.contains(t)));
        assert!(
            path.iter().any(|&(x, z)| x == 10 && z >= 15),
            "从墙头绕过去"
        );
        assert!(
            path.windows(2)
                .all(|w| (w[0].0 - w[1].0).abs() <= 1 && (w[0].1 - w[1].1).abs() <= 1)
        );
    }

    #[test]
    fn a_star_stops_at_the_closest_tile_when_the_goal_is_walled_off() {
        let mut g = TestGrid::open(20, 20, 10.);
        for (x, z) in [
            (14, 9),
            (14, 10),
            (14, 11),
            (15, 9),
            (15, 11),
            (16, 9),
            (16, 10),
            (16, 11),
        ] {
            g.walls.insert((x, z));
        }
        let path = a_star(&g, (2, 10), (15, 10)).unwrap();
        let end = *path.last().unwrap();
        assert_ne!(end, (15, 10));
        let d = |(x, z): (i32, i32)| (x - 15) * (x - 15) + (z - 10) * (z - 10);
        assert_eq!(d(end), 4, "终点四周一圈都是墙，停在离它两格的地方：{end:?}");
    }

    #[test]
    fn line_of_sight_is_blocked_by_an_unwalkable_tile() {
        let mut g = TestGrid::open(20, 20, 10.);
        assert!(line_clear(&g, (1, 1), (12, 5)));
        g.walls.insert((6, 3));
        assert!(!line_clear(&g, (1, 1), (12, 5)));
        assert!(!line_clear(&g, (6, 3), (1, 1)), "起点走不通");
    }

    #[test]
    fn path_geometry_projects_and_walks_along_segments() {
        let p = Path::new(vec![
            Vec3::new(0., 0., 0.),
            Vec3::new(30., 0., 0.),
            Vec3::new(30., 0., 40.),
        ]);
        assert_eq!(p.total, 70.);
        assert_eq!(p.project(&Vec3::new(10., 0., 5.)), 10.);
        assert_eq!(p.project(&Vec3::new(33., 0., 20.)), 50.);
        let q = p.point_at(50.).unwrap();
        assert_eq!((q.x, q.z), (30., 20.));
        assert!(p.point_at(71.).is_none());
        let (on, dir) = p.near(&Vec3::new(31., 0., 10.));
        assert!(on);
        assert_eq!((dir.x, dir.z), (0., 1.));
        assert!(!p.near(&Vec3::new(10., 7., 0.)).0, "带 y 算距离");
    }

    #[test]
    fn find_path_emits_tile_centres_at_ground_level() {
        let g = TestGrid::open(20, 20, 10.);
        let p = find_path(&g, &Vec3::new(21., 238., 21.), &Vec3::new(61., 238., 21.)).unwrap();
        assert_eq!(p.points.len(), 5);
        assert_eq!((p.points[0].x, p.points[0].y), (25., 0.));
        assert_eq!(p.points[4].x, 65.);
        assert!(
            find_path(&g, &Vec3::new(21., 0., 21.), &Vec3::new(22., 0., 22.)).is_none(),
            "同一格：只有一个点"
        );
    }
}
