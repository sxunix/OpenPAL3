//! 滑雪滑道本体（SCE 107 / 模式 11）。
//!
//! 规则照 `PAL3_unpacked.exe` 写：雪球每帧的更新是 `FUN_00531260`，三个
//! 状态处理函数是 `FUN_00530f70`（撞后减速）、`FUN_00531010`（跳跃）、
//! `FUN_005310c0`（收尾），摆放在 `FUN_0052ed30`，结算在 `FUN_00532410`。
//! 逆向记录在 `switch/re/notes/ski.md`。
//!
//! 一句话说清这个游戏：景天（角色 101）踩着剑沿 `Skee_path.ASE` 那条固定路径往下滑（源码
//! `subgame/Skee/Skee_slider.cpp`，下面说的「雪球」「球」都是指他——这些注释最早按 exe 读，以为滑的
//! 是个球），**没有左右操控**，玩家只有两个动作——按住左键加速、点右键跳。过了半程之后背后会有一个
//! 雪球以同样的巡航速度追上来，被追上就算没滑完。路上摆了六个障碍（撞上就减速到停），三件收集品悬在
//! 跳起来才够得着的高度。碰撞全是不转的盒子相交（[`Aabb`]）。

use super::ski_path::SkiPath;

/// 引擎把平滑过的帧时间乘上这个数再交给滑雪的每帧更新
/// （`FUN_0052f350` 里的 `_DAT_005f4b9c`）。也就是说下面这些速度、加速度
/// 都是「每 1/46.68 秒」的量，不是每秒。
pub const DT_SCALE: f32 = 46.68;

/// 巡航速度，也是不加速时的速度上限（`+0x29c`，初值就等于它）。
pub const CRUISE_SPEED: f32 = 10.86;

/// 按住左键能冲到的上限（`+0x2a0`）。
pub const BOOST_MAX: f32 = 28.86;

/// 按住左键每帧加多少（`LAB_00531857` 那句 `+ 0.2`，**不乘 dt**）。
pub const BOOST_STEP: f32 = 0.2;

/// 没到巡航速度时每帧补多少（`+0x318`）。撞完之后靠它爬回来。
pub const ACCEL: f32 = 0.12;

/// 收尾状态每单位 dt 掉多少速度（`FUN_005310c0` 里的 `0.5`）。
pub const STOP_RATE: f32 = 0.5;

/// 起跳初速（`+0x32c`）。
pub const JUMP_SPEED: f32 = 6.0;

/// 下落时每帧给起跳速度加多少（`+0x330`）。
pub const GRAVITY: f32 = 0.03;

/// 追上来那个雪球的速度（`+0xe94`，和巡航速度同值）。
pub const CHASER_SPEED: f32 = CRUISE_SPEED;

/// 过半程之后等多久才放出追来的雪球（`m_WaitBall.Wait(300)`，毫秒）。
pub const CHASER_DELAY_SECS: f32 = 0.3;

/// 倒计时初值，秒（`+0x11a0 = 0x3b`）。
pub const TIME_LIMIT_SECS: i32 = 59;

/// 每捡到一件多少分（`FUN_00532410`）。
pub const SCORE_PER_PICKUP: i32 = 100;

/// 剩下一秒多少分。
pub const SCORE_PER_SECOND: i32 = 30;

/// 障碍个数（`obstruct0..5.POL`，碰撞循环 `iVar15 < 6`）。
pub const OBSTACLE_COUNT: usize = 6;

/// 收集品个数（`skee_sav.sv` 三行，碰撞循环 `iVar8 < 3`）。
pub const TREASURE_COUNT: usize = 3;

/// 第一个障碍摆在第几个路径点（`iVar5 = iVar13 + 300`）。
pub const FIRST_OBSTACLE_INDEX: usize = 300;

/// 算障碍间距时从总点数里减掉多少（`(N - 0x618) / 6`）。
pub const OBSTACLE_MARGIN: usize = 0x618;

/// 收集品下标的偏移（`rand15() % N + 100`）。
pub const FIRST_TREASURE_INDEX: usize = 100;

/// 六个障碍摆在哪几个路径点上——**固定的，不随机**。
///
/// `FUN_0052ed30`：`step = (N - 1560) / 6`，下标从 300 开始每次加 `step`。
/// 实机 N = 4950，于是 step = 565，六个下标是 300 / 865 / 1430 / 1995 /
/// 2560 / 3125。
pub fn obstacle_indices(points: usize) -> [usize; OBSTACLE_COUNT] {
    // 原版是有符号除法；点数少于 1560 时会得到负的间距，这里照着走。
    let step = (points as i32 - OBSTACLE_MARGIN as i32) / OBSTACLE_COUNT as i32;
    let mut out = [0usize; OBSTACLE_COUNT];
    let mut idx = FIRST_OBSTACLE_INDEX as i32;
    for slot in out.iter_mut() {
        *slot = idx.max(0) as usize;
        idx += step;
    }
    out
}

/// 收集品摆在哪个路径点上——**这个是随机的**。
///
/// `FUN_0052ed30`：`(rand15() % N + 100)`。注意原版没有再夹一次，下标最大
/// 能到 `N + 99`，越界读数组；移植端夹在最后一个点上，这一处和原版不同，
/// 是为了不越界。
pub fn treasure_index(rand: u32, points: usize) -> usize {
    if points == 0 {
        return 0;
    }
    let idx = rand as usize % points + FIRST_TREASURE_INDEX;
    idx.min(points - 1)
}

/// 收集品比脚下的路径点高多少：`r + r / 3`，其中 `r` 是**取整之后**的球半径
/// （`__ftol(ball->+0x1e4)`，整数除法）。
pub fn treasure_lift(radius: f32) -> f32 {
    let r = radius as i32;
    (r + r / 3) as f32
}

/// 跳能跳多高（`+0x320 = +0x1e4 × 4/3`，这个是浮点算的）。
pub fn jump_ceiling(radius: f32) -> f32 {
    radius * (1. / 3.) + radius
}

/// 雪球的状态（`+0x310`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// 0，正常滚。
    Rolling,
    /// 1，撞到障碍之后掉速（`FUN_00530f70`）。
    Hit,
    /// 2，跳在空中（`FUN_00531010`）。
    Jumping,
    /// 3，收尾，速度归零后结算（`FUN_005310c0`）。
    Stopping,
}

/// 一局怎么结束的。原版存的是 `+0x302 = (传给 FUN_00530c00 的参数 == 0)`，
/// 只有 `Cleared` 那一条会真的把收集品塞进背包。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// 路径滚到头（`FUN_0052dc40` 返回空）→ `FUN_00530c00(0)`。
    Cleared,
    /// 被后面那个雪球追上（`FUN_00530f20`）→ `FUN_00530c00(1)`。
    Caught,
    /// 倒计时归零 → 同样是 `FUN_00530c00(1)`。
    TimedOut,
}

impl Outcome {
    /// 原版的 `+0x302`：只有它为真、且还剩至少一秒，结算才发东西。
    pub fn pays_out(self) -> bool {
        self == Outcome::Cleared
    }
}

/// 玩家这一帧按了什么。原版读的是鼠标：`DAT_00bfc5f4 == 1` 是左键按住，
/// `(DAT_00bfc5f8 & 0xf) == 1` 是右键按下。
#[derive(Debug, Clone, Copy, Default)]
pub struct SkiInput {
    /// 左键按住——加速。
    pub boost: bool,
    /// 右键按下这一帧——跳。
    pub jump: bool,
}

/// 路上的一个障碍。
#[derive(Debug, Clone, Copy)]
pub struct Obstacle {
    /// 摆在路径的哪一段距离上。
    pub distance: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// 路上的一件收集品。
#[derive(Debug, Clone, Copy)]
pub struct Treasure {
    pub id: u32,
    pub distance: f32,
    pub x: f32,
    pub y: f32,
    pub z: f32,
    /// 背包里已经有了就不摆出来（`FUN_0052ece0` 那一步）。
    pub present: bool,
}

/// 不转的碰撞盒（`gbOBBox`，滑雪里的盒子都只平移不转）：中心相对物体原点的偏移、三边**全长**
/// （`engine/physics/gbcollide.h`：`width = xlen`）。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Aabb {
    pub center: [f32; 3],
    pub size: [f32; 3],
}

impl Aabb {
    pub fn from_min_max(min: [f32; 3], max: [f32; 3]) -> Self {
        Self {
            center: [0, 1, 2].map(|k| (min[k] + max[k]) * 0.5),
            size: [0, 1, 2].map(|k| max[k] - min[k]),
        }
    }

    /// `gbOBBox::IsColOBB` 对两个不转的盒子：分离轴只剩三根坐标轴（叉乘轴全是零向量，分不开），
    /// 每根轴上中心距不超过两边全长之和的一半就是碰上。`at` 是各自物体的位置。
    pub fn hits(&self, at: (f32, f32, f32), other: &Aabb, other_at: (f32, f32, f32)) -> bool {
        let a = [at.0, at.1, at.2];
        let b = [other_at.0, other_at.1, other_at.2];
        (0..3).all(|k| {
            let d = (other.center[k] + b[k]) - (self.center[k] + a[k]);
            d.abs() <= (self.size[k] + other.size[k]) * 0.5
        })
    }

    /// 最长的一条边。
    pub fn longest(&self) -> f32 {
        self.size[0].max(self.size[1]).max(self.size[2])
    }

    /// `gbOBBox::GetRadius`：半对角线。
    pub fn radius(&self) -> f32 {
        let [x, y, z] = self.size;
        (x * x + y * y + z * z).sqrt() * 0.5
    }
}

/// 各个模型的碰撞盒，都从模型文件读。
#[derive(Debug, Clone, Copy)]
pub struct Shapes {
    /// 开局时人物碰撞盒的高 `box.ylen`（`Skee_slider::LoadSlider`：`Init("101")` 之后、还在第一个
    /// 动作 c01 上时量的，exe `FUN_005306a0` 末尾存进 `+0x1e4`）。跳的高度上限 `h × 4/3`、古董抬高
    /// `h + h/3` 都按它。
    pub man_height: f32,
    /// 人物的碰撞盒是**当前动作**的包围盒（`gbMVActorDef::GetCollideOBB` = 动作的 `GetBoundBox`）：
    /// 滑的时候取 h02，跳的时候取 h05 [推断：随机换上的 h03 / h04、摔完起身的 h01 没分开，盒子差得不多]。
    pub man_slide: Aabb,
    pub man_jump: Aabb,
    /// `obstruct0..5.pol` 的盒子（`FUN_0052f800` 里 `GetOBBox`）。
    pub obstacles: [Aabb; OBSTACLE_COUNT],
    /// 三件古董各自物品模型的盒子（`FUN_0052f8e0`），和 `ids` 对应。
    pub treasures: [Aabb; TREASURE_COUNT],
    /// 追来的雪球 `Ball.pol` 的盒子：追上的判定是两边沿路径的距离差小于它的 `GetRadius`。
    pub ball: Aabb,
}

impl Shapes {
    /// 实机文件量出来的值（人物 c01 / h02 / h05 的 `.mv3` 头、`obstruct0.pol`、`Ball.pol`），装了
    /// 游戏就该从文件读，这里只是兜底；古董没有现成的量，按障碍那个盒子填。
    pub fn shipped() -> Self {
        let obstacle = Aabb {
            center: [0., 0., 0.],
            size: [144.76, 29.82, 55.08],
        };
        Self {
            man_height: 76.038,
            man_slide: Aabb::from_min_max([-7.644, 0.389, -19.256], [22.531, 68.203, 30.775]),
            man_jump: Aabb::from_min_max([-26.393, -1.057, -17.558], [34.992, 120.942, 34.347]),
            obstacles: [obstacle; OBSTACLE_COUNT],
            treasures: [obstacle; TREASURE_COUNT],
            ball: Aabb {
                center: [0., 0., 0.],
                size: [131.24, 131.06, 131.06],
            },
        }
    }
}

/// 一局滑雪。
#[derive(Debug, Clone)]
pub struct SkiRun {
    path: SkiPath,

    /// 雪球位置（`+0x278/27c/280`）。
    pub pos: (f32, f32, f32),
    /// 沿路径已经滚了多远（`+0x290`）。
    pub travelled: f32,
    /// 当前速度（`+0x294`）。
    pub speed: f32,
    /// 当前朝向（`+0x284/288/28c`）。初值就是原版那个
    /// `(0, 0, −0.001)`——第一帧目标点正好是球脚下、差向量为零，靠它把球
    /// 推出去。
    dir: (f32, f32, f32),

    phase: Phase,
    /// 上一个状态（`+0x314`）。
    prev_phase: Phase,

    /// `+0x31c`：撞完之后是在掉速（true）还是在回速（false）。
    braking: bool,
    /// `+0x31d`：回到上限之后锁住，不再动。
    brake_locked: bool,

    /// `+0x31e`：跳跃是不是还在往上。
    rising: bool,
    /// `+0x324`：离地高度。
    jump_height: f32,
    /// `+0x328`：跳跃速度。
    jump_vel: f32,
    /// `+0x334`：跳跃时用的 y。
    jump_y: f32,
    /// `+0x320`：跳的高度上限。
    ceiling: f32,
    /// 各个模型的碰撞盒。
    shapes: Shapes,

    /// `+0x2ac`：滚过半程了没有。加速键和追球都要它。
    started: bool,
    /// `+0xe90`：追上来那个雪球滚了多远。
    pub chaser: f32,
    /// 过半程到放出雪球之间还要等多久（`m_WaitBall.Wait(300)`）。
    chase_wait: Option<f32>,
    /// 雪球已经放到雪道上（`m_isShowBall`）。
    chaser_out: bool,

    obstacles: Vec<Obstacle>,
    treasures: Vec<Treasure>,

    /// `+0x11a0`：剩余秒数。
    pub seconds_left: i32,
    tick: f32,

    /// `+0x50c` / `+0x518`：这一局捡到的。
    picked: Vec<u32>,

    outcome: Option<Outcome>,
}

impl SkiRun {
    /// 开一局。`ids` 是三个能捡的物品号（`FUN_0052f6c0` 里写死的
    /// `0x1783 / 0x178d / 0x1781`），`owned` 说哪几件背包里已经有了（有的
    /// 就不摆），`rand` 给收集品挑摆放点。
    pub fn new(path: SkiPath, shapes: Shapes, ids: &[u32], owned: &[bool], rand: &[u32]) -> Self {
        let radius = shapes.man_height;
        let points = path.points.len();

        let obstacles = obstacle_indices(points)
            .iter()
            .filter_map(|&i| path.points.get(i.min(points.saturating_sub(1))))
            .map(|p| Obstacle {
                distance: p.distance,
                x: p.x,
                y: p.y,
                z: p.z,
            })
            .collect();

        let lift = treasure_lift(radius);
        let treasures = ids
            .iter()
            .take(TREASURE_COUNT)
            .enumerate()
            .filter_map(|(i, &id)| {
                let idx = treasure_index(rand.get(i).copied().unwrap_or(0), points);
                let p = path.points.get(idx)?;
                Some(Treasure {
                    id,
                    distance: p.distance,
                    x: p.x,
                    // 原版把收集品抬到 r + r/3 —— 正好是跳的高度上限，
                    // 所以只有跳起来才够得着。
                    y: p.y + lift,
                    z: p.z,
                    present: !owned.get(i).copied().unwrap_or(false),
                })
            })
            .collect();

        let start = path.points.first().copied().unwrap_or_default();
        Self {
            path,
            pos: (start.x, start.y, start.z),
            travelled: 0.,
            speed: CRUISE_SPEED,
            dir: (0., 0., -0.001),
            phase: Phase::Rolling,
            prev_phase: Phase::Rolling,
            braking: false,
            brake_locked: false,
            rising: false,
            jump_height: 0.,
            jump_vel: JUMP_SPEED,
            jump_y: start.y,
            ceiling: jump_ceiling(radius),
            shapes,
            started: false,
            chaser: 0.,
            chase_wait: None,
            chaser_out: false,
            obstacles,
            treasures,
            seconds_left: TIME_LIMIT_SECS,
            tick: 0.,
            picked: Vec::new(),
            outcome: None,
        }
    }

    pub fn path(&self) -> &SkiPath {
        &self.path
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn obstacles(&self) -> &[Obstacle] {
        &self.obstacles
    }

    pub fn treasures(&self) -> &[Treasure] {
        &self.treasures
    }

    pub fn picked(&self) -> &[u32] {
        &self.picked
    }

    pub fn outcome(&self) -> Option<Outcome> {
        self.outcome
    }

    /// 追球开跑了没有——原版是过半程时置的 `+0x2ac`。
    pub fn chasing(&self) -> bool {
        self.started
    }

    /// 雪球现在的高度：跳在空中用抬起来的那个 y。
    pub fn height(&self) -> f32 {
        if self.phase == Phase::Jumping {
            self.jump_y
        } else {
            self.pos.1
        }
    }

    /// 分数：`件数 × 100 + 剩余秒 × 30`（`FUN_00532410`）。
    pub fn score(&self) -> i32 {
        self.picked.len() as i32 * SCORE_PER_PICKUP + self.seconds_left * SCORE_PER_SECOND
    }

    /// 这一局的收集品到底进不进背包。原版：`剩余秒 ≥ 1 且 +0x302 != 0`，
    /// 再加上进游戏时那个 `+0x304`（上一个模式是 4 就一件不发）。
    pub fn awards(&self) -> bool {
        self.outcome.map(|o| o.pays_out()).unwrap_or(false) && self.seconds_left >= 1
    }

    fn set_phase(&mut self, phase: Phase) {
        self.prev_phase = self.phase;
        self.phase = phase;
    }

    fn finish(&mut self, outcome: Outcome) {
        if self.outcome.is_none() {
            self.outcome = Some(outcome);
            self.set_phase(Phase::Stopping);
        }
    }

    /// 走一帧。`delta_sec` 是真实秒数，里面自己乘 [`DT_SCALE`]。
    pub fn update(&mut self, delta_sec: f32, input: SkiInput) {
        let dt = delta_sec * DT_SCALE;

        self.countdown(delta_sec);

        match self.phase {
            Phase::Hit => self.step_hit(dt),
            Phase::Jumping => self.step_jump(dt),
            Phase::Stopping => self.step_stop(dt),
            Phase::Rolling => {}
        }

        if self.outcome.is_some() && self.phase == Phase::Stopping && self.speed <= 0. {
            return;
        }

        // 沿路径追下一个点。取不到点 = 滚到头了。
        let Some(target) = self.path.at(self.travelled) else {
            self.finish(Outcome::Cleared);
            return;
        };
        if self.travelled >= self.path.length() {
            self.finish(Outcome::Cleared);
            return;
        }

        let raw = (
            target.0 - self.pos.0,
            target.1 - self.pos.1,
            target.2 - self.pos.2,
        );
        // 原版是把差向量写进 `+0x284` 再 `gbVec3D::Normalize`。差向量为零时
        // （第一帧就是，球正站在路径点 0 上）保留上一帧的朝向——起始那个
        // `(0, 0, −0.001)` 就是干这个用的。
        if let Some(unit) = normalize(raw) {
            self.dir = unit;
        }
        let dir = self.dir;
        let before = self.pos;
        self.pos = (
            self.pos.0 + dir.0 * self.speed * dt,
            self.pos.1 + dir.1 * self.speed * dt,
            self.pos.2 + dir.2 * self.speed * dt,
        );
        self.travelled += distance(before, self.pos);

        // 起跳：只有正常滚的时候能跳。
        if input.jump && self.phase == Phase::Rolling {
            self.jump_vel = JUMP_SPEED;
            self.jump_y = self.pos.1;
            self.rising = true;
            self.jump_height = 0.;
            self.set_phase(Phase::Jumping);
        }

        // 加速：正常滚、且过了半程。
        if self.phase == Phase::Rolling && self.started && input.boost {
            self.speed = (self.speed + BOOST_STEP).min(BOOST_MAX);
        }

        // 没到巡航速度就往回补（撞完减到 0 之后靠这个爬起来）。
        if self.phase != Phase::Hit && self.phase != Phase::Stopping && self.speed < CRUISE_SPEED {
            self.speed += ACCEL;
        }

        self.push_off_obstacle();
        if self.phase != Phase::Hit && self.phase != Phase::Stopping {
            self.collide();
        }

        // 追球：过半程时记下滑到的距离（`+0x2b0`），300 毫秒后把雪球放到那里（`FUN_0052fb00`），
        // 从那以后按巡航速度往前滚，追上就完。放出来之前它不动、也追不上。
        if self.started {
            if let Some(wait) = self.chase_wait.as_mut() {
                *wait -= delta_sec;
                if *wait <= 0. {
                    self.chase_wait = None;
                    self.chaser_out = true;
                }
            } else if self.chaser_out {
                self.chaser += CHASER_SPEED * dt;
                if (self.travelled - self.chaser).abs() < self.shapes.ball.radius() {
                    self.finish(Outcome::Caught);
                    return;
                }
            }
        } else if self.travelled >= self.path.length() * 0.5 {
            self.started = true;
            self.chaser = self.travelled;
            self.chase_wait = Some(CHASER_DELAY_SECS);
        }
    }

    /// 追来的雪球放到雪道上了没有（画它用）。
    pub fn chaser_out(&self) -> bool {
        self.chaser_out
    }

    /// `FUN_00530f70`：撞完先掉速，掉到 0 回正常滚。
    fn step_hit(&mut self, dt: f32) {
        if self.brake_locked {
            return;
        }
        let step = dt * ACCEL;
        if self.braking {
            self.speed -= step;
            if self.speed <= 0. {
                self.speed = 0.;
                self.braking = false;
                self.set_phase(Phase::Rolling);
            }
        } else {
            self.speed += step;
            if self.speed >= CRUISE_SPEED {
                self.speed = CRUISE_SPEED;
                self.braking = true;
                self.brake_locked = true;
            }
        }
    }

    /// `FUN_00531010`：上升到 `+0x320` 转下落，落地回正常滚。
    fn step_jump(&mut self, dt: f32) {
        let step = dt * self.jump_vel;
        if self.rising {
            self.jump_height += step;
            self.jump_y = self.jump_height + self.pos.1;
            if self.jump_height >= self.ceiling {
                self.rising = false;
            }
        } else {
            self.jump_y -= step;
            self.jump_vel += GRAVITY;
            if self.jump_y <= self.pos.1 {
                self.jump_y = self.pos.1;
                self.rising = true;
                self.jump_height = 0.;
                self.jump_vel = JUMP_SPEED;
                self.set_phase(Phase::Rolling);
            }
        }
    }

    /// `FUN_005310c0`：收尾，速度按 `dt × 0.5` 掉到 0。
    fn step_stop(&mut self, dt: f32) {
        if self.speed > 0. {
            self.speed -= dt * STOP_RATE;
            if self.speed < 0. {
                self.speed = 0.;
            }
        }
    }

    fn countdown(&mut self, delta_sec: f32) {
        if self.outcome.is_some() {
            return;
        }
        self.tick += delta_sec;
        while self.tick >= 1. {
            self.tick -= 1.;
            self.seconds_left -= 1;
            if self.seconds_left < 1 {
                self.seconds_left = 0;
                self.finish(Outcome::TimedOut);
                return;
            }
        }
    }

    /// 人物现在的碰撞盒（当前动作的包围盒）。
    fn man_box(&self) -> Aabb {
        if self.phase == Phase::Jumping {
            self.shapes.man_jump
        } else {
            self.shapes.man_slide
        }
    }

    /// 摔完刚转回正常滑（`+0x314 == 1`，上一个状态是撞）：还压着哪个障碍，就沿路径往前推它最长边的
    /// 两倍，再把人贴到那个路径点上（`FUN_00531160`）。原版这一段一直查到「起身」那个动作播完才把
    /// `+0x314` 清掉；这里推一次就清 [推断：推过去就不再压着，多查几帧也不会再推]。
    fn push_off_obstacle(&mut self) {
        if self.prev_phase != Phase::Hit || self.phase != Phase::Rolling {
            return;
        }
        self.prev_phase = Phase::Rolling;
        let man = self.man_box();
        let here = (self.pos.0, self.pos.1, self.pos.2);
        let hit = self
            .obstacles
            .iter()
            .zip(&self.shapes.obstacles)
            .find(|(o, b)| man.hits(here, b, (o.x, o.y, o.z)))
            .map(|(_, b)| b.longest());
        if let Some(longest) = hit {
            self.travelled += longest * 2.;
            if let Some(p) = self.path.at(self.travelled) {
                self.pos = p;
            }
        }
    }

    fn collide(&mut self) {
        let here = (self.pos.0, self.height(), self.pos.2);
        let man = self.man_box();

        // 障碍：撞上就进减速状态（`FUN_00530be0(1)`，`+0x31c = 1` 先掉速）。推过去要等停下来之后，见
        // [`Self::push_off_obstacle`]。
        let hit = self
            .obstacles
            .iter()
            .zip(&self.shapes.obstacles)
            .any(|(o, b)| man.hits(here, b, (o.x, o.y, o.z)));
        if hit {
            self.braking = true;
            self.brake_locked = false;
            self.set_phase(Phase::Hit);
            return;
        }

        // 收集品：背包里已经有的原版就不捡（这里 `present` 已经把这一层
        // 筛掉了），捡到就把模型藏起来、记进这一局的槽。
        for (t, b) in self.treasures.iter_mut().zip(&self.shapes.treasures) {
            if !t.present {
                continue;
            }
            if man.hits(here, b, (t.x, t.y, t.z)) {
                t.present = false;
                if !self.picked.contains(&t.id) {
                    self.picked.push(t.id);
                }
                break;
            }
        }
    }
}

/// 零长向量归一化不出方向，返回 `None`，调用方保留上一帧的。
fn normalize(v: (f32, f32, f32)) -> Option<(f32, f32, f32)> {
    let len = (v.0 * v.0 + v.1 * v.1 + v.2 * v.2).sqrt();
    (len > f32::EPSILON).then(|| (v.0 / len, v.1 / len, v.2 / len))
}

fn distance(a: (f32, f32, f32), b: (f32, f32, f32)) -> f32 {
    let (dx, dy, dz) = (a.0 - b.0, a.1 - b.1, a.2 - b.2);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 实机那条路的点数。
    const POINTS: usize = 4950;

    /// 一条直的假雪道：沿 x 每 `spacing` 个单位一个点，点数照实机。
    fn straight(points: usize, spacing: f32) -> SkiPath {
        let mut text = String::from("*GEOMOBJECT {\n*NODE_NAME \"skee_path\"\n");
        for i in 0..points {
            text.push_str(&format!(
                "*MESH_VERTEX {i} {} 0.0 0.0\n",
                i as f32 * spacing
            ));
        }
        text.push_str("}\n");
        SkiPath::parse(&text)
    }

    /// 三件收集品分别摆在第 100 / 2100 / 4100 个点上，互相离得开。
    const RAND: [u32; 3] = [0, 2000, 4000];

    fn with_spacing(spacing: f32) -> SkiRun {
        SkiRun::new(
            straight(POINTS, spacing),
            Shapes::shipped(),
            &[6019, 6029, 6017],
            &[false; 3],
            &RAND,
        )
    }

    fn run() -> SkiRun {
        with_spacing(1.)
    }

    #[test]
    fn the_six_obstacles_sit_where_the_original_puts_them() {
        // 实机 4950 个点：step = (4950 − 1560) / 6 = 565。
        assert_eq!(
            obstacle_indices(4950),
            [300, 865, 1430, 1995, 2560, 3125],
            "障碍下标和 FUN_0052ed30 对不上"
        );
    }

    #[test]
    fn the_treasures_hang_at_jump_height() {
        // 抬高量用的是取整之后的半径：r + r/3，整数除法。
        assert_eq!(treasure_lift(24.), 32.);
        assert_eq!(treasure_lift(10.), 13., "10 + 10/3 = 13，不是 13.33");
        // 实机人高 76.04：取整成 76，抬 76 + 25 = 101。
        assert_eq!(treasure_lift(Shapes::shipped().man_height), 101.);
        // 跳的上限是浮点算的 r × 4/3。
        assert!((jump_ceiling(24.) - 32.).abs() < 1e-4);
        assert!(
            jump_ceiling(Shapes::shipped().man_height)
                >= treasure_lift(Shapes::shipped().man_height),
            "跳不到收集品那个高度就没法捡"
        );
    }

    #[test]
    fn a_treasure_index_never_runs_off_the_path() {
        // 原版是 `rand % N + 100`，最大能到 N + 99——那是越界读；移植端夹住。
        assert_eq!(treasure_index(0, 4950), 100);
        assert_eq!(treasure_index(4949, 4950), 4949);
        assert_eq!(treasure_index(0, 0), 0);
    }

    #[test]
    fn the_ball_starts_at_cruise_speed() {
        let r = run();
        assert_eq!(r.speed, CRUISE_SPEED);
        assert_eq!(r.phase(), Phase::Rolling);
        assert_eq!(r.seconds_left, TIME_LIMIT_SECS);
    }

    #[test]
    fn boosting_only_works_after_halfway() {
        // 点距 10：半程之后要过 850 才到下一个障碍，加速前不会撞上。
        let mut r = with_spacing(10.);
        let input = SkiInput {
            boost: true,
            jump: false,
        };
        // 一开始按加速没用——原版把加速挂在 `+0x2ac` 上。
        r.update(1. / 60., input);
        assert!(r.speed <= CRUISE_SPEED + 1e-4, "半程之前不该加速");

        // 手动滚过半程。
        while !r.chasing() && r.outcome().is_none() {
            r.update(1. / 60., SkiInput::default());
        }
        assert!(r.chasing());
        assert_eq!(r.phase(), Phase::Rolling);
        let before = r.speed;
        r.update(1. / 60., input);
        assert!(r.speed > before, "过了半程按住左键要加速");
        // 一直按到顶就停在 28.86。
        for _ in 0..500 {
            r.update(1. / 60., input);
            if r.outcome().is_some() {
                break;
            }
        }
        assert!(r.speed <= BOOST_MAX + 1e-3, "加速上限是 {BOOST_MAX}");
    }

    #[test]
    fn a_jump_goes_up_and_comes_back_down() {
        // 点距拉大，起跳那一段路上没有障碍。
        let mut r = with_spacing(10.);
        r.update(
            1. / 60.,
            SkiInput {
                boost: false,
                jump: true,
            },
        );
        assert_eq!(r.phase(), Phase::Jumping);
        let mut peak: f32 = 0.;
        for _ in 0..2000 {
            r.update(1. / 60., SkiInput::default());
            peak = peak.max(r.height() - r.pos.1);
            if r.phase() != Phase::Jumping {
                break;
            }
        }
        assert_eq!(r.phase(), Phase::Rolling, "跳完要落回地面");
        assert!(
            peak >= jump_ceiling(Shapes::shipped().man_height) - 1e-3,
            "最高只跳到 {peak}"
        );
    }

    #[test]
    fn hitting_an_obstacle_stalls_the_ball() {
        let mut r = run();
        // 直接把球送到第一个障碍跟前。
        let o = r.obstacles()[0];
        r.travelled = o.distance;
        r.pos = r.path().at(o.distance).unwrap();
        r.update(1. / 60., SkiInput::default());
        assert_eq!(r.phase(), Phase::Hit, "撞上障碍要进减速状态");

        // 掉到 0 的那一帧就已经回到正常滚、并被 `ACCEL` 补回 0.12，所以
        // 看的是这一段里的最低速度。
        let mut slowest = r.speed;
        for _ in 0..4000 {
            r.update(1. / 60., SkiInput::default());
            slowest = slowest.min(r.speed);
            if r.phase() == Phase::Rolling {
                break;
            }
        }
        assert!(slowest <= ACCEL, "撞完速度要掉到 0，最低只到 {slowest}");
        assert_eq!(r.phase(), Phase::Rolling, "停住之后回正常滚");
    }

    #[test]
    fn a_treasure_in_the_way_is_only_picked_up_once() {
        let mut r = run();
        let t = r.treasures()[0];
        r.travelled = t.distance;
        r.pos = (t.x, t.y, t.z);
        r.update(1. / 60., SkiInput::default());
        assert_eq!(r.picked(), &[t.id]);
        // 再滚一帧不该重复记。
        r.pos = (t.x, t.y, t.z);
        r.update(1. / 60., SkiInput::default());
        assert_eq!(r.picked().len(), 1);
    }

    #[test]
    fn a_treasure_overhead_needs_a_jump() {
        // 古董挂在路面上 101（人高 76 + 25），滑的盒子顶在 68，碰不到；跳起来的盒子（h05，到 121）够得着。
        let mut r = with_spacing(10.);
        let t = r.treasures()[0];
        let below = r.path().at(t.distance).unwrap();
        r.travelled = t.distance;
        r.pos = below;
        r.update(1. / 60., SkiInput::default());
        assert!(r.picked().is_empty(), "不跳不该捡到");

        let mut r = with_spacing(10.);
        r.travelled = t.distance - 60.;
        r.pos = r.path().at(r.travelled).unwrap();
        r.update(
            1. / 60.,
            SkiInput {
                boost: false,
                jump: true,
            },
        );
        for _ in 0..600 {
            r.update(1. / 60., SkiInput::default());
            if !r.picked().is_empty() || r.phase() != Phase::Jumping {
                break;
            }
        }
        assert_eq!(r.picked(), &[t.id], "跳起来要捡到");
    }

    #[test]
    fn a_tumble_pushes_past_the_obstacle_only_after_stopping() {
        let mut r = run();
        let o = r.obstacles()[0];
        r.travelled = o.distance;
        r.pos = r.path().at(o.distance).unwrap();
        r.update(1. / 60., SkiInput::default());
        assert_eq!(r.phase(), Phase::Hit);
        let at_hit = r.travelled;
        assert!(at_hit < o.distance + 10., "撞上的当帧不推");
        for _ in 0..4000 {
            r.update(1. / 60., SkiInput::default());
            if r.phase() == Phase::Rolling {
                break;
            }
        }
        // 停下来回到正常滑那一帧，还压着障碍就推过最长边的两倍（144.76 × 2）。
        assert!(
            r.travelled >= at_hit + 289.,
            "停下后要推过障碍：{}",
            r.travelled
        );
    }

    #[test]
    fn something_already_in_the_bag_is_not_on_the_slope() {
        let r = SkiRun::new(
            straight(POINTS, 1.),
            Shapes::shipped(),
            &[6019, 6029, 6017],
            &[true, false, false],
            &RAND,
        );
        assert!(!r.treasures()[0].present, "背包里有的不该摆出来");
        assert!(r.treasures()[1].present);
    }

    #[test]
    fn running_out_of_time_pays_nothing() {
        // 巡航速度 10.86 换算下来约 507 单位/秒，59 秒连半程都滚不到（追球还没出来）。
        let mut r = with_spacing(20.);
        for _ in 0..4200 {
            r.update(1. / 60., SkiInput::default());
            if r.outcome().is_some() {
                break;
            }
        }
        assert_eq!(r.outcome(), Some(Outcome::TimedOut));
        assert_eq!(r.seconds_left, 0);
        assert!(!r.awards(), "超时一件都不发");
        assert_eq!(r.score(), r.picked().len() as i32 * SCORE_PER_PICKUP);
    }

    #[test]
    fn the_chaser_starts_where_halfway_was_reached() {
        let mut r = with_spacing(1.);
        while !r.chasing() && r.outcome().is_none() {
            r.update(1. / 60., SkiInput::default());
        }
        let half = r.chaser;
        assert!(
            (half - r.path().length() * 0.5).abs() < 20.,
            "记下的是过半程那一刻滑到的距离"
        );
        for _ in 0..12 {
            r.update(1. / 60., SkiInput::default());
        }
        assert_eq!(r.chaser, half, "放出来之前不动");
    }

    #[test]
    fn getting_caught_pays_nothing() {
        let mut r = run();
        // 推到半程之后，把追球贴到身后。
        while !r.chasing() && r.outcome().is_none() {
            r.update(1. / 60., SkiInput::default());
        }
        // 放出雪球之前追不上。
        r.update(1. / 60., SkiInput::default());
        assert!(!r.chaser_out());
        for _ in 0..30 {
            r.update(1. / 60., SkiInput::default());
        }
        assert!(r.chaser_out(), "过半程 0.3 秒后放出雪球");
        r.chaser = r.travelled;
        r.update(1. / 60., SkiInput::default());
        assert_eq!(r.outcome(), Some(Outcome::Caught));
        assert!(!r.awards(), "被追上一件都不发");
    }

    #[test]
    fn rolling_the_whole_path_pays_out() {
        let mut r = run();
        for _ in 0..60_000 {
            // 一直按加速，障碍到跟前就跳，才跑得过追球。
            let ahead = r
                .obstacles()
                .iter()
                .any(|o| (0.0..150.).contains(&(o.distance - r.travelled)));
            r.update(
                1. / 240.,
                SkiInput {
                    boost: true,
                    jump: ahead,
                },
            );
            if r.outcome().is_some() {
                break;
            }
        }
        assert_eq!(r.outcome(), Some(Outcome::Cleared), "该滚到头");
        assert!(r.awards(), "滚完且还有时间就该发东西");
        assert_eq!(
            r.score(),
            r.picked().len() as i32 * SCORE_PER_PICKUP + r.seconds_left * SCORE_PER_SECOND
        );
    }
}
