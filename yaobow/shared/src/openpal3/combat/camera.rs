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
//! 移镜头只照 `FUN_00506340` 的「匀速」分支（`+0x59 != 0 && +0x5b == 0`，0 号和
//! 20 号序列都是它）做了：位置沿起点→终点按速度走，角度按时间比例插。另外两个
//! 分支、跟随目标（`+0x66`）、盯着目标（`+0x65`）、`+0x5d`/`+0x5e` 两个附带动作都
//! 没做，碰到了一律按匀速分支走 **[推断]**。逆向记录在 `switch/re/notes/combat_camera.md`。

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
        })
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
        if !rec.moves_camera() {
            // 这一步不动镜头 [推断]：没有别的附带动作可做，直接算完。
            self.done = true;
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
        }
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
