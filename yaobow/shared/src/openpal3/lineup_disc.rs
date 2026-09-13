//! 阵法页的 3D 五灵盘。原版这一页不贴 2D 的盘，而是一个透视小场景：`wl.pol` 盘面、
//! 在队的角色站在各自的站位上、空着的站位上立着 `pointer.cvd`，先画 3D 再画 2D 界面
//! （虚表 0x56ba34 的 `0x4850e0`）。逆向记录在 `switch/re/notes/lineup_page_layout.md` §3。
//!
//! 和 [`super::model_view`] 一样画进离屏目标（清成透明）再贴到界面上。

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crosscom::ComRc;
use radiance::comdef::{IEntity, IEntityExt, IScene};
use radiance::math::Vec3;
use radiance::rendering::{RenderTarget, RenderingEngine};
use radiance::scene::{CoreScene, ISceneExt, SceneLighting};

use super::asset_manager::AssetManager;
use super::model_view::{TARGET, clear_retired, retire};
use super::scene::{
    CvdPlayMode, RoleAnimationRepeatMode, RoleController, create_entity_from_cvd_model,
    play_cvd_subtree,
};

pub const DISC_MODEL: &str = "/basedata/basedata/ui/GameMainUI/Lineup/wl.pol";
pub const POINTER_MODEL: &str = "/basedata/basedata/ui/GameMainUI/Lineup/pointer.cvd";

/// 盘面的位置：`FUN_0047d030(−30, 30, 0)`。
pub const DISC_AT: [f32; 3] = [-30., 30., 0.];

/// 六个站位（`FUN_00483650` 写 `this+0x26f68` 起，每组 x/y/z）。y 全取 `this+0x26fd4`、
/// 第六组取 `this+0x26fd0..d8`——那是盘面节点（`this+0x26fb0`）的位置 `+0x20`，由
/// `FUN_0047d030(−30, 30, 0)` 经 `ECX` 写进去，所以按地址扫不到。也就是说人站在盘面上，
/// 第六格就是盘心。
pub const POSITIONS: [[f32; 3]; 6] = [
    [-30., 30., -56.],
    [25., 30., -18.],
    [4., 30., 42.],
    [-65., 30., 42.],
    [-80., 30., -20.],
    [-30., 30., 0.],
];

/// 角色和站位指针的公转中心：盘心（`FUN_0047d130(盘的位置)`，写节点 `+0x2c`）。
/// 指针的中心另抬高 [`POINTER_LIFT`]，水平上一样。
pub const CENTER: [f32; 3] = DISC_AT;

/// 转盘速度：`FUN_0047d690` 每帧转 `dt × 50 × +0x88` 度，`+0x88` 是构造写的 1.8。
pub const SPIN_SPEED: f32 = 50. * 1.8;
/// 点中的人离正前方不到这么多度就不转（`0x485408`：夹角 < 1.5 或 > 358.5）。
pub const FRONT_TOLERANCE: f32 = 1.5;

/// 手柄右摇杆推满时每秒改多少度俯角。原版只有鼠标，这是移植端自己加的。
pub const PAD_PITCH_SPEED: f32 = 90.;

/// 站位指针抬高多少（构造 `FUN_00482ed0` 写 `+0x26f54 = 12.0`）。
pub const POINTER_LIFT: f32 = 12.;

/// 相机俯角：构造写 `+0x26f48 = −38.0`，拖动时夹在 [−86, −15]（0x56ba50 / 0x56ba54）。
pub const PITCH: f32 = -38.;
pub const PITCH_MIN: f32 = -86.;
pub const PITCH_MAX: f32 = -15.;

/// `FUN_0047c7e0(0, 俯角, 3.2)` 的第三个参数：滚转。
const ROLL: f32 = 3.2;
/// `FUN_0047c6e0(0, 50, 400, 1)`：看向 (0, 50, 0)，距离 400。
const LOOK_AT: [f32; 3] = [0., 50., 0.];
const DISTANCE: f32 = 400.;
/// `FUN_0047c640` 往相机 `+0` 写 50.0（非 0 即透视）。按 4:3 的水平视角理解，和战斗相机
/// 同一个读法 [推断]。
const FOV: f32 = 50.;

/// 角色节点载入时的动作：`FUN_0047cb90(模型名, 0x10)`，16 号是 `j01`。
pub const ROLE_ACTION: &str = "j01";

/// 我方五人的模型目录，下标是角色号（景天、雪见、龙葵、紫萱、长卿）。原版从角色记录里
/// 取模型名（`&DAT_00c0600e` 起跨 0x2200），龙葵换形象后会跟着变；这里先用固定的 [推断]。
pub const ROLE_MODELS: [&str; 5] = ["101", "104", "105", "107", "102"];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiscCamera {
    pub eye: [f32; 3],
    pub target: [f32; 3],
    pub up: [f32; 3],
}

/// `FUN_0047c830(1)`，偏航 0：
///
/// ```text
/// dir = (0, sin b, −cos b)          eye = 看点 − 400·dir
/// up  = (−sin c, cos c·cos b, sin b·cos c)
/// ```
pub fn camera(pitch: f32) -> DiscCamera {
    let (b, c) = (pitch.to_radians(), ROLL.to_radians());
    let dir = [0., b.sin(), -b.cos()];
    DiscCamera {
        eye: [
            LOOK_AT[0] - DISTANCE * dir[0],
            LOOK_AT[1] - DISTANCE * dir[1],
            LOOK_AT[2] - DISTANCE * dir[2],
        ],
        target: LOOK_AT,
        up: [-c.sin(), c.cos() * b.cos(), b.sin() * c.cos()],
    }
}

/// 节点绕竖轴转多少度（`FUN_0047d440`，节点 `+0x78` 置位时）。`offset` 是位置减公转中心，
/// `+0x74` 是 `normalize(offset)` 和 +z 的夹角，转 `360 − 夹角`，在中心右边（x > 0）时取负。
/// 站在中心的方向是零向量，按夹角 90° 算 [推断：`gbVec3D::Normalize` 对零向量不动]。
pub fn facing(pos: [f32; 3]) -> f32 {
    let len = (pos[0] * pos[0] + pos[1] * pos[1] + pos[2] * pos[2]).sqrt();
    let cos = if len > 1e-6 { pos[2] / len } else { 0. };
    let turn = 360. - cos.clamp(-1., 1.).acos().to_degrees();
    if 0. < pos[0] { -turn } else { turn }
}

/// 把相对盘心的水平偏移转 `spin` 度：公转角 θ（偏移 = r·(−sin θ, ·, cos θ)，`FUN_0047d320`）
/// 加上 `spin`。
pub fn spun(offset: [f32; 3], spin: f32) -> [f32; 3] {
    let (s, c) = spin.to_radians().sin_cos();
    [
        offset[0] * c - offset[2] * s,
        offset[1],
        offset[0] * s + offset[2] * c,
    ]
}

/// 偏移的公转角，(−180, 180]；0 是正前方（+z，朝镜头）。
pub fn orbit_angle(offset: [f32; 3]) -> f32 {
    (-offset[0]).atan2(offset[2]).to_degrees()
}

/// 点中一个人以后盘要转到哪（`0x485403` 起）：把他转到正前方，走近的那一边；
/// 已经差不多在正前方、或站在盘心的，不转（`None`）。
pub fn spin_target(offset: [f32; 3], spin: f32) -> Option<f32> {
    if offset[0].abs() < 1e-4 && offset[2].abs() < 1e-4 {
        return None;
    }
    let theta = orbit_angle(spun(offset, spin));
    if theta.abs() < FRONT_TOLERANCE {
        return None;
    }
    Some(spin - theta)
}

/// 点一下之后该做什么（`0x485352` 起两个循环：先看角色，再看空站位的指针）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscAction {
    None,
    /// 选中这个人（头顶出指针，盘转过来）。
    Select(u32),
    /// 再点同一个人：取消选中。
    Deselect,
    /// 选中了 a 又点 b：两人换站位（`0x485605`）。
    Swap(u32, u32),
    /// 选中了一个人又点空站位：挪过去（`0x485765`）。
    Move(u32, u8),
}

/// `selected` 是已经选中的人；`role` 是这次点中的人，`slot` 是点中的空站位（只在没点中人时看）。
pub fn resolve_click(selected: Option<u32>, role: Option<u32>, slot: Option<u8>) -> DiscAction {
    match (selected, role, slot) {
        (None, Some(r), _) => DiscAction::Select(r),
        (Some(s), Some(r), _) if s == r => DiscAction::Deselect,
        (Some(s), Some(r), _) => DiscAction::Swap(s, r),
        (Some(s), None, Some(k)) => DiscAction::Move(s, k),
        _ => DiscAction::None,
    }
}

thread_local! {
    static DEBUG_CLICK: std::cell::Cell<Option<(f32, f32)>> = const { std::cell::Cell::new(None) };
}

/// 测试用：让五灵盘下一帧当作在画布 (x, y) 上点了一下（agent 的 `lineup_click`）。
pub fn queue_debug_click(x: f32, y: f32) {
    DEBUG_CLICK.with(|c| c.set(Some((x, y))));
}

/// 取走 [`queue_debug_click`] 放进来的那一下。
pub fn take_debug_click() -> Option<(f32, f32)> {
    DEBUG_CLICK.with(|c| c.take())
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

/// 过 800×600 画布上一点的视线（`FUN_0047d9d0` → `gbCamera::GetRayToScreen`）。
pub fn ray(cam: &DiscCamera, x: f32, y: f32) -> ([f32; 3], [f32; 3]) {
    let f = normalize(sub(cam.target, cam.eye));
    let r = normalize(cross(f, cam.up));
    let u = cross(r, f);
    let th = (FOV.to_radians() / 2.).tan();
    let tv = th * 3. / 4.;
    let (nx, ny) = (2. * x / 800. - 1., 1. - 2. * y / 600.);
    let d = [0, 1, 2].map(|k| f[k] + r[k] * nx * th + u[k] * ny * tv);
    (cam.eye, normalize(d))
}

/// 射线和轴对齐包围盒相交（原版用盘的 OBB，`gbRay::IsColOBB`；盘没转，AABB 一样）。
pub fn hits_box(origin: [f32; 3], dir: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> bool {
    box_distance(origin, dir, lo, hi).is_some()
}

/// 射线进盒子时走了多远；打不中是 `None`。
pub fn box_distance(origin: [f32; 3], dir: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> Option<f32> {
    let (mut t0, mut t1) = (0f32, f32::INFINITY);
    for k in 0..3 {
        if dir[k].abs() < 1e-9 {
            if origin[k] < lo[k] || origin[k] > hi[k] {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo[k] - origin[k]) / dir[k], (hi[k] - origin[k]) / dir[k]);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

/// 实体（连子实体）渲染对象的 `local_aabb` 并起来。角色脚下的影子（`ROLE_SHADOW`，60×60 的
/// 面片）不算，不然五个人的盒子都是 ±30、互相叠着。
fn local_bounds(e: &ComRc<IEntity>) -> Option<([f32; 3], [f32; 3])> {
    fn collect(e: &ComRc<IEntity>, acc: &mut Option<([f32; 3], [f32; 3])>) {
        if e.name() == "ROLE_SHADOW" {
            return;
        }
        if let Some(rc) = e.get_rendering_component() {
            for h in rc.render_objects() {
                if let Some((a, b)) = h.as_dyn().local_aabb() {
                    let (lo, hi) = acc.get_or_insert((a, b));
                    for k in 0..3 {
                        lo[k] = lo[k].min(a[k]);
                        hi[k] = hi[k].max(b[k]);
                    }
                }
            }
        }
        for c in e.children() {
            collect(&c, acc);
        }
    }
    let mut acc = None;
    collect(e, &mut acc);
    acc
}

pub struct LineupDisc {
    scene: ComRc<IScene>,
    disc: ComRc<IEntity>,
    /// 盘面的世界包围盒（没转时的；盘是扁的，转了也差不多）。
    disc_bounds: Option<([f32; 3], [f32; 3])>,
    roles: Vec<(u32, ComRc<IEntity>)>,
    /// 各人模型的局部包围盒，点选和头顶指针的高度用。
    role_bounds: Vec<Option<([f32; 3], [f32; 3])>>,
    pointers: Vec<ComRc<IEntity>>,
    pointer_bounds: Option<([f32; 3], [f32; 3])>,
    /// 选中的人头顶那个指针（原版每个角色节点各挂一个，`+0x15` 置位才画）。
    head_pointer: Option<ComRc<IEntity>>,
    selected: Option<u32>,
    /// 整盘转了多少度、要转到多少度。
    spin: f32,
    spin_to: f32,
    target: RefCell<Option<Box<dyn RenderTarget>>>,
    engine: Rc<RefCell<dyn RenderingEngine>>,
    pitch: f32,
}

impl LineupDisc {
    /// `team` 是在队角色号（0..4）。没有渲染引擎、盘不在包里时是 `None`。
    pub fn build(am: &Rc<AssetManager>, team: &[u32]) -> Option<Self> {
        use mini_fs::StoreExt;
        clear_retired();
        let engine = am.rendering_engine()?;
        let factory = am.component_factory();
        let scene = CoreScene::create();
        // 原版这一页没见挂灯，和别的界面视图一样用满环境光 [推断]。
        scene.set_lighting(SceneLighting::new([1., 1., 1.], vec![]));

        am.vfs().open(DISC_MODEL).ok()?;
        let disc = super::loaders::pol::create_entity_from_pol_model_with_texture(
            &factory,
            am.vfs(),
            Path::new(DISC_MODEL),
            "LINEUP_DISC".into(),
            true,
            None,
        );
        disc.transform()
            .borrow_mut()
            .set_position(&Vec3::new(DISC_AT[0], DISC_AT[1], DISC_AT[2]));
        disc.load();
        let disc_bounds = local_bounds(&disc).map(|(lo, hi)| {
            (
                [0, 1, 2].map(|k| lo[k] + DISC_AT[k]),
                [0, 1, 2].map(|k| hi[k] + DISC_AT[k]),
            )
        });
        scene.add_entity(disc.clone());

        let mut roles = vec![];
        let mut role_bounds = vec![];
        for &id in team {
            let Some(model) = ROLE_MODELS.get(id as usize) else {
                continue;
            };
            let Some(e) = am.load_role(model, ROLE_ACTION, format!("LINEUP_ROLE_{id}"), true)
            else {
                continue;
            };
            if let Some(rc) = RoleController::get_role_controller(e.clone()) {
                let rc = rc.inner::<RoleController>();
                rc.set_active(true);
                rc.play_anim(ROLE_ACTION, RoleAnimationRepeatMode::Loop);
            }
            e.load();
            role_bounds.push(local_bounds(&e));
            scene.add_entity(e.clone());
            roles.push((id, e));
        }

        let mut pointers = vec![];
        let mut head_pointer = None;
        if am.vfs().open(POINTER_MODEL).is_ok() {
            let make = |name: String| {
                let p = create_entity_from_cvd_model(
                    factory.clone(),
                    am.vfs(),
                    Path::new(POINTER_MODEL),
                    name,
                    true,
                );
                p.load();
                play_cvd_subtree(&p, CvdPlayMode::Loop, 1.);
                scene.add_entity(p.clone());
                p
            };
            for k in 0..POSITIONS.len() {
                pointers.push(make(format!("LINEUP_POINTER_{k}")));
            }
            let head = make("LINEUP_HEAD_POINTER".into());
            head.set_visible(false);
            head_pointer = Some(head);
        }
        let pointer_bounds = pointers.first().and_then(local_bounds);

        let target = factory.create_render_target(TARGET.0, TARGET.1);
        let disc = Self {
            scene,
            disc,
            disc_bounds,
            roles,
            role_bounds,
            pointers,
            pointer_bounds,
            head_pointer,
            selected: None,
            spin: 0.,
            spin_to: 0.,
            target: RefCell::new(Some(target)),
            engine,
            pitch: PITCH,
        };
        disc.apply_camera();
        Some(disc)
    }

    pub fn team(&self) -> Vec<u32> {
        self.roles.iter().map(|(id, _)| *id).collect()
    }

    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// 改俯角（`0x485230`：加上鼠标纵向位移，夹在 [−86, −15]，再重算相机）。
    pub fn set_pitch(&mut self, pitch: f32) {
        self.pitch = pitch.clamp(PITCH_MIN, PITCH_MAX);
        self.apply_camera();
    }

    fn apply_camera(&self) {
        let cam = camera(self.pitch);
        let mut c = self.scene.camera_mut();
        c.set_fov43(FOV.to_radians());
        c.set_aspect(4. / 3.);
        // `look_at` 以世界 +y 为上；原版的 up 是这个再绕视线转 3.2°，偏向 −x。
        c.transform_mut()
            .set_position(&Vec3::new(cam.eye[0], cam.eye[1], cam.eye[2]))
            .look_at(&Vec3::new(cam.target[0], cam.target[1], cam.target[2]))
            .rotate_axis_angle_local(&Vec3::new(0., 0., 1.), ROLL.to_radians());
    }

    /// 画布坐标 (x, y)（800×600）上的点是不是落在盘上（`FUN_0047db40`）。
    pub fn hits_disc(&self, x: f32, y: f32) -> bool {
        let Some((lo, hi)) = self.disc_bounds else {
            return false;
        };
        let (o, d) = ray(&camera(self.pitch), x, y);
        hits_box(o, d, lo, hi)
    }

    pub fn selected(&self) -> Option<u32> {
        self.selected
    }

    /// 盘还在转。转的时候角色和指针都点不中（`FUN_0047da60` 看节点 `+0x79`）。
    pub fn spinning(&self) -> bool {
        self.spin != self.spin_to
    }

    /// 站位 `slot` 现在在哪（转过以后）。
    fn slot_at(&self, slot: usize) -> [f32; 3] {
        add(CENTER, spun(sub(POSITIONS[slot], CENTER), self.spin))
    }

    /// 在 800×600 画布上点了一下（`0x485352` 起）。先看点没点中在队的人，再看点没点中
    /// 空站位上的指针；盘在转的时候什么也点不中。站位的改动由调用方写回存档，下一帧
    /// [`Self::place`] 照新站位摆。
    pub fn click(&mut self, x: f32, y: f32) -> DiscAction {
        if self.spinning() {
            return DiscAction::None;
        }
        let (o, d) = ray(&camera(self.pitch), x, y);
        let hit = |e: &ComRc<IEntity>, b: &Option<([f32; 3], [f32; 3])>| {
            let (lo, hi) = (*b)?;
            let p = e.transform().borrow().position();
            let p = [p.x, p.y, p.z];
            box_distance(o, d, add(lo, p), add(hi, p))
        };
        // 原版按编号取第一个点中的（`0x48536c` 的循环一命中就跳出）。
        let role = self
            .roles
            .iter()
            .zip(&self.role_bounds)
            .find(|((_, e), b)| hit(e, b).is_some())
            .map(|((id, _), _)| *id);
        let slot = if role.is_none() && self.selected.is_some() {
            self.pointers
                .iter()
                .position(|p| p.visible() && hit(p, &self.pointer_bounds).is_some())
                .map(|k| k as u8)
        } else {
            None
        };
        let action = resolve_click(self.selected, role, slot);
        match action {
            DiscAction::Select(id) => {
                self.selected = Some(id);
                if let Some((_, e)) = self.roles.iter().find(|(i, _)| *i == id) {
                    let p = e.transform().borrow().position();
                    let unspun = spun(sub([p.x, p.y, p.z], CENTER), -self.spin);
                    if let Some(to) = spin_target(unspun, self.spin) {
                        self.spin_to = to;
                    }
                }
            }
            DiscAction::Deselect | DiscAction::Swap(..) | DiscAction::Move(..) => {
                self.selected = None;
            }
            DiscAction::None => {}
        }
        action
    }

    /// 转盘往目标走一帧（`FUN_0047d690`）。
    pub fn step(&mut self, delta_sec: f32) {
        let step = SPIN_SPEED * delta_sec;
        if self.spin < self.spin_to {
            self.spin = (self.spin + step).min(self.spin_to);
        } else if self.spin > self.spin_to {
            self.spin = (self.spin - step).max(self.spin_to);
        }
        if !self.spinning() {
            self.spin = self.spin.rem_euclid(360.);
            self.spin_to = self.spin;
        }
    }

    /// 按各人的站位（0..5，没排是 `None`）摆角色、藏掉有人站的站位上的指针
    /// （`FUN_00485980`），整盘按当前转角转过去。没排站位的站在盘心。
    pub fn place(&self, position_of: impl Fn(u32) -> Option<u8>) {
        // 盘面自转（模式 1，`+0x38`）；和公转同向同速，所以砖和站位一直对得上。
        self.disc
            .transform()
            .borrow_mut()
            .clear_rotation()
            .set_position(&Vec3::new(DISC_AT[0], DISC_AT[1], DISC_AT[2]))
            .rotate_axis_angle_local(&Vec3::UP, (-self.spin).to_radians());

        let mut taken = [false; 6];
        let mut head = None;
        for ((id, e), b) in self.roles.iter().zip(&self.role_bounds) {
            let slot = position_of(*id).map(usize::from);
            let pos = match slot {
                Some(p) if p < POSITIONS.len() => {
                    taken[p] = true;
                    self.slot_at(p)
                }
                _ => CENTER,
            };
            // 站盘心的另外转 90°（`FUN_00485980` 的 case 6 写节点 `+0x38 = 90`）。
            let extra = if slot == Some(5) { 90. } else { 0. };
            e.transform()
                .borrow_mut()
                .clear_rotation()
                .set_position(&Vec3::new(pos[0], pos[1], pos[2]))
                .rotate_axis_angle_local(
                    &Vec3::UP,
                    (facing(sub(pos, CENTER)) + extra).to_radians(),
                );
            if self.selected == Some(*id) {
                // 挂在角色节点上的指针画在 位置 + (0, +0x18, 0)，`+0x18 = +0x1c + 12`，
                // `+0x1c` 取自模型包围盒 [推断：模型高度]。
                let height = b.map(|(_, hi)| hi[1]).unwrap_or(0.);
                head = Some([pos[0], pos[1] + height + POINTER_LIFT, pos[2]]);
            }
        }
        for (k, p) in self.pointers.iter().enumerate() {
            let at = self.slot_at(k);
            p.transform()
                .borrow_mut()
                .clear_rotation()
                .set_position(&Vec3::new(at[0], at[1] + POINTER_LIFT, at[2]))
                .rotate_axis_angle_local(&Vec3::UP, facing(sub(at, CENTER)).to_radians());
            p.set_visible(!taken[k]);
        }
        if let Some(h) = &self.head_pointer {
            h.set_visible(head.is_some());
            if let Some([x, y, z]) = head {
                h.transform().borrow_mut().set_position(&Vec3::new(x, y, z));
            }
        }
    }

    pub fn render(&self, delta_sec: f32) {
        self.scene.update(delta_sec);
        if let Some(target) = self.target.borrow_mut().as_mut() {
            self.engine
                .borrow_mut()
                .render_scene_to_target(self.scene.clone(), target.as_mut());
        }
    }

    pub fn texture_id(&self) -> Option<u64> {
        self.target.borrow().as_ref().map(|t| t.imgui_texture_id())
    }
}

impl Drop for LineupDisc {
    fn drop(&mut self) {
        if let Some(target) = self.target.borrow_mut().take() {
            retire(self.scene.clone(), target);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_camera_sits_above_and_in_front() {
        let c = camera(PITCH);
        // 400 × (sin 38°, cos 38°) 加到看点 (0, 50, 0) 上。
        assert!(c.eye[0].abs() < 1e-4);
        assert!((c.eye[1] - 296.26).abs() < 0.05, "{:?}", c.eye);
        assert!((c.eye[2] - 315.21).abs() < 0.05, "{:?}", c.eye);
        // up 和视线垂直，是单位长，稍微偏向 −x。
        let f = normalize(sub(c.target, c.eye));
        let dot: f32 = (0..3).map(|k| f[k] * c.up[k]).sum();
        assert!(dot.abs() < 1e-5);
        let len: f32 = c.up.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((len - 1.).abs() < 1e-5);
        assert!(c.up[0] < 0. && c.up[0] > -0.06);
    }

    #[test]
    fn the_centre_slot_is_the_middle_of_the_disc() {
        // 站位 y 都是盘面的 y，第六格就是盘心（FUN_00483650）。
        assert!(POSITIONS.iter().all(|p| p[1] == DISC_AT[1]));
        assert_eq!(POSITIONS[5], DISC_AT);
    }

    #[test]
    fn spinning_brings_the_picked_one_to_the_front_the_short_way() {
        // 正前方的偏移公转角是 0。
        assert!(orbit_angle([0., 0., 10.]).abs() < 1e-4);
        // 站位 1 在盘心右后方：转过去以后正对镜头（x 0，z > 0）。
        let o = sub(POSITIONS[1], CENTER);
        let to = spin_target(o, 0.).unwrap();
        let front = spun(o, to);
        assert!(front[0].abs() < 1e-3 && front[2] > 0., "{front:?}");
        // 走近的那边：不超过半圈。
        assert!(to.abs() <= 180.);
        // 已经在正前方的、站盘心的不转。
        assert_eq!(spin_target(front, 0.), None);
        assert_eq!(spin_target([0., 0., 0.], 0.), None);
        // 转过一次以后再点，从当前转角接着算。
        let o3 = sub(POSITIONS[3], CENTER);
        let to3 = spin_target(o3, to).unwrap();
        let f3 = spun(o3, to3);
        assert!(f3[0].abs() < 1e-3 && f3[2] > 0.);
        assert!((to3 - to).abs() <= 180.);
    }

    #[test]
    fn clicks_select_swap_move_and_deselect() {
        assert_eq!(resolve_click(None, Some(2), None), DiscAction::Select(2));
        assert_eq!(resolve_click(Some(2), Some(2), None), DiscAction::Deselect);
        assert_eq!(
            resolve_click(Some(2), Some(0), None),
            DiscAction::Swap(2, 0)
        );
        assert_eq!(
            resolve_click(Some(2), None, Some(4)),
            DiscAction::Move(2, 4)
        );
        // 没选人时点空站位、什么也没点中：没事。
        assert_eq!(resolve_click(None, None, Some(4)), DiscAction::None);
        assert_eq!(resolve_click(Some(1), None, None), DiscAction::None);
    }

    #[test]
    fn characters_turn_their_back_on_the_centre() {
        // 正前方 (0, 0, z>0)：夹角 0，转 360°，等于不转。
        assert!((facing([0., 0., 42.]) - 360.).abs() < 1e-3);
        // 左后方的 0 号站位不取负，右边的 1 号取负。
        assert!(facing([-30., 0., -56.]) > 0.);
        assert!(facing([25., 0., -18.]) < 0.);
        // 左右对称的两个点转的角度正好相反。
        assert!((facing([10., 0., 10.]) + facing([-10., 0., 10.])).abs() < 1e-3);
        // 原点按 90° 算。
        assert!((facing([0., 0., 0.]) - 270.).abs() < 1e-3);
    }

    #[test]
    fn the_centre_ray_goes_through_the_look_point() {
        let c = camera(PITCH);
        let (o, d) = ray(&c, 400., 300.);
        let to = normalize(sub(c.target, o));
        assert!((0..3).all(|k| (to[k] - d[k]).abs() < 1e-5));
        // 画面中心的射线打得中看点附近的盒子，左上角打不中。
        let (lo, hi) = ([-10., 40., -10.], [10., 60., 10.]);
        assert!(hits_box(o, d, lo, hi));
        let (o2, d2) = ray(&c, 0., 0.);
        assert!(!hits_box(o2, d2, lo, hi));
    }
}
