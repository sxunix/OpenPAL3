//! 存盘点的特效（MeshFx 第 2 类 `MXSavePoint`，`.scn` 里名字是 `+2` 的物件；exe 建在 0x4b7d30，
//! 虚表 0x56f460）。
//!
//! 三样东西，都摆在物件的位置、按物件朝向（+0xe0）转过：
//!
//! * **石头**（`effect\stone.pol`，`Attach` 0x4b7f80）：浮在 `25 + 8·sin(3t)` 高处，绕 Y 转 `3t` 度。
//! * **地上那块**（`Effect\save.tga`，一个四边形，边长 60、高 2，白色）和**一圈光柱**（`Effect\ray.tga`，
//!   12 面锥：底半径 24、顶半径 40、高 50，贴图横着铺 3 遍，`ev` 改成 0.94，颜色 (245,188,28)），
//!   `Render` 0x4b80c0：先抬高 1，再绕 Y 转 `−t` 弧度（`−57.29578·t` 度），加法混合
//!   （`gfxscript\AdditiveBlend.gbf`）。
//!
//! `t` 是 `Update`（0x4b7f60）每帧加上的 `dt`。
//!
//! [推断] `gbDynVertBuf::RenderGeomQuad` 没逆，四个顶点的贴图坐标按 `(su,sv) (eu,sv) (eu,ev) (su,ev)` 排。

use std::rc::Rc;

use crosscom::ComRc;
use radiance::comdef::{IEntity, IEntityExt};
use radiance::components::mesh::{Geometry, TexCoord};
use radiance::math::{Vec2, Vec3};
use radiance::rendering::{BlendMode, ComponentFactory, ParticleMaterialDef, VertexComponents};
use radiance::scene::CoreEntity;

use super::particles::pack_color;

/// 光柱的面数（`SAVEPOINT_MAX_SIDE`）。
pub const SIDES: usize = 12;
/// 地上那块的半边长（`±30`）和高度（2）。
pub const GROUND_HALF: f32 = 30.;
pub const GROUND_Y: f32 = 2.;
/// `Geom_NewCone(40, 24, 50, 12, …, 3)`（exe `FUN_004b63c0`）：顶半径、底半径、高、贴图铺几遍。
pub const CONE_TOP: f32 = 40.;
pub const CONE_BOTTOM: f32 = 24.;
pub const CONE_HEIGHT: f32 = 50.;
pub const CONE_TEX_LOOP: f32 = 3.;
/// 光柱四边形的 `ev`（0x3f70a3d7）。
pub const CONE_EV: f32 = 0.94;
/// 光柱的颜色（`RGB(245,188,28)`，alpha 255）。
pub const CONE_COLOR: [f32; 4] = [245. / 255., 188. / 255., 28. / 255., 1.];

/// 一个四边形：四个顶点（局部坐标）和四个贴图坐标。
#[derive(Debug, Clone, Copy)]
pub struct Quad {
    pub v: [[f32; 3]; 4],
    pub uv: [[f32; 2]; 4],
}

fn quad_uv(su: f32, sv: f32, eu: f32, ev: f32) -> [[f32; 2]; 4] {
    [[su, sv], [eu, sv], [eu, ev], [su, ev]]
}

/// 地上那块（`m_trans`）。
pub fn ground_quad() -> Quad {
    let s = GROUND_HALF;
    Quad {
        v: [
            [-s, GROUND_Y, -s],
            [-s, GROUND_Y, s],
            [s, GROUND_Y, s],
            [s, GROUND_Y, -s],
        ],
        uv: quad_uv(0., 0., 1., 1.),
    }
}

/// `Geom_NewCone`（`effect\GeomFactory.cpp:12`，exe `FUN_004b63c0`）：第 i 面是底圈 i、i+1 两点和顶圈
/// i+1、i 两点，`su` 每面加 1/铺几遍；建完再把 `ev` 改成 0.94。
pub fn cone_quads() -> Vec<Quad> {
    let step = std::f32::consts::PI * 2. / SIDES as f32;
    let du = 1. / CONE_TEX_LOOP;
    (0..SIDES)
        .map(|i| {
            let (a, b) = (i as f32 * step, i as f32 * step + step);
            let su = i as f32 * du;
            Quad {
                v: [
                    [CONE_BOTTOM * a.cos(), 0., CONE_BOTTOM * a.sin()],
                    [CONE_BOTTOM * b.cos(), 0., CONE_BOTTOM * b.sin()],
                    [CONE_TOP * b.cos(), CONE_HEIGHT, CONE_TOP * b.sin()],
                    [CONE_TOP * a.cos(), CONE_HEIGHT, CONE_TOP * a.sin()],
                ],
                uv: quad_uv(su, 0., su + du, CONE_EV),
            }
        })
        .collect()
}

/// 石头这一帧离物件原点多高（`25 + 8·sin(3t)`）、绕 Y 转了多少度（`3t`）。
pub fn stone_pose(t: f32) -> (f32, f32) {
    (25. + 8. * (t * 3.).sin(), t * 3.)
}

/// 绕 Y 转 `deg` 度（和 `object_block::object_point` 的朝向同一个转法）。
fn yaw(v: [f32; 3], deg: f32) -> [f32; 3] {
    let (s, c) = deg.to_radians().sin_cos();
    [v[0] * c + v[2] * s, v[1], -v[0] * s + v[2] * c]
}

/// 局部顶点在这一帧的世界坐标：先抬高 1、绕 Y 转 `−t` 弧度，再按物件朝向转、平移到物件位置。
pub fn place(v: [f32; 3], t: f32, base: &Vec3, rotate: f32) -> Vec3 {
    let p = yaw(v, (-t).to_degrees());
    let p = yaw([p[0], p[1] + 1., p[2]], rotate);
    Vec3::new(base.x + p[0], base.y + p[1], base.z + p[2])
}

/// 场景里的一个存盘点特效。
pub struct SavePointFx {
    /// `.scn` 序号。
    pub node: usize,
    pub base: Vec3,
    /// 物件朝向（度）。
    pub rotate: f32,
    pub t: f32,
    pub stone: Option<ComRc<IEntity>>,
    pub ground: ComRc<IEntity>,
    pub cone: ComRc<IEntity>,
}

/// 建一个 `count` 个四边形的加法混合网格，顶点每帧重写。
pub fn build_quads_entity(
    factory: &Rc<dyn ComponentFactory>,
    name: String,
    texture_name: &str,
    texture: image::RgbaImage,
    count: usize,
) -> ComRc<IEntity> {
    let mut verts = Vec::with_capacity(count * 4);
    let mut uv = Vec::with_capacity(count * 4);
    let mut packed = Vec::with_capacity(count * 4);
    let mut indices = Vec::with_capacity(count * 6);
    for i in 0..count as u32 {
        for _ in 0..4 {
            verts.push(Vec3::new(0., 0., 0.));
            uv.push(TexCoord::new(0., 0.));
            packed.push(TexCoord::new(0., 0.));
        }
        let b = i * 4;
        indices.extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    let material =
        ParticleMaterialDef::create_with_image(texture_name, Some(texture), BlendMode::Additive);
    let geometry = Geometry::new(&verts, None, &[uv, packed], indices, material);
    let object = factory.create_render_object(
        geometry.vertices.clone(),
        geometry.indices.clone(),
        &geometry.material,
        true,
    );
    let component = factory.create_rendering_component(vec![object]);
    let entity = CoreEntity::create(name, true);
    entity.set_rendering_component(Some(Rc::new(component)));
    entity
}

/// 把四边形按这一帧的转角写进网格。
pub fn write_quads(entity: &ComRc<IEntity>, quads: &[Quad], color: [f32; 4], fx: &SavePointFx) {
    let Some(rc) = entity.get_rendering_component() else {
        return;
    };
    let packed = pack_color(color);
    for object in rc.render_objects() {
        object.as_dyn().update_vertices(&|mut buffer| {
            for (i, q) in quads.iter().enumerate() {
                for k in 0..4 {
                    let v = i * 4 + k;
                    let p = place(q.v[k], fx.t, &fx.base, fx.rotate);
                    buffer.set_component(v, VertexComponents::POSITION, |o: &mut Vec3| *o = p);
                    buffer.set_component(v, VertexComponents::TEXCOORD, |o: &mut Vec2| {
                        *o = Vec2::new(q.uv[k][0], q.uv[k][1])
                    });
                    buffer.set_component(v, VertexComponents::TEXCOORD2, |o: &mut Vec2| {
                        *o = Vec2::new(packed.0, packed.1)
                    });
                }
            }
        });
    }
}

impl SavePointFx {
    /// `Update` + 摆位置：时间加 `dt`，石头挪到这一帧的高度、转角，两块网格重写。
    pub fn step(&mut self, dt: f32) {
        self.t += dt;
        if let Some(stone) = &self.stone {
            let (h, deg) = stone_pose(self.t);
            let p = Vec3::new(self.base.x, self.base.y + h, self.base.z);
            stone
                .transform()
                .borrow_mut()
                .set_position(&p)
                .clear_rotation()
                .rotate_axis_angle_local(&Vec3::UP, (self.rotate + deg).to_radians());
        }
        write_quads(&self.ground, &[ground_quad()], [1., 1., 1., 1.], self);
        write_quads(&self.cone, &cone_quads(), CONE_COLOR, self);
    }

    pub fn set_visible(&self, visible: bool) {
        if let Some(s) = &self.stone {
            s.set_visible(visible);
        }
        self.ground.set_visible(visible);
        self.cone.set_visible(visible);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cone_matches_geom_new_cone() {
        let q = cone_quads();
        assert_eq!(q.len(), 12);
        // 第 0 面：底圈 0°、30°，顶圈 30°、0°；贴图 0..1/3、v 0..0.94。
        assert!((q[0].v[0][0] - 24.).abs() < 1e-4 && q[0].v[0][1] == 0.);
        assert!((q[0].v[3][0] - 40.).abs() < 1e-4 && q[0].v[3][1] == 50.);
        assert!((q[1].uv[0][0] - 1. / 3.).abs() < 1e-6);
        assert_eq!(q[0].uv[2][1], 0.94);
    }

    #[test]
    fn stone_bobs_between_17_and_33() {
        assert_eq!(stone_pose(0.), (25., 0.));
        let (h, deg) = stone_pose(std::f32::consts::FRAC_PI_6);
        assert!((h - 33.).abs() < 1e-4);
        assert!((deg - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    }

    #[test]
    fn quads_are_lifted_by_one_and_spun_backwards() {
        let base = Vec3::new(100., 10., 200.);
        let p = place([30., 2., 0.], 0., &base, 0.);
        assert!((p.x - 130.).abs() < 1e-4 && (p.y - 13.).abs() < 1e-4 && (p.z - 200.).abs() < 1e-4);
        // 转 −π/2 弧度：(30, 0, 0) 绕 Y 转 −90° 到 (0, 0, 30)。
        let p = place([30., 0., 0.], std::f32::consts::FRAC_PI_2, &base, 0.);
        assert!((p.x - 100.).abs() < 1e-3 && (p.z - 230.).abs() < 1e-3);
    }
}
