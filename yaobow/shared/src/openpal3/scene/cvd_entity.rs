use crate::openpal3::comdef::ICvdModel;
use crate::openpal3::loaders::cvd_loader::{
    CvdMaterial, CvdModelNode, CvdPositionKeyFrames, CvdRotationKeyFrames, CvdScaleKeyFrames,
    CvdVertex, cvd_load_from_file,
};
use crosscom::ComRc;
use mini_fs::{MiniFs, StoreExt};
use radiance::comdef::{IAnimatedMeshComponent, IComponent, IComponentImpl, IEntity, IEntityExt};
use radiance::components::mesh::{
    AnimatedMeshComponent, Geometry, MorphAnimationState, MorphTarget,
};
use radiance::math::{Mat44, Quaternion, Vec2, Vec3};
use radiance::rendering::{
    ComponentFactory, MaterialDef, Pal3GeomMaterialDef, VertexBuffer, VertexComponents,
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
        // 停在第 0 帧，播不播、怎么播由 `CvdModelComponent::play` 统一管——场景
        // 加载时按物件类型定（`scene_object_initial_play`），开门、机关由命令触发。
        animated.stop();
        let animated: ComRc<IAnimatedMeshComponent> = ComRc::from_object(animated);
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

    // CVD 在原版里和 POL 同样是 `gbGeomNode`，走同一条 `AttachSelf`
    // （gbengine.dll `0x10011280`）：没有光照贴图 ⇒ 挂灯的 flag 传 0 ⇒ 照样吃
    // 最多两盏灯，落到 `geom_1L` / `geom_2L`。所以这里和 POL 单贴图面用同一套
    // 材质，不另给「只有环境光」的那一套。CVD 顶点自带法线。
    // 保留默认的 `BlendMode::AlphaTest` 抠图回退，和旧渲染器的行为一致。
    Pal3GeomMaterialDef::create(texture_path.to_str().unwrap(), |name| vfs.open(name).ok())
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
        // 同一个节点的顶点变形跟着一起走（原版是同一个 `gbGeomNode` 的时间）。
        if let Some(m) = self.morph() {
            let a = m.inner::<AnimatedMeshComponent>();
            match mode {
                CvdPlayMode::Hold => a.stop(),
                CvdPlayMode::Once | CvdPlayMode::Loop => {
                    a.set_loop_playback(mode == CvdPlayMode::Loop);
                    a.set_time_scale(speed.max(0.001));
                    m.play(true);
                }
            }
        }
    }

    fn morph(&self) -> Option<ComRc<IAnimatedMeshComponent>> {
        self.entity
            .get_component(IAnimatedMeshComponent::uuid())
            .and_then(|c| c.query_interface::<IAnimatedMeshComponent>())
    }

    /// Whether a one-shot animation has run to its end. `SceneOpenDoor`
    /// blocks the script on this.
    pub fn finished(&self) -> bool {
        let morph_busy = self.morph().is_some_and(|m| {
            matches!(
                m.inner::<AnimatedMeshComponent>().morph_animation_state(),
                MorphAnimationState::Playing | MorphAnimationState::Holding
            )
        });
        matches!(*self.play_mode.borrow(), CvdPlayMode::Hold)
            || (matches!(*self.play_mode.borrow(), CvdPlayMode::Once)
                && self.elapsed.get() >= self.duration()
                && !morph_busy)
    }

    /// 从当前位置按 `speed` 播到这一轮结束就停（类型 17 等待到点时 `SetTimeScale(2.0)`，
    /// 原版是默认循环、等一轮播完的消息）。和 [`Self::play`] 不同，不从头开始。
    pub fn finish_cycle(&self, speed: f32) {
        *self.play_mode.borrow_mut() = CvdPlayMode::Once;
        self.speed.set(speed.max(0.001));
        if let Some(m) = self.morph() {
            let a = m.inner::<AnimatedMeshComponent>();
            a.set_loop_playback(false);
            a.set_time_scale(speed.max(0.001));
            m.play(false);
        }
    }

    /// 停在最后一帧（读档回来时，放下的吊桥要直接是放下的样子）。
    pub fn hold_last_frame(&self) {
        let duration = self.duration();
        *self.play_mode.borrow_mut() = CvdPlayMode::Hold;
        self.elapsed.set(duration);
        if duration > 0. {
            self.apply_at(duration);
        }
    }

    /// 回到第 0 帧停住（`[vtbl+0x18](0)` + `SetTimeScale(0)`）。
    pub fn rewind(&self) {
        *self.play_mode.borrow_mut() = CvdPlayMode::Hold;
        self.elapsed.set(0.);
        if self.duration() > 0. {
            self.apply_at(0.);
        }
        if let Some(m) = self.morph() {
            m.inner::<AnimatedMeshComponent>().rewind();
        }
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

/// 把一个物件下所有 CVD 节点按同一种方式播（关键帧和顶点变形一起）。返回有没有 CVD。
pub fn play_cvd_subtree(entity: &ComRc<IEntity>, mode: CvdPlayMode, speed: f32) -> bool {
    let mut any = false;
    if let Some(c) = entity
        .get_component(ICvdModel::uuid())
        .and_then(|c| c.query_interface::<ICvdModel>())
    {
        c.inner::<CvdModelComponent>().play(mode, speed);
        any = true;
    }
    for child in entity.children() {
        any |= play_cvd_subtree(&child, mode, speed);
    }
    any
}

/// 物件下所有带 CVD 组件的节点（包括它自己）。
pub fn cvd_components(entity: &ComRc<IEntity>, out: &mut Vec<ComRc<ICvdModel>>) {
    if let Some(c) = entity
        .get_component(ICvdModel::uuid())
        .and_then(|c| c.query_interface::<ICvdModel>())
    {
        out.push(c);
    }
    for child in entity.children() {
        cvd_components(&child, out);
    }
}

/// 类型 17 物件（p1 为 0、模型是 CVD）的待机重播，`FUN_00426eb0` 的 case 0x11：
///
/// ```text
/// +0x51 == 0: +0x44 += dt; 到了 +0x48 → +0x48 = rand%10 + 1, +0x44 = 0, +0x51 = 1, SetTimeScale(2.0)
/// +0x51 == 1: 等「播完」消息（FUN_0042b1f0）→ +0x51 = 0, [vtbl+0x18](0), SetTimeScale(0)
/// ```
///
/// 建物件时（`0x42409f`）`+0x44 = 0`、`+0x48 = rand % 10`，模型还是默认的循环、倍率 1，
/// 所以第一次到点前它照常循环，到点后按 2 倍播完手上这一轮再回第 0 帧停住；之后每次
/// 等 1–10 秒整从头按 2 倍播一遍。
#[derive(Debug, Clone, PartialEq)]
pub struct IdleReplay {
    waited: f32,
    wait: f32,
    playing: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleReplayStep {
    Nothing,
    /// 按 2 倍播到这一轮结束（[`CvdModelComponent::finish_cycle`]）。
    Play,
    /// 回第 0 帧停住（[`CvdModelComponent::rewind`]）。
    Rewind,
}

/// 等待到点后播放的倍率。
pub const IDLE_REPLAY_SPEED: f32 = 2.0;

impl IdleReplay {
    /// `first` 是建物件时抽的 `rand()`。
    pub fn new(first: i32) -> Self {
        Self {
            waited: 0.,
            wait: (first % 10) as f32,
            playing: false,
        }
    }

    /// 推一帧。`finished` 是物件的 CVD 这一轮播完没有；`rand` 只在到点时抽一次。
    pub fn step(
        &mut self,
        delta_sec: f32,
        finished: bool,
        rand: impl FnOnce() -> i32,
    ) -> IdleReplayStep {
        if !self.playing {
            self.waited += delta_sec;
            if self.wait <= self.waited {
                self.wait = (rand() % 10 + 1) as f32;
                self.waited = 0.;
                self.playing = true;
                return IdleReplayStep::Play;
            }
        } else if finished {
            self.playing = false;
            return IdleReplayStep::Rewind;
        }
        IdleReplayStep::Nothing
    }
}

/// 类型 17 的第三个参数（`+0x100`，scn 记录 +0x54）：1 上下浮动、2 绕竖轴转。
/// 浮动 `y = sin(t) × 6 + 原高度`（t 从 0 起按秒累加，`+0x54`），转是每秒 80°（`+0xe0`）。
pub const FLOAT_AMPLITUDE: f32 = 6.0;
pub const SPIN_DEGREES_PER_SEC: f32 = 80.0;

/// 场景加载时物件的 CVD 怎么播（`FUN_00423a70` `0x42405a` 起，按物件类型 `+0xf4`）。
///
/// 原版 `gbGeomNode` 载进来默认就是循环、倍率 1；建物件时再按类型改：
///
/// - 1、2、12、19、23、26、29、32、41（模型是 CVD 时）和 4、6、14：`SetTimeScale(0)`，停住，
///   等交互或命令再播；
/// - 17：参数 p1（记录 +0x50，运行时 +0xfc）非 0 时停住；为 0 时照默认循环，之后由
///   [`IdleReplay`] 接管（`+0x48 = rand % 10` 是第一次重播前等的秒数）；
/// - 4、6、12、14、16、19、23、26、32、41 再 `SetAnimLoop(1)` 改成只播一遍——其中只有 16
///   没被停住，所以调查点进场景播一遍就停；
/// - 其余类型：循环。
pub fn scene_object_initial_play(node_type: u8, p1: i32) -> Option<(CvdPlayMode, f32)> {
    match node_type {
        1 | 2 | 4 | 6 | 12 | 14 | 19 | 23 | 26 | 29 | 32 | 41 => None,
        17 if p1 != 0 => None,
        16 => Some((CvdPlayMode::Once, 1.)),
        _ => Some((CvdPlayMode::Loop, 1.)),
    }
}

/// 场景自己的 `<场景名>.cvd`（`FUN_0041fe30` 载、`FUN_004200a0` 每帧 `SetTimeScale(0.2)`）。
pub const SCENE_CVD_SPEED: f32 = 0.2;

#[cfg(test)]
mod initial_play_tests {
    use super::*;

    #[test]
    fn idle_replay_waits_plays_then_rewinds() {
        let mut r = IdleReplay::new(13); // 13 % 10 = 3 秒
        let never = || -> i32 { panic!("没到点不该抽随机数") };
        assert_eq!(r.step(1.0, true, never), IdleReplayStep::Nothing);
        assert_eq!(r.step(1.5, true, never), IdleReplayStep::Nothing);
        // 到 3 秒：抽下一次的等待（24 % 10 + 1 = 5 秒），开始播。
        assert_eq!(r.step(0.5, false, || 24), IdleReplayStep::Play);
        // 播放中不计时，只等播完。
        assert_eq!(r.step(10.0, false, never), IdleReplayStep::Nothing);
        assert_eq!(r.step(0.1, true, never), IdleReplayStep::Rewind);
        // 回到等待，5 秒后再播。
        assert_eq!(r.step(4.9, true, never), IdleReplayStep::Nothing);
        assert_eq!(r.step(0.1, true, || 9), IdleReplayStep::Play);
        assert_eq!(r.wait, 10.0, "9 % 10 + 1");
    }

    #[test]
    fn idle_replay_with_zero_wait_plays_on_the_first_frame() {
        let mut r = IdleReplay::new(20);
        assert_eq!(r.step(0.016, false, || 0), IdleReplayStep::Play);
        assert_eq!(r.wait, 1.0, "之后至少等 1 秒");
    }

    #[test]
    fn switches_and_doors_wait_everything_else_loops() {
        assert_eq!(scene_object_initial_play(14, 0), None, "门");
        assert_eq!(scene_object_initial_play(23, 0), None, "开关");
        assert_eq!(scene_object_initial_play(26, 0), None, "宝箱");
        assert_eq!(scene_object_initial_play(17, 1), None);
        assert_eq!(
            scene_object_initial_play(17, 0),
            Some((CvdPlayMode::Loop, 1.))
        );
        assert_eq!(
            scene_object_initial_play(16, 0),
            Some((CvdPlayMode::Once, 1.))
        );
        assert_eq!(
            scene_object_initial_play(25, 0),
            Some((CvdPlayMode::Loop, 1.))
        );
        assert_eq!(
            scene_object_initial_play(0, 0),
            Some((CvdPlayMode::Loop, 1.))
        );
    }
}
