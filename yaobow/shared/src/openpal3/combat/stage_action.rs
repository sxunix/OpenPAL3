//! flexeff `action` 类：仙术演出时让舞台上的某个人做动作、走位、转身、淡入淡出。
//! 定义记录没有资源，类型名是「景天」「雪」这类角色名；挂在 150–159、197–201、485–512
//! 等组上，和 camera 类一起出现。
//!
//! 逆向记在 `switch/re/notes/combat_vfx.md` §11。原版（vtable 0x56f5e4，时间线那一步是
//! FUN_004c0c10，延迟子块 `+72`、时长 `+76`）：
//!
//! * 子块 `+84` 取整是舞台位号（0..=10，越界的只等时间走完）；`+0` 取整是动作号。
//! * 第一次进来：
//!   - 把那个人标成「特效在控制」（舞台角色 `+0x17c = 1`，实例全结束后每帧函数 FUN_004c0740 清回 0）；
//!   - `FUN_0050ac50(动作号, 1)`：用 `"z%02d"` 拼动作名（5011 → `z5011`、3 → `z03`），强制从头播；
//!   - 子块 `+68` > 0 时记下当前位置作为插值起点。
//! * 每帧：
//!   - 位置：`+68` > 0 时从起点线性插值到 `+40..+48`（绝对坐标），用时长做分母；否则位置 += `+4..+12` × dt；
//!   - `+89` 非零且位置变了：朝移动方向转身，yaw = acos(方向·(0,0,1))，方向 x < 0 时取 360° − 它；
//!   - `+56` 非零：alpha += dt × `+56`，截 0..1（`gbVisNode::SetBlend`，> 0.98 当不透明）；
//!   - 子块 `+60 / +64` 写进角色 `+0x180 / +0x184`，用途没查到；
//!   - `+90` 非零且动作播到最后一帧：若 `+92` > 0 就转到 `+92` 度，这一步提前结束；
//!   - 到时长或被强停：若 `+92` > 0 就转到 `+92` 度，这一步结束。
//! * `+80` 选武器拖光（blade）的时长 0.25 / 0.01 秒，移植端没有拖光，不接。
//!
//! 朝向约定：yaw θ（度）对应朝向 (sin θ, cos θ)。我方站姿朝向 (√½, √½) 正是 45°，怪物 225°，
//! 实机 `+92` 的取值里就有 45 和 225。

/// 一条动作提示（一个启用的子块）。
#[derive(Debug, Clone, PartialEq)]
pub struct ActionCue {
    /// 舞台位号，`+84`。
    pub slot: i32,
    /// 动作号，`+0`；≤ 0 不换动作。
    pub motion: i32,
    delay: f32,
    duration: f32,
    velocity: [f32; 3],
    target: [f32; 3],
    lerp: bool,
    fade: f32,
    face_move: bool,
    face_at_end: bool,
    final_yaw: f32,
    t: f32,
    started: bool,
    done: bool,
    start: [f32; 3],
}

/// 这一帧要对那个人做的事。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ActionFrame {
    /// 刚开始：接管、播动作。
    pub begin: bool,
    pub position: Option<[f32; 3]>,
    /// 度，朝向 (sin, cos)。
    pub yaw: Option<f32>,
    pub alpha: Option<f32>,
    /// 这一步结束（只在开始过的提示上报）。
    pub end: bool,
}

impl ActionCue {
    pub fn from_sub(sub: &[u8]) -> Self {
        let f = |o: usize| f32::from_le_bytes(sub[o..o + 4].try_into().unwrap());
        Self {
            slot: f(84).round_ties_even() as i32,
            motion: f(0).round_ties_even() as i32,
            delay: f(72).max(0.),
            duration: f(76).max(0.),
            velocity: [f(4), f(8), f(12)],
            target: [f(40), f(44), f(48)],
            lerp: f(68) > 0.,
            fade: f(56),
            face_move: sub[89] != 0,
            face_at_end: sub[90] != 0,
            final_yaw: f(92),
            t: 0.,
            started: false,
            done: false,
            start: [0.; 3],
        }
    }

    /// `FUN_0050ac50` 拼的动作名。
    pub fn motion_name(&self) -> Option<String> {
        (self.motion > 0).then(|| format!("z{:02}", self.motion))
    }

    pub fn finished(&self) -> bool {
        self.done
    }

    pub fn started(&self) -> bool {
        self.started
    }

    fn valid_slot(&self) -> bool {
        (0..=10).contains(&self.slot)
    }

    /// 往前走 `delta_sec`。`pos` / `alpha` 是那个人现在的位置和透明度，
    /// `anim_done` 表示他当前的动作已经播到最后一帧。
    pub fn step(
        &mut self,
        delta_sec: f32,
        pos: [f32; 3],
        alpha: f32,
        anim_done: bool,
    ) -> ActionFrame {
        let mut frame = ActionFrame::default();
        if self.done {
            return frame;
        }
        self.t += delta_sec;
        if self.t <= self.delay {
            return frame;
        }
        let local = self.t - self.delay;
        let mut d = delta_sec;
        if local > self.duration {
            d -= local - self.duration;
        }
        let d = d.max(0.);
        if !self.started {
            self.started = true;
            frame.begin = self.valid_slot();
            self.start = pos;
        }
        if self.valid_slot() {
            let next = if self.lerp {
                if self.duration - local > 0. {
                    let k = local / self.duration;
                    std::array::from_fn(|i| (self.target[i] - self.start[i]) * k + self.start[i])
                } else {
                    self.target
                }
            } else {
                std::array::from_fn(|i| self.velocity[i] * d + pos[i])
            };
            frame.position = Some(next);
            if self.face_move && next != pos {
                let v = [next[0] - pos[0], next[1] - pos[1], next[2] - pos[2]];
                let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                if len > 0. {
                    let mut yaw = (v[2] / len).clamp(-1., 1.).acos();
                    if v[0] < 0. {
                        yaw = std::f32::consts::TAU - yaw;
                    }
                    frame.yaw = Some(yaw.to_degrees());
                }
            }
            if self.fade != 0. {
                frame.alpha = Some((alpha + d * self.fade).clamp(0., 1.));
            }
            // 动作播完就提前收（开始那一帧看到的还是上一个动作的状态，不算）。
            if self.face_at_end && anim_done && !frame.begin {
                if self.final_yaw > 0. {
                    frame.yaw = Some(self.final_yaw);
                }
                self.done = true;
                frame.end = true;
                return frame;
            }
        }
        if local >= self.duration {
            if self.valid_slot() && self.final_yaw > 0. {
                frame.yaw = Some(self.final_yaw);
            }
            self.done = true;
            frame.end = self.valid_slot();
        }
        frame
    }
}

/// yaw（度）对应的朝向 (x, z)。
pub fn yaw_direction(yaw_deg: f32) -> (f32, f32) {
    let (s, c) = yaw_deg.to_radians().sin_cos();
    (s, c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub_with(pairs: &[(usize, f32)], bytes: &[(usize, u8)]) -> Vec<u8> {
        let mut b = vec![0u8; 124];
        for (o, v) in pairs {
            b[*o..*o + 4].copy_from_slice(&v.to_le_bytes());
        }
        for (o, v) in bytes {
            b[*o] = *v;
        }
        b
    }

    #[test]
    fn the_motion_number_becomes_a_z_name() {
        let c = ActionCue::from_sub(&sub_with(&[(0, 5011.), (84, 1.)], &[]));
        assert_eq!(c.motion_name().as_deref(), Some("z5011"));
        let c = ActionCue::from_sub(&sub_with(&[(0, 3.)], &[]));
        assert_eq!(c.motion_name().as_deref(), Some("z03"));
        assert_eq!(c.slot, 0);
    }

    #[test]
    fn lerp_walks_from_where_it_started_to_the_target() {
        let mut c = ActionCue::from_sub(&sub_with(
            &[(84, 2.), (76, 2.), (68, 1.), (40, 100.), (48, -100.)],
            &[],
        ));
        let f = c.step(1.0, [0., 0., 0.], 1., false);
        assert!(f.begin);
        assert_eq!(f.position, Some([50., 0., -50.]));
        let f = c.step(1.0, [50., 0., -50.], 1., false);
        assert_eq!(f.position, Some([100., 0., -100.]));
        assert!(f.end && c.finished());
    }

    #[test]
    fn velocity_moves_and_faces_the_way_it_goes() {
        let mut c = ActionCue::from_sub(&sub_with(&[(84, 1.), (76, 5.), (4, 100.)], &[(89, 1)]));
        let f = c.step(0.5, [0., 0., 0.], 1., false);
        assert_eq!(f.position, Some([50., 0., 0.]));
        assert!((f.yaw.unwrap() - 90.).abs() < 1e-3, "往 +x 走朝 90°");
        let mut c = ActionCue::from_sub(&sub_with(&[(84, 1.), (76, 5.), (4, -100.)], &[(89, 1)]));
        let f = c.step(0.5, [0., 0., 0.], 1., false);
        assert!((f.yaw.unwrap() - 270.).abs() < 1e-3, "往 −x 走朝 270°");
    }

    #[test]
    fn it_turns_to_the_final_yaw_when_time_is_up() {
        let mut c =
            ActionCue::from_sub(&sub_with(&[(84, 1.), (72, 0.5), (76, 1.), (92, 45.)], &[]));
        assert_eq!(
            c.step(0.4, [0.; 3], 1., false),
            ActionFrame::default(),
            "延迟里什么都不做"
        );
        c.step(0.2, [0.; 3], 1., false);
        let f = c.step(1.0, [0.; 3], 1., false);
        assert_eq!(f.yaw, Some(45.));
        assert!(f.end);
    }

    #[test]
    fn a_finished_motion_ends_it_early_but_not_on_the_first_frame() {
        let mut c = ActionCue::from_sub(&sub_with(&[(84, 1.), (76, 10.), (92, 225.)], &[(90, 1)]));
        let f = c.step(0.1, [0.; 3], 1., true);
        assert!(f.begin && !f.end, "开始那一帧的「播完」是上一个动作的");
        let f = c.step(0.1, [0.; 3], 1., true);
        assert!(f.end);
        assert_eq!(f.yaw, Some(225.));
    }

    #[test]
    fn fading_is_clamped() {
        let mut c = ActionCue::from_sub(&sub_with(&[(84, 1.), (76, 5.), (56, -2.)], &[]));
        let f = c.step(0.25, [0.; 3], 1., false);
        assert!((f.alpha.unwrap() - 0.5).abs() < 1e-5);
        let f = c.step(1.0, [0.; 3], 0.5, false);
        assert_eq!(f.alpha, Some(0.));
    }

    #[test]
    fn an_out_of_range_slot_only_waits() {
        let mut c = ActionCue::from_sub(&sub_with(&[(84, 20.), (76, 1.), (4, 100.)], &[]));
        let f = c.step(0.5, [0.; 3], 1., false);
        assert!(!f.begin && f.position.is_none());
        let f = c.step(1.0, [0.; 3], 1., false);
        assert!(!f.end && c.finished());
    }
}
