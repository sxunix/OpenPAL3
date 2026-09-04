use crate::openpal3::comdef::ICvdModel;
use crate::openpal3::loaders::cvd_loader::{
    CvdMaterial, CvdModelNode, CvdPositionKeyFrames, CvdRotationKeyFrames, CvdScaleKeyFrames,
    CvdVertex, cvd_load_from_file,
};
use crosscom::ComRc;
use mini_fs::{MiniFs, StoreExt};
use radiance::comdef::{IAnimatedMeshComponent, IComponent, IComponentImpl, IEntity, IEntityExt};
use radiance::components::mesh::{AnimatedMeshComponent, Geometry, MorphTarget};
use radiance::math::{Mat44, Quaternion, Vec2, Vec3};
use radiance::rendering::{
    ComponentFactory, MaterialDef, Pal3PropMaterialDef, VertexBuffer, VertexComponents,
};
use radiance::scene::CoreEntity;
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
};

pub fn create_entity_from_cvd_model<P: AsRef<Path>>(
    component_factory: Rc<dyn ComponentFactory>,
    vfs: &MiniFs,
    path: P,
    name: String,
    visible: bool,
) -> ComRc<IEntity> {
    let entity = CoreEntity::create(name, visible);
    let mesh_component = CvdModelComponent::new(entity.clone(), component_factory.clone());
    entity.add_component(
        ICvdModel::uuid(),
        crosscom::ComRc::from_object(mesh_component),
    );

    let cvd = cvd_load_from_file(vfs, path.as_ref()).unwrap();
    for (_i, node) in cvd.models.iter().enumerate() {
        entity.attach(new_from_cvd_model_node(
            component_factory.clone(),
            vfs,
            path.as_ref(),
            node,
            visible,
        ));
    }

    entity
}

fn new_from_cvd_model_node<P: AsRef<Path>>(
    component_factory: Rc<dyn ComponentFactory>,
    vfs: &MiniFs,
    path: P,
    node: &CvdModelNode,
    visible: bool,
) -> ComRc<IEntity> {
    let mut scale_factor = 1.;
    let mut position_keyframes = None;
    let mut rotation_keyframes = None;
    let mut scale_keyframes = None;
    let mut meshes = vec![];
    // One geometry list per mesh frame, so the frames can be handed to the
    // engine's morph animation. `meshes` keeps frame 0 for the static path.
    let mut frame_geometries: Vec<Vec<Geometry>> = vec![];
    let mut animation_time_keys: Vec<f32> = vec![];
    if let Some(model) = &node.model {
        frame_geometries = vec![vec![]; model.mesh.frames.len()];
        animation_time_keys = model.mesh.animation_time_keys.clone();
        position_keyframes = model.position_keyframes.clone();
        rotation_keyframes = model.rotation_keyframes.clone();
        scale_keyframes = model.scale_keyframes.clone();

        for material in &model.mesh.materials {
            if material.triangles.is_none() {
                continue;
            }
            let mat = load_texture(material, vfs, path.as_ref());

            for (frame, verts) in model.mesh.frames.iter().enumerate() {
                let mesh = CvdMesh::new(verts, material, mat.clone());
                if frame == 0 {
                    meshes.push(mesh.clone());
                }
                frame_geometries[frame].push(Geometry {
                    material: mesh.material,
                    vertices: mesh.vertices,
                    indices: mesh.indices,
                });
            }
        }

        scale_factor = model.scale_factor;
    }

    let entity = CoreEntity::create("cvd_obj".to_string(), true);

    let mesh_component = CvdModelComponent {
        entity: entity.clone(),
        component_factory: component_factory.clone(),
        position_keyframes,
        rotation_keyframes,
        scale_keyframes,
        // With more than one frame the vertices are driven by the morph
        // component below, so the static path must not also build render
        // objects for them.
        meshes: if frame_geometries.len() > 1 {
            vec![]
        } else {
            meshes
        },
        scale_factor: Cell::new(scale_factor),
        play_mode: RefCell::new(CvdPlayMode::Hold),
        speed: Cell::new(1.),
        elapsed: Cell::new(0.),
    };

    mesh_component.setup_transform(entity.clone(), scale_factor);

    entity.add_component(
        ICvdModel::uuid(),
        crosscom::ComRc::from_object(mesh_component),
    );

    // A CVD mesh's own animation is a vertex morph across its frames —
    // `CvdModelRenderer::UpdateMesh` in Pal3.Unity rebuilds the vertex
    // buffer every frame from a frame index and an influence. That is the
    // same shape as an MV3 actor's morph targets, so it reuses the engine's
    // animated mesh rather than a second vertex-upload path. Timestamps are
    // rebased so the first frame sits at 0, as Pal3.Unity's reader does.
    if frame_geometries.len() > 1 {
        let base = animation_time_keys.first().copied().unwrap_or(0.);
        let targets: Vec<MorphTarget> = frame_geometries
            .into_iter()
            .enumerate()
            .map(|(i, geometries)| {
                let t = animation_time_keys.get(i).copied().unwrap_or(0.) - base;
                MorphTarget::new(geometries, t, component_factory.clone())
            })
            .collect();

        let animated = AnimatedMeshComponent::new(entity.clone(), component_factory.clone());
        animated.set_morph_targets(targets, 0.);
        animated.set_loop_playback(true);
        let animated: ComRc<IAnimatedMeshComponent> = ComRc::from_object(animated);
        animated.play(true);
        entity.add_component(
            IAnimatedMeshComponent::uuid(),
            animated.query_interface::<IComponent>().unwrap(),
        );
    }

    if let Some(children) = &node.children {
        for child in children {
            entity.attach(new_from_cvd_model_node(
                component_factory.clone(),
                vfs,
                path.as_ref(),
                &child,
                visible,
            ));
        }
    }

    entity
}

fn load_texture<P: AsRef<Path>>(
    material: &CvdMaterial,
    vfs: &MiniFs,
    model_path: P,
) -> MaterialDef {
    let mut model_dir = model_path.as_ref().to_owned();
    model_dir.pop();
    let dds_path = model_dir.join(PathBuf::from(&material.texture_name).with_extension("dds"));
    let texture_path = if vfs.open(&dds_path).is_ok() {
        dds_path
    } else {
        model_dir.join(&material.texture_name)
    };

    // CVD props carry no baked lightmap; like the original engine, shade them
    // with the scene's dynamic `.lgt` lights via the lit material (CVD vertices
    // ship normals). Keep the default `BlendMode::AlphaTest` cutout fallback,
    // matching the legacy renderer's "always cutout" behavior.
    Pal3PropMaterialDef::create(texture_path.to_str().unwrap(), |name| vfs.open(name).ok())
}

/// How a CVD node's keyframe tracks are driven.
///
/// `Hold` is the default and reproduces what the port did before there
/// was any playback: frame 0, frozen. The original decides per scene
/// object whether a prop animates; until that exists, nothing moves
/// unless a command asks it to — `SceneOpenDoor` (87) is the first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CvdPlayMode {
    #[default]
    Hold,
    Once,
    Loop,
}

/// Pick the two keyframes bracketing `t` and how far between them it
/// falls. `None` when the track is absent or empty; a single keyframe
/// yields itself twice at weight 0.
fn sample<T>(frames: Option<&Vec<T>>, t: f32, at: impl Fn(&T) -> f32) -> Option<(&T, &T, f32)> {
    let frames = frames?;
    let first = frames.first()?;
    if t <= at(first) {
        return Some((first, first, 0.));
    }
    let last = frames.last()?;
    if t >= at(last) {
        return Some((last, last, 0.));
    }
    let i = frames.partition_point(|f| at(f) <= t).max(1);
    let (a, b) = (&frames[i - 1], &frames[i]);
    let span = at(b) - at(a);
    let w = if span > f32::EPSILON {
        (t - at(a)) / span
    } else {
        0.
    };
    Some((a, b, w))
}

pub struct CvdModelComponent {
    entity: ComRc<IEntity>,
    component_factory: Rc<dyn ComponentFactory>,
    position_keyframes: Option<CvdPositionKeyFrames>,
    rotation_keyframes: Option<CvdRotationKeyFrames>,
    scale_keyframes: Option<CvdScaleKeyFrames>,
    meshes: Vec<CvdMesh>,
    /// The node's own uniform scale, kept so the transform can be rebuilt
    /// from identity each frame rather than accumulated.
    scale_factor: Cell<f32>,
    play_mode: RefCell<CvdPlayMode>,
    speed: Cell<f32>,
    elapsed: Cell<f32>,
}

ComObject_CvdModel!(super::CvdModelComponent);

impl IComponentImpl for CvdModelComponent {
    fn on_loading(&self) -> crosscom::Void {
        // With more than one mesh frame the vertices belong to the morph
        // component, which builds and owns the rendering. Setting an empty
        // rendering component here would clobber it.
        if self.meshes.is_empty() {
            return;
        }

        let mut objects = vec![];
        for mesh in &self.meshes {
            let ro = self.component_factory.create_render_object(
                mesh.vertices.clone(),
                mesh.indices.clone(),
                &mesh.material,
                false,
            );

            objects.push(ro);
        }

        let component = self.component_factory.create_rendering_component(objects);
        self.entity
            .set_rendering_component(Some(Rc::new(component)));
    }

    fn on_unloading(&self) {}

    fn on_updating(&self, delta_sec: f32) -> crosscom::Void {
        let mode = *self.play_mode.borrow();
        if matches!(mode, CvdPlayMode::Hold) {
            return;
        }
        let duration = self.duration();
        if duration <= 0. {
            return;
        }

        let mut t = self.elapsed.get() + delta_sec * self.speed.get();
        match mode {
            CvdPlayMode::Loop => {
                while t > duration {
                    t -= duration;
                }
            }
            CvdPlayMode::Once => t = t.min(duration),
            CvdPlayMode::Hold => unreachable!(),
        }
        self.elapsed.set(t);
        self.apply_at(t);
    }
}

impl CvdModelComponent {
    pub fn new(entity: ComRc<IEntity>, component_factory: Rc<dyn ComponentFactory>) -> Self {
        Self {
            entity,
            component_factory: component_factory.clone(),
            position_keyframes: None,
            rotation_keyframes: None,
            scale_keyframes: None,
            meshes: vec![],
            scale_factor: Cell::new(1.),
            play_mode: RefCell::new(CvdPlayMode::Hold),
            speed: Cell::new(1.),
            elapsed: Cell::new(0.),
        }
    }

    /// Start the node's keyframe animation. `speed` scales time —
    /// `SceneOpenDoor` (opcode 87) runs the door at 2.0
    /// (`gbVisNode::SetTimeScale` at 0x42bea0).
    pub fn play(&self, mode: CvdPlayMode, speed: f32) {
        *self.play_mode.borrow_mut() = mode;
        self.speed.set(speed.max(0.001));
        self.elapsed.set(0.);
    }

    /// Whether a one-shot animation has run to its end. `SceneOpenDoor`
    /// blocks the script on this.
    pub fn finished(&self) -> bool {
        matches!(*self.play_mode.borrow(), CvdPlayMode::Hold)
            || (matches!(*self.play_mode.borrow(), CvdPlayMode::Once)
                && self.elapsed.get() >= self.duration())
    }

    /// Longest timestamp across the three tracks.
    fn duration(&self) -> f32 {
        let last = |t: Option<f32>| t.unwrap_or(0.);
        let p = self
            .position_keyframes
            .as_ref()
            .and_then(|k| k.frames.last())
            .map(|f| f.timestamp);
        let r = self
            .rotation_keyframes
            .as_ref()
            .and_then(|k| k.frames.last())
            .map(|f| f.timestamp);
        let sc = self
            .scale_keyframes
            .as_ref()
            .and_then(|k| k.frames.last())
            .map(|f| f.timestamp);
        last(p).max(last(r)).max(last(sc))
    }

    /// Rebuild the node's transform for time `t`, from identity — the
    /// tracks compose, so they cannot be applied incrementally.
    fn apply_at(&self, t: f32) {
        let entity = &self.entity;
        entity
            .transform()
            .borrow_mut()
            .set_matrix(Mat44::new_identity());

        if let Some(p) = sample(
            self.position_keyframes.as_ref().map(|k| &k.frames),
            t,
            |f| f.timestamp,
        )
        .map(|(a, b, w)| Vec3::lerp(&a.position, &b.position, w))
        {
            entity.transform().borrow_mut().translate_local(&p);
        }

        let sf = self.scale_factor.get();
        entity
            .transform()
            .borrow_mut()
            .scale_local(&Vec3::new(sf, sf, sf));

        if let Some(q) = sample(
            self.rotation_keyframes.as_ref().map(|k| &k.frames),
            t,
            |f| f.timestamp,
        )
        .map(|(a, b, w)| Quaternion::slerp(&a.quaternion, &b.quaternion, w))
        {
            entity.transform().borrow_mut().rotate_quaternion_local(&q);
        }

        if let Some((a, b, w)) = sample(self.scale_keyframes.as_ref().map(|k| &k.frames), t, |f| {
            f.timestamp
        }) {
            let scale = Vec3::lerp(&a.scale, &b.scale, w);
            let q2 = Quaternion::slerp(&a.quaternion, &b.quaternion, w);
            let mut q3 = q2;
            q3.inverse();
            entity
                .transform()
                .borrow_mut()
                .rotate_quaternion_local(&q2)
                .scale_local(&scale)
                .rotate_quaternion_local(&q3);
        }
    }

    pub fn setup_transform(&self, entity: ComRc<IEntity>, scale_factor: f32) {
        self.scale_factor.set(scale_factor);
        if let Some(p) = self
            .position_keyframes
            .as_ref()
            .and_then(|frame| frame.frames.get(0))
            .and_then(|f| Some(f.position))
        {
            entity.transform().borrow_mut().translate_local(&p);
        }

        entity.transform().borrow_mut().scale_local(&Vec3::new(
            scale_factor,
            scale_factor,
            scale_factor,
        ));

        if let Some(q) = self
            .rotation_keyframes
            .as_ref()
            .and_then(|frame| frame.frames.get(0))
            .and_then(|f| Some(f.quaternion))
        {
            entity.transform().borrow_mut().rotate_quaternion_local(&q);
        }

        if let Some(frame) = self
            .scale_keyframes
            .as_ref()
            .and_then(|frame| frame.frames.get(0))
        {
            let scale = frame.scale;
            let q2 = frame.quaternion;
            let mut q3 = q2;
            q3.inverse();

            entity
                .transform()
                .borrow_mut()
                .rotate_quaternion_local(&q2)
                .scale_local(&scale)
                .rotate_quaternion_local(&q3);
        }
    }
}

#[derive(Clone)]
struct CvdMesh {
    material: MaterialDef,
    vertices: VertexBuffer,
    indices: Vec<u32>,
}

impl CvdMesh {
    pub fn new(
        all_vertices: &Vec<CvdVertex>,
        cvd_material: &CvdMaterial,
        material: MaterialDef,
    ) -> Self {
        let components =
            VertexComponents::POSITION | VertexComponents::NORMAL | VertexComponents::TEXCOORD;

        let mut index_map = std::collections::HashMap::new();
        let mut reversed_index = vec![];
        let mut get_new_index = |index: u16| -> u32 {
            if index_map.contains_key(&index) {
                index_map[&index]
            } else {
                let new_index = reversed_index.len() as u32;
                reversed_index.push(index as usize);
                index_map.insert(index, new_index);
                new_index
            }
        };

        let mut indices: Vec<u32> = vec![];
        for t in cvd_material.triangles.as_ref().unwrap() {
            indices.push(get_new_index(t.indices[0]));
            indices.push(get_new_index(t.indices[1]));
            indices.push(get_new_index(t.indices[2]));
        }

        let mut vertices = VertexBuffer::new(components, reversed_index.len());
        for i in 0..reversed_index.len() {
            let vert = &all_vertices[reversed_index[i]];
            vertices.set_data(
                i,
                Some(&Vec3::new(
                    vert.position.x,
                    vert.position.y,
                    vert.position.z,
                )),
                Some(&Vec3::new(vert.normal.x, vert.normal.y, vert.normal.z)),
                Some(&Vec2::new(vert.tex_coord.x, vert.tex_coord.y)),
                None,
            );
        }

        CvdMesh {
            material,
            vertices,
            indices,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::sample;

    #[derive(Debug, PartialEq)]
    struct F(f32, i32);

    fn frames() -> Vec<F> {
        vec![F(0., 10), F(1., 20), F(3., 40)]
    }

    #[test]
    fn a_time_before_the_first_key_holds_it() {
        let f = frames();
        let (a, b, w) = sample(Some(&f), -1., |x| x.0).unwrap();
        assert_eq!((a.1, b.1, w), (10, 10, 0.));
    }

    #[test]
    fn a_time_past_the_last_key_holds_it() {
        let f = frames();
        let (a, b, w) = sample(Some(&f), 99., |x| x.0).unwrap();
        assert_eq!((a.1, b.1, w), (40, 40, 0.));
    }

    #[test]
    fn a_time_between_keys_brackets_them_with_a_weight() {
        let f = frames();
        let (a, b, w) = sample(Some(&f), 2., |x| x.0).unwrap();
        assert_eq!((a.1, b.1), (20, 40));
        assert!((w - 0.5).abs() < 1e-6, "1..3 的中点权重应为 0.5，实得 {w}");
    }

    #[test]
    fn an_exact_key_time_lands_on_that_key() {
        let f = frames();
        let (a, _, w) = sample(Some(&f), 1., |x| x.0).unwrap();
        assert_eq!(a.1, 20);
        assert_eq!(w, 0.);
    }

    #[test]
    fn a_missing_or_empty_track_samples_to_nothing() {
        assert!(sample::<F>(None, 0., |x| x.0).is_none());
        let empty: Vec<F> = vec![];
        assert!(sample(Some(&empty), 0., |x| x.0).is_none());
    }
}
