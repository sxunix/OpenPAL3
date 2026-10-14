//! 场景物件挡路：原版判一格能不能走（`FUN_00421020`）时，除了格子自己（记录 +4 字节 > 1），还看格子上登记的
//! 物件。逆向记在 `switch/re/notes/combat_flow.md`「场景物件阻挡」一节。
//!
//! 每层一张「格子 → 物件号」字节表（0xff = 没有），物件号就是 `.scn` 里物件的顺序。载入场景时每层清表，
//! 再按顺序对这一层的物件做 `FUN_0042ad40`：按类型改「可通过」（`IsNonBlocking`），多数类型调
//! `FUN_0042a010` 按模型包围盒登记压住的格子；返回 0 且 `TriggerType` 位 0 为 1 的再把触发矩形整块登记。
//! 后登记的覆盖先登记的。

use radiance::math::{Mat44, Vec3};

use super::scene::NAV_BLOCK_SIZE as TILE;

const NONE: u8 = 0xff;
/// `SetAxisAngle` 里角度换弧度的常数（gbengine 0x100f5648）。
const DEG: f32 = 0.017_453_292;

/// 一个物件登记要用到的 `.scn` 字段。
#[derive(Debug, Clone)]
pub struct BlockNode {
    /// 脚本用的编号（`.scn` 编号的低字节）。
    pub object_id: i32,
    pub name: String,
    /// 物件类型（文件 +0x48）。
    pub kind: u8,
    /// `TriggerType`（+0x24）。
    pub trigger_type: u8,
    /// `IsNonBlocking`（+0x25），非 0 可通过。
    pub non_blocking: u8,
    /// `InitActive`（+0x01）。
    pub init_active: bool,
    /// `SwitchState`（+0x03）。
    pub switch_state: u8,
    pub layer: usize,
    /// `Parameters[0]`（+0x4c）/ `Parameters[3]`（+0x58）。
    pub p0: i32,
    pub p3: i32,
    /// 触发矩形（+0x38，格子坐标 x0 z0 x1 z1）。
    pub trigger_rect: [i32; 4],
    pub position: Vec3,
    /// 绕 Y（+0x34）、绕 X（+0x184）转的角度。
    pub y_deg: f32,
    pub x_deg: f32,
}

/// 物件模型的包围盒：盒上一点 = `m · d`，`d` 在 ±`ext`/2 里（原版 `gbOBBox`，`m` 这里写成列向量形式）。
#[derive(Debug, Clone, Copy)]
pub struct Obb {
    pub m: Mat44,
    pub ext: [f32; 3],
}

#[derive(Debug, Clone, Copy)]
pub enum Model {
    /// 没有模型（名字空、或文件没找到）。
    None,
    /// `+` 开头的特效物件（模型类型 4）。[未做] 原版包围盒来自特效对象的虚函数。
    Fx,
    Box(Obb),
}

/// 一层导航网格的尺寸和最小坐标。
#[derive(Debug, Clone, Copy)]
pub struct LayerGrid {
    pub w: i32,
    pub h: i32,
    pub min_x: f32,
    pub min_z: f32,
}

impl LayerGrid {
    /// 格子坐标（`__ftol`，截断）。
    fn tile(&self, p: &Vec3) -> (i32, i32) {
        (
            ((p.x - self.min_x) / TILE) as i32,
            ((p.z - self.min_z) / TILE) as i32,
        )
    }
}

#[derive(Debug, Clone)]
struct Obj {
    object_id: i32,
    kind: u8,
    non_blocking: bool,
    p0: i32,
    active: bool,
}

#[derive(Debug, Clone)]
pub struct ObjectBlocking {
    grids: Vec<LayerGrid>,
    maps: Vec<Vec<u8>>,
    objs: Vec<Obj>,
    /// 重新登记（物件动作 9）要用：各物件的 `.scn` 字段（位置、层随物件动作改）、模型包围盒、场景名。
    nodes: Vec<BlockNode>,
    models: Vec<Model>,
    scene: (String, String),
}

impl ObjectBlocking {
    /// 载入场景时建表（`FUN_0041fa90` → 每层 `FUN_00420940`）。`scene` 是（场景包名, 子场景名），
    /// 两处按场景特判要用。
    pub fn build(
        nodes: &[BlockNode],
        models: &[Model],
        grids: Vec<LayerGrid>,
        scene: (&str, &str),
    ) -> Self {
        let maps = grids
            .iter()
            .map(|g| vec![NONE; (g.w.max(0) * g.h.max(0)) as usize])
            .collect();
        let mut b = Self {
            grids,
            maps,
            objs: nodes
                .iter()
                .map(|n| Obj {
                    object_id: n.object_id,
                    kind: n.kind,
                    non_blocking: n.non_blocking != 0,
                    p0: n.p0,
                    active: n.init_active,
                })
                .collect(),
            nodes: nodes.to_vec(),
            models: models.to_vec(),
            scene: (scene.0.to_string(), scene.1.to_string()),
        };
        for layer in 0..b.grids.len() {
            for (i, n) in nodes.iter().enumerate() {
                if n.layer != layer || i >= NONE as usize {
                    continue;
                }
                let registered = b.init_object(i, n, models[i], scene);
                if registered && n.trigger_type & 1 != 0 {
                    b.fill_rect(layer, n.trigger_rect, i as u8);
                }
            }
        }
        b
    }

    /// 脚本改了物件在不在场（`+0xad`）。
    pub fn set_active(&mut self, object_id: i32, active: bool) {
        for o in self.objs.iter_mut().filter(|o| o.object_id == object_id) {
            o.active = active;
        }
    }

    /// 物件动作 10（`FUN_0042ebe0` case 10）改「可通过」+0xd1。`i` 是物件在 `.scn` 里的序号。
    pub fn set_passable(&mut self, i: usize, passable: bool) {
        if let Some(o) = self.objs.get_mut(i) {
            o.non_blocking = passable;
        }
    }

    /// `FUN_0042a010(1)`：清掉物件 `i` 压住的格子（各层）。
    pub fn clear(&mut self, i: usize) {
        for map in self.maps.iter_mut() {
            for v in map.iter_mut().filter(|v| **v as usize == i) {
                *v = NONE;
            }
        }
    }

    /// `FUN_0042a010(0)`：按物件现在的位置、层重新按模型包围盒登记（物件动作 9，上下传送板换层后用）。
    pub fn register_at(&mut self, i: usize, layer: usize, position: Vec3) {
        if i >= self.nodes.len() || layer >= self.grids.len() || i >= NONE as usize {
            return;
        }
        self.nodes[i].layer = layer;
        self.nodes[i].position = position;
        let n = self.nodes[i].clone();
        let model = self.models[i];
        let scene = self.scene.clone();
        self.register(i, &n, model, (&scene.0, &scene.1));
    }

    /// 物件 `i` 现在绕 Y 的角度（旋转石梁转过以后跟着改，重新登记格子时按它算）。
    pub fn yaw(&self, i: usize) -> Option<f32> {
        self.nodes.get(i).map(|n| n.y_deg)
    }

    pub fn set_yaw(&mut self, i: usize, deg: f32) {
        if let Some(n) = self.nodes.get_mut(i) {
            n.y_deg = deg;
        }
    }

    /// `FUN_0042c280`（神树花换模型）：原来有模型就先清掉压住的格子，换成 `model`、按它重新登记，
    /// 「可通过」= `open`。
    pub fn set_flower(&mut self, i: usize, model: Model, open: bool) {
        if i >= self.nodes.len() || i >= NONE as usize {
            return;
        }
        if !matches!(self.models[i], Model::None) {
            self.clear(i);
        }
        self.models[i] = model;
        let n = self.nodes[i].clone();
        let scene = self.scene.clone();
        self.register(i, &n, model, (&scene.0, &scene.1));
        self.objs[i].non_blocking = open;
    }

    /// `FUN_00429db0` 给的包围盒 y 边长（模型类型 2/3 的 +0xa4）；没有模型是 0。
    pub fn model_height(&self, i: usize) -> f32 {
        match self.models.get(i) {
            Some(Model::Box(obb)) => obb.ext[1],
            _ => 0.,
        }
    }

    /// 物件模型包围盒的 x、z 边长（`GetRawOBB` 的 `xlen`、`zlen`）；没有模型是 `None`。
    pub fn model_extent(&self, i: usize) -> Option<(f32, f32)> {
        match self.models.get(i) {
            Some(Model::Box(obb)) => Some((obb.ext[0], obb.ext[2])),
            _ => None,
        }
    }

    /// `Scene::Obj_CanRoleGoTile`（`gamebase/scene.cpp:902`）：物件 `me` 看这一格能不能走——格子上登记的
    /// 是它自己就只看格子；别的在场物件按类型：升降台 0x16、移动浮板 5 是「格子能走 或 可通过」，其余
    /// 「格子能走 且 可通过」。越界不能走。
    pub fn walkable_for(&self, me: usize, layer: usize, x: i32, z: i32, base: bool) -> bool {
        let Some(g) = self.grids.get(layer) else {
            return base;
        };
        if x < 0 || z < 0 || x >= g.w || z >= g.h {
            return false;
        }
        let id = self.maps[layer][(z * g.w + x) as usize] as usize;
        if id == me {
            return base;
        }
        let Some(o) = self.objs.get(id) else {
            return base;
        };
        if !o.active {
            return base;
        }
        match o.kind {
            0x16 | 5 => base || o.non_blocking,
            _ => base && o.non_blocking,
        }
    }

    /// 物件 `i` 在不在场（+0xad）。
    pub fn active(&self, i: usize) -> bool {
        self.objs.get(i).is_some_and(|o| o.active)
    }

    /// 物件 `i` 的类型。
    pub fn kind(&self, i: usize) -> Option<u8> {
        self.objs.get(i).map(|o| o.kind)
    }

    /// `FUN_00421020`：这一格能不能走。`base` 是格子自己的判定（记录 +4 字节 > 1）。格子上有在场的物件时：
    /// 类型 5/7/0x13/0x16/0x22/0x25/0x26/0x28、以及「0xb 且参数 0 为 1」按「格子能走 或 可通过」，
    /// 其它按「格子能走 且 可通过」。越界不能走。
    pub fn walkable(&self, layer: usize, x: i32, z: i32, base: bool) -> bool {
        let Some(g) = self.grids.get(layer) else {
            return base;
        };
        if x < 0 || z < 0 || x >= g.w || z >= g.h {
            return false;
        }
        let id = self.maps[layer][(z * g.w + x) as usize];
        let Some(o) = self.objs.get(id as usize) else {
            return base;
        };
        if !o.active {
            return base;
        }
        match o.kind {
            0x16 | 5 | 7 | 0x13 | 0x22 | 0x25 | 0x26 | 0x28 => base || o.non_blocking,
            0xb if o.p0 == 1 => base || o.non_blocking,
            _ => base && o.non_blocking,
        }
    }

    /// 每个登记了格子的物件：（序号, 脚本编号, 类型, 压住的格数, 可不可通过）。调试用。
    pub fn summary(&self) -> Vec<(usize, i32, u8, usize, bool)> {
        let mut counts = vec![0usize; self.objs.len()];
        for map in &self.maps {
            for &id in map {
                if let Some(c) = counts.get_mut(id as usize) {
                    *c += 1;
                }
            }
        }
        self.objs
            .iter()
            .enumerate()
            .filter(|(i, _)| counts[*i] > 0)
            .map(|(i, o)| (i, o.object_id, o.kind, counts[i], o.non_blocking))
            .collect()
    }

    /// 格子上登记的物件号（测试和调试用）。
    pub fn owner(&self, layer: usize, x: i32, z: i32) -> Option<usize> {
        let g = self.grids.get(layer)?;
        if x < 0 || z < 0 || x >= g.w || z >= g.h {
            return None;
        }
        let id = self.maps[layer][(z * g.w + x) as usize];
        (id != NONE).then_some(id as usize)
    }

    /// `FUN_0042ad40`：按类型初始化，顺带改「可通过」、登记包围盒。返回 true 表示还要登记触发矩形（原版返回 0）。
    fn init_object(&mut self, i: usize, n: &BlockNode, model: Model, scene: (&str, &str)) -> bool {
        if n.name == "+3" {
            return false;
        }
        let nb = |b: &mut Self, v: bool| b.objs[i].non_blocking = v;
        match n.kind {
            1 | 2 | 8 | 0xc | 0x10 | 0x13 | 0x17 | 0x1a | 0x1e | 0x29 | 0x2a => {
                self.register(i, n, model, scene);
                nb(self, false);
                false
            }
            5 | 7 => {
                self.register(i, n, model, scene);
                nb(self, true);
                true
            }
            0x24 => {
                nb(self, true);
                true
            }
            0xb => {
                self.register(i, n, model, scene);
                nb(self, n.p0 == 1);
                false
            }
            0xd => {
                self.register(i, n, model, scene);
                false
            }
            0x11 => {
                self.register(i, n, model, scene);
                nb(self, n.p0 as u8 != 0);
                false
            }
            0x16 => {
                self.register(i, n, model, scene);
                nb(self, n.p3 as u8 != 0);
                false
            }
            0x19 if n.p0 == 0 => {
                self.register(i, n, model, scene);
                nb(self, false);
                false
            }
            0x1c => {
                if n.p0 == 1 {
                    nb(self, n.switch_state != 0);
                    self.register(i, n, model, scene);
                }
                false
            }
            0x1d | 0x21 | 0x22 | 0x26 | 0x28 => {
                self.register(i, n, model, scene);
                nb(self, true);
                false
            }
            // 0x18、0x19（参数 0 非 0）、0x1f、0x25：建表时不登记。神树花（0x25）原版这里按花开没开换模型、
            // 登记（`FUN_0042c280`），移植端由载入场景的命令套完存档后补（`ScnScene::switch_flowers`）。
            0x18 | 0x19 | 0x1f | 0x25 => false,
            _ => true,
        }
    }

    /// `FUN_0042a010(0)`：按模型包围盒登记物件压住的格子。
    fn register(&mut self, i: usize, n: &BlockNode, model: Model, scene: (&str, &str)) {
        if n.name.eq_ignore_ascii_case("g01.cvd") {
            return;
        }
        let obb = match model {
            Model::Box(obb) => obb,
            // 存盘点按圆登记，半径写死 3（`FillMapRect`，`gamebase/sceneobj.cpp:3420`），
            // 只要圆心。圆心取 `box.trans.Transform((0,0,0))`，也就是 OBB 自己的中心；
            // 存盘点那个特效给的 OBB 正是以物件原点为中心的——
            // `MXSavePoint::GetOBB`（`effect/MXSavePoint.cpp:135`）写的是
            // `box.trans.LoadIdentity(); box.xlen = box.zlen = 48; box.ylen = 60;`。
            //
            // 名字以 `+` 开头的是特效物件，数字是 `MeshFxClass`
            // （`effect/MeshFxCreator.h:19`）：+0 瘴气、+1 传送点、+2 存盘点、
            // +3 火把、+4 战斗特效。`+4` 那个 `MXCEft::GetOBB` 是个空函数
            // （`MXCEft.cpp:28`），原版读到的是栈上没初始化的 OBB，所以这里跳过它。
            Model::Fx if n.kind == 0x2a && n.name != "+4" => Obb {
                m: Mat44::new_identity(),
                ext: [0.; 3],
            },
            _ => return,
        };
        let layer = n.layer;
        let g = self.grids[layer];
        let id = i as u8;

        // 按类型把包围盒的 x、z 边长加减若干格（0x42a0b8 的跳转表）。
        let [mut x, y, mut z] = obb.ext;
        let u = TILE;
        match n.kind {
            5 | 0x17 => {
                x += u * 2.5;
                z += u * 2.5;
            }
            0xb => {
                if n.p0 == 1 {
                    x += u * 2.5;
                    z += u * 2.5;
                }
            }
            0x13 => {
                x -= u * 3.5;
                z += u * 1.5;
            }
            0x16 | 0x22 => {
                if x > z {
                    x += u * 1.5;
                    z -= u * 1.5;
                } else {
                    x -= u * 1.5;
                    z += u * 1.5;
                }
            }
            7 => {
                if x > z {
                    x += u * 4.5;
                    z -= u * 2.5;
                } else {
                    x -= u * 2.5;
                    z += u * 4.5;
                }
            }
            0x1d | 0x21 => {
                x -= u * 1.5;
                z -= u * 1.5;
            }
            0x1a => {}
            _ => {
                x += u * 1.5;
                z += u * 1.5;
            }
        }
        let (hx, hy, hz) = (x * 0.5, y * 0.5, z * 0.5);
        let world = |d: [f32; 3]| object_point(n, &obb.m, d);

        let rect_of = |b: &Self| {
            // `gbOBBox::GetAABBox`：8 个角变换后的轴对齐包围盒。
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for sx in [-hx, hx] {
                for sy in [-hy, hy] {
                    for sz in [-hz, hz] {
                        let p = world([sx, sy, sz]);
                        for (k, v) in [p.x, p.y, p.z].into_iter().enumerate() {
                            lo[k] = lo[k].min(v);
                            hi[k] = hi[k].max(v);
                        }
                    }
                }
            }
            let _ = b;
            let (x0, z0) = g.tile(&Vec3::new(lo[0], 0., lo[2]));
            let (x1, z1) = g.tile(&Vec3::new(hi[0], 0., hi[2]));
            [x0.min(x1), z0.min(z1), x0.max(x1), z0.max(z1)]
        };

        let scene_is =
            |a: &str, s: &str| scene.0.eq_ignore_ascii_case(a) && scene.1.eq_ignore_ascii_case(s);
        if (n.kind == 0x17 && scene_is("m15", "2")) || (n.kind == 0x16 && scene_is("M22", "3")) {
            let r = rect_of(self);
            self.fill_rect(layer, r, id);
            return;
        }
        match n.kind {
            0x25 | 0x26 | 0x2a => {
                let c = world([0., 0., 0.]);
                let (cx, cz) = g.tile(&c);
                let r = ((x / TILE) as i32).min((z / TILE) as i32) / 2;
                let r = match n.kind {
                    0x26 => r + 1,
                    0x2a => 3,
                    _ => r,
                };
                self.fill_circle(layer, cx, cz, r, id);
            }
            0xb => {
                let r = rect_of(self);
                self.fill_rect(layer, r, id);
            }
            _ => {
                // 六个面，挑变换后 y 起伏最小、x 和 z 都有长度（> 0.05）的那个面当多边形填（立着的盒子挑中顶面）。
                let faces: [[[f32; 3]; 4]; 6] = [
                    [[-hx, hy, hz], [hx, hy, hz], [hx, -hy, hz], [-hx, -hy, hz]],
                    [
                        [-hx, hy, -hz],
                        [hx, hy, -hz],
                        [hx, -hy, -hz],
                        [-hx, -hy, -hz],
                    ],
                    [
                        [-hx, hy, hz],
                        [-hx, hy, -hz],
                        [-hx, -hy, -hz],
                        [-hx, -hy, hz],
                    ],
                    [[hx, hy, hz], [hx, hy, -hz], [hx, -hy, -hz], [hx, -hy, hz]],
                    [[-hx, hy, -hz], [hx, hy, -hz], [hx, hy, hz], [-hx, hy, hz]],
                    [
                        [-hx, -hy, -hz],
                        [hx, -hy, -hz],
                        [hx, -hy, hz],
                        [-hx, -hy, hz],
                    ],
                ];
                let spread = |c: &[Vec3; 4], k: fn(&Vec3) -> f32| {
                    ((k(&c[0]) - k(&c[1])).abs()
                        + (k(&c[1]) - k(&c[2])).abs()
                        + (k(&c[2]) - k(&c[3])).abs())
                    .abs()
                };
                let mut best = 40000.;
                let mut pick = 0;
                let mut corners = [[Vec3::new(0., 0., 0.); 4]; 6];
                for (f, face) in faces.iter().enumerate() {
                    let c = face.map(world);
                    corners[f] = c;
                    let ys = spread(&c, |p| p.y);
                    if ys < best && spread(&c, |p| p.x) > 0.05 && spread(&c, |p| p.z) > 0.05 {
                        best = ys;
                        pick = f;
                    }
                }
                let poly = corners[pick].map(|p| g.tile(&p));
                self.fill_polygon(layer, &poly, id);
            }
        }
    }

    /// `FUN_00420320`：矩形（夹到地图里，两头都含）整块填。
    fn fill_rect(&mut self, layer: usize, r: [i32; 4], id: u8) {
        let g = self.grids[layer];
        let (x0, z0) = (r[0].max(0), r[1].max(0));
        let (x1, z1) = (r[2].min(g.w - 1), r[3].min(g.h - 1));
        for z in z0..=z1 {
            for x in x0..=x1 {
                self.maps[layer][(z * g.w + x) as usize] = id;
            }
        }
    }

    /// `FUN_00421dd0`：圆（外接矩形夹到地图里，(dx² + dz²) ≤ r² 的格子）。
    fn fill_circle(&mut self, layer: usize, cx: i32, cz: i32, r: i32, id: u8) {
        let g = self.grids[layer];
        let (x0, z0) = ((cx - r).max(0), (cz - r).max(0));
        let (x1, z1) = ((cx + r).min(g.w - 1), (cz + r).min(g.h - 1));
        for z in z0..=z1 {
            for x in x0..=x1 {
                if (cx - x) * (cx - x) + (cz - z) * (cz - z) <= r * r {
                    self.maps[layer][(z * g.w + x) as usize] = id;
                }
            }
        }
    }

    /// `FUN_00421d10`：四边形按列扫描。x 取四个顶点的范围（宽不到 4 格时以中点为中心展成 ±2.5 格），
    /// 夹到 [0, 宽)；每一列求出和四条边的交点（`FUN_0042ff40`），正好两个才填，z 同样不到 4 格就展宽，
    /// 夹到 [0, 高)，右端不含。
    fn fill_polygon(&mut self, layer: usize, v: &[(i32, i32); 4], id: u8) {
        let g = self.grids[layer];
        let (x0, x1) = widen(
            v.iter().map(|p| p.0).min().unwrap(),
            v.iter().map(|p| p.0).max().unwrap(),
        );
        for x in x0.max(0)..x1.min(g.w) {
            if let Some((z0, z1)) = column_span(v, x) {
                for z in z0.max(0)..z1.min(g.h) {
                    self.maps[layer][(z * g.w + x) as usize] = id;
                }
            }
        }
    }
}

/// 区间宽不到 4 格时以中点为中心展成 ±2.5（`FUN_0042fec0` / `FUN_0042ff40` 末尾）。
fn widen(lo: i32, hi: i32) -> (i32, i32) {
    if hi - lo < 4 {
        let c = (lo + hi) as f64 * 0.5;
        ((c - 2.5) as i32, (c + 2.5) as i32)
    } else {
        (lo, hi)
    }
}

/// `FUN_0042ff40`：第 x 列和四边形的交点。竖边正好在这一列就取它两个端点；否则逐条边求交（`FUN_004300a0`），
/// 正好两个交点才算。
fn column_span(v: &[(i32, i32); 4], x: i32) -> Option<(i32, i32)> {
    let mut hits = [0; 4];
    let mut count = 0;
    for i in 0..4 {
        let (a, b) = (v[i], v[(i + 1) & 3]);
        if a.0 == b.0 {
            if x == a.0 {
                hits[0] = a.1;
                hits[1] = b.1;
                count = 2;
                break;
            }
        } else if let Some(z) = edge_z(x, a, b) {
            hits[count] = z;
            count += 1;
        }
    }
    if count != 2 {
        return None;
    }
    Some(widen(hits[0].min(hits[1]), hits[0].max(hits[1])))
}

/// `FUN_004300a0`：x 落在 (较小端, 较大端] 里时按直线插出 z（加 0.5 截断），z 还得在边的 z 范围里。
fn edge_z(x: i32, a: (i32, i32), b: (i32, i32)) -> Option<i32> {
    if x > a.0.max(b.0) || x <= a.0.min(b.0) {
        return None;
    }
    let slope = (b.1 - a.1) as f64 / (b.0 - a.0) as f64;
    let z = (a.1 as f64 - a.0 as f64 * slope + x as f64 * slope + 0.5) as i32;
    (a.1.min(b.1) <= z && z <= a.1.max(b.1)).then_some(z)
}

/// `FUN_00429ee0`：包围盒上一点到世界——先过模型包围盒矩阵，再绕 X 转、绕 Y 转（gbengine 的
/// `SetAxisAngle`：绕 X y' = y cos − z sin、z' = y sin + z cos；绕 Y x' = x cos + z sin、
/// z' = −x sin + z cos），最后平移到物件位置。
fn object_point(n: &BlockNode, m: &Mat44, d: [f32; 3]) -> Vec3 {
    let f = m.floats();
    let p = [
        f[0][0] * d[0] + f[0][1] * d[1] + f[0][2] * d[2] + f[0][3],
        f[1][0] * d[0] + f[1][1] * d[1] + f[1][2] * d[2] + f[1][3],
        f[2][0] * d[0] + f[2][1] * d[1] + f[2][2] * d[2] + f[2][3],
    ];
    let (sx, cx) = (n.x_deg * DEG).sin_cos();
    let (x, y, z) = (p[0], p[1] * cx - p[2] * sx, p[1] * sx + p[2] * cx);
    let (sy, cy) = (n.y_deg * DEG).sin_cos();
    Vec3::new(
        x * cy + z * sy + n.position.x,
        y + n.position.y,
        -x * sy + z * cy + n.position.z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> LayerGrid {
        LayerGrid {
            w: 40,
            h: 40,
            min_x: 0.,
            min_z: 0.,
        }
    }

    fn node(kind: u8) -> BlockNode {
        BlockNode {
            object_id: 1,
            name: "box.pol".into(),
            kind,
            trigger_type: 2,
            non_blocking: 1,
            init_active: true,
            switch_state: 0,
            layer: 0,
            p0: 0,
            p3: 0,
            trigger_rect: [0; 4],
            position: Vec3::new(240., 0., 240.),
            y_deg: 0.,
            x_deg: 0.,
        }
    }

    fn boxed(ext: f32) -> Model {
        Model::Box(Obb {
            m: Mat44::new_identity(),
            ext: [ext, ext, ext],
        })
    }

    #[test]
    fn a_chest_blocks_its_footprint_widened_by_a_tile_and_a_half() {
        // 箱子（0x29）：边长 24 外扩 1.5 格成 42，顶面投到格子上是 18..21。x 方向宽 3 不到 4 也展成
        // 17..22，可 17、22 两列求不出交点，照样不填；每一列的 z 区间 18..21 展成 17..22（右端不含）。
        let b = ObjectBlocking::build(&[node(0x29)], &[boxed(24.)], vec![grid()], ("q01", "1"));
        assert_eq!(b.owner(0, 19, 19), Some(0));
        assert_eq!(b.owner(0, 18, 17), Some(0));
        assert_eq!(b.owner(0, 21, 21), Some(0));
        assert_eq!(b.owner(0, 17, 19), None);
        assert_eq!(b.owner(0, 22, 19), None);
        assert_eq!(b.owner(0, 19, 22), None);
        assert!(!b.walkable(0, 19, 19, true), "箱子的可通过被改成 0");
        assert!(b.walkable(0, 25, 25, true));
    }

    #[test]
    fn an_inactive_object_does_not_block() {
        let mut b = ObjectBlocking::build(&[node(0x29)], &[boxed(24.)], vec![grid()], ("q01", "1"));
        b.set_active(1, false);
        assert!(b.walkable(0, 19, 19, true));
    }

    #[test]
    fn a_bridge_makes_its_tiles_walkable() {
        // 剑桥（0x22）按「格子能走 或 可通过」判，可通过被改成 1。
        let b = ObjectBlocking::build(&[node(0x22)], &[boxed(60.)], vec![grid()], ("q01", "1"));
        assert!(b.walkable(0, 20, 20, false));
        assert!(!b.walkable(0, 2, 2, false));
    }

    #[test]
    fn a_saving_point_blocks_a_circle_of_radius_three() {
        let mut n = node(0x2a);
        n.name = "+5".into();
        let b = ObjectBlocking::build(&[n], &[Model::Fx], vec![grid()], ("q01", "1"));
        assert_eq!(b.owner(0, 20, 23), Some(0));
        assert_eq!(b.owner(0, 22, 22), Some(0));
        assert_eq!(b.owner(0, 23, 23), None, "3² + 3² > 3²");
    }

    #[test]
    fn tile_triggers_register_their_rectangle_after_the_model() {
        let mut n = node(0);
        n.trigger_type = 1;
        n.trigger_rect = [3, 4, 5, 6];
        let b = ObjectBlocking::build(&[n], &[Model::None], vec![grid()], ("q01", "1"));
        assert_eq!(b.owner(0, 3, 4), Some(0));
        assert_eq!(b.owner(0, 5, 6), Some(0));
        assert_eq!(b.owner(0, 6, 6), None);
        assert!(b.walkable(0, 4, 5, true), "可通过是 1 时不挡");
        assert!(!b.walkable(0, 4, 5, false));
    }

    #[test]
    fn rotation_follows_gbengine_axis_angle() {
        let mut n = node(0x29);
        n.y_deg = 90.;
        let p = object_point(&n, &Mat44::new_identity(), [10., 0., 0.]);
        assert!(
            (p.x - 240.).abs() < 1e-3 && (p.z - 230.).abs() < 1e-3,
            "{p:?}"
        );
        n.y_deg = 0.;
        n.x_deg = 90.;
        let p = object_point(&n, &Mat44::new_identity(), [0., 10., 0.]);
        assert!((p.z - 250.).abs() < 1e-3 && p.y.abs() < 1e-3, "{p:?}");
    }

    #[test]
    fn a_column_through_vertices_hits_each_edge_on_one_side_only() {
        // 菱形：x = 10 那一列穿过上下两个顶点。边的范围左开右闭，只有左边两条边算交上，正好两个。
        let v = [(10, 0), (20, 10), (10, 20), (0, 10)];
        assert_eq!(column_span(&v, 10), Some((0, 20)));
        assert_eq!(column_span(&v, 0), None, "最左那一列一条边都不算交上");
        assert_eq!(column_span(&v, 15), Some((5, 15)));
    }
}
