//! 界面里的 3D 小视图。原版在页面上放一个 3D 视图控件（构造 `FUN_00439030`，虚表
//! 0x56ac20），自带一台正交相机，模型按扩展名分三种载（`FUN_00439230`：`pol` / `cvd` /
//! 其余当角色模型）。买卖页、冶炼页的物品预览，侠义榜的书本和怪物都是它。
//!
//! 移植端的界面总画在 3D 之上，所以改成一个只放这一个模型的离屏场景，每帧画进渲染
//! 目标（清成透明），再把纹理贴到界面上。逆向记录在 `switch/re/notes/dealer_layout.md`
//! §6b、`justicebook_page_layout.md` §1。

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use crosscom::ComRc;
use radiance::comdef::{IEntity, IEntityExt, IScene};
use radiance::math::{Mat44, Vec3};
use radiance::rendering::{RenderTarget, RenderingEngine};
use radiance::scene::{CoreScene, ISceneExt, SceneLighting};

use super::asset_manager::AssetManager;
use super::comdef::ICvdModel;
use super::scene::{
    CvdModelComponent, CvdPlayMode, RoleAnimationRepeatMode, RoleController,
    create_entity_from_cvd_model, play_cvd_subtree,
};

/// 渲染目标：4:3，和 800×600 的界面同比例。
pub const TARGET: (u32, u32) = (1024, 768);
/// 相机的 z（两种摆法里都写死 1600）和斜看时的高（`FUN_00439930` 的第三个参数）。
const EYE_Z: f32 = 1600.;

/// 自转的速度，度 / 秒：构造函数 `FUN_00439030` 写进 `+0xf4` 的 98（int）。
pub const SPIN_DEG_PER_SEC: f32 = 98.;

/// 转到目标角一步（`FUN_00439a20` 里 `+0xec == 2` 那一支，源码 `UI3DCtrl::Update` 的 case 2）：
/// 角度照样加 `速度 × dt`；目标本来就比当前角小的话，一越过目标就**归 0**，否则停在目标上。
/// 返回 (新角度, 转完没有)。
pub fn turn_step(angle: f32, target: f32, deg_per_sec: f32, delta_sec: f32) -> (f32, bool) {
    let a = angle + deg_per_sec * delta_sec;
    if a > target {
        (if target < angle { 0. } else { target }, true)
    } else {
        (a, false)
    }
}

/// 自转一步（`FUN_00439a20` 里 `+0xec == 1` 那一支）：`+0xf8 += +0xf4 × dt`，不小于 359 就归 0。
pub fn spin_step(angle: f32, deg_per_sec: f32, delta_sec: f32) -> f32 {
    let a = angle + deg_per_sec * delta_sec;
    if a < 359. { a } else { 0. }
}

/// 视图里放什么。
#[derive(Debug, Clone, PartialEq)]
pub enum ModelSource {
    /// `.pol`，可以把每个材质的第一张贴图换掉（物品按图标换颜色）。
    Pol {
        path: PathBuf,
        texture: Option<String>,
    },
    Cvd(PathBuf),
    /// 角色模型（`basedata\ROLE\<名>`），循环播 `action`。
    Role {
        name: String,
        action: String,
    },
}

/// 视图控件的相机：`FUN_0043a070(size)` 设正交半宽，再二选一摆位置：
///
/// - 斜看（`FUN_00439930(x, y, h)`，买卖页 / 冶炼页）：
///   `dx = −(2x/800 − 1)·s`，`dy = (1 − 2y/600)·s`，眼 `(dx, h − dy, 1600)` 看 `(dx, −dy, 0)`；
/// - 正看（`FUN_00439850(x, y)`，侠义榜）：
///   `dx` 同上，`dy = −(1 − 2y/600)·s`，眼 `(dx, dy, 1600)` 看 `(dx, dy, 0)`。
///
/// 纵向也乘的是半宽，不是半高——照原样。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewCamera {
    pub size: f32,
    pub x: i32,
    pub y: i32,
    /// `Some(h)` 斜看（h 是 `FUN_00439930` 的第三个参数，600）；`None` 正看。
    pub tilt: Option<f32>,
}

impl ViewCamera {
    pub fn eye_target(&self) -> ([f32; 3], [f32; 3]) {
        let dx = -((2. * self.x as f32 / 800. - 1.) * self.size);
        let fy = (1. - 2. * self.y as f32 / 600.) * self.size;
        match self.tilt {
            Some(h) => ([dx, h - fy, EYE_Z], [dx, -fy, 0.]),
            None => ([dx, -fy, EYE_Z], [dx, -fy, 0.]),
        }
    }

    /// 原版是正交相机（`gbCamera` `+0` 视角 0、`+4` 正交半宽）。移植端的相机只有透视，
    /// 用很窄的视角从同一个位置看：注视点处的半宽正好是 `size` [近似]。
    pub fn fov43(&self) -> f32 {
        let (e, t) = self.eye_target();
        let d = ((e[0] - t[0]).powi(2) + (e[1] - t[1]).powi(2) + (e[2] - t[2]).powi(2)).sqrt();
        2. * (self.size / d).atan()
    }
}

/// GL 的离屏纹理是自下而上的，Switch 上 v 要翻过来 [推断：还没在机器上看过]。
pub fn v_range(v0: f32, v1: f32) -> (f32, f32) {
    if cfg!(switchgl) {
        (1. - v0, 1. - v1)
    } else {
        (v0, v1)
    }
}

// 退下来的场景和渲染目标，等下一次建视图时再真正释放。Vulkan 的 `render_scene_to_target`
// 当场提交，把目标自己的「画完」信号量交给引擎，本帧主画面提交时等它；同一帧里析构
// 目标会让主画面去等一个已销毁的信号量，进程直接崩（冶炼页出完货按离开时复现过）。
thread_local! {
    static RETIRED: RefCell<Vec<(ComRc<IScene>, Box<dyn RenderTarget>)>> =
        const { RefCell::new(Vec::new()) };
}

/// 把场景和渲染目标挂到退役表上，等下一次建视图时再释放（原因见上）。
pub(crate) fn retire(scene: ComRc<IScene>, target: Box<dyn RenderTarget>) {
    RETIRED.with(|r| r.borrow_mut().push((scene, target)));
}

/// 下一次建视图时先把退役表清掉——那时至少隔了一帧主画面。
pub(crate) fn clear_retired() {
    RETIRED.with(|r| r.borrow_mut().clear());
}

pub struct ModelView {
    scene: ComRc<IScene>,
    entity: ComRc<IEntity>,
    target: RefCell<Option<Box<dyn RenderTarget>>>,
    engine: Rc<RefCell<dyn RenderingEngine>>,
    /// 绕 Y 轴的角度（度，`+0xf8`）。`yaw` 和自转写的是同一个字段。
    angle: Cell<f32>,
    /// 自转速度（度 / 秒），0 不转。原版 `FUN_0043a090(1, 0)` 打开（`+0xec = 1`）。
    spin: Cell<f32>,
    /// 转之前的变换。
    base: Mat44,
    /// 建的时候的相机（平移就按它的半宽算定了，`SetOriginPt`）。
    camera: ViewCamera,
    /// 模式 2 的目标角（`+0xfc`）；`None` 不在转。
    turn_target: Cell<Option<f32>>,
    /// 上一次模式 2 转完了（`+0xf0`，`IsRotateTo`）。
    turned: Cell<bool>,
}

impl ModelView {
    /// 建不起来（没有渲染引擎、模型不在包里）时是 `None`。`yaw` 是度（`FUN_0043a0e0`
    /// 写 `+0xf8`，画的时候绕 Y 轴转）。
    pub fn build(
        am: &Rc<AssetManager>,
        source: &ModelSource,
        camera: ViewCamera,
        yaw: f32,
    ) -> Option<Self> {
        use mini_fs::StoreExt;
        RETIRED.with(|r| r.borrow_mut().clear());
        let engine = am.rendering_engine()?;
        let factory = am.component_factory();
        let entity = match source {
            ModelSource::Pol { path, texture } => {
                am.vfs().open(path).ok()?;
                super::loaders::pol::create_entity_from_pol_model_with_texture(
                    &factory,
                    am.vfs(),
                    path,
                    "MODEL_VIEW".into(),
                    true,
                    texture.as_deref(),
                )
            }
            ModelSource::Cvd(path) => {
                am.vfs().open(path).ok()?;
                create_entity_from_cvd_model(
                    factory.clone(),
                    am.vfs(),
                    path,
                    "MODEL_VIEW".into(),
                    true,
                )
            }
            ModelSource::Role { name, action } => {
                let e = am.load_role(name, action, "MODEL_VIEW".into(), true)?;
                if let Some(rc) = RoleController::get_role_controller(e.clone()) {
                    let rc = rc.inner::<RoleController>();
                    rc.set_active(true);
                    rc.play_anim(action, RoleAnimationRepeatMode::Loop);
                }
                e
            }
        };
        let base = *entity.transform().borrow().matrix();
        entity
            .transform()
            .borrow_mut()
            .rotate_axis_angle_local(&Vec3::UP, yaw.to_radians());

        let scene = CoreScene::create();
        // 原版的 3D 视图（`ui/ui3dctrl.cpp:147`）把全局那盏灯 `gs_LightNode`
        // 按 `q_Y(90) ⊗ q_X(45)` 摆好再 `Attach()`。那盏灯在
        // `pal3.cpp:413` 建的时候是：方向光、`m_Ambient = (1, 1, 1)`、
        // `m_Color = (0.28, 0.28, 0.28)`。
        //
        // 也就是说环境光本来就是满白，方向光只额外加 0.28 的灰。这里和鉴宝
        // 一样只给满环境光，差的是那 0.28 的方向项。
        scene.set_lighting(SceneLighting::new([1., 1., 1.], vec![]));
        entity.load();
        scene.add_entity(entity.clone());

        let (eye, at) = camera.eye_target();
        {
            let mut cam = scene.camera_mut();
            cam.set_fov43(camera.fov43());
            cam.set_aspect(4. / 3.);
            cam.transform_mut()
                .set_position(&Vec3::new(eye[0], eye[1], eye[2]))
                .look_at(&Vec3::new(at[0], at[1], at[2]));
        }
        let target = factory.create_render_target(TARGET.0, TARGET.1);
        Some(Self {
            scene,
            entity,
            target: RefCell::new(Some(target)),
            engine,
            angle: Cell::new(yaw),
            spin: Cell::new(0.),
            base,
            camera,
            turn_target: Cell::new(None),
            turned: Cell::new(false),
        })
    }

    /// CVD 模型怎么播（`SetTimeScale` / `SetAnimLoop`）；别的模型不管。
    pub fn play(&self, mode: CvdPlayMode, speed: f32) {
        play_cvd_subtree(&self.entity, mode, speed);
    }

    /// 只播一遍的 CVD 播完了没有（没有 CVD 算播完）。
    pub fn finished(&self) -> bool {
        fn walk(e: &ComRc<IEntity>) -> bool {
            let own = e
                .get_component(ICvdModel::uuid())
                .and_then(|c| c.query_interface::<ICvdModel>())
                .map_or(true, |c| c.inner::<CvdModelComponent>().finished());
            own && e.children().iter().all(walk)
        }
        walk(&self.entity)
    }

    /// 打开 / 关掉自转（`deg_per_sec` 为 0 关）。
    pub fn set_spin(&self, deg_per_sec: f32) {
        self.spin.set(deg_per_sec);
    }

    pub fn spin(&self) -> f32 {
        self.spin.get()
    }

    pub fn angle(&self) -> f32 {
        self.angle.get()
    }

    /// 改角度。原版换模型（`FUN_00439230`）不动 `+0xf8`，换了物品接着上一个的角度转。
    pub fn set_angle(&self, degrees: f32) {
        self.angle.set(degrees);
        self.entity
            .transform()
            .borrow_mut()
            .set_matrix(self.base)
            .rotate_axis_angle_local(&Vec3::UP, degrees.to_radians());
    }

    /// 只改正交半宽（`UI3DCtrl::SetOrthosize`）：相机的平移是建的时候按当时的半宽定的，不跟着变——
    /// 御剑页的剑从 1000 缩到 100，就是这样从画面中间附近一边变大一边走到位的。
    pub fn set_ortho(&self, size: f32) {
        let (e, t) = self.camera.eye_target();
        let d = ((e[0] - t[0]).powi(2) + (e[1] - t[1]).powi(2) + (e[2] - t[2]).powi(2)).sqrt();
        self.scene.camera_mut().set_fov43(2. * (size / d).atan());
    }

    /// 模式 2（`SetRotateMode(2, deg)`）：从当前角再转 `deg` 度，转的速度 `deg_per_sec`。
    /// 原版还有一步「与 360 对齐」，但那是浮点 `x / 360 × 360`，等于没做。
    pub fn turn_by(&self, deg: f32, deg_per_sec: f32) {
        self.spin.set(deg_per_sec);
        self.turn_target.set(Some(self.angle.get() + deg));
        self.turned.set(false);
    }

    /// 上一次 [`Self::turn_by`] 转完了没有。
    pub fn turned(&self) -> bool {
        self.turned.get()
    }

    pub fn render(&self, delta_sec: f32) {
        let spin = self.spin.get();
        if let Some(target) = self.turn_target.get() {
            let (a, done) = turn_step(self.angle.get(), target, spin, delta_sec);
            self.set_angle(a);
            if done {
                self.turn_target.set(None);
                self.turned.set(true);
                self.spin.set(0.);
            }
        } else if spin != 0. {
            self.set_angle(spin_step(self.angle.get(), spin, delta_sec));
        }
        self.scene.update(delta_sec);
        if let Some(target) = self.target.borrow_mut().as_mut() {
            self.engine
                .borrow_mut()
                .render_scene_to_target(self.scene.clone(), target.as_mut());
        }
    }

    /// 渲染目标的 imgui 纹理 id。
    pub fn texture_id(&self) -> Option<u64> {
        self.target.borrow().as_ref().map(|t| t.imgui_texture_id())
    }
}

impl Drop for ModelView {
    fn drop(&mut self) {
        if let Some(target) = self.target.get_mut().take() {
            let scene = self.scene.clone();
            RETIRED.with(|r| r.borrow_mut().push((scene, target)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_stops_on_the_target_or_snaps_to_zero() {
        // 0 → 180，每秒 98：一秒后 98，再过一秒越过 180，停在 180。
        assert_eq!(turn_step(0., 180., 98., 1.), (98., false));
        assert_eq!(turn_step(98., 180., 98., 1.), (180., true));
        // 目标比当前小（原版那一支）：一加就越过，角度归 0。
        assert_eq!(turn_step(200., 180., 98., 0.1), (0., true));
    }

    #[test]
    fn the_spin_wraps_to_zero_at_359() {
        // 每秒 98°：一秒后 98，3.6 秒后 352.8，再走一帧过 359 就回 0（不是减 360）。
        assert!((spin_step(0., SPIN_DEG_PER_SEC, 1.) - 98.).abs() < 1e-4);
        assert!((spin_step(350., SPIN_DEG_PER_SEC, 0.05) - 354.9).abs() < 1e-3);
        assert_eq!(spin_step(355., SPIN_DEG_PER_SEC, 0.05), 0.);
    }

    #[test]
    fn the_two_camera_placements() {
        // 斜看：买卖页一般物品 (180, 330, 268)。
        let c = ViewCamera {
            size: 180.,
            x: 330,
            y: 268,
            tilt: Some(600.),
        };
        let (eye, target) = c.eye_target();
        // 横向偏 (400 − 330) / 400 × 180 = 31.5。
        assert!((eye[0] - 31.5).abs() < 1e-4 && (target[0] - 31.5).abs() < 1e-4);
        assert!((eye[1] - target[1] - 600.).abs() < 1e-3);
        assert!(((c.fov43() / 2.).tan() * 1708.8 - 180.).abs() < 0.5);
        // 正看：侠义榜的书 (126, 566, 298)，眼和注视点只差 z。
        let c = ViewCamera {
            size: 126.,
            x: 566,
            y: 298,
            tilt: None,
        };
        let (eye, target) = c.eye_target();
        assert_eq!((eye[0], eye[1]), (target[0], target[1]));
        assert!((eye[2] - 1600.).abs() < 1e-4 && target[2] == 0.);
        assert!(((c.fov43() / 2.).tan() * 1600. - 126.).abs() < 0.01);
    }
}
