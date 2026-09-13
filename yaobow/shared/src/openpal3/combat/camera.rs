//! 战斗镜头：`cbdata\flexcam.dat` 里的镜头序列（`D:\Project\PAL3\Combat\CBStageCam.cpp`）。
//!
//! 文件 430000 字节（`FUN_00504f40` 分三次读）：
//!
//! ```text
//! 0      2000 条镜头记录，每条 0xcc 字节
//! 408000 2000 字节，每条记录一个「占用」字节（FUN_00505250 清空序列时清掉）
//! 410000 100 个序列 × 50 步的 i32 记录号，-1 = 空
//! ```
//!
//! 播放（`FUN_005061a0`，每帧）：当前序列 `+0x68fd0`、当前步 `+0x68fd4`；这一步
//! 没完就交给 `FUN_00506340`（记录 `+0x5c` 置位时）移镜头，完了下一帧换下一步，
//! 步数用完就停——镜头停在最后一个机位。开战时 0x512f07/0x512f20 播序列：
//! 我方偷袭（`+0x219`）且 `FUN_004c6410()+4` 时播 [`SNEAK_SEQUENCE`]，否则
//! [`STAND_SEQUENCE`]。
//!
//! 移镜头（`FUN_00506340`）做了「匀速」分支（`+0x59 != 0 && +0x5b == 0`，绝大多数记录）
//! 和 `+0x59 == 0` 那一支；另有盯着目标（`+0x65`）、绕圈（`+0x5d`，`FUN_005075e0`）、
//! 滑行（`+0x5e`，`FUN_00507b20`）。没做的三支——`FUN_00506340` 里 `+0x5b` 那一支、
//! `FUN_005075e0` 里 `+0x5a != 0` 那一支、跟随目标（`+0x66`）——发行的 flexcam.dat 里
//! 没有一条记录用到。逆向记录在 `switch/re/notes/combat_camera.md`。

use super::stage::CombatCamera;

pub const FLEXCAM_PATH: &str = "/basedata/basedata/cbdata/flexcam.dat";
pub const RECORDS: usize = 2000;
pub const RECORD_SIZE: usize = 0xcc;
pub const SEQUENCES: usize = 100;
pub const STEPS: usize = 50;
const USED_OFFSET: usize = RECORDS * RECORD_SIZE;
const TABLE_OFFSET: usize = USED_OFFSET + RECORDS;
pub const FILE_SIZE: usize = TABLE_OFFSET + SEQUENCES * STEPS * 4;

/// 平时开战播的序列（0x512f19：`push ebx`，ebx = 0）。
pub const STAND_SEQUENCE: usize = 0;
/// 我方偷袭时开战播的序列（0x512f02：`push 0x14`）。
pub const SNEAK_SEQUENCE: usize = 20;
/// 打赢（敌方全灭或回合到顶）：`FUN_004f7910` 结算完 `FUN_00505770(3)`。
pub const WIN_SEQUENCE: usize = 3;
/// 全灭：同一函数 0x4f7a61 `push 0xa`。
pub const LOSE_SEQUENCE: usize = 10;
/// 逃掉了（phase 4）：0x4f7e2a `push 8`。
pub const ESCAPE_SEQUENCE: usize = 8;

/// 散场前播哪个序列。
pub fn ending_sequence(outcome: super::state::Outcome) -> Option<usize> {
    use super::state::Outcome;
    match outcome {
        Outcome::Win => Some(WIN_SEQUENCE),
        Outcome::Lose => Some(LOSE_SEQUENCE),
        Outcome::Escaped => Some(ESCAPE_SEQUENCE),
        Outcome::Ongoing => None,
    }
}

/// 一条镜头记录（字段按 `FUN_00506340` 的读法）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CamRecord {
    /// `+0x1c`，秒。
    pub duration: f32,
    /// `+0x20..+0x34`：x y z yaw pitch roll。
    pub from: CombatCamera,
    /// `+0x38..+0x4c`。
    pub to: CombatCamera,
    /// `+0x58..+0x67` 十六个标志字节。
    pub flags: [u8; 16],
    /// `+0x68`：跟随 / 盯着的战斗位（-1 = 战场中心）。
    pub target: i32,
    /// `+0x74`、`+0x78`：两段加速度从什么时候开始。
    pub accel_start: [f32; 2],
    /// `+0x7c`、`+0x80`：两段加速度。
    pub accel: [f32; 2],
    /// `+0xac`：滑行那一步算进度时加在时长上的（`FUN_00507b20`）。实机数据全是 0。
    pub extra: f32,
    /// `+0x6c`：`+0x59 == 0` 那种移动方式的速度（`FUN_00506340`）。
    pub speed: f32,
    /// `+0x84` / `+0x88`：绕圈（`FUN_005075e0`）的总角度和角速度，单位是**度**
    /// （`gbQuaternion::SetYawPitchRoll` 进门先乘 π/180）。
    pub orbit_total: f32,
    pub orbit_speed: f32,
    /// `+0x8c..+0x94`：绕圈的圆心。
    pub orbit_center: [f32; 3],
    /// `+0x9c`：绕完一圈时高度从圆心的 y 挪到这里（按转过的比例插）。
    pub orbit_height: f32,
}

impl CamRecord {
    /// `+0x58`：起点用记录里的 `from`；否则用镜头当前的机位（`FUN_005062d0`）。
    pub fn keeps_from(&self) -> bool {
        self.flags[0] != 0
    }
    /// `+0x5c`：这一步要移镜头（`FUN_005061a0` 才会调 `FUN_00506340`）。
    pub fn moves_camera(&self) -> bool {
        self.flags[4] != 0
    }
    /// `+0x5f`：走满距离就算完。
    pub fn stops_at_distance(&self) -> bool {
        self.flags[7] != 0
    }
    /// `+0x61`：yaw 绕一圈（见 [`CamPlayer`]）。
    pub fn wraps_yaw(&self) -> bool {
        self.flags[9] != 0
    }
    /// `+0x59`：「匀速」那一支；为 0 时用记录里的速度 `+0x6c`（见 [`CamPlayer`]）。
    pub fn uniform(&self) -> bool {
        self.flags[1] != 0
    }
    /// `+0x5d`：这一步的第二个动作（`FUN_005075e0`）。只有 `+0x59 && +0x5b && !+0x5a`
    /// 时才真的做（绕圈），其余直接返回。
    pub fn orbits(&self) -> bool {
        self.flags[5] != 0 && self.flags[1] != 0 && self.flags[3] != 0 && self.flags[2] == 0
    }
    /// `+0x5e`：这一步「滑行」（`FUN_00507b20`）——速度大小不变，方向每帧朝本步的方向
    /// 拐一点；上一步也是滑行就接着上一步的速度走，几步连起来是一条圆滑的弧。
    /// 阵型面板打开时播的 6 号序列四步全是这样。
    pub fn glides(&self) -> bool {
        self.flags[6] != 0
    }
    /// `+0x65`：一路盯着 [`CamRecord::target`]——朝向不插值，每帧由
    /// `FUN_0041c900` 从镜头位置朝目标算。
    pub fn looks_at_target(&self) -> bool {
        self.flags[0xd] != 0
    }

    fn parse(b: &[u8]) -> Self {
        let f = |o: usize| f32::from_le_bytes(b[o..o + 4].try_into().unwrap());
        let pose = |o: usize| CombatCamera {
            x: f(o),
            y: f(o + 4),
            z: f(o + 8),
            yaw: f(o + 12),
            pitch: f(o + 16),
            roll: f(o + 20),
        };
        let mut flags = [0u8; 16];
        flags.copy_from_slice(&b[0x58..0x68]);
        Self {
            duration: f(0x1c),
            from: pose(0x20),
            to: pose(0x38),
            flags,
            target: i32::from_le_bytes(b[0x68..0x6c].try_into().unwrap()),
            accel_start: [f(0x74), f(0x78)],
            accel: [f(0x7c), f(0x80)],
            extra: f(0xac),
            speed: f(0x6c),
            orbit_total: f(0x84),
            orbit_speed: f(0x88),
            orbit_center: [f(0x8c), f(0x90), f(0x94)],
            orbit_height: f(0x9c),
        }
    }

    /// 两段加速度在 `t` 秒时贡献的路程（`FUN_00506340` 里那两个 `if`）。
    fn accel_distance(&self, t: f32) -> f32 {
        let mut d = 0.;
        if self.accel_start[0] <= t {
            let s = t - self.accel_start[0];
            d += s * self.accel[0] * s * 0.5;
        }
        if self.accel_start[1] <= t {
            let s = t - self.accel_start[1];
            d += (self.accel[1] + self.accel[0]) * s * s * 0.5;
        }
        d
    }
}

/// 整个 `flexcam.dat`。
#[derive(Debug, Clone)]
pub struct Flexcam {
    pub records: Vec<CamRecord>,
    /// 每个序列的记录号，按步排好（-1 之后的不要）。
    pub sequences: Vec<Vec<usize>>,
}

impl Flexcam {
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < FILE_SIZE {
            return None;
        }
        let records = (0..RECORDS)
            .map(|i| CamRecord::parse(&bytes[i * RECORD_SIZE..(i + 1) * RECORD_SIZE]))
            .collect();
        let sequences = (0..SEQUENCES)
            .map(|s| {
                (0..STEPS)
                    .map(|k| {
                        let o = TABLE_OFFSET + (s * STEPS + k) * 4;
                        i32::from_le_bytes(bytes[o..o + 4].try_into().unwrap())
                    })
                    // FUN_00505290：数到第一个 -1 为止。
                    .take_while(|&r| r != -1)
                    .filter_map(|r| usize::try_from(r).ok().filter(|&r| r < RECORDS))
                    .collect()
            })
            .collect();
        Some(Self { records, sequences })
    }

    pub fn sequence(&self, seq: usize) -> Vec<CamRecord> {
        self.sequences
            .get(seq)
            .map(|steps| steps.iter().map(|&r| self.records[r]).collect())
            .unwrap_or_default()
    }
}

/// 播一个镜头序列。
#[derive(Debug, Clone)]
pub struct CamPlayer {
    steps: Vec<CamRecord>,
    step: usize,
    /// 这一步开始了多久。
    t: f32,
    /// 这一步开始时的机位（`+0x69008`），和位移的总长、速度。
    start: Option<(CombatCamera, f32, f32)>,
    done: bool,
    pub pose: CombatCamera,
    /// 滑行的当前速度（`+0x68fe0`）和这一步的方向速度（`+0x68fec`）。
    velocity: Option<[f32; 3]>,
    glide_target: [f32; 3],
}

impl CamPlayer {
    /// 从 `current` 这个机位开始播 `steps`。空序列返回 `None`
    /// （`FUN_00505770` 数出 0 步就不播）。
    pub fn new(steps: Vec<CamRecord>, current: CombatCamera) -> Option<Self> {
        (!steps.is_empty()).then_some(Self {
            steps,
            step: 0,
            t: 0.,
            start: None,
            done: false,
            pose: current,
            velocity: None,
            glide_target: [0.; 3],
        })
    }

    /// `FUN_00505800`：停在当前机位。序列号清成 −1，所以之后也不再盯人。
    pub fn stop(&mut self) {
        self.steps.clear();
        self.step = 0;
        self.start = None;
        self.velocity = None;
    }

    /// 步数用完、镜头停下了。
    pub fn finished(&self) -> bool {
        self.step >= self.steps.len()
    }

    /// 这一步要不要盯着谁：`Some(战斗位)`，-1 = 战场中心。停下后沿用最后一步的。
    pub fn look_target(&self) -> Option<i32> {
        let rec = self.steps.get(self.step).or_else(|| self.steps.last())?;
        rec.looks_at_target().then_some(rec.target)
    }

    /// 往前走 `dt` 秒，返回这一帧的机位。
    pub fn update(&mut self, dt: f32) -> CombatCamera {
        // 上一帧这一步完了：换下一步（FUN_005061a0 的 else 分支）。
        if self.done {
            self.done = false;
            self.step += 1;
            self.start = None;
            self.t = 0.;
        }
        let Some(rec) = self.steps.get(self.step).copied() else {
            return self.pose;
        };
        // `FUN_005061a0`：`+0x5c` / `+0x5d` / `+0x5e` 三个动作谁开着就跑谁，步子只在某个
        // 动作置了「完成」（`+0x16`）后才换。实机数据每条只开一个。
        if rec.moves_camera() {
            if !rec.uniform() {
                return self.drift(rec, dt);
            }
        } else if rec.glides() {
            return self.glide(rec, dt);
        } else if rec.orbits() {
            return self.orbit(rec, dt);
        } else {
            // 没有能跑的动作（比如 1 号序列第一步只开了 `+0x5d`、却不满足绕圈的条件，
            // `FUN_005075e0` 直接返回）：这一步永远不完，镜头停在原地。
            return self.pose;
        }

        // 这一步第一帧：记下起点、总长、速度。
        let (from, dist, speed) = *self.start.get_or_insert_with(|| {
            let mut from = if rec.keeps_from() {
                rec.from
            } else {
                self.pose
            };
            if rec.wraps_yaw() {
                // 原样照抄：目标 yaw 大就把起点再加一圈，小就减一圈。
                let d = rec.to.yaw - from.yaw;
                if d > 0. {
                    from.yaw += std::f32::consts::TAU;
                } else if d < 0. {
                    from.yaw -= std::f32::consts::TAU;
                }
            }
            let dist = distance(&from, &rec.to);
            let speed = if rec.duration == 0. {
                0.
            } else {
                let d0 = rec.duration - rec.accel_start[0];
                let d1 = rec.duration - rec.accel_start[1];
                ((dist
                    - (d0 * d0 * rec.accel[0] * 0.5
                        + (rec.accel[1] + rec.accel[0]) * d1 * d1 * 0.5))
                    / rec.duration)
                    .max(0.)
            };
            (from, dist, speed)
        });

        self.t += dt;
        let t = self.t;
        let frac = if rec.duration <= 0. {
            1.
        } else {
            (t / rec.duration).min(1.)
        };
        let moved = (t * speed + rec.accel_distance(t)).min(dist);
        let (ux, uy, uz) = if dist > 0. {
            (
                (rec.to.x - from.x) / dist,
                (rec.to.y - from.y) / dist,
                (rec.to.z - from.z) / dist,
            )
        } else {
            (0., 0., 0.)
        };
        let lerp = |a: f32, b: f32| a + (b - a) * frac;
        self.pose = CombatCamera {
            x: from.x + ux * moved,
            y: from.y + uy * moved,
            z: from.z + uz * moved,
            yaw: lerp(from.yaw, rec.to.yaw),
            pitch: lerp(from.pitch, rec.to.pitch),
            roll: lerp(from.roll, rec.to.roll),
        };
        if rec.duration == 0. && dist > 0. {
            self.pose = rec.to;
            self.done = true;
        }
        if frac >= 1. {
            self.done = true;
        }
        if moved == dist && rec.stops_at_distance() {
            self.done = true;
        }
        self.pose
    }
}

impl CamPlayer {
    /// 这一步的起点：记录里的，或者镜头当前的（`FUN_005062d0`）；`+0x61` 时 yaw 加减一圈。
    fn step_start(&self, rec: &CamRecord) -> CombatCamera {
        let mut from = if rec.keeps_from() {
            rec.from
        } else {
            self.pose
        };
        if rec.wraps_yaw() {
            let d = rec.to.yaw - from.yaw;
            if d > 0. {
                from.yaw += std::f32::consts::TAU;
            } else if d < 0. {
                from.yaw -= std::f32::consts::TAU;
            }
        }
        from
    }

    /// `FUN_00506340` 的 `+0x59 == 0` 那一支：位置 = 起点 + 方向 × (t·速度 + 两段加速度)，
    /// **不夹到终点**；角度按 t / 时长插。实机三条（70 / 75 / 78 号序列）速度都是 0、起止
    /// 同一点，效果就是原地转到记录里的角度。
    fn drift(&mut self, rec: CamRecord, dt: f32) -> CombatCamera {
        if self.start.is_none() {
            let from = self.step_start(&rec);
            self.start = Some((from, distance(&from, &rec.to), rec.speed));
        }
        let (from, dist, speed) = self.start.unwrap_or((self.pose, 0., 0.));
        self.t += dt;
        let t = self.t;
        let frac = if rec.duration > 0. {
            t / rec.duration
        } else {
            1.
        };
        let moved = t * speed + rec.accel_distance(t);
        let (ux, uy, uz) = if dist > 0. {
            (
                (rec.to.x - from.x) / dist,
                (rec.to.y - from.y) / dist,
                (rec.to.z - from.z) / dist,
            )
        } else {
            (0., 0., 0.)
        };
        let lerp = |a: f32, b: f32| a + (b - a) * frac.min(1.);
        self.pose = CombatCamera {
            x: from.x + ux * moved,
            y: from.y + uy * moved,
            z: from.z + uz * moved,
            yaw: lerp(from.yaw, rec.to.yaw),
            pitch: lerp(from.pitch, rec.to.pitch),
            roll: lerp(from.roll, rec.to.roll),
        };
        if frac >= 1. {
            self.done = true;
        }
        self.pose
    }

    /// `FUN_005075e0` 的绕圈（`+0x59 && +0x5b && !+0x5a`，77 / 80 号序列）：转过的角度
    /// a = t·`+0x88` + 两段加速度，夹到总角度 `+0x84`（按正负号）；起点绕圆心按 a 转，
    /// 高度按 a / 总角度 从 `+0x90` 往 `+0x9c` 挪；角度保持起点的（这两条都盯着目标，朝向由
    /// 舞台每帧算）。转满算完。
    fn orbit(&mut self, rec: CamRecord, dt: f32) -> CombatCamera {
        if self.start.is_none() {
            let from = self.step_start(&rec);
            self.start = Some((from, 0., 0.));
        }
        let (from, _, _) = self.start.unwrap_or((self.pose, 0., 0.));
        self.t += dt;
        let total = rec.orbit_total;
        let mut a = self.t * rec.orbit_speed + rec.accel_distance(self.t);
        let done = if total <= 0. {
            if a < total {
                a = total;
            }
            a <= total
        } else {
            if a > total {
                a = total;
            }
            total <= a
        };
        let [cx, cy, cz] = rec.orbit_center;
        let (s, c) = a.to_radians().sin_cos();
        let (ox, oz) = (from.x - cx, from.z - cz);
        let rise = if total != 0. {
            a / total * (rec.orbit_height - cy)
        } else {
            0.
        };
        // gbengine：`SetYawPitchRoll(a,0,0)` 得绕 Y 的四元数，`GetMatrix` 出行主序矩阵，
        // `RotateIn` 做 v·M——x' = x·cos a + z·sin a，z' = −x·sin a + z·cos a。移植端机位就是
        // 原版坐标，照搬。
        self.pose = CombatCamera {
            x: cx + ox * c + oz * s,
            y: from.y + rise,
            z: cz - ox * s + oz * c,
            ..from
        };
        if done {
            self.done = true;
        }
        self.pose
    }

    /// `FUN_00507b20`：滑行一步。
    fn glide(&mut self, rec: CamRecord, dt: f32) -> CombatCamera {
        let prev_glides =
            self.step > 0 && self.steps.get(self.step - 1).is_some_and(|r| r.glides());
        if self.start.is_none() {
            let mut from = if rec.keeps_from() {
                rec.from
            } else {
                self.pose
            };
            if rec.wraps_yaw() {
                let d = rec.to.yaw - from.yaw;
                if d > 0. {
                    from.yaw += std::f32::consts::TAU;
                } else if d < 0. {
                    from.yaw -= std::f32::consts::TAU;
                }
            }
            let k = if rec.duration > 0. {
                1. / rec.duration
            } else {
                1.
            };
            self.glide_target = [
                (rec.to.x - from.x) * k,
                (rec.to.y - from.y) * k,
                (rec.to.z - from.z) * k,
            ];
            // 第一步、或上一步不是滑行：起始速度就是这一步的方向速度（0x507bf0）。
            if !prev_glides || self.velocity.is_none() {
                self.velocity = Some(self.glide_target);
            }
            self.pose = from;
            self.start = Some((from, 0., 0.));
        }
        let (from, _, _) = self.start.unwrap_or((self.pose, 0., 0.));
        self.t += dt;
        let frac = if rec.duration > 0. {
            self.t / (rec.extra + rec.duration)
        } else {
            1.
        };
        // 速度大小不变，方向朝本步的方向拐：v = |v| · normalize(v + dt·目标)。
        let v = self.velocity.unwrap_or(self.glide_target);
        let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let mut nv = [
            v[0] + dt * self.glide_target[0],
            v[1] + dt * self.glide_target[1],
            v[2] + dt * self.glide_target[2],
        ];
        let nl = (nv[0] * nv[0] + nv[1] * nv[1] + nv[2] * nv[2]).sqrt();
        if nl > 0. {
            for c in &mut nv {
                *c *= len / nl;
            }
        }
        self.velocity = Some(nv);
        let lerp = |a: f32, b: f32| a + (b - a) * frac.min(1.);
        self.pose = CombatCamera {
            x: self.pose.x + dt * nv[0],
            y: self.pose.y + dt * nv[1],
            z: self.pose.z + dt * nv[2],
            yaw: lerp(from.yaw, rec.to.yaw),
            pitch: lerp(from.pitch, rec.to.pitch),
            roll: lerp(from.roll, rec.to.roll),
        };
        if frac >= 1. {
            self.done = true;
        }
        self.pose
    }
}

fn distance(a: &CombatCamera, b: &CombatCamera) -> f32 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2) + (b.z - a.z).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(x: f32, yaw: f32) -> CombatCamera {
        CombatCamera {
            x,
            y: 0.,
            z: 0.,
            yaw,
            pitch: 0.,
            roll: 0.,
        }
    }

    fn rec(duration: f32, from: CombatCamera, to: CombatCamera) -> CamRecord {
        let mut flags = [0u8; 16];
        flags[0] = 1; // keeps_from
        flags[1] = 1; // 匀速分支
        flags[4] = 1; // moves_camera
        CamRecord {
            duration,
            from,
            to,
            flags,
            target: 0,
            accel_start: [0., 0.],
            accel: [0., 0.],
            extra: 0.,
            speed: 0.,
            orbit_total: 0.,
            orbit_speed: 0.,
            orbit_center: [0.; 3],
            orbit_height: 0.,
        }
    }

    #[test]
    fn a_zero_speed_drift_only_turns_in_place() {
        let mut r = rec(1., pose(0., 0.), pose(100., 1.));
        r.flags[1] = 0; // +0x59 == 0
        let mut p = CamPlayer::new(vec![r], pose(0., 0.)).unwrap();
        let mut last = p.pose;
        for _ in 0..30 {
            last = p.update(0.05);
        }
        assert_eq!(last.x, 0., "速度 0 不挪位置");
        assert!((last.yaw - 1.).abs() < 1e-4, "角度转到记录里的");
        assert!(p.finished() || p.done);
    }

    #[test]
    fn an_orbit_step_turns_the_start_about_the_centre() {
        let mut r = rec(1., pose(10., 0.), pose(10., 0.));
        r.flags = [0; 16];
        r.flags[0] = 1; // 起点用记录里的
        r.flags[1] = 1; // +0x59
        r.flags[3] = 1; // +0x5b
        r.flags[5] = 1; // +0x5d
        r.orbit_total = 180.;
        r.orbit_speed = 180.;
        r.orbit_height = 20.;
        let mut p = CamPlayer::new(vec![r], pose(0., 0.)).unwrap();
        let mut last = p.pose;
        for _ in 0..30 {
            last = p.update(0.05);
        }
        assert!((last.x + 10.).abs() < 1e-3, "转半圈到对面: {}", last.x);
        assert!((last.y - 20.).abs() < 1e-3, "高度挪到 +0x9c");
    }

    #[test]
    fn an_orbit_turns_the_same_way_as_gbengine() {
        let mut r = rec(1., pose(10., 0.), pose(10., 0.));
        r.flags = [0; 16];
        r.flags[0] = 1;
        r.flags[1] = 1;
        r.flags[3] = 1;
        r.flags[5] = 1;
        r.orbit_total = 90.;
        r.orbit_speed = 90.;
        let mut p = CamPlayer::new(vec![r], pose(0., 0.)).unwrap();
        let mut last = p.pose;
        for _ in 0..30 {
            last = p.update(0.05);
        }
        // (10, 0) 转 +90°：x' = 10·cos + 0 = 0，z' = −10·sin = −10。
        assert!(
            last.x.abs() < 1e-3 && (last.z + 10.).abs() < 1e-3,
            "{last:?}"
        );
    }

    #[test]
    fn a_step_with_nothing_to_run_never_ends() {
        let mut r = rec(1., pose(0., 0.), pose(100., 0.));
        r.flags = [0; 16];
        r.flags[5] = 1; // 只开了 +0x5d，却不满足绕圈的条件
        let mut p = CamPlayer::new(vec![r], pose(5., 0.)).unwrap();
        for _ in 0..100 {
            p.update(0.05);
        }
        assert!(!p.finished(), "FUN_005061a0 永远等不到这一步完成");
        assert_eq!(p.pose.x, 5.);
    }

    fn glide_rec(duration: f32, to: CombatCamera) -> CamRecord {
        let mut r = rec(duration, pose(0., 0.), to);
        r.flags = [0; 16];
        r.flags[6] = 1; // +0x5e 滑行，+0x5c 不置
        r
    }

    #[test]
    fn a_first_glide_step_runs_straight_at_constant_speed() {
        let mut p = CamPlayer::new(vec![glide_rec(1., pose(100., 0.))], pose(0., 0.)).unwrap();
        let mut x = 0.;
        for _ in 0..10 {
            x = p.update(0.05).x;
        }
        assert!((x - 50.).abs() < 1., "半秒走一半: {x}");
    }

    #[test]
    fn a_second_glide_step_keeps_the_speed_and_bends_toward_its_way() {
        let mut to2 = pose(100., 0.);
        to2.z = 100.;
        let steps = vec![glide_rec(1., pose(100., 0.)), glide_rec(1., to2)];
        let mut p = CamPlayer::new(steps, pose(0., 0.)).unwrap();
        while p.step == 0 || p.start.is_none() {
            p.update(0.05);
        }
        let v = p.velocity.unwrap();
        let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        assert!((speed - 100.).abs() < 1., "速度大小接着上一步: {speed}");
        assert!(v[0] > v[2], "刚拐，方向还主要朝 x");
    }

    #[test]
    fn the_file_layout_adds_up() {
        assert_eq!(FILE_SIZE, 430000, "FUN_00504f40 读 0x639c0 + 2000 + 20000");
        assert_eq!(RECORDS * RECORD_SIZE, 0x639c0);
    }

    #[test]
    fn a_step_moves_at_constant_speed_and_lerps_the_angles() {
        let mut p =
            CamPlayer::new(vec![rec(1., pose(0., 0.), pose(100., 1.))], pose(-5., 0.)).unwrap();
        let a = p.update(0.25);
        assert!((a.x - 25.).abs() < 1e-3 && (a.yaw - 0.25).abs() < 1e-5);
        let b = p.update(0.5);
        assert!((b.x - 75.).abs() < 1e-3);
        let c = p.update(0.5); // 超时：到终点、算完
        assert!((c.x - 100.).abs() < 1e-3 && (c.yaw - 1.).abs() < 1e-5);
        assert!(!p.finished(), "下一帧才换步");
        p.update(0.1);
        assert!(p.finished());
        // 停住以后一直是最后的机位。
        assert_eq!(p.update(1.).x, 100.);
    }

    #[test]
    fn a_zero_length_step_jumps_to_its_end() {
        let steps = vec![
            rec(0., pose(0., 0.), pose(50., 2.)),
            rec(1., pose(50., 2.), pose(50., 2.)),
        ];
        let mut p = CamPlayer::new(steps, pose(0., 0.)).unwrap();
        assert_eq!(p.update(0.01).x, 50.);
        assert_eq!(p.update(0.01).x, 50., "第二步原地待着");
    }

    #[test]
    fn without_keeps_from_the_step_starts_where_the_camera_is() {
        let mut r = rec(1., pose(999., 0.), pose(100., 0.));
        r.flags[0] = 0;
        let mut p = CamPlayer::new(vec![r], pose(0., 0.)).unwrap();
        assert!((p.update(0.5).x - 50.).abs() < 1e-3);
    }

    #[test]
    fn each_ending_has_its_own_sequence() {
        use crate::openpal3::combat::state::Outcome;
        assert_eq!(ending_sequence(Outcome::Win), Some(3));
        assert_eq!(ending_sequence(Outcome::Lose), Some(10));
        assert_eq!(ending_sequence(Outcome::Escaped), Some(8));
        assert_eq!(ending_sequence(Outcome::Ongoing), None);
    }

    #[test]
    fn a_look_at_step_reports_its_target_even_after_it_stops() {
        let mut r = rec(1., pose(0., 0.), pose(10., 0.));
        r.flags[0xd] = 1;
        r.target = 0;
        let mut p = CamPlayer::new(vec![r], pose(0., 0.)).unwrap();
        assert_eq!(p.look_target(), Some(0));
        p.update(2.);
        p.update(0.1);
        assert!(p.finished());
        assert_eq!(p.look_target(), Some(0), "停住后还盯着");
        let plain =
            CamPlayer::new(vec![rec(1., pose(0., 0.), pose(1., 0.))], pose(0., 0.)).unwrap();
        assert_eq!(plain.look_target(), None);
    }

    #[test]
    fn an_empty_sequence_does_not_play() {
        assert!(CamPlayer::new(vec![], pose(0., 0.)).is_none());
    }

    #[test]
    fn parsing_reads_the_fields_where_the_player_reads_them() {
        let mut bytes = vec![0u8; FILE_SIZE];
        let put = |b: &mut Vec<u8>, o: usize, v: f32| b[o..o + 4].copy_from_slice(&v.to_le_bytes());
        put(&mut bytes, 0x1c, 0.8);
        put(&mut bytes, 0x20, -32.5);
        put(&mut bytes, 0x38, -78.4);
        put(&mut bytes, 0x44, 3.107);
        bytes[0x58] = 1;
        bytes[0x5c] = 1;
        // 序列 0 = [0]，其余全 -1。
        for k in 0..SEQUENCES * STEPS {
            let o = TABLE_OFFSET + k * 4;
            bytes[o..o + 4].copy_from_slice(&(-1i32).to_le_bytes());
        }
        bytes[TABLE_OFFSET..TABLE_OFFSET + 4].copy_from_slice(&0i32.to_le_bytes());
        let f = Flexcam::parse(&bytes).unwrap();
        assert_eq!(f.sequences[0], vec![0]);
        assert!(f.sequences[1].is_empty());
        let r = f.records[0];
        assert_eq!(
            (r.duration, r.from.x, r.to.x, r.to.yaw),
            (0.8, -32.5, -78.4, 3.107)
        );
        assert!(r.keeps_from() && r.moves_camera());
        assert!(Flexcam::parse(&bytes[..FILE_SIZE - 1]).is_none());
    }
}

/// 镜头震动（`FUN_005054f0`，镜头每帧更新里的那一段；参数是镜头对象 `+0x68ffc` 幅度、
/// `+0x69000` 每秒衰减、`+0x69004` 频率）。
///
/// 每帧：幅度 −= dt × 衰减；幅度还大于 0 时，上下偏移 = sin(总时间 × 频率) × 幅度，临时加在
/// 镜头位姿的 y（`+0x69024`）上，摆完镜头再还原——所以不会累积。特效 `quake` 类
/// （0x4d35e0）写的是 幅度 = 子块 `+0`、衰减 = `+4`、频率 = `+8`；战斗调试对话框那处写
/// (10, 10, 90)。sin 里的时间是战斗时钟（战斗系统 `+0x50`，镜头构造 `FUN_005052e0` 存下的
/// `+0x68fb0` 就指向战斗系统），不是震动自己的钟，由调用方传进来。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shake {
    pub amplitude: f32,
    pub decay: f32,
    pub frequency: f32,
}

impl Shake {
    pub fn new(amplitude: f32, decay: f32, frequency: f32) -> Self {
        Self {
            amplitude,
            decay,
            frequency,
        }
    }

    /// 震完了（幅度降到 0）。
    pub fn finished(&self) -> bool {
        self.amplitude <= 0.
    }

    /// 往前走 `dt` 秒，返回这一帧镜头 y 的偏移；`clock` 是战斗时钟。
    pub fn step(&mut self, dt: f32, clock: f32) -> f32 {
        if self.amplitude <= 0. {
            self.amplitude = 0.;
            return 0.;
        }
        self.amplitude -= dt * self.decay;
        if self.amplitude <= 0. {
            self.amplitude = 0.;
            return 0.;
        }
        (clock * self.frequency).sin() * self.amplitude
    }
}

#[cfg(test)]
mod shake_tests {
    use super::Shake;

    #[test]
    fn it_decays_to_nothing() {
        // 幅度 10、每秒衰减 10：一秒后震完。
        let mut s = Shake::new(10., 10., 90.);
        let mut peak = 0f32;
        for i in 0..50 {
            peak = peak.max(s.step(0.01, i as f32 * 0.01).abs());
        }
        assert!(peak <= 10. && peak > 1., "{peak}");
        for _ in 0..60 {
            s.step(0.01, 0.);
        }
        assert!(s.finished());
        assert_eq!(s.step(0.01, 0.), 0.);
    }

    #[test]
    fn the_offset_follows_the_sine() {
        let mut s = Shake::new(10., 0., 1.);
        // 相位跟战斗时钟走，不是从 0 起。
        let off = s.step(0.5, 3.0);
        assert!((off - 3f32.sin() * 10.).abs() < 1e-5);
    }
}
