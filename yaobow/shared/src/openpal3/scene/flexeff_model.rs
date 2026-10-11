//! flexeff `model` 类（`CEffModel`，源文件 `D:\Project\PAL3\combat\EffModel.cpp`）：
//! 在特效位置摆一个 `.cvd` / `.pol` 模型（魔剑、风巢之类）。
//!
//! 逆向记在 `switch/re/notes/combat_vfx.md` §9，要点（vtable 0x56f9dc）：
//!
//! * `+4` 0x4cfeb0：`FUN_004d0950` 用引擎的 `gbGeomNode::LoadFromFile` 读资源。
//! * `+8` 0x4cfe20 摆位：
//!   - 位置 = 播放点 + 定义 `+0xb8..`，绕圈偏移清零；
//!   - 转角取定义 `+0xd8 / +0xc4 / +0xdc`（绕 X / Y / Z），三轴缩放都取 `+0xc8`；
//!   - 设为不可见，`gbGeomNode::Reset(0)`。
//! * 时间线（`+0x30` 0x4d03b0 排、0x4d05e0 走）：延迟子块 `+72`，持续 `+76`。这一步里每帧：
//!   - 第一次进来：置可见，`Reset(0)` 让动画从头播；
//!   - 位置 += 子块 `+4..+12` × dt；
//!   - 三轴缩放各加子块 `+0 / +92 / +84`，三轴转角各加 `+40 / +44 / +48`——**按帧加，不乘 dt**；
//!   - 绕圈：转轴 `+20..+28`（零向量按 `gbVec3D::Normalize` 当 +y），角度 = (t − 起点) × `+36` 度，
//!     转 `(+32, 0, 0)` 得偏移（和 particle 类同一套）；
//!   - `+56` 非零时 alpha += dt × `+56`，截在 0..1，`gbVisNode::SetBlend`；
//!   - 到时长或被强停，这一步结束。
//! * `+0x14` FUN_004cffb0：推时间线；实例没了就清可见；然后 `gbGeomNode::Update()`（引擎时钟推动画）。
//! * `+0x10` 0x4d01c0：可见时矩阵栈 平移(位置 + 绕圈偏移) → 绕 X → 绕 Y → 绕 Z → 缩放，再 `Render`。
//!
//! gbengine 的几条底层约定都在 `engine/math/gbmath.cpp` 里查实了，见
//! [`super::flexeff_explosive`] 开头：角度按度、旋转右手系、矩阵栈
//! 「后压的先作用」。 这一类的实际作用顺序是
//! 缩放 → Z → Y → X → 平移。
//!
//! 动画**确实是循环的**：`gbGeomNode` 构造里 `nLoop = 0`
//! （`geometry/gbgeomnode.cpp:27`，头文件注明「nLoop==0 表示无限循环」），
//! `CEffModel` 从不调 `SetLoop`；`gbGeomNode::Update` 按
//! `pGfxMgr->GetDeltaTime() * TimeScale` 推进，到头就把 `CurTime` 减一个周期。
//!
//! 两处移植端自己定的：
//! - 按帧累加的量（缩放、转角，`EffModel.cpp:238`..`:245` 确实不乘 `fDeltaTime`）
//!   按 60 帧每秒换算成每秒。原版没有帧率上限，快慢跟着机器走，只能挑一个；
//!   `encamp` 的指针速度、`dealer::flash_alpha` 都是同一个选择。
//! - alpha（`+56` → `gbVisNode::SetBlend`）还没接到材质上，实机数据也没用到。

use crosscom::ComRc;
use fileformats::pal3::flexeff::EffectDef;
use radiance::comdef::{IEntity, IEntityExt};
use radiance::math::{Transform, Vec3};

use crate::openpal3::comdef::ICvdModel;
use crate::openpal3::scene::{CvdModelComponent, CvdPlayMode};

/// 按帧累加的量换成每秒时用的帧率。原版没有帧率上限，这是移植端挑的数，
/// 不是查出来的——见模块开头。
pub const ORIGINAL_FPS: f32 = 60.;

/// 定义记录里 `model` 用得上的量。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ModelDef {
    /// `+0xb8..+0xc0`：相对播放点的偏移。
    pub offset: [f32; 3],
    /// `+0xd8 / +0xc4 / +0xdc`：绕 X / Y / Z 的初始转角（度）。实机主要是 `+0xc4`。
    pub rotation: [f32; 3],
    /// `+0xc8`：三轴同一个缩放。
    pub scale: f32,
}

impl ModelDef {
    pub fn from_def(def: &EffectDef<'_>) -> Self {
        let b = def.raw();
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        Self {
            offset: [f(0xb8), f(0xbc), f(0xc0)],
            rotation: [f(0xd8), f(0xc4), f(0xdc)],
            scale: f(0xc8),
        }
    }
}

/// 子块里 `model` 用到的字段。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ModelSub {
    pub delay: f32,
    pub duration: f32,
    /// `+4..+12`：每秒平移。
    pub velocity: [f32; 3],
    /// `+0 / +92 / +84`：三轴缩放每帧加多少。
    pub grow: [f32; 3],
    /// `+40 / +44 / +48`：三轴转角每帧加多少度。
    pub spin: [f32; 3],
    /// `+20..+28` 转轴、`+32` 半径、`+36` 每秒几度。
    pub axis: [f32; 3],
    pub radius: f32,
    pub orbit: f32,
    /// `+56`：alpha 每秒变多少。
    pub fade: f32,
}

impl ModelSub {
    pub fn from_sub(sub: &[u8]) -> Self {
        let f = |o: usize| f32::from_le_bytes(sub[o..o + 4].try_into().unwrap());
        Self {
            delay: f(72).max(0.),
            duration: f(76).max(0.),
            velocity: [f(4), f(8), f(12)],
            grow: [f(0), f(92), f(84)],
            spin: [f(40), f(44), f(48)],
            axis: [f(20), f(24), f(28)],
            radius: f(32),
            orbit: f(36),
            fade: f(56),
        }
    }
}

/// 一个正在播的 `model` 实例。
#[derive(Debug, Clone)]
pub struct ModelEffect {
    pub def: ModelDef,
    sub: ModelSub,
    position: [f32; 3],
    orbit: [f32; 3],
    /// 度。
    rotation: [f32; 3],
    scale: [f32; 3],
    alpha: f32,
    t: f32,
    started: bool,
    done: bool,
}

impl ModelEffect {
    pub fn new(def: ModelDef, sub: &[u8], origin: [f32; 3]) -> Self {
        Self {
            def,
            sub: ModelSub::from_sub(sub),
            position: [
                origin[0] + def.offset[0],
                origin[1] + def.offset[1],
                origin[2] + def.offset[2],
            ],
            orbit: [0.; 3],
            rotation: def.rotation,
            scale: [def.scale; 3],
            alpha: 1.,
            t: 0.,
            started: false,
            done: false,
        }
    }

    /// 往前走 `delta_sec`。返回 `true` 表示这一帧刚开始播（该显示出来、动画从头放）。
    pub fn step(&mut self, delta_sec: f32) -> bool {
        if self.done {
            return false;
        }
        self.t += delta_sec;
        if self.t <= self.sub.delay {
            return false;
        }
        let local = self.t - self.sub.delay;
        let mut d = delta_sec;
        if local > self.sub.duration {
            d -= local - self.sub.duration;
        }
        let d = d.max(0.);
        let began = !self.started;
        self.started = true;

        let frames = d * ORIGINAL_FPS;
        for k in 0..3 {
            self.position[k] += self.sub.velocity[k] * d;
            self.scale[k] += self.sub.grow[k] * frames;
            self.rotation[k] += self.sub.spin[k] * frames;
        }
        // 绕圈：Rodrigues 转 (半径, 0, 0)。
        let a = self.sub.axis;
        let len = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        let k = if len < 1e-6 {
            [0., 1., 0.]
        } else {
            [a[0] / len, a[1] / len, a[2] / len]
        };
        let (s, c) = (local.min(self.sub.duration) * self.sub.orbit)
            .to_radians()
            .sin_cos();
        let r = self.sub.radius;
        // v = (r, 0, 0)；k × v = (0, k.z·r, −k.y·r)；k·v = k.x·r。
        let cross = [0., k[2] * r, -k[1] * r];
        let dot = k[0] * r;
        self.orbit = [
            r * c + cross[0] * s + k[0] * dot * (1. - c),
            cross[1] * s + k[1] * dot * (1. - c),
            cross[2] * s + k[2] * dot * (1. - c),
        ];
        if self.sub.fade != 0. {
            self.alpha = (self.alpha + d * self.sub.fade).clamp(0., 1.);
        }
        if local >= self.sub.duration {
            self.done = true;
        }
        began
    }

    pub fn visible(&self) -> bool {
        self.started && !self.done
    }

    pub fn finished(&self) -> bool {
        self.done
    }

    /// 平移（位置 + 绕圈偏移）、转角（度）、缩放。
    pub fn pose(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        (
            [
                self.position[0] + self.orbit[0],
                self.position[1] + self.orbit[1],
                self.position[2] + self.orbit[2],
            ],
            self.rotation,
            self.scale,
        )
    }
}

/// 整棵实体树一起显示 / 隐藏（`.cvd` 的网格挂在子节点上）。
pub fn set_visible_tree(entity: &ComRc<IEntity>, visible: bool) {
    entity.set_visible(visible);
    for child in entity.children() {
        set_visible_tree(&child, visible);
    }
}

/// 让整棵树上的 CVD 关键帧从头播（`gbGeomNode::Reset(0)` 之后由 `Update` 推）。
pub fn play_tree(entity: &ComRc<IEntity>) {
    if let Some(c) = entity
        .get_component(ICvdModel::uuid())
        .and_then(|c| c.query_interface::<ICvdModel>())
    {
        c.inner::<CvdModelComponent>().play(CvdPlayMode::Loop, 1.);
    }
    for child in entity.children() {
        play_tree(&child);
    }
}

/// 0x4d01c0 的矩阵栈：平移 → 绕 X → 绕 Y → 绕 Z → 缩放。
pub fn place(entity: &ComRc<IEntity>, effect: &ModelEffect) {
    let (p, r, s) = effect.pose();
    let mut t = Transform::new();
    t.set_position(&Vec3::new(p[0], p[1], p[2]))
        .rotate_axis_angle_local(&Vec3::new(1., 0., 0.), r[0].to_radians())
        .rotate_axis_angle_local(&Vec3::new(0., 1., 0.), r[1].to_radians())
        .rotate_axis_angle_local(&Vec3::new(0., 0., 1.), r[2].to_radians())
        .scale_local(&Vec3::new(s[0], s[1], s[2]));
    *entity.transform().borrow_mut() = t;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_def() -> ModelDef {
        ModelDef {
            offset: [10., 0., 0.],
            rotation: [0., 45., 0.],
            scale: 2.,
        }
    }

    fn sub_with(pairs: &[(usize, f32)]) -> Vec<u8> {
        let mut b = vec![0u8; 124];
        for (o, v) in pairs {
            b[*o..*o + 4].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    #[test]
    fn it_shows_only_inside_its_window() {
        let mut e = ModelEffect::new(a_def(), &sub_with(&[(72, 0.5), (76, 1.)]), [0.; 3]);
        assert!(!e.step(0.4));
        assert!(!e.visible());
        assert!(e.step(0.2), "越过延迟那一帧报「开始」");
        assert!(e.visible());
        assert!(!e.step(0.2));
        e.step(1.0);
        assert!(e.finished() && !e.visible());
    }

    #[test]
    fn it_drifts_and_spins_per_frame() {
        // 每秒平移 (100,0,0)；每帧绕 Y 转 2 度，按 60 fps 一秒 120 度。
        let mut e = ModelEffect::new(
            a_def(),
            &sub_with(&[(76, 5.), (4, 100.), (44, 2.)]),
            [0.; 3],
        );
        e.step(0.5);
        let (p, r, s) = e.pose();
        assert!((p[0] - 60.).abs() < 1e-3, "10 + 100 × 0.5");
        assert!((r[1] - 105.).abs() < 1e-3, "45 + 2 × 30 帧");
        assert_eq!(s, [2., 2., 2.]);
    }

    #[test]
    fn it_orbits_around_y_by_default() {
        // 转轴全零 → +y；半径 60、每秒 180 度：半秒转 90 度，(60,0,0) → (0,0,−60)。
        let mut e = ModelEffect::new(
            ModelDef {
                offset: [0.; 3],
                rotation: [0.; 3],
                scale: 1.,
            },
            &sub_with(&[(76, 5.), (32, 60.), (36, 180.)]),
            [0.; 3],
        );
        e.step(0.5);
        let (p, _, _) = e.pose();
        assert!(
            p[0].abs() < 1e-3 && p[1].abs() < 1e-3 && (p[2] + 60.).abs() < 1e-3,
            "{p:?}"
        );
    }
}
