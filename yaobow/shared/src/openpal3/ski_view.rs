//! 滑雪的 3D 画面（`Skee_mgr::Skee_Render`）：景天踩着剑 J13 当雪板往下滑，雪道 `m22-6.pol`、六个
//! 障碍、三件古董，过半程后后面追来一个雪球；镜头跟在人身后。规则（速度、跳、撞、捡、追、计时）在
//! [`crate::openpal3::states::ski`]，这里只照规则层每帧的状态摆东西。源码 `subgame/Skee/*.cpp`，
//! exe 里对得上的地址见 `switch/re/notes/ski.md`。
//!
//! 坐标都照原版 `gbMatrixStack` 的顺序乘：行向量 `v · Rz · Ry · T`，换到这边的列向量就是
//! `T · Ry · Rz · v`。`Rotate(角, 轴)` 的角是**度**（`gbQuaternion::SetAxisAngle` 乘 π/180）。
//! 移植端读 `.mv3` 时把 x、z 取反了（绕 Y 转半圈），所以人物矩阵末尾再乘一个 `diag(−1, 1, −1)`；
//! `.pol` 是原样读的，雪板、雪道、障碍、雪球不用乘。

use std::collections::HashMap;
use std::rc::Rc;

use common::store_ext::StoreExt2;
use crosscom::ComRc;
use mini_fs::StoreExt;
use radiance::comdef::{IEntity, IEntityExt, IScene, ISceneExt};
use radiance::math::{Mat44, Vec3};
use radiance::scene::{CoreScene, SceneLighting};

use crate::openpal3::asset_manager::AssetManager;
use crate::openpal3::scene::{RoleAnimationRepeatMode, RoleController, RoleState};
use crate::openpal3::states::ski::{Phase, SkiRun};

const SCENE_POL: &str = "/basedata/basedata/SubGame/Skee/SkeeScene/Scene/m22-6.pol";
/// 当雪板的剑（`FUN_005306a0`，`SubGame\Skee\Weapon\J13.pol`）。
const BOARD_POL: &str = "/basedata/basedata/SubGame/Skee/Weapon/J13.pol";
const BALL_POL: &str = "/basedata/basedata/SubGame/Skee/SkeeScene/Ball/Ball.pol";
/// 障碍 `"%s%s%d.pol"`（`FUN_0052f800`），0..5。
const OBSTACLE_DIR: &str = "/basedata/basedata/SubGame/Skee/SkeeScene/Obstruct";

/// 滑的人：`FUN_00530810` 里 `FUN_005306a0("101")`。
pub const MAN: &str = "101";
/// 动作（`Skee_std.h`）：准备、滑、滑（另一种）、颠、跳、摔。
const ACT_READY: &str = "h01";
const ACT_DOING1: &str = "h02";
const ACT_DOING2: &str = "h03";
const ACT_JOLT: &str = "h04";
const ACT_JUMP: &str = "h05";
const ACT_TUMBLE: &str = "h06";
/// 停下来之后（`UpdateFinish`：`GotoAction(ACT_STAND)`）。
const ACT_STAND: &str = "c01";
const ACTIONS: [&str; 7] = [
    ACT_READY, ACT_DOING1, ACT_DOING2, ACT_JOLT, ACT_JUMP, ACT_TUMBLE, ACT_STAND,
];

/// 雪板挂在人物的这个挂点上（`GetTagNodeTM(&tm, "tag_weapon1")`）。
const BOARD_TAG: &str = "tag_weapon1";

/// 镜头（`Skee_slider::ResetSlider`）：离看点 780、偏航 90、俯仰 −56、视角 26；看点是人头上 200。
const CAM_DIST: f32 = 780.;
const CAM_YAW: f32 = 90.;
const CAM_PITCH: f32 = -56.;
pub const CAM_FOV_DEG: f32 = 26.;
const CAM_LIFT: f32 = 200.;
/// 雪球出现时镜头往旁边偏 80 度（`SetCameraMotionTime(80, 0, 0, 0, 2600)`）：每次更新挪 1 度，到了停
/// 2.6 秒，再一度一度挪回来。
const CAM_SWING_DEG: f32 = 80.;
const CAM_SWING_HOLD: f32 = 2.6;

/// 原版每帧的量都是「每 1/46.68 秒」（`Skee_mgr::Skee_Update` 的 `time_scale`）；镜头摆、古董转、
/// 雪球滚那几处按次加，这里按这个帧率折成每秒 [推断：原版按每次更新算，帧率没锁]。
const FRAME_RATE: f32 = 46.68;
/// 古董每帧转 6 度（`Skee_obstruct::Update`）。
const TREASURE_SPIN_DEG: f32 = 6.;
/// 雪球每帧绕 X 滚 16 度。
const BALL_ROLL_DEG: f32 = 16.;

/// `.mv3` 挂点一帧（`gbMVTagFrame`）：时间、位置、四元数（x, y, z, w），后面 3×3 缩放不用。
#[derive(Debug, Clone, Copy)]
struct TagFrame {
    tick: u32,
    pos: [f32; 3],
    rot: [f32; 4],
}

/// 一个动作文件里的挂点轨迹。
fn read_tag(asset_mgr: &AssetManager, action: &str, tag: &str) -> Option<Vec<TagFrame>> {
    let path = asset_mgr
        .basedata_path()
        .join("ROLE")
        .join(MAN)
        .join(format!("{action}.mv3"));
    let raw = asset_mgr.vfs().read_to_end(&path).ok()?;
    let mv3 = fileformats::mv3::read_mv3(&mut std::io::Cursor::new(raw)).ok()?;
    let node = mv3.unknown_data.iter().find(|t| {
        let end = t
            .unknown0
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(t.unknown0.len());
        t.unknown0[..end].eq_ignore_ascii_case(tag.as_bytes())
    })?;
    let frames: Vec<TagFrame> = node
        .unknown2
        .iter()
        .map(|f| TagFrame {
            tick: f[0].to_bits(),
            pos: [f[1], f[2], f[3]],
            rot: [f[4], f[5], f[6], f[7]],
        })
        .collect();
    (!frames.is_empty()).then_some(frames)
}

/// `gbMVTagNode::ComputeToWorldTM`：时间对最后一帧取余，前后两帧之间位置线性插、四元数 slerp，然后
/// **w 取反**再出矩阵，位置加在平移上。
fn tag_matrix(frames: &[TagFrame], tick: u32) -> Mat44 {
    let (a, b, t) = match frames.len() {
        0 => return Mat44::new_identity(),
        1 => (frames[0], frames[0], 0.),
        _ => {
            let last = frames[frames.len() - 1].tick.max(1);
            let tick = tick % last;
            let i = (0..frames.len() - 1)
                .find(|&i| tick < frames[i + 1].tick)
                .unwrap_or(frames.len() - 2);
            let (a, b) = (frames[i], frames[i + 1]);
            let span = b.tick.saturating_sub(a.tick).max(1) as f64;
            (a, b, (tick.saturating_sub(a.tick) as f64 / span) as f32)
        }
    };
    let pos = [0, 1, 2].map(|k| a.pos[k] * (1. - t) + b.pos[k] * t);
    let mut q = slerp(a.rot, b.rot, t);
    q[3] = -q[3];
    let mut m = quat_matrix(q);
    let f = m.floats_mut();
    for k in 0..3 {
        f[k][3] = pos[k];
    }
    m
}

/// `gbQuaternion::Slerp`：点积为负时把第二个取反（走近的那条弧）。
fn slerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let mut cos = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let b = if cos < 0. {
        cos = -cos;
        b.map(|v| -v)
    } else {
        b
    };
    let (s0, s1) = if 1. - cos > 1e-6 {
        let omega = cos.min(1.).acos();
        let sin = omega.sin();
        (((1. - t) * omega).sin() / sin, (t * omega).sin() / sin)
    } else {
        (1. - t, t)
    };
    [0, 1, 2, 3].map(|k| a[k] * s0 + b[k] * s1)
}

/// 四元数（x, y, z, w）的旋转矩阵，列向量约定。`gbQuaternion::GetMatrix` 写的是行向量版
/// （第一行 = x 轴转过去的样子），转置过来就是这个。
fn quat_matrix([x, y, z, w]: [f32; 4]) -> Mat44 {
    let mut m = Mat44::new_identity();
    let f = m.floats_mut();
    f[0][0] = 1. - 2. * (y * y + z * z);
    f[1][0] = 2. * (x * y + z * w);
    f[2][0] = 2. * (x * z - y * w);
    f[0][1] = 2. * (x * y - z * w);
    f[1][1] = 1. - 2. * (x * x + z * z);
    f[2][1] = 2. * (y * z + x * w);
    f[0][2] = 2. * (x * z + y * w);
    f[1][2] = 2. * (y * z - x * w);
    f[2][2] = 1. - 2. * (x * x + y * y);
    m
}

/// `gbMatrixStack::Rotate(角, 轴)`：绕轴转「角」度。
fn rotate(axis: [f32; 3], deg: f32) -> Mat44 {
    let half = deg.to_radians() * 0.5;
    let s = half.sin();
    quat_matrix([axis[0] * s, axis[1] * s, axis[2] * s, half.cos()])
}

fn translate(p: &Vec3) -> Mat44 {
    let mut m = Mat44::new_identity();
    let f = m.floats_mut();
    f[0][3] = p.x;
    f[1][3] = p.y;
    f[2][3] = p.z;
    m
}

fn mul(a: &Mat44, b: &Mat44) -> Mat44 {
    Mat44::multiplied(a, b)
}

/// 移植端 `.mv3` 的局部坐标 = 原版绕 Y 转半圈。
fn mv3_flip() -> Mat44 {
    let mut m = Mat44::new_identity();
    let f = m.floats_mut();
    f[0][0] = -1.;
    f[2][2] = -1.;
    m
}

/// `Skee_slider::Update` 里的两个角（度）：行进角是方向和 +Z 的夹角（x < 0 时取 360 − 它），平衡角是
/// 方向和 +Y 的夹角。
fn step_and_balance(dir: &Vec3) -> (f32, f32) {
    let n = dir.norm();
    if n <= 0. {
        return (0., 90.);
    }
    let d = Vec3::scalar_mul(1. / n, dir);
    let mut step = d.z.clamp(-1., 1.).acos();
    if d.x < 0. {
        step = std::f32::consts::TAU - step;
    }
    (step.to_degrees(), d.y.clamp(-1., 1.).acos().to_degrees())
}

/// 人物现在该播哪个动作（`Skee_slider::Update` 里那几处 `GotoAction`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pose {
    /// 正常滑：h02 循环，隔一阵随机换成 h03 / h04 播一遍。
    Sliding,
    /// 一遍就回到滑的动作（h01 起身、h03 / h04、跳完落地的 h04）。
    Once(&'static str),
    Jumping,
    Tumbling,
    Standing,
}

pub struct SkiView {
    pub scene: ComRc<IScene>,
    man: ComRc<IEntity>,
    board: Option<ComRc<IEntity>>,
    ball: Option<ComRc<IEntity>>,
    treasures: Vec<Option<ComRc<IEntity>>>,
    tags: HashMap<&'static str, Vec<TagFrame>>,
    pose: Pose,
    action: &'static str,
    phase: Phase,
    /// 上一帧的位置，算方向用（原版 `m_sliderPrevPos`）。
    prev_pos: Vec3,
    step: f32,
    balance: f32,
    /// 正常滑时隔多久换一次动作（`m_actiontic`，毫秒）和已经过了多久。
    action_tic: f32,
    action_clock: f32,
    rand: crate::openpal3::states::shop::ShopRng,
    /// 镜头偏的角度、到头之后停了多久、在不在往回挪。
    swing: f32,
    swing_hold: f32,
    swing_back: bool,
    swing_on: bool,
    chasing: bool,
    treasure_spin: f32,
    ball_roll: f32,
    ball_prev: Option<Vec3>,
    ball_step: f32,
}

impl SkiView {
    /// `treasure_models`：三件古董各自的物品模型名（物品记录 +0x24），和 `run.treasures()` 一一对应。
    pub fn new(
        asset_mgr: &Rc<AssetManager>,
        run: &SkiRun,
        treasure_models: &[Option<String>],
        seed: u32,
    ) -> Option<Self> {
        let factory = asset_mgr.component_factory();
        let vfs = asset_mgr.vfs();
        let scene = CoreScene::create();
        scene.set_lighting(SceneLighting::new([1., 1., 1.], vec![]));

        let pol = |path: &str, name: &str| -> Option<ComRc<IEntity>> {
            if vfs.open(path).is_err() {
                log::warn!("Ski: 没有 {path}");
                return None;
            }
            let e = crate::openpal3::loaders::pol::create_entity_from_pol_model(
                &factory,
                vfs,
                path,
                name.to_string(),
                true,
            );
            scene.add_entity(e.clone());
            Some(e)
        };

        pol(SCENE_POL, "SKEE_SCENE")?;
        let man = asset_mgr.load_role(MAN, ACT_DOING1, "SKEE_MAN".to_string(), true)?;
        if let Some(rc) = RoleController::get_role_controller(man.clone()) {
            let rc = rc.inner::<RoleController>();
            rc.set_active(true);
            // 播一遍的动作播完停在那，由这里换下一个（原版 `IsOver` 后才 `GotoAction`）。
            rc.set_auto_play_idle(false);
            rc.play_anim(ACT_DOING1, RoleAnimationRepeatMode::Loop);
        }
        scene.add_entity(man.clone());
        let board = pol(BOARD_POL, "SKEE_BOARD");
        let ball = pol(BALL_POL, "SKEE_BALL");
        if let Some(b) = &ball {
            b.set_visible(false);
        }
        // 障碍不动，摆好交给场景就行。
        for (i, o) in run.obstacles().iter().enumerate() {
            let Some(e) = pol(
                &format!("{OBSTACLE_DIR}/obstruct{i}.pol"),
                &format!("SKEE_OBSTRUCT_{i}"),
            ) else {
                continue;
            };
            // `Render`：`Translate(m_nodePos[i])`，不转。
            e.transform()
                .borrow_mut()
                .set_matrix(translate(&Vec3::new(o.x, o.y, o.z)));
        }
        // 古董用物品自己的模型：`FUN_0052f8e0` 拼 `Item\%s\%s.pol`（`%s` 是物品记录 +0x24 的模型名）。
        let treasures = run
            .treasures()
            .iter()
            .zip(treasure_models)
            .map(|(t, model)| {
                let model = model.as_ref()?;
                let e = pol(
                    &format!("/basedata/basedata/item/{model}/{model}.pol"),
                    &format!("SKEE_TREASURE_{}", t.id),
                )?;
                e.set_visible(t.present);
                Some(e)
            })
            .collect();
        let tags = ACTIONS
            .iter()
            .filter_map(|&a| Some((a, read_tag(asset_mgr, a, BOARD_TAG)?)))
            .collect();

        let (x, y, z) = run.pos;
        let mut rand = crate::openpal3::states::shop::ShopRng::new(seed);
        let action_tic = (rand.rand15() % 2600) as f32;
        let mut view = Self {
            scene,
            man,
            board,
            ball,
            treasures,
            tags,
            pose: Pose::Sliding,
            action: ACT_DOING1,
            phase: run.phase(),
            prev_pos: Vec3::new(x, y, z),
            step: 0.,
            balance: 90.,
            action_tic,
            action_clock: 0.,
            rand,
            swing: 0.,
            swing_hold: 0.,
            swing_back: false,
            swing_on: false,
            chasing: false,
            treasure_spin: 0.,
            ball_roll: 0.,
            ball_prev: None,
            ball_step: 0.,
        };
        view.update(run, 0.);
        Some(view)
    }

    fn play(&mut self, action: &'static str, mode: RoleAnimationRepeatMode) {
        self.action = action;
        if let Some(rc) = RoleController::get_role_controller(self.man.clone()) {
            rc.inner::<RoleController>().play_anim(action, mode);
        }
    }

    fn action_over(&self) -> bool {
        RoleController::get_role_controller(self.man.clone())
            .is_none_or(|rc| rc.inner::<RoleController>().state() == RoleState::AnimationFinished)
    }

    /// 人物的动作跟着规则层的状态换。
    fn step_pose(&mut self, run: &SkiRun, dt: f32) {
        let phase = run.phase();
        if phase != self.phase {
            match (self.phase, phase) {
                (_, Phase::Jumping) => {
                    self.pose = Pose::Jumping;
                    self.play(ACT_JUMP, RoleAnimationRepeatMode::Repeat(1));
                }
                (_, Phase::Hit) => {
                    self.pose = Pose::Tumbling;
                    self.play(ACT_TUMBLE, RoleAnimationRepeatMode::Repeat(1));
                }
                // 跳完落地：先颠一下（`_SkeeACT_Jolt`）。
                (Phase::Jumping, Phase::Rolling) => {
                    self.pose = Pose::Once(ACT_JOLT);
                    self.play(ACT_JOLT, RoleAnimationRepeatMode::Repeat(1));
                }
                // 摔完掉速到停：起身（`_SkeeACT_Ready`）。
                (Phase::Hit, Phase::Rolling) => {
                    self.pose = Pose::Once(ACT_READY);
                    self.play(ACT_READY, RoleAnimationRepeatMode::Repeat(1));
                }
                _ => {}
            }
            self.phase = phase;
        }
        match self.pose {
            Pose::Once(_) if self.action_over() => {
                self.pose = Pose::Sliding;
                self.play(ACT_DOING1, RoleAnimationRepeatMode::Loop);
            }
            // 正常滑：过了 `m_actiontic` 毫秒就重新掷一次，`((rand() % 300) / 10) / 10` 是 0 / 1 / 2，
            // 分别是 h02 / h03 / h04。
            Pose::Sliding => {
                self.action_clock += dt * 1000.;
                if self.action_clock >= self.action_tic {
                    self.action_clock = 0.;
                    self.action_tic = (1000 + self.rand.rand15() % 2600) as f32;
                    let act = (self.rand.rand15() % 300) / 10 / 10;
                    match act {
                        1 => {
                            self.pose = Pose::Once(ACT_DOING2);
                            self.play(ACT_DOING2, RoleAnimationRepeatMode::Repeat(1));
                        }
                        2 => {
                            self.pose = Pose::Once(ACT_JOLT);
                            self.play(ACT_JOLT, RoleAnimationRepeatMode::Repeat(1));
                        }
                        _ => {}
                    }
                }
            }
            Pose::Jumping | Pose::Tumbling if phase == Phase::Rolling && self.action_over() => {
                self.pose = Pose::Sliding;
                self.play(ACT_DOING1, RoleAnimationRepeatMode::Loop);
            }
            _ => {}
        }
        // `UpdateFinish`：速度减到 0 才站住。
        if phase == Phase::Stopping && run.speed <= 0. && self.pose != Pose::Standing {
            self.pose = Pose::Standing;
            self.play(ACT_STAND, RoleAnimationRepeatMode::Loop);
        }
    }

    /// 每帧：摆人、雪板、古董、雪球，跟镜头。`dt` 是秒。
    pub fn update(&mut self, run: &SkiRun, dt: f32) {
        self.step_pose(run, dt);
        let frames = dt * FRAME_RATE;

        let (x, y, z) = run.pos;
        let pos = Vec3::new(x, y, z);
        let dir = Vec3::sub(&pos, &self.prev_pos);
        if dir.norm() > 1e-4 {
            (self.step, self.balance) = step_and_balance(&dir);
        }
        self.prev_pos = pos;

        // `Skee_slider::Render`：跳的时候 y 换成跳的高度；收尾时朝向再转 −90、不倾斜，雪板收起来。
        let finishing = self.phase == Phase::Stopping;
        let at = Vec3::new(x, run.height(), z);
        let yaw = if finishing {
            self.step - 90.
        } else {
            self.step
        };
        let tilt = if finishing || self.phase == Phase::Hit {
            0.
        } else {
            self.balance - 90.
        };
        let base = mul(
            &mul(&translate(&at), &rotate([0., 1., 0.], yaw)),
            &rotate([0., 0., 1.], tilt),
        );
        self.man
            .transform()
            .borrow_mut()
            .set_matrix(mul(&base, &mv3_flip()));
        if let Some(board) = &self.board {
            board.set_visible(!finishing);
            let tick = RoleController::get_role_controller(self.man.clone())
                .map(|rc| (rc.inner::<RoleController>().anim_time() * 4580.) as u32)
                .unwrap_or(0);
            let tag = self
                .tags
                .get(self.action)
                .map(|f| tag_matrix(f, tick))
                .unwrap_or_else(Mat44::new_identity);
            board.transform().borrow_mut().set_matrix(mul(&base, &tag));
        }

        // 古董：`Translate(pos)` 后绕 Y 转，每帧 6 度。
        self.treasure_spin = (self.treasure_spin + TREASURE_SPIN_DEG * frames) % 360.;
        for (t, e) in run.treasures().iter().zip(&self.treasures) {
            let Some(e) = e else { continue };
            let shown = t.present && !run.picked().contains(&t.id);
            e.set_visible(shown);
            if shown {
                e.transform().borrow_mut().set_matrix(mul(
                    &translate(&Vec3::new(t.x, t.y, t.z)),
                    &rotate([0., 1., 0.], self.treasure_spin),
                ));
            }
        }

        // 追来的雪球：`Translate(pos)`、绕 Y 转行进角、绕 X 滚（每帧 16 度）。
        if let Some(ball) = &self.ball {
            let chasing = run.chaser_out() && run.outcome().is_none();
            ball.set_visible(chasing);
            if chasing {
                let p = run.path().at(run.chaser).unwrap_or(run.pos);
                let p = Vec3::new(p.0, p.1, p.2);
                if let Some(prev) = self.ball_prev {
                    let d = Vec3::sub(&p, &prev);
                    if d.norm() > 1e-4 {
                        self.ball_step = step_and_balance(&d).0;
                    }
                }
                self.ball_prev = Some(p);
                self.ball_roll = (self.ball_roll + BALL_ROLL_DEG * frames) % 360.;
                ball.transform().borrow_mut().set_matrix(mul(
                    &mul(&translate(&p), &rotate([0., 1., 0.], self.ball_step)),
                    &rotate([1., 0., 0.], self.ball_roll),
                ));
            }
        }

        // 雪球一出来，镜头偏 80 度看它，停 2.6 秒再摆回来。
        if run.chasing() && !self.chasing {
            self.swing_on = true;
            self.swing_back = false;
            self.swing_hold = 0.;
        }
        self.chasing = run.chasing();
        if self.swing_on {
            if !self.swing_back {
                self.swing = (self.swing + frames).min(CAM_SWING_DEG);
                if self.swing >= CAM_SWING_DEG {
                    self.swing_hold += dt;
                    if self.swing_hold >= CAM_SWING_HOLD {
                        self.swing_back = true;
                    }
                }
            } else {
                self.swing = (self.swing - frames).max(0.);
                if self.swing <= 0. {
                    self.swing_on = false;
                }
            }
        }

        // 镜头（`Skee_camera::ComputeInfo`）：看点在人头上 200，偏航 `90 − (行进角 − 90)`，俯仰 −56，
        // 退后 780。
        let yaw = (CAM_YAW - (self.step - 90.) + self.swing).to_radians();
        let pitch = CAM_PITCH.to_radians();
        let forward = Vec3::new(
            yaw.sin() * pitch.cos(),
            pitch.sin(),
            -pitch.cos() * yaw.cos(),
        );
        let target = Vec3::new(x, y + CAM_LIFT, z);
        let eye = Vec3::sub(&target, &Vec3::scalar_mul(CAM_DIST, &forward));
        let mut cam = self.scene.camera_mut();
        cam.set_fov43(CAM_FOV_DEG.to_radians());
        cam.transform_mut().set_position(&eye).look_at(&target);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: &Mat44, v: [f32; 3]) -> [f32; 3] {
        let f = m.floats();
        [0, 1, 2].map(|r| f[r][0] * v[0] + f[r][1] * v[1] + f[r][2] * v[2] + f[r][3])
    }

    fn close(a: [f32; 3], b: [f32; 3]) -> bool {
        (0..3).all(|k| (a[k] - b[k]).abs() < 1e-4)
    }

    #[test]
    fn rotate_takes_degrees_and_is_right_handed() {
        // `SetAxisAngle`：绕 Y x' = x cos + z sin、z' = −x sin + z cos；绕 Z x' = x cos − y sin。
        assert!(close(
            apply(&rotate([0., 1., 0.], 90.), [0., 0., 1.]),
            [1., 0., 0.]
        ));
        assert!(close(
            apply(&rotate([0., 0., 1.], 90.), [1., 0., 0.]),
            [0., 1., 0.]
        ));
        assert!(close(
            apply(&rotate([1., 0., 0.], 90.), [0., 1., 0.]),
            [0., 0., 1.]
        ));
    }

    #[test]
    fn the_step_angle_points_the_model_along_the_way() {
        // 行进角转过去，模型的 +Z 就指着走的方向。
        for d in [[1., 0., 0.], [0., 0., -1.], [-1., 0., 1.]] {
            let dir = Vec3::new(d[0], d[1], d[2]);
            let (step, balance) = step_and_balance(&dir);
            assert!((balance - 90.).abs() < 1e-3);
            let n = dir.norm();
            let fwd = apply(&rotate([0., 1., 0.], step), [0., 0., 1.]);
            assert!(close(fwd, [d[0] / n, 0., d[2] / n]), "{d:?} → {fwd:?}");
        }
    }

    #[test]
    fn tag_frames_interpolate_and_wrap() {
        let id = [0., 0., 0., -1.];
        let frames = [
            TagFrame {
                tick: 0,
                pos: [0., 0., 0.],
                rot: id,
            },
            TagFrame {
                tick: 100,
                pos: [10., 0., 0.],
                rot: id,
            },
            TagFrame {
                tick: 200,
                pos: [10., 20., 0.],
                rot: id,
            },
        ];
        // w 取反后是单位四元数（不转），只看平移。
        assert!(close(
            apply(&tag_matrix(&frames, 50), [0., 0., 0.]),
            [5., 0., 0.]
        ));
        assert!(close(
            apply(&tag_matrix(&frames, 150), [1., 0., 0.]),
            [11., 10., 0.]
        ));
        // 对最后一帧的时间取余。
        assert!(close(
            apply(&tag_matrix(&frames, 250), [0., 0., 0.]),
            [5., 0., 0.]
        ));
    }
}
