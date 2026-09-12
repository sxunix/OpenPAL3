//! 场景 CVD 的播放：拿实机资源、不渲染，确认「不叫就不动、叫了会动」。
//!
//! 规则见 `switch/re/notes/cvd_playback.md`：物件的 CVD 进场景按类型定播不播
//! （`scene_object_initial_play`），场景自己的 `<场景名>.cvd` 按 0.2 倍率循环。
//!
//! ```sh
//! YAOBOW_PAL3_ASSETS=/path/to/PAL3 cargo test -p shared --test pal3_cvd_playback
//! ```

use std::cell::RefMut;
use std::path::PathBuf;
use std::rc::Rc;

use crosscom::ComRc;
use imgui::TextureId;
use radiance::comdef::{IAnimatedMeshComponent, IEntity, IEntityExt};
use radiance::components::mesh::{AnimatedMeshComponent, MorphAnimationState};
use radiance::rendering::{
    ComponentFactory, MaterialDef, RenderObject, RenderObjectHandle, RenderingComponent, Texture,
    TextureDef, VertexBuffer, VideoPlayer,
};
use shared::openpal3::scene::{
    CvdPlayMode, SCENE_CVD_SPEED, create_entity_from_cvd_model, play_cvd_subtree,
    scene_object_initial_play,
};

struct DummyTexture;
impl Texture for DummyTexture {
    fn width(&self) -> u32 {
        1
    }
    fn height(&self) -> u32 {
        1
    }
}

struct DummyObject;
impl RenderObject for DummyObject {
    fn update_vertices(&self, _: &dyn Fn(RefMut<VertexBuffer>)) {}
}

/// 什么都不画的工厂：只要组件能建起来、时间能推。
struct NullFactory;
impl ComponentFactory for NullFactory {
    fn create_texture(&self, _: &TextureDef) -> Box<dyn Texture> {
        Box::new(DummyTexture)
    }
    fn create_imgui_texture(
        &self,
        _: &[u8],
        _: u32,
        _: u32,
        _: u32,
        _: Option<TextureId>,
    ) -> (Box<dyn Texture>, TextureId) {
        (Box::new(DummyTexture), TextureId::new(1))
    }
    fn remove_imgui_texture(&self, _: Option<TextureId>) {}
    fn create_render_object(
        &self,
        _: VertexBuffer,
        _: Vec<u32>,
        _: &MaterialDef,
        _: bool,
    ) -> RenderObjectHandle {
        RenderObjectHandle::from_dyn(Rc::new(DummyObject))
    }
    fn create_rendering_component(&self, _: Vec<RenderObjectHandle>) -> RenderingComponent {
        RenderingComponent::new()
    }
    fn create_video_player(&self) -> Box<VideoPlayer> {
        Box::new(VideoPlayer::new())
    }
    fn create_render_target(&self, _: u32, _: u32) -> Box<dyn radiance::rendering::RenderTarget> {
        unimplemented!("预览不在这个测试里")
    }
}

/// 整棵树每个节点的局部变换，加上顶点变形在不在播。
fn snapshot(e: &ComRc<IEntity>, out: &mut Vec<[[f32; 4]; 4]>, playing: &mut usize) {
    out.push(*e.transform().borrow().matrix().floats());
    if let Some(m) = e
        .get_component(IAnimatedMeshComponent::uuid())
        .and_then(|c| c.query_interface::<IAnimatedMeshComponent>())
    {
        if matches!(
            m.inner::<AnimatedMeshComponent>().morph_animation_state(),
            MorphAnimationState::Playing | MorphAnimationState::Holding
        ) {
            *playing += 1;
        }
    }
    for c in e.children() {
        snapshot(&c, out, playing);
    }
}

fn run(e: &ComRc<IEntity>, seconds: f32) {
    let dt = 1. / 30.;
    let mut t = 0.;
    while t < seconds {
        e.update(dt);
        t += dt;
    }
}

#[test]
fn scene_cvd_holds_until_told_then_moves() {
    let Ok(assets) = std::env::var("YAOBOW_PAL3_ASSETS") else {
        eprintln!("skipping: set YAOBOW_PAL3_ASSETS");
        return;
    };
    let vfs = packfs::init_virtual_fs(&assets, None);
    let factory: Rc<dyn ComponentFactory> = Rc::new(NullFactory);

    for path in ["/scene/M01/1/1.cvd", "/scene/Q10/Q10/Q10.cvd"] {
        let e = create_entity_from_cvd_model(
            factory.clone(),
            &vfs,
            PathBuf::from(path),
            "SCENE_CVD".into(),
            true,
        );
        e.load();

        let (mut before, mut p0) = (vec![], 0);
        snapshot(&e, &mut before, &mut p0);
        run(&e, 3.);
        let (mut idle, mut p1) = (vec![], 0);
        snapshot(&e, &mut idle, &mut p1);
        assert_eq!(p1, 0, "{path}: 没叫播，顶点帧不该在走");
        assert_eq!(before, idle, "{path}: 没叫播，节点不该动");

        assert!(play_cvd_subtree(&e, CvdPlayMode::Loop, SCENE_CVD_SPEED));
        run(&e, 3.);
        let (mut after, mut p2) = (vec![], 0);
        snapshot(&e, &mut after, &mut p2);
        let moved = before.iter().zip(&after).filter(|(a, b)| a != b).count();
        println!(
            "{path}: {} 个节点，{moved} 个动了，{p2} 个顶点帧在播",
            before.len()
        );
        assert!(moved > 0 || p2 > 0, "{path}: 叫了循环却什么都没动");
    }

    // 门、开关、宝箱进场景停着；普通物品循环。
    assert_eq!(scene_object_initial_play(14, 0), None);
    assert_eq!(
        scene_object_initial_play(25, 0),
        Some((CvdPlayMode::Loop, 1.))
    );
}
